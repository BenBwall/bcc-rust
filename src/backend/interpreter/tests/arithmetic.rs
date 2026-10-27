//! Integer and floating arithmetic, comparisons and conversions without
//! poison: wrapping at every width, signed and unsigned forms.

use super::*;
use crate::x87::X87;

/// The result of `opcode.ty flags v0, v1` on `a` and `b`.
pub(super) fn binary(opcode: &str, ty: &str, a: u128, b: u128) -> RuntimeValue {
    let text = format!(
        "function @f({ty}, {ty}) -> {ty} external {{\nblock0(v0: {ty}, v1: {ty}):\n    v2 = \
         {opcode} v0, v1\n    return v2\n}}\n"
    );
    returned_with(&text, "f", &[int(a), int(b)])
}

#[test]
fn addition_wraps_at_every_width() {
    let cases = [
        ("iadd.i1", "i1", 1, 1, 0),
        ("iadd.i8", "i8", 0x7F, 1, 0x80),
        ("iadd.i8", "i8", 0xFF, 1, 0),
        ("iadd.i16", "i16", 0xFFFF, 2, 1),
        ("iadd.i32", "i32", 0xFFFF_FFFF, 1, 0),
        ("iadd.i64", "i64", u128::from(u64::MAX), 3, 2),
        ("iadd.i128", "i128", u128::MAX, 1, 0),
        ("isub.i8", "i8", 0, 1, 0xFF),
        ("isub.i32", "i32", 5, 7, 0xFFFF_FFFE),
        ("isub.i128", "i128", 0, 1, u128::MAX),
        ("imul.i8", "i8", 16, 17, 0x10),
        ("imul.i16", "i16", 0x100, 0x100, 0),
        ("imul.i64", "i64", 1 << 32, 1 << 32, 0),
        ("imul.i128", "i128", 1 << 64, 1 << 64, 0),
    ];
    for (opcode, ty, a, b, expected) in cases {
        assert_eq!(binary(opcode, ty, a, b), int(expected), "{opcode} {a} {b}");
    }
}

#[test]
fn division_and_remainder_follow_signedness() {
    let cases = [
        ("udiv.i8", 0xF9, 2, 0x7C),
        ("sdiv.i8", 0xF9, 2, 0xFD),
        ("urem.i8", 0xF9, 2, 1),
        ("srem.i8", 0xF9, 2, 0xFF),
        ("sdiv.i8", 7, 0xFE, 0xFD),
        ("srem.i8", 7, 0xFE, 1),
        ("sdiv.i8", 0x80, 2, 0xC0),
    ];
    for (opcode, a, b, expected) in cases {
        assert_eq!(
            binary(opcode, "i8", a, b),
            int(expected),
            "{opcode} {a} {b}"
        );
    }
    let minimum = 1 << 127;
    assert_eq!(binary("sdiv.i128", "i128", minimum, 2), int(3 << 126));
    assert_eq!(binary("udiv.i128", "i128", minimum, 2), int(1 << 126));
}

#[test]
fn bitwise_operations_and_shifts() {
    let cases = [
        ("and.i8", 0b1100, 0b1010, 0b1000),
        ("or.i8", 0b1100, 0b1010, 0b1110),
        ("xor.i8", 0b1100, 0b1010, 0b0110),
        ("shl.i8", 0x81, 1, 0x02),
        ("lshr.i8", 0x80, 1, 0x40),
        ("ashr.i8", 0x80, 1, 0xC0),
        ("ashr.i8", 0x40, 6, 0x01),
        ("shl.i8", 1, 7, 0x80),
    ];
    for (opcode, a, b, expected) in cases {
        assert_eq!(
            binary(opcode, "i8", a, b),
            int(expected),
            "{opcode} {a} {b}"
        );
    }
    assert_eq!(binary("shl.i128", "i128", 1, 127), int(1 << 127));
    assert_eq!(binary("ashr.i128", "i128", 1 << 127, 127), int(u128::MAX));
}

#[test]
fn comparisons_distinguish_signed_and_unsigned() {
    let compare = |cond: &str, a: u128, b: u128| {
        let text = format!(
            "function @f(i8, i8) -> i1 external {{\nblock0(v0: i8, v1: i8):\n    v2 = icmp.i8 \
             {cond} v0, v1\n    return v2\n}}\n"
        );
        returned_with(&text, "f", &[int(a), int(b)])
    };
    let cases = [
        ("eq", 5, 5, 1),
        ("ne", 5, 5, 0),
        ("slt", 0xFF, 0, 1),
        ("ult", 0xFF, 0, 0),
        ("sle", 0x80, 0x80, 1),
        ("sgt", 0x7F, 0x80, 1),
        ("ugt", 0x7F, 0x80, 0),
        ("sge", 0, 0xFF, 1),
        ("uge", 0, 0xFF, 0),
        ("ule", 3, 4, 1),
    ];
    for (cond, a, b, expected) in cases {
        assert_eq!(compare(cond, a, b), int(expected), "{cond} {a} {b}");
    }
}

#[test]
fn integer_conversions() {
    let text = "\
function @f(i8) -> i64 external {
block0(v0: i8):
    v1 = sext.i32 v0
    v2 = zext.i64 v1
    v3 = trunc.i16 v2
    v4 = sext.i64 v3
    v5 = zext.i64 v0
    v6 = iadd.i64 v4, v5
    return v6
}
";
    // sext 0x80 to i32 is 0xFFFF_FF80; its zext to i64 keeps the high half
    // clear; trunc to i16 gives 0xFF80, which sext makes -128. Adding the
    // zero-extended 0x80 gives 0.
    assert_eq!(returned_with(text, "f", &[int(0x80)]), int(0));
    assert_eq!(returned_with(text, "f", &[int(0x7F)]), int(0xFE));
}

#[test]
fn float_arithmetic_and_conversions() {
    let text = "\
function @f() -> i64 external {
block0:
    v0 = fconst.f64 0x3FF8000000000000
    v1 = fconst.f64 0x4002000000000000
    v2 = fadd.f64 v0, v1
    v3 = fmul.f64 v2, v2
    v4 = fneg.f64 v3
    v5 = fptosi.i64 v4
    return v5
}
";
    // (1.5 + 2.25)^2 = 14.0625, negated and truncated toward zero: -14.
    assert_eq!(returned(text, "f"), int(u128::from(-14_i64 as u64)));
    let text = "\
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = sitofp.f32 v0
    v2 = fpext.f64 v1
    v3 = fconst.f64 0x3FE0000000000000
    v4 = fdiv.f64 v2, v3
    v5 = fptrunc.f32 v4
    v6 = bitcast.i32 v5
    return v6
}
";
    // -3 / 0.5 = -6.0f, whose bits are 0xC0C00000.
    assert_eq!(
        returned_with(text, "f", &[int(0xFFFF_FFFD)]),
        int(0xC0C0_0000)
    );
    let text = "\
function @f(i32) -> f64 external {
block0(v0: i32):
    v1 = uitofp.f64 v0
    v2 = fconst.f64 0x4000000000000000
    v3 = frem.f64 v1, v2
    return v3
}
";
    assert_eq!(
        returned_with(text, "f", &[int(0xFFFF_FFFF)]),
        RuntimeValue::F64(1.0)
    );
}

#[test]
fn float_comparisons_handle_nan() {
    let compare = |cond: &str, a: u64, b: u64| {
        let text = format!(
            "function @f() -> i1 external {{\nblock0:\n    v0 = fconst.f64 0x{a:X}\n    v1 = \
             fconst.f64 0x{b:X}\n    v2 = fcmp.f64 {cond} v0, v1\n    return v2\n}}\n"
        );
        returned(&text, "f")
    };
    let nan = 0x7FF8_0000_0000_0000;
    let one = 1.0_f64.to_bits();
    let two = 2.0_f64.to_bits();
    let cases = [
        ("oeq", one, one, 1),
        ("oeq", nan, nan, 0),
        ("ueq", nan, one, 1),
        ("one", one, two, 1),
        ("one", nan, two, 0),
        ("une", nan, nan, 1),
        ("olt", one, two, 1),
        ("ult", nan, two, 1),
        ("ole", two, one, 0),
        ("ogt", two, one, 1),
        ("oge", one, one, 1),
        ("ugt", one, two, 0),
        ("uge", nan, one, 1),
        ("ule", two, one, 0),
        ("ord", one, two, 1),
        ("ord", one, nan, 0),
        ("uno", nan, one, 1),
        ("uno", one, two, 0),
    ];
    for (cond, a, b, expected) in cases {
        assert_eq!(compare(cond, a, b), int(expected), "{cond} {a:x} {b:x}");
    }
}

/// x87 `f80` values compute as the processor does: 1/3 rounds up in its
/// last of 64 bits, stores keep all ten bytes, and conversions round to
/// the narrower formats.
#[test]
fn extended_floats_compute_as_the_x87_unit_does() {
    let text = "\
function @f(ptr) -> f80 external {
block0(v0: ptr):
    v1 = fconst.f80 0x3FFF8000000000000000
    v2 = iconst.i32 3
    v3 = sitofp.f80 v2
    v4 = fdiv.f80 v1, v3
    store.f80 v4, v0, align 16
    v5 = load.f80 v0, align 16
    return v5
}
function @g() -> f80 external {
    slot0 = stack_slot 16, align 16
block0:
    v0 = stack_addr slot0
    v1 = call @f(v0)
    return v1
}
";
    assert_eq!(
        returned(text, "g"),
        RuntimeValue::F80(X87::from_bits(0x3FFD_AAAA_AAAA_AAAA_AAAB))
    );
    let text = "\
function @f() -> i64 external {
block0:
    v0 = fconst.f80 0x403EFFFFFFFFFFFFFFFF
    v1 = fptoui.i64 v0
    return v1
}
";
    assert_eq!(returned(text, "f"), int(u128::from(u64::MAX)));
    let text = "\
function @f() -> f64 external {
block0:
    v0 = fconst.f80 0x3FFDAAAAAAAAAAAAAAAB
    v1 = fptrunc.f64 v0
    return v1
}
";
    assert_eq!(returned(text, "f"), RuntimeValue::F64(1.0 / 3.0));
    // An unnormal is an invalid operand: unordered with itself.
    let text = "\
function @f() -> i1 external {
block0:
    v0 = fconst.f80 0x3FFF4000000000000000
    v1 = fcmp.f80 uno v0, v0
    return v1
}
";
    assert_eq!(returned(text, "f"), int(1));
}

/// `fptosi` and `fptoui` from x87 values give the truncated integer at the
/// edges of every width and poison just past them; `sitofp` and `uitofp`
/// from `i128` round ties to even.
#[test]
fn extended_integer_conversions_poison_just_outside_each_width() {
    let convert = |op: &str, to: &str, value: X87| {
        let text = format!(
            "function @f() -> {to} external {{\nblock0:\n    v0 = fconst.f80 0x{:020X}\n    v1 = \
             {op}.{to} v0\n    return v1\n}}\n",
            value.to_bits()
        );
        returned(&text, "f")
    };
    let half = X87::from_f64(0.5);
    for (to, bits) in [
        ("i8", 8),
        ("i16", 16),
        ("i32", 32),
        ("i64", 64),
        ("i128", 128),
    ] {
        let minimum = 1_u128 << (bits - 1);
        let mask = u128::MAX >> (128 - bits);
        let at_minimum = X87::from_integer(true, minimum);
        let below = X87::from_integer(true, minimum + if bits == 128 { 1 << 64 } else { 1 });
        assert_eq!(convert("fptosi", to, at_minimum), int(minimum), "{to}");
        assert_eq!(convert("fptosi", to, below), RuntimeValue::Poison, "{to}");
        assert_eq!(
            convert("fptosi", to, X87::from_integer(false, minimum)),
            RuntimeValue::Poison,
            "{to}"
        );
        assert_eq!(convert("fptoui", to, half.negate()), int(0), "{to}");
        assert_eq!(
            convert("fptoui", to, X87::from_integer(true, 1)),
            RuntimeValue::Poison,
            "{to}"
        );
        let top = if bits == 128 {
            mask - ((1 << 64) - 1)
        } else {
            mask
        };
        assert_eq!(
            convert("fptoui", to, X87::from_integer(false, top)),
            int(top),
            "{to}"
        );
        assert_eq!(
            convert(
                "fptoui",
                to,
                X87::from_integer(false, mask).add(X87::from_integer(false, 1))
            ),
            RuntimeValue::Poison,
            "{to}"
        );
        if bits < 64 {
            let maximum = X87::from_integer(false, minimum - 1).add(half);
            assert_eq!(convert("fptosi", to, maximum), int(minimum - 1), "{to}");
        }
    }
    let from = |op: &str, bits: u128| {
        let text = format!(
            "function @f() -> f80 external {{\nblock0:\n    v0 = iconst.i128 {}\n    v1 = \
             {op}.f80 v0\n    return v1\n}}\n",
            bits.cast_signed()
        );
        returned(&text, "f")
    };
    for (bits, signed, unsigned) in [
        (
            (1_u128 << 64) + 1,
            X87::from_integer(false, 1 << 64),
            X87::from_integer(false, 1 << 64),
        ),
        (
            1 << 127,
            X87::from_integer(true, 1 << 127),
            X87::from_integer(false, 1 << 127),
        ),
        (
            (1 << 127) + (3 << 63),
            X87::from_integer(true, (1 << 127) - (3 << 63)),
            X87::from_integer(false, (1 << 127) + (1 << 65)),
        ),
        (
            u128::MAX,
            X87::from_integer(true, 1),
            X87::from_integer(false, u128::MAX),
        ),
    ] {
        assert_eq!(from("sitofp", bits), RuntimeValue::F80(signed), "{bits:#x}");
        assert_eq!(
            from("uitofp", bits),
            RuntimeValue::F80(unsigned),
            "{bits:#x}"
        );
    }
}

/// `f128` computes as libgcc's soft-float routines: correctly rounded,
/// with the x86-64 default NaN for an invalid operation and the larger NaN
/// of two (the first operand of `fadd` on a tie, the second of `fsub`).
#[test]
fn binary128_arithmetic_matches_libgcc() {
    let one = 0x3FFF_0000_0000_0000_0000_0000_0000_0000;
    let three = 0x4000_8000_0000_0000_0000_0000_0000_0000;
    let quad = |opcode: &str, a: u128, b: u128| {
        let text = format!(
            "function @f() -> f128 external {{\nblock0:\n    v0 = fconst.f128 {a:#x}\n    v1 = \
             fconst.f128 {b:#x}\n    v2 = {opcode}.f128 v0, v1\n    return v2\n}}\n"
        );
        returned(&text, "f")
    };
    let positive_nan = 0x7FFF_8000_0000_0000_0000_0000_0000_0005;
    let negative_nan = 0xFFFF_8000_0000_0000_0000_0000_0000_0005;
    let infinity = 0x7FFF_u128 << 112;
    for (opcode, a, b, expected) in [
        ("fadd", one, one, 0x4000_0000_0000_0000_0000_0000_0000_0000),
        (
            "fdiv",
            one,
            three,
            0x3FFD_5555_5555_5555_5555_5555_5555_5555,
        ),
        (
            "fmul",
            three,
            three,
            0x4002_2000_0000_0000_0000_0000_0000_0000,
        ),
        (
            "fsub",
            infinity,
            infinity,
            0xFFFF_8000_0000_0000_0000_0000_0000_0000,
        ),
        (
            "frem",
            three,
            0x4000_0000_0000_0000_0000_0000_0000_0000,
            one,
        ),
        ("fadd", positive_nan, negative_nan, positive_nan),
        ("fsub", positive_nan, negative_nan, negative_nan),
    ] {
        assert_eq!(
            quad(opcode, a, b),
            RuntimeValue::F128(expected),
            "{opcode} {a:#x} {b:#x}"
        );
    }
    let convert = |op: &str, to: &str, operand: &str| {
        let text = format!(
            "function @f() -> {to} external {{\nblock0:\n    v0 = {operand}\n    v1 = {op}.{to} \
             v0\n    return v1\n}}\n"
        );
        returned(&text, "f")
    };
    let third = "fconst.f128 0x3FFD5555555555555555555555555555";
    assert_eq!(
        convert("fptrunc", "f64", third),
        RuntimeValue::F64(1.0 / 3.0)
    );
    assert_eq!(
        convert(
            "fptosi",
            "i32",
            "fconst.f128 0xC0008000000000000000000000000000"
        ),
        int(0xFFFF_FFFD)
    );
    assert_eq!(
        convert("fpext", "f128", "fconst.f64 0x3FF0000000000000"),
        RuntimeValue::F128(one)
    );
    assert_eq!(
        convert("sitofp", "f128", "iconst.i64 -3"),
        RuntimeValue::F128(three | 1 << 127)
    );
}
