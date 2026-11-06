//! `__builtin_clear_padding` as IR: one fill for a run of padding, a load,
//! a mask and a store for a byte that shares padding with value bits, nothing
//! for an object without padding, and a loop for an array that would otherwise
//! take many operations. What each fills is compared with what GCC 14.2 and
//! Clang 23.1 clear, by running them, in
//! `tests/fixtures/programs/builtin_clear_padding.c`. GNU extension: GCC
//! manual, "Other Builtins".

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
};

fn gcc() -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_gnu_extensions(true)
        .with_compiler_identity(CompilerIdentity::Gcc)
}

/// Counts the lines of `text` that hold `what`.
fn count(text: &str, what: &str) -> usize {
    text.lines().filter(|line| line.contains(what)).count()
}

#[test]
fn a_run_of_padding_is_one_fill_and_a_scalar_has_none() {
    let text = lowered("void f(struct { char a; int b; } *p) { __builtin_clear_padding(p); }");
    assert_eq!(count(&text, "fill "), 1, "{text}");
    assert!(text.contains("iconst.i64 3"), "{text}");
    assert_eq!(count(&text, "load."), 0, "{text}");
    let text = lowered("void f(int *p) { __builtin_clear_padding(p); }");
    assert_eq!(count(&text, "fill "), 0, "{text}");
    assert_eq!(count(&text, "store."), 0, "{text}");
}

#[test]
fn a_byte_shared_with_a_bit_field_is_frozen_and_masked() {
    let text = lowered(
        "void f(struct { int a : 3; int : 2; char c; } *p) { __builtin_clear_padding(p); }",
    );
    assert_eq!(count(&text, "load.i8"), 1, "{text}");
    assert_eq!(count(&text, "freeze.i8"), 1, "{text}");
    assert!(text.contains("iconst.i8 7"), "{text}");
    assert_eq!(count(&text, "store.i8"), 1, "{text}");
    assert_eq!(count(&text, "fill "), 1, "{text}");
}

#[test]
fn padding_of_neighboring_elements_joins_into_one_fill() {
    // Bytes 0, 2-3 and 5: the last byte of one element and the first of the
    // next make one run.
    let text = lowered(
        "void f(struct { char : 8; char a; char : 8; } (*p)[2]) { __builtin_clear_padding(p); }",
    );
    assert_eq!(count(&text, "fill "), 3, "{text}");
    // Elements of nothing but padding are one run with what comes before.
    let text = lowered(
        "void f(struct { long long a; char b[0]; struct { char : 8; } c[9]; } *p) { \
         __builtin_clear_padding(p); }",
    );
    assert_eq!(count(&text, "fill "), 1, "{text}");
    assert_eq!(count(&text, "load."), 0, "{text}");
}

#[test]
fn a_small_array_is_inline_and_a_large_one_a_loop() {
    let text = lowered(
        "struct p { char a; short b; }; void f(struct p (*q)[3]) { __builtin_clear_padding(q); }",
    );
    assert_eq!(count(&text, "fill "), 3, "{text}");
    assert_eq!(count(&text, "brif "), 0, "{text}");
    for source in [
        "struct p { char a; short b; }; void f(struct p (*q)[33]) { __builtin_clear_padding(q); }",
        "struct p { char a; short b; }; void f(struct p (*q)[1000000000000]) { \
         __builtin_clear_padding(q); }",
        "struct p { char a; short b; }; void f(struct p (*q)[1000][1000]) { \
         __builtin_clear_padding(q); }",
    ] {
        let text = lowered(source);
        assert_eq!(count(&text, "fill "), 1, "{text}");
        assert_eq!(count(&text, "brif "), 1, "{text}");
    }
    // An array in a record keeps the record's runs around its loop.
    let text = lowered(
        "struct p { char a; short b; }; void f(struct { char a; struct p c[64]; char d; } *q) { \
         __builtin_clear_padding(q); }",
    );
    assert_eq!(count(&text, "brif "), 1, "{text}");
    assert_eq!(count(&text, "fill "), 3, "{text}");
    // Elements without padding need no loop however many there are.
    let text = lowered("void f(char (*q)[1000000000000]) { __builtin_clear_padding(q); }");
    assert_eq!(count(&text, "brif "), 0, "{text}");
    assert_eq!(count(&text, "fill "), 0, "{text}");
}

#[test]
fn a_variable_length_array_is_a_loop_under_gcc_and_nothing_under_clang() {
    let source =
        "void f(int n, struct { char a; short b; } (*p)[n][2]) { __builtin_clear_padding(p); }";
    let text = lower_with(source, gcc()).unwrap();
    assert_eq!(count(&text, "brif "), 1, "{text}");
    assert_eq!(count(&text, "fill "), 2, "{text}");
    let text = lowered(source);
    assert_eq!(count(&text, "brif "), 0, "{text}");
    assert_eq!(count(&text, "fill "), 0, "{text}");
}

#[test]
fn value_bits_follow_the_target() {
    let source = "void f(long double *p) { __builtin_clear_padding(p); }";
    assert_eq!(
        count(&lower_for(source, Target::LinuxGnu).unwrap(), "fill "),
        1
    );
    assert_eq!(
        count(&lower_for(source, Target::WindowsGnu).unwrap(), "fill "),
        1
    );
    // `long double` is binary64 there, and binary128 on WebAssembly.
    for target in [
        Target::WindowsMsvc,
        Target::Wasm32Wasi,
        Target::Wasm32Unknown,
    ] {
        assert_eq!(
            count(&lower_for(source, target).unwrap(), "fill "),
            0,
            "{target:?}"
        );
    }
    // Clang's Microsoft record layout: bytes 1-7, 13-15 and 17-23 are padding
    // and byte 0 holds a three-bit field.
    let text = lower_for(
        "void f(struct { int a : 3; long long b : 40; char c; } *p) { __builtin_clear_padding(p); \
         }",
        Target::WindowsMsvc,
    )
    .unwrap();
    assert_eq!(count(&text, "fill "), 3, "{text}");
    assert_eq!(count(&text, "load.i8"), 1, "{text}");
}

#[test]
fn a_type_too_large_to_describe_is_refused_by_name() {
    let refused = |source: &str| {
        let errors = lower_with(source, gcc()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("__builtin_clear_padding")),
            "{source}: {errors:?}"
        );
    };
    // A union no member covers with value bits is merged on a copy of it.
    refused(
        "union u { struct { char a; short b; } c[1000000]; struct { short d; char e; } f; \
         };\nvoid g(union u *p) { __builtin_clear_padding(p); }",
    );
    // Records that double in size, which no array compresses.
    let mut source = String::from("struct s0 { char a; int b; };\n");
    for level in 1..=24 {
        source.push_str(&format!(
            "struct s{level} {{ struct s{previous} a; char b; struct s{previous} c; }};\n",
            previous = level - 1
        ));
    }
    source.push_str("void g(struct s24 *p) { __builtin_clear_padding(p); }");
    refused(&source);
}

#[test]
fn large_unions_a_member_covers_need_no_copy() {
    let text = lowered(
        "union u { char a[100000000]; struct { char b; int c; } d; };\nvoid g(union u *p) { \
         __builtin_clear_padding(p); }",
    );
    assert_eq!(count(&text, "fill "), 0, "{text}");
}
