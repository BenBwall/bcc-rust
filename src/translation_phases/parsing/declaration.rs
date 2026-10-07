//! Declaration frame: specifiers, init-declarator lists, and terminators.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `declaration`, `init-declarator-list`, and `init-declarator` (C99: §6.7
//! paragraph 1, p. 97; PDF p. 109; §A.2.2, p. 411; PDF p. 423) at file
//! scope, in blocks, in `for` clauses, and in old-style parameter
//! declaration lists. The frame also recognizes a declaration-shaped prefix
//! that heads a `function-definition` (§6.9.1 paragraph 1, p. 141;
//! PDF p. 153) and hands it to the external-declaration frame.
//!
//! Each declarator's identifier enters scope as soon as the declarator ends
//! (§6.2.1 paragraph 7, p. 30; PDF p. 42), as a typedef name under a
//! `typedef` specifier (§6.7.7 paragraph 3, p. 123; PDF p. 135). A typedef
//! without declarators is checked against the constraint of §6.7 paragraph
//! 2, p. 97; PDF p. 109; the rest of paragraphs 2-4, the definition
//! semantics of paragraph 5, p. 97; PDF p. 109, and the completeness rule of
//! paragraph 7, p. 98; PDF p. 110, are left to semantic analysis.

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
        DirectDeclarator,
        InitDeclarator,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
    },
    errors::{
        DeclarationContinuation,
        DeclarationPlace,
        ParserErrorType,
    },
    expression_operators::is_operator,
    initializer::InitializerFrame,
    machine::{
        InitializerResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    modern::{
        ModernFrame,
        ModernKind,
        ModernValue,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::NameClass,
    statement::is_statement_keyword,
    syntax::StorageClass,
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            KeywordTokenType,
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

/// Parses a declaration shell around declaration specifiers, comma-separated
/// declarators, optional initializers, and the terminating semicolon.
///
/// C99: declaration, init-declarator-list, and init-declarator are §6.7,
/// p. 97; PDF p. 109.
#[derive(Debug)]
pub(super) struct DeclarationFrame<'tu, 'p> {
    /// Current declaration transition.
    phase: DeclarationPhase,
    /// Specifiers shared by every init-declarator in this declaration.
    declaration_specifiers: Option<DeclarationSpecifiers<'tu>>,
    /// Init-declarators parsed so far, stored as one list when the
    /// declaration reduces.
    pub(super) init_declarators: ArenaVec<'p, InitDeclarator<'tu>>,
    /// Provenance accumulated across specifiers, declarators, and separators.
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    /// Hard-error count on entry, used to scope recovery to this declaration.
    starting_error_count: usize,
    /// Provenance of `=` retained while the initializer child runs.
    initializer_source: Option<SourceVectors>,
    /// Grammar context controlling function-definition handoff and `}`
    /// ownership.
    context: DeclarationContext,
    is_function_definition_head: bool,
}

/// Where a declaration appears, which decides how it may end and what it
/// may hand off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclarationContext {
    /// An `external-declaration`: C99 §6.9 paragraph 1, p. 140; PDF p. 152.
    External,
    /// A `block-item`: C99 §6.8.2 paragraph 1, p. 132; PDF p. 144.
    Block,
    /// The declaration clause of `for`: C99 §6.8.5 paragraph 1, p. 135;
    /// PDF p. 147. Its storage-class constraint (paragraph 3, p. 135;
    /// PDF p. 147) is left to semantic analysis.
    ForInitializer,
    SelectionHeader,
    /// One declaration of a function definition's `declaration-list`: C99
    /// §6.9.1 paragraph 1, p. 141; PDF p. 153.
    OldStyleParameter,
}

/// State transitions for [`DeclarationFrame`].
///
/// C99: §6.7, p. 97; PDF p. 109. The phases encode that production
/// iteratively.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclarationPhase {
    AwaitAssertion,
    /// Push declaration specifiers.
    Start,
    /// Receive specifiers and decide whether a declarator follows.
    AwaitSpecifiers,
    /// Receive one named declarator.
    AwaitDeclarator,
    /// Resume at a separator after recovering a missing declarator or a
    /// rejected continuation, without diagnosing the synchronization token
    /// again.
    AfterRecovery,
    /// Classify the token following a completed declarator.
    AfterDeclarator,
    /// Push the initializer child after consuming `=`.
    PushInitializer,
    /// Attach a recovered initializer placeholder.
    AwaitInitializer,
    /// Push another declarator after consuming `,`.
    BeforeNextDeclarator,
    /// Store the declaration and return it.
    Finish,
}

impl<'tu, 'p> DeclarationFrame<'tu, 'p> {
    pub(super) fn new(
        arena: &'p Bump,
        context: DeclarationContext,
        starting_error_count: usize,
    ) -> Self {
        Self {
            phase: DeclarationPhase::Start,
            declaration_specifiers: None,
            init_declarators: ArenaVec::new_in(arena),
            source_vectors: ArenaVec::new_in(arena),
            starting_error_count,
            initializer_source: None,
            context,
            is_function_definition_head: false,
        }
    }

    /// Tokens that may still continue this declaration after its latest
    /// declarator, for the continuation diagnostic.
    fn continuation(&self) -> DeclarationContinuation {
        let place = match self.context {
            | DeclarationContext::External => DeclarationPlace::External,
            | DeclarationContext::Block => DeclarationPlace::Block,
            | DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader =>
                DeclarationPlace::ForInitializer,
            | DeclarationContext::OldStyleParameter => DeclarationPlace::OldStyleParameter,
        };
        DeclarationContinuation {
            place,
            initializer: self
                .init_declarators
                .last()
                .is_some_and(|init| init.initializer.is_none()),
            function_body: place == DeclarationPlace::External
                && matches!(
                    self.init_declarators.as_slice(),
                    [init] if init.initializer.is_none()
                ),
        }
    }

    fn recovery_kind(&self) -> SynchronizationKind {
        match self.context {
            | DeclarationContext::Block => SynchronizationKind::BlockDeclaration,
            | DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader =>
                SynchronizationKind::ForInitializer,
            | DeclarationContext::External => SynchronizationKind::Declaration,
            | DeclarationContext::OldStyleParameter => SynchronizationKind::OldStyleParameter,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | DeclarationPhase::AwaitAssertion => {
                let Some(ParseValue::Modern(ModernValue::Assertion(assertion))) = returned else {
                    panic!("assertion declaration protocol: {returned:?}")
                };
                ParseAction::Reduce(ParseValue::Declaration(parser.alloc_syntax(Declaration {
                    assertion:                   Some(assertion),
                    declaration_specifiers:      DeclarationSpecifiers::new(),
                    init_declarators:            crate::util::arena_list::ArenaList::empty(),
                    source_vectors:              assertion.source_vectors,
                    recovered:                   assertion.recovered,
                    is_function_definition_head: false,
                })))
            },
            | DeclarationPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if token
                    .is_some_and(|x| x.kind == TokenType::Keyword(KeywordTokenType::StaticAssert))
                {
                    self.phase = DeclarationPhase::AwaitAssertion;
                    return ParseAction::Push(ParseFrame::Modern(ModernFrame::new(
                        parser.arena,
                        ModernKind::Assertion,
                        parser.hard_error_count,
                    )));
                }
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
                // init-declarator-list. A typedef without one names no type,
                // so retain the tree but diagnose it. C99 §6.7p2 asks only
                // for a declarator, a tag, or enumeration members, so
                // `typedef struct s { int m; };` is valid and only warned
                // about; `typedef int;` declares nothing. Other empty
                // declarations (`int;`) are left to semantic analysis.
                if is_operator(token, OperatorTokenType::Semicolon) {
                    if specifiers.storage_class == Some(StorageClass::Typedef) {
                        let error = if specifiers.type_specifiers.may_declare_tag_or_enumerators() {
                            ParserErrorType::TypedefDeclaresNoName
                        } else {
                            ParserErrorType::ExpectedDeclaratorInTypedef(
                                token.map(|token| token.kind),
                            )
                        };
                        parser.report(error, token);
                    }
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Consume
                } else {
                    self.phase = DeclarationPhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        parser.arena,
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
                        ParserErrorType::ExpectedDeclaratorInDeclaration(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterRecovery;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   self.recovery_kind(),
                        target: ParseFrameKind::Declaration,
                    });
                };

                let source_vectors = declarator.source_vectors;
                self.source_vectors.push(source_vectors);
                self.init_declarators.push(InitDeclarator {
                    declarator,
                    initializer: None,
                    source_vectors,
                });

                // C scope begins immediately after the declarator,
                // before its initializer or a
                // later comma-separated declarator. Publish
                // now so typedef shadowing affects the very next token.
                // C99 §6.2.1p7; typedef names share the ordinary name space
                // (§6.7.7p3).
                if let Some(identifier) = declarator.identifier() {
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
            | DeclarationPhase::AfterRecovery => {
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
                let has_sole_uninitialized_declarator = matches!(
                    self.init_declarators.as_slice(),
                    [init] if init.initializer.is_none()
                );
                // An identifier-list head keeps its declaration-list even
                // after a recovered error in the head: no other declaration
                // form can continue with a declaration there, while a broken
                // non-function declarator (`int x[3 int y;`) must still
                // resynchronize at the next declaration.
                let old_style_parameters = if has_sole_uninitialized_declarator
                    && parser.hard_error_count != self.starting_error_count
                {
                    self.init_declarators
                        .first()
                        .and_then(|init| init.declarator.function_suffix())
                        .and_then(|suffix| match suffix {
                            | DirectDeclarator::KAndRStyleFunction { parameters } =>
                                Some(parameters),
                            | _ => None,
                        })
                } else {
                    None
                };
                // After an error in an identifier-list head, a following
                // declaration still begins the declaration list only when
                // it declares one of the listed parameters; otherwise the
                // head most likely lacks its `;`.
                let continues_old_style_definition = parser.hard_error_count
                    != self.starting_error_count
                    && token.is_some_and(|token| parser.declaration_starter(token))
                    && old_style_parameters.is_some_and(|parameters| {
                        parser.next_declaration_declares_one_of(parameters.as_slice())
                    });
                let starts_function_definition = self.context == DeclarationContext::External
                    && has_sole_uninitialized_declarator
                    && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || parser.hard_error_count == self.starting_error_count
                            && token.is_some_and(|token| parser.declaration_starter(token))
                        || continues_old_style_definition);
                // The same prefix can continue as another
                // init-declarator, an
                // initializer, a completed declaration, or a function
                // body. A sole declarator
                // followed by `{` or a declaration-list
                // matches the function-definition production. Whether
                // that declarator denotes a
                // function type, and whether its form
                // permits a declaration-list, are separate C
                // constraints (C99 §6.9.1p2 and p5-6).
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
                } else if matches!(
                    self.context,
                    DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader
                ) && is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if self.context == DeclarationContext::OldStyleParameter
                    && is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                {
                    parser.report(
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                            self.continuation(),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    parser.report(
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                            self.continuation(),
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
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                            self.continuation(),
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
                        && is_operator(parser.cursor.following(), OperatorTokenType::Colon))
                {
                    parser.report(
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                            self.continuation(),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            None,
                            self.continuation(),
                        ),
                        None,
                    );
                    self.phase = DeclarationPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
                            token.map(|token| token.kind),
                            self.continuation(),
                        ),
                        token,
                    );
                    self.phase = DeclarationPhase::AfterRecovery;
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
                    parser.arena,
                    parser.hard_error_count,
                    matches!(
                        self.context,
                        DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader
                    ),
                    false,
                )))
            },
            | DeclarationPhase::AwaitInitializer => {
                let Some(ParseValue::Initializer(InitializerResult {
                    initializer: initializer_index,
                    recovered,
                })) = returned
                else {
                    panic!("initializer frame returned an unexpected value: {returned:?}");
                };
                let _ = recovered;
                let source_vectors = initializer_index.source_vectors;
                let initializer_source = self
                    .initializer_source
                    .map_or(source_vectors, |equals_source| {
                        parser.context.merge_vectors(equals_source, source_vectors)
                    });
                self.source_vectors.push(source_vectors);
                if let Some(init_declarator) = self.init_declarators.last_mut() {
                    init_declarator.initializer = Some(initializer_index);
                    init_declarator.source_vectors = parser
                        .context
                        .merge_vectors(init_declarator.source_vectors, initializer_source);
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
                    parser.arena,
                    DeclaratorMode::Named,
                )))
            },
            | DeclarationPhase::Finish => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Arena insertion is the reduction boundary: all child
                // slices and source ranges are complete before the
                // declaration is stored and returned.
                let source_vectors = parser.context.merge_vector_list(&self.source_vectors);
                let init_declarators = parser.alloc_syntax_list(&mut self.init_declarators);
                let declaration = parser.alloc_syntax(Declaration {
                    assertion: None,
                    declaration_specifiers: self
                        .declaration_specifiers
                        .expect("a declaration cannot finish without specifiers"),
                    init_declarators,
                    source_vectors,
                    recovered: parser.hard_error_count > self.starting_error_count,
                    is_function_definition_head: self.is_function_definition_head,
                });
                ParseAction::Reduce(ParseValue::Declaration(declaration))
            },
        }
    }
}
