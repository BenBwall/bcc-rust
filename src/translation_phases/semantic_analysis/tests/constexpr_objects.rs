//! C23 `constexpr` objects and compound literals: the constraints on them,
//! and their names and `.` members as named constants under each identity.
//! C23 §6.7.2p2-6 and p16-17, §6.6p7-15, §6.5.3.6p4-7.

use std::fmt::Write as _;

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        CompilerIdentity,
        ExtensionPolicy,
    },
    translation_phases::semantic_analysis::errors::ConstexprFault,
};

/// The semantic error kinds of `source` in C23 under `identity`.
fn kinds(source: &str, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(identity)
            .with_target(crate::target::Target::LinuxGnu),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    result.push(error.kind);
                }
            }
        },
    );
    result
}

/// Asserts that `source` has no semantic errors under either identity.
fn holds(source: &str) {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(kinds(source, identity), [], "{identity:?}: {source}");
    }
}

/// Asserts that `source` reports exactly `fault` under `identity`.
fn rejects(source: &str, identity: CompilerIdentity, fault: ConstexprFault) {
    assert_eq!(
        kinds(source, identity),
        [SemanticErrorKind::InvalidConstexpr(fault)],
        "{identity:?}: {source}"
    );
}

#[test]
fn named_constants_are_integer_constant_expression_operands() {
    holds(
        "constexpr int n = 4; int a[n]; _Static_assert(n == 4); _Static_assert(sizeof a == 16); \
         enum { E = n * 2 }; struct s { int x; double d; int c[2]; }; constexpr struct s v = { 3, \
         2.5, { 7, 8 } }; _Static_assert(v.x == 3); struct b { int f : v.x; }; alignas(n) char c; \
         union u { int i; float f; }; constexpr union u w = { 5 }; _Static_assert(w.i == 5); void \
         f(int k) { constexpr long m = 6; static int s = m; switch (k) { case m: case v.x: break; \
         } int bounded[m]; _Static_assert(sizeof bounded == 6 * sizeof(int)); }",
    );
}

#[test]
fn constexpr_objects_are_const_with_internal_linkage() {
    holds(
        "constexpr int a = 1; extern const int a; _Static_assert(a == 1); \
         _Static_assert(_Generic(&a, const int *: 1, default: 0)); constexpr int *p = nullptr; \
         _Static_assert(_Generic(&p, int *const *: 1, default: 0)); constexpr char s[] = \"ab\"; \
         _Static_assert(_Generic(&s, const char (*)[3]: 1, default: 0)); int *q = p; struct t { \
         int x; }; constexpr struct t k = { 2 }; struct t copy = k;",
    );
}

#[test]
fn constexpr_initializers_hold_to_the_constraints() {
    use ConstexprFault as F;
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        for (source, fault) in [
            ("void f(int g) { constexpr int a = g; }", F::NotConstant),
            ("constexpr int *p = (int *)64;", F::PointerNotNull),
            ("constexpr int a = 5.0;", F::IntegerNotIce),
            ("constexpr unsigned a = -1;", F::NotRepresentable),
            ("constexpr signed char a = 1234;", F::NotRepresentable),
            ("constexpr float f = 0.1;", F::NotRepresentable),
            ("constexpr float f = 16777217;", F::NotRepresentable),
            (
                "constexpr double d = __builtin_complex(1.0, 0.0);",
                F::ComplexToReal,
            ),
            ("constexpr volatile int a = 1;", F::Qualifiers),
            ("constexpr _Atomic int a = 1;", F::Qualifiers),
            (
                "struct s { int *restrict p; }; constexpr struct s a = {};",
                F::MemberQualifiers,
            ),
            (
                "constexpr unsigned char u[] = \"\\x80\";",
                F::NotRepresentable,
            ),
            ("constexpr int a;", F::MissingInitializer),
            ("constexpr int f(void);", F::NotObject),
            (
                "void f(int n) { constexpr int (*p)[n] = nullptr; }",
                F::VariablyModified,
            ),
        ] {
            rejects(source, identity, fault);
        }
    }
    holds(
        "constexpr float f = 0.5; constexpr double d = 16777217; constexpr float g = 1ULL << 63; \
         constexpr unsigned u = -1U; constexpr char c[] = u8\"x\"; constexpr volatile int *p = \
         nullptr; constexpr double nan = __builtin_nan(\"\"); constexpr float qf = \
         __builtin_nan(\"\");",
    );
}

#[test]
fn only_the_initialized_union_member_is_a_named_constant() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        rejects(
            "union u { int a; int b; }; void f(void) { constexpr union u v = { .a = 1 }; \
             constexpr int x = v.b; }",
            identity,
            ConstexprFault::NotConstant,
        );
    }
}

#[test]
fn identities_differ_on_floating_casts_and_strings() {
    // GCC takes a cast of a floating named constant as an integer constant
    // expression operand; Clang folds it, and takes any integer constant
    // of integer type for an integer object.
    let source =
        "constexpr double d = 2.0; enum { E = (int)d }; constexpr int i = (int)(double)1.0;";
    assert_eq!(
        kinds(source, CompilerIdentity::Gcc),
        [SemanticErrorKind::InvalidConstexpr(
            ConstexprFault::IntegerNotIce
        )]
    );
    assert_eq!(kinds(source, CompilerIdentity::Clang), []);
    // Clang reads a narrow string's elements as signed char, GCC as char.
    assert_eq!(
        kinds(
            "constexpr signed char s[] = \"\\xff\";",
            CompilerIdentity::Gcc
        ),
        []
    );
    // Clang checks no complex value.
    let complex = "constexpr _Complex float z = 0.1;";
    assert_eq!(
        kinds(complex, CompilerIdentity::Gcc),
        [SemanticErrorKind::InvalidConstexpr(
            ConstexprFault::NotRepresentable
        )]
    );
    assert_eq!(kinds(complex, CompilerIdentity::Clang), []);
}

#[test]
fn gcc_rejects_underspecified_forms() {
    use ConstexprFault as F;
    for (source, fault) in [
        ("constexpr int a = 1, b = 2;", F::MultipleDeclarators),
        ("constexpr int a = sizeof a;", F::SelfReference),
        (
            "static const int a; constexpr const int a = 1;",
            F::Redeclared,
        ),
    ] {
        rejects(source, CompilerIdentity::Gcc, fault);
        assert_eq!(kinds(source, CompilerIdentity::Clang), [], "{source}");
    }
    assert!(matches!(
        kinds(
            "constexpr int a = sizeof (struct t { int x; });",
            CompilerIdentity::Gcc
        )[..],
        [SemanticErrorKind::InvalidInference(_)]
    ));
}

#[test]
fn constexpr_compound_literals_are_named_constants() {
    let gcc = |source| kinds(source, CompilerIdentity::Gcc);
    assert_eq!(
        gcc(
            "struct s { int a; }; enum { E = (constexpr struct s){ 3 }.a }; \
             _Static_assert((constexpr int){ 4 } == 4); constexpr int k = (constexpr int){ 5 }; \
             int x = (constexpr int){ 1 } + k;"
        ),
        []
    );
    assert_eq!(
        gcc("void f(void) { (constexpr unsigned char){ -1 }; }"),
        [SemanticErrorKind::InvalidConstexpr(
            ConstexprFault::NotRepresentable
        )]
    );
}

/// A bit-field member of a named constant is one too, with the value of
/// its width: in a static assertion, an enumerator and an array bound,
/// through copies and after a designator writes it again.
#[test]
fn bit_field_members_are_named_constants() {
    holds(
        "struct S { unsigned x : 3; int y : 3; bool b : 1; unsigned z; }; constexpr struct S s = \
         { 3, -4, 1, 7 }; static_assert(s.x == 3); static_assert(s.y == -4); static_assert(s.b); \
         static_assert(s.z == 7); enum { E = s.x + s.y }; static_assert(E == -1); constexpr \
         struct S t = s; static_assert(t.x == 3); int a[t.x]; static_assert(sizeof a == 3 * \
         sizeof(int)); struct A { int k; struct { unsigned u : 4; }; }; constexpr struct A q = { \
         1, { 9 } }; static_assert(q.u == 9);",
    );
    // The later designator gives the value; GCC's `-Woverride-init` and
    // Clang's `-Winitializer-overrides` report the earlier one.
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            kinds(
                "struct S { unsigned x : 3; int y : 3; }; constexpr struct S r = { .x = 1, .y = \
                 2, .x = 5 }; static_assert(r.x == 5 && r.y == 2);",
                identity
            ),
            [SemanticErrorKind::InitializerOverride],
            "{identity:?}"
        );
    }
}

/// GCC rejects a value a bit-field's width cannot hold; Clang takes it
/// truncated to the width.
#[test]
fn bit_field_width_bounds_the_value_under_gcc() {
    for source in [
        "struct S { unsigned x : 3; }; constexpr struct S s = { 8 };",
        "struct S { int y : 3; }; constexpr struct S s = { 4 };",
    ] {
        rejects(
            source,
            CompilerIdentity::Gcc,
            ConstexprFault::NotRepresentable,
        );
    }
    assert_eq!(
        kinds(
            "struct S { unsigned x : 3; int y : 3; }; constexpr struct S s = { 8, 4 }; \
             static_assert(s.x == 0); static_assert(s.y == -4);",
            CompilerIdentity::Clang
        ),
        []
    );
}

/// An integer of more than 64 bits initializes a `_Float128` when the
/// type holds its value exactly.
#[test]
fn wide_integers_convert_exactly_to_binary128() {
    assert_eq!(
        kinds(
            "constexpr _BitInt(128) n = 1; constexpr _Float128 f = n; constexpr unsigned \
             _BitInt(128) m = (unsigned _BitInt(128))1 << 127; constexpr _Float128 g = m; \
             constexpr _BitInt(128) k = -((_BitInt(128))1 << 126); constexpr _Float128 h = k;",
            CompilerIdentity::Gcc
        ),
        []
    );
    rejects(
        "constexpr _BitInt(128) n = ((_BitInt(128))1 << 120) + 1; constexpr _Float128 f = n;",
        CompilerIdentity::Gcc,
        ConstexprFault::NotRepresentable,
    );
}

/// A chain of structure copies stays a named constant however long it is.
#[test]
fn long_copy_chains_stay_named_constants() {
    let mut source = String::from("struct S { int n; }; constexpr struct S s0 = { 42 };");
    for index in 1..=200 {
        write!(source, " constexpr struct S s{index} = s{};", index - 1).unwrap();
    }
    source.push_str(" static_assert(s200.n == 42);");
    holds(&source);
    // A copy of a union's largest member covers the union, yet the union
    // keeps the member its initializer selects.
    holds(
        "struct s { int a; int b; }; union u { int i; struct s e; }; constexpr struct s s0 = { 1, \
         7 }; constexpr union u v = { .e = s0 }; static_assert(v.e.b == 7); constexpr union u w = \
         v; static_assert(w.e.a == 1);",
    );
}

/// Clang 23 parses no storage-class specifier in a compound literal, so
/// its identity rejects `constexpr` and the others there.
#[test]
fn clang_rejects_storage_classes_in_compound_literals() {
    for source in [
        "int a = (constexpr int){ 1 };",
        "void f(void) { (void)(static int){ 1 }; }",
        "void f(void) { (void)(register int){ 1 }; }",
        "void f(void) { (void)(thread_local int){ 1 }; }",
    ] {
        assert_eq!(
            kinds(source, CompilerIdentity::Clang),
            [SemanticErrorKind::LiteralStorageClass],
            "{source}"
        );
    }
}
