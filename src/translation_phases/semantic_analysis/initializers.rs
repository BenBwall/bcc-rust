//! Phase-7 current-object initializer traversal.
//! C99: §6.7.8, pp. 125-130; PDF pp. 137-142. Cursors and brace frames
//! live in arenas; brace elision and designators never recurse on the host
//! stack. `overrides.rs` records which subobjects an initializer has
//! written, for Clang's `-Winitializer-overrides`.

pub(super) mod bss;
mod overrides;
mod plan;

use overrides::{
    Step,
    Write,
    Writes,
};
use plan::{
    Completion,
    Container,
    PlanBuilder,
    Site,
};
use rustc_hash::FxBuildHasher;

use super::{
    super::parsing::declaration_syntax::BracedInitializerList,
    Analyzer,
    BindingKind,
    BitRange,
    Duration,
    FlexibleArray,
    InitOperation,
    Linkage,
    ScopeKind,
    SemanticErrorKind,
    Subobject,
    expressions::{
        ConstantClass,
        ConstantFolding,
        ConversionKind,
    },
    integer::Integer,
    types::{
        ArrayBound,
        Member,
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Designator,
                DesignatorType,
                Initializer,
                InitializerElement,
                InitializerType,
                TypeQualifiers,
            },
            syntax::{
                Expression,
                ExpressionType,
                UnaryOperator,
            },
        },
        preprocessing::{
            KeywordTokenType,
            LiteralEncoding,
            StringTokenType,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Applies scalar assignment conversion and aggregate current-object rules.
    /// The returned type completes an array of unknown size from its highest
    /// initialized index. C99: §6.7.8p11-22, pp. 126-127; PDF pp. 138-139.
    ///
    /// The second result describes the flexible array member the
    /// initializer gives elements to; only a static object
    /// (`static_object`) may initialize its own structure's member, a GNU
    /// extension. Elsewhere only an empty list may, as in Clang.
    /// C99: §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    ///
    /// An initializer that replaces an earlier one for the same subobject
    /// is reported as Clang's `-Winitializer-overrides` reports it.
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    pub(super) fn check_initializer(
        &mut self,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        static_storage: bool,
        static_object: bool,
        declaration_site: Option<SourceVectors>,
    ) -> (TypeId, Option<FlexibleArray>) {
        if initializer.recovered || self.types.unanalyzed(ty) {
            return (ty, None);
        }
        // §6.7.8p3 excludes variable length array types, not pointers to them.
        if (matches!(self.types.nodes[ty.index], TypeKind::Array(..)) && self.variably_modified(ty))
            || (!self.complete_object(ty)
                && !matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Array(_, ArrayBound::Incomplete)
                ))
        {
            self.error(
                SemanticErrorKind::InvalidInitializer,
                initializer.source_vectors,
                None,
                None,
            );
            return (ty, None);
        }
        // The scratch arena lives for the whole translation unit, so buffers
        // made per initializer would never be freed (issue #92); reuse one
        // set instead. A nested check, should analysis ever start one, takes
        // a fresh set.
        let mut buffers = self
            .initializer_buffers
            .take()
            .unwrap_or_else(|| CheckBuffers::new_in(self.scratch));
        buffers.reset(initializer.source_vectors);
        let checked = self.check_initializer_with(
            &mut buffers,
            ty,
            initializer,
            static_storage,
            static_object,
            declaration_site,
        );
        self.initializer_buffers = Some(buffers);
        checked
    }

    /// [`Self::check_initializer`] once the object's type admits an
    /// initializer, over cleared `buffers`.
    fn check_initializer_with(
        &mut self,
        buffers: &mut CheckBuffers<'tu, 's>,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        static_storage: bool,
        static_object: bool,
        declaration_site: Option<SourceVectors>,
    ) -> (TypeId, Option<FlexibleArray>) {
        let CheckBuffers {
            work,
            writes,
            leaves,
            plan,
        } = buffers;
        // The extents of the object's own flexible array member once an
        // element of it is reached.
        let mut flexible: Option<FlexibleExtent> = None;
        let mut count = 0;
        let before = self.semantic_errors;
        let mut unmodeled = false;
        work.push(InitWork::Value(
            ty,
            initializer,
            Writes::ROOT,
            Site::root(initializer.source_vectors),
        ));
        while let Some(task) = work.pop() {
            #[cfg(test)]
            self.review_step(3);
            match task {
                | InitWork::Value(target, init, node, site) => {
                    if init.recovered {
                        continue;
                    }
                    let destination = site.destination(target);
                    if self.initializer_unanalyzed(target, leaves) {
                        plan.unmodeled(destination, site.source);
                        continue;
                    }
                    let bit_field = site.bits.is_some();
                    let completion =
                        self.completion(target, std::ptr::eq(init, initializer), &site);
                    match init.kind {
                        | InitializerType::AssignmentExpression(e) => {
                            let info = self.expression_info(e);
                            if self.types.unanalyzed(info.ty) {
                                if self.aggregate(target) {
                                    plan.invalidate();
                                } else {
                                    plan.store(
                                        e,
                                        destination,
                                        site.bits,
                                        site.scalar_source.unwrap_or(e.source_vectors),
                                    );
                                }
                                continue;
                            }
                            if let Some(extent) = self.initialize_string(target, e) {
                                plan.string(e, destination, completion);
                                if std::ptr::eq(init, initializer) {
                                    count = count.max(extent);
                                } else if let Some(flexible) = &mut flexible
                                    && self.flexible_array_type(target, ty)
                                {
                                    flexible.extend(extent);
                                }
                                continue;
                            }
                            let diagnostic_site = if std::ptr::eq(init, initializer) {
                                declaration_site.unwrap_or(e.source_vectors)
                            } else {
                                e.source_vectors
                            };
                            if let Some(length) =
                                self.copy_array_literal(plan, target, e, &site, static_storage)
                            {
                                if std::ptr::eq(init, initializer) {
                                    count = count.max(length);
                                }
                                continue;
                            }
                            let errors_before_conversion = self.semantic_errors;
                            if matches!(self.types.nodes[target.index], TypeKind::Array(..))
                                || !self.assignment_conversion_at(target, info, diagnostic_site)
                            {
                                self.error(
                                    SemanticErrorKind::InvalidInitializer,
                                    diagnostic_site,
                                    None,
                                    None,
                                );
                                continue;
                            }
                            if self.semantic_errors != errors_before_conversion {
                                // Suppress constant-initializer errors after a
                                // rejected conversion (C99 §6.7.8p11).
                                continue;
                            }
                            self.convert(e, target.unqualified(), ConversionKind::Assignment);
                            // GNU: a static object's compound literal of its
                            // type gives it the literal's bytes, so the plan
                            // takes the literal from inside the wrappers GCC
                            // folds away, fewer of them inside braces, where
                            // Clang's evaluator still reads through them.
                            let (literal, converted) = if static_storage {
                                self.static_literal(target, e, std::ptr::eq(init, initializer))
                            } else {
                                (None, None)
                            };
                            // C99 §6.7.8p13: a structure or union initialized
                            // by an expression of its type is copied whole, as
                            // is a static scalar's compound literal of its
                            // type (GNU), whose bytes are the literal's own.
                            if matches!(
                                self.types.nodes[self.types.non_atomic(target).index],
                                TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
                            ) || (literal.is_some() && !bit_field)
                            {
                                plan.copy(
                                    literal.unwrap_or(e),
                                    destination,
                                    self.enclosing_unions(&site),
                                );
                            } else {
                                plan.store(
                                    literal.or(converted).unwrap_or(e),
                                    destination,
                                    site.bits,
                                    site.scalar_source.unwrap_or(e.source_vectors),
                                );
                            }
                            if let Some(literal) = literal.or(converted) {
                                self.check_static_compound_literal(e, literal);
                            } else if static_storage {
                                self.check_static_initializer(target, info, e, site.bits);
                            }
                        },
                        | InitializerType::InitializerList(list) => {
                            // GNU vector extension of C99 §6.7.8p13: a single
                            // compatible vector initializes the whole object.
                            if self.vector(target).is_some()
                                && let [element] = list.elements.as_slice()
                                && element.designation.is_none()
                                && let InitializerType::AssignmentExpression(e) =
                                    element.initializer.kind
                                && self.vector_assignment(
                                    target,
                                    self.types.non_atomic(self.expression_info(e).ty),
                                )
                            {
                                work.push(InitWork::Value(
                                    target,
                                    element.initializer,
                                    node,
                                    Site {
                                        scalar_source: Some(
                                            site.scalar_source.unwrap_or(site.source),
                                        ),
                                        ..site
                                    },
                                ));
                                continue;
                            }
                            // A character or wide array may have its string
                            // enclosed in braces (§6.7.8p14-15). A string that
                            // does not fit the array, as `{ "x" }` for an array
                            // of pointers, initializes its first element like
                            // any other list (§6.7.8p17), as in Clang.
                            if let [element] = list.elements.as_slice()
                                && element.designation.is_none()
                                && let InitializerType::AssignmentExpression(e) =
                                    element.initializer.kind
                                && self.braced_string_fits(target, e)
                                && let Some(extent) = self.initialize_string(target, e)
                            {
                                plan.string(e, destination, completion);
                                if std::ptr::eq(init, initializer) {
                                    count = count.max(extent);
                                } else if let Some(flexible) = &mut flexible
                                    && self.flexible_array_type(target, ty)
                                {
                                    flexible.extend(extent);
                                }
                                continue;
                            }
                            // A brace pair around a scalar's initializer is
                            // part of it (§6.7.8p11): the scalar is its only
                            // subobject, with the outer pair's provenance.
                            let (zero, scalar) = if self.aggregate(target) {
                                (plan.zero(destination, completion, site.source), None)
                            } else {
                                if list.elements.as_slice().is_empty() {
                                    plan.empty_scalar(destination, init.source_vectors);
                                }
                                (
                                    PlanBuilder::NO_SCOPE,
                                    Some(Site {
                                        scalar_source: Some(
                                            site.scalar_source.unwrap_or(site.source),
                                        ),
                                        ..site
                                    }),
                                )
                            };
                            let container = self.plan_container(&site, target);
                            let current = self.scratch.alloc(Current {
                                container:  target,
                                index:      self.first_subobject(target),
                                parent:     None,
                                root_index: 0,
                                at:         container,
                            });
                            work.push(InitWork::List(ListWork {
                                root: target,
                                list,
                                position: 0,
                                cursor: current,
                                infer: std::ptr::eq(init, initializer),
                                base: node,
                                container,
                                zero,
                                close: 0,
                                scalar,
                            }));
                        },
                    }
                },
                | InitWork::List(task) => {
                    let ListWork {
                        root,
                        list,
                        position,
                        cursor,
                        infer,
                        base,
                        container,
                        zero,
                        close,
                        scalar,
                    } = task;
                    // The previous element is checked, with every range
                    // designator it repeated over.
                    plan.close_repeats(close);
                    let Some(element) = list.elements.as_slice().get(position) else {
                        plan.close_zero(zero);
                        continue;
                    };
                    let next_task = |cursor, close| {
                        InitWork::List(ListWork {
                            position: position + 1,
                            cursor,
                            close,
                            ..task
                        })
                    };
                    let source = scalar.map_or(element.source_vectors, |site| site.source);
                    let mut cursor = *cursor;
                    // The step of the designation's last range designator
                    // and its lower bound: the element writes every element
                    // of that range (an earlier range, its last).
                    let mut range = None;
                    // Where writes inside each designated subobject begin.
                    let mut starts = ArenaVec::new_in(self.scratch);
                    // The range designators whose elements the plan repeats
                    // this element's operations for.
                    let mut repeats = 0;
                    if let Some(designation) = element.designation {
                        let mut selected = root;
                        let mut selected_at = container;
                        let mut parent = None;
                        let mut valid = true;
                        let mut unknown_designator = false;
                        // The designator that names a flexible array member,
                        // the member, and the cursor at it.
                        let mut flexible_designator = None;
                        let mut depth = 0_usize;
                        let designators = designation.designators.as_slice();
                        for (ordinal, designator) in designators.iter().enumerate() {
                            // A member of an anonymous member is reached
                            // through that member (C11 §6.7.2.1p13).
                            let mut path = ArenaVec::new_in(self.scratch);
                            let mut lower = None;
                            let index = match designator.kind {
                                | DesignatorType::Array(expression) => {
                                    let info = self.expression_info(expression.expression());
                                    unknown_designator |= self.types.unanalyzed(info.ty);
                                    if !info.ice
                                        || !matches!(
                                            self.types.nodes[selected.index],
                                            TypeKind::Array(..)
                                        )
                                    {
                                        None
                                    } else {
                                        info.integer.and_then(Integer::to_u64)
                                    }
                                },
                                | DesignatorType::Field(name) | DesignatorType::GnuField(name) =>
                                    if let TypeKind::Tag(id) = self.types.nodes[selected.index]
                                        && let Some(&index) =
                                            self.member_indices.get(&(id, name.name))
                                        && let Some(field) =
                                            self.types.tags[id].fields.get().get(index)
                                    {
                                        path.extend(field.path.iter().map(|i| i as u64));
                                        path.pop()
                                    } else {
                                        None
                                    },
                                // GNU Designated Initializers: both inclusive
                                // endpoints are integer constant expressions.
                                // All elements have the same type; checking the
                                // last also validates the bound and
                                // continuation. <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>
                                | DesignatorType::Range(range) => {
                                    let lower_info = self.expression_info(range.lower.expression());
                                    let upper = self.expression_info(range.upper.expression());
                                    unknown_designator |= self.types.unanalyzed(lower_info.ty)
                                        || self.types.unanalyzed(upper.ty);
                                    match (
                                        lower_info.integer.and_then(Integer::to_u64),
                                        upper.integer.and_then(Integer::to_u64),
                                    ) {
                                        | (Some(lower_index), Some(upper_index))
                                            if lower_info.ice
                                                && upper.ice
                                                && lower_index <= upper_index
                                                && matches!(
                                                    self.types.nodes[selected.index],
                                                    TypeKind::Array(..)
                                                ) =>
                                        {
                                            lower = Some(lower_index);
                                            Some(upper_index)
                                        },
                                        | _ => None,
                                    }
                                },
                                | DesignatorType::Error => {
                                    valid = false;
                                    break;
                                },
                            };
                            let Some(index) = index else {
                                valid = false;
                                break;
                            };
                            path.push(index);
                            let steps = path.len();
                            for (step, &index) in path.iter().enumerate() {
                                #[cfg(test)]
                                self.review_step(3);
                                let Some(child) = self.subobject(selected, index) else {
                                    valid = false;
                                    break;
                                };
                                cursor = Current {
                                    container: selected,
                                    index,
                                    parent,
                                    root_index: parent
                                        .map_or(index, |p: &Current<'_>| p.root_index),
                                    at: selected_at,
                                };
                                if flexible_designator.is_none()
                                    && let Some(member) = self.flexible_member(selected, index)
                                {
                                    flexible_designator = Some((ordinal, member, cursor));
                                }
                                self.plan_container_entry(
                                    plan,
                                    &cursor,
                                    zero,
                                    designator.source_vectors,
                                );
                                if let Some(lower) = lower {
                                    match self.subobject_stride(selected) {
                                        | Some(stride) => plan.open_repeat(lower, index, stride),
                                        | None => plan.invalidate(),
                                    }
                                    repeats += 1;
                                }
                                match self.plan_child(selected, &selected_at, index, child) {
                                    | Some(at) => selected_at = at,
                                    | None => plan.invalidate(),
                                }
                                parent = Some(&*self.scratch.alloc(cursor));
                                selected = child;
                                depth += 1;
                                starts.push(if step + 1 < steps {
                                    designator.source_vectors
                                } else {
                                    designators
                                        .get(ordinal + 1)
                                        .map_or(element.initializer.source_vectors, |next| {
                                            next.source_vectors
                                        })
                                });
                            }
                            if !valid {
                                break;
                            }
                            if let Some(lower) = lower {
                                range = Some((depth - 1, lower));
                            }
                        }
                        if !valid {
                            unmodeled |= unknown_designator;
                            if !unknown_designator
                                && !designation.recovered
                                && !designation
                                    .designators
                                    .iter()
                                    .any(|d| matches!(d.kind, DesignatorType::Error))
                            {
                                self.error(
                                    SemanticErrorKind::InvalidDesignator,
                                    designation.source_vectors,
                                    None,
                                    None,
                                );
                            }
                            work.push(next_task(self.scratch.alloc(cursor), repeats));
                            continue;
                        }
                        // As in GCC and Clang, a designator may name a
                        // flexible array member, but not an element inside
                        // it, and only a brace-enclosed list or a string
                        // literal initializes it there.
                        if let Some((at, member, at_member)) = flexible_designator
                            && self.flexible_designation(designators, at, member, element)
                        {
                            work.push(next_task(
                                self.scratch.alloc(self.advance_current(at_member)),
                                repeats,
                            ));
                            continue;
                        }
                    }
                    let Some(mut target) = self.subobject(cursor.container, cursor.index) else {
                        self.error(
                            SemanticErrorKind::ExcessInitializer,
                            element.source_vectors,
                            None,
                            None,
                        );
                        work.push(next_task(self.scratch.alloc(cursor), repeats));
                        continue;
                    };
                    if infer {
                        count = count.max(cursor.root_index.saturating_add(1));
                    }
                    let mut own_flexible = false;
                    if self
                        .flexible_member(cursor.container, cursor.index)
                        .is_some()
                    {
                        // Only the object's own list reaches its structure's
                        // member at the outermost level.
                        let own =
                            infer && cursor.parent.is_none() && cursor.container.index == ty.index;
                        own_flexible = own && static_object;
                        let (empty, whole) = match element.initializer.kind {
                            | InitializerType::InitializerList(list) =>
                                (list.elements.as_slice().is_empty(), true),
                            | InitializerType::AssignmentExpression(e) =>
                                (false, self.whole_object_expression(target, e)),
                        };
                        self.enter_flexible_array(
                            target,
                            own_flexible,
                            empty,
                            whole,
                            element.source_vectors,
                            &mut flexible,
                        );
                    } else if element.designation.is_none() {
                        self.plan_container_entry(plan, &cursor, zero, source);
                    }
                    // Without explicit braces, descend until an expression can
                    // initialize the complete current subobject or a scalar
                    // leaf.
                    if let InitializerType::AssignmentExpression(e) = element.initializer.kind {
                        while self.aggregate(target) && !self.whole_object_expression(target, e) {
                            #[cfg(test)]
                            self.review_step(3);
                            let next_index = self.first_subobject(target);
                            let Some(child) = self.subobject(target, next_index) else {
                                break;
                            };
                            if self.flexible_member(target, next_index).is_some() {
                                self.enter_flexible_array(
                                    child,
                                    false,
                                    false,
                                    false,
                                    element.source_vectors,
                                    &mut flexible,
                                );
                            }
                            cursor = self.plan_descent(
                                plan,
                                cursor,
                                (target, next_index),
                                zero,
                                (source, e.source_vectors),
                            );
                            target = child;
                        }
                    }
                    if let Some(flexible) = &mut flexible {
                        self.extend_flexible(flexible, &cursor, ty);
                    }
                    let node = self.record_write(writes, base, &cursor, &starts, range, element);
                    // A braced scalar's element initializes the scalar itself.
                    let site = match scalar {
                        | Some(site) => site,
                        | None =>
                            match self.plan_site(cursor.container, &cursor.at, cursor.index, source)
                            {
                                | Some(site) => Site {
                                    own_flexible,
                                    ..site
                                },
                                | None => {
                                    plan.invalidate();
                                    Site::root(source)
                                },
                            },
                    };
                    let next = self.advance_current(cursor);
                    work.push(next_task(self.scratch.alloc(next), repeats));
                    work.push(InitWork::Value(target, element.initializer, node, site));
                },
            }
        }
        if unmodeled {
            return (self.types.unknown(), None);
        }
        let (ty, flexible) =
            match self.complete_initialized_type(ty, initializer, count, flexible, before) {
                | Ok(completed) => completed,
                | Err(rejected) => return rejected,
            };
        if self.semantic_errors == before
            && let Some(plan) = plan.finish(self.types.tu, ty, flexible.map(|array| array.written))
        {
            _ = self
                .initializers
                .plans
                .insert(std::ptr::from_ref(initializer).addr(), plan);
        }
        (ty, flexible)
    }

    /// An initializer of an object with static storage duration is a
    /// constant expression: an arithmetic constant, an address constant or
    /// a null pointer constant fitting the target type.
    /// C99: §6.7.8 paragraph 4, p. 125; PDF p. 137; §6.6 paragraphs 7-9,
    /// pp. 95-96; PDF pp. 107-108.
    fn check_static_initializer(
        &mut self,
        target: TypeId,
        info: super::ExpressionInfo<'tu>,
        e: &'tu Expression<'tu>,
        bits: Option<BitRange>,
    ) {
        let bit_width = bits.map(|bits| bits.width);
        let bit_field = bits.is_some();
        let overflow = self.static_float_conversion(target, info, e, bit_width)
            || info.folding == ConstantFolding::Overflow;
        if overflow
            && matches!(
                self.context.configuration.compiler_identity(),
                crate::configuration::CompilerIdentity::Gcc
            )
        {
            self.context.report_extension(
                crate::configuration::Feature::OverflowingConstant,
                "overflow in constant expression",
                e.source_vectors,
            );
        } else if overflow && bit_field {
            // Clang's evaluator gives up on the undefined conversion, and
            // its code generation emits no bit-field without a value.
            self.error(
                SemanticErrorKind::NonConstantInitializer,
                e.source_vectors,
                None,
                None,
            );
            return;
        }
        let address = info.constant == ConstantClass::Address
            || (info.static_address
                && matches!(
                    self.types.nodes[info.ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                ));
        let valid = if matches!(
            // C99 §6.3.1.2p1 and §6.6p7: address constants
            // initialize bool, including C11 atomic bool.
            self.types.nodes[self.types.non_atomic(target).index],
            TypeKind::Scalar(Scalar::Bool)
        ) && address
        {
            true
        } else if self.arithmetic(target) {
            (info.constant == ConstantClass::Arithmetic
                && (info.integer.is_some()
                    || info.floating().is_some()
                    || info.binary128().is_some()
                    || info.unfolded_binary128))
                // GNU extension: GCC "Labels as
                // Values"; as in Clang, not into a
                // bit-field.
                || (info.constant == ConstantClass::LabelDifference
                    && !bit_field
                    && self.label_difference_fits(target, info.ty))
                || (address && self.pointer_width_integer(target))
        } else {
            address
                || self.null_pointer_constant(info)
                || (matches!(
                    self.types.nodes[self.types.non_atomic(target).index],
                    TypeKind::Pointer(_)
                ) && info.ice
                    && info.integer.is_some())
        };
        if !valid {
            self.error(
                if info.constant == ConstantClass::Arithmetic {
                    SemanticErrorKind::ConstantOverflow
                } else {
                    SemanticErrorKind::NonConstantInitializer
                },
                e.source_vectors,
                None,
                None,
            );
        }
    }

    /// Folds the conversion of the floating constant `e` to the integer
    /// type `target` of a static object, out of the integer's range, to the
    /// value its identity's compiler stores, which lowering then writes:
    /// GCC (`convert_and_check`) saturates to the bit-field's own width
    /// (`100.0` stores 7 in a 4-bit `int`) and warns under `-Woverflow`;
    /// Clang saturates to the declared type, whose low bits a bit-field
    /// keeps, and warns only for a literal (`-Wliteral-conversion`, in
    /// [`Self::literal_conversion_warning`]). Returns whether the value was
    /// out of range. The behavior is undefined in C.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55; §6.7.8 paragraph 4,
    /// p. 125; PDF p. 137.
    fn static_float_conversion(
        &mut self,
        target: TypeId,
        info: super::ExpressionInfo<'tu>,
        e: &'tu Expression<'tu>,
        bit_width: Option<u32>,
    ) -> bool {
        use super::super::preprocessing::FloatTokenType;
        let value = match (info.floating(), info.binary128()) {
            | (Some(value), _) => FloatTokenType::LongDouble(value.real),
            | (_, Some(value)) => FloatTokenType::Float128(value.real),
            | _ => return false,
        };
        if info.integer.is_some() || self.integer_type(info.ty).is_some() {
            return false;
        }
        let gcc = matches!(
            self.context.configuration.compiler_identity(),
            crate::configuration::CompilerIdentity::Gcc
        );
        let Some((stored, true)) =
            self.saturated_float_cast(target, value, bit_width.filter(|_| gcc))
        else {
            return false;
        };
        if let Some(&index) = self.expression_indices.get(&std::ptr::from_ref(e).addr()) {
            self.expressions[index].integer = Some(stored);
        }
        if gcc {
            let text = crate::diagnostics::format_in!(
                self.scratch,
                "overflow in conversion from `{}` to `{}` changes value to {}",
                self.conversion_spelling(info.ty),
                self.conversion_spelling(target),
                stored
            );
            let text = self.context.string_cache.intern(text);
            self.group_diagnostic(
                crate::configuration::DiagnosticGroup::Overflow,
                SemanticErrorKind::FloatOverflow(text),
                e.source_vectors,
                None,
                None,
            );
        }
        true
    }

    /// The compound literal whose bytes `e` gives a static object or
    /// element of type `target`, and else the arithmetic literal of another
    /// type whose value GCC converts to an element's type. The initializer
    /// is `whole` when it is the object's own, without braces; inside
    /// braces GCC peels fewer wrappers, where Clang's evaluator still reads
    /// through them.
    /// GNU extension: GCC manual, "Compound Literals".
    fn static_literal(
        &mut self,
        target: TypeId,
        e: &'tu Expression<'tu>,
        whole: bool,
    ) -> (Option<&'tu Expression<'tu>>, Option<&'tu Expression<'tu>>) {
        let peel = if whole || self.context.configuration.folds_compound_literals() {
            Peel::Whole
        } else {
            Peel::Element
        };
        let literal = self.whole_compound_literal(target, e, peel);
        let converted = if whole || literal.is_some() {
            None
        } else {
            self.converted_element_literal(target, e)
        };
        (literal, converted)
    }

    /// Checks the compound literal `literal`, which
    /// [`Self::whole_compound_literal`] found in `e`, as the initializer of a
    /// static object, which GCC and Clang accept as if its braced list were
    /// written in its place. GCC reports it under `-pedantic` ("initializer
    /// element is not constant"); Clang, whose evaluator reads the literal,
    /// reports nothing, even under `-pedantic-errors`. Each value the
    /// literal's plan stores is checked against its destination as the
    /// static object's own would be. A literal in a block is an automatic
    /// object (§6.5.2.5p6), so neither it nor an array literal nested in it
    /// gives an address constant (§6.6p9).
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.6 paragraph
    /// 10, p. 96; PDF p. 108 permits other forms of constant expression;
    /// §6.5.2.5 paragraph 6, p. 75; PDF p. 87.
    fn check_static_compound_literal(
        &mut self,
        e: &'tu Expression<'tu>,
        literal: &'tu Expression<'tu>,
    ) {
        if !self.context.configuration.folds_compound_literals() {
            self.context.report_extension(
                crate::configuration::Feature::ConstantCompoundLiteral,
                "compound literal as a constant initializer",
                e.source_vectors,
            );
        }
        if let ExpressionType::CompoundLiteral { initializer, .. } = literal.kind {
            self.check_static_plan(initializer);
        }
    }

    /// Copies the compound literal of array type, possibly parenthesized,
    /// that initializes the array `target` at `site` as if its braced list
    /// were written in its place, and returns the number of elements it
    /// has, which completes an array of unknown size. GCC requires static
    /// storage, and rejects an automatic array ("array initialized from
    /// non-constant array expression", or "invalid initializer" for a
    /// literal with side effects); Clang accepts any storage, but only a
    /// literal without side effects, and reports the extension under
    /// `-Wgnu-compound-literal-initializer`. Any other array expression is
    /// no initializer of an array, which the caller reports.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.7.8
    /// paragraph 16, p. 126; PDF p. 138.
    fn copy_array_literal(
        &mut self,
        plan: &mut PlanBuilder<'tu, 's>,
        target: TypeId,
        e: &'tu Expression<'tu>,
        site: &Site<'s>,
        static_storage: bool,
    ) -> Option<u64> {
        let (literal, length) = self.matching_array_literal(target, e)?;
        let gcc = matches!(
            self.context.configuration.compiler_identity(),
            crate::configuration::CompilerIdentity::Gcc
        );
        let side_effects = self.has_side_effects(literal);
        if gcc && !static_storage || !gcc && side_effects {
            // GCC wraps a literal with side effects, which is then no
            // literal ("invalid initializer").
            let kind = if gcc && side_effects {
                SemanticErrorKind::InvalidInitializer
            } else {
                SemanticErrorKind::NonConstantArrayInitializer
            };
            self.error(kind, e.source_vectors, None, None);
        } else if !gcc {
            self.context.report_extension(
                crate::configuration::Feature::ArrayCompoundLiteral,
                "array initialized from a compound literal",
                e.source_vectors,
            );
        }
        if static_storage {
            self.check_static_compound_literal(e, literal);
        }
        let ty = match self.types.nodes[target.index] {
            | TypeKind::Array(element, ArrayBound::Incomplete) => self
                .types
                .intern(TypeKind::Array(element, ArrayBound::Constant(length))),
            | _ => target,
        };
        plan.copy(literal, site.destination(ty), self.enclosing_unions(site));
        Some(length)
    }

    /// The compound literal, possibly parenthesized, of an array type that
    /// may initialize the array `target`, and its number of elements. GCC
    /// requires array types compatible once each element type's qualifiers
    /// are dropped (`digest_init` compares main variants). Clang requires
    /// the same element type, and the same bound unless `target`'s is
    /// unknown (`hasCompatibleArrayTypes`).
    /// GNU extension: GCC manual, "Compound Literals".
    fn matching_array_literal(
        &mut self,
        target: TypeId,
        e: &'tu Expression<'tu>,
    ) -> Option<(&'tu Expression<'tu>, u64)> {
        let TypeKind::Array(target_element, target_bound) = self.types.nodes[target.index] else {
            return None;
        };
        let literal = unparenthesized(e);
        if !matches!(literal.kind, ExpressionType::CompoundLiteral { .. }) {
            return None;
        }
        let ty = self.expression_info(literal).ty;
        let TypeKind::Array(element, ArrayBound::Constant(length)) = self.types.nodes[ty.index]
        else {
            return None;
        };
        let bounds = match target_bound {
            | ArrayBound::Incomplete => true,
            | ArrayBound::Constant(target_length) => target_length == length,
            | _ => false,
        };
        let compatible = bounds
            && if matches!(
                self.context.configuration.compiler_identity(),
                crate::configuration::CompilerIdentity::Gcc
            ) {
                let [a, b] = [target_element, element].map(|ty| self.element_main_variant(ty));
                self.types.composite(a, b).is_some()
            } else {
                target_element == element
            };
        compatible.then_some((literal, length))
    }

    /// `ty` without the qualifiers, `_Atomic` included, of its innermost
    /// element type, which GCC's main variant of an array type drops.
    fn element_main_variant(&mut self, mut ty: TypeId) -> TypeId {
        let mut bounds = ArenaVec::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = self.types.nodes[ty.index] {
            bounds.push(bound);
            ty = element;
        }
        let mut ty = self.types.non_atomic(ty).unqualified();
        while let Some(bound) = bounds.pop() {
            ty = self.types.intern(TypeKind::Array(ty, bound));
        }
        ty
    }

    /// The compound literal whose bytes `e` gives an object of type
    /// `target`: `e` is the literal, possibly parenthesized, and `target` is
    /// a structure or union, or a scalar type compatible with the
    /// literal's, qualifiers aside (GCC rejects `static double d =
    /// (int){3};`). `peel` adds the wrappers GCC's folder removes before
    /// it checks a static initializer; GCC folds no other operator away, so
    /// `(int){3} + 1`, `(int)(double)(int){3}` and `(unsigned)(int){3}` for
    /// an `int` are not literals here.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.5.3.2
    /// paragraph 4, p. 79; PDF p. 91 (`*&`); §6.5.4 paragraph 4, p. 81;
    /// PDF p. 93 (casts); §6.5.15 paragraph 4, p. 90; PDF p. 102 (`?:`).
    fn whole_compound_literal(
        &mut self,
        target: TypeId,
        e: &'tu Expression<'tu>,
        peel: Peel,
    ) -> Option<&'tu Expression<'tu>> {
        let target = self.types.non_atomic(target);
        let record = matches!(
            self.types.nodes[target.index],
            TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
        );
        let (e, reference) = self.peel_compound_literal(target, e, peel)?;
        let literal = self.expression_info(e).ty;
        let fits = record || self.compatible_value(literal, target);
        let fits = match reference {
            // Clang reads an atomic literal only as a whole initializer.
            | Some(_) if self.context.configuration.folds_compound_literals() =>
                fits && self.types.non_atomic(literal) == literal,
            | Some(reference) => self.exact_literal_type(e) && literal.index == reference.index,
            | None => fits,
        };
        fits.then_some(e)
    }

    /// The compound literal inside the wrappers `peel` allows around `e`,
    /// the initializer of an object of type `target`, and the reference
    /// type of the outermost cast or `?:` among them, if any.
    fn peel_compound_literal(
        &mut self,
        target: TypeId,
        mut e: &'tu Expression<'tu>,
        peel: Peel,
    ) -> Option<(&'tu Expression<'tu>, Option<TypeId>)> {
        let target = self.types.non_atomic(target);
        let record = matches!(
            self.types.nodes[target.index],
            TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
        );
        let scalar = !record && self.scalar_type(target);
        if !record && !scalar {
            return None;
        }
        // GCC folds a pair of conversions away only where the literal has
        // exactly the type of the outermost cast or `?:` around it, its
        // reference type, which must be the object's own type, qualifiers
        // aside: it never folds the implicit conversion to the object's
        // type. Clang's evaluator reads through any conversion that keeps
        // the value.
        let clang = self.context.configuration.folds_compound_literals();
        let mut reference: Option<TypeId> = None;
        loop {
            e = match e.kind {
                | ExpressionType::Parenthesized { expression } => expression,
                | ExpressionType::CompoundLiteral { .. } => break,
                | _ if peel == Peel::Parentheses => return None,
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression,
                } => operand_expression,
                | ExpressionType::Generic(_) => self.expression_info(e).selected_expression?,
                | ExpressionType::Builtin(builtin)
                    if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                    self.expression_info(e).selected_expression?,
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } => {
                    let mut address = operand_expression;
                    while let ExpressionType::Parenthesized { expression } = address.kind {
                        address = expression;
                    }
                    let ExpressionType::Unary {
                        operator: UnaryOperator::AddressOf,
                        operand_expression,
                    } = address.kind
                    else {
                        return None;
                    };
                    operand_expression
                },
                | _ if peel == Peel::Element => return None,
                | ExpressionType::Cast {
                    operand_expression, ..
                } => {
                    let ty = self.expression_info(e).ty;
                    let peeled = match reference {
                        | Some(reference) => self.round_trips(ty, reference),
                        | None if clang => self.round_trips(ty, target),
                        | None => self.compatible_value(ty, target),
                    };
                    if !peeled {
                        return None;
                    }
                    _ = reference.get_or_insert(ty);
                    operand_expression
                },
                | ExpressionType::Conditional(c) => {
                    let ty = self.expression_info(e).ty;
                    if !self.compatible_value(ty, reference.unwrap_or(target)) {
                        return None;
                    }
                    _ = reference.get_or_insert(ty);
                    let condition = self.expression_info(c.condition_expression);
                    if condition.constant != ConstantClass::Arithmetic {
                        return None;
                    }
                    if Self::constant_truth(condition)? {
                        c.then_expression
                    } else {
                        c.else_expression
                    }
                },
                | _ => return None,
            };
        }
        Some((e, reference))
    }

    /// Whether the compound literal `literal` has a type GCC's folder
    /// treats as one cast's: written without a typedef name, qualifiers or
    /// `_Atomic`, so that it is the main variant of its type.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.7.7
    /// paragraph 3, p. 123; PDF p. 135 (a typedef name only names a type).
    fn exact_literal_type(&self, literal: &'tu Expression<'tu>) -> bool {
        let ExpressionType::CompoundLiteral { type_name, .. } = literal.kind else {
            return false;
        };
        let ty = self.expression_info(literal).ty;
        !matches!(
            type_name.declaration_specifiers.type_specifiers,
            super::super::parsing::declaration_syntax::TypeSpecifiers::TypedefName(_)
        ) && ty.qualifiers.is_empty()
            && !matches!(self.types.nodes[ty.index], TypeKind::Atomic(_))
    }

    /// Whether values of types `a` and `b` have compatible types, atomic
    /// and other qualifiers aside.
    /// C99: §6.2.7 paragraph 1, p. 37; PDF p. 49.
    fn compatible_value(&mut self, a: TypeId, b: TypeId) -> bool {
        let [a, b] = [a, b].map(|ty| self.types.non_atomic(ty).unqualified());
        self.types.composite(a, b).is_some()
    }

    /// Whether GCC's folder removes the pair of conversions of a value of
    /// type `value` to `wide` and back, which keeps the value: compatible
    /// types, qualifiers aside; integer or pointer types at least as wide
    /// (`_Bool` is one bit wide); or real floating types at least as wide.
    /// GCC compares the floating types' precision in bits, so it also
    /// removes `(_Float16)(__bf16)`, whose value it then keeps unrounded;
    /// under the Clang identity, whose evaluator converts through `wide`,
    /// `wide`'s significand and exponent range must cover `value`'s.
    /// C99: §6.3.1.3 paragraph 1, p. 43; PDF p. 55; §6.3.1.5 paragraph 1,
    /// p. 44; PDF p. 56; §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    fn round_trips(&mut self, wide: TypeId, value: TypeId) -> bool {
        if self.compatible_value(wide, value) {
            return true;
        }
        let [wide, value] = [wide, value].map(|ty| self.types.non_atomic(ty).unqualified());
        let width = |ty| {
            if self.pointer_target(ty).is_some() {
                Some(self.types.target.pointer.size * 8)
            } else {
                self.integer_type(ty).map(|(bits, _)| u64::from(bits))
            }
        };
        if let (Some(wide), Some(value)) = (width(wide), width(value)) {
            return wide >= value;
        }
        let format = |ty: TypeId| match self.types.nodes[ty.index] {
            | TypeKind::Scalar(scalar) => self.floating_format(scalar),
            | _ => None,
        };
        let (Some(wide), Some(value)) = (format(wide), format(value)) else {
            return false;
        };
        if self.context.configuration.folds_compound_literals() {
            wide.digits >= value.digits && wide.max_exponent >= value.max_exponent
        } else {
            wide.bits >= value.bits
        }
    }

    /// The format of a real floating type.
    /// C99: §5.2.4.2.2 paragraphs 8-9, pp. 24-25; PDF pp. 36-37.
    fn floating_format(&self, scalar: Scalar) -> Option<FloatingFormat> {
        let format = |digits, max_exponent, bits| FloatingFormat {
            digits,
            max_exponent,
            bits,
        };
        Some(match scalar.representation() {
            | Scalar::Float16 | Scalar::Fp16 => format(11, 16, 16),
            | Scalar::BFloat16 => format(8, 128, 16),
            | Scalar::Float => format(24, 128, 32),
            | Scalar::Double => format(53, 1024, 64),
            | Scalar::LongDouble if self.types.target.long_double_is_double() =>
                format(53, 1024, 64),
            | Scalar::LongDouble => format(64, 16384, 80),
            | Scalar::Float128 => format(113, 16384, 128),
            | _ => return None,
        })
    }

    /// Gives a scalar compound literal of type `info.ty` the value its
    /// initializer stores, converted to its type, where Clang's evaluator
    /// reads it: an arithmetic, non-volatile literal whose one stored value
    /// is an arithmetic constant folds into the expressions around it in
    /// every mode, at file and block scope, so `(int){3} + 1` initializes a
    /// static object and `enum { E = (int){3} }` folds as
    /// `-Wgnu-folding-constant` reports. The literal stays an lvalue and is
    /// no integer constant expression (§6.6p6). GCC folds no read of a
    /// literal; [`Self::whole_compound_literal`] has the forms it accepts.
    /// C99: §6.5.2.5 paragraph 4, p. 75; PDF p. 87 (the literal's value);
    /// implementation latitude of §6.6 paragraph 10, p. 96; PDF p. 108.
    pub(super) fn fold_compound_literal(
        &mut self,
        info: &mut super::ExpressionInfo<'tu>,
        initializer: &'tu Initializer<'tu>,
    ) {
        let ty = info.ty;
        if self.context.configuration.folds_compound_literals()
            && !ty.qualifiers.contains(TypeQualifiers::VOLATILE)
            && self.literal_value(info, initializer)
        {
            info.folding = ConstantFolding::CompoundLiteral;
        }
    }

    /// Gives `info`, an arithmetic, non-atomic compound literal's record,
    /// the value its initializer stores converted to its type, and returns
    /// whether that value is an arithmetic constant; the literal stays no
    /// integer constant expression.
    /// C99: §6.5.2.5 paragraph 4, p. 75; PDF p. 87; §6.7.8 paragraph 11,
    /// p. 126; PDF p. 138.
    fn literal_value(
        &mut self,
        info: &mut super::ExpressionInfo<'tu>,
        initializer: &'tu Initializer<'tu>,
    ) -> bool {
        let ty = info.ty;
        if !self.arithmetic(ty) || self.types.non_atomic(ty) != ty {
            return false;
        }
        let Some(&plan) = self.initializers.plan(initializer) else {
            return false;
        };
        let value = match plan.operations {
            | [
                InitOperation::Store {
                    value,
                    bit_field: None,
                    ..
                },
            ] => self.expression_info(plan.values[value.0 as usize].expression),
            // C23 (N3220) §6.7.11p11: an empty initializer gives zero.
            | [InitOperation::EmptyScalar { .. }] => {
                let mut zero =
                    Self::expression_result(info.expression, self.types.scalar(Scalar::Int));
                zero.integer = Some(Integer::int(0));
                zero.constant = ConstantClass::Arithmetic;
                zero
            },
            | _ => return false,
        };
        if value.constant != ConstantClass::Arithmetic {
            return false;
        }
        self.fold_conversion(info, ty, value);
        info.ice = false;
        info.constant == ConstantClass::Arithmetic
    }

    /// The arithmetic compound literal of another type than `target`'s
    /// that GCC accepts as an element of a static object's braced list,
    /// inside the wrappers [`Peel::Element`] allows: GCC takes the value
    /// the literal's own initializer gives it and converts that to the
    /// element's type (`static double a[1] = { (int){1} };`), unlike a
    /// whole initializer; Clang's evaluator converts it too, also through
    /// `*&`. The literal's record gets that value, which lowering stores as
    /// the element's.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.7.8
    /// paragraph 11, p. 126; PDF p. 138.
    fn converted_element_literal(
        &mut self,
        target: TypeId,
        e: &'tu Expression<'tu>,
    ) -> Option<&'tu Expression<'tu>> {
        if !self.arithmetic(target) {
            return None;
        }
        let (literal, None) = self.peel_compound_literal(target, e, Peel::Element)? else {
            return None;
        };
        let ExpressionType::CompoundLiteral { initializer, .. } = literal.kind else {
            return None;
        };
        let key = std::ptr::from_ref(literal).addr();
        let &index = self.expression_indices.get(&key)?;
        let mut info = self.expressions[index];
        if !self.literal_value(&mut info, initializer) {
            return None;
        }
        self.expressions[index] = info;
        Some(literal)
    }

    /// Applies [`Self::check_static_initializer`] to every value the plan
    /// of a compound literal's `initializer` stores, and descends into the
    /// compound literals it copies. A literal without a plan was rejected,
    /// with its own diagnostics, or is unmodeled and refused by lowering.
    fn check_static_plan(&mut self, initializer: &'tu Initializer<'tu>) {
        let Some(&plan) = self.initializers.plan(initializer) else {
            return;
        };
        for operation in plan.operations {
            match *operation {
                | InitOperation::Store {
                    value,
                    destination,
                    bit_field,
                    ..
                } => {
                    let e = plan.values[value.0 as usize].expression;
                    let info = self.expression_info(e);
                    self.check_static_initializer(destination.ty, info, e, bit_field);
                },
                | InitOperation::Copy {
                    value, destination, ..
                } => {
                    let e = plan.values[value.0 as usize].expression;
                    // A static plan already copies the literal itself; an
                    // automatic one copies its expression as written.
                    if let Some(nested) =
                        self.whole_compound_literal(destination.ty, e, Peel::Parentheses)
                        && let ExpressionType::CompoundLiteral { initializer, .. } = nested.kind
                    {
                        self.check_static_plan(initializer);
                    } else {
                        let info = self.expression_info(e);
                        self.check_static_initializer(destination.ty, info, e, None);
                    }
                },
                | InitOperation::Zero { .. }
                | InitOperation::Select { .. }
                | InitOperation::String { .. }
                | InitOperation::Repeat { .. }
                | InitOperation::EmptyScalar { .. }
                | InitOperation::Unmodeled { .. } => {},
            }
        }
    }

    /// The object type an initializer completes and the flexible array
    /// member it fills; `Err` holds the result to return without a plan,
    /// when the initializer left the type incomplete or too large.
    /// C99: §6.7.8 paragraph 22, p. 127; PDF p. 139.
    fn complete_initialized_type(
        &mut self,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        count: u64,
        flexible: Option<FlexibleExtent>,
        before: usize,
    ) -> Result<(TypeId, Option<FlexibleArray>), (TypeId, Option<FlexibleArray>)> {
        let flexible = match flexible {
            | Some(extent) if self.semantic_errors == before => {
                if extent.written > 0
                    && !self.validate_array_size(
                        extent.element,
                        i128::from(extent.written),
                        initializer.source_vectors,
                    )
                {
                    return Err((self.types.unknown(), None));
                }
                let mut array = |elements| {
                    self.types.intern(TypeKind::Array(
                        extent.element,
                        ArrayBound::Constant(elements),
                    ))
                };
                Some(FlexibleArray {
                    completed: array(extent.current),
                    written:   array(extent.written),
                })
            },
            | _ => None,
        };
        Ok(
            if let TypeKind::Array(element, ArrayBound::Incomplete) = self.types.nodes[ty.index] {
                if count == 0 {
                    if self.semantic_errors > before {
                        return Err((self.types.unknown(), None));
                    }
                    // An empty list gives an array of unknown size no
                    // elements: GCC and Clang complete it as a zero-length
                    // array, which C forbids (C99 §6.7.5.2p1, p. 116; PDF
                    // p. 128; N3220 §6.7.11p4).
                    // GNU extension: GCC manual, "Arrays of Length Zero".
                    self.context.report_extension(
                        crate::configuration::Feature::ZeroLengthArrays,
                        "zero-length array",
                        initializer.source_vectors,
                    );
                }
                // C99 §6.7.8p22, p. 127; PDF p. 139: completion produces an
                // object type subject to the same size limit as a declared
                // bound.
                if !self.validate_array_size(element, i128::from(count), initializer.source_vectors)
                {
                    return Err((self.types.unknown(), None));
                }
                (
                    self.types
                        .intern(TypeKind::Array(element, ArrayBound::Constant(count))),
                    None,
                )
            } else {
                (ty, flexible)
            },
        )
    }

    /// Only the root array of unknown size (`root`) and the object's own
    /// flexible array member are completed by their initializers; any other
    /// flexible array member takes only an empty list, which stores nothing.
    /// GNU extension: GCC manual, "Arrays of Length Zero".
    fn completion(&self, target: TypeId, root: bool, site: &Site<'_>) -> Completion {
        if !matches!(
            self.types.nodes[target.index],
            TypeKind::Array(_, ArrayBound::Incomplete)
        ) {
            Completion::Complete
        } else if root {
            Completion::Root
        } else if site.own_flexible {
            Completion::Flexible
        } else {
            Completion::Skip
        }
    }

    /// Brace elision may have descended into an element of the flexible
    /// array member of an object of type `root`, so the member's own
    /// position is on the cursor's path.
    fn extend_flexible(&self, flexible: &mut FlexibleExtent, cursor: &Current<'_>, root: TypeId) {
        let mut path = Some(cursor);
        while let Some(current) = path {
            if self.flexible_array_type(current.container, root) {
                flexible.extend(current.index.saturating_add(1));
                break;
            }
            path = current.parent;
        }
    }

    /// Brace elision descends from `cursor` into its subobject `container`,
    /// whose subobject `index` the element initializes next. A vector's
    /// lanes are not placed by the plan. `sources` are the element's and its
    /// expression's provenance.
    /// C99: §6.7.8 paragraph 20, p. 127; PDF p. 139.
    fn plan_descent(
        &self,
        plan: &mut PlanBuilder<'tu, 's>,
        cursor: Current<'s>,
        (container, index): (TypeId, u64),
        zero: u32,
        (source, expression): (SourceVectors, SourceVectors),
    ) -> Current<'s> {
        let at = self
            .plan_child(cursor.container, &cursor.at, cursor.index, container)
            .unwrap_or_else(|| {
                plan.invalidate();
                cursor.at
            });
        if self.vector(container).is_some() {
            plan.unmodeled(
                Subobject {
                    offset: at.offset,
                    ty:     container.qualified(cursor.at.qualifiers),
                },
                expression,
            );
        }
        let cursor = Current {
            container,
            index,
            parent: Some(self.scratch.alloc(cursor)),
            root_index: cursor.root_index,
            at,
        };
        self.plan_container_entry(plan, &cursor, zero, source);
        cursor
    }

    /// The checker's current subobject is subobject `cursor.index` of
    /// `cursor.container`: if that is a union, the element selects that
    /// member (§6.7.8p17), within the list whose zero scope is `zero`.
    fn plan_container_entry(
        &self,
        plan: &mut PlanBuilder<'tu, 's>,
        cursor: &Current<'s>,
        zero: u32,
        source: SourceVectors,
    ) {
        let destination = Subobject {
            offset: cursor.at.offset,
            ty:     cursor.container.qualified(cursor.at.qualifiers),
        };
        if self.union_type(cursor.container) {
            plan.select(destination, cursor.index, zero, source);
        }
    }

    /// The size of an element of the array `ty`.
    fn subobject_stride(&self, ty: TypeId) -> Option<u64> {
        match self.types.nodes[ty.index] {
            | TypeKind::Array(element, _) => self.types.layout(element).map(|layout| layout.size),
            | _ => None,
        }
    }

    /// Member `index` of `container`, if it is a flexible array member.
    /// C99: §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    fn flexible_member(&self, container: TypeId, index: u64) -> Option<Member> {
        let TypeKind::Tag(id) = self.types.nodes[container.index] else {
            return None;
        };
        if self.types.tags[id].kind == TagKind::Enum {
            return None;
        }
        usize::try_from(index)
            .ok()
            .and_then(|index| self.types.tags[id].members.get().get(index))
            .copied()
            .filter(|member| {
                matches!(
                    self.types.nodes[member.ty.index],
                    TypeKind::Array(_, ArrayBound::Incomplete)
                )
            })
    }

    /// Checks an element whose designator `designators[at]` names the
    /// flexible array member `member`: as GCC and Clang require, no further
    /// designator may lead inside it, and its initializer is a
    /// brace-enclosed list or a string literal. Reports each violation as
    /// Clang does and returns whether there was one, so that the element
    /// initializes nothing.
    /// GNU extension: GCC manual, "Arrays of Length Zero".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Zero-Length.html>
    /// C99: §6.7.2.1 paragraph 16, p. 103; PDF p. 115; §6.7.8 paragraph 16,
    /// p. 126; PDF p. 138.
    fn flexible_designation(
        &mut self,
        designators: &[Designator<'tu>],
        at: usize,
        member: Member,
        element: &InitializerElement<'tu>,
    ) -> bool {
        let declared = member
            .name
            .map_or(member.source_vectors, |name| name.source_vectors);
        let mut invalid = false;
        if let Some(inner) = designators.get(at + 1) {
            self.error(
                SemanticErrorKind::FlexibleArrayDesignator,
                inner.source_vectors,
                None,
                Some(declared),
            );
            invalid = true;
        }
        if let InitializerType::AssignmentExpression(e) = element.initializer.kind
            && !matches!(unparenthesized(e).kind, ExpressionType::StringLiteral(_))
        {
            self.error(
                SemanticErrorKind::FlexibleArrayBraces,
                e.source_vectors,
                None,
                Some(declared),
            );
            invalid = true;
        }
        invalid
    }

    /// Records that `element` writes the subobject at `cursor`, through
    /// every element of a range designator's range when `range` gives its
    /// step and lower bound, inside the list whose object has the node
    /// `base`; `inner` holds where writes inside each subobject the
    /// element's designation names begin. Reports what the write overrides
    /// as Clang's `-Winitializer-overrides` does, and returns the
    /// subobject's node. A range of more than [`RANGE_WRITES`] elements
    /// records only its last.
    /// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
    fn record_write(
        &mut self,
        writes: &mut Writes<'_>,
        base: u32,
        cursor: &Current<'_>,
        inner: &[SourceVectors],
        range: Option<(usize, u64)>,
        element: &InitializerElement<'tu>,
    ) -> u32 {
        let mut steps = ArenaVec::new_in(self.scratch);
        let mut path = Some(cursor);
        while let Some(current) = path {
            steps.push(Step {
                index: current.index,
                union: matches!(
                    self.types.nodes[current.container.index],
                    TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Union
                ),
                inner: element.initializer.source_vectors,
            });
            path = current.parent;
        }
        steps.reverse();
        for (step, &source) in steps.iter_mut().zip(inner) {
            step.inner = source;
        }
        let kind = match element.initializer.kind {
            | InitializerType::InitializerList(_) => Write::List,
            | InitializerType::AssignmentExpression(_) => Write::Expression,
        };
        let source = element.initializer.source_vectors;
        let range = range.filter(|&(step, lower)| {
            steps
                .get(step)
                .is_some_and(|upper| upper.index - lower < RANGE_WRITES)
        });
        let (node, found) = writes.write(base, &steps, range, kind, source);
        // Clang reports a replaced union member at the element's
        // designation, a partly replaced expression where the element
        // enters it, and a replaced subobject at the new initializer.
        let reports = [
            found.member.map(|previous| {
                (
                    SemanticErrorKind::InitializerOverride,
                    element.source_vectors,
                    previous,
                )
            }),
            found.partial.map(|(at, previous)| {
                (SemanticErrorKind::PartialInitializerOverride, at, previous)
            }),
            found
                .whole
                .map(|previous| (SemanticErrorKind::InitializerOverride, source, previous)),
        ];
        for (kind, at, previous) in reports.into_iter().flatten() {
            self.group_diagnostic(
                crate::configuration::DiagnosticGroup::InitializerOverrides,
                kind,
                at,
                None,
                Some(previous),
            );
        }
        node
    }

    /// An initializer reached the flexible array member of type `member`.
    /// A static object may initialize its own structure's member (`own`),
    /// and an empty list may initialize any, which Clang accepts as an
    /// extension (`-Wgnu-flexible-array-initializer`); anything else is an
    /// error. Starts counting the own member's elements in `flexible`. A
    /// list or string that initializes the whole member (`whole`) overrides
    /// every earlier initializer of it (§6.7.8p19, p. 127; PDF p. 139), so
    /// the member's extent restarts, as in GCC and Clang; an element that
    /// brace elision reaches overrides only that element.
    /// GNU extension: GCC manual, "Arrays of Length Zero".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Zero-Length.html>
    fn enter_flexible_array(
        &mut self,
        member: TypeId,
        own: bool,
        empty: bool,
        whole: bool,
        source: SourceVectors,
        flexible: &mut Option<FlexibleExtent>,
    ) {
        if !own && !empty {
            self.error(
                SemanticErrorKind::FlexibleArrayInitializer,
                source,
                None,
                None,
            );
            return;
        }
        self.context.report_extension(
            crate::configuration::Feature::FlexibleArrayInitializers,
            "flexible array initialization",
            source,
        );
        if own && let TypeKind::Array(element, _) = self.types.nodes[member.index] {
            let written = flexible.map_or(0, |extent| extent.written);
            let current = match flexible {
                | Some(extent) if !whole => extent.current,
                | _ => 0,
            };
            *flexible = Some(FlexibleExtent {
                element,
                current,
                written,
            });
        }
    }

    /// Whether `container` is the flexible array member an initializer of an
    /// object of type `root` is giving elements to: an array of unknown size
    /// other than the object itself.
    fn flexible_array_type(&self, container: TypeId, root: TypeId) -> bool {
        container.index != root.index
            && matches!(
                self.types.nodes[container.index],
                TypeKind::Array(_, ArrayBound::Incomplete)
            )
    }

    /// Checks declaration initialization, completes array types and retains
    /// constant values.
    /// C99: §6.7.8 paragraphs 3-5, p. 125; PDF p. 137.
    /// C99: §6.7.8 paragraph 22, p. 127; PDF p. 139.
    /// C99: §6.7 paragraph 7, p. 98; PDF p. 110.
    /// C99: §6.6 paragraph 10, p. 96; PDF p. 108.
    pub(super) fn initialize_declaration(
        &mut self,
        index: usize,
        initializer: &'tu Initializer<'tu>,
    ) {
        let binding = self.bindings[index];
        if initializer.recovered {
            self.bindings[index].ty = self.types.unknown();
            return;
        }
        // C99 §6.7.8p5, p. 125; PDF p. 137.
        if binding.kind == BindingKind::Object
            && binding.linkage != Linkage::None
            && self.scopes[binding.scope].kind != ScopeKind::File
        {
            self.error(
                SemanticErrorKind::LinkedBlockInitializer,
                initializer.source_vectors,
                Some(binding.name.name),
                None,
            );
            return;
        }
        if binding.kind != BindingKind::Object {
            self.error(
                SemanticErrorKind::InvalidInitializer,
                initializer.source_vectors,
                None,
                None,
            );
            return;
        }
        let (ty, flexible) = self.check_initializer(
            binding.ty,
            initializer,
            binding.duration.is_static_or_thread(),
            binding.duration.is_static_or_thread(),
            Some(binding.name.source_vectors),
        );
        self.bindings[index].ty = ty;
        if let Some(flexible) = flexible {
            _ = self.flexible_arrays.insert(index, flexible);
        }
        if !self.types.unanalyzed(ty) && !self.complete_object(ty) {
            self.error(
                SemanticErrorKind::IncompleteObject,
                initializer.source_vectors,
                Some(binding.name.name),
                None,
            );
        }
        // GCC folds a static const, non-volatile integer object's constant
        // initializer where its value is read in GNU modes.
        if binding.duration == Duration::Static
            && ty.qualifiers.contains(TypeQualifiers::CONST)
            && !ty.qualifiers.contains(TypeQualifiers::VOLATILE)
            && let Some((bits, signed)) = self.integer_type(ty)
            && let Some(e) = scalar_initializer_expression(initializer)
            && let info = self.expression_info(e)
            && info.constant == ConstantClass::Arithmetic
            && let Some(value) = info.integer
        {
            // C99 §6.3.1.2p1, p. 43; PDF p. 55: _Bool uses truth
            // conversion rather than truncating to its one-bit model.
            self.bindings[index].value = Some(
                if matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool)) {
                    Integer::int(i128::from(value.value != 0)).cast(bits, signed)
                } else {
                    value.cast(bits, signed)
                },
            );
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn aggregate(&self, ty: TypeId) -> bool {
        matches!(
            self.types.nodes[ty.index],
            TypeKind::Array(..) | TypeKind::Vector { .. }
        ) || matches!(self.types.nodes[ty.index], TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum)
    }

    /// Cache immutable array-tail identities, not mutable tag taint. Each
    /// classification still checks the terminal tag's current taint state.
    /// C99: §6.7.8p3, p. 125; PDF p. 137.
    fn initializer_unanalyzed(
        &self,
        mut ty: TypeId,
        leaves: &mut ArenaMap<'_, usize, TypeId>,
    ) -> bool {
        let mut path = ArenaVec::new_in(self.scratch);
        loop {
            #[cfg(test)]
            self.review_step(2);
            if let Some(&leaf) = leaves.get(&ty.index) {
                ty = leaf;
                break;
            }
            if let TypeKind::Array(element, _) = self.types.nodes[ty.index] {
                path.push(ty.index);
                ty = element;
            } else {
                break;
            }
        }
        for index in path {
            _ = leaves.insert(index, ty);
        }
        self.types.unanalyzed(ty)
    }

    /// Recognizes string and compatible record expressions that initialize a
    /// whole object.
    /// C99: §6.7.8 paragraphs 13-15, p. 126; PDF p. 138.
    fn whole_object_expression(&mut self, target: TypeId, e: &'tu Expression<'tu>) -> bool {
        if let ExpressionType::StringLiteral(_) = unparenthesized(e).kind {
            // C99 §6.7.8p15: wchar_t is target-specific (unsigned short
            // on Windows), including when braces are elided in aggregates.
            // C11 §6.7.9p15: so are char16_t and char32_t arrays, which a
            // `u` or `U` literal initializes.
            let TypeKind::Array(element, _) = self.types.nodes[target.index] else {
                return false;
            };
            return match self.types.nodes[element.index] {
                | TypeKind::Scalar(Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar) =>
                    true,
                | TypeKind::Scalar(scalar) =>
                    scalar == self.types.target.wchar_t
                        || matches!(scalar, Scalar::UnsignedShort | Scalar::UnsignedInt),
                | _ => false,
            };
        }
        if self.vector_assignment(target, self.expression_info(e).ty) {
            return true;
        }
        // GCC: an array compound literal initializes an array element or
        // member whole, as it does the object (Clang only the object).
        if matches!(
            self.context.configuration.compiler_identity(),
            crate::configuration::CompilerIdentity::Gcc
        ) && self.matching_array_literal(target, e).is_some()
        {
            return true;
        }
        matches!(self.types.nodes[target.index], TypeKind::Tag(_))
            && self
                .types
                .composite(
                    target.unqualified(),
                    self.expression_info(e).ty.unqualified(),
                )
                .is_some()
    }

    /// Whether the sole element `e` of a braced list initializes the whole
    /// array `target` as a string: a string literal of the array's own
    /// character or wide kind. Any other expression, or a string that does
    /// not fit, initializes the first element instead.
    /// C99: §6.7.8 paragraphs 14-15 and 17, pp. 126-127; PDF pp. 138-139.
    fn braced_string_fits(&mut self, target: TypeId, e: &'tu Expression<'tu>) -> bool {
        let ExpressionType::StringLiteral(value) = unparenthesized(e).kind else {
            return false;
        };
        let TypeKind::Array(element, _) = self.types.nodes[target.index] else {
            return false;
        };
        let (literal_element, _) = self.string_type(value);
        self.string_fits(element, value, literal_element)
    }

    /// Whether a string literal `value`, whose elements have type
    /// `literal_element`, may initialize an array of `element`.
    /// A character array takes a character or UTF-8 literal (C23 `char8_t`
    /// included); a wide array the literal of its own element type.
    /// C11: §6.7.9 paragraphs 14-15, p. 140; PDF p. 158.
    fn string_fits(
        &self,
        element: TypeId,
        value: StringTokenType,
        literal_element: TypeId,
    ) -> bool {
        if matches!(
            value,
            StringTokenType::String(_) | StringTokenType::EncodedString(_, LiteralEncoding::Utf8)
        ) {
            matches!(
                self.types.nodes[element.index],
                TypeKind::Scalar(Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar)
            )
        } else {
            element.unqualified() == literal_element
        }
    }

    /// C99: §6.7.8p14-15, p. 126; PDF p. 138; §6.5.1p5,
    /// p. 69; PDF p. 81. Parentheses retain the literal's type and value.
    fn initialize_string(&mut self, target: TypeId, e: &'tu Expression<'tu>) -> Option<u64> {
        let ExpressionType::StringLiteral(value) = unparenthesized(e).kind else {
            return None;
        };
        let TypeKind::Array(element, bound) = self.types.nodes[target.index] else {
            return None;
        };
        // An array of arrays receives the string through brace elision into
        // its first element instead (§6.7.8p20).
        if self.aggregate(element) {
            return None;
        }
        let (literal_element, count) = self.string_type(value);
        if !self.string_fits(element, value, literal_element) {
            self.error(
                SemanticErrorKind::InvalidInitializer,
                e.source_vectors,
                None,
                None,
            );
            return Some(count);
        }
        if let ArrayBound::Constant(extent) = bound
            && extent < count - 1
        {
            // Only the characters that fit are stored.
            self.group_diagnostic(
                crate::configuration::DiagnosticGroup::ExcessInitializers,
                SemanticErrorKind::ExcessString,
                e.source_vectors,
                None,
                None,
            );
        }
        Some(count)
    }

    /// Selects the first initializable member, skipping unnamed bit-fields.
    /// C99: §6.7.8 paragraph 9, p. 126; PDF p. 138.
    /// C99: §6.7.8 paragraph 17, pp. 126-127; PDF pp. 138-139.
    fn first_subobject(&self, ty: TypeId) -> u64 {
        if let TypeKind::Tag(id) = self.types.nodes[ty.index]
            && self.types.tags[id].kind != TagKind::Enum
        {
            self.types.tags[id]
                .members
                .get()
                .iter()
                .position(Member::initializable)
                .map_or(u64::MAX, |n| n as u64)
        } else {
            0
        }
    }

    /// Selects an available array element or record member for initialization.
    /// C99: §6.7.8 paragraphs 2-3, p. 125; PDF p. 137.
    /// C99: §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    /// C99: §6.7.8 paragraph 9, p. 126; PDF p. 138.
    fn subobject(&self, ty: TypeId, index: u64) -> Option<TypeId> {
        match self.types.nodes[ty.index] {
            | TypeKind::Vector { element, count, .. } => (index < count).then_some(element),
            | TypeKind::Array(element, bound) =>
                if matches!(bound, ArrayBound::Constant(n) if index >= n)
                    || matches!(bound, ArrayBound::Variable(_) | ArrayBound::Star)
                {
                    None
                } else {
                    Some(element)
                },
            | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum => self.types.tags[id]
                .members
                .get()
                .get(usize::try_from(index).ok()?)
                .filter(|m| m.initializable())
                .map(|m| m.ty),
            | _ => (index == 0).then_some(ty),
        }
    }

    /// Advances the current object in member/element order after
    /// initialization.
    /// C99: §6.7.8 paragraphs 17-20, pp. 126-127; PDF pp. 138-139.
    fn advance_current(&self, mut cursor: Current<'s>) -> Current<'s> {
        loop {
            let mut next = cursor.index.saturating_add(1);
            if let TypeKind::Tag(id) = self.types.nodes[cursor.container.index]
                && self.types.tags[id].kind != TagKind::Enum
            {
                let members = self.types.tags[id].members.get();
                next = if self.types.tags[id].kind == TagKind::Union {
                    members.len() as u64
                } else {
                    members
                        .iter()
                        .enumerate()
                        .skip(usize::try_from(next).unwrap_or(members.len()))
                        .find(|(_, m)| m.initializable())
                        .map_or(members.len() as u64, |(i, _)| i as u64)
                };
            }
            if self.subobject(cursor.container, next).is_some() || cursor.parent.is_none() {
                return Current {
                    index: next,
                    root_index: if cursor.parent.is_none() {
                        next
                    } else {
                        cursor.root_index
                    },
                    ..cursor
                };
            }
            cursor = *cursor.parent.unwrap_or(&cursor);
        }
    }
}

fn unparenthesized<'tu>(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let ExpressionType::Parenthesized { expression } = e.kind {
        e = expression;
    }
    e
}

/// C99: §6.7.8p11, p. 126; PDF p. 138. Scalar braces preserve the
/// initializing expression and its assignment conversion.
fn scalar_initializer_expression<'tu>(
    mut initializer: &'tu Initializer<'tu>,
) -> Option<&'tu Expression<'tu>> {
    loop {
        if initializer.recovered {
            return None;
        }
        match initializer.kind {
            | InitializerType::AssignmentExpression(expression) => return Some(expression),
            | InitializerType::InitializerList(list) => {
                let [element] = list.elements.as_slice() else {
                    return None;
                };
                if element.designation.is_some() {
                    return None;
                }
                initializer = element.initializer;
            },
        }
    }
}

/// One selected subobject; parent cursors preserve continuation after a nested
/// designator or an elided brace. C99: §6.7.8p17-20, pp. 126-127;
/// PDF pp. 138-139.
#[derive(Clone, Copy)]
struct Current<'s> {
    container:  TypeId,
    index:      u64,
    parent:     Option<&'s Current<'s>>,
    root_index: u64,
    /// Where `container` is in the plan.
    at:         Container<'s>,
}

/// The checker's working storage, kept by the analyzer between
/// initializers. Its vectors and maps keep their capacity when cleared, so
/// a translation unit of many initializers grows the scratch arena only to
/// fit the largest one.
pub(super) struct CheckBuffers<'tu, 's> {
    work:   ArenaVec<'s, InitWork<'tu, 's>>,
    writes: Writes<'s>,
    leaves: ArenaMap<'s, usize, TypeId>,
    plan:   PlanBuilder<'tu, 's>,
}

impl<'s> CheckBuffers<'_, 's> {
    fn new_in(scratch: &'s Bump) -> Self {
        Self {
            work:   ArenaVec::new_in(scratch),
            writes: Writes::new_in(scratch),
            leaves: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            plan:   PlanBuilder::new_in(scratch),
        }
    }

    /// Empties every buffer for the initializer at `source`.
    fn reset(&mut self, source: SourceVectors) {
        self.work.clear();
        self.writes.reset(source);
        self.leaves.clear();
        self.plan.reset();
    }
}

/// Resumable brace-list and current-object initialization tasks. Each
/// carries the [`Writes`] node of the subobject it initializes.
/// C99: §6.7.8 paragraphs 17-20, pp. 126-127; PDF pp. 138-139.
#[derive(Clone, Copy)]
enum InitWork<'tu, 's> {
    /// A target type, its initializer, its node, and where the plan places
    /// it.
    Value(TypeId, &'tu Initializer<'tu>, u32, Site<'s>),
    List(ListWork<'tu, 's>),
}

/// The next element of a brace-enclosed list for an object of type `root`.
#[derive(Clone, Copy)]
struct ListWork<'tu, 's> {
    root:      TypeId,
    list:      &'tu BracedInitializerList<'tu>,
    position:  usize,
    cursor:    &'s Current<'s>,
    /// The list is the declared object's own, whose array it completes.
    infer:     bool,
    /// The [`Writes`] node of the list's object.
    base:      u32,
    /// Where the list's object is in the plan, and its zero scope.
    container: Container<'s>,
    zero:      u32,
    /// Range repeats the previous element opened.
    close:     u32,
    /// For a braced scalar, the scalar's site.
    scalar:    Option<Site<'s>>,
}

/// The extents of the flexible array member a static object's own
/// initializer gives elements: its element type, the extent the latest
/// initializer of the whole member gives it, and the largest extent any
/// initializer of it writes.
/// GNU extension: GCC manual, "Arrays of Length Zero".
#[derive(Clone, Copy)]
struct FlexibleExtent {
    element: TypeId,
    current: u64,
    written: u64,
}

impl FlexibleExtent {
    /// An element of the member up to `extent` is initialized.
    fn extend(&mut self, extent: u64) {
        self.current = self.current.max(extent);
        self.written = self.written.max(self.current);
    }
}

/// The most elements of a GNU range designator whose writes are recorded
/// for `-Winitializer-overrides`; a longer range records only its last
/// element, so it may miss a warning but never invents one.
/// GNU extension: GCC manual, "Designated Initializers".
const RANGE_WRITES: u64 = 1 << 16;

/// The wrappers around a compound literal that still let it give a static
/// object its bytes, as GCC's folder removes them.
/// GNU extension: GCC manual, "Compound Literals".
#[derive(Clone, Copy, PartialEq, Eq)]
enum Peel {
    /// Parentheses only: a literal an automatic literal copies as written.
    Parentheses,
    /// Also `__extension__`, `*&` and the operand `_Generic` or
    /// `__builtin_choose_expr` selects: an element of a braced list, where
    /// GCC keeps casts and `?:` (`{ (int)(int){3} }` is not constant).
    Element,
    /// Also an outermost cast to the object's own type, casts inside it
    /// through types that keep every value of that type
    /// (`(double)(double){0x1p20}`, `(int)(long)(int){3}`, but not
    /// `(double)(float){1}` for a `float`), and the operand a `?:` of the
    /// object's type with a constant condition selects (`1 ? (short){3} :
    /// (short)4` is an `int`): an initializer without braces.
    Whole,
}

/// The characteristics of a real floating format that decide whether a
/// conversion through it keeps a value: `DBL_MANT_DIG`, `DBL_MAX_EXP` and,
/// as GCC compares them, the precision in bits.
/// C99: §5.2.4.2.2 paragraphs 8-9, pp. 24-25; PDF pp. 36-37.
#[derive(Clone, Copy)]
struct FloatingFormat {
    digits:       u32,
    max_exponent: u32,
    bits:         u32,
}
