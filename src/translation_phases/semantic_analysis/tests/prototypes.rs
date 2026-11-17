//! Clang 23's `-Wdeprecated-non-prototype` and its argument counts for
//! calls to a definition without a prototype; the golden
//! `sema-deprecated-non-prototype-clang` pins their words, places and order.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup,
    ExtensionPolicy,
    WarningOption,
};

/// The reports of `source`: each one's kind and where it is, as the byte
/// offset of its first token.
fn reports(source: &str, configuration: CompilerConfiguration) -> Vec<(SemanticErrorKind, usize)> {
    let mut reports = Vec::new();
    with_exact_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                let offset = context
                    .get_source_vectors(error.source_vectors)
                    .first()
                    .map_or(usize::MAX, |vector| vector.index as usize);
                reports.push((error.kind, offset));
            }
        }
    });
    reports
}

fn clang() -> CompilerConfiguration {
    CompilerConfiguration::default().with_compiler_identity(CompilerIdentity::Clang)
}

const OLD_STYLE: &str = "int f(); int f(a) int a; { return a; } int g(void) { return f(1, 2); }";

#[test]
fn clang_reports_identifier_lists_and_their_calls() {
    let reports = reports(OLD_STYLE, clang());
    assert_eq!(
        reports,
        [
            (
                SemanticErrorKind::DeprecatedNonPrototype,
                OLD_STYLE.find("f(a)").unwrap()
            ),
            (
                SemanticErrorKind::DeprecatedNonPrototype,
                OLD_STYLE.find("f()").unwrap()
            ),
            (
                SemanticErrorKind::CallArgumentCount,
                OLD_STYLE.find("2)").unwrap() + 1
            ),
        ],
        "{reports:?}"
    );
}

#[test]
fn other_compilers_revisions_and_options_report_nothing() {
    for configuration in [
        CompilerConfiguration::default().with_compiler_identity(CompilerIdentity::Gcc),
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(CompilerIdentity::Clang),
        clang().with_warning_option(
            DiagnosticGroup::DeprecatedNonPrototype,
            WarningOption::Enable(false),
        ),
    ] {
        let reports = reports(OLD_STYLE, configuration);
        assert!(
            reports
                .iter()
                .all(|&(kind, _)| kind != SemanticErrorKind::DeprecatedNonPrototype),
            "{reports:?}"
        );
    }
}

/// Clang gives a compatible redeclaration of a library builtin the
/// builtin's prototype, and takes an implicit declaration as declared.
#[test]
fn builtins_and_implicit_declarations_have_prototypes() {
    for source in [
        "long labs(); long f(void) { return labs(1L); }",
        "int f(void) { return g(1); }",
    ] {
        let reports = reports(
            source,
            clang().with_warning_option(
                DiagnosticGroup::ImplicitFunctionDeclaration,
                WarningOption::Enable(false),
            ),
        );
        assert!(reports.is_empty(), "{source}: {reports:?}");
    }
}
