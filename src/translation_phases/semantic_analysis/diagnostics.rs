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
    errors::ReportOrder,
};
use crate::{
    translation_phases::{
        SourceVectors,
        TranslationError,
    },
    util::string_cache::StringCacheId,
};

impl Analyzer<'_, '_, '_> {
    /// Runs `report` with `operands` as the ranges Clang underlines with `~`
    /// beside the caret of what it reports: an assignment's or initializer's
    /// value, a binary operator's operands (Clang's
    /// `TextDiagnostic::emitCaret` draws a diagnostic's ranges).
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn with_highlights<R>(
        &mut self,
        operands: [SourceVectors; 2],
        report: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = std::mem::replace(&mut self.highlights, operands);
        let result = report(self);
        self.highlights = outer;
        result
    }

    /// Runs `check` with `context` as Clang's context location for the
    /// implicit conversions it checks: the call whose arguments convert, the
    /// assignment operator, the `return`, the declarator, or the
    /// brace-enclosed list an element belongs to.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn with_conversion_context<R>(
        &mut self,
        context: SourceVectors,
        check: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = std::mem::replace(&mut self.conversion_context, context);
        let result = check(self);
        self.conversion_context = outer;
        result
    }

    /// Whether Clang drops a conversion warning because its context location
    /// is a system macro's body token: `CheckImplicitConversion` returns
    /// early on `SourceManager::isInSystemMacro(CC)` for nearly every
    /// conversion it reports, so `_mm256_dp_ps(a, b, 0xff)` is silent where
    /// the immediate converts to the builtin's `char` parameter, wherever
    /// the argument was written. Only Clang's identities (Clang and
    /// clang-cl) ask; GCC asks of the warning's own location
    /// ([`Self::spelled_group_diagnostic`]).
    /// C99: §5.1.1.3p1 and footnote 8, p. 11; PDF p. 23.
    pub(super) fn conversion_in_system_macro(&self) -> bool {
        self.context.configuration.compiler_identity().rules()
            == crate::configuration::RuleSet::Clang
            && self.context.in_system_macro(self.conversion_context)
    }

    /// Runs `report` with what it reports placed `order` among the other
    /// diagnostics, for a warning GCC reports after the place it names.
    pub(super) fn with_report_order<R>(
        &mut self,
        order: ReportOrder,
        report: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = std::mem::replace(&mut self.report_order, order);
        let result = report(self);
        self.report_order = outer;
        result
    }

    /// The highlighted operands, for Clang's format only: the others draw
    /// none. They keep the places macros wrote them, which the renderer
    /// draws at the invocation and again in Clang's `expanded from macro`
    /// note.
    fn clang_highlights(&self) -> [SourceVectors; 2] {
        if self.context.configuration.diagnostic_format()
            == crate::configuration::DiagnosticFormat::Clang
        {
            self.highlights
        } else {
            [SourceVectors::default(); 2]
        }
    }

    /// Central emitter for pointer constraints; classification has no severity
    /// decisions. Accepted warnings do not taint initializer/ICE recovery.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn pointer_diagnostic(
        &mut self,
        group: crate::configuration::DiagnosticGroup,
        source_vectors: SourceVectors,
    ) {
        self.group_diagnostic(
            group,
            SemanticErrorKind::PointerConstraint(group),
            source_vectors,
            None,
            None,
        );
    }

    /// Reports `kind` under its warning group's configuration, as Clang
    /// does for a diagnostic that belongs to a group. Every group is a Clang
    /// warning, some defaulting to an error, so a system header withholds it
    /// at any severity. It is reported at its expansion location, as Clang
    /// reports it, with the earlier source `previous` it refers to.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) fn group_diagnostic(
        &mut self,
        group: crate::configuration::DiagnosticGroup,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        previous: Option<SourceVectors>,
    ) {
        self.group_diagnostic_read_at(group, kind, source_vectors, name, previous, source_vectors);
    }

    /// [`Self::group_diagnostic`] under the settings at `settings_at`, which
    /// is where a compiler that reports `kind` later, at another place, asks
    /// them.
    pub(super) fn group_diagnostic_read_at(
        &mut self,
        group: crate::configuration::DiagnosticGroup,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        previous: Option<SourceVectors>,
        settings_at: SourceVectors,
    ) {
        if self.tainted {
            return;
        }
        let Some(severity) =
            self.context
                .group_severity_read_at(group, source_vectors, settings_at)
        else {
            return;
        };
        let origin = source_vectors;
        let source_vectors = self.context.file_location(source_vectors);
        let previous = previous.map(|previous| self.context.file_location(previous));
        let highlights = self.clang_highlights();
        let errors = usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
        self.semantic_errors += errors;
        self.error_diagnostics += errors;
        // GCC counts an error by default, not a warning `-Werror` made one.
        if group.default_severity() == crate::translation_phases::ErrorSeverity::Error {
            self.unused.gcc_errors += errors;
        }
        self.context.classified_group_error(
            group,
            settings_at,
            TranslationError::Semantic(SemanticError {
                extension_severity: Some(severity),
                kind,
                source_vectors,
                name,
                previous,
                highlights,
                origin,
                order: self.report_order,
            }),
        );
    }

    /// [`Self::group_diagnostic`] for a warning GCC also withholds when a
    /// system header spells its token, as a system macro's body does; most of
    /// GCC's type-conversion warnings move to the invocation instead, and
    /// Clang goes by the invocation alone. The settings are those at
    /// `settings_at`, as for [`Self::group_diagnostic_read_at`].
    pub(super) fn spelled_group_diagnostic(
        &mut self,
        group: crate::configuration::DiagnosticGroup,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        settings_at: SourceVectors,
    ) {
        if self.context.configuration.compiler_identity()
            != crate::configuration::CompilerIdentity::Gcc
            || !self.context.spelled_in_system_header(source_vectors)
        {
            self.group_diagnostic_read_at(group, kind, source_vectors, name, None, settings_at);
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn error(
        &mut self,
        kind: SemanticErrorKind,
        source_vectors: SourceVectors,
        name: Option<StringCacheId>,
        previous: Option<SourceVectors>,
    ) {
        if let Some(group) = kind.attribute_group() {
            self.group_diagnostic(group, kind, source_vectors, name, previous);
            return;
        }
        if !self.tainted {
            self.semantic_errors += 1;
            let at = source_vectors;
            let source_vectors = self.context.file_location(source_vectors);
            let highlights = self.clang_highlights();
            let error = SemanticError {
                extension_severity: None,
                kind,
                source_vectors,
                name,
                previous,
                highlights,
                origin: at,
                order: self.report_order,
            };
            let severity = crate::translation_phases::GetSeverity::severity(&error);
            let hard = usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
            self.error_diagnostics += hard;
            self.hard_errors += hard;
            if !self.context.withholds(severity, false, source_vectors) {
                self.context
                    .classified_error(at, TranslationError::Semantic(error));
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
            let gcc_errors = &mut self.unused.gcc_errors;
            let order = self.report_order;
            self.context.report_extension_diagnostic(
                feature,
                baseline,
                source_vectors,
                |severity| {
                    let errors =
                        usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
                    *semantic_errors += errors;
                    *error_diagnostics += errors;
                    // `-pedantic-errors` makes GCC's pedantic warnings errors.
                    *gcc_errors += errors;
                    TranslationError::Semantic(SemanticError {
                        extension_severity: Some(severity),
                        kind,
                        source_vectors,
                        name,
                        previous: None,
                        highlights: [SourceVectors::default(); 2],
                        origin: SourceVectors::default(),
                        order,
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
