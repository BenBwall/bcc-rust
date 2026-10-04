//! Translation-unit level frame dispatching external declarations.

use std::fmt::Debug;

use super::{
    Parser,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    errors::{
        RecoverySummary,
        RelatedParserDiagnostic,
    },
    expression_operators::is_operator,
    function_definition::FunctionDefinitionFrame,
    machine::{
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    syntax::ExternalDeclaration,
};
use crate::translation_phases::{
    Context,
    ErrorSeverity,
    TranslationError,
    preprocessing::{
        OperatorTokenType,
        Token,
    },
};

/// Root frame that converts one declaration child into a valid or explicitly
/// recovered external-declaration item.
///
/// C99: external-declaration is specified by §6.9, p. 140; PDF p. 152.
#[derive(Debug, Clone, Copy)]
pub(super) struct ExternalDeclarationFrame {
    /// Current root-frame transition.
    phase:                     ExternalDeclarationPhase,
    /// Hard-error count at entry, used only to classify the yielded AST.
    starting_error_count:      usize,
    /// Pending-diagnostic boundary used to attach root-level recovery context
    /// to the primary diagnostic for this external declaration.
    starting_diagnostic_count: usize,
}

/// Transitions for one external declaration.
///
/// C99: §6.9, p. 140; PDF p. 152. The phase split is an implementation detail.
#[derive(Debug, Clone, Copy)]
pub(super) enum ExternalDeclarationPhase {
    /// Push the declaration child without consuming its first token.
    Start,
    /// Classify the completed declaration from diagnostics emitted since entry.
    AwaitDeclaration,
    /// Receive a function definition selected from the completed declaration
    /// head.
    AwaitFunctionDefinition,
}

impl<'tu, 'p> ExternalDeclarationFrame {
    pub(super) fn new(starting_error_count: usize, starting_diagnostic_count: usize) -> Self {
        Self {
            phase: ExternalDeclarationPhase::Start,
            starting_error_count,
            starting_diagnostic_count,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'tu, 'p>,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | ExternalDeclarationPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = ExternalDeclarationPhase::AwaitDeclaration;
                ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                    parser.arena,
                    DeclarationContext::External,
                    parser.hard_error_count,
                )))
            },
            | ExternalDeclarationPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("declaration frame returned an unexpected value: {returned:?}");
                };
                let is_definition = parser.declaration_is_definition_head(declaration)
                    && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || token.is_some_and(|token| parser.declaration_starter(token)));
                if is_definition {
                    self.phase = ExternalDeclarationPhase::AwaitFunctionDefinition;
                    return ParseAction::Push(ParseFrame::FunctionDefinition(
                        FunctionDefinitionFrame::new(
                            parser.arena,
                            declaration,
                            self.starting_error_count,
                        ),
                    ));
                }
                if parser.hard_error_count > self.starting_error_count {
                    if !parser.declaration_is_meaningful(declaration) {
                        let declaration_source = parser.syntax[declaration].source_vectors;
                        let source = if declaration_source.length == 0 {
                            token.map_or(declaration_source, |token| token.source_vectors)
                        } else {
                            declaration_source
                        };
                        let related = token.map(|token| {
                            context.diagnostic_slice(&[RelatedParserDiagnostic {
                                message:        "parsing resumes here",
                                source_vectors: token.source_vectors,
                            }])
                        });
                        if let Some(TranslationError::Parsing(error)) = context
                            .pending_errors
                            .iter_mut()
                            .skip(self.starting_diagnostic_count)
                            .find(|error| matches!(error, TranslationError::Parsing(error) if error.severity == ErrorSeverity::Error && error.recovery.is_none()))
                        {
                            if let Some(related) = related {
                                error.related = related;
                            }
                            error.recovery = Some(RecoverySummary {
                                owner: ParseFrameKind::ExternalDeclaration,
                                discarded: None,
                                discarded_tokens: 0,
                                stopped_at: token.map(|token| token.kind),
                            });
                        }
                        return ParseAction::Reduce(ParseValue::ExternalDeclaration(
                            ExternalDeclaration::Error(source),
                        ));
                    }
                    return ParseAction::Reduce(ParseValue::ExternalDeclaration(
                        ExternalDeclaration::RecoveredDeclaration(declaration),
                    ));
                }
                ParseAction::Reduce(ParseValue::ExternalDeclaration(
                    ExternalDeclaration::Declaration(declaration),
                ))
            },
            | ExternalDeclarationPhase::AwaitFunctionDefinition => {
                let Some(ParseValue::FunctionDefinition(definition)) = returned else {
                    panic!("function-definition frame returned an unexpected value: {returned:?}");
                };
                let recovered = parser.syntax[definition].recovered;
                ParseAction::Reduce(ParseValue::ExternalDeclaration(if recovered {
                    ExternalDeclaration::RecoveredFunctionDefinition(definition)
                } else {
                    ExternalDeclaration::FunctionDefinition(definition)
                }))
            },
        }
    }
}
