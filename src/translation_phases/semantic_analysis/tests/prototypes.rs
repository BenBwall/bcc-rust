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

/// A declaration without a prototype takes the prototype of the one it
/// redeclares once Clang has compared them, and Clang compares the next
/// declaration with that: the second comparison reports `f()` as one with
/// a parameter, which C23 does not support. The golden leaves this out,
/// since bcc quotes no source for the second report at the place it just
/// reported, where Clang quotes it again after the note between them.
#[test]
fn a_redeclaration_takes_the_prototype_it_redeclares() {
    const SOURCE: &str = "int f(long); int f(); int f(long);";
    let mut words = Vec::new();
    with_exact_configuration(SOURCE, clang(), |context, _| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error
                && error.kind == SemanticErrorKind::DeprecatedNonPrototype
            {
                let offset = context
                    .get_source_vectors(error.source_vectors)
                    .first()
                    .map_or(usize::MAX, |vector| vector.index as usize);
                let name = error.name.map_or("", |name| context.string_cache.at(name));
                words.push((
                    offset,
                    name.contains("zero-parameter prototype"),
                    name.contains("not supported in C23"),
                ));
            }
        }
    });
    let at = SOURCE.find("f()").unwrap();
    assert_eq!(words, [(at, true, false), (at, false, true)], "{words:?}");
}
