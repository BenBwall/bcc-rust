//! Which records wrapping a vector travel whole in a YMM register once AVX is
//! on. The psABI's rule says a 32-byte aggregate does when its first eightbyte
//! is SSE and the others SSEUP, and the two compilers disagree on what else may
//! share the record, so each has its own walk. Both agree on `struct { __m256
//! v; }`, on `union { __m256 v; float f; }` and on leaving `struct { double
//! d[4]; }` in memory; only GCC accepts `struct { __m256 v; int z[0]; }`, and
//! only Clang `union { __m256 v; double d[4]; }`.
//!
//! GCC merges each scalar's class into the record's four eightbytes
//! (`classify_argument` in `gcc/config/i386/i386.cc`). Clang keeps two classes
//! and looks at each field's size (`X86_64ABIInfo::classify` in
//! `clang/lib/CodeGen/Targets/X86.cpp`): a structure's field must fill it, and
//! an array wider than XMM that isn't one vector counts for nothing at all.
//!
//! System V AMD64 psABI §3.2.3, "Classification", rules 1 and 5.

use super::{
    LoweringErrorKind,
    types::{
        self,
        Repr,
        layout,
    },
    vectors,
};
use crate::{
    configuration::RuleSet,
    ir::Type,
    translation_phases::semantic_analysis::{
        ArrayBound,
        TagKind,
        TypeId,
        TypeKind,
        Types,
    },
};

/// The size of the records and vectors AVX passes whole.
const YMM_BYTES: u64 = 32;

/// Whether a value of the record type `ty` travels whole in a YMM register
/// under AVX as the compiler of `rules` passes it.
pub(super) fn record_travels_whole(
    types: &Types<'_>,
    ty: TypeId,
    rules: RuleSet,
) -> Result<bool, LoweringErrorKind> {
    if types.tag(ty).is_none() || layout(types, ty)?.size != YMM_BYTES {
        return Ok(false);
    }
    if rules == RuleSet::Gcc {
        let mut eightbytes = [Class::Empty; 4];
        Ok(gcc_merge(types, ty, 0, &mut eightbytes)?
            && eightbytes == [Class::Sse, Class::SseUp, Class::SseUp, Class::SseUp])
    } else {
        let (low, high) = clang_classify(types, ty, 0)?;
        Ok(low == Class::Sse && high == Class::SseUp)
    }
}

/// The widest vector a value of type `ty` holds by value, in bytes, or 0.
pub(super) fn widest_vector(types: &Types<'_>, ty: TypeId) -> Result<u64, LoweringErrorKind> {
    let ty = types.non_atomic(ty);
    Ok(match types.kind(ty) {
        | TypeKind::Vector { .. } => layout(types, ty)?.size,
        | TypeKind::Array(element, _) => widest_vector(types, element)?,
        | TypeKind::Tag(_) => {
            let mut widest = 0;
            if let Some(tag) = types.tag(ty).filter(|tag| tag.kind != TagKind::Enum) {
                for member in tag.members.get() {
                    if !member.invalid {
                        widest = widest.max(widest_vector(types, member.ty)?);
                    }
                }
            }
            widest
        },
        | _ => 0,
    })
}

/// An eightbyte's class, as far as these walks need one: anything else ends
/// in memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Empty,
    Sse,
    SseUp,
    Memory,
}

/// psABI §3.2.3, "Classification", rule 4, for the classes above.
const fn merge(current: Class, new: Class) -> Class {
    match (current, new) {
        | (Class::Empty, other) | (other, Class::Empty) => other,
        | (Class::Memory, _) | (_, Class::Memory) => Class::Memory,
        | (Class::SseUp, Class::SseUp) => Class::SseUp,
        | _ => Class::Sse,
    }
}

/// The scalar `ty`: how many SSE eightbytes it covers, or `Wide` for
/// `__float128`, which is SSE and SSEUP. `None` means memory: an integer, a
/// pointer or an x87 value in a record this wide is memory for both compilers.
fn scalar_class(types: &Types<'_>, ty: TypeId) -> Result<Option<Scalar>, LoweringErrorKind> {
    Ok(match types::repr(types, ty)? {
        | Repr::Float { ty: Type::F128 } => Some(Scalar::Wide),
        | Repr::Float {
            ty: Type::F16 | Type::BF16 | Type::F32 | Type::F64,
        }
        | Repr::Complex {
            component: Type::F16 | Type::F32,
        } => Some(Scalar::Sse(1)),
        | Repr::Complex {
            component: Type::F64,
        } => Some(Scalar::Sse(2)),
        | _ => None,
    })
}

/// What [`scalar_class`] finds.
#[derive(Clone, Copy)]
enum Scalar {
    Sse(u64),
    Wide,
}

/// Merges `ty` at byte `offset` into GCC's four eightbytes, or returns false if
/// it puts the record in memory.
fn gcc_merge(
    types: &Types<'_>,
    ty: TypeId,
    offset: u64,
    eightbytes: &mut [Class; 4],
) -> Result<bool, LoweringErrorKind> {
    let ty = types.non_atomic(ty);
    if types
        .natural_alignment(ty)
        .is_some_and(|align| align > 0 && !offset.is_multiple_of(align))
    {
        return Ok(false);
    }
    Ok(match types.kind(ty) {
        | TypeKind::Scalar(_) => match scalar_class(types, ty)? {
            | Some(Scalar::Wide) =>
                gcc_mark(eightbytes, offset, Class::Sse)
                    && gcc_mark(eightbytes, offset + 8, Class::SseUp),
            | Some(Scalar::Sse(count)) =>
                (0..count).all(|index| gcc_mark(eightbytes, offset + 8 * index, Class::Sse)),
            | None => false,
        },
        | TypeKind::Vector { .. } => {
            let size = layout(types, ty)?.size;
            match vectors::member_leaf(types, ty, RuleSet::Gcc)? {
                | Some(Type::F128) if size <= YMM_BYTES && !int128_lanes(types, ty) =>
                    gcc_mark(eightbytes, offset, Class::Sse)
                        && (8..size)
                            .step_by(8)
                            .all(|byte| gcc_mark(eightbytes, offset + byte, Class::SseUp)),
                | Some(Type::F64) => gcc_mark(eightbytes, offset, Class::Sse),
                | _ => false,
            }
        },
        | TypeKind::Array(element, bound) => {
            let count = match bound {
                | ArrayBound::Constant(count) => count,
                | _ => 0,
            };
            let size = layout(types, element)?.size;
            for index in 0..count {
                if !gcc_merge(types, element, offset + index * size, eightbytes)? {
                    return Ok(false);
                }
            }
            true
        },
        | TypeKind::Tag(_) => {
            let Some(tag) = types.tag(ty).filter(|tag| tag.kind != TagKind::Enum) else {
                return Ok(false);
            };
            for member in tag.members.get() {
                if member.invalid {
                    continue;
                }
                // An unnamed bit-field is padding; a named one is INTEGER data.
                if member.width.is_some() {
                    if member.name.is_some() && member.width != Some(0) {
                        return Ok(false);
                    }
                    continue;
                }
                if !gcc_merge(types, member.ty, offset + member.offset, eightbytes)? {
                    return Ok(false);
                }
            }
            true
        },
        | _ => false,
    })
}

/// Merges `class` into the eightbyte at byte `offset`.
fn gcc_mark(eightbytes: &mut [Class; 4], offset: u64, class: Class) -> bool {
    match eightbytes.get_mut((offset / 8) as usize) {
        | Some(slot) => {
            *slot = merge(*slot, class);
            *slot != Class::Memory
        },
        | None => false,
    }
}

/// Clang's two classes of `ty` at byte `offset`.
fn clang_classify(
    types: &Types<'_>,
    ty: TypeId,
    offset: u64,
) -> Result<(Class, Class), LoweringErrorKind> {
    const MEMORY: (Class, Class) = (Class::Memory, Class::Empty);
    let ty = types.non_atomic(ty);
    let place = |class: Class| {
        if offset < 8 {
            (class, Class::Empty)
        } else {
            (Class::Empty, class)
        }
    };
    Ok(match types.kind(ty) {
        | TypeKind::Scalar(_) => match scalar_class(types, ty)? {
            | Some(Scalar::Wide) => (Class::Sse, Class::SseUp),
            | Some(Scalar::Sse(1)) => place(Class::Sse),
            | Some(Scalar::Sse(_)) => (Class::Sse, Class::Sse),
            | None => MEMORY,
        },
        | TypeKind::Vector { .. } => {
            let size = layout(types, ty)?.size;
            match vectors::member_leaf(types, ty, RuleSet::Clang)? {
                | _ if int128_lanes(types, ty) && size != 16 => MEMORY,
                | Some(Type::F128) if size == 16 || size == YMM_BYTES => (Class::Sse, Class::SseUp),
                | Some(Type::F64) => place(Class::Sse),
                | _ => MEMORY,
            }
        },
        | TypeKind::Array(element, bound) => {
            let count = match bound {
                | ArrayBound::Constant(count) => count,
                | _ => 0,
            };
            let element_size = layout(types, element)?.size;
            let size = count * element_size;
            if types
                .natural_alignment(element)
                .is_some_and(|align| align > 0 && !offset.is_multiple_of(align))
            {
                return Ok(MEMORY);
            }
            // "The only case a 256-bit wide vector could be used is when the
            // array contains a single 256-bit element." Clang leaves the
            // classes it reset, so the array counts for nothing.
            if size > 16 && (size != element_size || size > YMM_BYTES) {
                return Ok((Class::Empty, Class::Empty));
            }
            let (mut low, mut high) = (Class::Empty, Class::Empty);
            for index in 0..count {
                let (element_low, element_high) =
                    clang_classify(types, element, offset + index * element_size)?;
                low = merge(low, element_low);
                high = merge(high, element_high);
                if low == Class::Memory || high == Class::Memory {
                    break;
                }
            }
            clang_post_merge(size, low, high)
        },
        | TypeKind::Tag(_) => {
            let Some(tag) = types.tag(ty).filter(|tag| tag.kind != TagKind::Enum) else {
                return Ok(MEMORY);
            };
            let size = layout(types, ty)?.size;
            if size > YMM_BYTES || tag.contains_flexible.get() {
                return Ok(MEMORY);
            }
            let (mut low, mut high) = (Class::Empty, Class::Empty);
            for member in tag.members.get() {
                if member.invalid {
                    continue;
                }
                if member.width.is_some() {
                    if member.name.is_some() {
                        return Ok(MEMORY);
                    }
                    continue;
                }
                let member_size = layout(types, member.ty)?.size;
                if size > 16 && tag.kind != TagKind::Union && size != member_size {
                    return Ok(MEMORY);
                }
                let at = offset + member.offset;
                if types
                    .natural_alignment(member.ty)
                    .is_some_and(|align| align > 0 && !at.is_multiple_of(align))
                {
                    return Ok(MEMORY);
                }
                let (member_low, member_high) = clang_classify(types, member.ty, at)?;
                low = merge(low, member_low);
                high = merge(high, member_high);
                if low == Class::Memory || high == Class::Memory {
                    break;
                }
            }
            clang_post_merge(size, low, high)
        },
        | _ => MEMORY,
    })
}

/// Clang's `X86_64ABIInfo::postMerge`.
const fn clang_post_merge(size: u64, low: Class, high: Class) -> (Class, Class) {
    if matches!(high, Class::Memory)
        || (size > 16 && !(matches!(low, Class::Sse) && matches!(high, Class::SseUp)))
    {
        return (Class::Memory, Class::Empty);
    }
    if matches!(high, Class::SseUp) && !matches!(low, Class::Sse) {
        return (low, Class::Sse);
    }
    (low, high)
}

/// Whether the vector `ty` has `__int128` lanes, which neither compiler passes
/// in a register unless they fit one.
pub(super) fn int128_lanes(types: &Types<'_>, ty: TypeId) -> bool {
    vectors::shape(types, ty)
        .is_some_and(|shape| matches!(shape.lane, Repr::Int { ty: Type::I128, .. }))
}
