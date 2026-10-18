//! Object-size queries retain the designated object and its remaining extent.
//! The explicit walk distinguishes a complete object from the nearest member
//! subobject, and saved VLA extents from unknown pointers. Nothing executes an
//! operand just to obtain a static size.
//! C99: §6.5.6p8, p. 83; PDF p. 95 (pointer positions in an array);
//! §6.5.3.4p2, p. 80; PDF p. 92 (runtime array extents).
//! GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>.

use super::{
    Analyzer,
    BinaryOperator,
    Expression,
    ExpressionType,
    Integer,
    ObjectSize,
    TypeKind,
    UnaryOperator,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn infer_object_size(
        &self,
        expression: &'tu Expression<'tu>,
        mode: u8,
        dynamic: bool,
    ) -> ObjectSize<'tu> {
        self.substituted_object_size(expression, mode, dynamic, &[], &[])
    }

    pub(super) fn substituted_object_size(
        &self,
        expression: &'tu Expression<'tu>,
        mode: u8,
        dynamic: bool,
        parameters: &[Option<usize>],
        arguments: &[&'tu Expression<'tu>],
    ) -> ObjectSize<'tu> {
        self.object_size_substitution(expression, mode, dynamic, parameters, arguments, None)
    }

    pub(super) fn cleanup_object_size(
        &self,
        expression: &'tu Expression<'tu>,
        mode: u8,
        dynamic: bool,
        parameters: &[Option<usize>],
        variable: usize,
    ) -> ObjectSize<'tu> {
        self.object_size_substitution(expression, mode, dynamic, parameters, &[], Some(variable))
    }

    fn object_size_substitution(
        &self,
        expression: &'tu Expression<'tu>,
        mode: u8,
        dynamic: bool,
        parameters: &[Option<usize>],
        arguments: &[&'tu Expression<'tu>],
        cleanup: Option<usize>,
    ) -> ObjectSize<'tu> {
        let mut node = expression;
        let mut lvalue = false;
        // Whether the current array lvalue was reached by decay, so that its
        // elements, not the array as an element of an enclosing one, are
        // what the pointer designates.
        let mut decayed = false;
        // Steps are met outermost first, so a later (inner) step may raise
        // a running offset that an earlier one lowered (`buf + 4 - 2`).
        // C99 §6.5.6p8: only the final position must lie within the object.
        let mut offset = 0_i128;
        let mut runtime_offset = false;
        loop {
            node = self.selected_designator(node);
            let info = self.expression_info(node);
            if !lvalue {
                if matches!(self.types.nodes[info.ty.index], TypeKind::Array(..)) {
                    lvalue = true;
                    decayed = true;
                    continue;
                }
                match node.kind {
                    | ExpressionType::Identifier(_) => {
                        if let Some(index) = parameters
                            .iter()
                            .position(|&p| p.is_some() && p == info.binding)
                        {
                            if let Some(&actual) = arguments.get(index) {
                                node = actual;
                                continue;
                            }
                            if index == 0
                                && let Some(variable) = cleanup
                            {
                                let ty = self.bindings[variable].ty;
                                if !runtime_offset {
                                    if let Some(layout) = self.types.layout(ty) {
                                        return ObjectSize::Constant(remaining(
                                            layout.size,
                                            offset,
                                        ));
                                    }
                                    if dynamic && self.variably_modified(ty) {
                                        return dynamic_remainder(ty, offset);
                                    }
                                }
                                return ObjectSize::Unknown;
                            }
                        }
                        if let Some(binding) = info.binding
                            && let Some(size) = self.extension_parameters.get(&binding)
                            && (size.mode == mode
                                || (size.mode, mode) == (0, 1)
                                || (size.mode, mode) == (3, 2))
                            && offset == 0
                            && !runtime_offset
                        {
                            return ObjectSize::Incoming { binding, offset: 0 };
                        }
                        return ObjectSize::Unknown;
                    },
                    | ExpressionType::Cast {
                        operand_expression, ..
                    } => node = operand_expression,
                    | ExpressionType::Unary {
                        operator: UnaryOperator::AddressOf,
                        operand_expression,
                    } => {
                        node = operand_expression;
                        lvalue = true;
                        decayed = false;
                    },
                    | ExpressionType::Binary {
                        operator:
                            operator @ (BinaryOperator::Addition | BinaryOperator::Subtraction),
                        left_expression,
                        right_expression,
                    } => {
                        let left = self.expression_info(left_expression);
                        let right = self.expression_info(right_expression);
                        let (pointer, index) = if matches!(
                            self.types.nodes[left.ty.index],
                            TypeKind::Pointer(_) | TypeKind::Array(..)
                        ) {
                            (left, right)
                        } else {
                            (right, left)
                        };
                        let constant = index
                            .integer
                            .filter(|_| index.ice)
                            .and_then(Integer::to_i128);
                        let Some(index) = constant else {
                            if dynamic {
                                runtime_offset = true;
                                node = pointer.expression;
                                continue;
                            }
                            return ObjectSize::Unknown;
                        };
                        let (TypeKind::Array(target, _) | TypeKind::Pointer(target)) =
                            self.types.nodes[pointer.ty.index]
                        else {
                            return ObjectSize::Unknown;
                        };
                        let Some(step) = self
                            .types
                            .layout(target)
                            .map(|layout| i128::from(layout.size))
                            .and_then(|size| size.checked_mul(index))
                        else {
                            return ObjectSize::Unknown;
                        };
                        let next = if operator == BinaryOperator::Addition {
                            offset.checked_add(step)
                        } else if std::ptr::eq(pointer.expression, left_expression) {
                            offset.checked_sub(step)
                        } else {
                            None
                        };
                        let Some(next) = next else {
                            return ObjectSize::Unknown;
                        };
                        offset = next;
                        node = pointer.expression;
                    },
                    | _ => return ObjectSize::Unknown,
                }
                continue;
            }
            match node.kind {
                | ExpressionType::Identifier(_) | ExpressionType::StringLiteral(_) => {
                    if dynamic
                        && runtime_offset
                        && (self.types.layout(info.ty).is_some() || self.variably_modified(info.ty))
                    {
                        return ObjectSize::Remainder {
                            base: node,
                            ty:   info.ty,
                        };
                    }
                    if let Some(size) = self.types.layout(info.ty).map(|layout| layout.size) {
                        return ObjectSize::Constant(remaining(size, offset));
                    }
                    if dynamic && self.variably_modified(info.ty) {
                        return dynamic_remainder(info.ty, offset);
                    }
                    return ObjectSize::Unknown;
                },
                | ExpressionType::DirectMember {
                    base_expression,
                    member,
                }
                | ExpressionType::IndirectMember {
                    base_expression,
                    member,
                } => {
                    if mode & 1 != 0 {
                        if runtime_offset {
                            // A root-object remainder could cross this member's
                            // boundary; no separate subobject address is saved.
                            return ObjectSize::Unknown;
                        }
                        if let Some(size) = outside_subobject(mode, offset) {
                            return size;
                        }
                        if offset >= 0 {
                            return self
                                .types
                                .layout(info.ty)
                                .map(|layout| layout.size)
                                .map_or(ObjectSize::Unknown, |size| {
                                    ObjectSize::Constant(remaining(size, offset))
                                });
                        }
                    }
                    let base = self.expression_info(base_expression);
                    let indirect = matches!(node.kind, ExpressionType::IndirectMember { .. });
                    let record = if indirect {
                        self.pointer_target(base.ty)
                    } else {
                        Some(base.ty)
                    };
                    let Some(record) = record else {
                        return ObjectSize::Unknown;
                    };
                    let TypeKind::Tag(id) = self.types.nodes[record.index] else {
                        return ObjectSize::Unknown;
                    };
                    let Some(&index) = self.member_indices.get(&(id, member.name)) else {
                        return ObjectSize::Unknown;
                    };
                    let Some(field) = self.types.tags[id].fields.get().get(index) else {
                        return ObjectSize::Unknown;
                    };
                    let Some(next) = offset.checked_add(i128::from(field.offset)) else {
                        return ObjectSize::Unknown;
                    };
                    offset = next;
                    node = base_expression;
                    lvalue = !indirect;
                },
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    // `&a[i]` of `char a[2][10]` designates an element of `a`,
                    // whose closest subobject is `a` itself; the decayed
                    // `a[i]` designates the inner array's first element.
                    if mode & 1 != 0
                        && decayed
                        && matches!(self.types.nodes[info.ty.index], TypeKind::Array(..))
                    {
                        if runtime_offset {
                            return ObjectSize::Unknown;
                        }
                        if let Some(size) = outside_subobject(mode, offset) {
                            return size;
                        }
                        if offset >= 0 {
                            return self
                                .types
                                .layout(info.ty)
                                .map_or(ObjectSize::Unknown, |layout| {
                                    ObjectSize::Constant(remaining(layout.size, offset))
                                });
                        }
                    }
                    let left = self.expression_info(left_expression);
                    let right = self.expression_info(right_expression);
                    let (pointer, index) = if matches!(
                        self.types.nodes[left.ty.index],
                        TypeKind::Pointer(_) | TypeKind::Array(..)
                    ) {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    let Some(index) = index
                        .integer
                        .filter(|_| index.ice)
                        .and_then(Integer::to_i128)
                    else {
                        if dynamic {
                            runtime_offset = true;
                            node = pointer.expression;
                            lvalue = false;
                            continue;
                        }
                        return ObjectSize::Unknown;
                    };
                    let Some(next) = self
                        .types
                        .layout(info.ty)
                        .map(|layout| i128::from(layout.size))
                        .and_then(|size| size.checked_mul(index))
                        .and_then(|step| offset.checked_add(step))
                    else {
                        return ObjectSize::Unknown;
                    };
                    offset = next;
                    node = pointer.expression;
                    lvalue = false;
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } => {
                    node = operand_expression;
                    lvalue = false;
                },
                | _ => return ObjectSize::Unknown,
            }
        }
    }
}

/// The bytes from a signed `offset` to the end of a `size`-byte object; a
/// position before the object or past its end leaves none, as in Clang.
fn remaining(size: u64, offset: i128) -> u64 {
    u64::try_from(offset).map_or(0, |offset| size.saturating_sub(offset))
}

/// A runtime-sized object's remainder at a constant `offset`.
fn dynamic_remainder(ty: super::TypeId, offset: i128) -> ObjectSize<'static> {
    match u64::try_from(offset) {
        | Ok(offset) => ObjectSize::Dynamic { ty, offset },
        | Err(_) if offset < 0 => ObjectSize::Constant(0),
        | Err(_) => ObjectSize::Unknown,
    }
}

/// A position before a subobject designates no part of it. Clang then
/// answers mode 3 with 0 and mode 1 from the complete object (`None`).
fn outside_subobject(mode: u8, offset: i128) -> Option<ObjectSize<'static>> {
    (offset < 0 && mode == 3).then_some(ObjectSize::Constant(0))
}
