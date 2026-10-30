//! GNU vectors in memory, computed lane by lane, and passed at calls as
//! the claimed compiler's convention passes them.

use super::*;

/// The values the refusal once covered lower now: assignment copies the
/// object, a load through a vector pointer copies from it, a literal stores
/// its lanes and a subscript loads one lane at its offset, with the
/// alignment the offset leaves.
#[test]
fn vector_values_lower_in_memory() {
    for (source, wanted) in [
        (
            "typedef int V __attribute__((vector_size(16))); V x, y; void f(void) { x = y; }",
            "copy",
        ),
        (
            "typedef int V __attribute__((vector_size(16))); V x; void f(int *p) { x = *(V *)p; }",
            "copy",
        ),
        (
            "typedef char V __attribute__((vector_size(4))); char a[4]; void f(void) { *(V *)a = \
             (V){0, 1, 2, 3}; }",
            "store.i8",
        ),
        (
            "typedef int V __attribute__((vector_size(16))); V x; int f(void) { return x[2]; }",
            "load.i32 v3, align 8",
        ),
    ] {
        let text = functions(&lowered(source));
        assert!(text.contains(wanted), "{source}\n{text}");
    }
}

/// An operator works on each lane without the integer promotions and
/// without `nsw`; a comparison sign-extends its truth into a mask lane; a
/// scalar operand is converted once and stored into every lane.
#[test]
fn operators_work_lane_by_lane() {
    let text = functions(&lowered(
        "typedef signed char V __attribute__((vector_size(4)));\nV add(V a, V b) { return a + b; \
         }\nV less(V a, V b) { return a < b; }\nV splat(V a, signed char s) { return a * s; }",
    ));
    assert_eq!(text.matches("iadd.i8 v").count(), 4, "{text}");
    assert!(!text.contains("nsw"), "{text}");
    assert_eq!(text.matches("icmp.i8 slt").count(), 4, "{text}");
    assert_eq!(text.matches("sext.i8").count(), 4, "{text}");
    assert_eq!(text.matches("imul.i8").count(), 4, "{text}");
}

/// System V parameters and results record the registers each compiler
/// uses: one SSE register for 16 bytes, a general register for 4, and, for
/// Clang only, XMM0 and XMM1 for a 32-byte result and an integer for a
/// one-lane `float`, which GCC passes in memory.
#[test]
fn vector_parameters_record_each_compilers_registers() {
    use crate::configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    };
    let source = "typedef float V __attribute__((vector_size(16)));\ntypedef float W \
                  __attribute__((vector_size(32)));\ntypedef float S \
                  __attribute__((vector_size(4)));\ntypedef char C \
                  __attribute__((vector_size(4)));\nV f(V a) { return a; }\nW g(W b) { return b; \
                  }\nS h(S c) { return c; }\nC k(C c) { return c; }";
    for (identity, wanted) in [
        (
            CompilerIdentity::Clang,
            [
                "@f(ptr result(size 16, align 16, vector, [0: f128]), ptr aggregate(size 16, \
                 align 16, vector, [0: f128]) noundef)",
                "@g(ptr result(size 32, align 32, vector, [0: f128, 16: f128]), ptr \
                 aggregate(size 32, align 32, vector) noundef)",
                "@h(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
                "@k(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
            ],
        ),
        (
            CompilerIdentity::Gcc,
            [
                "@f(ptr result(size 16, align 16, vector, [0: f128]), ptr aggregate(size 16, \
                 align 16, vector, [0: f128]) noundef)",
                "@g(ptr result(size 32, align 32, vector), ptr aggregate(size 32, align 32, \
                 vector) noundef)",
                "@h(ptr result(size 4, align 4, vector), ptr aggregate(size 4, align 4, vector) \
                 noundef)",
                "@k(ptr result(size 4, align 4, vector, [0: i32]), ptr aggregate(size 4, align 4, \
                 vector, [0: i32]) noundef)",
            ],
        ),
    ] {
        let configuration = CompilerConfiguration::default().with_compiler_identity(identity);
        let text = lower_with(source, configuration).expect("lowers");
        for signature in wanted {
            assert!(
                text.contains(signature),
                "{identity:?}: {signature}\n{text}"
            );
        }
    }
}

/// The vector builtins lowering does not expand are refused by name rather
/// than called.
#[test]
fn unexpanded_vector_builtins_are_refused() {
    let errors = lower(
        "typedef int V __attribute__((vector_size(16))); V f(V a, V b) { return \
         __builtin_elementwise_max(a, b); }",
    )
    .expect_err("refused");
    assert_eq!(errors, ["not yet supported by lowering: this builtin"]);
}
