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
