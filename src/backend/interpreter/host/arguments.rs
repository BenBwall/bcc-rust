//! Where a formatted-output host function finds the values its conversions
//! consume: the call's own variadic operands, or a `va_list` operand.
//!
//! Before ABI lowering the machine resolves a `va_list`'s cursor and hands
//! the host the remaining values ([`HostVaList::Values`]). After it the list
//! is the target's own layout in memory, read here as the target's `va_arg`
//! would: for System V, each integer or pointer from the register save area
//! until `gp_offset` passes the six general registers, each `double` from
//! its vector half until `fp_offset` passes the eight vector registers, then
//! one eightbyte at a time from the overflow area, and each x87 `long
//! double` from a 16-byte aligned pair of eightbytes in the overflow area;
//! for Win64, one 8-byte slot per argument, which holds the address of a
//! `long double` passed by reference. Reading advances a local copy, so the
//! caller's list is left as the C library may leave it: indeterminate.
//!
//! C99: §7.15p3, p. 249; PDF p. 261; §7.19.6.8-§7.19.6.13, pp. 292-294;
//! PDF pp. 304-306 (the `v` functions take a `va_list`). System V psABI
//! §3.5.7, "Variable Argument Lists"; Microsoft x64, "Varargs".

use super::super::{
    memory::Memory,
    trap::{
        Fault,
        UbKind,
    },
    value::RuntimeValue,
};
use crate::{
    abi::{
        Convention,
        sysv::va_list,
    },
    ir::{
        Align,
        Type,
    },
};

/// How a host call's `va_list` operand is read.
#[derive(Clone, Copy, Debug)]
#[expect(
    variant_size_differences,
    reason = "A borrowed slice is the cursor's values; boxing would allocate per call."
)]
pub(crate) enum HostVaList<'c> {
    /// The callee takes no `va_list`.
    None,
    /// The values a pre-ABI cursor has left, in order.
    Values(&'c [RuntimeValue]),
    /// A post-ABI list in this convention's layout.
    Target(Convention),
}

/// What a conversion consumes, which decides where a target list keeps it.
#[derive(Clone, Copy, Debug)]
pub(super) enum ArgumentClass {
    /// An integer conversion of this many bits, after default promotions.
    Integer(u32),
    Double,
    /// An x87 `long double`, through the `L` modifier.
    LongDouble,
    Pointer,
}

/// The remaining arguments of one formatted-output call.
pub(super) enum FormatArguments<'v> {
    Values(std::slice::Iter<'v, RuntimeValue>),
    SystemV {
        gp:       u64,
        fp:       u64,
        overflow: RuntimeValue,
        save:     RuntimeValue,
    },
    Win64 {
        next: RuntimeValue,
    },
}

impl<'v> FormatArguments<'v> {
    /// The arguments a `va_list` operand `list` designates.
    pub(super) fn from_va_list(
        list: RuntimeValue,
        source: HostVaList<'v>,
        memory: &Memory<'_>,
    ) -> Result<Self, Fault> {
        Ok(match source {
            | HostVaList::Values(values) => Self::Values(values.iter()),
            | HostVaList::Target(Convention::Wasm32) =>
                return Err(Fault::Unsupported(
                    super::super::trap::Unsupported::VaListTarget,
                )),
            | HostVaList::Target(Convention::SystemV) => {
                let field =
                    |at: u64| memory.ptr_add(list, RuntimeValue::Int(u128::from(at)), false);
                let offset = |at: u64| -> Result<u64, Fault> {
                    let value = memory.load(Type::I32, field(at), Align::BYTE)?;
                    value
                        .as_int()
                        .and_then(|bits| u64::try_from(bits).ok())
                        .ok_or(UbKind::InvalidVaList.into())
                };
                Self::SystemV {
                    gp:       offset(va_list::GP_OFFSET)?,
                    fp:       offset(va_list::FP_OFFSET)?,
                    overflow: memory.load(
                        Type::Ptr,
                        field(va_list::OVERFLOW_ARG_AREA),
                        Align::BYTE,
                    )?,
                    save:     memory.load(Type::Ptr, field(va_list::REG_SAVE_AREA), Align::BYTE)?,
                }
            },
            | HostVaList::Target(Convention::Win64) => Self::Win64 { next: list },
            | HostVaList::None => return Err(UbKind::InvalidVaList.into()),
        })
    }

    /// The next argument, read as `class` from a target list.
    pub(super) fn next(
        &mut self,
        class: ArgumentClass,
        memory: &Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        let ty = match class {
            | ArgumentClass::Integer(bits) if bits <= 32 => Type::I32,
            | ArgumentClass::Integer(_) => Type::I64,
            | ArgumentClass::Double => Type::F64,
            | ArgumentClass::LongDouble => Type::F80,
            | ArgumentClass::Pointer => Type::Ptr,
        };
        let advance = |pointer: RuntimeValue, bytes: u64| {
            memory.ptr_add(pointer, RuntimeValue::Int(u128::from(bytes)), false)
        };
        let address = match self {
            | Self::Values(values) =>
                return values
                    .next()
                    .copied()
                    .ok_or(UbKind::MissingFormatArgument.into()),
            | Self::SystemV {
                gp,
                fp,
                overflow,
                save,
            } if matches!(class, ArgumentClass::LongDouble) => {
                let address = align_to(memory, *overflow, 16);
                *overflow = advance(address, 16);
                address
            },
            | Self::SystemV {
                gp,
                fp,
                overflow,
                save,
            } => {
                let (offset, limit, step) = if matches!(class, ArgumentClass::Double) {
                    (fp, va_list::SAVE_AREA_SIZE, 16)
                } else {
                    (gp, va_list::GP_AREA_SIZE, 8)
                };
                if *offset + step <= limit {
                    let address = advance(*save, *offset);
                    *offset += step;
                    address
                } else {
                    let address = *overflow;
                    *overflow = advance(address, 8);
                    address
                }
            },
            | Self::Win64 { next } => {
                let address = *next;
                *next = advance(address, 8);
                if matches!(class, ArgumentClass::LongDouble) {
                    memory.load(Type::Ptr, address, Align::BYTE)?
                } else {
                    address
                }
            },
        };
        memory.load(ty, address, Align::BYTE)
    }
}

/// `pointer` rounded up to a multiple of `align` bytes, as `va_arg` aligns
/// the overflow area for a type aligned to 16 (psABI §3.5.7, step 7).
fn align_to(memory: &Memory<'_>, pointer: RuntimeValue, align: u64) -> RuntimeValue {
    let address = pointer.as_pointer().map_or(0, |pointer| pointer.address);
    // A power of two: the distance up to its next multiple, which a guest
    // address near the top of the space cannot overflow.
    let padding = address.wrapping_neg() & (align - 1);
    memory.ptr_add(pointer, RuntimeValue::Int(u128::from(padding)), false)
}
