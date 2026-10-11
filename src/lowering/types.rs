//! C types as lowering holds them: which become IR values and of what type,
//! which live only in memory, and their sizes and alignments from the
//! target's data model.

use super::{
    Construct,
    LoweringErrorKind,
};
use crate::{
    ir::{
        Align,
        Type,
    },
    target::{
        Layout,
        Scalar,
        Target,
    },
    translation_phases::{
        parsing::declaration_syntax::TypeQualifiers,
        semantic_analysis::{
            ArrayBound,
            TagKind,
            TypeId,
            TypeKind,
            Types,
        },
    },
};

/// How lowering holds a value of a C type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Repr {
    Void,
    /// An integer type other than `_Bool`, with its C signedness.
    /// C99: §6.2.5 paragraphs 4-8, pp. 33-34; PDF pp. 45-46.
    Int {
        ty:     Type,
        signed: bool,
    },
    /// `_Bool`: an `i8` holding 0 or 1.
    /// C99: §6.2.5 paragraph 2, p. 33; PDF p. 45.
    Bool,
    Pointer,
    /// An array, structure or union: an object in memory, never a value.
    Aggregate,
    Function,
}

/// The representation of `ty`, or why lowering cannot handle it.
pub(super) fn repr(types: &Types<'_>, ty: TypeId) -> Result<Repr, LoweringErrorKind> {
    let unsupported = |construct| Err(LoweringErrorKind::Unsupported(construct));
    match types.kind(ty) {
        | TypeKind::Unknown => Err(LoweringErrorKind::UnanalyzedType),
        | TypeKind::Scalar(scalar) => scalar_repr(types, scalar),
        | TypeKind::Pointer(_) => Ok(Repr::Pointer),
        | TypeKind::Array(element, _) =>
            if types.unanalyzed(element) {
                Err(LoweringErrorKind::UnanalyzedType)
            } else {
                Ok(Repr::Aggregate)
            },
        | TypeKind::Function { .. } => Ok(Repr::Function),
        | TypeKind::Tag(_) => {
            let tag = types.tag(ty).expect("a tag type names its tag");
            if tag.tainted.get() {
                return Err(LoweringErrorKind::UnanalyzedType);
            }
            match tag.kind {
                | TagKind::Enum => scalar_repr(types, tag.compatible.get()),
                | TagKind::Struct | TagKind::Union => Ok(Repr::Aggregate),
            }
        },
        | TypeKind::Atomic(_) => unsupported(Construct::Atomic),
        | TypeKind::Vector { .. } => unsupported(Construct::Vector),
    }
}

/// C99: §6.2.5 paragraphs 2-11, pp. 33-34; PDF pp. 45-46.
fn scalar_repr(types: &Types<'_>, scalar: Scalar) -> Result<Repr, LoweringErrorKind> {
    match scalar {
        | Scalar::Void => Ok(Repr::Void),
        | Scalar::Bool => Ok(Repr::Bool),
        | Scalar::Float | Scalar::Double | Scalar::LongDouble | Scalar::Float128 =>
            Err(LoweringErrorKind::Unsupported(Construct::FloatingPoint)),
        | Scalar::ComplexFloat
        | Scalar::ComplexDouble
        | Scalar::ComplexLongDouble
        | Scalar::ComplexFloat128 =>
            Err(LoweringErrorKind::Unsupported(Construct::ComplexArithmetic)),
        | _ => {
            let (bits, signed) =
                types
                    .target
                    .integer(scalar)
                    .ok_or(LoweringErrorKind::MissingFact(
                        "an integer type has no width",
                    ))?;
            Ok(Repr::Int {
                ty: int_type(bits),
                signed,
            })
        },
    }
}

/// The IR integer type of a C integer `bits` wide.
pub(super) fn int_type(bits: u32) -> Type {
    match bits {
        | 0..=8 => Type::I8,
        | 9..=16 => Type::I16,
        | 17..=32 => Type::I32,
        | 33..=64 => Type::I64,
        | _ => Type::I128,
    }
}

impl Repr {
    /// The IR type of a value of this representation, if it has values.
    pub(super) const fn value_type(self) -> Option<Type> {
        match self {
            | Self::Int { ty, .. } => Some(ty),
            | Self::Bool => Some(Type::I8),
            | Self::Pointer => Some(Type::Ptr),
            | Self::Void | Self::Aggregate | Self::Function => None,
        }
    }

    /// Whether it is an integer, `_Bool` or pointer: a C scalar type that
    /// lowering supports.
    /// C99: §6.2.5 paragraph 21, p. 36; PDF p. 48.
    pub(super) const fn is_scalar(self) -> bool {
        matches!(self, Self::Int { .. } | Self::Bool | Self::Pointer)
    }

    pub(super) const fn signed(self) -> bool {
        matches!(self, Self::Int { signed: true, .. })
    }
}

/// The size and alignment of a complete object type; a variable length
/// array has none.
/// C99: §6.5.3.4 paragraph 2, p. 80; PDF p. 92.
pub(super) fn layout(types: &Types<'_>, ty: TypeId) -> Result<Layout, LoweringErrorKind> {
    types.layout(ty).ok_or_else(|| {
        let mut element = ty;
        while let TypeKind::Array(inner, bound) = types.kind(element) {
            if bound == ArrayBound::Variable {
                return LoweringErrorKind::Unsupported(Construct::VariableLengthArray);
            }
            element = inner;
        }
        LoweringErrorKind::MissingFact("an object type has no layout")
    })
}

pub(super) fn align(layout: Layout) -> Align {
    Align::from_bytes(layout.align).unwrap_or(Align::BYTE)
}

pub(super) fn is_volatile(ty: TypeId) -> bool {
    ty.qualifiers.contains(TypeQualifiers::VOLATILE)
}

/// The pointed-to or element type of a pointer or array type.
pub(super) fn target_type(types: &Types<'_>, ty: TypeId) -> Option<TypeId> {
    match types.kind(ty) {
        | TypeKind::Pointer(target) | TypeKind::Array(target, _) => Some(target),
        | _ => None,
    }
}

/// The LLVM data layout of a target: little-endian x86-64 with ELF or COFF
/// symbol mangling, 64-bit `long long`, 16-byte `__int128` and `long
/// double` storage, and a 16-byte aligned stack. The IR records it so the
/// LLVM back end can emit it unchanged.
pub(super) fn data_layout(target: Target) -> &'static str {
    match target {
        | Target::LinuxGnu | Target::LinuxMusl =>
            "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
        | Target::WindowsGnu | Target::WindowsMsvc =>
            "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
    }
}
