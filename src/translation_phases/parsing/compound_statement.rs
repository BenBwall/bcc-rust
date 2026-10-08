//! Compound-statement frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `compound-statement`, `block-item-list`, and `block-item`.
//! C99: §6.8.2, p. 132; PDF p. 144; §A.2.3, p. 415; PDF p. 427.
//!
//! A compound statement is a block (§6.8.2 paragraph 2, p. 132; PDF p. 144),
//! so the frame opens a block scope whose identifiers end at its `}`
//! (§6.2.1 paragraph 4, p. 29; PDF p. 41). Blocks nest through the frame
//! stack, not recursion, so the 127-level minimum of §5.2.4.1, p. 20;
//! PDF p. 32 imposes no fixed ceiling. Declaration semantics and
//! constraints belong to semantic analysis.

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
        StatementType,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Resumable `compound-statement`: `{ block-item-list(opt) }`, where each
/// `block-item` is a `declaration` or a `statement`.
///
/// C99: §6.8.2, p. 132; PDF p. 144. A function body predeclares `__func__`
/// as if declared just after its `{` under §6.4.2.2 paragraph 1, p. 52;
/// PDF p. 64.
#[derive(Debug)]
pub(super) struct CompoundStatementFrame<'tu, 'p> {
    phase:                     CompoundStatementPhase,
    pub(super) items:          ArenaVec<'p, BlockItem<'tu>>,
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    starting_error_count:      usize,
    entry_scope_depth:         Option<usize>,
    function_body:             bool,
    has_statement:             bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CompoundStatementPhase {
    Start,
    ItemOrClose,
    AwaitDeclaration,
    AwaitFunctionDefinition,
    AwaitStatement,
    Finish,
}

#[expect(
    clippy::missing_assert_message,
    reason = "Frame phases assert the typed driver protocol, whose mismatch already identifies \
              the invariant."
)]
impl<'tu, 'p> CompoundStatementFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump, starting_error_count: usize, function_body: bool) -> Self {
        Self {
            phase: CompoundStatementPhase::Start,
            items: ArenaVec::new_in(arena),
            source_vectors: ArenaVec::new_in(arena),
            starting_error_count,
            entry_scope_depth: None,
            function_body,
            has_statement: false,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
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
                            .get_or_insert_with(|| parser.context.string_cache.intern("__func__"));
                        parser.scopes.publish(name, NameClass::Ordinary);
                    }
                    self.phase = CompoundStatementPhase::ItemOrClose;
                    ParseAction::Consume
                } else {
                    parser.report(
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
                        ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None),
                        None,
                    );
                    self.phase = CompoundStatementPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    // C99 §6.8.1p1: `identifier :` begins a labeled
                    // statement even when the identifier names a typedef;
                    // labels have their own name space (§6.2.3p1).
                    let is_label = token
                        .is_some_and(|token| matches!(token.kind, TokenType::Identifier))
                        && is_operator(parser.cursor.following(), OperatorTokenType::Colon);
                    if !is_label
                        && token.is_some_and(|token| parser.declaration_starter(token))
                        && parser.extension_precedes_declaration()
                        && (!parser.attribute_starter(token)
                            || parser.attributes_precede_declaration())
                    {
                        if self.has_statement {
                            parser.extension(
                                crate::configuration::Feature::MixedDeclarations,
                                "mixed declarations and code",
                                token.expect("declaration starts here"),
                            );
                        }
                        self.phase = CompoundStatementPhase::AwaitDeclaration;
                        ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                            parser.arena,
                            DeclarationContext::Block,
                            parser.hard_error_count,
                        )))
                    } else {
                        self.phase = CompoundStatementPhase::AwaitStatement;
                        ParseAction::Push(ParseFrame::Statement(
                            StatementFrame::new(parser.hard_error_count, None)
                                .with_block_item(true),
                        ))
                    }
                }
            },
            | CompoundStatementPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("block declaration returned an unexpected value: {returned:?}");
                };
                if declaration.is_definition_head() {
                    let mut extension = declaration.declaration_specifiers.extensions;
                    let mut suppressed = false;
                    while let Some(item) = extension {
                        suppressed |=
                            item.kind == super::modern::SpecifierExtensionKind::ExtensionMarker;
                        extension = item.next;
                    }
                    if !suppressed && let Some(token) = token {
                        parser.extension(
                            crate::configuration::Feature::NestedFunctions,
                            "nested function definition",
                            token,
                        );
                    }
                    self.phase = CompoundStatementPhase::AwaitFunctionDefinition;
                    return ParseAction::Push(ParseFrame::FunctionDefinition(
                        super::function_definition::FunctionDefinitionFrame::new(
                            parser.arena,
                            declaration,
                            parser.hard_error_count,
                        ),
                    ));
                }
                let source = declaration.source_vectors;
                self.source_vectors.push(source);
                self.items.push(BlockItem::Declaration(declaration));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Continue
            },
            | CompoundStatementPhase::AwaitFunctionDefinition => {
                let Some(ParseValue::FunctionDefinition(definition)) = returned else {
                    panic!("nested function child protocol");
                };
                self.source_vectors.push(definition.source_vectors);
                self.items.push(BlockItem::FunctionDefinition(definition));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Continue
            },
            | CompoundStatementPhase::AwaitStatement => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("block statement returned an unexpected value: {returned:?}");
                };
                let source = statement.source_vectors;
                self.source_vectors.push(source);
                self.items.push(BlockItem::Statement(statement));
                // A GNU `__label__` declaration opens the block before its
                // declarations; it is not code that they follow.
                self.has_statement |= !matches!(statement.kind, StatementType::LocalLabels(_));
                self.phase = CompoundStatementPhase::ItemOrClose;
                ParseAction::Continue
            },
            | CompoundStatementPhase::Finish => {
                debug_assert!(returned.is_none());
                let item_start = parser.alloc_syntax_list(&mut self.items);
                let source_vectors = parser.context.merge_vector_list(&self.source_vectors);
                let index = parser.alloc_syntax(Statement {
                    kind: StatementType::Compound { items: item_start },
                    source_vectors,
                    recovered: parser.hard_error_count > self.starting_error_count,
                });
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("compound statement entered block scope"),
                );
                ParseAction::Reduce(ParseValue::CompoundStatement(index))
            },
        }
    }
}
