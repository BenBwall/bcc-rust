//! Declarator frame for concrete and abstract declarators.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `declarator`, `direct-declarator`, `pointer`, and `type-qualifier-list`
//! (C99: §6.7.5 paragraph 1, p. 114; PDF p. 126) with the pointer, array, and
//! function derivations of §6.7.5.1-§6.7.5.3, pp. 115-121; PDF pp. 127-133,
//! and of `abstract-declarator` and `direct-abstract-declarator` (§6.7.6
//! paragraph 1, p. 122; PDF p. 134); summarized in §A.2.2, pp. 413-414;
//! PDF pp. 425-426. Parameter lists belong to the parameter-list frame.
//!
//! Diagnosed here: array suffixes outside the four §6.7.5 paragraph 1 forms
//! and qualifiers before an abstract `[*]` (§6.7.6 paragraph 1). Left to
//! semantic analysis: the array constraints of §6.7.5.2 paragraphs 1-2,
//! p. 116; PDF p. 128 (including where `static` and qualifiers may appear),
//! the return-type constraint of §6.7.5.3 paragraph 1, p. 118; PDF p. 130,
//! and the type each derivation specifies. Pointer levels and parenthesized
//! declarators nest through the frame stack, so the minimums of 12
//! declarators and 63 parenthesized-declarator levels (§5.2.4.1, p. 20;
//! PDF p. 32; §6.7.5 paragraph 7, p. 115; PDF p. 127) impose no fixed
//! ceiling.

use std::{
    cell::Cell,
    fmt::Debug,
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
    frame_pool::FramePools,
    machine::{
        ExpressionResult,
        ParameterListResult,
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
    parameter_list::ParameterListFrame,
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    syntax::{
        Expression,
        Identifier,
    },
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
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaVec,
            Bump,
        },
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
pub(super) struct DeclaratorFrame<'tu, 'p> {
    /// Current pointer/base/suffix transition.
    phase: DeclaratorPhase,
    attribute_resume: DeclaratorPhase,
    /// Whether the declarator requires or permits an identifier.
    mode: DeclaratorMode,
    /// Qualifiers for completed pointer levels, outermost first.
    pub(super) pointer_qualifiers: ArenaVec<'p, TypeQualifiers>,
    /// Direct base and suffixes accumulated before arena insertion.
    pub(super) direct_declarators: ArenaVec<'p, DirectDeclarator<'tu>>,
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
    /// `(*(*(*x)())())()` chains linear. The flag lives in the parse arena
    /// and returns to the frame pools when the frame that took it pops.
    chain_named: Option<&'p Cell<bool>>,
    /// Whether this frame took `chain_named` from the pools, as the
    /// outermost declarator of its chain.
    owns_chain_named: bool,
    /// Qualifiers accumulated for the active array suffix.
    array_qualifiers: TypeQualifiers,
    /// Whether array qualifiers occurred before `static`.
    array_qualifiers_before_static: bool,
    /// Whether the active array suffix contains `static`.
    array_is_static: bool,
    /// Whether the active array suffix uses the `[*]` form.
    array_is_pointer: bool,
    /// Parsed assignment-expression bound for the active array suffix.
    array_assignment_expression: Option<&'tu Expression<'tu>>,
    /// Provenance accumulated across every declarator component.
    pub(super) source_vectors: Option<SourceVectors>,
    /// Opening delimiter of the active parenthesized declarator.
    nested_open: Option<SourceVectors>,
    /// Parenthesized declarator waiting for its `)`. It is stored once its
    /// delimiters are complete, because stored syntax never changes.
    pub(super) nested: Option<ParenthesizedDeclarator<'tu>>,
}

/// State transitions for pointer, base, and suffix portions of a declarator.
///
/// C99: pointer, array, and function-derived declarator productions are
/// §6.7.5.1-§6.7.5.3, pp. 115-121; PDF pp. 127-133.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclaratorPhase {
    AwaitAttributes,
    AwaitAsm,
    AwaitPointerAttributes,
    /// Decide whether the declarator begins with a pointer or direct base.
    PointerOrBase,
    /// Collect qualifiers for the current pointer level.
    PointerQualifiers,
    /// Parse the identifier or opening parenthesis of the direct base.
    Base,
    /// Push a nested declarator after `(`.
    PushNested,
    /// Disambiguate an abstract `(` as grouping or a function suffix.
    ///
    /// C99: empty parentheses in a type name are a function declarator, not
    /// grouping, §6.7.6 footnote 128, p. 122; PDF p. 134; an identifier that
    /// could be a typedef name or a parameter name is taken as a typedef
    /// name, §6.7.5.3 paragraph 11, p. 119; PDF p. 131.
    ClassifyAbstractParenthesis,
    /// Receive a nested parenthesized declarator.
    AwaitNested,
    /// Require the `)` owned by a parenthesized declarator.
    ExpectNestedClose,
    /// Consume zero or more array/function suffixes.
    Suffix,
    /// Parse array qualifiers, `static`, `*`, or an optional bound.
    ///
    /// C99: the four array suffix forms of §6.7.5 paragraph 1, p. 114;
    /// PDF p. 126, and §6.7.6 paragraph 1, p. 122; PDF p. 134.
    Array,
    /// Require the closing bracket of an expression-free array suffix.
    ArrayExpectClose,
    /// Receive and close an array-bound expression.
    AwaitArrayBound,
    /// Decide whether a function suffix is empty, K&R, or prototype-style.
    ///
    /// C99: `( parameter-type-list )` and `( identifier-list? )`, §6.7.5
    /// paragraph 1, p. 114; PDF p. 126; abstract declarators take only
    /// `( parameter-type-list? )`, §6.7.6 paragraph 1, p. 122; PDF p. 134.
    FunctionStart,
    /// Receive a parameter-list child.
    AwaitParameterList,
    /// Store qualifiers/direct parts and return the declarator.
    Finish,
}

impl<'tu, 'p> DeclaratorFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump, mode: DeclaratorMode) -> Self {
        Self {
            phase: DeclaratorPhase::PointerOrBase,
            attribute_resume: DeclaratorPhase::PointerOrBase,
            mode,
            pointer_qualifiers: ArenaVec::new_in(arena),
            direct_declarators: ArenaVec::new_in(arena),
            current_qualifiers: TypeQualifiers::empty(),
            has_pointer_level: false,
            has_direct_declarator: false,
            named: false,
            chain_named: None,
            owns_chain_named: false,
            array_qualifiers: TypeQualifiers::empty(),
            array_qualifiers_before_static: false,
            array_is_static: false,
            array_is_pointer: false,
            array_assignment_expression: None,
            source_vectors: None,
            nested_open: None,
            nested: None,
        }
    }

    /// C23: §6.7.7.4 paragraph 13, p. 130; PDF p. 143 makes every empty
    /// function declarator a prototype, as if its parameter list were `void`.
    fn empty_function(parser: &Parser<'_, 'tu, 'p>, legacy: bool) -> DirectDeclarator<'tu> {
        if legacy && parser.context.configuration.standard() < crate::configuration::CStandard::C23
        {
            DirectDeclarator::KAndRStyleFunction {
                parameters: ArenaList::empty(),
            }
        } else {
            DirectDeclarator::Function {
                parameter_list: ArenaList::empty(),
                is_variadic:    false,
            }
        }
    }

    /// GNU declarator labels and shared attribute attachment points.
    /// C99: vendor extension to §6.7.5, p. 114; PDF p. 126.
    fn step_extension(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> Option<ParseAction<'tu, 'p>> {
        if matches!(
            self.phase,
            DeclaratorPhase::PointerOrBase
                | DeclaratorPhase::Base
                | DeclaratorPhase::PointerQualifiers
                | DeclaratorPhase::Suffix
        ) && let Some(token) = token
            && let TokenType::Keyword(keyword) = token.kind
            && (super::msvc::calling_convention(keyword)
                || matches!(
                    self.phase,
                    DeclaratorPhase::Suffix
                        | DeclaratorPhase::Base
                        | DeclaratorPhase::PointerOrBase
                ) && super::msvc::type_modifier(keyword))
        {
            self.direct_declarators
                .push(DirectDeclarator::MsModifier(keyword, token.source_vectors));
            parser.merge_source(&mut self.source_vectors, token);
            return Some(ParseAction::Consume);
        }
        if matches!(
            self.phase,
            DeclaratorPhase::PointerOrBase | DeclaratorPhase::Base | DeclaratorPhase::Array
        ) && parser.attribute_starter(token)
        {
            self.attribute_resume = self.phase;
            self.phase = DeclaratorPhase::AwaitAttributes;
            return Some(ParseAction::Push(ParseFrame::Modern(ModernFrame::new(
                parser.arena,
                ModernKind::Attributes,
                parser.hard_error_count,
            ))));
        }
        match self.phase {
            | DeclaratorPhase::AwaitAsm => {
                let Some(ParseValue::Gnu(super::gnu::GnuValue::Asm(asm))) = returned else {
                    panic!("asm label child protocol");
                };
                self.direct_declarators
                    .push(DirectDeclarator::AsmLabel(asm));
                self.source_vectors = Some(
                    parser
                        .context
                        .merge_vectors(self.source_vectors.unwrap_or_default(), asm.source_vectors),
                );
                self.phase = DeclaratorPhase::Suffix;
                Some(ParseAction::Continue)
            },
            | DeclaratorPhase::AwaitPointerAttributes | DeclaratorPhase::AwaitAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(attributes))) = returned else {
                    panic!("declarator attributes protocol: {returned:?}")
                };
                self.direct_declarators
                    .push(DirectDeclarator::Attributes(attributes));
                self.source_vectors = Some(parser.context.merge_vectors(
                    self.source_vectors.unwrap_or_default(),
                    attributes.source_vectors,
                ));
                self.phase = if matches!(self.phase, DeclaratorPhase::AwaitPointerAttributes) {
                    DeclaratorPhase::PointerQualifiers
                } else {
                    self.attribute_resume
                };
                Some(ParseAction::Continue)
            },
            | _ => None,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        if let Some(action) = self.step_extension(parser, token, returned) {
            return action;
        }
        match self.phase {
            | DeclaratorPhase::AwaitAsm
            | DeclaratorPhase::AwaitPointerAttributes
            | DeclaratorPhase::AwaitAttributes => unreachable!("extension phases handled above"),
            | DeclaratorPhase::PointerOrBase => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.merge_source(&mut self.source_vectors, token);
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
                if parser.attribute_starter(token) {
                    self.phase = DeclaratorPhase::AwaitPointerAttributes;
                    return ParseAction::Push(ParseFrame::Modern(ModernFrame::new(
                        parser.arena,
                        ModernKind::Attributes,
                        parser.hard_error_count,
                    )));
                }
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if let Some(token) = token
                    && let Some(qualifier) = type_qualifier(token.kind)
                {
                    // C99 §6.7.3p4: a repeated qualifier is harmless; warn.
                    if self.mode != DeclaratorMode::Abstract
                        && self.current_qualifiers.contains(qualifier)
                    {
                        report_duplicate_type_qualifier(parser, token, qualifier);
                    }
                    self.current_qualifiers.insert(qualifier);
                    parser.merge_source(&mut self.source_vectors, token);
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
                    parser.merge_source(&mut self.source_vectors, token);
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
                    parser.merge_source(&mut self.source_vectors, token);
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
                    parser.merge_source(&mut self.source_vectors, token);
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
                // C99 §6.7.5p1: a direct-declarator starts with an identifier
                // or `(`.
                if self.mode == DeclaratorMode::Named {
                    parser.report(
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
                ParseAction::Push(ParseFrame::Declarator(self.nested_frame(parser)))
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
                        .push(Self::empty_function(parser, true));
                    self.has_direct_declarator = true;
                    parser.merge_source(&mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Ellipsis)
                    || token.is_some_and(|token| parser.declaration_starter(token) && !matches!(token.kind, TokenType::Keyword(k) if super::msvc::calling_convention(k) || super::msvc::type_modifier(k))) {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    Self::push_parameter_list(parser, false)
                } else {
                    self.phase = DeclaratorPhase::AwaitNested;
                    ParseAction::Push(ParseFrame::Declarator(self.nested_frame(parser)))
                }
            },
            | DeclaratorPhase::AwaitNested => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("nested declarator returned an unexpected value: {returned:?}");
                };
                let Some(declarator) = declarator else {
                    parser.report(
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
                        parser.context.merge_vectors(existing, source_vectors)
                    }));
                self.nested = Some(ParenthesizedDeclarator {
                    declarator,
                    delimiters: self.nested_open.take().unwrap_or_default(),
                });
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
                self.close_nested(parser, token)
            },
            | DeclaratorPhase::Suffix => {
                if token.is_some_and(|x| x.kind == TokenType::Keyword(KeywordTokenType::Asm)) {
                    self.phase = DeclaratorPhase::AwaitAsm;
                    return super::gnu::GnuFrame::push(
                        parser,
                        super::gnu::GnuKind::Asm { label: true },
                    );
                }
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if parser.attribute_starter(token) {
                    self.attribute_resume = DeclaratorPhase::Suffix;
                    self.phase = DeclaratorPhase::AwaitAttributes;
                    return ParseAction::Push(ParseFrame::Modern(ModernFrame::new(
                        parser.arena,
                        ModernKind::Attributes,
                        parser.hard_error_count,
                    )));
                }
                // Direct-declarator suffixes repeat left-to-right.
                // Re-enter this phase after
                // every array or function child to preserve
                // binding order in the arena slice.
                if is_operator(token, OperatorTokenType::OpeningSquareBracket) {
                    let token = token.expect("opening-square-bracket token exists");
                    parser.merge_source(&mut self.source_vectors, token);
                    self.array_qualifiers = TypeQualifiers::empty();
                    self.array_qualifiers_before_static = false;
                    self.array_is_static = false;
                    self.array_is_pointer = false;
                    self.array_assignment_expression = None;
                    self.phase = DeclaratorPhase::Array;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                    let token = token.expect("opening-parenthesis token exists");
                    parser.merge_source(&mut self.source_vectors, token);
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
                        report_duplicate_type_qualifier(parser, token, qualifier);
                    }
                    if self.array_qualifiers.is_empty() {
                        parser.c99_syntax_extension("qualified array parameter", token);
                    }
                    // C99 §6.7.5p1: qualifiers go before or after `static`,
                    // not both.
                    if self.array_is_static && self.array_qualifiers_before_static {
                        parser.report(
                    ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                    Some(token),
                );
                    }
                    if !self.array_is_static {
                        self.array_qualifiers_before_static = true;
                    }
                    self.array_qualifiers.insert(qualifier);
                    parser.merge_source(&mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if token
                    .is_some_and(|token| token.kind == TokenType::Keyword(KeywordTokenType::Static))
                {
                    let token = token.expect("static token exists");
                    parser.c99_syntax_extension("static array parameter", token);
                    if self.array_is_static {
                        parser.report(ParserErrorType::StaticSpecifiedTwice, Some(token));
                    }
                    self.array_is_static = true;
                    parser.merge_source(&mut self.source_vectors, token);
                    return ParseAction::Consume;
                }
                if is_array_pointer_marker(parser, token) {
                    let token = token.expect("asterisk token exists");
                    parser.c99_syntax_extension("variable length array marker", token);
                    // C99 §6.7.6p1: an abstract declarator's `[ * ]` takes no
                    // qualifiers, whichever suffixes or grouping precede it.
                    if self.mode != DeclaratorMode::Named
                        && !self.named
                        && !self.array_qualifiers.is_empty()
                    {
                        parser.report(
                    ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
                    Some(token),
                );
                    }
                    // C99 §6.7.5p1: the `[*]` form takes no `static`.
                    if self.array_is_static {
                        parser.report(
                            ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator,
                            Some(token),
                        );
                    }
                    self.array_is_pointer = true;
                    parser.merge_source(&mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::ArrayExpectClose;
                    return ParseAction::Consume;
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    // C99 §6.7.5p1: both `static` forms require a bound.
                    if self.array_is_static {
                        parser.report(
                    ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
                    Some(token),
                );
                    }
                    self.push_array();
                    parser.merge_source(&mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    return ParseAction::Consume;
                }
                if token.is_none() {
                    parser.report(
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None),
                        None,
                    );
                    self.push_array();
                    self.phase = DeclaratorPhase::Finish;
                    return ParseAction::Reprocess;
                }
                self.phase = DeclaratorPhase::AwaitArrayBound;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
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
                    parser.merge_source(&mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Asterisk) {
                    let token = token.expect("asterisk token exists");
                    parser.report(ParserErrorType::PointerSpecifiedTwice, Some(token));
                    parser.merge_source(&mut self.source_vectors, token);
                    ParseAction::Consume
                } else if let Some(token) = token {
                    parser.report(
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
                    let ParseValue::Expression(ExpressionResult {
                        expression: index, ..
                    }) = returned
                    else {
                        panic!("array-bound frame returned an unexpected value: {returned:?}");
                    };
                    let source_vectors = index.source_vectors;
                    check_array_extension(parser, index);
                    self.array_assignment_expression = Some(index);
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            parser.context.merge_vectors(existing, source_vectors)
                        }));
                }
                if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                    let token = token.expect("closing-square-bracket token exists");
                    self.push_array();
                    parser.merge_source(&mut self.source_vectors, token);
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
                    let direct = Self::empty_function(parser, allow_k_and_r);
                    self.direct_declarators.push(direct);
                    self.has_direct_declarator = true;
                    let token = token.expect("closing-parenthesis token exists");
                    parser.merge_source(&mut self.source_vectors, token);
                    self.phase = DeclaratorPhase::Suffix;
                    ParseAction::Consume
                } else if token.is_none() {
                    parser.report(
                        ParserErrorType::UnexpectedEndOfFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = DeclaratorPhase::Finish;
                    ParseAction::Reprocess
                } else {
                    self.phase = DeclaratorPhase::AwaitParameterList;
                    Self::push_parameter_list(parser, allow_k_and_r)
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
                        parser.context.merge_vectors(existing, source_vectors)
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
                    parser.report(ParserErrorType::TypeQualifiersWithoutDeclarator, token);
                    return ParseAction::Reduce(ParseValue::Declarator(None));
                }

                // Commit both flat component lists atomically before
                // returning the value that
                // references their stable slices.
                let pointer_start = parser.alloc_syntax_list(&mut self.pointer_qualifiers);
                let direct_start = parser.alloc_syntax_list(&mut self.direct_declarators);
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

    /// Pushes a parameter-list child in a pooled box.
    fn push_parameter_list(
        parser: &mut Parser<'_, 'tu, 'p>,
        allow_k_and_r: bool,
    ) -> ParseAction<'tu, 'p> {
        let frame = ParameterListFrame::new(parser.arena, allow_k_and_r);
        ParseAction::Push(ParseFrame::ParameterList(
            parser.pools.parameter_list(frame),
        ))
    }

    /// Completes the parenthesized declarator waiting for its `)` and stores
    /// it, with the `)` among its delimiters when present.
    ///
    /// C99: parenthesized direct-declarator is §6.7.5, p. 114; PDF p. 126;
    /// it binds as the unparenthesized declarator, paragraph 6, p. 115;
    /// PDF p. 127.
    fn close_nested(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
    ) -> ParseAction<'tu, 'p> {
        self.phase = DeclaratorPhase::Suffix;
        let mut nested = self
            .nested
            .take()
            .expect("a parenthesized declarator waits for its closing parenthesis");
        let closing =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::ClosingParenthesis));
        if let Some(token) = closing {
            nested.delimiters = parser
                .context
                .merge_vectors(nested.delimiters, token.source_vectors);
        }
        let parenthesized = parser.alloc_syntax(nested);
        self.direct_declarators
            .push(DirectDeclarator::Parenthesized(parenthesized));
        if let Some(token) = closing {
            parser.merge_source(&mut self.source_vectors, token);
            ParseAction::Consume
        } else {
            parser.report(
                ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(
                    token.map(|token| token.kind),
                ),
                token,
            );
            ParseAction::Reprocess
        }
    }

    /// Creates the declarator nested inside this one's `(`, sharing the
    /// chain flag through which it reports an identifier. Abstract
    /// declarators never declare one, so they take no flag.
    fn nested_frame(&mut self, parser: &mut Parser<'_, 'tu, 'p>) -> Self {
        let mut nested = Self::new(parser.arena, self.mode);
        if self.mode != DeclaratorMode::Abstract {
            let chain_named = *self.chain_named.get_or_insert_with(|| {
                self.owns_chain_named = true;
                parser.pools.take_chain_flag()
            });
            nested.chain_named = Some(chain_named);
        }
        nested
    }

    /// Returns this popped frame's lists and, if it began its chain, the
    /// chain flag. Every nested declarator of the chain has popped by then.
    pub(super) fn reclaim_pooled(&mut self, pools: &mut FramePools<'tu, 'p>) {
        pools
            .pointer_qualifiers
            .reclaim(&mut self.pointer_qualifiers);
        pools
            .direct_declarators
            .reclaim(&mut self.direct_declarators);
        if self.owns_chain_named
            && let Some(chain_named) = self.chain_named.take()
        {
            self.owns_chain_named = false;
            pools.reclaim_chain_flag(chain_named);
        }
    }
}
fn check_array_extension<'tu>(parser: &mut Parser<'_, 'tu, '_>, index: &'tu Expression<'tu>) {
    let mut operand = index;
    while let super::syntax::ExpressionType::Parenthesized { expression, .. } = operand.kind {
        operand = expression;
    }
    if let super::syntax::ExpressionType::Constant(super::syntax::Constant::Integer(value)) =
        operand.kind
        && i128::from(value) == 0
    {
        parser.extension_source(
            crate::configuration::Feature::ZeroLengthArrays,
            "zero-length array",
            index.source_vectors,
        );
    }
}
