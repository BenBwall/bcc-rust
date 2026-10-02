//! Parameter-type-list and identifier-list frame.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_specifiers::{
        DeclarationSpecifiersFrame,
        SpecifierMode,
    },
    declaration_syntax::{
        DeclarationSpecifiers,
        DirectDeclarator,
        ParameterDeclaration,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
    },
    errors::ParserErrorType,
    expression_operators::is_operator,
    machine::{
        ParameterListResult,
        ParseAction,
        ParseFrame,
        ParseFrameKind,
        ParseValue,
    },
    recovery::{
        SynchronizationKind,
        SynchronizationSet,
    },
    scope::{
        NameClass,
        ScopeKind,
    },
    syntax::{
        Identifier,
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

/// Parses either a prototype parameter list or an allowed K&R identifier list
/// and owns the suffix's closing parenthesis.
///
/// C99: parameter-type-list, parameter-list, parameter-declaration, and
/// identifier-list are §6.7.5, p. 114; PDF p. 126; function declarator rules
/// are §6.7.5.3, pp. 118-121; PDF pp. 130-133.
#[derive(Debug)]
pub(super) struct ParameterListFrame {
    /// Current prototype/K&R transition.
    phase: ParameterListPhase,
    /// Whether this syntactic position permits a K&R identifier list.
    allow_k_and_r: bool,
    /// Prototype parameters accumulated before arena insertion.
    pub(super) parameters: Vec<ParameterDeclaration>,
    /// K&R identifiers accumulated before arena insertion.
    pub(super) identifiers: Vec<Identifier>,
    /// Specifiers retained while an optional parameter declarator runs.
    pending_specifiers: Option<DeclarationSpecifiers>,
    /// Specifier provenance retained for parameter-source construction.
    pending_source: Option<SourceVectors>,
    /// Whether `...` terminated the prototype parameter list.
    is_variadic: bool,
    /// Whether variadic recovery may unwind at a later declaration starter.
    can_unwind_variadic_recovery: bool,
    /// Provenance accumulated across the entire parenthesized suffix.
    pub(super) source_vectors: Vec<SourceVectors>,
    /// Scope depth restored by every parameter-list exit.
    entry_scope_depth: Option<usize>,
}

/// State transitions for prototype and K&R parameter-list forms.
///
/// C99: §6.7.5 and §6.7.5.3, pp. 114 and 118-121; PDF pp. 126 and 130-133.
#[derive(Debug, Clone, Copy)]
pub(super) enum ParameterListPhase {
    /// Enter prototype scope and select K&R versus prototype syntax.
    Start,
    /// Consume one K&R parameter identifier.
    KAndRIdentifier,
    /// Require `,` or `)` after a K&R identifier.
    KAndRSeparator,
    /// Push declaration specifiers for one prototype parameter.
    PrototypeParameter,
    /// Receive parameter specifiers and decide whether a declarator follows.
    AwaitSpecifiers,
    /// Receive the optional named or abstract parameter declarator.
    AwaitDeclarator,
    /// Require `,` or `)` after a prototype parameter.
    PrototypeSeparator,
    /// Parse `...` or the next parameter after a comma.
    AfterComma,
    /// Require `)` immediately after `...`.
    ExpectCloseAfterEllipsis,
    /// Leave scope and return a K&R function suffix.
    FinishKAndR,
    /// Leave scope and return a prototype function suffix.
    FinishPrototype,
}

impl ParameterListFrame {
    pub(super) fn new(allow_k_and_r: bool) -> Self {
        Self {
            phase: ParameterListPhase::Start,
            allow_k_and_r,
            parameters: Vec::new(),
            identifiers: Vec::new(),
            pending_specifiers: None,
            pending_source: None,
            is_variadic: false,
            can_unwind_variadic_recovery: false,
            source_vectors: Vec::new(),
            entry_scope_depth: None,
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
            | ParameterListPhase::Start => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Prototype scope starts with the parameter list. A
                // visible typedef forces
                // prototype syntax; only a non-typedef
                // identifier can select the legacy identifier-list
                // branch.
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::FunctionPrototype);
                if self.allow_k_and_r
                    && token.is_some_and(|token| {
                        token.kind == TokenType::Identifier
                            && !parser.scopes.is_typedef(token.contents)
                    })
                {
                    self.phase = ParameterListPhase::KAndRIdentifier;
                } else {
                    self.phase = ParameterListPhase::PrototypeParameter;
                }
                ParseAction::Reprocess
            },
            | ParameterListPhase::KAndRIdentifier => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                let Some(token) = token else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            None,
                        ),
                        None,
                    );
                    self.phase = ParameterListPhase::FinishKAndR;
                    return ParseAction::Reprocess;
                };
                // Once identifier-list syntax is selected, a
                // declaration starter cannot
                // silently switch dialects mid-list.
                if parser.declaration_starter(token) {
                    parser.report(
                        context,
                        ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator,
                        Some(token),
                    );
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    });
                }
                if token.kind != TokenType::Identifier || parser.scopes.is_typedef(token.contents) {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    });
                }
                self.identifiers.push(Identifier::from_token(token));
                self.source_vectors.push(token.source_vectors);
                self.phase = ParameterListPhase::KAndRSeparator;
                ParseAction::Consume
            },
            | ParameterListPhase::KAndRSeparator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = ParameterListPhase::KAndRIdentifier;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    None,
                ),
                None,
            );
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| {
                    token.kind == TokenType::Identifier && !parser.scopes.is_typedef(token.contents)
                }) {
                    // Repair an omitted comma without discarding the
                    // next parameter name:
                    // diagnose, then reprocess it as an item.
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::KAndRIdentifier;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::KAndRParameter,
                        target: ParseFrameKind::ParameterList,
                    })
                }
            },
            | ParameterListPhase::PrototypeParameter => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                self.phase = ParameterListPhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(SpecifierMode::Declaration),
                ))
            },
            | ParameterListPhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("parameter specifiers returned an unexpected value: {returned:?}");
                };
                self.pending_specifiers = Some(specifiers);
                self.pending_source = Some(specifiers.source_vectors);
                // parameter-declaration permits an absent abstract
                // declarator, so a separator
                // can complete the parameter immediately.
                if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.parameters.push(ParameterDeclaration {
                        declaration_specifiers: specifiers,
                        declarator:             None,
                        source_vectors:         specifiers.source_vectors,
                    });
                    self.source_vectors.push(specifiers.source_vectors);
                    self.pending_source = None;
                    self.phase = ParameterListPhase::PrototypeSeparator;
                    ParseAction::Continue
                } else {
                    self.phase = ParameterListPhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::MaybeAbstract,
                    )))
                }
            },
            | ParameterListPhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("parameter declarator returned an unexpected value: {returned:?}");
                };
                let declarator_source = declarator.map(|declarator| declarator.source_vectors);
                // Parameter names enter prototype scope as soon as
                // their declarator completes
                // and may hide typedefs in later entries.
                if let Some(identifier) =
                    declarator.and_then(|declarator| parser.declarator_identifier(declarator))
                {
                    parser.scopes.publish(identifier.name, NameClass::Ordinary);
                }
                let parameter_source = match (self.pending_source.take(), declarator_source) {
                    | (Some(specifiers), Some(declarator)) =>
                        context.merge_vectors(specifiers, declarator),
                    | (Some(specifiers), None) => specifiers,
                    | (None, Some(declarator)) => declarator,
                    | (None, None) => SourceVectors::default(),
                };
                self.parameters.push(ParameterDeclaration {
                    declaration_specifiers: self
                        .pending_specifiers
                        .take()
                        .expect("parameter declarator follows specifiers"),
                    declarator,
                    source_vectors: parameter_source,
                });
                self.source_vectors.push(parameter_source);
                self.phase = ParameterListPhase::PrototypeSeparator;
                ParseAction::Continue
            },
            | ParameterListPhase::PrototypeSeparator => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Comma) {
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = ParameterListPhase::AfterComma;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    None,
                ),
                None,
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    // Preserve a plausible next parameter after an
                    // omitted comma instead
                    // of consuming it during recovery.
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::PrototypeParameter;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                context,
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    ParseAction::Recover(SynchronizationSet {
                        kind:   SynchronizationKind::Parameter,
                        target: ParseFrameKind::ParameterList,
                    })
                }
            },
            | ParameterListPhase::AfterComma => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // `, ...` terminates parameter-type-list. A closing
                // delimiter here means the
                // comma had no following parameter, which gets
                // its own diagnostic rather than a specifier cascade.
                if is_operator(token, OperatorTokenType::Ellipsis) {
                    self.is_variadic = true;
                    if let Some(token) = token {
                        self.source_vectors.push(token.source_vectors);
                    }
                    self.phase = ParameterListPhase::ExpectCloseAfterEllipsis;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    parser.report(
                        context,
                        ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                    || token.is_none()
                {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else {
                    self.phase = ParameterListPhase::PrototypeParameter;
                    ParseAction::Reprocess
                }
            },
            | ParameterListPhase::ExpectCloseAfterEllipsis => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Ellipsis is terminal in the grammar, so the owning
                // `)` is the only legal
                // continuation.
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Consume
                } else if is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    let token = token.expect("unwind token exists");
                    parser.report(
                context,
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    token.kind,
                ),
                Some(token),
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if let Some(token) = token
                    && self.can_unwind_variadic_recovery
                    && parser.declaration_starter(token)
                {
                    parser.report(
                context,
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    token.kind,
                ),
                Some(token),
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_none() {
                    parser.report(
                        context,
                        ParserErrorType::UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if let Some(token) = token {
                    parser.report(
                context,
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    token.kind,
                ),
                Some(token),
            );
                    let follows_comma = is_operator(Some(token), OperatorTokenType::Comma);
                    self.can_unwind_variadic_recovery = !follows_comma;
                    ParseAction::Recover(SynchronizationSet {
                        kind:   if follows_comma {
                            SynchronizationKind::VariadicTrailingParameter
                        } else {
                            SynchronizationKind::VariadicParameterList
                        },
                        target: ParseFrameKind::ParameterList,
                    })
                } else {
                    unreachable!("EOF is handled before malformed variadic tokens")
                }
            },
            | ParameterListPhase::FinishKAndR => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Both successful and recovered exits must close
                // prototype scope before the
                // enclosing declarator resumes.
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("parameter list entered prototype scope"),
                );
                let start = parser.syntax.identifiers.len().to_u32();
                parser.append_syntax(|syntax| &mut syntax.identifiers, &mut self.identifiers);
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::KAndRStyleFunction {
                        parameters: SyntaxList::new(
                            parser.syntax_id,
                            start,
                            parser.syntax.identifiers.len().to_u32(),
                        ),
                    },
                    source_vectors:    context.merge_vector_list(&self.source_vectors),
                }))
            },
            | ParameterListPhase::FinishPrototype => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Mirror the K&R exit: never leak prototype bindings
                // into the enclosing file or
                // parameter scope.
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("parameter list entered prototype scope"),
                );
                let start = parser.syntax.parameter_declarations.len().to_u32();
                parser.append_syntax(
                    |syntax| &mut syntax.parameter_declarations,
                    &mut self.parameters,
                );
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::Function {
                        parameter_list: SyntaxList::new(
                            parser.syntax_id,
                            start,
                            parser.syntax.parameter_declarations.len().to_u32(),
                        ),
                        is_variadic:    self.is_variadic,
                    },
                    source_vectors:    context.merge_vector_list(&self.source_vectors),
                }))
            },
        }
    }
}
