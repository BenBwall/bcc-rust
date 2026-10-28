//! Phase-7 constraint and extension reporting. Recovered syntax suppresses
//! dependent diagnostics; extension severity follows the context's policy.
//! Diagnostic wording and source labels are defined in errors.rs.
//! C99: §4 paragraph 6, p. 7; PDF p. 19;
//! §5.1.1.3 paragraph 1, p. 11; PDF p. 23;
//! §6.6 paragraph 4, p. 95; PDF p. 107.

use super::{
    Analyzer,
    ArenaVec,
    ConstantClass,
    SemanticError,
    SemanticErrorKind,
};
use crate::{
    translation_phases::{
        SourceVectors,
        TranslationError,
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
    },
    util::string_cache::StringCacheId,
};

impl Analyzer<'_, '_, '_> {
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
        if self.tainted {
            return;
        }
        let Some(severity) = self.context.group_severity(group, source_vectors) else {
            return;
        };
        let source_vectors = self.context.file_location(source_vectors);
        let previous = previous.map(|previous| self.context.file_location(previous));
        let errors = usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
        self.semantic_errors += errors;
        self.error_diagnostics += errors;
        self.context
            .append_pending_errors([TranslationError::Semantic(SemanticError {
                extension_severity: Some(severity),
                kind,
                source_vectors,
                name,
                previous,
            })]);
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
            let source_vectors = self.context.file_location(source_vectors);
            let error = SemanticError {
                extension_severity: None,
                kind,
                source_vectors,
                name,
                previous,
            };
            let severity = crate::translation_phases::GetSeverity::severity(&error);
            self.error_diagnostics +=
                usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
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
                    *semantic_errors +=
                        usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
                    *error_diagnostics +=
                        usize::from(severity == crate::translation_phases::ErrorSeverity::Error);
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

    /// Reports arithmetic on a bit-precise value wider than the constant
    /// evaluator's 128 bits where an integer constant expression is
    /// required ([`super::Integer::convert`]). An array bound reports it
    /// too: GCC and Clang take such a bound as a constant, not as a
    /// variable length array's.
    /// C23: §6.6 paragraph 6, p. 95; PDF p. 108.
    pub(super) fn wide_constant(&mut self, source: SourceVectors) {
        self.error(
            SemanticErrorKind::UnsupportedWideConstant,
            source,
            None,
            None,
        );
    }

    /// Whether `expression` failed to fold because an operand has a
    /// bit-precise type wider than the constant model: some arithmetic
    /// constant in it has no folded value though it has such a type or
    /// such an operand ([`super::Integer::convert`]). A context that
    /// requires a constant reports [`Self::wide_constant`] for it instead
    /// of its own error.
    /// C23: §6.6 paragraph 6, p. 95; PDF p. 108.
    pub(super) fn wide_unfolded(&self, expression: &Expression<'_>) -> bool {
        let info = |expression: &Expression<'_>| {
            self.expression_indices
                .get(&std::ptr::from_ref(expression).addr())
                .map(|&index| &self.expressions[index])
        };
        let wide = |expression: &Expression<'_>| {
            info(expression).is_some_and(|info| {
                self.integer_type(info.ty)
                    .is_some_and(|(bits, _)| bits > 128)
            })
        };
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push(expression);
        while let Some(expression) = pending.pop() {
            let mut operands = ArenaVec::new_in(self.scratch);
            match expression.kind {
                | ExpressionType::Parenthesized { expression }
                | ExpressionType::Unary {
                    operand_expression: expression,
                    ..
                }
                | ExpressionType::Cast {
                    operand_expression: expression,
                    ..
                } => operands.push(expression),
                | ExpressionType::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => operands.extend([left_expression, right_expression]),
                | ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c) =>
                    operands.extend([c.condition_expression, c.then_expression, c.else_expression]),
                | _ => {},
            }
            if info(expression).is_some_and(|info| {
                info.constant == ConstantClass::Arithmetic && info.integer.is_none()
            }) && (wide(expression) || operands.iter().any(|&operand| wide(operand)))
            {
                return true;
            }
            pending.extend(operands);
        }
        false
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
