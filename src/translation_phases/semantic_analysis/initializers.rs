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
    Duration,
    FlexibleArray,
    InitOperation,
    Linkage,
    ScopeKind,
    SemanticErrorKind,
    Subobject,
    expressions::{
        ConstantClass,
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
            },
        },
        preprocessing::{
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
                            // C99 §6.7.8p13: a structure or union initialized
                            // by an expression of its type is copied whole, as
                            // is a static scalar's compound literal of its
                            // type (GNU), whose bytes are the literal's own.
                            if matches!(
                                self.types.nodes[self.types.non_atomic(target).index],
                                TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
                            ) || (static_storage
                                && !bit_field
                                && self.whole_compound_literal(target, e).is_some())
                            {
                                plan.copy(e, destination, self.enclosing_unions(&site));
                            } else {
                                plan.store(
                                    e,
                                    destination,
                                    site.bits,
                                    site.scalar_source.unwrap_or(e.source_vectors),
                                );
                            }
                            if static_storage && !self.check_static_compound_literal(target, e) {
                                self.check_static_initializer(target, info, e, bit_field);
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
        bit_field: bool,
    ) {
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

    /// Checks `e` as the initializer of a static object of type `target`
    /// when it is a compound literal of that type, which GCC accepts as if
    /// its braced list were written in its place (Clang only for a
    /// structure or union), and returns whether it was. A scalar literal
    /// counts only as the whole initializer: GCC rejects `(int){3} + 1`. Each
    /// value the literal's plan stores is checked against its destination as
    /// the static object's own would be. A literal in a block is an automatic
    /// object (§6.5.2.5p6), so neither it nor an array literal nested in it
    /// gives an address constant (§6.6p9).
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.6 paragraph
    /// 10, p. 96; PDF p. 108 permits other forms of constant expression;
    /// §6.5.2.5 paragraph 6, p. 75; PDF p. 87.
    fn check_static_compound_literal(&mut self, target: TypeId, e: &'tu Expression<'tu>) -> bool {
        let Some(initializer) = self.whole_compound_literal(target, e) else {
            return false;
        };
        self.context.report_extension(
            crate::configuration::Feature::ConstantCompoundLiteral,
            "compound literal as a constant initializer",
            e.source_vectors,
        );
        self.check_static_plan(initializer);
        true
    }

    /// The initializer of `e` when it is a (parenthesized) compound literal
    /// that the object of type `target` copies: `target` is a structure or
    /// union, or a scalar type compatible with the literal's, qualifiers
    /// aside (GCC rejects `static double d = (int){3};`).
    fn whole_compound_literal(
        &mut self,
        target: TypeId,
        mut e: &'tu Expression<'tu>,
    ) -> Option<&'tu Initializer<'tu>> {
        let target = self.types.non_atomic(target);
        let record = matches!(
            self.types.nodes[target.index],
            TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum
        );
        let scalar = !record && self.scalar_type(target);
        if !record && !scalar {
            return None;
        }
        let initializer = loop {
            match e.kind {
                | ExpressionType::Parenthesized { expression } => e = expression,
                | ExpressionType::CompoundLiteral { initializer, .. } => break initializer,
                | _ => return None,
            }
        };
        if scalar {
            let literal = self.types.non_atomic(self.expression_info(e).ty);
            _ = self
                .types
                .composite(target.unqualified(), literal.unqualified())?;
        }
        Some(initializer)
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
                    self.check_static_initializer(destination.ty, info, e, bit_field.is_some());
                },
                | InitOperation::Copy {
                    value, destination, ..
                } => {
                    let e = plan.values[value.0 as usize].expression;
                    if let Some(nested) = self.whole_compound_literal(destination.ty, e) {
                        self.check_static_plan(nested);
                    } else {
                        let info = self.expression_info(e);
                        self.check_static_initializer(destination.ty, info, e, false);
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
            self.error(
                SemanticErrorKind::ExcessInitializer,
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
