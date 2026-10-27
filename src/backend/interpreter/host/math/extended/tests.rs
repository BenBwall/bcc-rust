//! Compares the `long double` functions with the host glibc's own, called
//! through the System V calling convention, on random encodings of every
//! class and on values near each function's branch points. The interpreter
//! follows glibc 2.43; on an older host glibc the test skips the `powl` cases
//! glibc 2.43 changed (commit 1b657c53c21a, "Simplify powl computation for
//! small integral y", BZ #33411: an invalid-encoding exponent, and a
//! pseudo-denormal or invalid base to an integral power from -3 to 3, and
//! `tgammal` of an unnormal, which reaches `powl` with one), and reports per
//! function how many it compared and skipped. Results that come
//! from the processor's microcode (`f2xm1`, `fyl2x`, `fyl2xp1`, `fpatan`)
//! are the same on both sides, since the interpreter runs those
//! instructions on the same processor. The test binary links glibc statically,
//! and a static libm has no SVID compatibility wrappers, so where a
//! dynamically linked program's wrapper returns `NAN` instead of the
//! function's real indefinite (the fixture `long_double_math.c` checks
//! those), the comparison expects the wrapper's value.

use std::{
    arch::asm,
    ffi::{
        CStr,
        c_char,
    },
};

use super::{
    Operation,
    extended,
    lgamma,
};
use crate::{
    backend::interpreter::{
        memory::Memory,
        value::RuntimeValue,
    },
    ir::Type,
    util::bump::Bump,
    x87::{
        Class,
        X87,
    },
};

unsafe extern "C" {
    fn gnu_get_libc_version() -> *const c_char;
}

/// The (major, minor) version of the glibc the test binary runs with.
fn host_glibc() -> (u32, u32) {
    // SAFETY: `gnu_get_libc_version` has no preconditions.
    let version = unsafe { gnu_get_libc_version() };
    // SAFETY: glibc returns a pointer to a static, null-terminated string.
    let version = unsafe { CStr::from_ptr(version) };
    let version = version.to_str().unwrap_or("0.0");
    let mut parts = version.split('.').map(|part| part.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Whether glibc 2.43 computes `operation` of `args` differently from
/// earlier releases (commit 1b657c53c21a, BZ #33411, which rewrote
/// `e_powl.S`'s path for a small integral exponent): `powl` of an
/// invalid-encoding exponent, or of a pseudo-denormal or invalid base to an
/// integral power from -3 to 3.
fn changed_in_glibc_2_43(operation: Operation, args: &[X87]) -> bool {
    if operation != Operation::Pow {
        return false;
    }
    let (x, y) = (args[0], args[1]);
    let (biased, significand) = x.magnitude_parts();
    let noncanonical = x.class() == Class::Unsupported || (biased == 0 && significand >> 63 != 0);
    let small_integral = matches!(y.class(), Class::Zero | Class::Finite { .. })
        && y.round_integral(crate::x87::Integral::Truncate) == y
        && y.with_sign(false)
            .compare(X87::from_integer(false, 3))
            .is_some_and(std::cmp::Ordering::is_le);
    y.class() == Class::Unsupported || (noncanonical && small_integral)
}

#[link(name = "m")]
unsafe extern "C" {
    fn sinl();
    fn cosl();
    fn tanl();
    fn asinl();
    fn acosl();
    fn atanl();
    fn atan2l();
    fn expl();
    fn exp2l();
    fn expm1l();
    fn logl();
    fn log2l();
    fn log10l();
    fn log1pl();
    fn powl();
    fn sinhl();
    fn coshl();
    fn tanhl();
    fn asinhl();
    fn acoshl();
    fn atanhl();
    fn cbrtl();
    fn hypotl();
    fn fmal();
    fn erfl();
    fn erfcl();
    fn lgammal();
    fn tgammal();
    static mut signgam: std::ffi::c_int;
}

/// Calls glibc's `function`, which takes up to three `long double`
/// arguments and returns one, with `args`.
fn glibc(function: unsafe extern "C" fn(), args: [X87; 3]) -> X87 {
    let bytes = args.map(|arg| arg.to_bits().to_le_bytes());
    let mut out = [0_u8; 16];
    // SAFETY: The block moves the stack pointer below the red zone, aligns
    // it, passes the three ten-byte arguments in memory as the System V ABI
    // passes `long double`, calls a libm function that takes at most three
    // of them, stores its `st(0)` result to `out` and restores the stack
    // pointer; `r12` and `r13` are callee-saved, so they survive the call.
    unsafe {
        asm!(
            "mov r13, rsp",
            "sub rsp, 256",
            "and rsp, -16",
            "sub rsp, 48",
            "fld tbyte ptr [{a}]",
            "fstp tbyte ptr [rsp]",
            "fld tbyte ptr [{b}]",
            "fstp tbyte ptr [rsp + 16]",
            "fld tbyte ptr [{c}]",
            "fstp tbyte ptr [rsp + 32]",
            "call {function}",
            "fstp tbyte ptr [r12]",
            "mov rsp, r13",
            a = in(reg) bytes[0].as_ptr(),
            b = in(reg) bytes[1].as_ptr(),
            c = in(reg) bytes[2].as_ptr(),
            function = in(reg) function,
            in("r12") out.as_mut_ptr(),
            out("r13") _,
            clobber_abi("C"),
        );
    }
    X87::from_bits(u128::from_le_bytes(out))
}

/// What the interpreter's host computes for `operation`.
fn interpreted(operation: Operation, args: &[X87]) -> X87 {
    let arena = Bump::new();
    let mut memory = Memory::new(&arena, 1024);
    let mut values = [RuntimeValue::Poison; 3];
    for (value, &arg) in values.iter_mut().zip(args) {
        *value = RuntimeValue::F80(arg);
    }
    match extended(
        operation,
        &values[..args.len()],
        &mut memory,
        Type::I64,
        &[],
    ) {
        | Ok(RuntimeValue::F80(value)) => value,
        | other => panic!("{other:?}"),
    }
}

/// A deterministic generator of encodings: every class, biased toward
/// magnitudes where the functions branch.
struct Encodings(u64);

impl Encodings {
    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn exponent(&mut self) -> u16 {
        let random = self.next_u64();
        let offset = u16::try_from((random >> 8) % 0x100).expect("masked");
        match random % 16 {
            | 0 => 0,
            | 1 => 0x7FFF,
            | 2 => 0x7FFE - offset % 32,
            | 3 => 1 + offset % 32,
            | 4 => 0x3FFF - 0x80 + offset,
            | 5 => 0x3FFF - 70 + offset % 8,
            | 6 => 0x400B + offset % 4,
            | 7 => 0x4003 + offset % 4,
            | 8 => u16::try_from((random >> 20) % 0x8000).expect("masked"),
            | _ => 0x3FFF - 8 + offset % 16,
        }
    }

    fn significand(&mut self) -> u64 {
        let random = self.next_u64();
        let bits = self.next_u64();
        match random % 16 {
            | 0 => 0,
            | 1 => 1 << 63,
            | 2 => u64::MAX,
            | 3 => bits >> 1,
            | 4 => (1 << 63) | (1 << 62) | (bits & 0xFF),
            | 5 => (1 << 63) | (bits << 40),
            | 6 => 0xC90F_DAA2_2168_C235 ^ (bits & 0xFFFF),
            | 7 => 0xB172_17F7_D1CF_79AC ^ (bits & 0xFFFF),
            | _ => (1 << 63) | bits,
        }
    }

    fn value(&mut self) -> X87 {
        let sign = u128::from(self.next_u64() & 1) << 79;
        let exponent = u128::from(self.exponent()) << 64;
        X87::from_bits(sign | exponent | u128::from(self.significand()))
    }

    /// A value that is often small integral, half-integral or exactly 1.
    fn exponent_operand(&mut self) -> X87 {
        let random = self.next_u64();
        let integer = |n: u64| X87::from_integer(random & 1 != 0, u128::from(n));
        match random % 8 {
            | 0 => integer((random >> 8) % 70),
            | 1 => integer((random >> 8) % 70).mul(X87::from_f64(0.5)),
            | 2 => integer(1 << 62 | (random >> 8)),
            | _ => self.value(),
        }
    }
}

const FUNCTIONS: [(&str, Operation, usize); 26] = [
    ("sinl", Operation::Sin, 1),
    ("cosl", Operation::Cos, 1),
    ("tanl", Operation::Tan, 1),
    ("asinl", Operation::Asin, 1),
    ("acosl", Operation::Acos, 1),
    ("atanl", Operation::Atan, 1),
    ("atan2l", Operation::Atan2, 2),
    ("expl", Operation::Exp, 1),
    ("exp2l", Operation::Exp2, 1),
    ("expm1l", Operation::Expm1, 1),
    ("logl", Operation::Log, 1),
    ("log2l", Operation::Log2, 1),
    ("log10l", Operation::Log10, 1),
    ("log1pl", Operation::Log1p, 1),
    ("powl", Operation::Pow, 2),
    ("sinhl", Operation::Sinh, 1),
    ("coshl", Operation::Cosh, 1),
    ("tanhl", Operation::Tanh, 1),
    ("asinhl", Operation::Asinh, 1),
    ("acoshl", Operation::Acosh, 1),
    ("atanhl", Operation::Atanh, 1),
    ("cbrtl", Operation::Cbrt, 1),
    ("hypotl", Operation::Hypot, 2),
    ("fmal", Operation::Fma, 3),
    ("erfl", Operation::Erf, 1),
    ("erfcl", Operation::Erfc, 1),
];

/// glibc's function named `name`.
fn function(name: &str) -> unsafe extern "C" fn() {
    match name {
        | "sinl" => sinl,
        | "cosl" => cosl,
        | "tanl" => tanl,
        | "asinl" => asinl,
        | "acosl" => acosl,
        | "atanl" => atanl,
        | "atan2l" => atan2l,
        | "expl" => expl,
        | "exp2l" => exp2l,
        | "expm1l" => expm1l,
        | "logl" => logl,
        | "log2l" => log2l,
        | "log10l" => log10l,
        | "log1pl" => log1pl,
        | "powl" => powl,
        | "sinhl" => sinhl,
        | "coshl" => coshl,
        | "tanhl" => tanhl,
        | "asinhl" => asinhl,
        | "acoshl" => acoshl,
        | "atanhl" => atanhl,
        | "cbrtl" => cbrtl,
        | "hypotl" => hypotl,
        | "erfl" => erfl,
        | "erfcl" => erfcl,
        | _ => fmal,
    }
}

/// The value a dynamically linked program's SVID wrapper returns for
/// `operation` of `x` (`w_asinl_compat.c`, `w_logl_compat.c`,
/// `w_log2l_compat.c`, `w_log10l_compat.c`), given the function's `result`.
fn wrapped(operation: Operation, x: X87, result: X87) -> X87 {
    let nan = X87::from_bits(0x7FFF_C000_0000_0000_0000);
    let order = x.compare(X87::zero(false));
    match operation {
        | Operation::Asin
            if x.with_sign(false).compare(X87::from_integer(false, 1))
                == Some(std::cmp::Ordering::Greater) =>
            nan,
        | Operation::Log | Operation::Log2 | Operation::Log10
            if order == Some(std::cmp::Ordering::Less) =>
            nan,
        | _ => result,
    }
}

#[test]
fn transcendental_functions_match_glibc_bit_for_bit() {
    let older_glibc = host_glibc() < (2, 43);
    let mut encodings = Encodings(0x9E37_79B9_7F4A_7C15);
    let mut failures = 0;
    for (name, operation, arity) in FUNCTIONS {
        let glibc_function = function(name);
        let (mut shown, mut compared, mut skipped) = (0, 0, 0);
        for case in 0..100_000 {
            let mut args = [encodings.value(), encodings.value(), encodings.value()];
            if operation == Operation::Pow {
                args[1] = encodings.exponent_operand();
            }
            if case % 4 == 0 && arity > 1 {
                // Operands of nearby magnitudes.
                let exponent = args[0].to_bits() & (0x7FFF << 64);
                args[1] = X87::from_bits((args[1].to_bits() & !(0x7FFF << 64)) | exponent);
            }
            if older_glibc && changed_in_glibc_2_43(operation, &args) {
                skipped += 1;
                continue;
            }
            compared += 1;
            let expected = wrapped(operation, args[0], glibc(glibc_function, args));
            let actual = interpreted(operation, &args[..arity]);
            if actual != expected {
                failures += 1;
                shown += 1;
                if shown <= 4 {
                    eprintln!("{name}{:?}: {actual:?}, glibc {expected:?}", &args[..arity]);
                }
            }
        }
        eprintln!("{name}: {compared} compared, {skipped} skipped, {shown} differ");
    }
    assert_eq!(failures, 0, "results that differ from glibc");
}

/// The value `tgammal`'s SVID wrapper (`w_tgammal_compat.c`) returns, given
/// what the static library's `tgammal`, which has none, returned: `NAN`
/// for a negative integer and for -∞, and an infinity with the argument's
/// sign for a pole or an overflow.
fn wrapped_tgamma(x: X87, result: X87) -> X87 {
    let zero = X87::zero(false);
    let finite = |value: X87| {
        value
            .with_sign(false)
            .compare(X87::from_bits(0x7FFE_FFFF_FFFF_FFFF_FFFF))
            .is_some_and(std::cmp::Ordering::is_le)
    };
    let minus_infinity = x.class() == Class::Infinite && x.is_negative();
    let is_zero = |value: X87| value.compare(zero) == Some(std::cmp::Ordering::Equal);
    if (finite(result) && !is_zero(result)) || !(finite(x) || minus_infinity) {
        return result;
    }
    if is_zero(x) {
        X87::infinity(x.is_negative())
    } else if x.round_integral(crate::x87::Integral::Floor) == x
        && x.compare(zero) == Some(std::cmp::Ordering::Less)
    {
        X87::from_bits(0x7FFF_C000_0000_0000_0000)
    } else if is_zero(result) {
        result
    } else {
        X87::infinity(x.is_negative())
    }
}

/// Whether glibc 2.43 computes `tgammal` of `x` differently from earlier
/// releases: an unnormal (a nonzero exponent without the integer bit)
/// reaches `gammal_positive`'s `powl (x_adj_mant, x_adj)` with an
/// invalid-encoding exponent, which commit 1b657c53c21a (BZ #33411) made
/// give 1 rather than the positive NaN, so `tgammal` ends with the real
/// indefinite (negative NaN) instead of a positive NaN. Pseudo-infinities
/// and pseudo-NaNs give the same in both.
fn tgamma_changed_in_glibc_2_43(x: X87) -> bool {
    let (biased, significand) = x.magnitude_parts();
    biased != 0 && biased != 0x7FFF && significand >> 63 == 0
}

/// `lgammal` (with the sign it leaves in `signgam`) and `tgammal` match
/// glibc on random encodings and on arguments around the negative
/// integers, the zeros of lgamma between -33 and -2, and the points where
/// the functions change method. The interpreter follows glibc 2.43; on an
/// older host glibc the test skips the `tgammal` cases glibc 2.43 changed
/// (`tgamma_changed_in_glibc_2_43`), and it reports per function how many
/// it compared and skipped.
#[test]
fn gamma_functions_match_glibc_bit_for_bit() {
    let older_glibc = host_glibc() < (2, 43);
    let mut encodings = Encodings(0x2545_F491_4F6C_DD1D);
    let mut arguments = |case: u32| {
        let random = encodings.next_u64();
        let fraction = X87::from_integer(false, u128::from(random >> 11))
            .scale(-53)
            .with_sign(random & 1 != 0);
        match case % 4 {
            | 0 => encodings.value(),
            // -34 to 34, often near an integer or a half-integer.
            | 1 => X87::from_integer(false, u128::from((random >> 20) % 68))
                .sub(X87::from_integer(false, 34))
                .add(fraction.scale(-i64::try_from((random >> 8) % 64).expect("small"))),
            // Up to 1800.
            | 2 => X87::from_integer(false, u128::from((random >> 20) % 1800)).add(fraction),
            | _ => fraction.scale(-i64::try_from((random >> 8) % 80).expect("small")),
        }
    };
    // Compared, skipped and differing cases of `lgammal` and `tgammal`.
    let (mut lgamma_counts, mut tgamma_counts) = ([0_u32; 3], [0_u32; 3]);
    for case in 0..200_000 {
        let x = arguments(case);
        let (actual, actual_sign) = lgamma(x).expect("an x87 host");
        let expected = glibc(lgammal, [x, x, x]);
        // SAFETY: glibc's `lgammal` has just set `signgam` on this thread.
        let expected_sign = unsafe { (&raw const signgam).read_volatile() };
        lgamma_counts[0] += 1;
        if (actual, actual_sign) != (expected, expected_sign) {
            lgamma_counts[2] += 1;
            if lgamma_counts[2] <= 4 {
                eprintln!(
                    "lgammal({x:?}): {actual:?} {actual_sign}, glibc {expected:?} {expected_sign}"
                );
            }
        }
        if older_glibc && tgamma_changed_in_glibc_2_43(x) {
            tgamma_counts[1] += 1;
            continue;
        }
        tgamma_counts[0] += 1;
        let actual = interpreted(Operation::Tgamma, &[x]);
        let expected = wrapped_tgamma(x, glibc(tgammal, [x, x, x]));
        if actual != expected {
            tgamma_counts[2] += 1;
            if tgamma_counts[2] <= 4 {
                eprintln!("tgammal({x:?}): {actual:?}, glibc {expected:?}");
            }
        }
    }
    for (name, [compared, skipped, differ]) in
        [("lgammal", lgamma_counts), ("tgammal", tgamma_counts)]
    {
        eprintln!("{name}: {compared} compared, {skipped} skipped, {differ} differ");
    }
    assert_eq!(
        lgamma_counts[2] + tgamma_counts[2],
        0,
        "results that differ from glibc"
    );
}

/// The cases glibc 2.43's `powl` rewrite changed, as an older glibc reported
/// them, are the ones the comparison skips there; ordinary powers are not.
#[test]
fn powl_cases_changed_in_glibc_2_43_are_classified() {
    let x87 = |bits: u128| X87::from_bits(bits);
    for (x, y, changed) in [
        (0xBFFB_C90F_DAA2_2168_5608, 0xBFFB_4816_1511_FD93_566C, true),
        (0xBFFC_8B79_F5EA_94A0_8663, 0x3FFC_0000_0000_0000_0000, true),
        (0x8000_A423_B61F_1B20_C989, 0x3FFF_8000_0000_0000_0000, true),
        (0x3FFF_4000_0000_0000_0000, 0x4000_C000_0000_0000_0000, true),
        (
            0x8000_A423_B61F_1B20_C989,
            0x4001_8000_0000_0000_0000,
            false,
        ),
        (
            0x3FFF_C000_0000_0000_0000,
            0x4000_C000_0000_0000_0000,
            false,
        ),
        (
            0x3FFF_C000_0000_0000_0000,
            0x3FFE_C000_0000_0000_0000,
            false,
        ),
    ] {
        assert_eq!(
            changed_in_glibc_2_43(Operation::Pow, &[x87(x), x87(y)]),
            changed,
            "{x:#x} {y:#x}"
        );
    }
    assert!(!changed_in_glibc_2_43(
        Operation::Exp,
        &[x87(0x3FFF_4000_0000_0000_0000)]
    ));
}
