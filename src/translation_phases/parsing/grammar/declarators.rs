//! Recursive declarator grammar for concrete and abstract declarators.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `declarator`, `direct-declarator`, `pointer`, and `type-qualifier-list`
//! (C99: §6.7.5 paragraph 1, p. 114; PDF p. 126) with the pointer, array, and
//! function derivations of §6.7.5.1-§6.7.5.3, pp. 115-121; PDF pp. 127-133,
//! and of `abstract-declarator` and `direct-abstract-declarator` (§6.7.6
//! paragraph 1, p. 122; PDF p. 134); summarized in §A.2.2, pp. 413-414;
//! PDF pp. 425-426. Parameter lists belong to `parameter_lists`.
//!
//! Diagnosed here: array suffixes outside the four §6.7.5 paragraph 1 forms
//! and qualifiers before an abstract `[*]` (§6.7.6 paragraph 1). Left to
//! semantic analysis: the array constraints of §6.7.5.2 paragraphs 1-2,
//! p. 116; PDF p. 128 (including where `static` and qualifiers may appear),
//! the return-type constraint of §6.7.5.3 paragraph 1, p. 118; PDF p. 130,
//! and the type each derivation specifies. Pointer levels iterate and
//! parenthesized declarators recurse natively, so the minimums of 12
//! declarators and 63 parenthesized-declarator levels (§5.2.4.1, p. 20;
//! PDF p. 32; §6.7.5 paragraph 7, p. 115; PDF p. 127) are bounded only by
//! the study depth limit.

use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                Declarator,
                DirectDeclarator,
                ParenthesizedDeclarator,
                PointerDeclarator,
                PointerLevel,
                TypeQualifiers,
            },
            errors::ParserErrorType,
            grammar::{
                ProductionKind,
                declaration_specifiers::{
                    report_duplicate_type_qualifier,
                    type_qualifier,
                },
                expression_operators::{
                    is_array_pointer_marker,
                    is_operator,
                },
                expressions::{
                    ExpressionBoundary,
                    ExpressionMode,
                },
                gnu::{
                    GnuKind,
                    GnuValue,
                },
                modern::{
                    ModernKind,
                    ModernValue,
                },
                unexpected_return,
            },
            modern::{
                AttributeSpecifier,
                SpecifierExtension,
                SpecifierExtensionKind,
            },
            msvc,
            recovery::{
                SynchronizationKind,
                SynchronizationSet,
            },
            syntax,
            syntax::{
                Expression,
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
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses one named, abstract, or maybe-abstract declarator. `None`
    /// inside the result is the typed, recoverable absence that optional
    /// abstract declarators and contextual missing-declarator diagnostics
    /// use.
    ///
    /// C99: declarator and direct-declarator are §6.7.5, pp. 114-121; PDF
    /// pp. 126-133. Abstract forms are §6.7.6, p. 122; PDF p. 134.
    #[expect(
        clippy::option_option,
        reason = "The outer `None` is a terminal resource failure; the inner one is the typed, \
                  recoverable absence of a declarator."
    )]
    pub(super) fn parse_declarator(
        &mut self,
        mode: DeclaratorMode,
    ) -> Option<Option<Declarator<'tu>>> {
        self.parse_declarator_with_name(mode)
            .map(|(declarator, _)| declarator)
    }

    /// Parses a declarator and reports whether it declares an identifier,
    /// directly or through a parenthesized nested declarator.
    #[inline(never)]
    fn parse_declarator_with_name(
        &mut self,
        mode: DeclaratorMode,
    ) -> Option<(Option<Declarator<'tu>>, bool)> {
        let entry = self.enter_production(ProductionKind::Declarator)?;
        let mut parts = DeclaratorParts {
            mode,
            pointer_levels: ArenaVec::new_in(self.arena),
            direct_declarators: ArenaVec::new_in(self.arena),
            has_pointer_level: false,
            has_direct_declarator: false,
            named: false,
            source_vectors: None,
        };
        self.pools.pointer_levels.lend(&mut parts.pointer_levels);
        self.pools
            .direct_declarators
            .lend(&mut parts.direct_declarators);
        let result = self.parse_declarator_body(&mut parts);
        self.pools.pointer_levels.reclaim(&mut parts.pointer_levels);
        self.pools
            .direct_declarators
            .reclaim(&mut parts.direct_declarators);
        self.leave_production(entry);
        result.map(|declarator| (declarator, parts.named))
    }

    #[expect(
        clippy::option_option,
        reason = "The outer `None` is a terminal resource failure; the inner one is the typed, \
                  recoverable absence of a declarator."
    )]
    fn parse_declarator_body(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
    ) -> Option<Option<Declarator<'tu>>> {
        self.parse_pointer(parts)?;
        if self.parse_direct_declarator_base(parts)? {
            self.parse_declarator_suffixes(parts)?;
        }
        // `None` is a typed, recoverable absence used by optional abstract
        // declarators and by callers that issue the contextual
        // missing-declarator diagnostic.
        if !parts.has_pointer_level && !parts.has_direct_declarator {
            return Some(None);
        }
        if parts.mode == DeclaratorMode::Named && !parts.has_direct_declarator {
            let token = self.cursor.current();
            self.report(ParserErrorType::TypeQualifiersWithoutDeclarator, token);
            return Some(None);
        }
        // Commit both flat component lists atomically before returning the
        // value that references their stable slices.
        let levels = self.alloc_syntax_list(&mut parts.pointer_levels);
        let kind = self.alloc_syntax_list(&mut parts.direct_declarators);
        Some(Some(Declarator {
            pointer: PointerDeclarator { levels },
            kind,
            source_vectors: parts.source_vectors.unwrap_or_default(),
        }))
    }

    /// Parses the optional `pointer` prefix. Each `*` owns the qualifiers
    /// and attributes immediately following it.
    ///
    /// C99: pointer declarators are §6.7.5.1, p. 115; PDF p. 127.
    fn parse_pointer(&mut self, parts: &mut DeclaratorParts<'tu, 'p>) -> Option<()> {
        loop {
            let token = self.cursor.current();
            if self.parse_ms_modifier(parts, token, true) {
                continue;
            }
            if self.attribute_starter(token) {
                let attributes = self.parse_declarator_attributes(parts)?;
                parts
                    .direct_declarators
                    .push(DirectDeclarator::Attributes(attributes));
                continue;
            }
            if !is_operator(token, OperatorTokenType::Asterisk) {
                return Some(());
            }
            let token = token.expect("asterisk token exists");
            self.merge_source(&mut parts.source_vectors, token);
            parts.has_pointer_level = true;
            parts.pointer_levels.push(PointerLevel {
                qualifiers: TypeQualifiers::empty(),
                attributes: None,
            });
            self.consume_token();
            break;
        }
        loop {
            let token = self.cursor.current();
            if self.parse_ms_modifier(parts, token, false) {
                continue;
            }
            if self.attribute_starter(token) {
                let attributes = self.parse_declarator_attributes(parts)?;
                // C23 §6.7.7.2p1: attributes after a `*` appertain to that
                // pointer.
                let level = parts
                    .pointer_levels
                    .last_mut()
                    .expect("pointer level exists");
                level.attributes = Some(self.alloc_syntax(SpecifierExtension {
                    kind:           SpecifierExtensionKind::Attributes(attributes),
                    next:           level.attributes,
                    source_vectors: attributes.source_vectors,
                }));
                continue;
            }
            if let Some(token) = token
                && let Some(qualifier) = type_qualifier(token.kind)
            {
                let level = parts
                    .pointer_levels
                    .last_mut()
                    .expect("pointer level exists");
                // C99 §6.7.3p4: a repeated qualifier is harmless; warn.
                if parts.mode != DeclaratorMode::Abstract && level.qualifiers.contains(qualifier) {
                    report_duplicate_type_qualifier(self, token, qualifier);
                }
                level.qualifiers.insert(qualifier);
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                continue;
            }
            if !is_operator(token, OperatorTokenType::Asterisk) {
                return Some(());
            }
            parts.pointer_levels.push(PointerLevel {
                qualifiers: TypeQualifiers::empty(),
                attributes: None,
            });
            let token = token.expect("asterisk token exists");
            self.merge_source(&mut parts.source_vectors, token);
            self.consume_token();
        }
    }

    /// Parses the identifier or parenthesized base of a direct declarator.
    /// Returns whether suffixes may follow.
    ///
    /// C99: direct-declarator is §6.7.5 paragraph 1, p. 114; PDF p. 126;
    /// direct-abstract-declarator is §6.7.6 paragraph 1, p. 122; PDF p. 134.
    fn parse_direct_declarator_base(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
    ) -> Option<bool> {
        loop {
            let token = self.cursor.current();
            if self.parse_ms_modifier(parts, token, true) {
                continue;
            }
            if self.attribute_starter(token) {
                let attributes = self.parse_declarator_attributes(parts)?;
                parts
                    .direct_declarators
                    .push(DirectDeclarator::Attributes(attributes));
                continue;
            }
            if let Some(token) = token
                && matches!(token.kind, TokenType::Identifier)
                && parts.mode != DeclaratorMode::Abstract
            {
                self.merge_source(&mut parts.source_vectors, token);
                parts
                    .direct_declarators
                    .push(DirectDeclarator::Identifier(Identifier::from_token(token)));
                parts.has_direct_declarator = true;
                parts.named = true;
                self.consume_token();
                return Some(true);
            }
            // In a named declarator, `(` must group another declarator. In
            // an abstract context it can instead start a function suffix.
            if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                let token = token.expect("opening-parenthesis token exists");
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                if parts.mode != DeclaratorMode::Named
                    && self.abstract_parenthesis_is_function(parts)?
                {
                    return Some(true);
                }
                self.parse_parenthesized_declarator(parts, token.source_vectors)?;
                return Some(true);
            }
            if parts.mode != DeclaratorMode::Named
                && is_operator(token, OperatorTokenType::OpeningSquareBracket)
            {
                return Some(true);
            }
            // C99 §6.7.5p1: a direct-declarator starts with an identifier
            // or `(`.
            if parts.mode == DeclaratorMode::Named {
                self.report(
                    ParserErrorType::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
            }
            return Some(false);
        }
    }

    /// Disambiguates an abstract `(` as a function suffix, parsing it when it
    /// is one. `()` is an abstract function declarator; a declaration
    /// starter begins a prototype; everything else is grouping.
    ///
    /// C99: empty parentheses in a type name are a function declarator, not
    /// grouping, §6.7.6 footnote 128, p. 122; PDF p. 134; an identifier that
    /// could be a typedef name or a parameter name is taken as a typedef
    /// name, §6.7.5.3 paragraph 11, p. 119; PDF p. 131.
    fn abstract_parenthesis_is_function(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
    ) -> Option<bool> {
        let token = self.cursor.current();
        if is_operator(token, OperatorTokenType::ClosingParenthesis) {
            let token = token.expect("closing-parenthesis token exists");
            parts.direct_declarators.push(empty_function(self, true));
            parts.has_direct_declarator = true;
            self.merge_source(&mut parts.source_vectors, token);
            self.consume_token();
            return Some(true);
        }
        let prototype = is_operator(token, OperatorTokenType::Ellipsis)
            || if self.attribute_starter(token) {
                // Leading attributes may start a parameter or a grouped
                // declarator, so classify what follows them.
                self.attributes_precede_declaration()
            } else {
                token.is_some_and(|token| {
                    self.declaration_starter(token)
                        && !matches!(token.kind, TokenType::Keyword(keyword)
                            if msvc::calling_convention(keyword) || msvc::type_modifier(keyword))
                })
            };
        if prototype {
            self.parse_function_parameters(parts, false)?;
        }
        Some(prototype)
    }

    /// Parses the declarator nested inside `(` and its closing `)`.
    ///
    /// C99: parenthesized direct-declarator is §6.7.5, p. 114; PDF p. 126;
    /// it binds as the unparenthesized declarator, paragraph 6, p. 115;
    /// PDF p. 127.
    fn parse_parenthesized_declarator(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
        opening: SourceVectors,
    ) -> Option<()> {
        let (declarator, named) = self.parse_declarator_with_name(parts.mode)?;
        let Some(declarator) = declarator else {
            let token = self.cursor.current();
            self.report(
                ParserErrorType::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(
                    token.map(|token| token.kind),
                ),
                token,
            );
            return Some(());
        };
        self.context
            .merge_into(&mut parts.source_vectors, declarator.source_vectors);
        parts.has_direct_declarator = true;
        parts.named = named;
        let mut nested = ParenthesizedDeclarator {
            declarator,
            delimiters: opening,
        };
        let closing = self
            .cursor
            .current()
            .filter(|token| is_operator(Some(*token), OperatorTokenType::ClosingParenthesis));
        if let Some(token) = closing {
            nested.delimiters = self
                .context
                .merge_vectors(nested.delimiters, token.source_vectors);
        }
        let parenthesized = self.alloc_syntax(nested);
        parts
            .direct_declarators
            .push(DirectDeclarator::Parenthesized(parenthesized));
        if let Some(token) = closing {
            self.merge_source(&mut parts.source_vectors, token);
            self.consume_token();
        } else {
            let token = self.cursor.current();
            self.report(
                ParserErrorType::ExpectedClosingParenthesisAfterParenthesizedDeclarator(
                    token.map(|token| token.kind),
                ),
                token,
            );
        }
        Some(())
    }

    /// Parses zero or more array, function, attribute, and asm-label
    /// suffixes left to right, preserving binding order.
    ///
    /// C99: array and function declarators are §6.7.5.2-§6.7.5.3,
    /// pp. 116-121; PDF pp. 128-133.
    fn parse_declarator_suffixes(&mut self, parts: &mut DeclaratorParts<'tu, 'p>) -> Option<()> {
        loop {
            let token = self.cursor.current();
            if self.parse_ms_modifier(parts, token, true) {
                continue;
            }
            if token.is_some_and(|token| {
                matches!(token.kind, TokenType::Keyword(KeywordTokenType::Asm))
                    || matches!(token.kind, TokenType::Keyword(KeywordTokenType::MsAsm))
                        && token.uses_ambiguous_asm(self.context.configuration)
            }) {
                let value = self.parse_gnu(GnuKind::Asm { label: true }, self.hard_error_count)?;
                let GnuValue::Asm(asm) = value else {
                    unexpected_return!("asm label child protocol: {value:?}")
                };
                parts
                    .direct_declarators
                    .push(DirectDeclarator::AsmLabel(asm));
                self.context
                    .merge_into(&mut parts.source_vectors, asm.source_vectors);
                continue;
            }
            if self.attribute_starter(token) {
                let attributes = self.parse_declarator_attributes(parts)?;
                parts
                    .direct_declarators
                    .push(DirectDeclarator::Attributes(attributes));
                continue;
            }
            let more = if is_operator(token, OperatorTokenType::OpeningSquareBracket) {
                let token = token.expect("opening-square-bracket token exists");
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                self.parse_array_suffix(parts)?
            } else if is_operator(token, OperatorTokenType::OpeningParenthesis) {
                let token = token.expect("opening-parenthesis token exists");
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                self.parse_function_suffix(parts)?
            } else {
                false
            };
            if !more {
                return Some(());
            }
        }
    }

    /// Parses array qualifiers, `static`, `*`, or an optional bound after
    /// `[`. Returns whether further suffixes may follow. `static`,
    /// qualifiers, and `*` stay independent facts so later semantic checks
    /// retain their original placement.
    ///
    /// C99: the four array suffix forms of §6.7.5 paragraph 1, p. 114;
    /// PDF p. 126, and §6.7.6 paragraph 1, p. 122; PDF p. 134.
    fn parse_array_suffix(&mut self, parts: &mut DeclaratorParts<'tu, 'p>) -> Option<bool> {
        let mut array = ArraySuffix::default();
        loop {
            let token = self.cursor.current();
            if self.attribute_starter(token) {
                let attributes = self.parse_declarator_attributes(parts)?;
                parts
                    .direct_declarators
                    .push(DirectDeclarator::Attributes(attributes));
                continue;
            }
            if let Some(token) = token
                && let Some(qualifier) = type_qualifier(token.kind)
            {
                if parts.mode != DeclaratorMode::Abstract && array.qualifiers.contains(qualifier) {
                    report_duplicate_type_qualifier(self, token, qualifier);
                }
                if array.qualifiers.is_empty() {
                    self.c99_syntax_extension("qualified array parameter", token);
                }
                // C99 §6.7.5p1: qualifiers go before or after `static`, not
                // both.
                if array.is_static && array.qualifiers_before_static {
                    self.report(
                        ParserErrorType::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
                        Some(token),
                    );
                }
                if !array.is_static {
                    array.qualifiers_before_static = true;
                }
                array.qualifiers.insert(qualifier);
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                continue;
            }
            if let Some(token) = token
                && matches!(token.kind, TokenType::Keyword(KeywordTokenType::Static))
            {
                self.c99_syntax_extension("static array parameter", token);
                if array.is_static {
                    self.report(ParserErrorType::StaticSpecifiedTwice, Some(token));
                }
                array.is_static = true;
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                continue;
            }
            if is_array_pointer_marker(self, token) {
                let token = token.expect("asterisk token exists");
                self.c99_syntax_extension("variable length array marker", token);
                // C99 §6.7.6p1: an abstract declarator's `[ * ]` takes no
                // qualifiers, whichever suffixes or grouping precede it.
                if parts.mode != DeclaratorMode::Named
                    && !parts.named
                    && !array.qualifiers.is_empty()
                {
                    self.report(
                        ParserErrorType::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
                        Some(token),
                    );
                }
                // C99 §6.7.5p1: the `[*]` form takes no `static`.
                if array.is_static {
                    self.report(
                        ParserErrorType::BothStaticAndPointerInArrayDirectDeclarator,
                        Some(token),
                    );
                }
                array.is_pointer = true;
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                return Some(self.parse_array_pointer_close(parts, array));
            }
            if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                let token = token.expect("closing-square-bracket token exists");
                // C99 §6.7.5p1: both `static` forms require a bound.
                if array.is_static {
                    self.report(
                        ParserErrorType::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
                        Some(token),
                    );
                }
                parts.push_array(&mut array);
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                return Some(true);
            }
            if token.is_none() {
                self.report(
                    ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(None),
                    None,
                );
                parts.push_array(&mut array);
                return Some(false);
            }
            let index = self
                .parse_expression_ending_at(
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::ArrayBound,
                )?
                .expression;
            let source_vectors = index.source_vectors;
            check_array_extension(self, index);
            array.assignment_expression = Some(index);
            self.context
                .merge_into(&mut parts.source_vectors, source_vectors);
            return Some(self.parse_array_close(parts, array));
        }
    }

    /// Requires the `]` after the VLA `*` marker. Extra input is diagnosed
    /// specifically, then recovery runs to the owning bracket or an
    /// enclosing declaration boundary.
    fn parse_array_pointer_close(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
        mut array: ArraySuffix<'tu>,
    ) -> bool {
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                let token = token.expect("closing-square-bracket token exists");
                parts.push_array(&mut array);
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                return true;
            }
            if is_operator(token, OperatorTokenType::Asterisk) {
                let token = token.expect("asterisk token exists");
                self.report(ParserErrorType::PointerSpecifiedTwice, Some(token));
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                continue;
            }
            if let Some(token) = token {
                self.report(
                    ParserErrorType::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(
                        token.kind,
                    ),
                    Some(token),
                );
            } else {
                self.report(
                    ParserErrorType::UnexpectedEndOfArrayDeclaratorAfterPointer,
                    None,
                );
            }
            self.recover_array_bound(parts);
            return self.parse_array_close(parts, array);
        }
    }

    /// Closes an array suffix after its bound or after recovery. A token
    /// that belongs to an enclosing production repairs the absent `]` and
    /// finishes the declarator without consuming it.
    fn parse_array_close(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
        mut array: ArraySuffix<'tu>,
    ) -> bool {
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::ClosingSquareBracket) {
                let token = token.expect("closing-square-bracket token exists");
                parts.push_array(&mut array);
                self.merge_source(&mut parts.source_vectors, token);
                self.consume_token();
                return true;
            }
            if is_operator(token, OperatorTokenType::Comma)
                || is_operator(token, OperatorTokenType::Semicolon)
                || is_operator(token, OperatorTokenType::ClosingParenthesis)
                || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                || token.is_none()
                || token.is_some_and(|token| self.declaration_starter(token))
            {
                if !array.is_pointer {
                    self.report(
                        ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                }
                parts.push_array(&mut array);
                return false;
            }
            self.report(
                ParserErrorType::ExpectedClosingSquareBracketInArrayDirectDeclarator(
                    token.map(|token| token.kind),
                ),
                token,
            );
            self.recover_array_bound(parts);
        }
    }

    fn recover_array_bound(&mut self, parts: &mut DeclaratorParts<'tu, 'p>) {
        let recovered = self.recover(SynchronizationSet {
            kind:   SynchronizationKind::ArrayBound,
            target: ProductionKind::Declarator,
        });
        if let Some(recovered) = recovered {
            self.context
                .merge_into(&mut parts.source_vectors, recovered);
        }
    }

    /// Parses a function suffix after `(`: empty, K&R, or prototype-style.
    /// Returns whether further suffixes may follow. An identifier list is
    /// legal only when the suffix is bound to a named declarator.
    ///
    /// C99: `( parameter-type-list )` and `( identifier-list? )`, §6.7.5
    /// paragraph 1, p. 114; PDF p. 126; abstract declarators take only
    /// `( parameter-type-list? )`, §6.7.6 paragraph 1, p. 122; PDF p. 134.
    fn parse_function_suffix(&mut self, parts: &mut DeclaratorParts<'tu, 'p>) -> Option<bool> {
        let allow_k_and_r = parts.named;
        let token = self.cursor.current();
        if is_operator(token, OperatorTokenType::ClosingParenthesis) {
            let direct = empty_function(self, allow_k_and_r);
            parts.direct_declarators.push(direct);
            parts.has_direct_declarator = true;
            let token = token.expect("closing-parenthesis token exists");
            self.merge_source(&mut parts.source_vectors, token);
            self.consume_token();
            return Some(true);
        }
        if token.is_none() {
            self.report(
                ParserErrorType::UnexpectedEndOfFunctionDeclaratorParameterList,
                None,
            );
            return Some(false);
        }
        self.parse_function_parameters(parts, allow_k_and_r)?;
        Some(true)
    }

    fn parse_function_parameters(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
        allow_k_and_r: bool,
    ) -> Option<()> {
        let list = self.parse_parameter_list(allow_k_and_r)?;
        self.context
            .merge_into(&mut parts.source_vectors, list.source_vectors);
        parts.direct_declarators.push(list.direct_declarator);
        parts.has_direct_declarator = true;
        Some(())
    }

    /// Consumes a Microsoft calling convention, or with `type_modifiers` a
    /// Microsoft type modifier, as a direct-declarator entry.
    /// MSVC extension: Microsoft Learn, "Microsoft-Specific Modifiers".
    fn parse_ms_modifier(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
        token: Option<Token>,
        type_modifiers: bool,
    ) -> bool {
        if let Some(token) = token
            && let TokenType::Keyword(keyword) = token.kind
            && (msvc::calling_convention(keyword) || type_modifiers && msvc::type_modifier(keyword))
        {
            parts
                .direct_declarators
                .push(DirectDeclarator::MsModifier(keyword, token.source_vectors));
            self.merge_source(&mut parts.source_vectors, token);
            self.consume_token();
            return true;
        }
        false
    }

    /// Parses an attribute-specifier sequence at a declarator attachment
    /// point and merges its provenance into the declarator.
    /// C23: §6.7.7.1 paragraph 1, p. 126; PDF p. 139.
    fn parse_declarator_attributes(
        &mut self,
        parts: &mut DeclaratorParts<'tu, 'p>,
    ) -> Option<&'tu AttributeSpecifier<'tu>> {
        let value = self.parse_modern(ModernKind::Attributes, None, self.hard_error_count)?;
        let ModernValue::Attributes(attributes) = value else {
            unexpected_return!("declarator attributes protocol: {value:?}")
        };
        self.context
            .merge_into(&mut parts.source_vectors, attributes.source_vectors);
        Some(attributes)
    }
}

/// Components of one declarator accumulated before arena insertion.
struct DeclaratorParts<'tu, 'p> {
    /// Whether the declarator requires or permits an identifier.
    mode:                  DeclaratorMode,
    /// Pointer levels, outermost first. The last one collects the
    /// qualifiers and attributes after its `*` while they are parsed.
    pointer_levels:        ArenaVec<'p, PointerLevel<'tu>>,
    /// Direct base and suffixes in binding order.
    direct_declarators:    ArenaVec<'p, DirectDeclarator<'tu>>,
    /// Whether at least one pointer level has been parsed.
    has_pointer_level:     bool,
    /// Whether an identifier, parenthesized base, or suffix has been parsed.
    has_direct_declarator: bool,
    /// Whether this declarator declares an identifier, directly or through a
    /// parenthesized nested declarator.
    named:                 bool,
    /// Provenance accumulated across every declarator component.
    source_vectors:        Option<SourceVectors>,
}

impl<'tu> DeclaratorParts<'tu, '_> {
    fn push_array(&mut self, array: &mut ArraySuffix<'tu>) {
        self.direct_declarators.push(DirectDeclarator::Array {
            type_qualifiers:       array.qualifiers,
            is_static:             array.is_static,
            is_pointer:            array.is_pointer,
            assignment_expression: array.assignment_expression.take(),
        });
        self.has_direct_declarator = true;
    }
}

/// Facts of the array suffix being parsed.
#[derive(Default)]
struct ArraySuffix<'tu> {
    qualifiers:               TypeQualifiers,
    /// Whether qualifiers occurred before `static`.
    qualifiers_before_static: bool,
    is_static:                bool,
    /// Whether the suffix uses the `[*]` form.
    is_pointer:               bool,
    assignment_expression:    Option<&'tu Expression<'tu>>,
}

/// Whether a declarator requires, forbids, or optionally accepts a name.
///
/// C99: named declarators are §6.7.5, pp. 114-121; PDF pp. 126-133; abstract
/// declarators are §6.7.6, p. 122; PDF p. 134.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::parsing) enum DeclaratorMode {
    /// Ordinary declarator requiring an identifier base.
    Named,
    /// Abstract declarator used where no identifier may be declared.
    Abstract,
    /// Parameter declarator that may be named or abstract.
    MaybeAbstract,
}

/// C23: §6.7.7.4 paragraph 13, p. 130; PDF p. 143 makes every empty
/// function declarator a prototype, as if its parameter list were `void`.
pub(in crate::translation_phases::parsing) fn empty_function<'tu>(
    parser: &Parser<'_, 'tu, '_>,
    legacy: bool,
) -> DirectDeclarator<'tu> {
    if legacy && parser.context.configuration.standard() < crate::configuration::CStandard::C23 {
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

pub(in crate::translation_phases::parsing) fn check_array_extension<'tu>(
    parser: &mut Parser<'_, 'tu, '_>,
    index: &'tu Expression<'tu>,
) {
    let mut operand = index;
    while let syntax::ExpressionType::Parenthesized { expression, .. } = operand.kind {
        operand = expression;
    }
    if let syntax::ExpressionType::Constant(syntax::Constant::Integer(value)) = operand.kind
        && i128::from(value) == 0
    {
        parser.extension_source(
            crate::configuration::Feature::ZeroLengthArrays,
            "zero-length array",
            index.source_vectors,
        );
    }
}
