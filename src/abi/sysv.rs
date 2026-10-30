//! The System V AMD64 calling convention: eightbyte classification of
//! aggregates, argument registers, and `va_arg` over the register save area.
//!
//! An aggregate of at most two eightbytes is classified one eightbyte at a
//! time: each scalar contributes a class to the eightbytes it covers, the
//! classes merge, and a clean-up pass turns impossible combinations into
//! MEMORY. An INTEGER eightbyte travels in a general register and an SSE
//! one in a vector register; a MEMORY aggregate is copied to the stack as an
//! argument and returned through a hidden pointer. An argument whose
//! registers are not all free goes to the stack whole.
//!
//! References: System V Application Binary Interface, AMD64 Architecture
//! Processor Supplement (<https://gitlab.com/x86-psABIs/x86-64-ABI>),
//! version 1.0: §3.2.3 "Parameter Passing" (classification, the merger and
//! its clean-up, register assignment and returns) and §3.5.7 "Variable
//! Argument Lists" (`va_list` and the `va_arg` algorithm). Where the
//! document leaves room, this follows Clang's `X86_64ABIInfo`
//! (`clang/lib/CodeGen/Targets/X86.cpp`), with which the result must link.

use super::{
    AbiErrorKind,
    plan::{
        ArgPlan,
        Parts,
        ResultPlan,
        raised,
    },
};
use crate::ir::{
    AggregateFlags,
    AggregateLayout,
    Align,
    Extension,
    ParamAbi,
    ResultRegisters,
    Signature,
    Type,
};

/// The plan of an aggregate argument: in registers if its classes allow and
/// the registers it needs are still free, else on the stack.
/// psABI §3.2.3: "If there are no registers available for any eightbyte of
/// an argument, the whole argument is passed on the stack."
pub(super) fn argument<'s>(
    layout: AggregateLayout<'s>,
    registers: &mut ArgumentRegisters,
) -> Result<ArgPlan<'s>, AbiErrorKind> {
    // Clang gives the stack slot at least eight bytes' alignment.
    let stack = ArgPlan::Stack {
        layout,
        align: raised(layout, Align::from_bytes(8).expect("8 is a power of two")).align,
    };
    // A vector's registers were chosen by lowering; none means memory.
    if layout.flags.contains(AggregateFlags::VECTOR) && layout.scalars.is_empty() {
        return Ok(stack);
    }
    let classes = match classify(layout) {
        | Classification::Memory => return Ok(stack),
        | Classification::Classes(classes) => classes,
    };
    // An argument with an x87 class is passed in memory.
    if classes.contains(&Class::X87) || classes.contains(&Class::X87Up) {
        return Ok(stack);
    }
    let Some(parts) = parts(layout, classes)? else {
        return Ok(ArgPlan::Ignore(layout));
    };
    let count = |wanted| {
        classes
            .iter()
            .map(|&class| u32::from(class == wanted))
            .sum::<u32>()
    };
    let (gp, sse) = (count(Class::Integer), count(Class::Sse));
    if registers.take(gp, sse) {
        Ok(ArgPlan::Registers { layout, parts })
    } else {
        Ok(stack)
    }
}

/// The plan of an aggregate result: in RAX and RDX, XMM0 and XMM1, or
/// ST0 by its classes, or in memory through a hidden pointer that takes the
/// first general register.
/// psABI §3.2.3, "Returning of Values".
pub(super) fn result<'s>(
    layout: AggregateLayout<'s>,
    registers: &mut ArgumentRegisters,
) -> Result<ResultPlan<'s>, AbiErrorKind> {
    let memory = |registers: &mut ArgumentRegisters| {
        _ = registers.take(1, 0);
        ResultPlan::Memory(layout)
    };
    if layout.flags.contains(AggregateFlags::COMPLEX)
        && layout
            .scalars
            .first()
            .is_some_and(|scalar| scalar.ty == Type::F80)
    {
        return Ok(ResultPlan::Registers {
            layout,
            registers: ResultRegisters {
                complex:       true,
                first:         Type::F80,
                second:        Some(Type::F80),
                second_offset: 16,
                align:         layout.align,
            },
        });
    }
    if layout.flags.contains(AggregateFlags::VECTOR) {
        match layout.scalars {
            | [] => return Ok(memory(registers)),
            // Clang's 32-byte vector result without AVX: XMM0 and XMM1.
            | [low, high] => {
                return Ok(ResultPlan::Registers {
                    layout,
                    registers: ResultRegisters {
                        complex:       false,
                        first:         low.ty,
                        second:        Some(high.ty),
                        second_offset: high.offset,
                        align:         layout.align,
                    },
                });
            },
            | _ => {},
        }
    }
    let classes = match classify(layout) {
        | Classification::Memory => return Ok(memory(registers)),
        | Classification::Classes(classes) => classes,
    };
    // A pointer comes back in RAX like any other INTEGER eightbyte; the
    // return area is memory, so its register is an integer.
    let integer = |ty| if ty == Type::Ptr { Type::I64 } else { ty };
    let registers_of = |first, second: Option<Type>| ResultPlan::Registers {
        layout,
        registers: ResultRegisters {
            complex:       layout.flags.contains(AggregateFlags::COMPLEX),
            second_offset: 8,
            first:         integer(first),
            second:        second.map(integer),
            align:         layout.align,
        },
    };
    if classes == [Class::X87, Class::X87Up] {
        return Ok(registers_of(Type::F80, None));
    }
    if classes.contains(&Class::X87) || classes.contains(&Class::X87Up) {
        return Ok(memory(registers));
    }
    Ok(match parts(layout, classes)? {
        | None => ResultPlan::Ignore(layout),
        | Some(parts) => match parts.types() {
            | &[first] => registers_of(first, None),
            | &[first, second] => registers_of(first, Some(second)),
            | _ => unreachable!("an aggregate has one or two parts"),
        },
    })
}

/// How an integer argument or result narrower than `int` travels: widened
/// to 32 bits by its producer, with its sign for a signed type and with
/// zeros for an unsigned one or `_Bool`.
/// The psABI §3.2.3 only fixes bits 1-7 of a `_Bool` and leaves the upper
/// bits of other narrow integers unspecified, but Clang widens these to 32
/// bits and relies on it on the receiving side
/// (`X86_64ABIInfo::classifyArgumentType` and `classifyReturnType` give
/// promotable integers `ABIArgInfo::getExtend`), so code that links with
/// it must as well.
pub(super) const fn extension(extension: Extension) -> Extension {
    match extension {
        | Extension::Signed | Extension::Sign => Extension::Sign,
        | Extension::Unsigned | Extension::Bool | Extension::Zero => Extension::Zero,
    }
}

/// The IR type of each eightbyte of an aggregate in registers, or `None`
/// when no eightbyte has a class and nothing is passed.
fn parts(layout: AggregateLayout<'_>, classes: [Class; 2]) -> Result<Option<Parts>, AbiErrorKind> {
    let part = |index: usize| part_type(layout, index, classes[index]);
    Ok(match classes {
        | [Class::NoClass, Class::NoClass] => None,
        | [Class::NoClass, _] => return Err(AbiErrorKind::EmptyLowEightbyte),
        | [Class::Sse, Class::SseUp] => Some(Parts::one(Type::F128)),
        | [_, Class::NoClass] => Some(Parts::one(part(0))),
        | _ => {
            // As a pair of registers the parts form a structure whose second
            // field must sit at offset 8, as Clang's
            // `GetX86_64ByValArgumentPair` ensures: a low part narrower than
            // eight bytes before a high part aligned to less than eight is
            // widened.
            let (low, high) = (part(0), part(1));
            let registers = ResultRegisters {
                complex:       false,
                second_offset: 8,
                first:         low,
                second:        Some(high),
                align:         layout.align,
            };
            let low = if registers.valid_layout() {
                low
            } else if low.is_float() {
                Type::F64
            } else {
                Type::I64
            };
            Some(Parts::two(low, high))
        },
    })
}

/// The register type of eightbyte `index` of class `class`: the type of a
/// lone scalar that starts it, else the narrowest integer, or `f32` or `f64`
/// for an SSE eightbyte, that covers its scalars. Upper bits of a register
/// beyond the aggregate are unspecified, so a 3-byte eightbyte may travel
/// as an `i32`.
fn part_type(layout: AggregateLayout<'_>, index: usize, class: Class) -> Type {
    let start = 8 * index as u64;
    let mut end = 0;
    let mut lone = None;
    let mut count = 0;
    for scalar in layout.scalars {
        let (from, to) = (
            u64::from(scalar.offset),
            u64::from(scalar.offset) + u64::from(scalar.ty.bytes()),
        );
        if to <= start || from >= start + 8 {
            continue;
        }
        count += 1;
        end = end.max(to.min(start + 8) - start);
        if from == start && scalar.ty.bytes() <= 8 {
            lone = Some(scalar.ty);
        }
    }
    if count == 1
        && let Some(ty) = lone
        && ty != Type::I1
    {
        return ty;
    }
    match (class, end) {
        | (Class::Sse, 0..=4) => Type::F32,
        | (Class::Sse, _) => Type::F64,
        | (_, 0..=1) => Type::I8,
        | (_, 2) => Type::I16,
        | (_, 3..=4) => Type::I32,
        | _ => Type::I64,
    }
}

/// An eightbyte's class.
/// psABI §3.2.3: INTEGER, SSE, SSEUP, X87, X87UP, `COMPLEX_X87`, `NO_CLASS` and
/// MEMORY. Top-level complex `long double` results take the separate
/// `COMPLEX_X87` path before this ordinary aggregate classification.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    clippy::enum_variant_names,
    reason = "The psABI calls the class of an empty eightbyte NO_CLASS."
)]
pub(crate) enum Class {
    NoClass,
    Integer,
    Sse,
    SseUp,
    X87,
    X87Up,
    Memory,
}

/// The classes of an aggregate's two eightbytes, or MEMORY for the whole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Classification {
    Memory,
    /// The low and high eightbyte; the high one is `NO_CLASS` for an
    /// aggregate of at most eight bytes.
    Classes([Class; 2]),
}

/// Classifies an aggregate by the psABI's algorithm.
///
/// An aggregate with a `__float128` at offset 0 and another member covering
/// its second eightbyte, such as `union { __float128 q; double d[2]; }`,
/// follows the psABI here, as GCC does: two SSE eightbytes travel in two
/// vector registers, or the whole aggregate on the stack. Clang deviates:
/// its IR type for the aggregate starts with an `fp128`, which
/// `GetX86_64ByValArgumentPair` widens to `i64`, so it passes the low
/// eightbyte in a general register while counting a vector one, and its own
/// `va_arg` reads both from the vector save area. Code built by Clang does
/// not link with GCC's or bcc's for such aggregates; `middle-end.md` lists
/// the shapes.
///
/// An aggregate larger than two eightbytes is MEMORY, since without vector
/// types no larger one is passed in registers; so is one with a misaligned
/// member or a flexible array member, as Clang decides, or with a one-lane
/// vector member that the claimed compiler classifies as MEMORY. Otherwise each
/// scalar's classes merge into the eightbytes it covers, and the clean-up
/// runs.
/// psABI §3.2.3, "Classification", rules 1-5.
pub(crate) fn classify(layout: AggregateLayout<'_>) -> Classification {
    // A transparent union's first member is classified in its place.
    let size = layout.first.map_or(layout.size, |first| first.size);
    if size > AggregateLayout::MAX_SCALAR_BYTES
        || layout.flags.intersects(
            AggregateFlags::UNALIGNED | AggregateFlags::FLEXIBLE | AggregateFlags::MEMORY,
        )
    {
        return Classification::Memory;
    }
    let mut classes = [Class::NoClass; 2];
    for scalar in layout.scalars {
        let first = (scalar.offset / 8) as usize;
        let (low, high) = match scalar.ty {
            | Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr =>
                (Class::Integer, None),
            | Type::I128 => (Class::Integer, Some(Class::Integer)),
            | Type::F16 | Type::BF16 | Type::F32 | Type::F64 => (Class::Sse, None),
            | Type::F80 => (Class::X87, Some(Class::X87Up)),
            | Type::F128 => (Class::Sse, Some(Class::SseUp)),
        };
        for (index, class) in [(first, Some(low)), (first + 1, high)] {
            if let Some(class) = class {
                match classes.get_mut(index) {
                    | Some(slot) => *slot = merge(*slot, class),
                    | None => return Classification::Memory,
                }
            }
        }
    }
    clean_up(classes)
}

/// Merges the class a scalar gives an eightbyte into the eightbyte's class.
/// psABI §3.2.3, "Classification", rule 4 (a)-(f).
const fn merge(current: Class, new: Class) -> Class {
    match (current, new) {
        | (a, b) if a as u8 == b as u8 => a,
        | (Class::NoClass, other) | (other, Class::NoClass) => other,
        | (Class::Memory, _) | (_, Class::Memory) => Class::Memory,
        | (Class::Integer, _) | (_, Class::Integer) => Class::Integer,
        | (Class::X87 | Class::X87Up, _) | (_, Class::X87 | Class::X87Up) => Class::Memory,
        | _ => Class::Sse,
    }
}

/// The post-merger clean-up.
/// psABI §3.2.3, "Classification", rule 5 (a)-(d); (c) cannot apply to two
/// eightbytes.
const fn clean_up(classes: [Class; 2]) -> Classification {
    match classes {
        | [Class::Memory, _] | [_, Class::Memory] => Classification::Memory,
        | [low, Class::X87Up] if !matches!(low, Class::X87) => Classification::Memory,
        | [Class::X87Up, _] => Classification::Memory,
        | [Class::SseUp, high] => Classification::Classes([Class::Sse, high]),
        | [low, Class::SseUp] if !matches!(low, Class::Sse | Class::SseUp) =>
            Classification::Classes([low, Class::Sse]),
        | classes => Classification::Classes(classes),
    }
}

/// The argument registers a call has used so far: six general registers
/// (RDI, RSI, RDX, RCX, R8, R9) and eight vector registers (XMM0-XMM7).
/// psABI §3.2.3, "Passing" and Figure 3.4.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct ArgumentRegisters {
    pub(crate) gp:  u32,
    pub(crate) sse: u32,
}

impl ArgumentRegisters {
    pub(crate) const GP: u32 = 6;
    pub(crate) const SSE: u32 = 8;

    /// Takes `gp` general and `sse` vector registers if all are free.
    pub(crate) fn take(&mut self, gp: u32, sse: u32) -> bool {
        if self.gp + gp <= Self::GP && self.sse + sse <= Self::SSE {
            self.gp += gp;
            self.sse += sse;
            true
        } else {
            false
        }
    }

    /// Takes the registers a scalar argument of type `ty` needs, if free:
    /// one general register for an integer or pointer, two for `i128`, one
    /// vector register for `f16`, `f32`, `f64` or `f128`, and none for `f80`,
    /// which always goes on the stack.
    pub(crate) fn take_scalar(&mut self, ty: Type) -> bool {
        let (gp, sse) = scalar_registers(ty);
        (gp, sse) != (0, 0) && self.take(gp, sse)
    }

    /// The registers the fixed parameters of a post-ABI signature use: a
    /// `registers` result area is not passed, an `sret` pointer takes a
    /// general register, and a `byval` aggregate is on the stack.
    pub(crate) fn after_fixed(signature: Signature<'_>) -> Self {
        let mut registers = Self::default();
        for (index, &ty) in signature.params.iter().enumerate() {
            match signature.param_abi(index) {
                | Some(ParamAbi::Registers(_) | ParamAbi::ByVal(_)) => {},
                | _ => _ = registers.take_scalar(ty),
            }
        }
        registers
    }
}

/// The general and vector registers a scalar of type `ty` needs.
/// psABI §3.2.3, "Classification", rule 1: `__int128` is two INTEGER
/// eightbytes, `__float128` SSE and SSEUP, and `long double` X87 and X87UP,
/// which is passed in memory.
const fn scalar_registers(ty: Type) -> (u32, u32) {
    match ty {
        | Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr => (1, 0),
        | Type::I128 => (2, 0),
        | Type::F16 | Type::BF16 | Type::F32 | Type::F64 | Type::F128 => (0, 1),
        | Type::F80 => (0, 0),
    }
}

/// The `va_list` of System V x86-64: `gp_offset` and `fp_offset` (offsets
/// into the register save area of the next general and vector argument),
/// `overflow_arg_area` (the next argument on the stack) and
/// `reg_save_area`, 24 bytes in all.
/// psABI §3.5.7, Figure 3.34.
pub(crate) mod va_list {
    pub(crate) const GP_OFFSET: u64 = 0;
    pub(crate) const FP_OFFSET: u64 = 4;
    pub(crate) const OVERFLOW_ARG_AREA: u64 = 8;
    pub(crate) const REG_SAVE_AREA: u64 = 16;
    pub(crate) const SIZE: u64 = 24;
    /// The register save area: six 8-byte general registers, then eight
    /// 16-byte vector registers.
    pub(crate) const SAVE_AREA_SIZE: u64 = 176;
    pub(crate) const GP_AREA_SIZE: u64 = 48;
}

/// Where `va_arg` finds an argument of one type.
/// psABI §3.5.7, "The `va_arg` Macro", steps 1-11.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct VaArgClass {
    /// The general registers it takes, or zero.
    pub(super) gp:             u32,
    /// Whether it takes a vector register.
    pub(super) sse:            bool,
    /// Its size and alignment in the overflow area on the stack.
    pub(super) overflow_size:  u64,
    pub(super) overflow_align: u64,
}

/// The `va_arg` class of a scalar type; `i1` has none, since C never passes
/// one.
pub(super) const fn va_arg_class(ty: Type) -> Option<VaArgClass> {
    let (gp, sse) = scalar_registers(ty);
    let wide = ty.bytes() > 8;
    match ty {
        | Type::I1 => None,
        | _ => Some(VaArgClass {
            gp,
            sse: sse == 1,
            overflow_size: if wide { 16 } else { 8 },
            overflow_align: if wide { 16 } else { 8 },
        }),
    }
}
