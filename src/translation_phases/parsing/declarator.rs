//! Declarator frame for concrete and abstract declarators.

use std::{
    cell::Cell,
    fmt::Debug,
    rc::Rc,
};

use super::{
    Parser,
    declaration_specifiers::{
        report_duplicate_type_qualifier,
        type_qualifier,
    },
    declaration_syntax::{
        Declarator,
        DirectDeclarator,
        ParenthesizedDeclarator,
        PointerDeclarator,
        TypeQualifiers,
    },
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    expression_operators::{
        is_array_pointer_marker,
        is_operator,
    },
    machine::{
        ExpressionResult,
        ParameterListResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    parameter_list::ParameterListFrame,
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    syntax::{
        ExpressionIndex,
        Identifier,
        ParenthesizedDeclaratorIndex,
        SyntaxList,
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

/// Whether a declarator requires, forbids, or optionally accepts a name.
///
/// C99: named declarators are §6.7.5, pp. 114-121; PDF pp. 126-133; abstract
/// declarators are §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclaratorMode {
    /// Ordinary declarator requiring an identifier base.
    Named,
    /// Abstract declarator used where no identifier may be declared.
    Abstract,
    /// Parameter declarator that may be named or abstract.
    MaybeAbstract,
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The booleans retain independent C array/declarator grammar facts."
)]
/// Iteratively parses named, abstract, or maybe-abstract declarators.
///
/// C99: declarator and direct-declarator are §6.7.5, pp. 114-121; PDF
/// pp. 126-133. Abstract forms are §6.7.6, p. 122; PDF p. 134.
pub(super) struct DeclaratorFrame {
    /// Current pointer/base/suffix transition.
    phase: DeclaratorPhase,
    /// Whether the declarator requires or permits an identifier.
    mode: DeclaratorMode,
    /// Qualifiers for completed pointer levels, outermost first.
    pub(super) pointer_qualifiers: Vec<TypeQualifiers>,
    /// Direct base and suffixes accumulated before arena insertion.
    pub(super) direct_declarators: Vec<DirectDeclarator>,
    /// Qualifiers being collected for the current pointer level.
    current_qualifiers: TypeQualifiers,
    /// Whether at least one pointer level has been parsed.
    has_pointer_level: bool,
    /// Whether an identifier or parenthesized base has been parsed.
    has_direct_declarator: bool,
    /// Whether this declarator declares an identifier, directly or through
    /// a parenthesized nested declarator.
    named: bool,
    /// Flag shared along one chain of parenthesized declarators, set when
    /// the innermost one parses its identifier. A nested declarator's
    /// identifier is also its parent's, so each `(`-nesting level learns
    /// whether it is named without walking its children again, keeping deep
    /// `(*(*(*x)())())()` chains linear.
    chain_named: Option<Rc<Cell<bool>>>,
    /// Qualifiers accumulated for the active array suffix.
    array_qualifiers: TypeQualifiers,
    /// Whether array qualifiers occurred before `static`.
    array_qualifiers_before_static: bool,
    /// Whether the active array suffix contains `static`.
    array_is_static: bool,
    /// Whether the active array suffix uses the `[*]` form.
    array_is_pointer: bool,
    /// Parsed assignment-expression bound for the active array suffix.
    array_assignment_expression: Option<ExpressionIndex>,
    /// Provenance accumulated across every declarator component.
    pub(super) source_vectors: Option<SourceVectors>,
    /// Opening delimiter of the active parenthesized declarator.
    nested_open: Option<SourceVectors>,
}

/// State transitions for pointer, base, and suffix portions of a declarator.
///
/// C99: pointer, array, and function-derived declarator productions are
/// §6.7.5.1-§6.7.5.3, pp. 115-121; PDF pp. 127-133.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclaratorPhase {
    /// Decide whether the declarator begins with a pointer or direct base.
    PointerOrBase,
    /// Collect qualifiers for the current pointer level.
    PointerQualifiers,
    /// Parse the identifier or opening parenthesis of the direct base.
    Base,
    /// Push a nested declarator after `(`.
    PushNested,
    /// Disambiguate an abstract `(` as grouping or a function suffix.
    ClassifyAbstractParenthesis,
    /// Receive a nested parenthesized declarator.
    AwaitNested,
    /// Require the `)` owned by a parenthesized declarator.
    ExpectNestedClose,
    /// Consume zero or more array/function suffixes.
    Suffix,
    /// Parse array qualifiers, `static`, `*`, or an optional bound.
    Array,
    /// Require the closing bracket of an expression-free array suffix.
    ArrayExpectClose,
    /// Receive and close an array-bound expression.
    AwaitArrayBound,
    /// Decide whether a function suffix is empty, K&R, or prototype-style.
    FunctionStart,
    /// Receive a parameter-list child.
    AwaitParameterList,
    /// Store qualifiers/direct parts and return the declarator.
    Finish,
}

impl DeclaratorFrame {
    pub(super) fn new(mode: DeclaratorMode) -> Self {
        Self {
            phase: DeclaratorPhase::PointerOrBase,
            mode,
            pointer_qualifiers: Vec::new(),
            direct_declarators: Vec::new(),
            current_qualifiers: TypeQualifiers::empty(),
            has_pointer_level: false,
            has_direct_declarator: false,
            named: false,
            chain_named: None,
            array_qualifiers: TypeQualifiers::empty(),
            array_qualifiers_before_static: false,
            array_is_static: false,
            array_is_pointer: false,
            array_assignment_expression: None,
            source_vectors: None,
            nested_open: None,
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
            | DeclaratorPhase::PointerOrBase => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.has_pointer_level = true;
                    self.current_qualifiers = TypeQualifiers::empty();
                    self.phase = DeclaratorPhase::PointerQualifiers;
                    ParseAction::Consume
                } else {
                    self.phase = DeclaratorPhase::Base;
                    ParseAction::Continue
                }
            },
            | DeclaratorPhase::PointerQualifiers => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if let Some(token) = token
                    && let Some(qualifier) = type_qualifier(token.kind)
                {
                    if self.mode != DeclaratorMode::Abstract
                        && self.current_qualifiers.contains(qualifier)
                    {
                        report_duplicate_type_qualifier(parser, context, token, qualifier);
                    }
                    self.current_qualifiers.insert(qualifier);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                // Each `*` owns the qualifiers immediately following
                // it. A second `*` closes the
                // current pointer level and starts the
                // next without involving Rust recursion.
                if is_operator(token, OperatorTokenType::Asterisk) {
                    self.pointer_qualifiers.push(self.current_qualifiers);
                    self.current_qualifiers = TypeQualifiers::empty();
                    let token = token.expect("asterisk token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else {
                    self.pointer_qualifiers.push(self.current_qualifiers);
                    self.current_qualifiers = TypeQualifiers::empty();
                    self.phase = DeclaratorPhase::Base;
                    ParseAction::Continue
                }
            },
            | DeclaratorPhase::Base => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if let Some(token) = token
                    && token.kind == TokenType::Identifier
                    && self.mode != DeclaratorMode::Abstract
                {
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.direct_declarators
                        .push(DirectDeclarator::Identifier(Identifier::from_token(token)));
                    self.has_direct_declarator = true;
                    self.named = true;
                    if let Some(chain_named) = &self.chain_named {
                        chain_named.set(true);
                    }
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Consume;
                }
                // In a named declarator, `(` must group another
                // declarator. In an abstract
                // context it can instead start a function suffix,
                // so defer that choice to the classification phase.
                if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    let token = token.expect("opening-parenthesis token exists");
                    self.nested_open = Some(token.source_vectors);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = if self.mode == DeclaratorMode::Named {
                        DeclaratorPhase::PushNested
                    } else {
                        DeclaratorPhase::ClassifyAbstractParenthesis
                    };
                    return ParseAction::Consume;
                }
                if self.mode != DeclaratorMode::Named
                    && is_operator(token, OperatorTokenType::OpeningSquareBracket)
                {
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Reprocess;
                }
                if self.mode == DeclaratorMode::Named {
                    parser.report(
                context,
                ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                    token.map(|token| token.kind),
                ),
                token,
            );
                }
                self.phase = DeclaratorPhase::Finish;
                ParseAction::Continue
            },
            | DeclaratorPhase::PushNested => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclaratorPhase::AwaitNested;
                ParseAction::Push(ParseFrame::Declarator(self.nested_frame()))
            },
            | DeclaratorPhase::ClassifyAbstractParenthesis => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `()` is an abstract function declarator; a
                // declaration starter begins a
                // prototype; everything else is parsed as a
                // parenthesized abstract declarator.
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    self.direct_declarators
                        .push(DirectDeclarator::KAndRStyleFunction {
                            parameters: SyntaxList::empty(),
                        });
                    self.has_direct_declarator = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    ParseAction::Push(ParseFrame::ParameterList(Box::new(
                        ParameterListFrame::new(false),
                    )))
                } else {
                    self.phase = DeclaratorPhase::AwaitNested;
                    ParseAction::Push(ParseFrame::Declarator(self.nested_frame()))
                }
            },
            | DeclaratorPhase::AwaitNested => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("nested declarator returned an unexpected value: {returned:?}");
                };
                let Some(declarator) = declarator else {
                    parser.report(
                context,
                ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Reprocess;
                };
                let source_vectors = declarator.source_vectors;
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                let parenthesized =
                    ParenthesizedDeclaratorIndex(parser.push_syntax(ParenthesizedDeclarator {
                        declarator,
                        delimiters: self.nested_open.take().unwrap_or_default(),
                    }));
                self.direct_declarators
                    .push(DirectDeclarator::Parenthesized(parenthesized));
                self.has_direct_declarator = true;
                self.named = self
                    .chain_named
                    .as_ref()
                    .is_some_and(|chain_named| chain_named.get());
                self.phase = DeclaratorPhase::ExpectNestedClose;
                ParseAction::Reprocess
            },
            | DeclaratorPhase::ExpectNestedClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = DeclaratorPhase::Suffix;
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    if let Some(DirectDeclarator::Parenthesized(index)) =
                        self.direct_declarators.last()
                    {
                        let delimiters = &mut parser.syntax[*index].delimiters;
                        *delimiters = context.merge_vectors(*delimiters, token.source_vectors);
                    }
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Reprocess
                }
            },
            | DeclaratorPhase::Suffix => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Direct-declarator suffixes repeat left-to-right.
                // Re-enter this phase after
                // every array or function child to preserve
                // binding order in the arena slice.
                if is_operator(token, OperatorTokenType::OpeningSquareBracket) {
                    let token = token.expect("opening-square-bracket token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.array_qualifiers = TypeQualifiers::empty();
                    self.array_qualifiers_before_static = false;
                    self.array_is_static = false;
                    self.array_is_pointer = false;
                    self.array_assignment_expression = None;
                    self.phase = DeclaratorPhase::Array;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    let token = token.expect("opening-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::FunctionStart;
                    ParseAction::Consume
                } else {
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Continue
                }
            },
            | DeclaratorPhase::Array => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // One state recognizes all four C99 array suffix
                // families. Keep `static`,
                // qualifiers, and `*` as independent facts so
                // later semantic checks retain their original
                // placement.
                if let Some(token) = token
                    && let Some(qualifier) = type_qualifier(token.kind)
                {
                    if self.mode != DeclaratorMode::Abstract
                        && self.array_qualifiers.contains(qualifier)
                    {
                        report_duplicate_type_qualifier(parser, context, token, qualifier);
                    }
                    if self.array_is_static && self.array_qualifiers_before_static {
                        parser.report(
                    context,
                    ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                    Some(token),
                );
                    }
                    if !self.array_is_static {
                        self.array_qualifiers_before_static = true;
                    }
                    self.array_qualifiers.insert(qualifier);
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Static))
                {
                    let token = token.expect("static token exists");
                    if self.array_is_static {
                        parser.report(context, ParserErrorType::StaticSpecifiedTwice, Some(token));
                    }
                    self.array_is_static = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if is_array_pointer_marker(parser, context, token) {
                    let token = token.expect("asterisk token exists");
                    // C99 §6.7.6p1: an abstract declarator's `[ * ]` takes no
                    // qualifiers, whichever suffixes or grouping precede it.
                    if self.mode != DeclaratorMode::Named
                        && !self.named
                        && !self.array_qualifiers.is_empty()
                    {
                        parser.report(
                    context,
                    ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
                    Some(token),
                );
                    }
                    if self.array_is_static {
                        parser.report(
                            context,
                            ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator,
                            Some(token),
                        );
                    }
                    self.array_is_pointer = true;
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::ArrayExpectClose;
                    return ParseAction::Consume;
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    if self.array_is_static {
                        parser.report(
                    context,
                    ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
                    Some(token),
                );
                    }
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Consume;
                }
                if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None),
                        None,
                    );
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    return ParseAction::Reprocess;
                }
                self.phase = DeclaratorPhase::AwaitArrayBound;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::ArrayBound,
                    parser.hard_error_count,
                )))
            },
            | DeclaratorPhase::ArrayExpectClose => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // After the VLA `*` marker, only `]` belongs to this
                // grammar alternative. Diagnose
                // extra input specifically, then recover
                // to the owning bracket or an enclosing declaration
                // boundary.
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.report(context, ParserErrorType::PointerSpecifiedTwice, Some(token));
                    parser.merge_source(context, &mut self.source_vectors, token);
                    ParseAction::Consume
                } else if let Some(token) = token {
                    parser.report(
                context,
                ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                    token.kind,
                ),
                Some(token),
            );
                    self.phase = DeclaratorPhase::AwaitArrayBound;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                } else {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfArrayDeclaratorAfterPointer,
                        None,
                    );
                    self.phase = DeclaratorPhase::AwaitArrayBound;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                }
            },
            | DeclaratorPhase::AwaitArrayBound => {
                // This phase is entered both after an expression child
                // and after direct recovery from malformed `[* ...]`
                // input. Merge a child only
                // when one actually returned.
                if let Some(returned) = returned {
                    let ParseValue::Expression(ExpressionResult { index, .. }) = returned else {
                        panic!("array-bound frame returned an unexpected value: {returned:?}");
                    };
                    let source_vectors = parser.syntax[index].source_vectors;
                    self.array_assignment_expression = Some(index);
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || token.is_some_and(|token| parser.declaration_starter(token))
                {
                    // These tokens belong to an enclosing production.
                    // Repair the absent
                    // `]`, finish this declarator, and reprocess the
                    // boundary instead of consuming it here.
                    if !self.array_is_pointer {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    if !self.array_is_pointer {
                        parser.report(
                            context,
                            ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                                None,
                            ),
                            None,
                        );
                    }
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::ArrayBound,
                        target: ParseFrameKind::Declarator,
                    })
                }
            },
            | DeclaratorPhase::FunctionStart => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // An identifier-list is legal only when this suffix is
                // bound to a named declarator.
                // Abstract function declarators always
                // interpret their contents as a prototype.
                let allow_k_and_r = self.named;
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let direct = if allow_k_and_r {
                        DirectDeclarator::KAndRStyleFunction {
                            parameters: SyntaxList::empty(),
                        }
                    } else {
                        DirectDeclarator::Function {
                            parameter_list: SyntaxList::empty(),
                            is_variadic:    false,
                        }
                    };
                    self.direct_declarators.push(direct);
                    self.has_direct_declarator = true;
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(context, &mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    ParseAction::Push(ParseFrame::ParameterList(Box::new(
                        ParameterListFrame::new(allow_k_and_r),
                    )))
                }
            },
            | DeclaratorPhase::AwaitParameterList => {
                let Some(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator,
                    source_vectors,
                })) = returned
                else {
                    panic!("parameter list returned an unexpected value: {returned:?}");
                };
                self.source_vectors =
                    Some(self.source_vectors.map_or(source_vectors, |existing| {
                        context.merge_vectors(existing, source_vectors)
                    }));
                self.direct_declarators.push(direct_declarator);
                self.has_direct_declarator = true;
                self.phase = DeclaratorPhase::Suffix;
                ParseAction::Reprocess
            },
            | DeclaratorPhase::Finish => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `None` is a typed, recoverable absence used by
                // optional abstract declarators
                // and by callers that issue the contextual
                // missing-declarator diagnostic.
                if !self.has_pointer_level && !self.has_direct_declarator {
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }
                if self.mode == DeclaratorMode::Named && !self.has_direct_declarator {
                    parser.report(
                        context,
                        ParserErrorType::TypeQualifiersWithoutDeclarator,
                        token,
                    );
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }

                // Commit both flat component lists atomically before
                // returning the value that
                // references their stable slices.
                let pointer_start = parser.append_syntax(&mut self.pointer_qualifiers);
                let direct_start = parser.append_syntax(&mut self.direct_declarators);
                let declarator = Declarator {
                    pointer:        PointerDeclarator {
                        type_qualifiers_list: pointer_start,
                    },
                    kind:           direct_start,
                    source_vectors: self.source_vectors.unwrap_or_default(),
                };
                ParseAction::Reduce(ParseValue::Declarator(Some(declarator)))
            },
        }
    }

    fn push_array(&mut self) {
        self.direct_declarators.push(DirectDeclarator::Array {
            type_qualifiers:       self.array_qualifiers,
            is_static:             self.array_is_static,
            is_pointer:            self.array_is_pointer,
            assignment_expression: self.array_assignment_expression.take(),
        });
        self.has_direct_declarator = true;
        self.array_qualifiers = TypeQualifiers::empty();
        self.array_qualifiers_before_static = false;
        self.array_is_static = false;
        self.array_is_pointer = false;
    }

    /// Creates the declarator nested inside this one's `(`, sharing the
    /// chain flag through which it reports an identifier. Abstract
    /// declarators never declare one, so they skip the allocation.
    fn nested_frame(&mut self) -> Self {
        let mut nested = Self::new(self.mode);
        if self.mode != DeclaratorMode::Abstract {
            nested.chain_named = Some(Rc::clone(
                self.chain_named
                    .get_or_insert_with(|| Rc::new(Cell::new(false))),
            ));
        }
        nested
    }
}
