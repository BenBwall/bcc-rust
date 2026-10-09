//! Phase-7 current-object initializer traversal.
//! C99: §6.7.8, pp. 125-130; PDF pp. 137-142. Cursors and brace frames
//! live in arenas; brace elision and designators never recurse on the host
//! stack.

use super::{
    super::parsing::declaration_syntax::BracedInitializerList,
    Analyzer,
    ArenaMap,
    ArenaVec,
    ArrayBound,
    BindingKind,
    ConstantClass,
    DesignatorType,
    Duration,
    Expression,
    ExpressionType,
    FxBuildHasher,
    Initializer,
    InitializerType,
    Integer,
    Linkage,
    Member,
    Scalar,
    ScopeKind,
    SemanticErrorKind,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    expressions::ConversionKind,
};

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
}

#[derive(Clone, Copy)]
enum InitWork<'tu, 's> {
    Value(TypeId, &'tu Initializer<'tu>),
    List(
        TypeId,
        &'tu BracedInitializerList<'tu>,
        usize,
        &'s Current<'s>,
        bool,
    ),
}

impl<'tu> Analyzer<'_, 'tu, '_> {
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
        let ty = self.check_initializer(
            binding.ty,
            initializer,
            binding.duration == Duration::Static,
        );
        self.bindings[index].ty = ty;
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

    /// Applies scalar assignment conversion and aggregate current-object rules.
    /// The returned type completes an array of unknown size from its highest
    /// initialized index. C99: §6.7.8p11-22, pp. 126-128; PDF pp. 138-140.
    pub(super) fn check_initializer(
        &mut self,
        ty: TypeId,
        initializer: &'tu Initializer<'tu>,
        static_storage: bool,
    ) -> TypeId {
        if initializer.recovered || self.types.unanalyzed(ty) {
            return ty;
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
            return ty;
        }
        let mut work = ArenaVec::new_in(self.scratch);
        let mut leaves = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
        let mut count = 0;
        let before = self.semantic_errors;
        let mut unmodeled = false;
        work.push(InitWork::Value(ty, initializer));
        while let Some(task) = work.pop() {
            match task {
                | InitWork::Value(target, init) => {
                    if init.recovered || self.initializer_unanalyzed(target, &mut leaves) {
                        continue;
                    }
                    match init.kind {
                        | InitializerType::AssignmentExpression(e) => {
                            let info = self.expression_info(e);
                            if self.types.unanalyzed(info.ty) {
                                continue;
                            }
                            if let Some(extent) = self.initialize_string(target, e) {
                                if std::ptr::eq(init, initializer) {
                                    count = count.max(extent);
                                }
                                continue;
                            }
                            if matches!(self.types.nodes[target.index], TypeKind::Array(..))
                                || !self.assignment_compatible(target, info)
                            {
                                self.error(
                                    SemanticErrorKind::InvalidInitializer,
                                    e.source_vectors,
                                    None,
                                    None,
                                );
                                continue;
                            }
                            self.convert(e, target.unqualified(), ConversionKind::Assignment);
                            if static_storage {
                                let address = info.constant == ConstantClass::Address
                                    || (info.static_address
                                        && matches!(
                                            self.types.nodes[info.ty.index],
                                            TypeKind::Array(..) | TypeKind::Function { .. }
                                        ));
                                let valid = if matches!(
                                    self.types.nodes[target.index],
                                    TypeKind::Scalar(Scalar::Bool)
                                ) && address
                                {
                                    true
                                } else if self.arithmetic(target) {
                                    info.constant == ConstantClass::Arithmetic
                                        && (info.integer.is_some() || info.floating.is_some())
                                } else {
                                    address || self.null_pointer_constant(info)
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
                        },
                        | InitializerType::InitializerList(list) => {
                            // A character array may have its string enclosed in
                            // braces.
                            if let [element] = list.elements.as_slice()
                                && element.designation.is_none()
                                && let InitializerType::AssignmentExpression(e) =
                                    element.initializer.kind
                                && let Some(extent) = self.initialize_string(target, e)
                            {
                                if std::ptr::eq(init, initializer) {
                                    count = count.max(extent);
                                }
                                continue;
                            }
                            let current = self.scratch.alloc(Current {
                                container:  target,
                                index:      self.first_subobject(target),
                                parent:     None,
                                root_index: 0,
                            });
                            work.push(InitWork::List(
                                target,
                                list,
                                0,
                                current,
                                std::ptr::eq(init, initializer),
                            ));
                        },
                    }
                },
                | InitWork::List(root, list, position, cursor, infer) => {
                    let Some(element) = list.elements.as_slice().get(position) else {
                        continue;
                    };
                    let mut cursor = *cursor;
                    if let Some(designation) = element.designation {
                        let mut selected = root;
                        let mut parent = None;
                        let mut valid = true;
                        let mut unknown_designator = false;
                        for designator in designation.designators {
                            // A member of an anonymous member is reached
                            // through that member (C11 §6.7.2.1p13).
                            let mut path = ArenaVec::new_in(self.scratch);
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
                                // GNU ranges are deliberately not given fabricated C99 meaning.
                                | DesignatorType::Range(_) | DesignatorType::Error => {
                                    unmodeled |=
                                        matches!(designator.kind, DesignatorType::Range(_));
                                    valid = false;
                                    break;
                                },
                            };
                            let Some(index) = index else {
                                valid = false;
                                break;
                            };
                            path.push(index);
                            for &index in &path {
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
                                };
                                parent = Some(&*self.scratch.alloc(cursor));
                                selected = child;
                            }
                            if !valid {
                                break;
                            }
                        }
                        if !valid {
                            unmodeled |= unknown_designator;
                            if !unknown_designator
                                && !designation.recovered
                                && !designation.designators.iter().any(|d| {
                                    matches!(
                                        d.kind,
                                        DesignatorType::Error | DesignatorType::Range(_)
                                    )
                                })
                            {
                                self.error(
                                    SemanticErrorKind::InvalidDesignator,
                                    designation.source_vectors,
                                    None,
                                    None,
                                );
                            }
                            work.push(InitWork::List(
                                root,
                                list,
                                position + 1,
                                self.scratch.alloc(cursor),
                                infer,
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
                        work.push(InitWork::List(
                            root,
                            list,
                            position + 1,
                            self.scratch.alloc(cursor),
                            infer,
                        ));
                        continue;
                    };
                    if infer {
                        count = count.max(cursor.root_index.saturating_add(1));
                    }
                    // Without explicit braces, descend until an expression can
                    // initialize the complete current subobject or a scalar
                    // leaf.
                    if let InitializerType::AssignmentExpression(e) = element.initializer.kind {
                        while self.aggregate(target) && !self.whole_object_expression(target, e) {
                            let next_index = self.first_subobject(target);
                            let Some(child) = self.subobject(target, next_index) else {
                                break;
                            };
                            cursor = Current {
                                container:  target,
                                index:      next_index,
                                parent:     Some(self.scratch.alloc(cursor)),
                                root_index: cursor.root_index,
                            };
                            target = child;
                        }
                    }
                    let next = self.advance_current(cursor);
                    work.push(InitWork::List(
                        root,
                        list,
                        position + 1,
                        self.scratch.alloc(next),
                        infer,
                    ));
                    work.push(InitWork::Value(target, element.initializer));
                },
            }
        }
        if unmodeled {
            return self.types.unknown();
        }
        if let TypeKind::Array(element, ArrayBound::Incomplete) = self.types.nodes[ty.index] {
            if count == 0 {
                return if self.semantic_errors > before {
                    self.types.unknown()
                } else {
                    ty
                };
            }
            // C99 §6.7.8p22, p. 127; PDF p. 139: completion produces an
            // object type subject to the same size limit as a declared bound.
            if !self.validate_array_size(element, i128::from(count), initializer.source_vectors) {
                return self.types.unknown();
            }
            self.types
                .intern(TypeKind::Array(element, ArrayBound::Constant(count)))
        } else {
            ty
        }
    }

    fn aggregate(&self, ty: TypeId) -> bool {
        matches!(self.types.nodes[ty.index], TypeKind::Array(..))
            || matches!(self.types.nodes[ty.index], TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum)
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

    fn whole_object_expression(&mut self, target: TypeId, e: &'tu Expression<'tu>) -> bool {
        if let ExpressionType::StringLiteral(_) = unparenthesized(e).kind {
            return matches!(self.types.nodes[target.index], TypeKind::Array(element, _) if matches!(self.types.nodes[element.index], TypeKind::Scalar(Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar | Scalar::Int)));
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
        let (literal_element, count) = self.string_type(value)?;
        let compatible = if matches!(
            self.types.nodes[literal_element.index],
            TypeKind::Scalar(Scalar::Char)
        ) {
            matches!(
                self.types.nodes[element.index],
                TypeKind::Scalar(Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar)
            )
        } else {
            element.unqualified() == literal_element
        };
        if !compatible {
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

    fn subobject(&self, ty: TypeId, index: u64) -> Option<TypeId> {
        match self.types.nodes[ty.index] {
            | TypeKind::Array(element, bound) =>
                if matches!(bound, ArrayBound::Constant(n) if index >= n)
                    || matches!(bound, ArrayBound::Variable | ArrayBound::Star)
                {
                    None
                } else {
                    Some(element)
                },
            | TypeKind::Tag(id) if self.types.tags[id].kind != TagKind::Enum => self.types.tags[id]
                .members
                .get()
                .get(usize::try_from(index).ok()?)
                .filter(|m| {
                    m.initializable()
                        && !matches!(
                            self.types.nodes[m.ty.index],
                            TypeKind::Array(_, ArrayBound::Incomplete)
                        )
                })
                .map(|m| m.ty),
            | _ => (index == 0).then_some(ty),
        }
    }

    fn advance_current<'s>(&self, mut cursor: Current<'s>) -> Current<'s> {
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
