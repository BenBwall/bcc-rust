//! Phase-7 constraint and extension reporting. Recovered syntax suppresses
//! dependent diagnostics; extension severity follows the context's policy.
//! Diagnostic wording and source labels are defined in errors.rs.
//! C99: §4 paragraph 6, p. 7; PDF p. 19;
//! §5.1.1.3 paragraph 1, p. 11; PDF p. 23;
//! §6.6 paragraph 4, p. 95; PDF p. 107.

use super::{
    Analyzer,
    SemanticError,
    SemanticErrorKind,
    SourceVectors,
    StringCacheId,
    TranslationError,
};

impl Analyzer<'_, '_, '_> {
    #[cold]
    #[inline(never)]
    pub(super) fn error(
        &mut self,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        previous: Option<SourceVectors>,
    ) {
        if !self.tainted {
            self.semantic_errors += 1;
            let error = SemanticError {
                extension_severity: None,
                kind,
                source_vectors,
                name,
                previous,
            };
            let severity = crate::translation_phases::GetSeverity::severity(&error);
            self.error_diagnostics += usize::from(severity == super::ErrorSeverity::Error);
            if !self.context.withholds(severity, false, source_vectors) {
                self.context
                    .append_pending_errors([TranslationError::Semantic(error)]);
            }
        }
    }

    /// Reports a typed semantic extension without duplicating policy decisions.
    /// C99: §4p6, p. 7; PDF p. 19; §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn extension_error(
        &mut self,
        feature: crate::configuration::Feature,
        baseline: super::DiagnosticPolicy,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
    ) {
        if !self.tainted {
            let semantic_errors = &mut self.semantic_errors;
            let error_diagnostics = &mut self.error_diagnostics;
            self.context.report_extension_diagnostic(
                feature,
                baseline,
                source_vectors,
                |severity| {
                    *semantic_errors += usize::from(severity == super::ErrorSeverity::Error);
                    *error_diagnostics += usize::from(severity == super::ErrorSeverity::Error);
                    TranslationError::Semantic(SemanticError {
                        extension_severity: Some(severity),
                        kind,
                        source_vectors,
                        name,
                        previous: None,
                    })
                },
            );
        }
    }

    /// Exceptional evaluation violates the representability constraint where
    /// an integer constant expression is required; an array bound instead
    /// becomes variable length.
    /// C99: §6.6 paragraph 4, p. 95; PDF p. 107.
    /// C99: §6.7.5.2 paragraph 4, pp. 116-117; PDF pp. 128-129.
    pub(super) fn exceptional_constant(&mut self, source: SourceVectors) {
        if !self.runtime_bound {
            self.error(SemanticErrorKind::ConstantOverflow, source, None, None);
        }
    }
}
