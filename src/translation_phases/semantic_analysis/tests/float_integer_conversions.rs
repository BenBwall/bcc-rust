//! Floating constants converted out of an integer type's range: the value
//! GCC and Clang store in a static object, GCC's `-Woverflow` and pedantic
//! report, Clang's `-Wliteral-conversion`, and the integer constant
//! expressions such a conversion is not. C99 §6.3.1.4p1.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup,
    ExtensionPolicy,
    WarningOption,
};

/// What `source` reports in GNU C17 under `identity` and `policy`:
/// semantic kinds (a conversion warning as its group) and the number of
/// extension reports.
fn diagnostics(
    source: &str,
    identity: CompilerIdentity,
    policy: ExtensionPolicy,
) -> (Vec<Kind>, usize) {
    let mut result = (Vec::new(), 0);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C17, policy)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity),
        |context, _| {
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::FloatOverflow(_),
                        ..
                    }) => result.0.push(Kind::Overflow),
                    | TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::LiteralConversion(_),
                        ..
                    }) => result.0.push(Kind::Literal),
                    | TranslationError::Semantic(e) => result.0.push(Kind::Other(e.kind)),
                    | TranslationError::Extension(_) => result.1 += 1,
                    | other => panic!("{source}: unexpected {other:?}"),
                }
            }
        },
    );
    result
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Kind {
    Overflow,
    Literal,
    Other(SemanticErrorKind),
}

#[test]
fn each_identity_reports_its_own_conversions() {
    use Kind::{
        Literal,
        Overflow,
    };
    // The source, GCC's reports, Clang's reports, and GCC's pedantic
    // overflow reports.
    for (source, gcc, clang, pedantic) in [
        ("static int x = 1e10;", &[Overflow][..], &[Literal][..], 1),
        ("int x = -1e10;", &[Overflow], &[Literal], 1),
        ("int x = (-(1e10));", &[Overflow], &[Literal], 1),
        ("unsigned x = -1.0;", &[Overflow], &[Literal], 1),
        // Truncation toward zero fits; only Clang reports the fraction.
        ("unsigned x = -0.5;", &[], &[Literal], 0),
        ("int x = 2147483647.5;", &[], &[Literal], 0),
        ("int x = -0.0;", &[], &[Literal], 0),
        ("int x = 2.0;", &[], &[], 0),
        ("_Bool x = 1e300;", &[], &[Literal], 0),
        ("_Bool x = 1.0;", &[], &[], 0),
        ("int x = 3e9f;", &[Overflow], &[Literal], 1),
        ("int x = 1e10L;", &[Overflow], &[Literal], 1),
        ("unsigned long long x = 1e20;", &[Overflow], &[Literal], 1),
        // Not a literal for Clang; GCC folds any constant.
        ("int x = __builtin_inf();", &[Overflow], &[], 1),
        ("int x = 1e10 * 2;", &[Overflow], &[], 1),
        // An explicit cast is no implicit conversion; GCC's pedantic report
        // follows the value through arithmetic.
        ("int x = (int)1e10;", &[], &[], 1),
        ("int x[] = { (int)-1e10 / 2 };", &[], &[], 1),
        // GCC saturates a bit-field to its own width.
        ("struct { int b : 4; } x = { 100.0 };", &[Overflow], &[], 1),
        // GCC reports nothing in a block; Clang reports a literal anywhere.
        (
            "void f(void) { int x = 1e10; x = 1.5; (void)x; }",
            &[],
            &[Literal, Literal],
            0,
        ),
        ("void g(int); void f(void) { g(1e10); }", &[], &[Literal], 0),
        ("int f(void) { return 2.5; }", &[], &[Literal], 0),
        (
            "void f(void) { static int x = 1e10; (void)x; }",
            &[Overflow],
            &[Literal],
            1,
        ),
    ] {
        assert_eq!(
            diagnostics(source, CompilerIdentity::Gcc, ExtensionPolicy::Allow),
            (gcc.to_vec(), 0),
            "GCC: {source}"
        );
        assert_eq!(
            diagnostics(source, CompilerIdentity::Gcc, ExtensionPolicy::Warn).1,
            pedantic,
            "GCC pedantic: {source}"
        );
        assert_eq!(
            diagnostics(source, CompilerIdentity::Clang, ExtensionPolicy::Warn),
            (clang.to_vec(), 0),
            "Clang: {source}"
        );
    }
}

#[test]
fn clang_rejects_an_overflowing_bit_field_value() {
    use SemanticErrorKind::NonConstantInitializer;
    for source in [
        "struct { unsigned a : 4; } x = { (unsigned)1e10 };",
        "struct { int b : 3; } x = { (int)-1e10 };",
    ] {
        assert_eq!(
            diagnostics(source, CompilerIdentity::Clang, ExtensionPolicy::Allow).0,
            [Kind::Other(NonConstantInitializer)],
            "{source}"
        );
        assert_eq!(
            diagnostics(source, CompilerIdentity::Gcc, ExtensionPolicy::Allow).0,
            [],
            "{source}"
        );
    }
    assert_eq!(
        diagnostics(
            "struct { unsigned a : 4; } x = { 1e10 };",
            CompilerIdentity::Clang,
            ExtensionPolicy::Allow
        )
        .0,
        [Kind::Literal, Kind::Other(NonConstantInitializer)]
    );
}

#[test]
fn an_out_of_range_conversion_is_no_integer_constant_expression() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        for source in [
            "enum { E = (int)1e10 };",
            "enum { E = (int)1e10 / 2 };",
            "_Static_assert((int)1e10 == 2147483647, \"\");",
            "struct { int b : (int)1e10; } x;",
        ] {
            assert!(
                diagnostics(source, identity, ExtensionPolicy::Allow)
                    .0
                    .contains(&Kind::Other(SemanticErrorKind::InvalidConstant)),
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn conversion_groups_follow_their_options() {
    let count = |identity, group, option| {
        let mut count = 0;
        with_configuration(
            "int x = 1e10;",
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_compiler_identity(identity)
                .with_warning_option(group, option),
            |context, unit| {
                count = context.take_pending_errors().len() + 10 * unit.errors;
            },
        );
        count
    };
    for (identity, group) in [
        (CompilerIdentity::Gcc, DiagnosticGroup::Overflow),
        (CompilerIdentity::Clang, DiagnosticGroup::LiteralConversion),
    ] {
        assert_eq!(count(identity, group, WarningOption::Enable(false)), 0);
        assert_eq!(count(identity, group, WarningOption::Error(true)), 11);
    }
}
