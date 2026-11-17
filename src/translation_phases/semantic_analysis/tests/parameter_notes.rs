//! Clang's `note: passing argument to parameter here`; the golden
//! `sema-parameter-note-clang` pins its words and places.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    CompilerIdentity,
    DiagnosticGroup,
};

/// Whether each report of `source` notes a parameter, in order.
fn noted(source: &str, identity: CompilerIdentity) -> Vec<(SemanticErrorKind, bool)> {
    let mut noted = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::default().with_compiler_identity(identity),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    noted.push((error.kind, error.parameter.is_some()));
                }
            }
        },
    );
    noted
}

#[test]
fn clang_notes_the_parameter_of_a_conversion_it_diagnoses() {
    let source = "void take(int *p); void (*through)(int *); void real(float f);
        void f(const int *c, long l) { take(c); take(l); through(c); real(16777217); }";
    let pointers = |group| SemanticErrorKind::PointerConstraint(group);
    assert_eq!(
        noted(source, CompilerIdentity::Clang),
        [
            (pointers(DiagnosticGroup::PointerQualifiers), true),
            (pointers(DiagnosticGroup::IntConversion), true),
            (pointers(DiagnosticGroup::PointerQualifiers), false),
            (
                SemanticErrorKind::ValueWarning(DiagnosticGroup::ImplicitConstIntFloatConversion),
                false
            ),
        ]
    );
    assert!(
        noted(source, CompilerIdentity::Gcc)
            .iter()
            .all(|&(_, noted)| !noted)
    );
}
