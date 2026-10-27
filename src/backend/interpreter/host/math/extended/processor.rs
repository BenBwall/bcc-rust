//! The x87 instructions glibc's x86-64 `long double` functions run, as the
//! processor executes them.
//!
//! `f2xm1`, `fyl2x`, `fyl2xp1` and `fpatan` compute their results in the
//! processor's microcode, which Intel and AMD document only to within one
//! unit in the last place; no software model reproduces those bits. They run
//! on the host's own x87 unit, so a function built on them gives the bits
//! glibc gives on the same processor. Their operands and the steps around
//! them run on the software model (`crate::x87`), which matches the
//! hardware exactly, and a host without an x87 unit cannot run them at all.
//! The rest of this file models the status that `fxam`, `fcom` and
//! `fucomi` report, which the assembly language functions branch on, and
//! `fscale`.

use crate::{
    backend::interpreter::trap::Fault,
    x87::{
        Class,
        X87,
    },
};

/// `f2xm1`: `2^x - 1`, for `x` in [-1, 1].
pub(super) fn f2xm1(x: X87) -> Result<X87, Fault> {
    instruction(Instruction::F2xm1, x, X87::zero(false))
}

/// `fyl2x`: `y × log2(x)`.
pub(super) fn fyl2x(x: X87, y: X87) -> Result<X87, Fault> {
    instruction(Instruction::Fyl2x, x, y)
}

/// `fyl2xp1`: `y × log2(x + 1)`, for `x` near zero.
pub(super) fn fyl2xp1(x: X87, y: X87) -> Result<X87, Fault> {
    instruction(Instruction::Fyl2xp1, x, y)
}

/// `fpatan`: the angle of the point (`x`, `y`), `atan(y / x)` in the
/// quadrant of the point.
pub(super) fn fpatan(x: X87, y: X87) -> Result<X87, Fault> {
    instruction(Instruction::Fpatan, x, y)
}

/// The constants the x87 unit loads (`fldl2e`, `fldln2`, `fldlg2`),
/// rounded to nearest.
pub(super) const LOG2_E: X87 = X87::from_bits(0x3FFF_B8AA_3B29_5C17_F0BC);
pub(super) const LN_2: X87 = X87::from_bits(0x3FFE_B172_17F7_D1CF_79AC);
pub(super) const LOG10_2: X87 = X87::from_bits(0x3FFD_9A20_9A84_FBCF_F799);

/// An instruction the host's x87 unit runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Instruction {
    F2xm1,
    Fyl2x,
    Fyl2xp1,
    Fpatan,
}

/// Runs `instruction` with `x` in `st(0)` and `y` in `st(1)` (unused by
/// `f2xm1`) under the default control word: 64-bit precision, round to
/// nearest, exceptions masked. Pending exception flags are cleared and the
/// thread's control word restored afterwards.
#[cfg(target_arch = "x86_64")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "A host without an x87 unit fails; this one cannot."
)]
pub(super) fn instruction(instruction: Instruction, x: X87, y: X87) -> Result<X87, Fault> {
    use std::arch::asm;
    let (x, y) = (x.to_bits().to_le_bytes(), y.to_bits().to_le_bytes());
    let mut out = [0_u8; 16];
    let (x, y, result) = (x.as_ptr(), y.as_ptr(), out.as_mut_ptr());
    let mut saved = 0_u16;
    let control = 0x037F_u16;
    macro_rules! run {
        ($operation:literal) => {
            // SAFETY: The sequence reads ten bytes at `x` and `y` and writes
            // ten at `out`, all live arrays of sixteen; it leaves the x87
            // stack empty, clears the exception flags it raised and
            // restores the control word it saved.
            unsafe {
                asm!(
                    "fnstcw [{saved}]",
                    "fldcw [{control}]",
                    "fld tbyte ptr [{y}]",
                    "fld tbyte ptr [{x}]",
                    $operation,
                    "fnclex",
                    "fldcw [{saved}]",
                    saved = in(reg) &raw mut saved,
                    control = in(reg) &raw const control,
                    x = in(reg) x,
                    y = in(reg) y,
                    out = in(reg) result,
                    out("st(0)") _,
                    out("st(1)") _,
                    options(nostack),
                );
            }
        };
    }
    match instruction {
        | Instruction::F2xm1 => run!("f2xm1\nfstp tbyte ptr [{out}]\nfstp st(0)"),
        | Instruction::Fyl2x => run!("fyl2x\nfstp tbyte ptr [{out}]"),
        | Instruction::Fyl2xp1 => run!("fyl2xp1\nfstp tbyte ptr [{out}]"),
        | Instruction::Fpatan => run!("fpatan\nfstp tbyte ptr [{out}]"),
    }
    Ok(X87::from_bits(u128::from_le_bytes(out)))
}

/// Without an x87 unit the microcoded instructions cannot run.
#[cfg(not(target_arch = "x86_64"))]
pub(super) fn instruction(_instruction: Instruction, _x: X87, _y: X87) -> Result<X87, Fault> {
    Err(Fault::Unsupported(
        crate::backend::interpreter::trap::Unsupported::WideFloat,
    ))
}

/// What `fxam` reports in the condition codes C3, C2 and C0 (`ah & 0x45`
/// after `fnstsw`); C1 is the sign bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Examined {
    /// An unnormal, pseudo-infinity or pseudo-NaN (000).
    Unsupported,
    /// 001.
    Nan,
    /// 010.
    Normal,
    /// 011.
    Infinity,
    /// 100.
    Zero,
    /// 110, pseudo-denormals included.
    Denormal,
}

/// `fxam`.
pub(super) fn examine(x: X87) -> Examined {
    match x.class() {
        | Class::Zero => Examined::Zero,
        | Class::Finite { .. } if x.magnitude_parts().0 == 0 => Examined::Denormal,
        | Class::Finite { .. } => Examined::Normal,
        | Class::Infinite => Examined::Infinity,
        | Class::Nan { .. } => Examined::Nan,
        | Class::Unsupported => Examined::Unsupported,
    }
}

/// The flags `fcomi` and `fucomi` set comparing `a` (`st(0)`) with `b`, and
/// the condition codes `fcom` sets, which `fnstsw` and `sahf` read the
/// same way: an unordered pair sets all three.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The three flags a comparison sets."
)]
pub(super) struct Flags {
    /// ZF, or C3: equal or unordered.
    pub(super) zero:   bool,
    /// PF, or C2: unordered.
    pub(super) parity: bool,
    /// CF, or C0: below or unordered.
    pub(super) carry:  bool,
    /// Whether `a` is above `b`: no flag set.
    pub(super) above:  bool,
}

/// Compares `a` with `b` as `fcomi`, `fucomi` and `fcom` do.
pub(super) fn compare(a: X87, b: X87) -> Flags {
    let order = a.compare(b);
    Flags {
        zero:   order.is_none_or(std::cmp::Ordering::is_eq),
        parity: order.is_none(),
        carry:  order.is_none_or(std::cmp::Ordering::is_lt),
        above:  order.is_some_and(std::cmp::Ordering::is_gt),
    }
}

/// `fscale`: `value × 2^n` for `n` the integer part of `power`, rounded
/// once. A NaN or invalid operand propagates as in arithmetic; an infinite
/// power scales a finite value to an infinity or a zero, and is invalid
/// for a zero or infinite value scaled the other way.
pub(super) fn fscale(value: X87, power: X87) -> X87 {
    match power.class() {
        | Class::Zero => value.scale(0),
        | Class::Finite { .. } => {
            // Beyond 2^20 every finite result has overflowed or underflowed.
            let limit = X87::from_integer(power.is_negative(), 1 << 20);
            let bounded = if power.with_sign(false).compare(limit.with_sign(false))
                == Some(std::cmp::Ordering::Greater)
            {
                limit
            } else {
                power
            };
            let bits = bounded
                .round_integral(crate::x87::Integral::Truncate)
                .to_integer(true, 64)
                .expect("a bounded integer");
            value.scale(
                u64::try_from(bits)
                    .expect("64-bit integer bits")
                    .cast_signed(),
            )
        },
        | Class::Infinite => match value.class() {
            | Class::Nan { .. } | Class::Unsupported => value.add(power),
            | Class::Zero if !power.is_negative() => X87::INDEFINITE,
            | Class::Infinite if power.is_negative() => X87::INDEFINITE,
            | Class::Zero | Class::Infinite => value,
            | Class::Finite { .. } if power.is_negative() => X87::zero(value.is_negative()),
            | Class::Finite { .. } => X87::infinity(value.is_negative()),
        },
        | Class::Nan { .. } | Class::Unsupported => value.add(power),
    }
}
