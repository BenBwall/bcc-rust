//! Compares the `long double` functions with the host glibc's own, called
//! through the System V calling convention, on random encodings of every
//! class and on values near each function's branch points. Only cases on
//! which glibc 2.39 and 2.43 agree are compared: `powl` of an invalid
//! exponent changed between them. The test binary links glibc statically,
//! and a static libm has no SVID compatibility wrappers, so where a
//! dynamically linked program's wrapper returns `NAN` instead of the
//! function's real indefinite (the fixture `long_double_math.c` checks
//! those), the comparison expects the wrapper's value.

use std::arch::asm;

use super::{
    Operation,
    extended,
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
    let mut encodings = Encodings(0x9E37_79B9_7F4A_7C15);
    let mut failures = 0;
    for (name, operation, arity) in FUNCTIONS {
        let glibc_function = function(name);
        let mut shown = 0;
        for case in 0..100_000 {
            let mut args = [encodings.value(), encodings.value(), encodings.value()];
            if operation == Operation::Pow {
                args[1] = encodings.exponent_operand();
                if args[1].class() == Class::Unsupported {
                    continue;
                }
            }
            if case % 4 == 0 && arity > 1 {
                // Operands of nearby magnitudes.
                let exponent = args[0].to_bits() & (0x7FFF << 64);
                args[1] = X87::from_bits((args[1].to_bits() & !(0x7FFF << 64)) | exponent);
            }
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
    }
    assert_eq!(failures, 0, "results that differ from glibc");
}
