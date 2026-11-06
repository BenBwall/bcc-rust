//! `-Wsign-compare` and `-Wmissing-field-initializers`, pinned against
//! GCC 14.2 and Clang 23.1.1.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
};

fn reports(source: &str, identity: CompilerIdentity, kind: SemanticErrorKind) -> usize {
    let mut count = 0;
    let configuration = CompilerConfiguration::default()
        .with_compiler_identity(identity)
        .with_gnu_extensions(true)
        .with_extra_warnings(true);
    with_configuration(source, configuration, |context, _| {
        count = context
            .take_pending_errors()
            .iter()
            .filter(|error| matches!(error, TranslationError::Semantic(e) if e.kind == kind))
            .count();
    });
    count
}

#[test]
fn sign_compare_follows_each_compiler() {
    for (source, gcc, clang) in [
        ("int f(int a, unsigned b) { return a < b; }", 1, 1),
        (
            "int f(unsigned u, int a) { return ((0x3000 & u) >> 12) != a; }",
            1,
            0,
        ),
        ("int f(int a, unsigned b) { return a == b; }", 1, 1),
        ("int f(int a, unsigned long b) { return a < b; }", 1, 1),
        ("int f(short a, unsigned b) { return a < b; }", 1, 1),
        ("int f(long a, unsigned b) { return a < b; }", 0, 0),
        ("int f(int a, unsigned short b) { return a < b; }", 0, 0),
        ("int f(unsigned char a, int b) { return a < b; }", 0, 0),
        ("int f(unsigned char a, unsigned b) { return a < b; }", 0, 0),
        ("int f(int a, unsigned b) { return (unsigned)a < b; }", 0, 0),
        ("int f(int a, unsigned b) { return a < (int)b; }", 0, 0),
        ("int f(unsigned b) { return 1 < b; }", 0, 0),
        ("int f(unsigned b) { return b == 0; }", 0, 0),
        ("int f(unsigned b) { return b > 0; }", 0, 0),
        ("int f(unsigned b) { return -1 < b; }", 1, 0),
        ("int f(unsigned b) { return b == -1; }", 1, 1),
        ("int f(int a, unsigned b) { return (a & 0xff) < b; }", 0, 0),
        ("int f(int a, unsigned b) { return (a & -1) < b; }", 1, 1),
        ("int f(int a, unsigned b) { return (a ? 1 : 2) < b; }", 0, 0),
        (
            "int f(int a, unsigned b) { return (a ? 1 : -1) < b; }",
            1,
            1,
        ),
        ("int f(int a, unsigned b) { return (a < 0) < b; }", 0, 0),
        ("int f(int a, unsigned b) { return a % 2 < b; }", 1, 1),
        ("int f(int a) { return a < sizeof a; }", 1, 1),
        ("int f(int a) { return a == sizeof a; }", 0, 0),
        (
            "enum E { A, B }; int f(enum E e, unsigned b) { return e < b; }",
            0,
            0,
        ),
        (
            "enum E { A, B }; int f(enum E e, int b) { return e < b; }",
            1,
            1,
        ),
        ("enum E { A, B }; int f(unsigned b) { return A < b; }", 0, 0),
        (
            "enum E { A = -1, B }; int f(enum E e, unsigned b) { return e < b; }",
            1,
            1,
        ),
        (
            "struct S { unsigned x : 3; }; int f(struct S s, int a) { return s.x < a; }",
            0,
            0,
        ),
        (
            "struct S { int y : 3; }; int f(struct S s, unsigned b) { return s.y < b; }",
            1,
            1,
        ),
        (
            "struct S { unsigned z : 32; }; int f(struct S s, int a) { return s.z < a; }",
            1,
            1,
        ),
        ("int f(float a, unsigned b) { return a < b; }", 0, 0),
        ("int f(_Bool a, unsigned b) { return a < b; }", 0, 0),
        (
            "int f(unsigned short a, unsigned short b, unsigned c) { return (a + b) < c; }",
            0,
            0,
        ),
        ("int f(int *p, int *q) { return p < q; }", 0, 0),
        (
            "unsigned f(int a, unsigned b, int c) { return c ? a : b; }",
            1,
            0,
        ),
        ("unsigned f(unsigned b, int c) { return c ? 1 : b; }", 0, 0),
        ("unsigned f(unsigned b, int c) { return c ? -1 : b; }", 1, 0),
    ] {
        let kind = SemanticErrorKind::SignCompare;
        assert_eq!(
            reports(source, CompilerIdentity::Gcc, kind),
            gcc,
            "GCC: {source}"
        );
        assert_eq!(
            reports(source, CompilerIdentity::Clang, kind),
            clang,
            "Clang: {source}"
        );
    }
}

#[test]
fn missing_fields_follow_each_compiler() {
    const S: &str = "struct S { int a, b; }; struct T { struct S s; int c; }; ";
    for (source, gcc, clang) in [
        ("struct S s = {1};", 1, 1),
        ("struct S s = {0};", 0, 0),
        ("struct S s = {};", 0, 0),
        ("struct S s = {1, 2};", 0, 0),
        ("struct S s = {.a = 1};", 0, 0),
        ("void f(void) { struct S s = {1}; (void)s; }", 1, 1),
        ("struct T t = {{1}};", 2, 2),
        ("struct T t = {{1, 2}, 3};", 0, 0),
        ("struct T t = {.s = {1}, .c = 1};", 0, 1),
        ("struct T t = {.s.a = 1};", 0, 0),
        ("struct S t[2] = {{1}, {2, 3}};", 1, 1),
        ("struct S t[2] = {{1, 2}};", 0, 0),
        ("int a[3] = {1};", 0, 0),
        ("struct S *p = &(struct S){1};", 1, 1),
        ("struct F { int a; int b[]; }; struct F f = {1};", 0, 0),
        (
            "struct B { int a; int : 3; int b; }; struct B b = {1};",
            1,
            1,
        ),
        (
            "struct B { int a; int b; int : 3; }; struct B b = {1, 2};",
            0,
            0,
        ),
        ("union U { int a, b; }; union U u = {1};", 0, 0),
        (
            "struct C { char a[2]; int b; }; struct C c = {\"x\"};",
            1,
            1,
        ),
    ] {
        let source = format!("{S}{source}");
        let kind = SemanticErrorKind::MissingFieldInitializer;
        assert_eq!(
            reports(&source, CompilerIdentity::Gcc, kind),
            gcc,
            "GCC: {source}"
        );
        assert_eq!(
            reports(&source, CompilerIdentity::Clang, kind),
            clang,
            "Clang: {source}"
        );
    }
}

/// Brace elision is not modeled: such a list stays silent at its level, where
/// the compilers may warn, rather than guessing.
#[test]
fn an_elided_list_stays_silent() {
    let kind = SemanticErrorKind::MissingFieldInitializer;
    let source = "struct S { int a, b; }; struct T { struct S s; int c; }; struct T t = {1, 2};";
    assert_eq!(reports(source, CompilerIdentity::Gcc, kind), 0);
    assert_eq!(reports(source, CompilerIdentity::Clang, kind), 0);
}
