//! Compound-statement frame.

use std::fmt::Debug;

use super::{
    Parser,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    errors::ParserErrorType,
    expression_operators::is_operator,
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
    },
    scope::{
        NameClass,
        ScopeKind,
    },
    statement::StatementFrame,
    syntax::{
        BlockItem,
        Statement,
        StatementIndex,
        StatementType,
    },
};
use crate::translation_phases::{
    Context,
    SourceVectors,
    preprocessing::{
        OperatorTokenType,
        Token,
        TokenType,
    },
};

#[derive(Debug)]
pub(super) struct CompoundStatementFrame {
    phase:                     CompoundStatementPhase,
    pub(super) items:          Vec<BlockItem>,
    pub(super) source_vectors: Vec<SourceVectors>,
    starting_error_count:      usize,
    entry_scope_depth:         Option<usize>,
    function_body:             bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CompoundStatementPhase {
    Start,
    ItemOrClose,
    AwaitDeclaration,
    AwaitStatement,
    Finish,
}

#[expect(
    clippy::missing_assert_message,
    reason = "Frame phases assert the typed driver protocol, whose mismatch already identifies \
              the invariant."
)]
impl CompoundStatementFrame {
    pub(super) fn new(starting_error_count: usize, function_body: bool) -> Self {
        Self {
            phase: CompoundStatementPhase::Start,
            items: Vec::new(),
            source_vectors: Vec::new(),
            starting_error_count,
            entry_scope_depth: None,
            function_body,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | CompoundStatementPhase::Start => {
                debug_assert!(returned.is_none());
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::Block);
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    let token = token.expect("opening brace exists");
                    self.source_vectors.push(token.source_vectors);
                    if self.function_body {
                        let name = *parser
                            .func_name
                            .get_or_insert_with(|| context.string_cache.intern("__func__"));
                        parser.scopes.publish(name, NameClass::Ordinary);
                    }
                    self.phase = CompoundStatementPhase::ItemOrClose;
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedOpeningCurlyBraceInCompoundStatement(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = CompoundStatementPhase::ItemOrClose;
                    ParseAction::Reprocess
                }
            },
            | CompoundStatementPhase::ItemOrClose => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    let token = token.expect("closing brace exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = CompoundStatementPhase::Finish;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None),
                        None,
                    );
                    self.phase = CompoundStatementPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    let is_label = token.is_some_and(|token| token.kind == TokenType::Identifier)
                        && is_operator(parser.cursor.following(context), OperatorTokenType::Colon);
                    if !is_label && token.is_some_and(|token| parser.declaration_starter(token)) {
                        self.phase = CompoundStatementPhase::AwaitDeclaration;
                        ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                            DeclarationContext::Block,
                            parser.hard_error_count,
                        )))
                    } else {
                        self.phase = CompoundStatementPhase::AwaitStatement;
                        ParseAction::Push(ParseFrame::Statement(StatementFrame::new(
                            parser.hard_error_count,
                            None,
                        )))
                    }
                }
            },
            | CompoundStatementPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("block declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax[declaration].source_vectors;
                self.source_vectors.push(source);
                self.items.push(BlockItem::Declaration(declaration));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Continue
            },
            | CompoundStatementPhase::AwaitStatement => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("block statement returned an unexpected value: {returned:?}");
                };
                let source = parser.statement_source(statement);
                self.source_vectors.push(source);
                self.items.push(BlockItem::Statement(statement));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Continue
            },
            | CompoundStatementPhase::Finish => {
                debug_assert!(returned.is_none());
                let item_start = parser.append_syntax(&mut self.items);
                let index = parser.push_syntax(Statement {
                    kind:           StatementType::Compound { items: item_start },
                    source_vectors: context.merge_vector_list(&self.source_vectors),
                    recovered:      parser.hard_error_count > self.starting_error_count,
                });
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("compound statement entered block scope"),
                );
                ParseAction::Reduce(ParseValue::CompoundStatement(StatementIndex(index)))
            },
        }
    }
}
