//! Recursive enum specifier grammar for the phase-7 study.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `enum-specifier`, `enumerator-list`, and `enumerator` (C99: §6.7.2.2
//! paragraph 1, p. 105; PDF p. 117; §A.2.2, p. 413; PDF p. 425), with the
//! tag forms of §6.7.2.3, pp. 106-107; PDF pp. 118-119.
//!
//! Each enumeration constant enters the ordinary name space as soon as its
//! enumerator ends (§6.2.1 paragraph 7, p. 30; PDF p. 42; §6.2.3
//! paragraph 1, p. 31; PDF p. 43), so it can hide a typedef name in later
//! tokens. Diagnosed here: a malformed list, including an empty one, which
//! the grammar does not allow. Left to semantic analysis: the value
//! constraint and value assignment of §6.7.2.2 paragraphs 2-3, p. 105;
//! PDF p. 117, the compatible integer type of paragraph 4, p. 105;
//! PDF p. 117 (implementation-defined), and the tag constraints of §6.7.2.3
//! paragraphs 1-3, p. 106; PDF p. 118. The underlying type's specifiers
//! recurse directly; values and attributes run through their own families.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                EnumSpecifier,
                Enumerator,
                TypeName,
            },
            errors::ParserErrorType,
            extensions::modern::{
                SpecifierExtension,
                SpecifierExtensionKind,
            },
            grammar::{
                ProductionKind,
                declaration_specifiers::SpecifierMode,
                expression_operators::is_operator,
                expressions::{
                    ConstantExpressionResult,
                    ExpressionBoundary,
                },
                modern::{
                    ModernKind,
                    ModernValue,
                },
                statements::is_statement_keyword,
                unexpected_return,
            },
            recovery::{
                SynchronizationKind,
                SynchronizationSet,
            },
            scope::NameClass,
            syntax::{
                ConstantExpression,
                Identifier,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses an enum specifier, including its optional tag, enumerators,
    /// trailing comma, and optional explicit values, starting at `enum`.
    ///
    /// C99: enumeration specifiers and enumerators are §6.7.2.2,
    /// pp. 105-106; PDF pp. 117-118; tags are §6.7.2.3, pp. 106-107;
    /// PDF pp. 118-119.
    pub(in crate::translation_phases::parsing) fn rd_enum_specifier(
        &mut self,
    ) -> Option<EnumSpecifierResult<'tu>> {
        let entry = self.enter_production(ProductionKind::EnumSpecifier)?;
        let mut enumeration = Enumeration::new(self);
        let result = self
            .rd_enumeration(&mut enumeration)
            .map(|()| self.rd_enumeration_finish(&mut enumeration));
        self.pools.enumerators.reclaim(&mut enumeration.enumerators);
        self.pools
            .source_vectors
            .reclaim(&mut enumeration.source_vectors);
        self.leave_production(entry);
        result
    }

    fn rd_enumeration(&mut self, enumeration: &mut Enumeration<'tu, 'p>) -> Option<()> {
        let Some(token) = self.cursor.current() else {
            self.report(ParserErrorType::ExpectedEnumKeyword(None), None);
            return Some(());
        };
        if !matches!(token.kind, TokenType::Keyword(KeywordTokenType::Enum)) {
            self.report(
                ParserErrorType::ExpectedEnumKeyword(Some(token.kind)),
                Some(token),
            );
        }
        enumeration.source_vectors.push(token.source_vectors);
        self.rd_consume();

        // Like tag specifiers for aggregates, an enum can be a tagged
        // reference, tagged definition, or anonymous definition.
        let mut named = false;
        loop {
            let token = self.cursor.current();
            if !named && self.attribute_starter(token) {
                enumeration.attributes = Some(self.rd_enum_attributes(enumeration, false)?);
                continue;
            }
            if is_operator(token, OperatorTokenType::Colon) && underlying_type_follows(self) {
                let token = token.expect("colon exists");
                if named && enumeration.underlying_type.is_some() {
                    self.report(
                        ParserErrorType::ExpectedIsoSyntax(
                            "an enum body or declarator after its underlying type",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                self.extension(
                    Feature::EnumUnderlyingType,
                    "fixed enum underlying type",
                    token,
                );
                enumeration.source_vectors.push(token.source_vectors);
                self.rd_consume();
                self.rd_underlying_type(enumeration)?;
                named = true;
                continue;
            }
            if !named
                && let Some(token) = token
                && matches!(token.kind, TokenType::Identifier)
            {
                enumeration.name = Some(Identifier::from_token(token));
                enumeration.source_vectors.push(token.source_vectors);
                self.rd_consume();
                named = true;
                continue;
            }
            if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                break;
            }
            if !named {
                // C99 §6.7.2.2p1: `enum` takes a tag, a list, or both.
                self.report(
                    ParserErrorType::EnumSpecifierWithoutNameAndBody(token.map(|token| token.kind)),
                    token,
                );
            }
            return Some(());
        }
        let token = self
            .cursor
            .current()
            .expect("opening-curly-brace token exists");
        enumeration.body_started = true;
        enumeration.body_starting_error_count = self.hard_error_count;
        enumeration.source_vectors.push(token.source_vectors);
        self.rd_consume();
        self.rd_enumerator_list(enumeration)?;
        while self.attribute_starter(self.cursor.current()) {
            enumeration.attributes = Some(self.rd_enum_attributes(enumeration, false)?);
        }
        Some(())
    }

    /// Parses the C23 enum-type-specifier after its `:`.
    /// C23: §6.7.3.3 paragraph 1, p. 109; PDF p. 122.
    fn rd_underlying_type(&mut self, enumeration: &mut Enumeration<'tu, 'p>) -> Option<()> {
        enumeration.body_starting_error_count = self.hard_error_count;
        let specifiers = self.rd_declaration_specifiers(SpecifierMode::TypeName, true)?;
        let x = self.alloc_syntax(TypeName {
            declaration_specifiers: specifiers,
            declarator:             None,
            source_vectors:         specifiers.source_vectors,
            recovered:              self.hard_error_count > enumeration.body_starting_error_count,
        });
        // Keep the first specifier when recovering an extra colon.
        // C23: §6.7.3.3 paragraph 1, p. 109; PDF p. 122 permits only one
        // optional enum-type-specifier.
        if enumeration.underlying_type.is_none() {
            enumeration.underlying_type = Some(x);
        }
        enumeration.source_vectors.push(x.source_vectors);
        Some(())
    }

    /// Parses one attribute group of the tag or, with `enumerator`, of the
    /// enumerator in progress, and returns the extended attribute chain.
    fn rd_enum_attributes(
        &mut self,
        enumeration: &mut Enumeration<'tu, 'p>,
        enumerator: bool,
    ) -> Option<&'tu SpecifierExtension<'tu>> {
        let returned = self.rd_modern(ModernKind::Attributes, None, self.hard_error_count)?;
        let ModernValue::Attributes(x) = returned else {
            unexpected_return!("enum attributes protocol: {returned:?}")
        };
        let next = if enumerator {
            enumeration.enumerator_attributes
        } else {
            enumeration.attributes
        };
        let attributes = self.alloc_syntax(SpecifierExtension {
            kind: SpecifierExtensionKind::Attributes(x),
            next,
            source_vectors: x.source_vectors,
        });
        enumeration.source_vectors.push(x.source_vectors);
        if enumerator {
            self.context
                .merge_into(&mut enumeration.current_enumerator_source, x.source_vectors);
        }
        Some(attributes)
    }

    /// Parses enumerators through the body's `}`, which it owns, or stops
    /// before the token where a malformed list ends.
    fn rd_enumerator_list(&mut self, enumeration: &mut Enumeration<'tu, 'p>) -> Option<()> {
        let mut position = ListPosition::EnumeratorOrClose;
        loop {
            let token = self.cursor.current();
            position = match position {
                | ListPosition::EnumeratorOrClose => {
                    // This position is also reached after a comma, which is
                    // why `}` accepts the standard's optional trailing-comma
                    // form. C99 §6.7.2.2p1: an enumerator-list has at least
                    // one enumerator, so `{}` is diagnosed.
                    if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                        let token = token.expect("closing-curly-brace token exists");
                        if enumeration.enumerators.is_empty()
                            && self.hard_error_count == enumeration.body_starting_error_count
                        {
                            self.report(
                                ParserErrorType::ExpectedEnumeratorBeforeClosingCurlyBrace,
                                Some(token),
                            );
                        }
                        enumeration.source_vectors.push(token.source_vectors);
                        self.rd_consume();
                        return Some(());
                    }
                    if let Some(token) = token
                        && matches!(token.kind, TokenType::Identifier)
                    {
                        enumeration.current_enumerator = Some(Identifier::from_token(token));
                        enumeration.current_enumerator_source = Some(token.source_vectors);
                        enumeration.source_vectors.push(token.source_vectors);
                        self.rd_consume();
                        ListPosition::AfterEnumeratorName
                    } else if is_operator(token, OperatorTokenType::Semicolon) || token.is_none() {
                        self.report(
                            ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                        return Some(());
                    } else if let Some(token) = token.filter(|&token| {
                        is_operator(Some(token), OperatorTokenType::ClosingParenthesis)
                    }) {
                        self.report(
                            ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                                Some(token.kind),
                            ),
                            Some(token),
                        );
                        if !enumerator_list_continues(self) {
                            return Some(());
                        }
                        // A stray `)` cannot end the list; its `}` follows.
                        enumeration.source_vectors.push(token.source_vectors);
                        self.rd_consume();
                        ListPosition::EnumeratorOrClose
                    } else if let Some(misplaced) = token
                        && misplaced_enumerator(self, misplaced)
                    {
                        // C99 §6.7.2.2p1: an enumeration constant is an
                        // identifier. A keyword or constant written in its
                        // place is one bad enumerator, not the start of a
                        // declaration that ends this body.
                        self.report(
                            ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                                Some(misplaced.kind),
                            ),
                            token,
                        );
                        enumeration.source_vectors.push(misplaced.source_vectors);
                        self.rd_consume();
                        ListPosition::AfterEnumeratorName
                    } else {
                        self.report(
                            ParserErrorType::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                        enumeration.resuming_after_error = true;
                        self.rd_enum_recover(enumeration);
                        ListPosition::AfterEnumerator
                    }
                },
                | ListPosition::AfterEnumeratorName => {
                    if self.attribute_starter(token) {
                        enumeration.enumerator_attributes =
                            Some(self.rd_enum_attributes(enumeration, true)?);
                        continue;
                    }
                    // The constant-expression is optional. Finalize
                    // immediately unless `=` explicitly transfers ownership
                    // to the value child.
                    if let Some(token) =
                        token.filter(|&token| is_operator(Some(token), OperatorTokenType::Equals))
                    {
                        enumeration.source_vectors.push(token.source_vectors);
                        self.merge_source(&mut enumeration.current_enumerator_source, token);
                        self.rd_consume();
                        self.rd_enumerator_value(enumeration)?;
                    } else {
                        enumeration.finish_enumerator(self, None);
                    }
                    ListPosition::AfterEnumerator
                },
                | ListPosition::AfterEnumerator => self.rd_after_enumerator(enumeration, token),
                | ListPosition::End => return Some(()),
            };
        }
    }

    /// Parses an explicit enumerator value after its `=` and completes the
    /// enumerator.
    fn rd_enumerator_value(&mut self, enumeration: &mut Enumeration<'tu, 'p>) -> Option<()> {
        let ConstantExpressionResult {
            expression: index,
            recovered,
        } = self.rd_constant_expression(ExpressionBoundary::Enumerator)?;
        enumeration.resuming_after_error = recovered;
        let source_vectors = index.expression().source_vectors;
        enumeration.source_vectors.push(source_vectors);
        self.context
            .merge_into(&mut enumeration.current_enumerator_source, source_vectors);
        enumeration.finish_enumerator(self, Some(index));
        Some(())
    }

    /// Requires `,` or `}` after one enumerator and returns the next list
    /// position.
    fn rd_after_enumerator(
        &mut self,
        enumeration: &mut Enumeration<'tu, 'p>,
        token: Option<Token>,
    ) -> ListPosition {
        // A following identifier is a strong omitted-comma recovery point:
        // preserve it and parse it as the next enumerator.
        let resuming_after_error = std::mem::take(&mut enumeration.resuming_after_error);
        if is_operator(token, OperatorTokenType::Comma) {
            let token = token.expect("comma token exists");
            if is_operator(
                self.cursor.following(),
                OperatorTokenType::ClosingCurlyBrace,
            ) {
                self.extension(Feature::TrailingEnumComma, "trailing enum comma", token);
            }
            enumeration.source_vectors.push(token.source_vectors);
            self.rd_consume();
            return ListPosition::EnumeratorOrClose;
        }
        if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
            let token = token.expect("closing-curly-brace token exists");
            enumeration.source_vectors.push(token.source_vectors);
            self.rd_consume();
            return ListPosition::End;
        }
        let error = ParserErrorType::ExpectedCommaOrClosingCurlyInEnumeratorList(
            token.map(|token| token.kind),
        );
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.report(error, token);
            return ListPosition::End;
        }
        if is_operator(token, OperatorTokenType::ClosingParenthesis) {
            let stray = enumerator_list_continues(self);
            if !(stray && resuming_after_error) {
                self.report(error, token);
            }
            if !stray {
                return ListPosition::End;
            }
            // A stray `)` cannot end the list; its `}` follows, so a
            // following `,` or `}` is still the separator.
            let token = token.expect("closing-parenthesis token exists");
            enumeration.source_vectors.push(token.source_vectors);
            self.rd_consume();
            return ListPosition::AfterEnumerator;
        }
        if token.is_some_and(|token| matches!(token.kind, TokenType::Identifier)) {
            self.report(error, token);
            return ListPosition::EnumeratorOrClose;
        }
        if let Some(misplaced) = token
            && misplaced_enumerator(self, misplaced)
        {
            // The `,` is missing before a keyword or constant that stands in
            // for the next enumerator; consume it so it is not diagnosed
            // again as an enumerator.
            self.report(error, token);
            enumeration.source_vectors.push(misplaced.source_vectors);
            self.rd_consume();
            return ListPosition::AfterEnumeratorName;
        }
        if token.is_some_and(|token| self.declaration_starter(token)) {
            self.report(error, token);
            enumeration.stopped_before_declaration = true;
            return ListPosition::End;
        }
        self.report(error, token);
        if token.is_none() {
            return ListPosition::End;
        }
        enumeration.resuming_after_error = true;
        self.rd_enum_recover(enumeration);
        ListPosition::AfterEnumerator
    }

    fn rd_enum_recover(&mut self, enumeration: &mut Enumeration<'tu, 'p>) {
        if let Some(recovered) = self.recover(SynchronizationSet {
            kind:   SynchronizationKind::EnumeratorValue,
            target: ProductionKind::EnumSpecifier,
        }) {
            enumeration.source_vectors.push(recovered);
        }
    }

    fn rd_enumeration_finish(
        &mut self,
        enumeration: &mut Enumeration<'tu, 'p>,
    ) -> EnumSpecifierResult<'tu> {
        let enumeration_list = enumeration
            .body_started
            .then(|| self.alloc_syntax_list(&mut enumeration.enumerators));
        let source_vectors = self.context.merge_vector_list(&enumeration.source_vectors);
        let index = self.alloc_syntax(EnumSpecifier {
            underlying_type: enumeration.underlying_type,
            attributes: enumeration.attributes,
            name: enumeration.name,
            enumeration_list,
            source_vectors,
        });
        EnumSpecifierResult {
            index,
            stopped_before_declaration: enumeration.stopped_before_declaration,
        }
    }
}

/// Where the enumerator list resumes.
///
/// C99: §6.7.2.2 paragraph 1, p. 105; PDF p. 117.
#[derive(Clone, Copy)]
enum ListPosition {
    /// An enumerator name or the body's closing brace.
    EnumeratorOrClose,
    /// Optional attributes and `=` value after an enumerator name.
    AfterEnumeratorName,
    /// The `,` or `}` after one enumerator.
    AfterEnumerator,
    /// The list ended, at its `}` or before the token that ends it.
    End,
}

/// One enum specifier under construction.
///
/// C99: §6.7.2.2, pp. 105-106; PDF pp. 117-118.
struct Enumeration<'tu, 'p> {
    attributes: Option<&'tu SpecifierExtension<'tu>>,
    enumerator_attributes: Option<&'tu SpecifierExtension<'tu>>,
    underlying_type: Option<&'tu TypeName<'tu>>,
    /// Optional enum tag.
    name: Option<Identifier>,
    /// Completed enumerators before arena insertion.
    enumerators: ArenaVec<'p, Enumerator<'tu>>,
    /// Enumerator name waiting for an optional explicit value.
    current_enumerator: Option<Identifier>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    /// Whether malformed-body recovery stopped before an outer declaration.
    stopped_before_declaration: bool,
    /// Hard-error count when `{` was consumed. A body that already reported
    /// a malformed enumerator does not also report that the list is empty.
    body_starting_error_count: usize,
    /// Provenance accumulated across the complete enum specifier.
    source_vectors: ArenaVec<'p, SourceVectors>,
    /// Provenance for the enumerator currently being built.
    current_enumerator_source: Option<SourceVectors>,
    /// Whether the enumerator before the current separator position already
    /// reported an error, so a stray `)` there is not diagnosed again.
    resuming_after_error: bool,
}

impl<'tu, 'p> Enumeration<'tu, 'p> {
    fn new(parser: &mut Parser<'_, 'tu, 'p>) -> Self {
        let mut enumeration = Self {
            attributes: None,
            enumerator_attributes: None,
            underlying_type: None,
            name: None,
            enumerators: ArenaVec::new_in(parser.arena),
            current_enumerator: None,
            body_started: false,
            stopped_before_declaration: false,
            body_starting_error_count: 0,
            source_vectors: ArenaVec::new_in(parser.arena),
            current_enumerator_source: None,
            resuming_after_error: false,
        };
        parser.pools.enumerators.lend(&mut enumeration.enumerators);
        parser
            .pools
            .source_vectors
            .lend(&mut enumeration.source_vectors);
        enumeration
    }

    fn finish_enumerator(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        expression: Option<ConstantExpression<'tu>>,
    ) {
        let source_vectors = self.current_enumerator_source.take().unwrap_or_default();
        if let Some(name) = self.current_enumerator.take() {
            self.enumerators.push(Enumerator {
                attributes: self.enumerator_attributes.take(),
                name,
                expression,
                source_vectors,
            });
            // Enumeration constants join the ordinary identifier namespace as
            // soon as their enumerator completes, affecting later values.
            // C99 §6.2.1p7: the scope begins after the defining enumerator,
            // so `A = A` reads an outer `A`.
            parser.scopes.publish(name.name, NameClass::Ordinary);
        }
    }
}

/// Whether the current `:` opens an enum-type-specifier, whose
/// specifier-qualifier-list must follow it. Any other colon belongs to
/// the enclosing grammar: a generic association or an unnamed bit-field.
/// C23: §6.7.3.3 paragraph 1, p. 109; PDF p. 122.
pub(in crate::translation_phases::parsing) fn underlying_type_follows(
    parser: &Parser<'_, '_, '_>,
) -> bool {
    parser
        .cursor
        .following()
        .is_some_and(|token| parser.type_name_starter(token))
}

/// Whether `token`, which cannot start an enumerator, stands alone in
/// enumerator position: a non-identifier, non-delimiter token followed by
/// `,`, `=`, or `}`. Anything else keeps the malformed-body recovery that
/// stops before a following declaration.
pub(in crate::translation_phases::parsing) fn misplaced_enumerator(
    parser: &mut Parser<'_, '_, '_>,
    token: Token,
) -> bool {
    !matches!(token.kind, TokenType::Identifier)
        && !matches!(
            token.kind,
            TokenType::Operator(
                OperatorTokenType::OpeningParenthesis
                    | OperatorTokenType::OpeningSquareBracket
                    | OperatorTokenType::OpeningCurlyBrace
                    | OperatorTokenType::ClosingParenthesis
                    | OperatorTokenType::ClosingSquareBracket
                    | OperatorTokenType::ClosingCurlyBrace
                    | OperatorTokenType::Comma
                    | OperatorTokenType::Semicolon
                    | OperatorTokenType::Equals
            )
        )
        && parser.cursor.following().is_some_and(|following| {
            matches!(
                following.kind,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Equals
                        | OperatorTokenType::ClosingCurlyBrace
                )
            )
        })
}

/// Whether the body's `}` follows a stray `)` before anything that
/// cannot appear in an enumerator list.
///
/// C99 §6.7.2.2p1: an enumerator list holds only enumerators, `=`,
/// constant expressions, and commas, so a `)` is stray when the list's
/// `}` still follows. Inside an enclosing parenthesis, as in
/// `int f(enum E { A ) int after;`, the `)` closes that parenthesis and
/// the body's `}` is missing. The scan is bounded so repeated errors stay
/// linear.
pub(in crate::translation_phases::parsing) fn enumerator_list_continues(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    const SCAN_LIMIT: usize = 64;
    let mut nesting = 0_u32;
    for index in 0..SCAN_LIMIT {
        let Some(token) = parser.cursor.lookahead(index) else {
            return false;
        };
        match token.kind {
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                if nesting == 0 {
                    return true;
                }
                nesting -= 1;
            },
            | TokenType::Operator(
                OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
                | OperatorTokenType::OpeningCurlyBrace,
            ) => nesting += 1,
            | TokenType::Operator(
                OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
            ) => {
                if nesting == 0 {
                    return false;
                }
                nesting -= 1;
            },
            | TokenType::Operator(OperatorTokenType::Semicolon) => return false,
            | kind if is_statement_keyword(kind)
                || nesting == 0
                    && !matches!(kind, TokenType::Identifier)
                    && parser.declaration_starter(token) =>
            {
                return false;
            },
            | _ => {},
        }
    }
    false
}

/// Enum child result before its type specifier is merged into the parent.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::parsing) struct EnumSpecifierResult<'tu> {
    /// Completed enum specifier.
    pub(super) index: &'tu EnumSpecifier<'tu>,
    /// Whether recovery stopped before a following declaration.
    pub(super) stopped_before_declaration: bool,
}
