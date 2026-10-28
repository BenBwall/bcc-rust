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
                    // GCC's pedantic report counts with the extensions.
                    | TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::PedanticOverflow,
                        ..
                    })
                    | TranslationError::Extension(_) => result.1 += 1,
                    | TranslationError::Semantic(e) => result.0.push(Kind::Other(e.kind)),
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
        // Clang checks builtin integer types only.
        ("enum E { A } x = 1.5;", &[], &[], 0),
        ("_Atomic int x = 2.5;", &[], &[], 0),
        // A comparison's result is a fresh constant to GCC.
        ("int x = (int)1e10 == 2147483647;", &[], &[], 0),
        // Nor is the operand `?:` does not select, its condition, or a
        // conversion to `_Bool`; the selected operand is.
        ("int x = (int)1e10 ? 1 : 2;", &[], &[], 0),
        ("int x = 1 ? 3 : (int)1e10;", &[], &[], 0),
        ("int x = (_Bool)(int)1e10;", &[], &[], 0),
        ("int x = 1 ? (int)1e10 : 3;", &[], &[], 1),
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
            let found = diagnostics(source, identity, ExtensionPolicy::Allow).0;
            assert!(
                found.contains(&Kind::Other(SemanticErrorKind::InvalidConstant))
                    || found.contains(&Kind::Other(SemanticErrorKind::ConstantOverflow)),
                "{identity:?}: {source}: {found:?}"
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
        // `-Wconversion` holds Clang's group: `-Werror=conversion` reaches it.
        (CompilerIdentity::Clang, DiagnosticGroup::Conversion),
    ] {
        assert_eq!(count(identity, group, WarningOption::Enable(false)), 0);
        assert_eq!(count(identity, group, WarningOption::Error(true)), 11);
    }
}

#[test]
fn overflow_options_control_gccs_pedantic_report() {
    // The kinds and error count of `int x = 1e10;` under GCC's rules.
    let run = |policy, options: &[(DiagnosticGroup, WarningOption)]| {
        let mut result = (Vec::new(), 0);
        with_configuration(
            "int x = 1e10;",
            options.iter().fold(
                CompilerConfiguration::new(CStandard::C17, policy)
                    .with_compiler_identity(CompilerIdentity::Gcc),
                |configuration, &(group, option)| configuration.with_warning_option(group, option),
            ),
            |context, unit| {
                for error in context.take_pending_errors() {
                    if let TranslationError::Semantic(error) = error {
                        result.0.push(error.kind);
                    }
                }
                result.1 = unit.errors;
            },
        );
        result
    };
    let overflow = |error| (DiagnosticGroup::Overflow, WarningOption::Error(error));
    let disabled = (DiagnosticGroup::Overflow, WarningOption::Enable(false));
    // Only `-pedantic` reports it; `-Wno-overflow` withdraws it, and
    // `-Werror=overflow` makes it an error, as in GCC.
    assert_eq!(run(ExtensionPolicy::Allow, &[overflow(true)]).0.len(), 1);
    assert_eq!(run(ExtensionPolicy::Warn, &[]).0.len(), 2);
    assert_eq!(
        run(ExtensionPolicy::Warn, &[overflow(true)]),
        (run(ExtensionPolicy::Warn, &[]).0, 2)
    );
    assert_eq!(run(ExtensionPolicy::Deny, &[]).1, 1);
    assert_eq!(run(ExtensionPolicy::Deny, &[disabled]), (vec![], 0));
    assert_eq!(run(ExtensionPolicy::Deny, &[overflow(false)]).1, 0);
}

#[test]
fn gcc_finds_no_constant_in_an_overflowing_conversion() {
    for (identity, expected) in [
        (CompilerIdentity::Gcc, [0, 0, 0]),
        // Clang's evaluator also folds `(int)1e10 + 1`, which bcc does not.
        (CompilerIdentity::Clang, [1, 1, 0]),
    ] {
        let source = format!(
            "_Static_assert(__builtin_constant_p((int)1e10) == {}, \"\"); \
             _Static_assert(__builtin_constant_p((unsigned char)300.0) == {}, \"\"); \
             _Static_assert(__builtin_constant_p((int)1e10 + 1) == {}, \"\");",
            expected[0], expected[1], expected[2]
        );
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_compiler_identity(identity),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert_eq!(unit.errors, 0, "{identity:?}: {errors:?}");
            },
        );
    }
}

#[test]
fn gccs_conversion_options_leave_clangs_groups_alone() {
    // GCC's `-Wconversion` holds no `-Wpointer-bool-conversion`.
    for (identity, errors) in [(CompilerIdentity::Gcc, 0), (CompilerIdentity::Clang, 1)] {
        with_configuration(
            "int a[2]; int f(void) { return a ? 1 : 0; }",
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_compiler_identity(identity)
                .with_warning_option(DiagnosticGroup::Conversion, WarningOption::Error(true)),
            |context, unit| {
                drop(context.take_pending_errors());
                assert_eq!(unit.errors, errors, "{identity:?}");
            },
        );
    }
}
