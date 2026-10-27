//! The Microsoft x64 calling convention, which both Windows targets use.
//!
//! Every argument takes one 8-byte slot, so classification needs only the
//! size: a structure or union of 1, 2, 4 or 8 bytes travels as an integer
//! of that size, and any other is passed by reference to a copy the caller
//! made, which the pre-ABI convention already provides. A result of 1, 2,
//! 4 or 8 bytes comes back in RAX; any other is stored through a hidden
//! pointer passed as the first argument. A `va_list` is a pointer to the
//! next 8-byte slot.
//!
//! Reference: Microsoft Learn, "x64 calling convention"
//! (<https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention>),
//! sections "Parameter passing", "Return values" and "Varargs". Clang also
//! passes an aggregate with a flexible array member by reference whatever
//! its size (`WinX86_64ABIInfo::classify` in
//! `clang/lib/CodeGen/Targets/X86.cpp`), and so does this.

use super::{
    AbiErrorKind,
    plan::{
        ArgPlan,
        Parts,
        ResultPlan,
    },
};
use crate::ir::{
    AggregateFlags,
    AggregateLayout,
    Extension,
    ResultRegisters,
    Type,
};

/// The plan of an aggregate argument: an integer of its size, or the
/// pointer to the caller's copy.
/// Microsoft x64, "Parameter passing": "Structs and unions of size 8, 16,
/// 32, or 64 bits, and __m64 types, are passed as if they were integers of
/// the same size. Structs or unions of other sizes are passed as a pointer
/// to memory allocated by the caller."
pub(super) fn argument(layout: AggregateLayout<'_>) -> ArgPlan<'_> {
    match integer(layout) {
        | Some(ty) => ArgPlan::Registers {
            layout,
            parts: Parts::one(ty),
        },
        | None => ArgPlan::Keep,
    }
}

/// The plan of an aggregate result: RAX, or memory through a hidden
/// pointer.
/// Microsoft x64, "Return values": "User-defined types can be returned by
/// value from global functions and static member functions. To return a
/// user-defined type by value in RAX, it must have a length of 1, 2, 4, 8,
/// 16, 32, or 64 bits. ... Otherwise, the caller must allocate memory for
/// the return value and pass a pointer to it as the first argument."
pub(super) fn result(layout: AggregateLayout<'_>) -> ResultPlan<'_> {
    match integer(layout) {
        | Some(first) => ResultPlan::Registers {
            layout,
            registers: ResultRegisters {
                complex: layout.flags.contains(AggregateFlags::COMPLEX),
                second_offset: 8,
                first,
                second: None,
                align: layout.align,
            },
        },
        | None => ResultPlan::Memory(layout),
    }
}

/// How an integer argument or result narrower than `int` travels: only a
/// `_Bool` is widened, with zeros, to 32 bits; the upper bits of the others
/// are unspecified.
/// Microsoft x64, "Parameter passing", leaves the unused bits of a slot
/// undefined; Clang nevertheless extends `_Bool` and relies on it
/// (`WinX86_64ABIInfo::classify`: "Bool type is always extended to the
/// ABI, other builtin types are not extended").
pub(super) const fn extension(extension: Extension) -> Option<Extension> {
    match extension {
        | Extension::Bool => Some(Extension::Zero),
        | Extension::Signed | Extension::Unsigned => None,
        | Extension::Sign | Extension::Zero => Some(extension),
    }
}

/// The integer type an aggregate travels as, if its size is 1, 2, 4 or 8
/// bytes and it has no flexible array member.
const fn integer(layout: AggregateLayout<'_>) -> Option<Type> {
    if layout.flags.contains(AggregateFlags::FLEXIBLE) {
        return None;
    }
    match layout.size {
        | 1 => Some(Type::I8),
        | 2 => Some(Type::I16),
        | 4 => Some(Type::I32),
        | 8 => Some(Type::I64),
        | _ => None,
    }
}

/// Whether `va_arg` can read `ty` from a slot: any scalar of at most eight
/// bytes. The wider `i128`, `f80` and `f128` are not supported yet.
/// Microsoft x64, "Varargs": each argument, including the variable ones,
/// takes one 8-byte slot.
pub(super) const fn check_va_arg(ty: Type) -> Result<(), AbiErrorKind> {
    match ty {
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Ptr
        | Type::F16
        | Type::BF16
        | Type::F32
        | Type::F64 => Ok(()),
        | _ => Err(AbiErrorKind::VaArgType(ty)),
    }
}
