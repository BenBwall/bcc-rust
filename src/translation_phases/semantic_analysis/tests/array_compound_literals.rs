//! An array initialized by a compound literal of its array type (GNU
//! extension `ArrayCompoundLiteral`), and a string literal too long for the
//! array it initializes, under each identity's rules.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

/// `(semantic errors, extension diagnostics)` of `source` in GNU C17 under
/// `identity` and `policy`.
fn diagnostics(
    source: &str,
    identity: CompilerIdentity,
    policy: ExtensionPolicy,
) -> (Vec<SemanticErrorKind>, usize) {
    let mut result = (Vec::new(), 0);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C17, policy)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity),
        |context, _| {
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(e) => result.0.push(e.kind),
                    | TranslationError::Extension(_) => result.1 += 1,
                    | other => panic!("{source}: unexpected {other:?}"),
                }
            }
        },
    );
    result
}

#[test]
fn static_arrays_take_a_compatible_literal_under_both_identities() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        for source in [
            "static char a[3] = (char[3]){\"ab\"};",
            "char a[] = (char[3]){\"ab\"};",
            "char a[3] = (char[]){\"ab\"};",
            "char a[3] = ((char[3]){\"ab\"});",
            "int a[2][2] = (int[2][2]){{1, 2}, {3, 4}};",
            "typedef __WCHAR_TYPE__ wchar_t; wchar_t a[3] = (wchar_t[3]){L\"ab\"};",
            "void f(void) { static int a[2] = (int[2]){1, 2}; (void)a; }",
        ] {
            assert_eq!(
                diagnostics(source, identity, ExtensionPolicy::Allow),
                (vec![], 0),
                "{identity:?}: {source}"
            );
            // GCC reports a constant compound literal, Clang the array one.
            assert_eq!(
                diagnostics(source, identity, ExtensionPolicy::Warn),
                (vec![], 1),
                "{identity:?}: {source}"
            );
        }
        for source in [
            "char a[4] = (char[3]){\"ab\"};",
            "char a[2] = (char[3]){\"ab\"};",
            "char a[3] = (unsigned char[3]){\"ab\"};",
            "char a[3] = 1 ? (char[3]){\"ab\"} : (char[3]){\"cd\"};",
        ] {
            assert_eq!(
                diagnostics(source, identity, ExtensionPolicy::Allow).0,
                [SemanticErrorKind::InvalidInitializer],
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn automatic_arrays_and_qualifiers_differ_between_identities() {
    use SemanticErrorKind::{
        InvalidInitializer as Invalid,
        NonConstantArrayInitializer as NonConstant,
        PointerConstraint,
    };
    let int_conversion = PointerConstraint(crate::configuration::DiagnosticGroup::IntConversion);
    // The source, GCC's errors and Clang's errors.
    for (source, gcc, clang) in [
        (
            "void f(void) { char a[3] = (char[3]){\"ab\"}; (void)a; }",
            &[NonConstant][..],
            &[][..],
        ),
        (
            "void f(int i) { int a[2] = (int[2]){i, 2}; (void)a; }",
            &[NonConstant],
            &[],
        ),
        (
            "void f(int i) { int a[2] = (int[2]){i++, 2}; (void)a; }",
            &[Invalid],
            &[NonConstant],
        ),
        (
            "int g(void); void f(void) { int a[2] = (int[2]){g(), 2}; (void)a; }",
            &[Invalid],
            &[NonConstant],
        ),
        ("char a[3] = (const char[3]){\"ab\"};", &[], &[Invalid]),
        ("const char a[3] = (char[3]){\"ab\"};", &[], &[Invalid]),
        ("_Atomic int a[2] = (int[2]){1, 2};", &[], &[Invalid]),
        // Inside braces GCC initializes an element or member whole; Clang
        // converts the decayed literal to the first character.
        (
            "struct S { char s[3]; int i; } s = { (char[3]){\"ab\"}, 1 };",
            &[],
            &[int_conversion],
        ),
        (
            "char a[2][3] = { (char[3]){\"ab\"}, (char[3]){\"cd\"} };",
            &[],
            &[int_conversion, int_conversion],
        ),
        (
            "char a[3] = { (char[3]){\"ab\"} };",
            &[int_conversion],
            &[int_conversion],
        ),
        // Only the object's own unknown bound completes from a literal; a
        // flexible array member takes none.
        (
            "struct F { int n; int a[]; }; static struct F f = { 1, (int[2]){7, 8} };",
            &[int_conversion],
            &[int_conversion],
        ),
        (
            "struct G { int n; char s[]; }; static struct G g = { 1, (char[3]){\"ab\"} };",
            &[int_conversion],
            &[int_conversion],
        ),
    ] {
        for (identity, expected) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            assert_eq!(
                diagnostics(source, identity, ExtensionPolicy::Allow).0,
                expected,
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn too_long_strings_warn_and_pedantic_errors_reject_them() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        for source in [
            "char a[3] = \"abcd\";",
            "char a[3] = { \"abcd\" };",
            "char a[3] = (char[3]){\"abcd\"};",
            "void f(void) { char a[2] = \"abc\"; (void)a; }",
        ] {
            let mut errors = 0;
            with_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                    .with_gnu_extensions(true)
                    .with_compiler_identity(identity),
                |context, unit| {
                    let found = context.take_pending_errors();
                    assert!(
                        found.iter().any(|error| matches!(error, TranslationError::Semantic(e) if e.kind == SemanticErrorKind::ExcessString)),
                        "{identity:?}: {source}: {found:?}"
                    );
                    errors = unit.errors;
                },
            );
            assert_eq!(errors, 0, "{identity:?}: {source}");
            with_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Deny)
                    .with_gnu_extensions(true)
                    .with_compiler_identity(identity),
                |context, unit| {
                    drop(context.take_pending_errors());
                    assert!(unit.errors >= 1, "{identity:?}: {source}");
                },
            );
        }
        // A string that fits, null character aside, is no excess.
        assert_eq!(
            diagnostics("char a[3] = \"abc\";", identity, ExtensionPolicy::Deny),
            (vec![], 0)
        );
    }
}
