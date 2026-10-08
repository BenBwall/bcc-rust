//! Statement frame covering every C99 statement family.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `statement` and its alternatives. C99: §6.8, pp. 131-139; PDF
//! pp. 143-151; §A.2.3, pp. 415-416; PDF pp. 427-428:
//!
//! - `labeled-statement`: §6.8.1, pp. 131-132; PDF pp. 143-144.
//! - `compound-statement`: §6.8.2, p. 132; PDF p. 144, delegated to the
//!   compound-statement frame.
//! - `expression-statement` and the null statement: §6.8.3, p. 132; PDF p. 144.
//! - `selection-statement` (`if`, `switch`): §6.8.4, p. 133; PDF p. 145.
//! - `iteration-statement` (`while`, `do`, `for`): §6.8.5, p. 135; PDF p. 147,
//!   with the `for` declaration clause of §6.8.5.3, p. 136; PDF p. 148.
//! - `jump-statement` (`goto`, `continue`, `break`, `return`): §6.8.6, p. 136;
//!   PDF p. 148.
//!
//! Selection and iteration statements, and each of their substatements, are
//! blocks (§6.8.4 paragraph 3, p. 133; PDF p. 145; §6.8.5 paragraph 5,
//! p. 135; PDF p. 147), so the frame opens implicit scopes for them.
//!
//! Only syntax and the duplicate-`default` constraint (§6.8.4.2 paragraph 3,
//! p. 134; PDF p. 146) are checked here. The remaining statement
//! constraints belong to semantic analysis: `case`/`default` outside a
//! `switch` (§6.8.1 paragraph 2, p. 131; PDF p. 143), unique label names
//! (§6.8.1 paragraph 3, p. 132; PDF p. 144), controlling-expression types
//! (§6.8.4.1 paragraph 1, p. 133; PDF p. 145; §6.8.4.2 paragraph 1, p. 134;
//! PDF p. 146; §6.8.5 paragraph 2, p. 135; PDF p. 147), integer constant
//! `case` values and duplicates (§6.8.4.2 paragraph 3, p. 134; PDF p. 146),
//! `for` declaration storage classes (§6.8.5 paragraph 3, p. 135;
//! PDF p. 147), `goto` targets (§6.8.6.1 paragraph 1, p. 137; PDF p. 149),
//! `continue` and `break` placement (§6.8.6.2 paragraph 1 and §6.8.6.3
//! paragraph 1, p. 138; PDF p. 150), and `return` operands (§6.8.6.4
//! paragraph 1, p. 139; PDF p. 151).

use std::fmt::Debug;

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    expression_operators::{
        is_operator,
        is_postfix_starter,
    },
    machine::{
        ConstantExpressionResult,
        ExpressionResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    modern::{
        AttributeSpecifier,
        ModernFrame,
        ModernKind,
        ModernValue,
    },
    recovery::{
        ExpressionTerminator,
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::{
        ScopeKind,
        SwitchScope,
    },
    syntax::{
        AttributedStatement,
        CaseRange,
        ConstantExpressionSlot,
        ExpressionSlot,
        ForInitializer,
        ForStatement,
        Identifier,
        SelectionHeader,
        Statement,
        StatementType,
    },
};
use crate::translation_phases::{
    Context,
    SourceVectors,
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        Token,
        TokenType,
    },
};

/// Tokens a malformed statement header may span before recovery stops
/// looking for its closing parenthesis.
const HEADER_RECOVERY_LOOKAHEAD: u16 = 256;

/// Statements whose header is `( expression )` followed by one substatement.
///
/// C99: `if` and `switch` are §6.8.4, p. 133; PDF p. 145; `while` is
/// §6.8.5, p. 135; PDF p. 147.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    If,
    Switch,
    While,
}

/// The prefix of a `labeled-statement`: `identifier :`,
/// `case constant-expression :`, or `default :`.
///
/// C99: §6.8.1 paragraph 1, p. 131; PDF p. 143.
#[derive(Debug, Clone, Copy)]
enum LabelPrefix<'tu> {
    Identifier(Identifier),
    Case(ConstantExpressionSlot<'tu>),
    Range(ConstantExpressionSlot<'tu>, ConstantExpressionSlot<'tu>),
    Default,
}

/// The operand-free `jump-statement`s `break ;` and `continue ;`.
///
/// C99: §6.8.6 paragraph 1, p. 136; PDF p. 148.
#[derive(Debug, Clone, Copy)]
enum SimpleJump {
    Break,
    Continue,
}

/// Resumable positions inside one statement production. The `Header*` and
/// `If*` phases cover the parenthesized-header statements, `Do*` the `do`
/// statement, and `For*` the `for` statement of §6.8.5, p. 135; PDF p. 147.
#[derive(Debug, Clone, Copy)]
enum StatementPhase<'tu> {
    Start,
    AwaitGnu,
    AwaitMsvc,
    ComputedGotoExpression,
    AwaitComputedGoto,
    ComputedGotoSemicolon(ExpressionSlot<'tu>),
    AwaitAttributes,
    AwaitAttributedStatement(&'tu AttributeSpecifier<'tu>),
    AwaitLabeledDeclaration(LabelPrefix<'tu>),
    NamedJumpSemicolon(SimpleJump, Identifier),
    AwaitHeaderDeclaration(HeaderKind),
    AwaitSelectionExpression(HeaderKind, &'tu super::declaration_syntax::Declaration<'tu>),
    PushCaseRange(ConstantExpressionSlot<'tu>),
    AwaitCaseRange(ConstantExpressionSlot<'tu>),
    AwaitCompound,
    AwaitExpression,
    ExpressionSemicolon(ExpressionSlot<'tu>),
    ReturnStart,
    AwaitReturnExpression,
    ReturnSemicolon(Option<ExpressionSlot<'tu>>),
    SimpleJumpSemicolon(SimpleJump),
    GotoIdentifier,
    GotoSemicolon(Identifier),
    IdentifierLabelColon(Identifier),
    CaseExpression,
    AwaitCaseExpression,
    CaseColon(ConstantExpressionSlot<'tu>),
    DefaultColon,
    PushLabeled(LabelPrefix<'tu>),
    AwaitLabeled(LabelPrefix<'tu>),
    HeaderOpening(HeaderKind),
    HeaderExpression(HeaderKind),
    AwaitHeaderExpression(HeaderKind),
    HeaderClosing(HeaderKind, ExpressionSlot<'tu>),
    PushHeaderBody(HeaderKind, ExpressionSlot<'tu>),
    AwaitHeaderBody(HeaderKind, ExpressionSlot<'tu>),
    IfAfterThen(ExpressionSlot<'tu>, &'tu Statement<'tu>),
    PushElse(ExpressionSlot<'tu>, &'tu Statement<'tu>),
    AwaitElse(ExpressionSlot<'tu>, &'tu Statement<'tu>),
    DoPushBody,
    DoAwaitBody,
    DoWhileKeyword(&'tu Statement<'tu>),
    DoOpening(&'tu Statement<'tu>),
    DoExpression(&'tu Statement<'tu>),
    DoAwaitExpression(&'tu Statement<'tu>),
    DoClosing(&'tu Statement<'tu>, ExpressionSlot<'tu>),
    DoSemicolon(&'tu Statement<'tu>, ExpressionSlot<'tu>),
    ForOpening,
    ForInitializer,
    AwaitForInitializerExpression,
    AwaitForInitializerDeclaration,
    ForInitializerSemicolon(ExpressionSlot<'tu>),
    ForCondition(Option<ForInitializer<'tu>>),
    AwaitForCondition(Option<ForInitializer<'tu>>),
    ForConditionSemicolon(Option<ForInitializer<'tu>>, ExpressionSlot<'tu>),
    ForIteration(Option<ForInitializer<'tu>>, Option<ExpressionSlot<'tu>>),
    AwaitForIteration(Option<ForInitializer<'tu>>, Option<ExpressionSlot<'tu>>),
    ForClosing(
        Option<ForInitializer<'tu>>,
        Option<ExpressionSlot<'tu>>,
        Option<ExpressionSlot<'tu>>,
    ),
    /// Consumes the counted remaining tokens of a malformed `for` header,
    /// which include an extra `;` clause, before its closing parenthesis.
    ForSkipHeader(
        Option<ForInitializer<'tu>>,
        Option<ExpressionSlot<'tu>>,
        Option<ExpressionSlot<'tu>>,
        u16,
    ),
    ForPushBody(
        Option<ForInitializer<'tu>>,
        Option<ExpressionSlot<'tu>>,
        Option<ExpressionSlot<'tu>>,
    ),
    ForAwaitBody(
        Option<ForInitializer<'tu>>,
        Option<ExpressionSlot<'tu>>,
        Option<ExpressionSlot<'tu>>,
    ),
    Recovered,
    Finish(StatementType<'tu>),
}

/// Resumable `statement` production.
///
/// C99: §6.8 paragraph 1, p. 131; PDF p. 143; §A.2.3, pp. 415-416;
/// PDF pp. 427-428.
#[derive(Debug)]
pub(super) struct StatementFrame<'tu> {
    phase:                     StatementPhase<'tu>,
    pub(super) source_vectors: Option<SourceVectors>,
    starting_error_count:      usize,
    entry_scope_depth:         Option<usize>,
    implicit_scope:            Option<ScopeKind>,
    owns_switch_scope:         bool,
    leave_else_unconsumed:     bool,
    /// C23 permits standalone labels among compound block items, not as a
    /// selection/iteration substatement whose required statement is absent.
    block_item:                bool,
}

impl HeaderKind {
    fn name(self) -> &'static str {
        match self {
            | Self::If => "if statement",
            | Self::Switch => "switch statement",
            | Self::While => "while statement",
        }
    }

    /// The implicit block each substatement forms.
    ///
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145; §6.8.5 paragraph 5,
    /// p. 135; PDF p. 147.
    fn scope_kind(self) -> ScopeKind {
        match self {
            | Self::If | Self::Switch => ScopeKind::ImplicitSelection,
            | Self::While => ScopeKind::ImplicitIteration,
        }
    }
}

#[expect(
    clippy::missing_assert_message,
    clippy::too_many_lines,
    reason = "The single iterative statement grammar dispatcher keeps all phase transitions and \
              delimiter ownership visible in one frame implementation."
)]
impl<'tu, 'p> StatementFrame<'tu> {
    pub(super) fn new(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self {
            phase: StatementPhase::Start,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            implicit_scope,
            owns_switch_scope: false,
            leave_else_unconsumed: false,
            block_item: false,
        }
    }

    pub(super) fn with_block_item(mut self, block_item: bool) -> Self {
        self.block_item = block_item;
        self
    }

    fn with_else_ownership(
        starting_error_count: usize,
        implicit_scope: Option<ScopeKind>,
        leave_else_unconsumed: bool,
    ) -> Self {
        Self {
            leave_else_unconsumed,
            ..Self::new(starting_error_count, implicit_scope)
        }
    }

    /// The `then` substatement leaves a following `else` to its `if`, so the
    /// `else` pairs with the lexically nearest `if`.
    ///
    /// C99: §6.8.4.1 paragraph 3, p. 134; PDF p. 146.
    fn for_if_then(starting_error_count: usize, implicit_scope: Option<ScopeKind>) -> Self {
        Self::with_else_ownership(starting_error_count, implicit_scope, true)
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        if self.entry_scope_depth.is_none() {
            self.entry_scope_depth = Some(parser.scopes.depth());
            if let Some(kind) = self.implicit_scope {
                parser.scopes.enter_scope(kind);
            }
        }

        match self.phase {
            | StatementPhase::AwaitMsvc => {
                let Some(ParseValue::Statement(child)) = returned else {
                    panic!("MSVC statement child protocol")
                };
                self.finish_existing(parser, child)
            },
            | StatementPhase::AwaitGnu => match returned {
                | Some(ParseValue::Gnu(super::gnu::GnuValue::Asm(x))) => {
                    self.source_vectors = Some(x.source_vectors);
                    self.finish(parser, StatementType::Asm(x))
                },
                | Some(ParseValue::Gnu(super::gnu::GnuValue::LocalLabels(labels, source))) => {
                    self.source_vectors = Some(source);
                    self.finish(parser, StatementType::LocalLabels(labels))
                },
                | _ => panic!("GNU statement child protocol"),
            },
            | StatementPhase::ComputedGotoExpression => {
                self.phase = StatementPhase::AwaitComputedGoto;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                    parser.hard_error_count,
                )))
            },
            | StatementPhase::AwaitComputedGoto => {
                let expression = super::machine::any_expression_value(returned);
                let slot = ExpressionSlot::Parsed(expression);
                parser
                    .context
                    .merge_into(&mut self.source_vectors, expression.source_vectors);
                self.phase = StatementPhase::ComputedGotoSemicolon(slot);
                ParseAction::Continue
            },
            | StatementPhase::ComputedGotoSemicolon(slot) => {
                self.own_semicolon_or_report(parser, token, "computed goto");
                self.phase = StatementPhase::Finish(StatementType::ComputedGoto(slot));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::AwaitAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(x))) = returned else {
                    panic!("statement attributes protocol: {returned:?}")
                };
                self.source_vectors = Some(x.source_vectors);
                self.phase = StatementPhase::AwaitAttributedStatement(x);
                ParseAction::Push(ParseFrame::Statement(
                    StatementFrame::with_else_ownership(
                        parser.hard_error_count,
                        None,
                        self.leave_else_unconsumed,
                    )
                    .with_block_item(self.block_item),
                ))
            },
            | StatementPhase::AwaitAttributedStatement(attributes) => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("attributed statement protocol: {returned:?}")
                };
                self.merge_statement(parser.context, statement);
                let node = parser.alloc_syntax(AttributedStatement {
                    attributes,
                    statement,
                });
                self.finish(parser, StatementType::Attributed(node))
            },
            | StatementPhase::AwaitLabeledDeclaration(prefix) => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("labeled declaration protocol: {returned:?}")
                };
                let child = parser.alloc_syntax(Statement {
                    kind:           StatementType::Declaration(declaration),
                    source_vectors: declaration.source_vectors,
                    recovered:      declaration.recovered,
                });
                self.merge_statement(parser.context, child);
                let kind = match prefix {
                    | LabelPrefix::Identifier(x) => StatementType::Label(x, child),
                    | LabelPrefix::Case(x) => StatementType::Case(x, child),
                    | LabelPrefix::Default => StatementType::Default(child),
                    | LabelPrefix::Range(lower, upper) =>
                        StatementType::CaseRange(parser.alloc_syntax(CaseRange {
                            lower,
                            upper,
                            statement: child,
                        })),
                };
                self.finish(parser, kind)
            },
            | StatementPhase::NamedJumpSemicolon(jump, identifier) => {
                self.own_semicolon_or_report(parser, token, "named jump statement");
                self.phase = StatementPhase::Finish(match jump {
                    | SimpleJump::Break => StatementType::NamedBreak(identifier),
                    | SimpleJump::Continue => StatementType::NamedContinue(identifier),
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::AwaitHeaderDeclaration(kind) => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("selection declaration protocol: {returned:?}")
                };
                if is_operator(token, OperatorTokenType::ClosingParenthesis)
                    && !is_operator(parser.cursor.previous, OperatorTokenType::Semicolon)
                {
                    if !matches!(declaration.init_declarators.as_slice(),[init] if init.initializer.is_some())
                    {
                        parser.report(
                            ParserErrorType::ExpectedIsoSyntax(
                                "a single initialized declaration in selection header",
                                token.map(|x| x.kind),
                            ),
                            token,
                        );
                    }
                    let header = parser.alloc_syntax(SelectionHeader {
                        declaration,
                        expression: None,
                    });
                    self.phase =
                        StatementPhase::HeaderClosing(kind, ExpressionSlot::Selection(header));
                    return ParseAction::Continue;
                }
                self.phase = StatementPhase::AwaitSelectionExpression(kind, declaration);
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingParenthesis,
                    parser.hard_error_count,
                )))
            },
            | StatementPhase::AwaitSelectionExpression(kind, declaration) => {
                let slot = Self::parsed_slot(returned);
                let header = parser.alloc_syntax(SelectionHeader {
                    declaration,
                    expression: Some(slot),
                });
                self.phase = StatementPhase::HeaderClosing(kind, ExpressionSlot::Selection(header));
                ParseAction::Continue
            },
            | StatementPhase::PushCaseRange(lower) => {
                self.phase = StatementPhase::AwaitCaseRange(lower);
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::ConstantExpression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                    parser.hard_error_count,
                )))
            },
            | StatementPhase::AwaitCaseRange(lower) => {
                let upper = Self::parsed_constant_slot(returned);
                self.merge_constant_slot(parser.context, upper);
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Range(lower, upper));

                if is_operator(token, OperatorTokenType::Colon) {
                    self.merge_token(parser, token.expect("colon exists"));
                    ParseAction::Consume
                } else {
                    self.own_colon_or_report(parser, token, "case range");
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::Start => {
                if let Some(token) = token
                    && let TokenType::Keyword(
                        keyword @ (KeywordTokenType::MsAsm
                        | KeywordTokenType::Try
                        | KeywordTokenType::Leave),
                    ) = token.kind
                {
                    if keyword == KeywordTokenType::MsAsm
                        && token.contents == KeywordTokenType::MsAsm.cache_id()
                        && parser.cursor.following().is_some_and(|next| {
                            matches!(
                                next.kind,
                                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                                    | TokenType::Keyword(
                                        KeywordTokenType::Volatile
                                            | KeywordTokenType::Inline
                                            | KeywordTokenType::Goto
                                    )
                            )
                        })
                    {
                        self.phase = StatementPhase::AwaitGnu;
                        return super::gnu::GnuFrame::push(
                            parser,
                            super::gnu::GnuKind::Asm { label: false },
                        );
                    }
                    self.phase = StatementPhase::AwaitMsvc;
                    return ParseAction::Push(ParseFrame::Msvc(parser.pools.msvc(
                        super::msvc::MsvcFrame::new(parser.arena, keyword, parser.hard_error_count),
                    )));
                }
                if let Some(token) = token {
                    let kind = match token.kind {
                        | TokenType::Keyword(KeywordTokenType::Asm) =>
                            Some(super::gnu::GnuKind::Asm { label: false }),
                        | TokenType::Keyword(KeywordTokenType::LocalLabel) =>
                            Some(super::gnu::GnuKind::LocalLabels),
                        | _ => None,
                    };
                    if let Some(kind) = kind {
                        self.phase = StatementPhase::AwaitGnu;
                        return super::gnu::GnuFrame::push(parser, kind);
                    }
                }
                debug_assert!(returned.is_none());
                if parser.attribute_starter(token) {
                    self.phase = StatementPhase::AwaitAttributes;
                    return ParseAction::Push(ParseFrame::Modern(parser.pools.modern(
                        ModernFrame::new(
                            parser.arena,
                            ModernKind::Attributes,
                            parser.hard_error_count,
                        ),
                    )));
                }
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = StatementPhase::AwaitCompound;
                    return ParseAction::Push(ParseFrame::CompoundStatement(
                        CompoundStatementFrame::new(parser.arena, parser.hard_error_count, false),
                    ));
                }
                // C99 §6.8.3p1, p3: a lone `;` is the null statement.
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    return ParseAction::Consume;
                }
                // C99 §6.8.1p1: `identifier :` declares a label, which has
                // function scope (§6.2.1p3). Uniqueness (§6.8.1p3) is a
                // semantic check.
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                    && is_operator(parser.cursor.following(), OperatorTokenType::Colon)
                {
                    let identifier = Identifier::from_token(token);
                    if let Some(labels) = parser.label_scopes.innermost_mut() {
                        _ = labels.definitions.insert(identifier.name);
                    }
                    self.merge_token(parser, token);
                    self.phase = StatementPhase::IdentifierLabelColon(identifier);
                    return ParseAction::Consume;
                }
                if let Some(token) = token
                    && let TokenType::Keyword(keyword) = token.kind
                {
                    match keyword {
                        | KeywordTokenType::Return => {
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::ReturnStart;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Break => {
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Break);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Continue => {
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::SimpleJumpSemicolon(SimpleJump::Continue);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Goto => {
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::GotoIdentifier;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Case => {
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::CaseExpression;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Default => {
                            // C99 §6.8.4.2p3: at most one `default` label
                            // per `switch`. The duplicate is the label itself,
                            // so the diagnostic
                            // belongs on its keyword rather than
                            // on whatever token follows it.
                            if parser.switch_scopes.len() > parser.switch_floor
                                && parser
                                    .switch_scopes
                                    .last()
                                    .is_some_and(|switch| switch.has_default)
                            {
                                parser.report(ParserErrorType::DuplicateDefaultLabel, Some(token));
                            } else if parser.switch_scopes.len() > parser.switch_floor
                                && let Some(switch) = parser.switch_scopes.last_mut()
                            {
                                switch.has_default = true;
                            }
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::DefaultColon;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::If => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::If);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Switch => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitSelection);
                            parser.switch_scopes.push(SwitchScope::default());
                            self.owns_switch_scope = true;
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::Switch);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::While => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::HeaderOpening(HeaderKind::While);
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Do => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::DoPushBody;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::For => {
                            Self::enter_construct_scope(parser, ScopeKind::ImplicitIteration);
                            self.merge_token(parser, token);
                            self.phase = StatementPhase::ForOpening;
                            return ParseAction::Consume;
                        },
                        | KeywordTokenType::Sizeof | KeywordTokenType::Extension => {},
                        | _ if parser.declaration_starter(token) => {
                            parser.report(
                                ParserErrorType::ExpectedStatement(Some(token.kind)),
                                Some(token),
                            );
                            self.phase = StatementPhase::Recovered;
                            return ParseAction::Recover(SynchronizationSet {
                                kind:   SynchronizationKind::Statement,
                                target: ParseFrameKind::Statement,
                            });
                        },
                        | _ => {},
                    }
                }
                // A typedef name that continues into declaration specifiers
                // or a declarator starts a declaration, which C99 allows only
                // as a block item, never as a substatement (§6.8.2p1). Compound
                // blocks route such tokens to a declaration
                // frame before a statement frame starts, so
                // only substatements reach this point.
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                    && parser.declaration_recovery_starts_here(token)
                {
                    parser.report(
                        ParserErrorType::ExpectedStatement(Some(token.kind)),
                        Some(token),
                    );
                    self.phase = StatementPhase::Recovered;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Statement,
                        target: ParseFrameKind::Statement,
                    });
                }
                // A closing delimiter or label colon cannot start any
                // statement or expression; skip just that token so the
                // following statement parses without a cascade.
                if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Operator(
                            OperatorTokenType::Colon
                                | OperatorTokenType::ClosingParenthesis
                                | OperatorTokenType::ClosingSquareBracket
                        )
                    )
                {
                    parser.report(
                        ParserErrorType::ExpectedStatement(Some(token.kind)),
                        Some(token),
                    );
                    self.merge_token(parser, token);
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    return ParseAction::Consume;
                }
                let stray_else = token.is_some_and(|token| {
                    matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else))
                });
                if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || stray_else
                {
                    parser.report(
                        ParserErrorType::ExpectedStatement(token.map(|token| token.kind)),
                        token,
                    );
                    if stray_else && !self.leave_else_unconsumed {
                        self.merge_token(parser, token.expect("else token exists"));
                        self.phase = StatementPhase::Finish(StatementType::Null);
                        return ParseAction::Consume;
                    }
                    return self.finish(parser, StatementType::Null);
                }
                // C99 §6.8.3p1: `expression-statement`.
                self.phase = StatementPhase::AwaitExpression;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                    parser.hard_error_count,
                )))
            },
            | StatementPhase::AwaitCompound => {
                let Some(ParseValue::CompoundStatement(statement)) = returned else {
                    panic!("compound statement returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, statement);
                self.finish_existing(parser, statement)
            },
            | StatementPhase::AwaitExpression => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser.context, slot);
                self.phase = StatementPhase::ExpressionSemicolon(slot);
                ParseAction::Continue
            },
            | StatementPhase::ExpressionSemicolon(slot) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "expression statement");
                self.phase = StatementPhase::Finish(StatementType::Expression(slot));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ReturnStart => {
                // C99 §6.8.6p1: `return expression(opt) ;`.
                debug_assert!(returned.is_none());
                let identifier_continues_expression = token
                    .is_some_and(|token| matches!(token.kind, TokenType::Identifier))
                    && parser.cursor.following().is_some_and(|following| {
                        matches!(
                            following.kind,
                            TokenType::Operator(OperatorTokenType::Asterisk)
                        ) || is_postfix_starter(following.kind)
                    });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Return(None));
                    ParseAction::Consume
                } else if Self::at_expression_boundary(token, ExpressionTerminator::Semicolon)
                    || token.is_some_and(|token| {
                        !identifier_continues_expression
                            && parser.declaration_recovery_starts_here(token)
                    })
                    || token.is_some_and(|token| matches!(token.kind, TokenType::Identifier))
                        && is_operator(parser.cursor.following(), OperatorTokenType::Colon)
                {
                    self.phase = StatementPhase::ReturnSemicolon(None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitReturnExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitReturnExpression => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser.context, slot);
                self.phase = StatementPhase::ReturnSemicolon(Some(slot));
                ParseAction::Reprocess
            },
            | StatementPhase::ReturnSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "return statement");
                self.phase = StatementPhase::Finish(StatementType::Return(expression));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::SimpleJumpSemicolon(jump) => {
                // C2y named loops: `break identifier ;`. Without that `;`
                // the identifier more likely starts the next statement after
                // a `break` missing its own.
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                    && is_operator(parser.cursor.following(), OperatorTokenType::Semicolon)
                {
                    parser.extension(
                        crate::configuration::Feature::NamedLoops,
                        "named loop control",
                        token,
                    );
                    self.merge_token(parser, token);
                    self.phase =
                        StatementPhase::NamedJumpSemicolon(jump, Identifier::from_token(token));
                    return ParseAction::Consume;
                }
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "jump statement");
                self.phase = StatementPhase::Finish(match jump {
                    | SimpleJump::Break => StatementType::Break,
                    | SimpleJump::Continue => StatementType::Continue,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::GotoIdentifier => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Asterisk))
                {
                    parser.extension(
                        crate::configuration::Feature::LabelsAsValues,
                        "computed goto",
                        token,
                    );
                    self.merge_token(parser, token);
                    self.phase = StatementPhase::ComputedGotoExpression;
                    return ParseAction::Consume;
                }
                debug_assert!(returned.is_none());
                let identifier = if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    let identifier = Identifier::from_token(token);
                    // C99 §6.8.6.1p1: the target is checked against the
                    // function's labels later.
                    if let Some(labels) = parser.label_scopes.innermost_mut() {
                        _ = labels.references.insert(identifier.name);
                    }
                    self.merge_token(parser, token);
                    self.phase = StatementPhase::GotoSemicolon(identifier);
                    return ParseAction::Consume;
                } else {
                    parser.report(
                        ParserErrorType::ExpectedGotoLabel(token.map(|token| token.kind)),
                        token,
                    );
                    Identifier::new(
                        parser.context.string_cache.intern("<missing-label>"),
                        parser.missing_syntax_source(),
                    )
                };
                self.phase = StatementPhase::GotoSemicolon(identifier);
                ParseAction::Reprocess
            },
            | StatementPhase::GotoSemicolon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "goto statement");
                self.phase = StatementPhase::Finish(StatementType::Goto(identifier));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::IdentifierLabelColon(identifier) => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, token, "identifier label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Identifier(identifier));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::CaseExpression => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::Colon) {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "case label",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::CaseColon(Self::missing_constant_slot(parser));
                    ParseAction::Reprocess
                } else {
                    // C99 §6.8.1p1: `case constant-expression :`; the
                    // integer-constant requirement (§6.8.4.2p3) is semantic.
                    self.phase = StatementPhase::AwaitCaseExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::ConstantExpression,
                        ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitCaseExpression => {
                let slot = Self::parsed_constant_slot(returned);
                self.merge_constant_slot(parser.context, slot);
                if is_operator(token, OperatorTokenType::Ellipsis) {
                    let token = token.expect("ellipsis exists");
                    if parser.pedantic_suppression == 0
                        && parser.context.configuration.standard()
                            < crate::configuration::CStandard::C2y
                    {
                        parser.context.report_extension_since(
                            "case range",
                            crate::configuration::FeatureOrigin::Gnu,
                            token.source_vectors,
                        );
                    }
                    self.merge_token(parser, token);
                    self.phase = StatementPhase::PushCaseRange(slot);
                    return ParseAction::Consume;
                }
                self.phase = StatementPhase::CaseColon(slot);
                ParseAction::Reprocess
            },
            | StatementPhase::CaseColon(expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(parser.cursor.following(), OperatorTokenType::Colon)
                {
                    self.own_colon_or_report(parser, token, "case label");
                    self.merge_token(parser, token.expect("semicolon exists"));
                    return ParseAction::Consume;
                }
                // A leftover run such as the `, 3` of `case 2, 3:` is skipped
                // to the real `:` instead of starting the labeled statement.
                if token.is_some()
                    && !is_operator(token, OperatorTokenType::Colon)
                    && super::expression::closer_follows_stray_run(
                        parser,
                        |token| matches!(token, TokenType::Operator(OperatorTokenType::Colon)),
                        true,
                    )
                    .is_some()
                {
                    self.own_colon_or_report(parser, token, "case label");
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::StatementExpression(
                            ExpressionTerminator::Colon,
                        ),
                        target: ParseFrameKind::Statement,
                    });
                }
                self.own_colon_or_report(parser, token, "case label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Case(expression));
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DefaultColon => {
                debug_assert!(returned.is_none());
                self.own_colon_or_report(parser, token, "default label");
                self.phase = StatementPhase::PushLabeled(LabelPrefix::Default);
                if is_operator(token, OperatorTokenType::Colon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushLabeled(prefix) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                    if !self.block_item {
                        parser.report(
                            ParserErrorType::ExpectedIsoSyntax(
                                "a statement after a label outside a compound block",
                                token.map(|x| x.kind),
                            ),
                            token,
                        );
                    }
                    parser.extension(
                        crate::configuration::Feature::C23Keywords,
                        "label at end of compound statement",
                        token.expect("closing brace exists"),
                    );
                    let child = parser.alloc_syntax(Statement {
                        kind:           StatementType::Null,
                        source_vectors: self.source_vectors.unwrap_or_default(),
                        recovered:      false,
                    });
                    let kind = match prefix {
                        | LabelPrefix::Identifier(x) => StatementType::Label(x, child),
                        | LabelPrefix::Case(x) => StatementType::Case(x, child),
                        | LabelPrefix::Default => StatementType::Default(child),
                        | LabelPrefix::Range(lower, upper) =>
                            StatementType::CaseRange(parser.alloc_syntax(CaseRange {
                                lower,
                                upper,
                                statement: child,
                            })),
                    };
                    return self.finish(parser, kind);
                }
                // C99 §6.8.1p1: `identifier :` is another label even when the
                // identifier names a typedef; labels have their own name
                // space (§6.2.3p1).
                let is_label = token.is_some_and(|x| matches!(x.kind, TokenType::Identifier))
                    && is_operator(parser.cursor.following(), OperatorTokenType::Colon);
                if !is_label
                    && token.is_some_and(|x| parser.declaration_starter(x))
                    && parser.extension_precedes_declaration()
                    && (!parser.attribute_starter(token) || parser.attributes_precede_declaration())
                {
                    if !self.block_item {
                        parser.report(
                            ParserErrorType::ExpectedIsoSyntax(
                                "a statement after a label outside a compound block",
                                token.map(|x| x.kind),
                            ),
                            token,
                        );
                    }
                    parser.extension(
                        crate::configuration::Feature::C23Keywords,
                        "label before declaration",
                        token.expect("declaration exists"),
                    );
                    self.phase = StatementPhase::AwaitLabeledDeclaration(prefix);
                    return ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.arena,
                        DeclarationContext::Block,
                        parser.hard_error_count,
                    )));
                }
                self.phase = StatementPhase::AwaitLabeled(prefix);
                ParseAction::Push(ParseFrame::Statement(
                    StatementFrame::with_else_ownership(
                        parser.hard_error_count,
                        None,
                        self.leave_else_unconsumed,
                    )
                    .with_block_item(self.block_item),
                ))
            },
            | StatementPhase::AwaitLabeled(prefix) => {
                let Some(ParseValue::Statement(statement)) = returned else {
                    panic!("labeled child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, statement);
                let kind = match prefix {
                    | LabelPrefix::Identifier(identifier) =>
                        StatementType::Label(identifier, statement),
                    | LabelPrefix::Case(expression) => StatementType::Case(expression, statement),
                    | LabelPrefix::Default => StatementType::Default(statement),
                    | LabelPrefix::Range(lower, upper) =>
                        StatementType::CaseRange(parser.alloc_syntax(CaseRange {
                            lower,
                            upper,
                            statement,
                        })),
                };
                self.finish(parser, kind)
            },
            | StatementPhase::HeaderOpening(kind) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::HeaderExpression(kind);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::HeaderExpression(kind) => {
                if matches!(kind, HeaderKind::If | HeaderKind::Switch)
                    && token.is_some_and(|x| parser.declaration_starter(x))
                    && parser.extension_precedes_declaration()
                {
                    parser.extension(
                        crate::configuration::Feature::IfSwitchDeclarations,
                        "selection declaration",
                        token.expect("declaration exists"),
                    );
                    self.phase = StatementPhase::AwaitHeaderDeclaration(kind);
                    return ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.arena,
                        DeclarationContext::SelectionHeader,
                        parser.hard_error_count,
                    )));
                }
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::HeaderClosing(kind, Self::missing_slot(parser));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitHeaderExpression(kind);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitHeaderExpression(kind) => {
                let slot = Self::parsed_slot(returned);
                self.merge_slot(parser.context, slot);
                self.phase = StatementPhase::HeaderClosing(kind, slot);
                ParseAction::Continue
            },
            | StatementPhase::HeaderClosing(kind, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            kind.name(),
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::PushHeaderBody(kind, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::PushHeaderBody(kind, expression) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitHeaderBody(kind, expression);
                let frame = if kind == HeaderKind::If {
                    StatementFrame::for_if_then(parser.hard_error_count, Some(kind.scope_kind()))
                } else {
                    StatementFrame::with_else_ownership(
                        parser.hard_error_count,
                        Some(kind.scope_kind()),
                        self.leave_else_unconsumed,
                    )
                };
                ParseAction::Push(ParseFrame::Statement(frame))
            },
            | StatementPhase::AwaitHeaderBody(kind, expression) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("selection/iteration body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, body);
                match kind {
                    | HeaderKind::If => {
                        self.phase = StatementPhase::IfAfterThen(expression, body);
                        ParseAction::Reprocess
                    },
                    | HeaderKind::Switch => self.finish(
                        parser,
                        StatementType::Switch {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                    | HeaderKind::While => self.finish(
                        parser,
                        StatementType::While {
                            condition_expression: expression,
                            body_statement:       body,
                        },
                    ),
                }
            },
            | StatementPhase::IfAfterThen(expression, then_statement) => {
                // C99 §6.8.4.1p3: `else` binds to the nearest `if`.
                debug_assert!(returned.is_none());
                if token.is_some_and(|token| {
                    matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else))
                }) {
                    self.merge_token(parser, token.expect("else token exists"));
                    self.phase = StatementPhase::PushElse(expression, then_statement);
                    ParseAction::Consume
                } else {
                    self.finish(
                        parser,
                        StatementType::If {
                            condition_expression: expression,
                            then_statement,
                            else_statement: None,
                        },
                    )
                }
            },
            | StatementPhase::PushElse(expression, then_statement) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::AwaitElse(expression, then_statement);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitSelection),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::AwaitElse(expression, then_statement) => {
                let Some(ParseValue::Statement(else_statement)) = returned else {
                    panic!("else child returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, else_statement);
                self.finish(
                    parser,
                    StatementType::If {
                        condition_expression: expression,
                        then_statement,
                        else_statement: Some(else_statement),
                    },
                )
            },
            | StatementPhase::DoPushBody => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::DoAwaitBody;
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::DoAwaitBody => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("do body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, body);
                self.phase = StatementPhase::DoWhileKeyword(body);
                ParseAction::Reprocess
            },
            | StatementPhase::DoWhileKeyword(body) => {
                debug_assert!(returned.is_none());
                if token.is_some_and(|token| {
                    matches!(token.kind, TokenType::Keyword(KeywordTokenType::While))
                }) {
                    self.merge_token(parser, token.expect("while token exists"));
                    self.phase = StatementPhase::DoOpening(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedWhileAfterDoBody(token.map(|token| token.kind)),
                        token,
                    );
                    if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                        // Only the keyword is missing; the condition follows.
                        self.phase = StatementPhase::DoOpening(body);
                        return ParseAction::Reprocess;
                    }
                    // Without `while (` the following tokens belong to the
                    // next statement, not to a condition that was never
                    // written, so finish here without consuming them.
                    let condition_expression = Self::missing_slot(parser);
                    self.finish(
                        parser,
                        StatementType::DoWhile {
                            condition_expression,
                            body_statement: body,
                        },
                    )
                }
            },
            | StatementPhase::DoOpening(body) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoExpression(body);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoExpression(body) => {
                debug_assert!(returned.is_none());
                if Self::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoClosing(body, Self::missing_slot(parser));
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::DoAwaitExpression(body);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::DoAwaitExpression(body) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser.context, expression);
                self.phase = StatementPhase::DoClosing(body, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::DoClosing(body, expression) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "do-while statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::DoSemicolon(body, expression);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::DoSemicolon(body, expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "do-while statement");
                self.phase = StatementPhase::Finish(StatementType::DoWhile {
                    condition_expression: expression,
                    body_statement:       body,
                });
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForOpening => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    self.merge_token(parser, token.expect("opening parenthesis exists"));
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedOpeningParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForInitializer;
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForInitializer => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, token, "for initializer");
                    self.phase = StatementPhase::ForCondition(None);
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token))
                    && parser.extension_precedes_declaration()
                {
                    // C99 §6.8.5p1: `for ( declaration ...`. The declared
                    // names are in scope for the rest of the header and the
                    // body (§6.8.5.3p1), the iteration statement's block. A
                    // GNU `__extension__` before an expression leaves it an
                    // expression.
                    parser.extension(
                        crate::configuration::Feature::ForDeclarations,
                        "for declaration",
                        token.expect("declaration starts here"),
                    );
                    self.phase = StatementPhase::AwaitForInitializerDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.arena,
                        DeclarationContext::ForInitializer,
                        parser.hard_error_count,
                    )))
                } else {
                    self.phase = StatementPhase::AwaitForInitializerExpression;
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForInitializerExpression => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser.context, expression);
                self.phase = StatementPhase::ForInitializerSemicolon(expression);
                ParseAction::Reprocess
            },
            | StatementPhase::AwaitForInitializerDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("for declaration returned an unexpected value: {returned:?}");
                };
                let source = declaration.source_vectors;
                parser.context.merge_into(&mut self.source_vectors, source);
                // A for-initializer declaration ends at the header's `)`
                // without reporting when its own `;` is missing; that `;`
                // belongs to the initializer, not to the condition.
                if is_operator(token, OperatorTokenType::ClosingParenthesis)
                    && !is_operator(parser.cursor.previous, OperatorTokenType::Semicolon)
                {
                    self.own_semicolon_or_report(parser, token, "for initializer");
                }
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Declaration(declaration)));
                ParseAction::Reprocess
            },
            | StatementPhase::ForInitializerSemicolon(expression) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "for initializer");
                self.phase =
                    StatementPhase::ForCondition(Some(ForInitializer::Expression(expression)));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForCondition(initializer) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.own_semicolon_or_report(parser, token, "for condition");
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = StatementPhase::ForPushBody(initializer, None, None);
                    ParseAction::Reprocess
                } else if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, token.expect("semicolon exists"));
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Consume
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.own_semicolon_or_report(parser, token, "for condition");
                    self.phase = StatementPhase::ForIteration(initializer, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForCondition(initializer);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForCondition(initializer) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser.context, expression);
                self.phase = StatementPhase::ForConditionSemicolon(initializer, expression);
                ParseAction::Reprocess
            },
            | StatementPhase::ForConditionSemicolon(initializer, condition) => {
                debug_assert!(returned.is_none());
                self.own_semicolon_or_report(parser, token, "for condition");
                self.phase = StatementPhase::ForIteration(initializer, Some(condition));
                if is_operator(token, OperatorTokenType::Semicolon) {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForIteration(initializer, condition) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, None);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else if token.is_none() {
                    self.phase = StatementPhase::ForClosing(initializer, condition, None);
                    ParseAction::Reprocess
                } else {
                    self.phase = StatementPhase::AwaitForIteration(initializer, condition);
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )))
                }
            },
            | StatementPhase::AwaitForIteration(initializer, condition) => {
                let expression = Self::parsed_slot(returned);
                self.merge_slot(parser.context, expression);
                self.phase = StatementPhase::ForClosing(initializer, condition, Some(expression));
                ParseAction::Reprocess
            },
            | StatementPhase::ForClosing(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.merge_token(parser, token.expect("closing parenthesis exists"));
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        parser.cursor.following(),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.merge_token(parser, token.expect("semicolon exists"));
                    ParseAction::Consume
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "for statement",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    // An extra `;` clause ends the iteration expression, and
                    // balanced recovery stops before every `;`, so the rest
                    // of the header is skipped by count once lookahead has
                    // found its `)`.
                    if is_operator(token, OperatorTokenType::Semicolon)
                        && let Some(distance) = Self::for_header_closer_distance(parser)
                    {
                        self.phase = StatementPhase::ForSkipHeader(
                            initializer,
                            condition,
                            iteration,
                            distance,
                        );
                        return ParseAction::Reprocess;
                    }
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    ParseAction::Reprocess
                }
            },
            | StatementPhase::ForSkipHeader(initializer, condition, iteration, remaining) => {
                debug_assert!(returned.is_none());
                let Some(token) = token else {
                    self.phase = StatementPhase::ForPushBody(initializer, condition, iteration);
                    return ParseAction::Reprocess;
                };
                self.merge_token(parser, token);
                self.phase = if remaining == 0 {
                    debug_assert!(is_operator(
                        Some(token),
                        OperatorTokenType::ClosingParenthesis
                    ));
                    StatementPhase::ForPushBody(initializer, condition, iteration)
                } else {
                    StatementPhase::ForSkipHeader(initializer, condition, iteration, remaining - 1)
                };
                ParseAction::Consume
            },
            | StatementPhase::ForPushBody(initializer, condition, iteration) => {
                debug_assert!(returned.is_none());
                self.phase = StatementPhase::ForAwaitBody(initializer, condition, iteration);
                ParseAction::Push(ParseFrame::Statement(StatementFrame::with_else_ownership(
                    parser.hard_error_count,
                    Some(ScopeKind::ImplicitIteration),
                    self.leave_else_unconsumed,
                )))
            },
            | StatementPhase::ForAwaitBody(initializer, condition, iteration) => {
                let Some(ParseValue::Statement(body)) = returned else {
                    panic!("for body returned an unexpected value: {returned:?}");
                };
                self.merge_statement(parser.context, body);
                let clauses = parser.alloc_syntax_part(ForStatement {
                    initializer,
                    condition_expression: condition,
                    iteration_expression: iteration,
                    body_statement: body,
                });
                self.finish(parser, StatementType::For(clauses))
            },
            | StatementPhase::Recovered => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::Semicolon) {
                    self.merge_token(parser, token.expect("semicolon exists"));
                    self.phase = StatementPhase::Finish(StatementType::Null);
                    ParseAction::Consume
                } else {
                    self.finish(parser, StatementType::Null)
                }
            },
            | StatementPhase::Finish(kind) => {
                debug_assert!(returned.is_none());
                self.finish(parser, kind)
            },
        }
    }

    fn enter_construct_scope(parser: &mut Parser<'_, 'tu, 'p>, kind: ScopeKind) {
        parser.scopes.enter_scope(kind);
    }

    fn at_expression_boundary(token: Option<Token>, terminator: ExpressionTerminator) -> bool {
        token.is_none_or(|token| {
            SynchronizationKind::StatementExpression(terminator).stops_before(token.kind)
        })
    }

    /// Counts the tokens from the current extra `;` of a `for` header up to
    /// the header's `)`.
    ///
    /// Extra `;`-separated clauses are admitted outside parentheses. The scan
    /// gives up at any token where skipping on could swallow the body or a
    /// following statement (a brace, a statement keyword, or a declaration
    /// starter outside parentheses), at the end of input, or after
    /// [`HEADER_RECOVERY_LOOKAHEAD`] tokens.
    fn for_header_closer_distance(parser: &mut Parser<'_, 'tu, 'p>) -> Option<u16> {
        let mut depth = 0_usize;
        let mut token = parser.cursor.current();
        for distance in 0..HEADER_RECOVERY_LOOKAHEAD {
            let current = token?;
            match current.kind {
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => depth += 1,
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis) => {
                    if depth == 0 {
                        return Some(distance);
                    }
                    depth -= 1;
                },
                | TokenType::Operator(
                    OperatorTokenType::OpeningCurlyBrace | OperatorTokenType::ClosingCurlyBrace,
                ) => return None,
                | TokenType::Operator(OperatorTokenType::Semicolon) if depth > 0 => return None,
                | kind if is_statement_keyword(kind) => return None,
                | _ if depth == 0 && parser.declaration_starter(current) => return None,
                | _ => {},
            }
            token = parser.cursor.lookahead(usize::from(distance));
        }
        None
    }

    fn missing_slot(parser: &mut Parser<'_, 'tu, 'p>) -> ExpressionSlot<'tu> {
        ExpressionSlot::Missing(parser.missing_syntax_source())
    }

    fn missing_constant_slot(parser: &mut Parser<'_, 'tu, 'p>) -> ConstantExpressionSlot<'tu> {
        ConstantExpressionSlot::Missing(parser.missing_syntax_source())
    }

    fn merge_token(&mut self, parser: &mut Parser<'_, 'tu, 'p>, token: Token) {
        parser.merge_source(&mut self.source_vectors, token);
    }

    fn merge_statement(&mut self, context: &mut Context<'_>, statement: &'tu Statement<'tu>) {
        let source = statement.source_vectors;
        self.source_vectors = Some(
            self.source_vectors
                .map_or(source, |existing| context.merge_vectors(existing, source)),
        );
    }

    fn parsed_slot(returned: Option<ParseValue<'tu>>) -> ExpressionSlot<'tu> {
        let Some(ParseValue::Expression(ExpressionResult {
            expression: index,
            recovered,
        })) = returned
        else {
            panic!("expression frame returned an unexpected value: {returned:?}");
        };
        let _ = recovered;
        ExpressionSlot::Parsed(index)
    }

    fn parsed_constant_slot(returned: Option<ParseValue<'tu>>) -> ConstantExpressionSlot<'tu> {
        let Some(ParseValue::ConstantExpression(ConstantExpressionResult {
            expression: index,
            recovered,
        })) = returned
        else {
            panic!("constant-expression frame returned an unexpected value: {returned:?}");
        };
        let _ = recovered;
        ConstantExpressionSlot::Parsed(index)
    }

    fn merge_slot(&mut self, context: &mut Context<'_>, slot: ExpressionSlot<'tu>) {
        let source = match slot {
            | ExpressionSlot::Selection(header) => header.declaration.source_vectors,
            | ExpressionSlot::Parsed(index) => index.source_vectors,
            | ExpressionSlot::Missing(source) => source,
        };
        if source.length() > 0 {
            self.source_vectors = Some(
                self.source_vectors
                    .map_or(source, |existing| context.merge_vectors(existing, source)),
            );
        }
    }

    fn merge_constant_slot(
        &mut self,
        context: &mut Context<'_>,
        slot: ConstantExpressionSlot<'tu>,
    ) {
        let source = match slot {
            | ConstantExpressionSlot::Parsed(index) => index.expression().source_vectors,
            | ConstantExpressionSlot::Missing(source) => source,
        };
        if source.length() > 0 {
            self.source_vectors = Some(
                self.source_vectors
                    .map_or(source, |existing| context.merge_vectors(existing, source)),
            );
        }
    }

    fn own_semicolon_or_report(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.merge_token(parser, token.expect("semicolon exists"));
        } else {
            parser.report(
                ParserErrorType::ExpectedSemicolonInStatement(
                    position,
                    token.map(|token| token.kind),
                ),
                token,
            );
        }
    }

    fn own_colon_or_report(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        position: &'static str,
    ) {
        if is_operator(token, OperatorTokenType::Colon) {
            self.merge_token(parser, token.expect("colon exists"));
        } else {
            parser.report(
                ParserErrorType::ExpectedColonInLabel(position, token.map(|token| token.kind)),
                token,
            );
        }
    }

    fn finish_existing(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        statement: &'tu Statement<'tu>,
    ) -> ParseAction<'tu, 'p> {
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(statement))
    }

    fn finish(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        kind: StatementType<'tu>,
    ) -> ParseAction<'tu, 'p> {
        let source_vectors = self
            .source_vectors
            .unwrap_or_else(|| parser.missing_syntax_source());
        let index = parser.alloc_syntax(Statement {
            kind,
            source_vectors,
            recovered: parser.hard_error_count > self.starting_error_count,
        });
        self.restore_scopes(parser);
        ParseAction::Reduce(ParseValue::Statement(index))
    }

    fn restore_scopes(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        if self.owns_switch_scope {
            let _switch_scope = parser
                .switch_scopes
                .pop()
                .expect("switch statement owns its switch scope");
            self.owns_switch_scope = false;
        }
        parser.scopes.restore_depth(
            self.entry_scope_depth
                .expect("statement frame recorded its entry depth"),
        );
    }
}

/// Returns whether a token begins a statement production rather than an
/// expression that the expression frame can consume.
///
/// C99: the keywords that open `labeled-statement`, `selection-statement`,
/// `iteration-statement`, and `jump-statement` in §6.8.1-§6.8.6,
/// pp. 131-136; PDF pp. 143-148, plus `else` from §6.8.4, p. 133;
/// PDF p. 145.
pub(super) fn is_statement_keyword(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Keyword(
            KeywordTokenType::Break
                | KeywordTokenType::Case
                | KeywordTokenType::Continue
                | KeywordTokenType::Default
                | KeywordTokenType::Do
                | KeywordTokenType::Else
                | KeywordTokenType::For
                | KeywordTokenType::Goto
                | KeywordTokenType::If
                | KeywordTokenType::Return
                | KeywordTokenType::Switch
                | KeywordTokenType::While
        )
    )
}
