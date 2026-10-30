//! The WebAssembly C ABI ("basic C ABI") that Clang implements for
//! `wasm32`, which both WebAssembly targets use.
//!
//! Scalars travel as WebAssembly values, narrow integers widened to 32 bits
//! with their signedness. A structure or union whose only non-empty member
//! is one scalar ("single-element struct") travels as that scalar, in
//! either direction; an empty one is not passed at all. Every other
//! aggregate argument is a pointer to a copy the caller made (LLVM's
//! `byval`, which the pre-ABI convention already provides), and every
//! other aggregate result is stored through a hidden pointer passed first
//! (`sret`). Complex values are aggregates here.
//!
//! The variable arguments of a call are stored in a buffer in the caller's
//! frame, each at its own alignment in slots of at least four bytes, and
//! the callee receives the buffer's address as one extra parameter after
//! its fixed ones. A `va_list` is a `char *` cursor into that buffer. An
//! aggregate other than a single-element one is passed indirectly there:
//! the buffer holds the address of a caller-owned copy.
//!
//! Reference: the WebAssembly tool conventions, "Basic C ABI"
//! (<https://github.com/WebAssembly/tool-conventions/blob/main/BasicCABI.md>),
//! and Clang's `WebAssemblyABIInfo`
//! (`clang/lib/CodeGen/Targets/WebAssembly.cpp`) with `emitVoidPtrVAArg` for
//! `va_arg`.

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

/// The plan of an aggregate argument: its scalar, nothing, or the pointer
/// to the caller's copy.
/// Clang `WebAssemblyABIInfo::classifyArgumentType`: "Ignore empty
/// structs/unions"; "Lower single-element structs to just pass a regular
/// value"; otherwise `DefaultABIInfo` passes it indirectly (`byval`).
pub(super) fn argument(layout: AggregateLayout<'_>) -> ArgPlan<'_> {
    // A one-lane `float` or `double` vector travels as its lane; another
    // vector by reference (`vectors.rs`).
    if layout.flags.contains(AggregateFlags::VECTOR) {
        return match layout.scalars {
            | [only] => ArgPlan::Registers {
                layout,
                parts: Parts::one(only.ty),
            },
            | _ => ArgPlan::Keep,
        };
    }
    if layout.size == 0 {
        return ArgPlan::Ignore(layout);
    }
    match single_element(layout) {
        | Some(ty) => ArgPlan::Registers {
            layout,
            parts: Parts::one(ty),
        },
        | None => ArgPlan::Keep,
    }
}

/// The plan of an aggregate result: its scalar, nothing, or memory
/// through a hidden pointer.
/// Clang `WebAssemblyABIInfo::classifyReturnType`.
pub(super) fn result(layout: AggregateLayout<'_>) -> ResultPlan<'_> {
    if layout.flags.contains(AggregateFlags::VECTOR) && layout.scalars.is_empty() {
        return ResultPlan::Memory(layout);
    }
    if layout.flags.contains(AggregateFlags::VECTOR) {
        return ResultPlan::Registers {
            layout,
            registers: ResultRegisters {
                complex:       false,
                second_offset: 8,
                first:         layout.scalars[0].ty,
                second:        None,
                align:         layout.align,
            },
        };
    }
    if layout.size == 0 {
        return ResultPlan::Ignore(layout);
    }
    match single_element(layout) {
        | Some(first) => ResultPlan::Registers {
            layout,
            registers: ResultRegisters {
                complex: false,
                second_offset: 8,
                first,
                second: None,
                align: layout.align,
            },
        },
        | None => ResultPlan::Memory(layout),
    }
}

/// The scalar a single-element structure or union travels as, or `None`.
/// Lowering decides single elements as Clang's `isSingleElementStruct`
/// does and records the verdict ([`AggregateFlags::SINGLE_ELEMENT`]): the
/// element is the record's one scalar when one covers it, and otherwise a
/// bit-field, which travels as the integer of its declared type, as large
/// as the record (Clang passes `struct { int x : 3; }` as an `i32`).
pub(super) fn single_element(layout: AggregateLayout<'_>) -> Option<Type> {
    if !layout.flags.contains(AggregateFlags::SINGLE_ELEMENT)
        || layout
            .flags
            .intersects(AggregateFlags::COMPLEX | AggregateFlags::FLEXIBLE)
    {
        return None;
    }
    match layout.scalars {
        | [scalar] if scalar.offset == 0 && u64::from(scalar.ty.bytes()) == layout.size =>
            Some(scalar.ty),
        | _ => match layout.size {
            | 1 => Some(Type::I8),
            | 2 => Some(Type::I16),
            | 4 => Some(Type::I32),
            | 8 => Some(Type::I64),
            | 16 => Some(Type::I128),
            | _ => None,
        },
    }
}

/// How an integer argument or result narrower than `int` travels: widened
/// to 32 bits with its signedness, `_Bool` with zeros.
/// Clang `DefaultABIInfo` extends promotable integer types.
pub(super) const fn extension(extension: Extension) -> Extension {
    match extension {
        | Extension::Signed | Extension::Sign => Extension::Sign,
        | Extension::Unsigned | Extension::Bool | Extension::Zero => Extension::Zero,
    }
}

/// Checks that `va_arg` reads `ty` from the argument buffer.
pub(super) const fn check_va_arg(ty: Type) -> Result<(), AbiErrorKind> {
    match ty {
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::I128
        | Type::Ptr
        | Type::F32
        | Type::F64
        | Type::F128 => Ok(()),
        | _ => Err(AbiErrorKind::VaArgType(ty)),
    }
}

/// The bytes and alignment an argument of type `ty` takes in the variable
/// argument buffer: its own size and alignment, at least four.
/// Clang `emitVoidPtrVAArg` with a four-byte slot that a higher alignment
/// may raise.
pub(crate) const fn va_slot(ty: Type) -> (u64, u64) {
    match ty {
        | Type::I64 | Type::F64 => (8, 8),
        | Type::I128 | Type::F128 => (16, 16),
        | _ => (4, 4),
    }
}
