//! Translation-unit level frame dispatching external declarations.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of one
//! `external-declaration` of a `translation-unit` (C99: §6.9 paragraph 1,
//! p. 140; PDF p. 152; §A.2.4, p. 416; PDF p. 428). Both alternatives begin
//! with `declaration-specifiers` and usually a declarator, so the frame
//! parses a declaration and turns it into a `function-definition` (§6.9.1
//! paragraph 1, p. 141; PDF p. 153) when a body or declaration list follows
//! its sole declarator.
//!
//! The constraints of §6.9 paragraphs 2-3 (no `auto` or `register`; at most
//! one external definition of an internal-linkage identifier), p. 140;
//! PDF p. 152, and the external-definition semantics of paragraph 5,
//! p. 140; PDF p. 152, and §6.9.2, pp. 143-144; PDF pp. 155-156, are left to
//! semantic analysis.

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
        unexpected_return,
    },
    syntax::ExternalDeclaration,
};
use crate::translation_phases::{
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
/// Its file-scope position is §6.9 paragraph 4, p. 140; PDF p. 152.
#[derive(Debug, Clone, Copy)]
pub(super) struct ExternalDeclarationFrame {
    /// Current root-frame transition.
    phase:                     ExternalDeclarationPhase,
    /// Hard-error count at entry, used to classify the yielded AST and handed
    /// to a function-definition child.
    starting_error_count:      usize,
    /// Pending-diagnostic boundary used to attach root-level recovery context
    /// to the primary diagnostic for this external declaration.
    starting_diagnostic_count: usize,
}

/// Transitions for one external declaration.
///
/// C99: §6.9, p. 140; PDF p. 152. The phase split is an implementation detail.
#[derive(Debug, Clone, Copy)]
enum ExternalDeclarationPhase {
    /// Push the declaration child without consuming its first token.
    Start,
    AwaitAsm,
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
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | ExternalDeclarationPhase::AwaitAsm => {
                let Some(ParseValue::Gnu(super::gnu::GnuValue::Asm(asm))) = returned else {
                    panic!("file asm child protocol");
                };
                ParseAction::Reduce(ParseValue::ExternalDeclaration(ExternalDeclaration::Asm(
                    asm,
                )))
            },
            | ExternalDeclarationPhase::Start => {
                if token.is_some_and(|x| {
                    matches!(
                        x.kind,
                        crate::translation_phases::preprocessing::TokenType::Keyword(
                            crate::translation_phases::preprocessing::KeywordTokenType::Asm,
                        )
                    ) || matches!(
                        x.kind,
                        crate::translation_phases::preprocessing::TokenType::Keyword(
                            crate::translation_phases::preprocessing::KeywordTokenType::MsAsm,
                        )
                    ) && x.contents
                        == crate::translation_phases::preprocessing::KeywordTokenType::MsAsm
                            .cache_id()
                }) {
                    self.phase = ExternalDeclarationPhase::AwaitAsm;
                    return super::gnu::GnuFrame::push(
                        parser,
                        super::gnu::GnuKind::Asm { label: false },
                    );
                }
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
                    unexpected_return!("declaration frame returned an unexpected value", returned);
                };
                // C99 §6.9.1p1: `declaration-specifiers declarator
                // declaration-list? compound-statement`.
                let is_definition = declaration.is_definition_head()
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
                    if !declaration.is_meaningful() {
                        let declaration_source = declaration.source_vectors;
                        let source = if declaration_source.length() == 0 {
                            token.map_or(declaration_source, |token| token.source_vectors)
                        } else {
                            declaration_source
                        };
                        let related = token.map(|token| {
                            parser.context.diagnostic_slice(&[RelatedParserDiagnostic {
                                message:        "parsing resumes here",
                                source_vectors: token.source_vectors,
                            }])
                        });
                        if let Some(TranslationError::Parsing(error)) = parser.context
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
                    unexpected_return!(
                        "function-definition frame returned an unexpected value",
                        returned,
                    );
                };
                let recovered = definition.recovered;
                ParseAction::Reduce(ParseValue::ExternalDeclaration(if recovered {
                    ExternalDeclaration::RecoveredFunctionDefinition(definition)
                } else {
                    ExternalDeclaration::FunctionDefinition(definition)
                }))
            },
        }
    }
}
