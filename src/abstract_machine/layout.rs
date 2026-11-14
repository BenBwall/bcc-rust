//! Types as storage: each type's size and alignment on x86-64, structure
//! layout with its padding, and the scalars ("leaves") an object holds. The
//! machine addresses an object by leaf: an int array's leaves are its
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

use super::model::{
    Member,
    Structure,
    Type,
    Value,
};

impl Type {
    /// Size and alignment in bytes, as bcc lays them out for x86-64.
    pub(super) fn size_align(self, structures: &[Structure]) -> (usize, usize) {
        match self {
            | Self::Int => (4, 4),
            | Self::Char => (1, 1),
            | Self::Pointer | Self::StructPointer(_) | Self::CharPointer(_) => (8, 8),
            | Self::Array(count) => (count.saturating_mul(4), 4),
            | Self::CharArray(count) => (count, 1),
            | Self::Struct(id) => structures
                .get(id)
                .map_or((0, 1), |structure| (structure.size, structure.align)),
        }
    }

    /// How many scalars an object of this type holds.
    pub(super) fn leaves(self, structures: &[Structure]) -> usize {
        match self {
            | Self::Array(count) | Self::CharArray(count) => count,
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

    /// The type spelled as C writes it: `int *`, `int[3]`, `struct point`.
    pub(super) fn spelling(self, structures: &[Structure]) -> String {
        let tag = |id: usize| structures.get(id).map_or("?", |structure| &structure.name);
        match self {
            | Self::Int => String::from("int"),
            | Self::Char => String::from("char"),
            | Self::Pointer => String::from("int *"),
            | Self::Array(count) => format!("int[{count}]"),
            | Self::CharArray(count) => format!("char[{count}]"),
            | Self::CharPointer(false) => String::from("char *"),
            | Self::CharPointer(true) => String::from("const char *"),
            | Self::Struct(id) => format!("struct {}", tag(id)),
            | Self::StructPointer(id) => format!("struct {} *", tag(id)),
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
        self.filled(structures, &mut |ty| {
            if ty.is_pointer() {
                Value::null()
            } else {
                Value::Integer(0)
            }
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
            | Self::Array(count) => Value::Array((0..count).map(|_| scalar(Self::Int)).collect()),
            | Self::CharArray(count) =>
                Value::Array((0..count).map(|_| scalar(Self::Char)).collect()),
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
            | Self::Array(count) => (offset < count).then_some(Self::Int),
            | Self::CharArray(count) => (offset < count).then_some(Self::Char),
            | Self::Struct(id) => {
                let (member, within) = member_at(structures.get(id)?, offset)?;
                member.ty.leaf_type(structures, within)
            },
            | scalar => (offset == 0).then_some(scalar),
        }
    }
}

/// Lay out `members` (name and type) as a structure tagged `name`: each
/// member at the next offset its alignment allows, the whole rounded up to
/// the largest alignment.
pub(super) fn structure(
    name: &str,
    members: &[(String, Type)],
    structures: &[Structure],
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
            r#type: ty.spelling(structures),
            offset,
            size,
            padding: 0,
            first,
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
