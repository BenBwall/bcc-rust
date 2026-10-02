//! Declaration frame: specifiers, init-declarator lists, and terminators.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_specifiers::{
        DeclarationSpecifiersFrame,
        SpecifierMode,
    },
    declaration_syntax::{
        Declaration,
        DeclarationSpecifiers,
        InitDeclarator,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
    },
    errors::ParserErrorType,
    expression_operators::is_operator,
    initializer::InitializerFrame,
    machine::{
        InitializerResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::NameClass,
    statement::is_statement_keyword,
    syntax::{
        DeclarationIndex,
        StorageClass,
        SyntaxList,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::vector_slice::UsizeExt,
};

/// Parses a declaration shell around declaration specifiers, comma-separated
/// declarators, optional initializers, and the terminating semicolon.
///
/// C99: declaration, init-declarator-list, and init-declarator are §6.7,
/// p. 97; PDF p. 109.
#[derive(Debug)]
pub(super) struct DeclarationFrame {
    /// Current declaration transition.
    phase: DeclarationPhase,
    /// Specifiers shared by every init-declarator in this declaration.
    declaration_specifiers: Option<DeclarationSpecifiers>,
    /// Initial arena length used to build this declaration's final slice.
    init_declarator_start: u32,
    /// Provenance accumulated across specifiers, declarators, and separators.
    pub(super) source_vectors: Vec<SourceVectors>,
    /// Hard-error count on entry, used to scope recovery to this declaration.
    starting_error_count: usize,
    /// Most recently stored init-declarator, used to attach an initializer.
    last_init_index: Option<u32>,
    /// Provenance of `=` retained while the initializer child runs.
    initializer_source: Option<SourceVectors>,
    /// Grammar context controlling function-definition handoff and `}`
    /// ownership.
    context: DeclarationContext,
    is_function_definition_head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclarationContext {
    External,
    Block,
    ForInitializer,
    OldStyleParameter,
}

/// State transitions for [`DeclarationFrame`].
///
/// C99: §6.7, p. 97; PDF p. 109. The phases encode that production
/// iteratively.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclarationPhase {
    /// Push declaration specifiers.
    Start,
    /// Receive specifiers and decide whether a declarator follows.
    AwaitSpecifiers,
    /// Receive one named declarator.
    AwaitDeclarator,
    /// Resume at a separator after recovering a missing declarator.
    AfterMissingDeclarator,
    /// Classify the token following a completed declarator.
    AfterDeclarator,
    /// Push the initializer child after consuming `=`.
    PushInitializer,
    /// Attach a recovered initializer placeholder.
    AwaitInitializer,
    /// Push another declarator after consuming `,`.
    BeforeNextDeclarator,
    /// Store the declaration and return its arena handle.
    Finish,
}

impl DeclarationFrame {
    pub(super) fn new(
        init_declarator_start: u32,
        context: DeclarationContext,
        starting_error_count: usize,
    ) -> Self {
        Self {
            phase: DeclarationPhase::Start,
            declaration_specifiers: None,
            init_declarator_start,
            source_vectors: Vec::new(),
            starting_error_count,
            last_init_index: None,
            initializer_source: None,
            context,
            is_function_definition_head: false,
        }
    }

    fn recovery_kind(&self) -> SynchronizationKind {
        match self.context {
            | DeclarationContext::Block => SynchronizationKind::BlockDeclaration,
            | DeclarationContext::ForInitializer => SynchronizationKind::ForInitializer,
            | DeclarationContext::External => SynchronizationKind::Declaration,
            | DeclarationContext::OldStyleParameter => SynchronizationKind::OldStyleParameter,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | DeclarationPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Specifiers are a child production because tag
                // specifiers may suspend again
                // for complete struct/union/enum bodies.
                self.phase = DeclarationPhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(SpecifierMode::Declaration),
                ))
            },
            | DeclarationPhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("specifier frame returned an unexpected value: {returned:?}");
                };
                self.declaration_specifiers = Some(specifiers);
                self.source_vectors.push(specifiers.source_vectors);
                // A bare `;` completes the grammar's optional
                // init-declarator-list. A typedef is the exception: it
                // must still introduce a name,
                // so retain the tree but diagnose it.
                if is_operator(token, OperatorTokenType::Semicolon) {
                    if specifiers.storage_class == Some(StorageClass::Typedef) {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedDeclaratorInTypedef(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else {
                    self.phase = DeclarationPhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::Named,
                    )))
                }
            },
            | DeclarationPhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("declarator frame returned an unexpected value: {returned:?}");
                };
                let Some(declarator) = declarator else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclaratorInDeclaration(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterMissingDeclarator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   self.recovery_kind(),
                        target: ParseFrameKind::Declaration,
                    });
                };

                let source_vectors = declarator.source_vectors;
                self.source_vectors.push(source_vectors);
                let init_index = parser.syntax.init_declarators.len().to_u32();
                parser.push_syntax(
                    |syntax| &mut syntax.init_declarators,
                    InitDeclarator {
                        declarator,
                        initializer: None,
                        source_vectors,
                    },
                );
                self.last_init_index = Some(init_index);

                // C scope begins immediately after the declarator,
                // before its initializer or a
                // later comma-separated declarator. Publish
                // now so typedef shadowing affects the very next token.
                if let Some(identifier) = parser.declarator_identifier(declarator) {
                    let class = if self.declaration_specifiers.is_some_and(|specifiers| {
                        specifiers.storage_class == Some(StorageClass::Typedef)
                    }) {
                        NameClass::Typedef
                    } else {
                        NameClass::Ordinary
                    };
                    parser.scopes.publish(identifier.name, class);
                }
                self.phase = DeclarationPhase::AfterDeclarator;
                ParseAction::Continue
            },
            | DeclarationPhase::AfterMissingDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Recovery stops before owning separators. Consume them
                // here; leave any unrelated
                // token untouched for the parent frame.
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = DeclarationPhase::BeforeNextDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    let token = token.expect("semicolon token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    self.phase = DeclarationPhase::Finish;
                    if self.context == DeclarationContext::External {
                        let token = token.expect("closing-curly-brace token exists");
                        self.source_vectors.push(token.source_vectors);
                        ParseAction::Consume
                    } else {
                        ParseAction::Reprocess
                    }
                } else {
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                }
            },
            | DeclarationPhase::AfterDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let has_sole_uninitialized_declarator =
                    parser.syntax.init_declarators.len().to_u32() == self.init_declarator_start + 1
                        && self
                            .last_init_index
                            .and_then(|index| parser.syntax.init_declarators.get(index as usize))
                            .is_some_and(|init| init.initializer.is_none());
                let starts_function_definition = self.context == DeclarationContext::External
                    && has_sole_uninitialized_declarator
                    && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || parser.hard_error_count == self.starting_error_count
                            && token.is_some_and(|token| parser.declaration_starter(token)));
                // The same prefix can continue as another
                // init-declarator, an
                // initializer, a completed declaration, or a function
                // body. A sole declarator
                // followed by `{` or a declaration-list
                // matches the function-definition production. Whether
                // that declarator denotes a
                // function type, and whether its form
                // permits a declaration-list, are separate C
                // constraints.
                if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = DeclarationPhase::BeforeNextDeclarator;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Equals) {
                    self.initializer_source = token.map(|token| token.source_vectors);
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = DeclarationPhase::PushInitializer;
                    ParseAction::Consume
                } else if starts_function_definition {
                    self.is_function_definition_head = true;
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if self.context == DeclarationContext::ForInitializer
                    && is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if self.context == DeclarationContext::OldStyleParameter
                    && is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    if self.context == DeclarationContext::External {
                        if let Some(token) = token {
                            self.source_vectors.push(token.source_vectors);
                        }
                        ParseAction::Consume
                    } else {
                        ParseAction::Reprocess
                    }
                } else if token.is_some_and(|token| {
                    parser.declaration_starter(token)
                        || matches!(
                            self.context,
                            DeclarationContext::Block | DeclarationContext::ForInitializer
                        ) && is_statement_keyword(token.kind)
                }) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if matches!(
                    self.context,
                    DeclarationContext::Block | DeclarationContext::ForInitializer
                ) && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                    || token.is_some_and(|token| token.kind == TokenType::Identifier)
                        && is_operator(parser.cursor.following(context), OperatorTokenType::Colon))
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(None),
                        None,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterDeclarator;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   self.recovery_kind(),
                        target: ParseFrameKind::Declaration,
                    })
                }
            },
            | DeclarationPhase::PushInitializer => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclarationPhase::AwaitInitializer;
                ParseAction::Push(ParseFrame::Initializer(InitializerFrame::new(
                    parser.hard_error_count,
                    self.context == DeclarationContext::ForInitializer,
                    false,
                )))
            },
            | DeclarationPhase::AwaitInitializer => {
                let Some(ParseValue::Initializer(InitializerResult {
                    index: initializer_index,
                    recovered,
                })) = returned
                else {
                    panic!("initializer frame returned an unexpected value: {returned:?}");
                };
                let _ = recovered;
                let source_vectors =
                    parser.syntax.initializers[initializer_index.0 as usize].source_vectors;
                let initializer_source = self
                    .initializer_source
                    .map_or(source_vectors, |equals_source| {
                        context.merge_vectors(equals_source, source_vectors)
                    });
                self.source_vectors.push(source_vectors);
                if let Some(index) = self.last_init_index {
                    let init_declarator = &mut parser.syntax.init_declarators[index as usize];
                    init_declarator.initializer = Some(initializer_index);
                    init_declarator.source_vectors =
                        context.merge_vectors(init_declarator.source_vectors, initializer_source);
                }
                self.phase = DeclarationPhase::AfterDeclarator;
                ParseAction::Continue
            },
            | DeclarationPhase::BeforeNextDeclarator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclarationPhase::AwaitDeclarator;
                ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                    DeclaratorMode::Named,
                )))
            },
            | DeclarationPhase::Finish => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Arena insertion is the reduction boundary: all child
                // slices and source ranges are
                // stable before the handle is returned.
                let index = parser.syntax.declarations.len().to_u32();
                let source_vectors = context.merge_vector_list(&self.source_vectors);
                parser.push_syntax(
                    |syntax| &mut syntax.declarations,
                    Declaration {
                        declaration_specifiers: self
                            .declaration_specifiers
                            .expect("a declaration cannot finish without specifiers"),
                        init_declarators: SyntaxList::new(
                            parser.syntax_id,
                            self.init_declarator_start,
                            parser.syntax.init_declarators.len().to_u32(),
                        ),
                        source_vectors,
                        recovered: parser.hard_error_count > self.starting_error_count,
                        is_function_definition_head: self.is_function_definition_head,
                    },
                );
                ParseAction::Reduce(ParseValue::Declaration(DeclarationIndex(
                    index,
                    parser.syntax_id,
                )))
            },
        }
    }
}
