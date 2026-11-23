//! Types as storage: each type's size and alignment on x86-64, structure
//! layout with its padding, and the scalars ("leaves") an object holds. The
//! machine addresses an object by leaf: an array's leaves are its
//! elements, a structure's are its members' leaves in declaration order, and
//! a scalar is one leaf. A value's shape always mirrors its type, so leaves
//! are counted from the value itself.
//!
//! C99: §6.2.5 paragraph 20, pp. 35-36; PDF pp. 47-48 (structure types).
//! C99: §6.7.2.1 paragraph 12, p. 103; PDF p. 115 (member alignment).
//! C99: §6.7.2.1 paragraph 13, p. 103; PDF p. 115 (member order, no leading
//! padding).
//! C99: §6.7.2.1 paragraph 15, p. 103; PDF p. 115 (trailing padding).

use std::fmt::Write;

use super::{
    floating::Real,
    model::{
        Arithmetic,
        Member,
        Signature,
        Structure,
        Target,
        Type,
        Value,
    },
};

impl Type {
    /// Size and alignment in bytes, as bcc lays them out for x86-64.
    pub(super) fn size_align(self, structures: &[Structure]) -> (usize, usize) {
        match self {
            | Self::Integer(integer) => (integer.bytes(), integer.bytes()),
            | Self::Floating(floating) => (floating.bytes(), floating.bytes()),
            | Self::Pointer(..)
            | Self::StructPointer(_)
            | Self::Indirect(..)
            | Self::FunctionPointer(_)
            | Self::VoidPointer => (8, 8),
            | Self::PointerArray(_, _, count) => (count.saturating_mul(8), 8),
            | Self::Array(element, count) =>
                (count.saturating_mul(element.bytes()), element.bytes()),
            | Self::Struct(id) => structures
                .get(id)
                .map_or((0, 1), |structure| (structure.size, structure.align)),
        }
    }

    /// Whether the type's size or layout holds only on x86-64 Linux: it is,
    /// or holds, a long, an unsigned long or a long double, which are 4, 4
    /// and 8 bytes on x86_64-pc-windows-msvc rather than 8, 8 and 16.
    /// Pointers are 8 bytes and the other scalars the same on both.
    pub(super) fn model_dependent(self, structures: &[Structure]) -> bool {
        let arithmetic = |arithmetic: Arithmetic| match arithmetic {
            | Arithmetic::Integer(integer) => !integer.on_linux().is_empty(),
            | Arithmetic::Floating(floating) => !floating.on_linux().is_empty(),
        };
        match self {
            | Self::Integer(integer) => arithmetic(Arithmetic::Integer(integer)),
            | Self::Floating(floating) => arithmetic(Arithmetic::Floating(floating)),
            | Self::Array(element, _) => arithmetic(element),
            | Self::Struct(id) => structures.get(id).is_some_and(|structure| {
                structure
                    .members
                    .iter()
                    .any(|member| member.ty.model_dependent(structures))
            }),
            | Self::Pointer(..)
            | Self::StructPointer(_)
            | Self::Indirect(..)
            | Self::PointerArray(..)
            | Self::FunctionPointer(_)
            | Self::VoidPointer => false,
        }
    }

    /// How many scalars an object of this type holds.
    pub(super) fn leaves(self, structures: &[Structure]) -> usize {
        match self {
            | Self::Array(_, count) | Self::PointerArray(_, _, count) => count,
            | Self::Struct(id) => structures.get(id).map_or(0, |structure| {
                structure
                    .members
                    .iter()
                    .map(|member| member.ty.leaves(structures))
                    .sum()
            }),
            | _ => 1,
        }
    }

    /// The type spelled as C writes it: `unsigned int *`, `int[3]`, `struct
    /// point`, `int (*)(int, int)`. `signatures` are the program's function
    /// types.
    pub(super) fn spelling(self, structures: &[Structure], signatures: &[Signature]) -> String {
        let named = |id: usize| {
            structures
                .get(id)
                .map_or_else(|| String::from("struct ?"), Structure::spelling)
        };
        // A pointer to a function puts its `*`s, and a table's length, in
        // parentheses between the return type and the parameters (C99
        // §6.7.6p2).
        let function = |signature: usize, declarator: &str| {
            signatures.get(signature).map_or_else(
                || format!("int ({declarator})(void)"),
                |known| {
                    format!(
                        "{} ({declarator})({})",
                        known.returns.spelling(structures, signatures),
                        known.parameter_list(structures, signatures)
                    )
                },
            )
        };
        match self {
            | Self::FunctionPointer(signature) => function(signature, "*"),
            | Self::Indirect(Target::Function(signature), depth) =>
                function(signature, &"*".repeat(usize::from(depth))),
            | Self::PointerArray(Target::Function(signature), depth, count) => function(
                signature,
                &format!("{}[{count}]", "*".repeat(usize::from(depth))),
            ),
            | Self::Integer(integer) => String::from(integer.spelling()),
            | Self::Floating(floating) => String::from(floating.spelling()),
            | Self::Pointer(element, false) => format!("{} *", element.spelling()),
            | Self::Pointer(element, true) => format!("const {} *", element.spelling()),
            | Self::Array(element, count) => format!("{}[{count}]", element.spelling()),
            | Self::Struct(id) => named(id),
            | Self::StructPointer(id) => format!("{} *", named(id)),
            | Self::VoidPointer => String::from("void *"),
            | Self::Indirect(target, depth) => {
                let mut text = target.pointer(1).spelling(structures, signatures);
                for _ in 1..depth {
                    text.push('*');
                }
                text
            },
            | Self::PointerArray(target, depth, count) => format!(
                "{}[{count}]",
                target.pointer(depth).spelling(structures, signatures)
            ),
        }
    }

    /// The value of an object of this type that has no value yet: every
    /// scalar indeterminate.
    pub(super) fn indeterminate(self, structures: &[Structure]) -> Value {
        self.filled(structures, &mut |_| Value::Indeterminate)
    }

    /// The value static initialization gives an object of this type without
    /// an initializer: zero for every arithmetic scalar, a null pointer for
    /// every pointer, member by member (C99 §6.7.8p10).
    pub(super) fn zero(self, structures: &[Structure]) -> Value {
        self.filled(structures, &mut |ty| match ty {
            | Self::Floating(floating) => Value::Floating(Real::zero(floating, false)),
            | ty if ty.is_pointer() => Value::null(),
            | _ => Value::Integer(0),
        })
    }

    /// A value of this type whose scalars come from `leaves`, in order.
    pub(super) fn shaped(self, structures: &[Structure], leaves: &[Value]) -> Value {
        let mut next = leaves.iter();
        self.filled(structures, &mut |_| {
            next.next().cloned().unwrap_or(Value::Indeterminate)
        })
    }

    /// A value of this type with each scalar from `scalar`, given the
    /// scalar's type, in leaf order.
    fn filled(self, structures: &[Structure], scalar: &mut impl FnMut(Self) -> Value) -> Value {
        match self {
            | Self::Array(..) | Self::PointerArray(..) => {
                let (element, count) = (self.element(), self.length().unwrap_or(0));
                Value::Array(
                    (0..count)
                        .map(|_| scalar(element.unwrap_or(Self::INT)))
                        .collect(),
                )
            },
            | Self::Struct(id) => Value::Struct {
                structure: id,
                members:   structures.get(id).map_or_else(Vec::new, |structure| {
                    structure
                        .members
                        .iter()
                        .map(|member| member.ty.filled(structures, scalar))
                        .collect()
                }),
            },
            | scalar_type => scalar(scalar_type),
        }
    }

    /// The type of the scalar at leaf `offset` of an object of this type.
    pub(super) fn leaf_type(self, structures: &[Structure], offset: usize) -> Option<Self> {
        match self {
            | Self::Array(..) | Self::PointerArray(..) => self
                .element()
                .filter(|_| offset < self.length().unwrap_or(0)),
            | Self::Struct(id) => {
                let (member, within) = member_at(structures.get(id)?, offset)?;
                member.ty.leaf_type(structures, within)
            },
            | scalar => (offset == 0).then_some(scalar),
        }
    }
}

impl Signature {
    /// The function type as C writes it: `int (int, int)`.
    pub(super) fn spelling(&self, structures: &[Structure], signatures: &[Signature]) -> String {
        format!(
            "{} ({})",
            self.returns.spelling(structures, signatures),
            self.parameter_list(structures, signatures)
        )
    }

    /// The parameter types, comma-separated, or `void` for none.
    pub(super) fn parameter_list(
        &self,
        structures: &[Structure],
        signatures: &[Signature],
    ) -> String {
        if self.parameters.is_empty() {
            return String::from("void");
        }
        self.parameters
            .iter()
            .map(|parameter| parameter.spelling(structures, signatures))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Lay out `members` (name and type) as a structure tagged `name`: each
/// member at the next offset its alignment allows, the whole rounded up to
/// the largest alignment.
pub(super) fn structure(
    name: &str,
    members: &[(String, Type)],
    structures: &[Structure],
    signatures: &[Signature],
) -> Structure {
    let mut laid: Vec<Member> = Vec::new();
    let (mut end, mut align, mut first): (usize, usize, usize) = (0, 1, 0);
    for (member_name, ty) in members {
        let (size, member_align) = ty.size_align(structures);
        let offset = end.next_multiple_of(member_align);
        if let Some(previous) = laid.last_mut() {
            previous.padding = offset - end;
        }
        laid.push(Member {
            name: member_name.clone(),
            r#type: ty.spelling(structures, signatures),
            offset,
            size,
            padding: 0,
            first,
            enumeration: None,
            ty: *ty,
        });
        first += ty.leaves(structures);
        end = offset + size;
        align = align.max(member_align);
    }
    let size = end.next_multiple_of(align);
    if let Some(last) = laid.last_mut() {
        last.padding = size - end;
    }
    Structure {
        name: String::from(name),
        tagged: true,
        size,
        align,
        members: laid,
    }
}

/// The member of `structure` holding leaf `offset`, and the leaf's offset
/// within that member.
pub(super) fn member_at(structure: &Structure, offset: usize) -> Option<(&Member, usize)> {
    structure
        .members
        .iter()
        .rev()
        .find(|member| member.first <= offset)
        .map(|member| (member, offset - member.first))
}

/// How many scalars `value` holds.
pub(super) fn count(value: &Value) -> usize {
    match value {
        | Value::Array(elements) => elements.len(),
        | Value::Struct { members, .. } => members.iter().map(count).sum(),
        | _ => 1,
    }
}

/// The scalar at leaf `index` of `value`.
pub(super) fn leaf(value: &Value, mut index: usize) -> Option<&Value> {
    match value {
        | Value::Array(elements) => elements.get(index),
        | Value::Struct { members, .. } => {
            for member in members {
                let size = count(member);
                if index < size {
                    return leaf(member, index);
                }
                index -= size;
            }
            None
        },
        | scalar => (index == 0).then_some(scalar),
    }
}

/// The scalar at leaf `index` of `value`, to store into.
pub(super) fn leaf_mut(value: &mut Value, mut index: usize) -> Option<&mut Value> {
    match value {
        | Value::Array(elements) => elements.get_mut(index),
        | Value::Struct { members, .. } => {
            for member in members {
                let size = count(member);
                if index < size {
                    return leaf_mut(member, index);
                }
                index -= size;
            }
            None
        },
        | scalar => (index == 0).then_some(scalar),
    }
}

/// Every scalar of `value`, in leaf order.
pub(super) fn flatten(value: &Value, leaves: &mut Vec<Value>) {
    match value {
        | Value::Array(elements) => leaves.extend(elements.iter().cloned()),
        | Value::Struct { members, .. } =>
            for member in members {
                flatten(member, leaves);
            },
        | scalar => leaves.push(scalar.clone()),
    }
}

/// How the source spells the subobject of `value` (an object's value) that
/// starts at leaf `offset`, after the object's name: `""` for the whole
/// object, `[2]`, `.x`, `.corner.y`, `.a[1]`, or `+1` past a scalar. When
/// `structure` is given, the subobject is that whole structure rather than
/// its first scalar.
pub(super) fn path(
    value: &Value,
    offset: i32,
    structure: Option<usize>,
    structures: &[Structure],
) -> String {
    let mut text = String::new();
    let mut value = value;
    let Ok(mut offset) = usize::try_from(offset) else {
        let _ = write!(text, "{offset:+}");
        return text;
    };
    loop {
        match value {
            | Value::Array(_) => {
                let _ = write!(text, "[{offset}]");
                return text;
            },
            | &Value::Struct {
                structure: id,
                ref members,
            } => {
                if offset == 0 && structure == Some(id) {
                    return text;
                }
                let Some((member, within)) = structures
                    .get(id)
                    .and_then(|layout| member_at(layout, offset))
                    .filter(|(member, within)| *within < member.ty.leaves(structures))
                else {
                    let _ = write!(text, "+{}", usize::from(offset > 0));
                    return text;
                };
                let index = structures[id]
                    .members
                    .iter()
                    .position(|candidate| candidate.name == member.name)
                    .unwrap_or_default();
                let _ = write!(text, ".{}", member.name);
                let Some(next) = members.get(index) else {
                    return text;
                };
                value = next;
                offset = within;
            },
            | _ => {
                if offset != 0 {
                    let _ = write!(text, "+{offset}");
                }
                return text;
            },
        }
    }
}
