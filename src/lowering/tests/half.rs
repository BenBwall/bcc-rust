//! `_Float16` values evaluated in `float`: chains of operators round once,
//! and the identity decides which other uses keep the excess.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
};

fn lowered_as(source: &str, identity: CompilerIdentity) -> String {
    let configuration = CompilerConfiguration::default().with_compiler_identity(identity);
    functions(
        &lower_with(source, configuration)
            .unwrap_or_else(|errors| panic!("lowering failed: {errors:?}\n{source}")),
    )
}

/// Both compilers widen each operand with `fpext`, compute in `f32` and
/// round the chain's result once.
#[test]
fn a_chain_of_operators_rounds_once() {
    let source = "_Float16 f(_Float16 a, _Float16 b, _Float16 c) { return a * b / c; }";
    for identity in [CompilerIdentity::Clang, CompilerIdentity::Gcc] {
        let text = lowered_as(source, identity);
        let rounds = text.matches("fptrunc.f16").count();
        assert_eq!(rounds, 1, "{identity:?}\n{text}");
        assert!(
            text.contains("fmul.f32") && text.contains("fdiv.f32"),
            "{text}"
        );
    }
}

/// GCC converts the excess to `float` and an integer operand straight to
/// `float`; Clang rounds the product to `_Float16` first and converts the
/// integer to `_Float16`.
#[test]
fn the_identity_decides_where_the_excess_ends() {
    let source = "float f(_Float16 a, _Float16 b) { return a * b; }\n_Float16 g(_Float16 a) { \
                  return a / 70000; }";
    let gcc = lowered_as(source, CompilerIdentity::Gcc);
    let clang = lowered_as(source, CompilerIdentity::Clang);
    assert_eq!(gcc.matches("fptrunc.f16").count(), 1, "{gcc}");
    assert_eq!(clang.matches("fptrunc.f16").count(), 2, "{clang}");
    // 70000 as `float`, and as the binary16 infinity widened.
    assert!(gcc.contains("fconst.f32 0x4788B800"), "{gcc}");
    assert!(clang.contains("fconst.f16 0x7C00"), "{clang}");
}

/// Constants fold to their bits: `f16` loads and stores keep their own
/// type, and a folded constant needs no conversion at run time.
#[test]
fn constants_and_memory_keep_binary16_bits() {
    let text = lowered_as(
        "_Float16 g = 1.5f16;\nvoid f(_Float16 *p) { *p = 0.1f16; }",
        CompilerIdentity::Clang,
    );
    assert!(text.contains("store.f16"), "{text}");
    assert!(text.contains("fconst.f16 0x2E66"), "{text}");
    assert!(!text.contains("fptrunc"), "{text}");
}

/// GCC tests a product's excess, which may be nonzero where the rounded
/// value is zero; Clang tests the rounded value.
#[test]
fn truth_tests_read_the_excess_for_gcc() {
    let source = "int f(_Float16 a) { if (a * a) return 1; return !(a * a) + (a * a && 1); }";
    let gcc = lowered_as(source, CompilerIdentity::Gcc);
    let clang = lowered_as(source, CompilerIdentity::Clang);
    assert!(!gcc.contains("fptrunc"), "{gcc}");
    assert_eq!(clang.matches("fptrunc.f16").count(), 3, "{clang}");
}

/// Both compilers round a statement expression's `_Float16` value.
#[test]
fn statement_expressions_round_their_value() {
    let source = "float f(_Float16 a, _Float16 b) { return ({ a * b; }) / b; }";
    for identity in [CompilerIdentity::Clang, CompilerIdentity::Gcc] {
        let text = lowered_as(source, identity);
        assert!(text.contains("fptrunc.f16"), "{identity:?}\n{text}");
    }
}

/// A `__bf16` computes in `f32` as a `_Float16` does. An integer becomes
/// one through `float` for Clang, rounding twice as LLVM converts it, and
/// once for GCC, through `double`, the x87 format, or `sitofp` for `i128`.
#[test]
fn bfloat16_rounds_integers_as_the_claimed_compiler_does() {
    let source = "__bf16 f(__bf16 a, __bf16 b) { return a * b + a; }\n__bf16 i(int x) { return x; \
                  }\n__bf16 l(long x) { return x; }\n__bf16 w(__int128 x) { return x; }";
    let gcc = lowered_as(source, CompilerIdentity::Gcc);
    let clang = lowered_as(source, CompilerIdentity::Clang);
    for text in [&gcc, &clang] {
        assert!(
            text.contains("fmul.f32") && text.contains("fadd.f32"),
            "{text}"
        );
    }
    assert_eq!(gcc.matches("fptrunc.bf16").count(), 3, "{gcc}");
    assert_eq!(clang.matches("fptrunc.bf16").count(), 4, "{clang}");
    for conversion in ["sitofp.f64", "sitofp.f80", "sitofp.bf16"] {
        assert!(gcc.contains(conversion), "{gcc}");
        assert!(!clang.contains(conversion), "{clang}");
    }
    assert_eq!(clang.matches("sitofp.f32").count(), 3, "{clang}");
}

/// GCC evaluates `__bf16 + _Float16` in `float`, widening both operands;
/// Clang rounds the `__bf16` to `_Float16` through `float` first.
#[test]
fn mixed_halves_meet_in_float_for_gcc() {
    let source = "float f(__bf16 b, _Float16 h) { return b + h; }";
    let gcc = lowered_as(source, CompilerIdentity::Gcc);
    let clang = lowered_as(source, CompilerIdentity::Clang);
    assert!(!gcc.contains("fptrunc"), "{gcc}");
    assert_eq!(clang.matches("fptrunc.f16").count(), 2, "{clang}");
}

/// `_Complex _Float16` multiplies in `float _Complex` through `__mulsc3`
/// and rounds both parts once, at the return.
#[test]
fn complex_half_computes_in_complex_float() {
    let source =
        "_Complex _Float16 f(_Complex _Float16 a, _Complex _Float16 b) { return a * b * a; }";
    for identity in [CompilerIdentity::Clang, CompilerIdentity::Gcc] {
        let text = lowered_as(source, identity);
        assert_eq!(text.matches("__mulsc3").count(), 3, "{identity:?}\n{text}");
        assert_eq!(
            text.matches("fptrunc.f16").count(),
            2,
            "{identity:?}\n{text}"
        );
    }
}
