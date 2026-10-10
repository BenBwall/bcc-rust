//! Expression frame and its Double-E precedence reducer.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of every
//! expression production. C99: §6.5, pp. 67-94; PDF pp. 79-106; §A.2.1,
//! pp. 409-411; PDF pp. 421-423:
//!
//! - `primary-expression`: §6.5.1, p. 69; PDF p. 81.
//! - `postfix-expression` and `argument-expression-list`: §6.5.2, pp. 69-70;
//!   PDF pp. 81-82, including compound literals (§6.5.2.5, p. 75; PDF p. 87).
//! - `unary-expression`, including `sizeof`: §6.5.3, p. 78; PDF p. 90.
//! - `cast-expression`: §6.5.4, p. 81; PDF p. 93.
//! - The binary levels §6.5.5-§6.5.14, pp. 82-89; PDF pp. 94-101, reduced by
//!   precedence (see `expression_operators`).
//! - `conditional-expression`: §6.5.15, p. 90; PDF p. 102.
//! - `assignment-expression`: §6.5.16, p. 91; PDF p. 103.
//! - `expression` (comma): §6.5.17, p. 94; PDF p. 106.
//! - `constant-expression`: §6.6 paragraph 1, p. 95; PDF p. 107.
//!
//! Whether `(` opens a `type-name` or a parenthesized expression depends on
//! whether the next identifier is a visible `typedef-name` (§6.7.7
//! paragraph 1, p. 123; PDF p. 135). Parenthesized subexpressions nest
//! through frames, so the 63-level minimum of §5.2.4.1, p. 20; PDF p. 32
//! imposes no fixed ceiling.
//!
//! Only syntax is checked. Every expression constraint and semantic rule
//! belongs to semantic analysis: declared identifiers (§6.5.1 paragraph 2
//! and footnote 79, p. 69; PDF p. 81), operand types in the constraints of
//! §6.5.2.1-§6.5.16.2, modifiable-lvalue assignment operands (§6.5.16
//! paragraph 2, p. 91; PDF p. 103), and the constant-expression constraints
//! and kinds of §6.6 paragraphs 3-10, pp. 95-96; PDF pp. 107-108.

use std::fmt::Debug;

mod lookahead;

pub(super) use lookahead::closer_follows_stray_run;
use lookahead::{
    brace_group_closes_before_parenthesis,
    brace_group_continues_expression,
    brace_group_is_block,
};

use super::{
    Parser,
    declaration_syntax::TypeName,
    errors::ParserErrorType,
    expression_operators::{
        LanguageExpressionOperator,
        binary_operator,
        is_assignment_operator,
        is_expression_operand_starter,
        is_postfix_starter,
        prefix_operator,
    },
    frame_pool::{
        FramePools,
        PoolBox,
    },
    initializer::InitializerFrame,
    machine::{
        ConstantExpressionResult,
        ExpressionResult,
        InitializerResult,
        ParseAction,
        ParseFrame,
        ParseValue,
        any_expression_value,
        expression_value,
    },
    modern::{
        ModernFrame,
        ModernKind,
        ModernValue,
        SyntaxOperand,
    },
    recovery::ExpressionTerminator,
    statement::is_statement_keyword,
    syntax::{
        BinaryOperator,
        ConditionalExpression,
        Constant,
        ConstantExpression,
        Expression,
        ExpressionType,
        Identifier,
        UnaryOperator,
    },
    type_name::TypeNameFrame,
};
use crate::{
    translation_phases::{
        GetPosition,
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

/// The expression nonterminal a frame parses, which bounds the operators it
/// may absorb.
///
/// C99: `Expression` is `expression` (§6.5.17 paragraph 1, p. 94;
/// PDF p. 106); `AssignmentExpression` is `assignment-expression` (§6.5.16
/// paragraph 1, p. 91; PDF p. 103); `ConstantExpression` is
/// `constant-expression`, syntactically a `conditional-expression` (§6.6
/// paragraph 1, p. 95; PDF p. 107); `CastExpression` is `cast-expression`
/// (§6.5.4 paragraph 1, p. 81; PDF p. 93); `UnaryExpression` is
/// `unary-expression` (§6.5.3 paragraph 1, p. 78; PDF p. 90).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpressionMode {
    Expression,
    AssignmentExpression,
    ConstantExpression,
    CastExpression,
    UnaryExpression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpressionParserState {
    Operand,
    Operator,
}

/// The enclosing production that owns the token ending this expression.
///
/// C99: `Statement` covers the expressions of §6.8.1-§6.8.6, pp. 131-136;
/// PDF pp. 143-148; `Argument` is `argument-expression-list` (§6.5.2,
/// p. 70; PDF p. 82); `Initializer` and `Designator` are `initializer` and
/// `designator` (§6.7.8 paragraph 1, p. 125; PDF p. 137); `ArrayBound` is an
/// array declarator's size (§6.7.5 paragraph 1, p. 114; PDF p. 126);
/// `StructMember` is a bit-field width (§6.7.2.1 paragraph 1, p. 101;
/// PDF p. 113); `Enumerator` is an enumerator value (§6.7.2.2 paragraph 1,
/// p. 105; PDF p. 117).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpressionBoundary {
    Statement(ExpressionTerminator),
    ClosingParenthesis,
    ClosingSquareBracket,
    Argument,
    Initializer,
    ArrayBound,
    StructMember,
    Enumerator,
    Designator,
}

/// One parsed operand and the grammar categories it still belongs to: a
/// `unary-expression` may be an assignment's left operand (§6.5.16
/// paragraph 1, p. 91; PDF p. 103), and only a `postfix-expression` takes a
/// postfix suffix (§6.5.2 paragraph 1, p. 69; PDF p. 81).
#[derive(Debug, Clone, Copy)]
pub(super) struct ExpressionOperand<'tu> {
    expression:         &'tu Expression<'tu>,
    unary_expression:   bool,
    postfix_expression: bool,
}

/// Resumable expression production: an operand/operator stack pair
/// reduced by precedence, with child frames for delimited operands.
///
/// C99: §6.5, pp. 67-94; PDF pp. 79-106; precedence and associativity
/// follow §6.5 paragraph 3 and footnote 74, p. 67; PDF p. 79.
#[derive(Debug)]
pub(super) struct ExpressionFrame<'tu, 'p> {
    mode:                  ExpressionMode,
    boundary:              ExpressionBoundary,
    recovery_boundary:     ExpressionBoundary,
    phase:                 ExpressionPhase<'tu>,
    operators:             ArenaVec<'p, LanguageExpressionOperator<'tu>>,
    operands:              ArenaVec<'p, ExpressionOperand<'tu>>,
    state:                 ExpressionParserState,
    starting_error_count:  usize,
    /// Postfix call under construction, boxed because most expression frames
    /// never parse a call. The box comes from and returns to the frame pools.
    pub(super) call:       Option<PoolBox<'p, CallState<'tu, 'p>>>,
    pending_sizeof_prefix: Option<SourceVectors>,
    /// The top operand is an error operand that replaced a stray token
    /// already diagnosed, so the tokens after it need no second report.
    stray_error_operand:   bool,
}

/// Arguments and provenance of the postfix call an expression frame is
/// building.
///
/// C99: `postfix-expression ( argument-expression-list(opt) )`, §6.5.2,
/// pp. 69-70; PDF pp. 81-82; function calls §6.5.2.2, p. 71; PDF p. 83.
#[derive(Debug)]
pub(super) struct CallState<'tu, 'p> {
    pub(super) arguments: ArenaVec<'p, &'tu Expression<'tu>>,
    source_vectors:       ArenaVec<'p, SourceVectors>,
    operator_sources:     ArenaVec<'p, SourceVectors>,
}

impl<'p> CallState<'_, 'p> {
    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            arguments:        ArenaVec::new_in(arena),
            source_vectors:   ArenaVec::new_in(arena),
            operator_sources: ArenaVec::new_in(arena),
        }
    }
}

/// Resumable positions inside one expression. `*Grouped` is
/// `( expression )` (§6.5.1); `*Subscript`, `Call*`, and `ExpectMember` are
/// postfix suffixes (§6.5.2); `*Prefix` and `Sizeof*` are unary operators
/// (§6.5.3); `*TypeName`, `*CompoundLiteral`, and `*CastOperand` follow a
/// parenthesized `type-name` (§6.5.2.5, §6.5.3, §6.5.4); `*Conditional*`
/// is `? :` (§6.5.15).
#[derive(Debug, Clone, Copy)]
enum ExpressionPhase<'tu> {
    Parse,
    AwaitModern(KeywordTokenType, SourceVectors),
    AwaitGnu,
    LabelAddress(SourceVectors),
    AwaitStatementExpression(SourceVectors),
    CloseStatementExpression(SourceVectors, &'tu super::syntax::Statement<'tu>),
    CountofStart(SourceVectors),
    AwaitCountofExpression(SourceVectors),
    /// GNU extension: like `sizeof`, C99 §6.5.3p1, p. 78; PDF p. 90.
    AlignofStart(SourceVectors),
    AwaitAlignofExpression(SourceVectors),
    Finish,
    RecoverUnexpectedBrace(u32, SourceVectors),
    PushGrouped(SourceVectors),
    AwaitGrouped(SourceVectors),
    CloseGrouped(SourceVectors, &'tu Expression<'tu>),
    PushSubscript(&'tu Expression<'tu>, SourceVectors),
    AwaitSubscript(&'tu Expression<'tu>, SourceVectors),
    CloseSubscript(&'tu Expression<'tu>, SourceVectors, &'tu Expression<'tu>),
    CallStart(&'tu Expression<'tu>, SourceVectors),
    PushCallArgument(&'tu Expression<'tu>),
    AwaitCallArgument(&'tu Expression<'tu>),
    CallSeparator(&'tu Expression<'tu>),
    ExpectMember(&'tu Expression<'tu>, bool, SourceVectors),
    PushPrefix(UnaryOperator, SourceVectors, ExpressionMode),
    AwaitPrefix(UnaryOperator, SourceVectors),
    SizeofStart(SourceVectors),
    PushSizeofExpression(SourceVectors),
    AwaitSizeofExpression(SourceVectors),
    PushTypeName(SourceVectors, TypeNameUse),
    AwaitTypeName(SourceVectors, TypeNameUse),
    CloseTypeName(SourceVectors, TypeNameUse, &'tu TypeName<'tu>),
    PushCompoundLiteral(&'tu TypeName<'tu>, SourceVectors, TypeNameUse),
    AwaitCompoundLiteral(&'tu TypeName<'tu>, SourceVectors, TypeNameUse),
    PushCastOperand(&'tu TypeName<'tu>, SourceVectors),
    AwaitCastOperand(&'tu TypeName<'tu>, SourceVectors),
    PushConditionalMiddle,
    AwaitConditionalMiddle,
    ExpectConditionalColon(&'tu Expression<'tu>),
    PushConditionalElse,
    AwaitConditionalElse,
    /// Discards this many already-diagnosed tokens before the closer that
    /// ends a malformed delimited expression, then finishes.
    SkipStray(usize),
}

/// What a parenthesized `type-name` in operand position introduces.
///
/// C99: `Cast` is `( type-name ) cast-expression` (§6.5.4 paragraph 1,
/// p. 81; PDF p. 93) or a compound literal; `Sizeof` is
/// `sizeof ( type-name )` (§6.5.3 paragraph 1, p. 78; PDF p. 90) or
/// `sizeof` applied to a compound literal; `UnaryCompoundLiteral` is the
/// `unary-expression` operand of `++`, `--`, or `sizeof`, which cannot be a
/// cast and so must be a compound literal (§6.5.2 paragraph 1, p. 69;
/// PDF p. 81).
#[derive(Debug, Clone, Copy)]
enum TypeNameUse {
    Cast,
    Sizeof(SourceVectors),
    UnaryCompoundLiteral,
}

impl<'tu, 'p> ExpressionFrame<'tu, 'p> {
    pub(super) fn new(
        arena: &'p Bump,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self::with_recovery_boundary(arena, mode, boundary, boundary, starting_error_count)
    }

    fn nested(
        &self,
        arena: &'p Bump,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self::with_recovery_boundary(
            arena,
            mode,
            boundary,
            self.recovery_boundary,
            starting_error_count,
        )
    }

    pub(super) fn with_recovery_boundary(
        arena: &'p Bump,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        recovery_boundary: ExpressionBoundary,
        starting_error_count: usize,
    ) -> Self {
        Self {
            mode,
            boundary,
            recovery_boundary,
            phase: ExpressionPhase::Parse,
            operators: ArenaVec::new_in(arena),
            operands: ArenaVec::new_in(arena),
            state: ExpressionParserState::Operand,
            starting_error_count,
            call: None,
            pending_sizeof_prefix: None,
            stray_error_operand: false,
        }
    }

    /// Gives the operator and operand stacks spare allocations.
    pub(super) fn lend_pooled(&mut self, pools: &mut FramePools<'tu, 'p>) {
        pools.operators.lend(&mut self.operators);
        pools.operands.lend(&mut self.operands);
    }

    /// Returns the operator and operand stacks' allocations to the pools.
    pub(super) fn reclaim_pooled(&mut self, pools: &mut FramePools<'tu, 'p>) {
        pools.operators.reclaim(&mut self.operators);
        pools.operands.reclaim(&mut self.operands);
        if let Some(mut call) = self.call.take() {
            call.arguments.clear();
            call.source_vectors.clear();
            call.operator_sources.clear();
            pools.calls.reclaim(call);
        }
    }

    fn closing_parenthesis_is_boundary(&self) -> bool {
        [self.boundary, self.recovery_boundary]
            .into_iter()
            .any(|boundary| {
                matches!(
                    boundary,
                    ExpressionBoundary::ClosingParenthesis
                        | ExpressionBoundary::Argument
                        | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
                )
            })
    }

    fn closing_square_bracket_is_boundary(&self) -> bool {
        [self.boundary, self.recovery_boundary]
            .into_iter()
            .any(|boundary| {
                matches!(
                    boundary,
                    ExpressionBoundary::ClosingSquareBracket
                        | ExpressionBoundary::ArrayBound
                        | ExpressionBoundary::Designator
                )
            })
    }

    #[expect(
        clippy::missing_assert_message,
        clippy::too_many_lines,
        reason = "The explicit expression frame keeps the C precedence grammar and transition \
                  invariants in one non-recursive state machine."
    )]
    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | ExpressionPhase::AwaitGnu => {
                let Some(ParseValue::Gnu(super::gnu::GnuValue::Builtin(builtin))) = returned else {
                    panic!("builtin child protocol");
                };
                let expression = parser.store_expression(
                    ExpressionType::Builtin(builtin),
                    builtin.source_vectors,
                    None,
                    builtin.recovered,
                );
                self.push_operand(expression, true, true);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Continue;
            },
            | ExpressionPhase::LabelAddress(source) => {
                self.phase = ExpressionPhase::Parse;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    let merged = parser.context.merge_vectors(source, token.source_vectors);
                    let expression = parser.store_expression(
                        ExpressionType::LabelAddress(Identifier::from_token(token)),
                        merged,
                        Some(source),
                        false,
                    );
                    self.push_operand(expression, true, false);
                    return ParseAction::Consume;
                }
                parser.report(
                    ParserErrorType::ExpectedGnuSyntax(
                        "label identifier after `&&`",
                        token.map(|x| x.kind),
                    ),
                    token,
                );
                self.push_error(parser, token);
                return ParseAction::Continue;
            },
            | ExpressionPhase::AwaitStatementExpression(source) => {
                if returned.is_none() {
                    return ParseAction::Push(ParseFrame::CompoundStatement(
                        super::compound_statement::CompoundStatementFrame::new(
                            parser.arena,
                            parser.hard_error_count,
                            false,
                        ),
                    ));
                }
                let Some(ParseValue::CompoundStatement(statement)) = returned else {
                    panic!("statement expression child protocol");
                };
                self.phase = ExpressionPhase::CloseStatementExpression(source, statement);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseStatementExpression(source, statement) => {
                let mut merged = parser
                    .context
                    .merge_vectors(source, statement.source_vectors);
                let consume = token.is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    )
                });
                if consume {
                    merged = parser
                        .context
                        .merge_vectors(merged, token.expect("closer exists").source_vectors);
                } else {
                    parser.report(
                        ParserErrorType::ExpectedGnuSyntax(
                            "`)` after statement expression",
                            token.map(|x| x.kind),
                        ),
                        token,
                    );
                }
                let expression = parser.store_expression(
                    ExpressionType::StatementExpression(statement),
                    merged,
                    Some(source),
                    statement.recovered || !consume,
                );
                self.push_operand(expression, true, true);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::CountofStart(source) => {
                if token.is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                }) && Self::parenthesized_type_name_follows(parser)
                    // C2y: a compound literal is a unary-expression operand.
                    && !Self::parenthesized_compound_literal_follows(parser)
                {
                    self.phase = ExpressionPhase::AwaitModern(KeywordTokenType::Countof, source);
                    let frame = ModernFrame::operand_after_keyword(
                        parser.arena,
                        parser.hard_error_count,
                        source,
                    );
                    return ParseAction::Push(ParseFrame::Modern(parser.pools.modern(frame)));
                }
                self.phase = ExpressionPhase::AwaitCountofExpression(source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::UnaryExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitCountofExpression(source) => {
                let operand = expression_value(returned);
                let merged = parser.context.merge_vectors(source, operand.source_vectors);
                let index = parser.store_expression(
                    ExpressionType::Countof(SyntaxOperand::Expression(operand)),
                    merged,
                    Some(source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Continue;
            },
            | ExpressionPhase::AlignofStart(source) => {
                // GNU extension to C99 §6.5.3p1: a parenthesized type name
                // or a unary-expression, including a compound literal.
                if token.is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                }) && Self::parenthesized_type_name_follows(parser)
                    && !Self::parenthesized_compound_literal_follows(parser)
                {
                    self.phase = ExpressionPhase::AwaitModern(KeywordTokenType::Alignof, source);
                    let frame = ModernFrame::operand_after_keyword(
                        parser.arena,
                        parser.hard_error_count,
                        source,
                    );
                    return ParseAction::Push(ParseFrame::Modern(parser.pools.modern(frame)));
                }
                self.phase = ExpressionPhase::AwaitAlignofExpression(source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::UnaryExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitAlignofExpression(source) => {
                let operand = expression_value(returned);
                let merged = parser.context.merge_vectors(source, operand.source_vectors);
                let expression = parser.store_expression(
                    ExpressionType::AlignofExpr(operand),
                    merged,
                    Some(source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(expression, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::AwaitModern(keyword, keyword_source) => {
                let (kind, source) = match returned {
                    | Some(ParseValue::Modern(ModernValue::Generic(x))) =>
                        (ExpressionType::Generic(x), x.source_vectors),
                    | Some(ParseValue::Modern(ModernValue::Operand(operand, source))) => (
                        match (keyword, operand) {
                            | (KeywordTokenType::Alignof, SyntaxOperand::Type(x)) =>
                                ExpressionType::AlignofType(x),
                            | (KeywordTokenType::Alignof, SyntaxOperand::Expression(x)) =>
                                ExpressionType::AlignofExpr(x),
                            | (_, operand) => ExpressionType::Countof(operand),
                        },
                        source,
                    ),
                    | _ => panic!("ISO expression protocol: {returned:?}"),
                };
                let index = parser.store_expression(
                    kind,
                    source,
                    Some(keyword_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, keyword == KeywordTokenType::Generic);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Continue;
            },
            | ExpressionPhase::RecoverUnexpectedBrace(depth, source_vectors) => {
                debug_assert!(returned.is_none());
                let Some(token) = token else {
                    self.push_error_with_source(parser, source_vectors, None);
                    self.phase = ExpressionPhase::Parse;
                    return self.finish(parser);
                };
                let source_vectors = parser
                    .context
                    .merge_vectors(source_vectors, token.source_vectors);
                let depth = match token.kind {
                    | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => depth + 1,
                    | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => depth - 1,
                    | _ => depth,
                };
                if depth == 0 {
                    self.push_error_with_source(parser, source_vectors, None);
                    self.phase = ExpressionPhase::Parse;
                } else {
                    self.phase = ExpressionPhase::RecoverUnexpectedBrace(depth, source_vectors);
                }
                return ParseAction::Consume;
            },
            | ExpressionPhase::PushGrouped(opening) => {
                // C99 §6.5.1p1: `( expression )`.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitGrouped(opening);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingParenthesis,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitGrouped(opening) => {
                let child = expression_value(returned);
                self.phase = ExpressionPhase::CloseGrouped(opening, child);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseGrouped(opening, child) => {
                debug_assert!(returned.is_none());
                let mut operator_source = opening;
                let mut source = parser.context.merge_vectors(opening, child.source_vectors);
                let consume = if let Some(close) = token
                    && matches!(
                        close.kind,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    ) {
                    source = parser.context.merge_vectors(source, close.source_vectors);
                    operator_source = parser
                        .context
                        .merge_vectors(operator_source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "grouped expression",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                let index = parser.store_expression(
                    ExpressionType::Parenthesized { expression: child },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::PushSubscript(base, opening) => {
                // C99 §6.5.2p1: `postfix-expression [ expression ]`.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitSubscript(base, opening);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingSquareBracket,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitSubscript(base, opening) => {
                let child = expression_value(returned);
                self.phase = ExpressionPhase::CloseSubscript(base, opening, child);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseSubscript(base, opening, child) => {
                debug_assert!(returned.is_none());
                let mut operator_source = opening;
                let mut source = parser.context.merge_vectors(base.source_vectors, opening);
                source = parser.context.merge_vectors(source, child.source_vectors);
                let consume = if let Some(close) = token
                    && matches!(
                        close.kind,
                        TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                    ) {
                    source = parser.context.merge_vectors(source, close.source_vectors);
                    operator_source = parser
                        .context
                        .merge_vectors(operator_source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingSquareBracketInSubscript(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                let index = parser.store_expression(
                    ExpressionType::Binary {
                        operator:         BinaryOperator::Subscript,
                        left_expression:  base,
                        right_expression: child,
                    },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::CallStart(base, opening) => {
                debug_assert!(returned.is_none());
                let base_source = base.source_vectors;
                let call = self.call_state(&mut parser.pools);
                call.source_vectors.clear();
                call.source_vectors.extend([base_source, opening]);
                call.operator_sources.clear();
                call.operator_sources.push(opening);
                if let Some(close) = token
                    && matches!(
                        close.kind,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    )
                {
                    self.merge_call_source(&mut parser.pools, close.source_vectors);
                    self.merge_call_operator_source(&mut parser.pools, close.source_vectors);
                    self.finish_call(parser, base);
                    self.phase = ExpressionPhase::Parse;
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushCallArgument(base);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushCallArgument(base) => {
                // C99 §6.5.2p1: each argument is an `assignment-expression`,
                // so a top-level comma separates arguments.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitCallArgument(base);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::Argument,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitCallArgument(base) => {
                let argument = expression_value(returned);
                self.call_state(&mut parser.pools).arguments.push(argument);
                self.merge_call_source(&mut parser.pools, argument.source_vectors);
                self.phase = ExpressionPhase::CallSeparator(base);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CallSeparator(base) => {
                debug_assert!(returned.is_none());
                if let Some(separator) = token
                    && matches!(
                        separator.kind,
                        TokenType::Operator(OperatorTokenType::Comma)
                    )
                {
                    self.merge_call_source(&mut parser.pools, separator.source_vectors);
                    self.merge_call_operator_source(&mut parser.pools, separator.source_vectors);
                    self.phase = ExpressionPhase::PushCallArgument(base);
                    return ParseAction::Consume;
                }
                // A child may return without consuming an enclosing
                // grammar boundary (for example
                // a label after a malformed initializer).
                // Avoid retrying the boundary as another call argument.
                if token.is_some_and(|token| {
                    is_expression_operand_starter(token.kind)
                        && !self.is_strong_grammar_boundary(parser, token)
                }) {
                    parser.report(
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionCall(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ExpressionPhase::PushCallArgument(base);
                    return ParseAction::Reprocess;
                }
                let consume = if let Some(close) = token
                    && matches!(
                        close.kind,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    ) {
                    self.merge_call_source(&mut parser.pools, close.source_vectors);
                    self.merge_call_operator_source(&mut parser.pools, close.source_vectors);
                    true
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "function call",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                self.finish_call(parser, base);
                self.phase = ExpressionPhase::Parse;
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::ExpectMember(base, indirect, operator_source) => {
                // C99 §6.5.2p1: `.` and `->` take an `identifier`.
                debug_assert!(returned.is_none());
                let Some(member_token) =
                    token.filter(|token| matches!(token.kind, TokenType::Identifier))
                else {
                    parser.report(
                        ParserErrorType::ExpectedMemberIdentifier(token.map(|token| token.kind)),
                        token,
                    );
                    let source = parser
                        .context
                        .merge_vectors(base.source_vectors, operator_source);
                    self.push_error_with_source(parser, source, Some(operator_source));
                    self.phase = ExpressionPhase::Parse;
                    return ParseAction::Reprocess;
                };
                let source = parser
                    .context
                    .merge_vectors(base.source_vectors, operator_source);
                let source = parser
                    .context
                    .merge_vectors(source, member_token.source_vectors);
                let kind = if indirect {
                    ExpressionType::IndirectMember {
                        base_expression: base,
                        member:          Identifier::from_token(member_token),
                    }
                } else {
                    ExpressionType::DirectMember {
                        base_expression: base,
                        member:          Identifier::from_token(member_token),
                    }
                };
                let index = parser.store_expression(
                    kind,
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, true);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Consume;
            },
            | ExpressionPhase::PushPrefix(operator, operator_source, child_mode) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitPrefix(operator, operator_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    child_mode,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitPrefix(operator, operator_source) => {
                if operator == UnaryOperator::Extension {
                    parser.pedantic_suppression -= 1;
                }
                let operand = expression_value(returned);
                let source = parser
                    .context
                    .merge_vectors(operator_source, operand.source_vectors);
                let index = parser.store_expression(
                    ExpressionType::Unary {
                        operator,
                        operand_expression: operand,
                    },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::SizeofStart(sizeof_source) => {
                // C99 §6.5.3p1: `sizeof ( type-name )` when a type name
                // follows `(`, otherwise `sizeof unary-expression`.
                debug_assert!(returned.is_none());
                if let Some(opening) = token
                    && matches!(
                        opening.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                    && Self::parenthesized_type_name_follows(parser)
                {
                    self.phase = ExpressionPhase::PushTypeName(
                        opening.source_vectors,
                        TypeNameUse::Sizeof(sizeof_source),
                    );
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushSizeofExpression(sizeof_source);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushSizeofExpression(sizeof_source) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitSizeofExpression(sizeof_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::UnaryExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitSizeofExpression(sizeof_source) => {
                let operand = expression_value(returned);
                let source = parser
                    .context
                    .merge_vectors(sizeof_source, operand.source_vectors);
                let index = parser.store_expression(
                    ExpressionType::SizeofExpr(operand),
                    source,
                    Some(sizeof_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, true, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushTypeName(opening_source, use_kind) => {
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitTypeName(opening_source, use_kind);
                let mut frame = TypeNameFrame::new(parser.hard_error_count);
                if Self::type_name_is_compound_literal(parser) {
                    frame = frame.with_storage();
                }
                return ParseAction::Push(ParseFrame::TypeName(frame));
            },
            | ExpressionPhase::AwaitTypeName(opening_source, use_kind) => {
                let Some(ParseValue::TypeName(type_name)) = returned else {
                    panic!("type-name frame returned an unexpected value: {returned:?}");
                };
                self.phase = ExpressionPhase::CloseTypeName(opening_source, use_kind, type_name);
                return ParseAction::Continue;
            },
            | ExpressionPhase::CloseTypeName(opening_source, use_kind, type_name) => {
                debug_assert!(returned.is_none());
                let mut source = parser
                    .context
                    .merge_vectors(opening_source, type_name.source_vectors);
                let consume = if let Some(close) = token
                    && matches!(
                        close.kind,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    ) {
                    source = parser.context.merge_vectors(source, close.source_vectors);
                    true
                } else {
                    parser.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "type name",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    false
                };
                // C99 §6.5.2p1: `( type-name ) {` begins a compound literal.
                let starts_compound_literal = consume
                    && parser.cursor.following().is_some_and(|following| {
                        matches!(
                            following.kind,
                            TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                        )
                    })
                    && !parser.cursor.lookahead(1).is_some_and(|first| {
                        !matches!(first.kind, TokenType::Identifier)
                            && (parser.declaration_starter(first)
                                || is_statement_keyword(first.kind))
                    });
                if starts_compound_literal {
                    self.phase = ExpressionPhase::PushCompoundLiteral(type_name, source, use_kind);
                } else {
                    match use_kind {
                        | TypeNameUse::Sizeof(sizeof_source) => {
                            let source = parser.context.merge_vectors(sizeof_source, source);
                            let index = parser.store_expression(
                                ExpressionType::SizeofType(type_name),
                                source,
                                Some(sizeof_source),
                                parser.hard_error_count > self.starting_error_count,
                            );
                            self.push_operand(index, true, false);
                            self.phase = ExpressionPhase::Parse;
                        },
                        | TypeNameUse::Cast => {
                            self.phase = ExpressionPhase::PushCastOperand(type_name, source);
                        },
                        | TypeNameUse::UnaryCompoundLiteral if consume => {
                            // A unary operator's operand cannot be a cast, so
                            // the `{` of a compound literal is missing after
                            // the `)`. Report it there and keep the operand as
                            // a recovered cast.
                            let following = parser.cursor.following();
                            parser.report(
                                ParserErrorType::ExpectedStatementExpression(
                                    "compound literal initializer",
                                    following.map(|following| following.kind),
                                ),
                                following,
                            );
                            if following.is_some_and(|following| {
                                is_expression_operand_starter(following.kind)
                            }) {
                                self.phase = ExpressionPhase::PushCastOperand(type_name, source);
                            } else {
                                self.push_error(parser, following);
                                self.phase = ExpressionPhase::Parse;
                            }
                        },
                        | TypeNameUse::UnaryCompoundLiteral => {
                            self.push_error(parser, token);
                            self.phase = ExpressionPhase::Parse;
                        },
                    }
                }
                return if consume {
                    ParseAction::Consume
                } else {
                    ParseAction::Reprocess
                };
            },
            | ExpressionPhase::PushCompoundLiteral(type_name, type_source, use_kind) => {
                debug_assert!(returned.is_none());
                if let Some(token) = token {
                    parser.extension(
                        crate::configuration::Feature::CompoundLiterals,
                        "compound literal",
                        token,
                    );
                }
                self.phase =
                    ExpressionPhase::AwaitCompoundLiteral(type_name, type_source, use_kind);
                return ParseAction::Push(ParseFrame::Initializer(InitializerFrame::new(
                    parser.arena,
                    parser.hard_error_count,
                    self.closing_parenthesis_is_boundary(),
                    self.closing_square_bracket_is_boundary(),
                )));
            },
            | ExpressionPhase::AwaitCompoundLiteral(type_name, type_source, use_kind) => {
                let Some(ParseValue::Initializer(InitializerResult { initializer, .. })) = returned
                else {
                    panic!(
                        "compound-literal initializer returned an unexpected value: {returned:?}"
                    );
                };
                let source = parser
                    .context
                    .merge_vectors(type_source, initializer.source_vectors);
                let compound = parser.store_expression(
                    ExpressionType::CompoundLiteral {
                        type_name,
                        initializer,
                    },
                    source,
                    Some(type_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                if let TypeNameUse::Sizeof(sizeof_source) = use_kind {
                    self.pending_sizeof_prefix = Some(sizeof_source);
                }
                self.push_operand(compound, true, true);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushCastOperand(type_name, type_source) => {
                if matches!(type_name.declaration_specifiers.type_specifiers, super::declaration_syntax::TypeSpecifiers::StructOrUnion(x) if x.struct_or_union == super::declaration_syntax::StructOrUnion::Union)
                {
                    parser.extension_source(
                        crate::configuration::Feature::UnionCasts,
                        "cast to union",
                        type_name.source_vectors,
                    );
                }
                // C99 §6.5.4p1: `( type-name ) cast-expression`.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitCastOperand(type_name, type_source);
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::CastExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitCastOperand(type_name, type_source) => {
                let operand = expression_value(returned);
                let source = parser
                    .context
                    .merge_vectors(type_source, operand.source_vectors);
                let index = parser.store_expression(
                    ExpressionType::Cast {
                        target_type:        type_name,
                        operand_expression: operand,
                    },
                    source,
                    Some(type_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushConditionalMiddle => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Colon))
                {
                    parser.extension(
                        crate::configuration::Feature::OmittedConditionalOperand,
                        "omitted conditional operand",
                        token,
                    );
                    self.complete_conditional_marker(None, Some(token.source_vectors));
                    return ParseAction::Consume;
                }
                // C99 §6.5.15p1: the middle operand is a full `expression`.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitConditionalMiddle;
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitConditionalMiddle => {
                let middle = expression_value(returned);
                self.phase = ExpressionPhase::ExpectConditionalColon(middle);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::ExpectConditionalColon(middle) => {
                debug_assert!(returned.is_none());
                if let Some(colon) = token
                    && matches!(colon.kind, TokenType::Operator(OperatorTokenType::Colon))
                {
                    self.complete_conditional_marker(Some(middle), Some(colon.source_vectors));
                    return ParseAction::Consume;
                }
                parser.report(
                    ParserErrorType::ExpectedColonInLabel(
                        "conditional expression",
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                self.complete_conditional_marker(Some(middle), None);
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::PushConditionalElse => {
                // C99 §6.5.15p1: the last operand is a
                // `conditional-expression`, parsed in constant-expression
                // mode (§6.6p1), so assignment and comma end it.
                debug_assert!(returned.is_none());
                self.phase = ExpressionPhase::AwaitConditionalElse;
                return ParseAction::Push(ParseFrame::Expression(self.nested(
                    parser.arena,
                    ExpressionMode::ConstantExpression,
                    self.boundary,
                    parser.hard_error_count,
                )));
            },
            | ExpressionPhase::AwaitConditionalElse => {
                let final_expression = any_expression_value(returned);
                let Some(LanguageExpressionOperator::Conditional {
                    middle,
                    question_source,
                    colon_source,
                }) = self.operators.pop()
                else {
                    panic!("conditional final operand must retain its completed marker");
                };
                let condition = self.pop_operand().expression;
                let omitted = middle.is_none();
                let middle = middle.unwrap_or(condition);
                let source = parser
                    .context
                    .merge_vectors(condition.source_vectors, question_source);
                let source = if omitted {
                    source
                } else {
                    parser.context.merge_vectors(source, middle.source_vectors)
                };
                let source = colon_source
                    .map_or(source, |colon| parser.context.merge_vectors(source, colon));
                let source = parser
                    .context
                    .merge_vectors(source, final_expression.source_vectors);
                let operands = parser.alloc_syntax_part(ConditionalExpression {
                    condition_expression: condition,
                    then_expression:      middle,
                    else_expression:      final_expression,
                });
                let operator_source = colon_source.map_or(question_source, |colon| {
                    parser.context.merge_vectors(question_source, colon)
                });
                let index = parser.store_expression(
                    if omitted {
                        ExpressionType::OmittedConditional(operands)
                    } else {
                        ExpressionType::Conditional(operands)
                    },
                    source,
                    Some(operator_source),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
                self.phase = ExpressionPhase::Parse;
                return ParseAction::Reprocess;
            },
            | ExpressionPhase::Finish => return self.finish(parser),
            | ExpressionPhase::SkipStray(remaining) => {
                debug_assert!(returned.is_none());
                if remaining == 0 || token.is_none() {
                    self.phase = ExpressionPhase::Parse;
                    return self.finish(parser);
                }
                self.phase = ExpressionPhase::SkipStray(remaining - 1);
                return ParseAction::Consume;
            },
            | ExpressionPhase::Parse => {
                debug_assert!(returned.is_none());
            },
        }

        if self.state == ExpressionParserState::Operator
            && self.pending_sizeof_prefix.is_some()
            && !token.is_some_and(|token| is_postfix_starter(token.kind))
        {
            let operand = self.pop_operand().expression;
            let sizeof_source = self
                .pending_sizeof_prefix
                .take()
                .expect("pending sizeof operand has an operator source");
            let source = parser
                .context
                .merge_vectors(sizeof_source, operand.source_vectors);
            let index = parser.store_expression(
                ExpressionType::SizeofExpr(operand),
                source,
                Some(sizeof_source),
                parser.hard_error_count > self.starting_error_count,
            );
            self.push_operand(index, true, false);
            return ParseAction::Reprocess;
        }

        if let Some(token) = token
            && Self::is_closing_delimiter(token.kind)
            && self.should_consume_unowned_closer(token.kind)
        {
            let position = if self.state == ExpressionParserState::Operand {
                "expression operand"
            } else {
                "operator in expression"
            };
            parser.report(
                ParserErrorType::ExpectedStatementExpression(position, Some(token.kind)),
                Some(token),
            );
            if self.state == ExpressionParserState::Operand {
                // The stray closer stands in for the missing operand.
                self.push_error(parser, Some(token));
            }
            self.phase = ExpressionPhase::Finish;
            return ParseAction::Consume;
        }

        if self.state == ExpressionParserState::Operator
            && self.is_boundary(token.map(|token| token.kind))
        {
            return self.finish(parser);
        }

        if self.state == ExpressionParserState::Operand {
            let Some(token) = token else {
                parser.report(
                    ParserErrorType::ExpectedStatementExpression("expression operand", None),
                    None,
                );
                self.push_error(parser, None);
                return self.finish(parser);
            };
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) {
                parser.pedantic_suppression += 1;
                self.phase = ExpressionPhase::PushPrefix(
                    UnaryOperator::Extension,
                    token.source_vectors,
                    ExpressionMode::CastExpression,
                );
                return ParseAction::Consume;
            }
            if let TokenType::Keyword(
                keyword @ (KeywordTokenType::BuiltinVaArg
                | KeywordTokenType::BuiltinVaStart
                | KeywordTokenType::BuiltinVaEnd
                | KeywordTokenType::BuiltinVaCopy
                | KeywordTokenType::BuiltinOffsetof
                | KeywordTokenType::BuiltinTypesCompatible
                | KeywordTokenType::BuiltinChooseExpr
                | KeywordTokenType::BuiltinConvertVector
                | KeywordTokenType::BuiltinBitCast),
            ) = token.kind
            {
                self.phase = ExpressionPhase::AwaitGnu;
                return super::gnu::GnuFrame::push(parser, super::gnu::GnuKind::Builtin(keyword));
            }
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::AmpersandAmpersand)
            ) {
                parser.extension(
                    crate::configuration::Feature::LabelsAsValues,
                    "label address",
                    token,
                );
                self.phase = ExpressionPhase::LabelAddress(token.source_vectors);
                return ParseAction::Consume;
            }
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Countof)) {
                self.phase = ExpressionPhase::CountofStart(token.source_vectors);
                return ParseAction::Consume;
            }
            if let TokenType::Keyword(
                keyword @ (KeywordTokenType::Generic | KeywordTokenType::Alignof),
            ) = token.kind
            {
                if keyword == KeywordTokenType::Alignof {
                    if KeywordTokenType::classify(token.contents, parser.context.configuration)
                        .is_some_and(|x| x.origin == Some(crate::configuration::FeatureOrigin::Gnu))
                    {
                        self.phase = ExpressionPhase::AlignofStart(token.source_vectors);
                        return ParseAction::Consume;
                    }
                    Self::report_alignof_expression(parser, token);
                }
                self.phase = ExpressionPhase::AwaitModern(keyword, token.source_vectors);
                return ParseAction::Push(parser.pooled_modern_frame(
                    if keyword == KeywordTokenType::Generic {
                        ModernKind::Generic
                    } else {
                        ModernKind::Operand {
                            type_only: false,
                            constant:  false,
                        }
                    },
                ));
            }
            // A brace group directly inside a call's or statement header's
            // `(` and closed before its `)` is one error operand even when it
            // holds statements: skip it whole so the parenthesis keeps its
            // `)`. A `({` that opens an expression is a GNU statement
            // expression and never reaches here.
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ) && parser.cursor.previous.is_some_and(|previous| {
                matches!(
                    previous.kind,
                    TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                )
            }) && brace_group_closes_before_parenthesis(parser)
            {
                parser.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "expression operand",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                self.phase = ExpressionPhase::RecoverUnexpectedBrace(1, token.source_vectors);
                return ParseAction::Consume;
            }
            // A visible typedef spelling is still a syntactically valid
            // primary expression. At the first initializer operand, do
            // not reinterpret declaration-shaped
            // lookahead as recovery evidence.
            let following_kind = parser.cursor.following().map(|following| following.kind);
            let identifier_is_unambiguous_recovery_boundary =
                matches!(token.kind, TokenType::Identifier)
                    && (matches!(
                        following_kind,
                        Some(TokenType::Operator(OperatorTokenType::Colon))
                    ) || self.recovery_boundary == ExpressionBoundary::Initializer
                        && matches!(following_kind, Some(TokenType::Identifier)));
            let identifier_is_operand = matches!(token.kind, TokenType::Identifier)
                && !identifier_is_unambiguous_recovery_boundary;
            let owned_boundary = self.is_owning_boundary(token.kind)
                || self.recovery_boundary != self.boundary
                    && Self::is_owning_boundary_for(self.recovery_boundary, token.kind);
            if owned_boundary
                || !identifier_is_operand && self.is_strong_grammar_boundary(parser, token)
            {
                // A declaration-specifier keyword inside this expression's
                // own delimiters, such as `f(int)` or `(static int)`, is a
                // stray token when the closer follows: skip to the closer
                // instead of abandoning the delimiters to a new declaration.
                let stray_run = if !owned_boundary
                    && matches!(token.kind, TokenType::Keyword(_))
                    && let Some(closer) = self.closer()
                {
                    closer_follows_stray_run(parser, closer, false)
                } else {
                    None
                };
                parser.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "expression operand",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                self.push_error(parser, Some(token));
                if let Some(stray_run) = stray_run {
                    self.phase = ExpressionPhase::SkipStray(stray_run);
                    return ParseAction::Reprocess;
                }
                return self.finish(parser);
            }
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Sizeof)) {
                self.phase = ExpressionPhase::SizeofStart(token.source_vectors);
                return ParseAction::Consume;
            }
            if let Some((operator, child_mode)) = prefix_operator(token.kind) {
                if operator == UnaryOperator::Extension {
                    parser.pedantic_suppression += 1;
                }
                self.phase =
                    ExpressionPhase::PushPrefix(operator, token.source_vectors, child_mode);
                return ParseAction::Consume;
            }
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            ) {
                if parser.cursor.following().is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                    )
                }) {
                    parser.extension(
                        crate::configuration::Feature::StatementExpressions,
                        "statement expression",
                        token,
                    );
                    self.phase = ExpressionPhase::AwaitStatementExpression(token.source_vectors);
                    return ParseAction::Consume;
                }
                let type_name_use = Self::parenthesized_type_name_follows(parser).then(|| {
                    if self.mode == ExpressionMode::UnaryExpression {
                        TypeNameUse::UnaryCompoundLiteral
                    } else {
                        TypeNameUse::Cast
                    }
                });
                if let Some(type_name_use) = type_name_use {
                    self.phase = ExpressionPhase::PushTypeName(token.source_vectors, type_name_use);
                    return ParseAction::Consume;
                }
                self.phase = ExpressionPhase::PushGrouped(token.source_vectors);
                return ParseAction::Consume;
            }
            if matches!(token.kind, TokenType::Identifier)
                && parser.func_name == Some(token.contents)
            {
                parser.extension(crate::configuration::Feature::Func, "__func__", token);
            }
            let kind = match token.kind {
                | TokenType::Keyword(KeywordTokenType::True) => ExpressionType::Boolean(true),
                | TokenType::Keyword(KeywordTokenType::False) => ExpressionType::Boolean(false),
                | TokenType::Keyword(KeywordTokenType::Nullptr) => ExpressionType::Nullptr,
                | TokenType::Identifier =>
                    ExpressionType::Identifier(Identifier::from_token(token)),
                | TokenType::Integer(value) => ExpressionType::Constant(Constant::Integer(value)),
                | TokenType::Float(value) => {
                    if matches!(
                        value,
                        super::super::preprocessing::FloatTokenType::Float128(_)
                            | super::super::preprocessing::FloatTokenType::ImaginaryFloat128(_)
                    ) {
                        parser.extension(
                            crate::configuration::Feature::Float128,
                            "binary128 floating constant",
                            token,
                        );
                    }
                    ExpressionType::Constant(Constant::Float(value))
                },
                | TokenType::Character(value) => ExpressionType::Constant(Constant::Char(value)),
                | TokenType::String(value) => ExpressionType::StringLiteral(value),
                | _ => {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "expression operand",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    if matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                    ) {
                        self.phase =
                            ExpressionPhase::RecoverUnexpectedBrace(1, token.source_vectors);
                        return ParseAction::Consume;
                    }
                    self.push_error(parser, Some(token));
                    if binary_operator(token.kind).is_some()
                        || matches!(
                            token.kind,
                            TokenType::Operator(OperatorTokenType::QuestionMark)
                        )
                    {
                        // Only the left operand is missing: the operator and
                        // its right operand still parse normally.
                        return ParseAction::Reprocess;
                    }
                    self.stray_error_operand = true;
                    return ParseAction::Consume;
                },
            };
            let index = parser.store_expression(
                kind,
                token.source_vectors,
                None,
                parser.hard_error_count > self.starting_error_count,
            );
            self.push_operand(index, true, true);
            ParseAction::Consume
        } else {
            let Some(token) = token else {
                return self.finish(parser);
            };
            // A brace list right after a `)` this expression consumed reads
            // as a compound literal whose type name did not parse, as in
            // `(a b){1, 2}`. Skip the list with the operand as one error
            // operand instead of leaking its `}` to the enclosing statement.
            // A statement block, as in `while (f(x) { ... }`, still ends the
            // expression so its owner names the missing `)`.
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            ) && parser.cursor.previous.is_some_and(|previous| {
                matches!(
                    previous.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
            }) && !brace_group_is_block(parser)
            {
                let operand = self.pop_operand().expression;
                if !operand.recovered {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "operator before brace list",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                let source = parser
                    .context
                    .merge_vectors(operand.source_vectors, token.source_vectors);
                self.phase = ExpressionPhase::RecoverUnexpectedBrace(1, source);
                return ParseAction::Consume;
            }
            // C99 §6.5.2p1: a postfix suffix applies only to a
            // `postfix-expression`.
            if is_postfix_starter(token.kind)
                && !self
                    .operands
                    .last()
                    .is_some_and(|operand| operand.postfix_expression)
            {
                parser.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "postfix operator after a non-postfix expression",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                // Keep the suffix in this expression, applied to the operand
                // and marked recovered, rather than leaking each of its
                // tokens into the enclosing production.
            }
            match token.kind {
                | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                    let base = self.pop_operand().expression;
                    self.phase = ExpressionPhase::PushSubscript(base, token.source_vectors);
                    return ParseAction::Consume;
                },
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                    let base = self.pop_operand().expression;
                    if let Some(call) = &mut self.call {
                        call.arguments.clear();
                        call.source_vectors.clear();
                    }
                    self.phase = ExpressionPhase::CallStart(base, token.source_vectors);
                    return ParseAction::Consume;
                },
                | TokenType::Operator(OperatorTokenType::Period | OperatorTokenType::Arrow) => {
                    let base = self.pop_operand().expression;
                    self.phase = ExpressionPhase::ExpectMember(
                        base,
                        matches!(token.kind, TokenType::Operator(OperatorTokenType::Arrow)),
                        token.source_vectors,
                    );
                    return ParseAction::Consume;
                },
                | TokenType::Operator(
                    OperatorTokenType::PlusPlus | OperatorTokenType::MinusMinus,
                ) => {
                    let base = self.pop_operand().expression;
                    let source = parser
                        .context
                        .merge_vectors(base.source_vectors, token.source_vectors);
                    let operator =
                        if matches!(token.kind, TokenType::Operator(OperatorTokenType::PlusPlus)) {
                            UnaryOperator::PostIncrement
                        } else {
                            UnaryOperator::PostDecrement
                        };
                    let index = parser.store_expression(
                        ExpressionType::Unary {
                            operator,
                            operand_expression: base,
                        },
                        source,
                        Some(token.source_vectors),
                        parser.hard_error_count > self.starting_error_count,
                    );
                    self.push_operand(index, true, true);
                    return ParseAction::Consume;
                },
                | _ => {},
            }
            // C99 §6.5.15p1: `?` follows a `logical-OR-expression`, so every
            // tighter operator (levels below 13) reduces first.
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::QuestionMark)
            ) && !matches!(
                self.mode,
                ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
            ) {
                while self.operators.last().is_some_and(|operator| {
                    !matches!(operator, LanguageExpressionOperator::Question { .. })
                        && operator.precedence() < 13
                }) {
                    self.reduce_one(parser);
                }
                self.operators.push(LanguageExpressionOperator::Question {
                    source_vectors: token.source_vectors,
                });
                self.phase = ExpressionPhase::PushConditionalMiddle;
                return ParseAction::Consume;
            }
            // C99: only `expression` absorbs a comma (§6.5.17p1), a
            // `constant-expression` takes no assignment (§6.6p1), and a
            // cast or unary operand takes no binary operator (§6.5.3p1,
            // §6.5.4p1).
            if let Some(operator) = binary_operator(token.kind)
                && !(operator == BinaryOperator::Comma && self.mode != ExpressionMode::Expression)
                && !(is_assignment_operator(operator)
                    && self.mode == ExpressionMode::ConstantExpression)
                && !matches!(
                    self.mode,
                    ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
                )
            {
                let incoming = LanguageExpressionOperator::Binary {
                    operator,
                    source_vectors: token.source_vectors,
                };
                while self.operators.last().is_some_and(|stacked| {
                    !matches!(stacked, LanguageExpressionOperator::Question { .. })
                        && stacked.has_precedence_over(incoming)
                }) {
                    self.reduce_one(parser);
                }
                // C99 §6.5.16p1: the left operand of an assignment operator
                // is a `unary-expression`.
                if is_assignment_operator(operator)
                    && !self
                        .operands
                        .last()
                        .is_some_and(|operand| operand.unary_expression)
                {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "unary-expression left operand of assignment",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                self.operators.push(incoming);
                self.state = ExpressionParserState::Operand;
                self.stray_error_operand = false;
                return ParseAction::Consume;
            }
            if let Some(closer) = self.closer() {
                // Inside delimiters this expression owns, skip a stray run
                // to the closer that follows it. Without such a closer, the
                // closer itself is missing: finish quietly so its owner
                // names the missing `)`, `]`, or `:` at this token.
                let Some(stray_run) = closer_follows_stray_run(parser, closer, false) else {
                    return self.finish(parser);
                };
                if !self.stray_error_operand {
                    parser.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "operator in delimited expression",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                self.phase = ExpressionPhase::SkipStray(stray_run);
                return ParseAction::Reprocess;
            }
            if is_statement_keyword(token.kind)
                && matches!(
                    self.boundary,
                    ExpressionBoundary::Statement(
                        ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon
                    )
                )
            {
                // The statement that owns the terminator reports the missing
                // `;` before the keyword.
                return self.finish(parser);
            }
            if matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Semicolon)
            ) {
                // A `;` never continues an expression, and every owner of
                // an expression that does not end at `;` (an enumerator
                // value) names what it expected there instead.
                return self.finish(parser);
            }
            parser.report(
                ParserErrorType::ExpectedStatementExpression(
                    "operator in expression",
                    Some(token.kind),
                ),
                Some(token),
            );
            self.finish(parser)
        }
    }

    /// The closer that ends this expression's own delimiters, when the
    /// boundary is a delimiter whose owner reports it missing.
    fn closer(&self) -> Option<fn(TokenType) -> bool> {
        match self.boundary {
            | ExpressionBoundary::ClosingParenthesis
            | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis) =>
                Some(|token| {
                    matches!(
                        token,
                        TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    )
                }),
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::ArrayBound
            | ExpressionBoundary::Designator => Some(|token| {
                matches!(
                    token,
                    TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                )
            }),
            | ExpressionBoundary::Argument => Some(|token| {
                matches!(
                    token,
                    TokenType::Operator(
                        OperatorTokenType::ClosingParenthesis | OperatorTokenType::Comma
                    )
                )
            }),
            | ExpressionBoundary::Statement(ExpressionTerminator::Colon) =>
                Some(|token| matches!(token, TokenType::Operator(OperatorTokenType::Colon))),
            | ExpressionBoundary::Statement(
                ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon,
            )
            | ExpressionBoundary::Initializer
            | ExpressionBoundary::StructMember
            | ExpressionBoundary::Enumerator => None,
        }
    }

    /// Returns whether the parenthesized type name starting at the current
    /// token belongs to a compound literal: the `)` closing it is followed
    /// directly by `{`. A `;` or `}` outside an aggregate body before that
    /// `)` ends the scan.
    ///
    /// C99: `( type-name ) { initializer-list }`, §6.5.2.5 paragraph 1,
    /// p. 75; PDF p. 87.
    /// Aggregate bodies: §6.7.2.1 paragraph 1, p. 101; PDF p. 113.
    /// C23: compound-literal storage, §6.5.3.6 paragraph 1,
    /// p. 78; PDF p. 91.
    fn type_name_is_compound_literal(parser: &Parser<'_, 'tu, 'p>) -> bool {
        let mut groups = 0usize;
        let mut braces = 0usize;
        let mut offset = 0usize;
        let mut token = parser.cursor.current();
        while let Some(current) = token {
            match current.kind {
                | TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
                ) => groups += 1,
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    if groups == 0 && braces == 0 =>
                    return parser.cursor.lookahead(offset).is_some_and(|x| {
                        matches!(
                            x.kind,
                            TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                        )
                    }),
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
                ) => groups = groups.saturating_sub(1),
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => braces += 1,
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if braces != 0 =>
                    braces -= 1,
                | TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
                ) if braces == 0 => return false,
                | _ => {},
            }
            token = parser.cursor.lookahead(offset);
            offset += 1;
        }
        false
    }

    /// Returns whether the current `(` opens a type name, or the storage
    /// classes of a compound literal before one, as in `(constexpr int){1}`.
    /// Storage-class and `inline` keywords that a compound literal cannot
    /// take are diagnosed by the type-name frame, as in `(static int)x`.
    /// C23: compound-literal storage-class specifiers §6.5.3.6 paragraph 1,
    /// p. 78; PDF p. 91.
    fn parenthesized_type_name_follows(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
        let storage = |token: Token| {
            matches!(
                token.kind,
                TokenType::Keyword(
                    KeywordTokenType::Auto
                        | KeywordTokenType::Constexpr
                        | KeywordTokenType::Extern
                        | KeywordTokenType::Inline
                        | KeywordTokenType::Register
                        | KeywordTokenType::Static
                        | KeywordTokenType::ThreadLocal
                        | KeywordTokenType::Typedef
                )
            )
        };
        let mut index = 0;
        while let Some(token) = parser.cursor.lookahead(index) {
            if parser.type_name_starter(token) {
                return true;
            }
            if !storage(token) {
                return false;
            }
            index += 1;
        }
        false
    }

    /// Returns whether the current `(` begins a compound literal: a type name
    /// whose `)` is followed by `{`.
    /// C99: §6.5.2.5 paragraph 1, p. 75; PDF p. 87.
    fn parenthesized_compound_literal_follows(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
        if !Self::parenthesized_type_name_follows(parser) {
            return false;
        }
        let mut groups = 0usize;
        let mut braces = 0usize;
        let mut index = 0;
        while let Some(token) = parser.cursor.lookahead(index) {
            index += 1;
            match token.kind {
                | TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
                ) => groups += 1,
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    if groups == 0 && braces == 0 =>
                    return parser.cursor.lookahead(index).is_some_and(|x| {
                        matches!(
                            x.kind,
                            TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                        )
                    }),
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
                ) => groups = groups.saturating_sub(1),
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => braces += 1,
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if braces != 0 =>
                    braces -= 1,
                | TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
                ) if braces == 0 => return false,
                | _ => {},
            }
        }
        false
    }

    /// Diagnoses `_Alignof` or `alignof` applied to a parenthesized
    /// expression, which GCC and Clang accept from `__alignof__`. The
    /// standard spellings take only a type name.
    /// C11: §6.5.3 paragraph 1, p. 88; PDF p. 106. Extension: GNU.
    fn report_alignof_expression(parser: &mut Parser<'_, 'tu, 'p>, keyword: Token) {
        let gnu_spelling =
            KeywordTokenType::classify(keyword.contents, parser.context.configuration)
                .is_some_and(|x| x.origin == Some(crate::configuration::FeatureOrigin::Gnu));
        let expression = parser.cursor.following().is_some_and(|x| {
            matches!(
                x.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            )
        }) && parser
            .cursor
            .lookahead(1)
            .is_some_and(|x| !parser.type_name_starter(x));
        if gnu_spelling || !expression || parser.pedantic_suppression != 0 {
            return;
        }
        let spelling = if parser
            .context
            .string_cache
            .at(keyword.contents)
            .starts_with('_')
        {
            "_Alignof (expression)"
        } else {
            "alignof (expression)"
        };
        parser.context.report_extension(
            crate::configuration::Feature::AlignofExpression,
            spelling,
            keyword.source_vectors,
        );
    }

    fn push_operand(
        &mut self,
        expression: &'tu Expression<'tu>,
        unary_expression: bool,
        postfix_expression: bool,
    ) {
        self.operands.push(ExpressionOperand {
            expression,
            unary_expression,
            postfix_expression,
        });
        self.state = ExpressionParserState::Operator;
        self.stray_error_operand = false;
    }

    fn pop_operand(&mut self) -> ExpressionOperand<'tu> {
        self.operands
            .pop()
            .expect("operator state has an expression operand")
    }

    /// Replaces the pending `?` marker with the conditional operator it
    /// begins, now that its middle operand and `:` are known, and goes on to
    /// the last operand.
    fn complete_conditional_marker(
        &mut self,
        middle: Option<&'tu Expression<'tu>>,
        colon_source: Option<SourceVectors>,
    ) {
        let Some(LanguageExpressionOperator::Question { source_vectors }) = self.operators.pop()
        else {
            panic!("a conditional's middle operand follows its question marker");
        };
        self.operators
            .push(LanguageExpressionOperator::Conditional {
                middle,
                question_source: source_vectors,
                colon_source,
            });
        self.phase = ExpressionPhase::PushConditionalElse;
    }

    fn pop_operand_or_error(&mut self, parser: &mut Parser<'_, 'tu, 'p>) -> ExpressionOperand<'tu> {
        if let Some(operand) = self.operands.pop() {
            return operand;
        }
        self.push_error(parser, None);
        self.operands
            .pop()
            .expect("error expression supplies one operand")
    }

    fn call_state(&mut self, pools: &mut FramePools<'tu, 'p>) -> &mut CallState<'tu, 'p> {
        self.call.get_or_insert_with(|| pools.take_call())
    }

    fn merge_call_source(&mut self, pools: &mut FramePools<'tu, 'p>, source: SourceVectors) {
        self.call_state(pools).source_vectors.push(source);
    }

    fn merge_call_operator_source(
        &mut self,
        pools: &mut FramePools<'tu, 'p>,
        source: SourceVectors,
    ) {
        self.call_state(pools).operator_sources.push(source);
    }

    fn finish_call(&mut self, parser: &mut Parser<'_, 'tu, 'p>, base: &'tu Expression<'tu>) {
        let call = self.call.as_mut().expect("a call is being finished");
        let arguments = parser.alloc_syntax_list(&mut call.arguments);
        let source_vectors = parser.context.merge_vector_list(&call.source_vectors);
        let operator_sources = parser.context.merge_vector_list(&call.operator_sources);
        let index = parser.store_expression(
            ExpressionType::Call {
                function_expression: base,
                arguments,
            },
            source_vectors,
            Some(operator_sources),
            parser.hard_error_count > self.starting_error_count,
        );
        call.source_vectors.clear();
        call.operator_sources.clear();
        self.push_operand(index, true, true);
    }

    fn reduce_one(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        let operator = self
            .operators
            .pop()
            .expect("a pending expression operator exists");
        match operator {
            | LanguageExpressionOperator::Binary {
                operator,
                source_vectors,
            } => {
                let right = self.pop_operand_or_error(parser).expression;
                let left = self.pop_operand_or_error(parser).expression;
                let source = parser
                    .context
                    .merge_vectors(left.source_vectors, source_vectors);
                let source = parser.context.merge_vectors(source, right.source_vectors);
                let index = parser.store_expression(
                    ExpressionType::Binary {
                        operator,
                        left_expression: left,
                        right_expression: right,
                    },
                    source,
                    Some(source_vectors),
                    parser.hard_error_count > self.starting_error_count,
                );
                self.push_operand(index, false, false);
            },
            | LanguageExpressionOperator::Question { .. }
            | LanguageExpressionOperator::Conditional { .. } =>
                panic!("conditional markers reduce only after their child frames return"),
        }
    }

    /// Returns whether the token ends this expression in operator position.
    ///
    /// C99: a cast or unary operand ends before any binary operator (§6.5.3
    /// paragraph 1, p. 78; PDF p. 90; §6.5.4 paragraph 1, p. 81; PDF p. 93);
    /// an `assignment-expression` ends before a comma (§6.5.16 paragraph 1,
    /// p. 91; PDF p. 103); a `constant-expression` ends before a comma or an
    /// assignment operator (§6.6 paragraph 1, p. 95; PDF p. 107).
    fn is_boundary(&self, token: Option<TokenType>) -> bool {
        if matches!(
            self.boundary,
            ExpressionBoundary::StructMember | ExpressionBoundary::Enumerator
        ) && matches!(token, Some(TokenType::Keyword(KeywordTokenType::Attribute)))
        {
            return true;
        }
        if matches!(
            self.boundary,
            ExpressionBoundary::Statement(ExpressionTerminator::Colon)
                | ExpressionBoundary::Designator
        ) && matches!(
            token,
            Some(TokenType::Operator(OperatorTokenType::Ellipsis))
        ) {
            return true;
        }
        let Some(token) = token else {
            return true;
        };
        if matches!(
            self.mode,
            ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
        ) && !is_postfix_starter(token)
        {
            return true;
        }
        if self.mode == ExpressionMode::AssignmentExpression
            && matches!(token, TokenType::Operator(OperatorTokenType::Comma))
        {
            return true;
        }
        if self.mode == ExpressionMode::ConstantExpression
            && matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Equals
                        | OperatorTokenType::PlusEquals
                        | OperatorTokenType::MinusEquals
                        | OperatorTokenType::AsteriskEquals
                        | OperatorTokenType::ForwardSlashEquals
                        | OperatorTokenType::PercentEquals
                        | OperatorTokenType::LessThanLessThanEquals
                        | OperatorTokenType::GreaterThanGreaterThanEquals
                        | OperatorTokenType::AmpersandEquals
                        | OperatorTokenType::CaretEquals
                        | OperatorTokenType::PipeEquals
                )
            )
        {
            return true;
        }
        if self.boundary == ExpressionBoundary::Argument
            && is_expression_operand_starter(token)
            && !is_postfix_starter(token)
            && binary_operator(token).is_none()
        {
            return true;
        }
        self.is_owning_boundary(token)
    }

    fn is_owning_boundary(&self, token: TokenType) -> bool {
        Self::is_owning_boundary_for(self.boundary, token)
    }

    fn is_owning_boundary_for(boundary: ExpressionBoundary, token: TokenType) -> bool {
        if Self::is_closing_delimiter(token) {
            return true;
        }
        match boundary {
            | ExpressionBoundary::Statement(
                ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon,
            ) => matches!(token, TokenType::Operator(OperatorTokenType::Semicolon)),
            | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
            | ExpressionBoundary::ClosingParenthesis => matches!(
                token,
                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            ),
            | ExpressionBoundary::Statement(ExpressionTerminator::Colon) =>
                matches!(token, TokenType::Operator(OperatorTokenType::Colon)),
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::ArrayBound
            | ExpressionBoundary::Designator => matches!(
                token,
                TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
            ),
            | ExpressionBoundary::Argument => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingParenthesis
                )
            ),
            | ExpressionBoundary::Initializer => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::Semicolon
                )
            ),
            | ExpressionBoundary::StructMember => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::ClosingCurlyBrace
                )
            ),
            | ExpressionBoundary::Enumerator => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                )
            ),
        }
    }

    fn is_closing_delimiter(token: TokenType) -> bool {
        matches!(
            token,
            TokenType::Operator(
                OperatorTokenType::ClosingParenthesis
                    | OperatorTokenType::ClosingSquareBracket
                    | OperatorTokenType::ClosingCurlyBrace
            )
        )
    }

    fn should_consume_unowned_closer(&self, token: TokenType) -> bool {
        match self.boundary {
            | ExpressionBoundary::Statement(ExpressionTerminator::Semicolon) => matches!(
                token,
                TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket
                )
            ),
            | ExpressionBoundary::Statement(
                ExpressionTerminator::ForSemicolon | ExpressionTerminator::ClosingParenthesis,
            ) => matches!(
                token,
                TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
            ),
            | ExpressionBoundary::Statement(ExpressionTerminator::Colon)
            | ExpressionBoundary::ClosingParenthesis
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::Argument
            | ExpressionBoundary::Initializer
            | ExpressionBoundary::ArrayBound
            | ExpressionBoundary::StructMember
            | ExpressionBoundary::Enumerator
            | ExpressionBoundary::Designator => false,
        }
    }

    fn is_strong_grammar_boundary(&self, parser: &mut Parser<'_, 'tu, 'p>, token: Token) -> bool {
        let identifier_precedes_conditional_colon = self.boundary
            == ExpressionBoundary::Statement(ExpressionTerminator::Colon)
            && matches!(token.kind, TokenType::Identifier)
            && parser.cursor.following().is_some_and(|following| {
                matches!(
                    following.kind,
                    TokenType::Operator(OperatorTokenType::Colon)
                )
            });
        // A brace in a statement-level expression ends it only when the
        // group reads as a statement block, as in `if (value { return; }`.
        // A misplaced brace list such as `a = {1, 2};` is skipped as one
        // error operand instead.
        matches!(
            token.kind,
            TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
        ) && matches!(
            self.recovery_boundary,
            ExpressionBoundary::Statement(_) | ExpressionBoundary::ClosingParenthesis
        ) && brace_group_is_block(parser)
            && !brace_group_continues_expression(parser)
            || Self::is_strong_grammar_boundary_for(parser, token, self.boundary)
            || self.recovery_boundary != self.boundary
                && !identifier_precedes_conditional_colon
                && Self::is_strong_grammar_boundary_for(parser, token, self.recovery_boundary)
    }

    pub(super) fn is_strong_grammar_boundary_for(
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Token,
        boundary: ExpressionBoundary,
    ) -> bool {
        let identifier_continues_as_postfix = matches!(token.kind, TokenType::Identifier)
            && parser
                .cursor
                .following()
                .is_some_and(|following| is_postfix_starter(following.kind));
        let declaration_starts_here = parser.declaration_recovery_starts_here(token);
        match boundary {
            | ExpressionBoundary::Initializer =>
                is_statement_keyword(token.kind)
                    || declaration_starts_here
                        && !matches!(
                            parser.cursor.following().map(|following| following.kind),
                            Some(TokenType::Operator(
                                OperatorTokenType::Comma
                                    | OperatorTokenType::Semicolon
                                    | OperatorTokenType::ClosingCurlyBrace
                            ))
                        ),
            | ExpressionBoundary::ArrayBound =>
                !matches!(token.kind, TokenType::Identifier)
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser.cursor.following().map(|following| following.kind),
                        Some(TokenType::Operator(OperatorTokenType::ClosingSquareBracket))
                    ),
            | ExpressionBoundary::StructMember =>
                !matches!(token.kind, TokenType::Identifier)
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser.cursor.following().map(|following| following.kind),
                        Some(TokenType::Operator(
                            OperatorTokenType::Comma
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        ))
                    ),
            | ExpressionBoundary::Enumerator =>
                !matches!(token.kind, TokenType::Identifier)
                    && parser.declaration_starter(token)
                    && !matches!(
                        parser.cursor.following().map(|following| following.kind),
                        Some(TokenType::Operator(
                            OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                        ))
                    ),
            | ExpressionBoundary::Statement(_) | ExpressionBoundary::ClosingParenthesis =>
                declaration_starts_here && !identifier_continues_as_postfix
                    || is_statement_keyword(token.kind)
                    || matches!(
                        boundary,
                        ExpressionBoundary::Statement(ExpressionTerminator::Semicolon)
                    ) && matches!(token.kind, TokenType::Identifier)
                        && parser.cursor.following().is_some_and(|following| {
                            matches!(
                                following.kind,
                                TokenType::Operator(OperatorTokenType::Colon)
                            )
                        }),
            | ExpressionBoundary::ClosingSquareBracket
            | ExpressionBoundary::Argument
            | ExpressionBoundary::Designator => false,
        }
    }

    fn push_error(&mut self, parser: &mut Parser<'_, 'tu, 'p>, anchor: Option<Token>) {
        let source_vectors = match anchor {
            | Some(token) if token.source_vectors.length() != 0 => {
                let mut combined = SourceVectors::default();
                for index in 0..token.source_vectors.length() as usize {
                    // Creating an anchor appends to the retained provenance,
                    // so the token's vectors are read again each time.
                    let source = &parser.context.get_source_vectors(token.source_vectors)[index];
                    let (position, source_file_index) =
                        (source.position(parser.context), source.source_file_index);
                    let anchor = parser.context.create_retained_source_vectors(
                        position,
                        source_file_index,
                        0,
                    );
                    combined = parser.context.merge_vectors(combined, anchor);
                }
                combined
            },
            | _ => parser.missing_syntax_source(),
        };
        self.push_error_with_source(parser, source_vectors, None);
    }

    fn push_error_with_source(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
    ) {
        let index = parser.store_expression(
            ExpressionType::Error,
            source_vectors,
            operator_source_vectors,
            true,
        );
        // An error operand stands for whatever was meant, so operators that
        // need a unary or postfix operand do not diagnose it a second time.
        self.push_operand(index, true, true);
    }

    /// Reduces every pending operator and returns the expression, typed as a
    /// `constant-expression` (§6.6 paragraph 1, p. 95; PDF p. 107) in that
    /// mode. Whether it is in fact constant (§6.6 paragraphs 3-10, pp. 95-96;
    /// PDF pp. 107-108) is left to semantic analysis.
    fn finish(&mut self, parser: &mut Parser<'_, 'tu, 'p>) -> ParseAction<'tu, 'p> {
        while !self.operators.is_empty() {
            self.reduce_one(parser);
        }
        let operand = self.pop_operand_or_error(parser);
        let recovered = parser.hard_error_count > self.starting_error_count;
        let expression = if recovered {
            parser.mark_expression_recovered(operand.expression)
        } else {
            operand.expression
        };
        if self.mode == ExpressionMode::ConstantExpression {
            ParseAction::Reduce(ParseValue::ConstantExpression(ConstantExpressionResult {
                expression: ConstantExpression(expression),
                recovered,
            }))
        } else {
            ParseAction::Reduce(ParseValue::Expression(ExpressionResult {
                expression,
                recovered,
            }))
        }
    }
}
