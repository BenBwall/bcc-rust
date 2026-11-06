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
        (
            "int f(int b, unsigned a) { return b == (a ? 1U : 2U); }",
            1,
            0,
        ),
        (
            "int f(int b, int c) { return b == (c ? 1U : 0x7FFFFFFFU); }",
            1,
            0,
        ),
        (
            "int f(int b, int c) { return b == (c ? 1U : 0x80000000U); }",
            1,
            1,
        ),
        (
            "int f(int b, unsigned a, int c) { return b == (c ? a : 2U); }",
            1,
            1,
        ),
        ("int f(int b, unsigned a) { return b == (a = 1U); }", 1, 0),
        ("int f(int b, unsigned a) { return b == (a >> 4); }", 1, 0),
        ("int f(int b, unsigned a) { return b == a / 2U; }", 1, 0),
        ("int f(int b, unsigned a) { return b == a + 1U; }", 1, 1),
        (
            "int f(int b, unsigned char c) { return b == c + 1U; }",
            1,
            0,
        ),
        (
            "int f(int b, unsigned char c) { return b == (c ? c : 1U); }",
            1,
            0,
        ),
        (
            "int f(long b, unsigned long a) { return b == (a >> 1); }",
            1,
            0,
        ),
        (
            "struct S { unsigned long l : 40; }; int f(struct S *p, long b) { return b == p->l; }",
            0,
            0,
        ),
        ("const int k = 5; int f(unsigned b) { return b < k; }", 1, 0),
        (
            "const int k = -5; int f(unsigned b) { return b < k; }",
            1,
            1,
        ),
        (
            "int f(int a, unsigned b) { return (a & 1) - (a & 2) < b; }",
            1,
            0,
        ),
        ("int f(int a, unsigned b) { return (1 << a) < b; }", 1, 0),
        (
            "int f(unsigned a, int b) { return (b < 0 ? 0 : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b <= 0 ? 0 : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b > 0 ? b : 0) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (0 > b ? 0 : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b < 5 ? 5 : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b < 1 ? 0 : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b >= 1 ? b : 0) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b < 7 ? 5 : b) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b > 5 ? 5 : b) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b < -5 ? -5 : b) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b, int c) { return ((b & 1) < c ? c : (b & 1)) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b, int c) { return (b < (c & 1) ? b : (c & 1)) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b, int c) { return (c < 0 ? 0 : b) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b < 0 ? -b : b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (b >= 0 ? b : -b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (0 > b ? -b : b) < a; }",
            0,
            1,
        ),
        (
            "unsigned f(int a, unsigned b, int c) { return c ? (a < 0 ? 0 : a) : b; }",
            0,
            0,
        ),
        ("int f(unsigned a, int b) { return b * b < a; }", 0, 1),
        (
            "int f(unsigned a, int b, int c) { return b * c < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return __builtin_abs(b) < a; }",
            0,
            1,
        ),
        (
            "int abs(int); int f(unsigned a, int b) { return abs(b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return __builtin_popcount(b) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return __builtin_ctz(b) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return ((b & 7) << 1) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return ((b & 7) << 29) < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned a, int b) { return ((b & 7) >> b) < a; }",
            1,
            0,
        ),
        ("int f(unsigned a, int b) { return (b = 1) < a; }", 0, 0),
        ("int f(unsigned a, int b) { return (b = -1) < a; }", 1, 1),
        (
            "int f(unsigned a, int b) { return (b = (a & 1)) < a; }",
            0,
            0,
        ),
        (
            "int f(unsigned a, unsigned char b) { return (int)b < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, unsigned short b) { return (short)b < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned long a, unsigned b) { return (int)b < a; }",
            1,
            1,
        ),
        (
            "int f(unsigned long a, unsigned b) { return (long)b < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, int b) { return (short)(b & 1) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, long b) { return (int)(b & 1) < a; }",
            0,
            1,
        ),
        (
            "int f(unsigned a, long b) { return (int)(b & 0xffffffff) < a; }",
            1,
            1,
        ),
        ("int f(int a, unsigned b) { return ({ 1; }) < b; }", 0, 0),
        (
            "int f(int a, unsigned b) { return ({ a & 1; }) < b; }",
            0,
            1,
        ),
        (
            "int f(int a, unsigned b) { return ({ int c = 1; c; }) < b; }",
            1,
            1,
        ),
        ("int f(void) { return -1 < 0U; }", 0, 0),
        ("int f(void) { return -1 == 0xFFFFFFFFU; }", 1, 0),
        ("int g = -1 < 0U;", 0, 0),
        ("int f(unsigned b) { return b < -1; }", 1, 1),
        ("int f(unsigned b) { return b >= -1; }", 1, 1),
        ("int f(unsigned b) { return b <= -1; }", 1, 0),
        ("int f(unsigned b) { return b > -1; }", 1, 0),
        ("int f(int b) { return b < 0U; }", 0, 0),
        ("int f(int b) { return b >= 0U; }", 0, 0),
        ("int f(int b) { return 0U > b; }", 0, 0),
        ("int f(int b) { return 0U <= b; }", 0, 0),
        ("int f(int b) { return b > 0U; }", 1, 1),
        ("#define Z 0U\nint f(int b) { return b < Z; }", 0, 1),
        (
            "enum { E = -1 }; int f(unsigned b) { return b <= E; }",
            1,
            1,
        ),
        ("int f(unsigned char c) { return (unsigned)c < -1; }", 0, 1),
        ("int f(unsigned char c) { return (unsigned)c == -1; }", 0, 1),
        ("int f(short c) { return c == 0x80000000U; }", 0, 0),
        ("int f(short c) { return c == 0xFFFFFFFFU; }", 1, 1),
        ("int f(short c) { return c < 70000U; }", 1, 1),
        ("int f(int a, unsigned b) { return sizeof(a < b); }", 0, 0),
        (
            "void f(int a, unsigned b) { __typeof__(a < b) c = 0; (void)c; }",
            0,
            0,
        ),
        ("int f(int a, unsigned b) { return 0 && a < b; }", 0, 0),
        ("int f(int a, unsigned b) { return 1 || a < b; }", 0, 0),
        ("int f(int a, unsigned b) { return 1 ? 1 : a < b; }", 0, 0),
        ("int f(int a, unsigned b) { return 0 ? a < b : 1; }", 0, 0),
        ("int f(int a, unsigned b) { return 1 && a < b; }", 1, 1),
        (
            "int f(int a, unsigned b, int c) { if (0 && a < b) return c; return 1; }",
            0,
            0,
        ),
        (
            "int f(int a, unsigned b) { return _Generic(a < b, int: 1); }",
            0,
            0,
        ),
        (
            "int f(int a, unsigned b) { return __builtin_choose_expr(1, 1, a < b); }",
            1,
            0,
        ),
        ("int f(int a, unsigned b) { return 1; return a < b; }", 1, 0),
        (
            "int f(int a, unsigned b) { if (0) return a < b; return 1; }",
            1,
            0,
        ),
        (
            "void g(void) __attribute__((noreturn)); int f(int a, unsigned b) { g(); return a < \
             b; }",
            1,
            0,
        ),
        (
            "int f(int a, unsigned b, int c) { return sizeof(c ? a : b); }",
            0,
            0,
        ),
        (
            "int f(__int128 a) { return a == (unsigned __int128)1 << 127; }",
            1,
            1,
        ),
        (
            "int f(__int128 a, unsigned __int128 b) { return a < b; }",
            1,
            1,
        ),
        ("int f(unsigned __int128 a) { return a < -1; }", 1, 1),
        ("int f(unsigned __int128 a) { return a <= -1; }", 1, 0),
        ("int f(unsigned _BitInt(200) a) { return a < -1; }", 1, 1),
        ("int f(_Atomic int a, unsigned b) { return a < b; }", 1, 0),
        ("int f(int a, _Atomic unsigned b) { return a < b; }", 1, 1),
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
