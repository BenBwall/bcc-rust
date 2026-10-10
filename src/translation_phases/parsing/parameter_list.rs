//! Parameter-type-list and identifier-list frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of the
//! contents of a function declarator's parentheses: `parameter-type-list`,
//! `parameter-list`, `parameter-declaration`, and `identifier-list` (C99:
//! §6.7.5 paragraph 1, p. 114; PDF p. 126; §A.2.2, pp. 413-414;
//! PDF pp. 425-426), with the function-declarator rules of §6.7.5.3,
//! pp. 118-121; PDF pp. 130-133.
//!
//! The list opens function prototype scope (§6.2.1 paragraph 4, p. 30;
//! PDF p. 42); every list keeps the bindings a later definition body
//! needs. An identifier that could be a typedef name or a parameter name is
//! taken as a typedef name (§6.7.5.3 paragraph 11, p. 119; PDF p. 131).
//! Left to semantic analysis: the storage-class constraint of paragraph 2
//! and the empty-list rule of paragraph 3, p. 118; PDF p. 130; the `(void)`
//! special case of paragraph 10 and the adjustments of paragraphs 7-8,
//! p. 119; PDF p. 131. Empty and identifier-list forms are obsolescent
//! (§6.11.6-§6.11.7, p. 163; PDF p. 175) but accepted. Parameters accumulate
//! in arena storage, so the 127-parameter minimum (§5.2.4.1, p. 21;
//! PDF p. 33) imposes no fixed ceiling.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_specifiers::DeclarationSpecifiersFrame,
    declaration_syntax::{
        DeclarationSpecifiers,
        Declarator,
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
        list_key,
    },
    syntax::Identifier,
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

/// Parses either a prototype parameter list or an allowed K&R identifier list
/// and owns the suffix's closing parenthesis.
///
/// C99: parameter-type-list, parameter-list, parameter-declaration, and
/// identifier-list are §6.7.5, p. 114; PDF p. 126; function declarator rules
/// are §6.7.5.3, pp. 118-121; PDF pp. 130-133. Only a named declarator may
/// take an identifier list; abstract ones take `parameter-type-list?`
/// (§6.7.6 paragraph 1, p. 122; PDF p. 134).
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The flags are independent recovery facts of one parameter list, not a hidden state \
              machine."
)]
pub(super) struct ParameterListFrame<'tu, 'p> {
    /// Current prototype/K&R transition.
    phase: ParameterListPhase,
    /// Whether this syntactic position permits a K&R identifier list.
    allow_k_and_r: bool,
    /// Prototype parameters accumulated before arena insertion.
    pub(super) parameters: ArenaVec<'p, ParameterDeclaration<'tu>>,
    /// K&R identifiers accumulated before arena insertion.
    pub(super) identifiers: ArenaVec<'p, Identifier>,
    /// Specifiers retained while an optional parameter declarator runs.
    pending_specifiers: Option<DeclarationSpecifiers<'tu>>,
    /// Specifier provenance retained for parameter-source construction.
    pending_source: Option<SourceVectors>,
    /// Whether `...` terminated the prototype parameter list.
    is_variadic: bool,
    /// Whether variadic recovery may unwind at a later declaration starter.
    can_unwind_variadic_recovery: bool,
    /// Provenance accumulated across the entire parenthesized suffix.
    pub(super) source_vectors: ArenaVec<'p, SourceVectors>,
    /// Scope depth restored by every parameter-list exit.
    entry_scope_depth: Option<usize>,
    /// Prototype-scope bindings introduced by parameter declarator names.
    /// Any further binding came from an enum specifier somewhere in the
    /// parameter declarations.
    parameter_name_bindings: usize,
    /// Whether a prototype parameter inside this identifier list was already
    /// diagnosed, so a trailing `...` is part of the same mistake.
    diagnosed_mixed_parameter: bool,
    /// `__extension__` suppression depth on entry. Every separator and exit
    /// restores it, so a marker covers only the parameter it begins (GNU
    /// extension; C99 §5.1.1.3, p. 11; PDF p. 23).
    suppression_entry: usize,
}

/// State transitions for prototype and K&R parameter-list forms.
///
/// C99: §6.7.5 and §6.7.5.3, pp. 114 and 118-121; PDF pp. 126 and 130-133.
#[derive(Debug, Clone, Copy)]
enum ParameterListPhase {
    /// Enter prototype scope and select K&R versus prototype syntax.
    ///
    /// C99: a visible typedef name selects a parameter declaration
    /// (§6.7.5.3 paragraph 11, p. 119; PDF p. 131).
    Start,
    /// Consume one K&R parameter identifier.
    KAndRIdentifier,
    /// Receive the specifiers of a prototype parameter that was diagnosed
    /// inside an identifier list and is parsed only to be skipped whole.
    KAndRMixedSpecifiers,
    /// Receive the optional declarator of that skipped prototype parameter.
    KAndRMixedDeclarator,
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
    ///
    /// C99: `parameter-list , ...` (§6.7.5 paragraph 1, p. 114; PDF p. 126;
    /// §6.7.5.3 paragraph 9, p. 119; PDF p. 131).
    AfterComma,
    /// Require `)` immediately after `...`.
    ExpectCloseAfterEllipsis,
    /// Leave scope and return a K&R function suffix.
    FinishKAndR,
    /// Leave scope and return a prototype function suffix.
    FinishPrototype,
}

impl<'tu, 'p> ParameterListFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump, allow_k_and_r: bool) -> Self {
        Self {
            phase: ParameterListPhase::Start,
            allow_k_and_r,
            parameters: ArenaVec::new_in(arena),
            identifiers: ArenaVec::new_in(arena),
            pending_specifiers: None,
            pending_source: None,
            is_variadic: false,
            can_unwind_variadic_recovery: false,
            source_vectors: ArenaVec::new_in(arena),
            entry_scope_depth: None,
            parameter_name_bindings: 0,
            diagnosed_mixed_parameter: false,
            suppression_entry: 0,
        }
    }

    /// Reports whether a parameter list that starts with a non-typedef
    /// identifier is really a prototype whose first type name is unknown.
    ///
    /// An identifier list contains only identifiers and commas (C99 §6.7.5,
    /// p. 114; PDF p. 126). An identifier followed by `*` or a
    /// declaration-specifier keyword, as in `(size_t *p)`, can only start a
    /// parameter declaration. After two adjacent identifiers, as in
    /// `(size_t n, int m)`, a bounded scan of the rest of the list looks for
    /// declaration syntax; without it, `(a b, c)` stays an identifier list with
    /// an omitted comma.
    fn unknown_type_name_starts_prototype(parser: &mut Parser<'_, 'tu, 'p>) -> bool {
        /// Tokens examined after two adjacent identifiers before the list is
        /// assumed to be an identifier list.
        const SCAN_LIMIT: usize = 64;
        let Some(following) = parser.cursor.following() else {
            return false;
        };
        match following.kind {
            | TokenType::Operator(OperatorTokenType::Asterisk) => true,
            | TokenType::Identifier => {
                for index in 1..=SCAN_LIMIT {
                    let Some(token) = parser.cursor.lookahead(index) else {
                        return false;
                    };
                    if parser.declaration_starter(token)
                        || matches!(
                            token.kind,
                            TokenType::Operator(
                                OperatorTokenType::Asterisk
                                    | OperatorTokenType::OpeningSquareBracket
                                    | OperatorTokenType::Ellipsis
                            )
                        )
                    {
                        return true;
                    }
                    if !matches!(
                        token.kind,
                        TokenType::Identifier | TokenType::Operator(OperatorTokenType::Comma)
                    ) {
                        return false;
                    }
                }
                false
            },
            | TokenType::Keyword(_) => parser.declaration_starter(following),
            | _ => false,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
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
                self.suppression_entry = parser.pedantic_suppression;
                parser.scopes.enter_scope(ScopeKind::FunctionPrototype);
                // C23: §6.7.7.1 paragraph 1, pp. 126-127; PDF pp. 139-140
                // permits an ellipsis without a preceding parameter-list.
                if is_operator(token, OperatorTokenType::Ellipsis) {
                    let token = token.expect("ellipsis exists");
                    parser.extension(
                        crate::configuration::Feature::C23Keywords,
                        "variadic function without named parameters",
                        token,
                    );
                    self.is_variadic = true;
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::ExpectCloseAfterEllipsis;
                    return ParseAction::Consume;
                }
                if self.allow_k_and_r
                    && token.is_some_and(|token| {
                        matches!(token.kind, TokenType::Identifier)
                            && !parser.scopes.is_typedef(token.contents)
                    })
                    && !Self::unknown_type_name_starts_prototype(parser)
                {
                    // C23: §6.7.7.1 paragraph 1, pp. 126-127; PDF
                    // pp. 139-140 removes identifier-list declarators.
                    // Retain the legacy shape for recovery, but never
                    // present it as native C23 grammar.
                    if !parser
                        .context
                        .configuration
                        .accepts(crate::configuration::Feature::OldStyleFunctionDeclarators)
                    {
                        parser.extension_diagnostic(
                            crate::configuration::Feature::OldStyleFunctionDeclarators,
                            ParserErrorType::ExpectedIsoSyntax(
                                "a prototype parameter list in C23",
                                token.map(|x| x.kind),
                            ),
                            token,
                        );
                    }
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
                        ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                            None,
                        ),
                        None,
                    );
                    self.phase = ParameterListPhase::FinishKAndR;
                    return ParseAction::Reprocess;
                };
                // Once identifier-list syntax is selected, a
                // declaration starter cannot silently switch dialects
                // mid-list (C99 §6.7.5p1: an identifier-list holds only
                // identifiers and commas). Diagnose the list once, then
                // parse each such parameter declaration whole and discard
                // it, so its declarator name is not mistaken for the next
                // identifier.
                if parser.declaration_starter(token) {
                    if !self.diagnosed_mixed_parameter {
                        parser.report(
                            ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator,
                            Some(token),
                        );
                    }
                    self.diagnosed_mixed_parameter = true;
                    self.phase = ParameterListPhase::KAndRMixedSpecifiers;
                    return ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                        DeclarationSpecifiersFrame::parameter(),
                    ));
                }
                // After a diagnosed prototype parameter, a trailing `...`
                // belongs to the same prototype-style list; reporting it
                // again would only repeat that diagnosis.
                if self.diagnosed_mixed_parameter
                    && is_operator(Some(token), OperatorTokenType::Ellipsis)
                {
                    self.source_vectors.push(token.source_vectors);
                    self.phase = ParameterListPhase::KAndRSeparator;
                    return ParseAction::Consume;
                }
                // Typedef names are declaration starters, handled above.
                if !matches!(token.kind, TokenType::Identifier) {
                    parser.report(
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
            | ParameterListPhase::KAndRMixedSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("parameter specifiers returned an unexpected value: {returned:?}");
                };
                self.source_vectors.push(specifiers.source_vectors);
                if is_operator(token, OperatorTokenType::Comma)
                    || is_operator(token, OperatorTokenType::ClosingParenthesis)
                {
                    self.phase = ParameterListPhase::KAndRSeparator;
                    ParseAction::Continue
                } else {
                    self.phase = ParameterListPhase::KAndRMixedDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        parser.arena,
                        DeclaratorMode::MaybeAbstract,
                    )))
                }
            },
            | ParameterListPhase::KAndRMixedDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("parameter declarator returned an unexpected value: {returned:?}");
                };
                if let Some(declarator) = declarator {
                    self.source_vectors.push(declarator.source_vectors);
                }
                self.phase = ParameterListPhase::KAndRSeparator;
                ParseAction::Continue
            },
            | ParameterListPhase::KAndRSeparator => {
                parser.pedantic_suppression = self.suppression_entry;
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
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::FinishKAndR;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| {
                    matches!(token.kind, TokenType::Identifier)
                        && !parser.scopes.is_typedef(token.contents)
                }) {
                    // Repair an omitted comma without discarding the
                    // next parameter name:
                    // diagnose, then reprocess it as an item.
                    parser.report(
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::KAndRIdentifier;
                    ParseAction::Reprocess
                } else {
                    parser.report(
                ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    // A declarator punctuator, as in `T *r`, means a
                    // parameter declaration was written in the identifier
                    // list: skip the rest of the item instead of stopping at
                    // its name and reporting that again. Other stray tokens
                    // keep the next name as a parameter.
                    let declarator_follows = token.is_some_and(|token| {
                        matches!(
                            token.kind,
                            TokenType::Operator(
                                OperatorTokenType::Asterisk
                                    | OperatorTokenType::OpeningSquareBracket
                                    | OperatorTokenType::OpeningParenthesis
                            )
                        )
                    });
                    ParseAction::Recover(SynchronizationSet {
                        kind:   if declarator_follows {
                            SynchronizationKind::Parameter
                        } else {
                            SynchronizationKind::KAndRParameter
                        },
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
                    DeclarationSpecifiersFrame::parameter(),
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
                        parser.arena,
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
                // C99 §6.2.1p4 and p7.
                if let Some(identifier) = declarator.and_then(Declarator::identifier)
                    && parser
                        .scopes
                        .publish_reporting_new(identifier.name, NameClass::Ordinary)
                {
                    self.parameter_name_bindings += 1;
                }
                let parameter_source = match (self.pending_source.take(), declarator_source) {
                    | (Some(specifiers), Some(declarator)) =>
                        parser.context.merge_vectors(specifiers, declarator),
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
                parser.pedantic_suppression = self.suppression_entry;
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
                } else if token.is_none()
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    parser.report(
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    // Preserve a plausible next parameter after an
                    // omitted comma instead
                    // of consuming it during recovery.
                    parser.report(
                ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                    token.map(|token| token.kind),
                ),
                token,
            );
                    self.phase = ParameterListPhase::PrototypeParameter;
                    ParseAction::Reprocess
                } else {
                    parser.report(
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
                } else if let Some(token) = token
                    && (is_operator(Some(token), OperatorTokenType::Semicolon)
                        || is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace)
                        || self.can_unwind_variadic_recovery && parser.declaration_starter(token))
                {
                    parser.report(
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    token.kind,
                ),
                Some(token),
            );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                } else if let Some(token) = token {
                    parser.report(
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
                    parser.report(
                        ParserErrorType::UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
                        None,
                    );
                    self.phase = ParameterListPhase::FinishPrototype;
                    ParseAction::Reprocess
                }
            },
            | ParameterListPhase::FinishKAndR => {
                parser.pedantic_suppression = self.suppression_entry;
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
                let start = parser.alloc_syntax_list(&mut self.identifiers);
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::KAndRStyleFunction { parameters: start },
                    source_vectors:    parser.context.merge_vector_list(&self.source_vectors),
                }))
            },
            | ParameterListPhase::FinishPrototype => {
                parser.pedantic_suppression = self.suppression_entry;
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
                // Mirror the K&R exit: never leak prototype bindings
                // into the enclosing scope. A list at any nesting depth
                // may still belong to a function definition, whose body
                // must see every name declared in its parameter declarations
                // (C99 §6.2.1p4). Names a definition can rebuild from its
                // parameter declarators need no copy; anything else, such as
                // an enumerator declared in an array bound, is retained.
                let entry_scope_depth = self
                    .entry_scope_depth
                    .expect("parameter list entered prototype scope");
                let start = parser.alloc_syntax_list(&mut self.parameters);
                if parser.scopes.innermost_binding_count() > self.parameter_name_bindings {
                    parser.scopes.retain_innermost_bindings(list_key(&start));
                }
                parser.scopes.restore_depth(entry_scope_depth);
                ParseAction::Reduce(ParseValue::ParameterList(ParameterListResult {
                    direct_declarator: DirectDeclarator::Function {
                        parameter_list: start,
                        is_variadic:    self.is_variadic,
                    },
                    source_vectors:    parser.context.merge_vector_list(&self.source_vectors),
                }))
            },
        }
    }
}
