//! Recursive ISO additions to translation phase 7: generic selections, static
//! assertions, attribute specifiers (including their GNU and MSVC
//! spellings), and the parenthesized operands of `_Alignof`, `_Alignas`,
//! `_Atomic`, `_BitInt`, and `typeof`.
//!
//! C11: generic selections §6.5.1.1, p. 78; PDF p. 96; static assertions
//! §6.7.10, p. 145; PDF p. 163. C99: the shared type-name and expression
//! children are §6.7.6, p. 122; PDF p. 134 and §6.5, pp. 67-94;
//! PDF pp. 79-106. Evaluation and type constraints belong to later analysis.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            errors::ParserErrorType,
            extensions::modern::{
                AttributeArgument,
                AttributeSpecifier,
                AttributeSyntax,
                GenericAssociation,
                GenericSelection,
                ModernKind,
                ModernValue,
                StaticAssertion,
                SyntaxOperand,
            },
            frames::{
                expression::{
                    ExpressionBoundary,
                    ExpressionFrame,
                    ExpressionMode,
                },
                type_name::TypeNameFrame,
            },
            machine::{
                ParseFrame,
                ParseFrameKind,
                ParseValue,
                any_expression_value,
                unexpected_return,
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
    /// Parses later-standard operands, generic associations, assertions and
    /// attributes. `keyword` is the already consumed keyword's provenance
    /// when the caller owns the keyword and this production starts at `(`.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    /// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
    /// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163.
    /// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    pub(super) fn rd_modern(
        &mut self,
        kind: ModernKind,
        keyword: Option<SourceVectors>,
        starting_errors: usize,
    ) -> Option<ModernValue<'tu>> {
        let entry = self.rd_enter(ParseFrameKind::Modern)?;
        let result = match self.cursor.current() {
            | Some(token) if kind == ModernKind::Attributes && keyword.is_none() => self
                .rd_attribute_specifier(token, starting_errors)
                .map(ModernValue::Attributes),
            | _ => self.rd_iso_construct(kind, keyword, starting_errors),
        };
        self.study_leave(entry);
        result
    }

    /// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96; §6.7.10 paragraph 1,
    /// p. 145; PDF p. 163. C11: §6.7.2.4 paragraph 1, p. 121; PDF p. 139;
    /// §6.7.5 paragraph 1, p. 127; PDF p. 145. C23: §6.7.3.6 paragraph 1,
    /// p. 117; PDF p. 130.
    fn rd_iso_construct(
        &mut self,
        kind: ModernKind,
        keyword_source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> Option<ModernValue<'tu>> {
        let mut source = keyword_source;
        let mut keyword = None;
        if keyword_source.is_none()
            && let Some(token) = self.cursor.current()
        {
            if let TokenType::Keyword(spelling) = token.kind {
                keyword = Some(spelling);
            }
            self.rd_iso_own(kind, &mut source, token);
        }
        self.rd_iso_punctuation(
            kind,
            &mut source,
            OperatorTokenType::OpeningParenthesis,
            "`(` in ISO construct",
        );
        let token = self.cursor.current();
        let type_only = matches!(
            kind,
            ModernKind::Operand {
                type_only: true,
                ..
            }
        );
        let is_type = type_only
            || kind != ModernKind::Assertion
                && keyword != Some(KeywordTokenType::BitInt)
                && token.is_some_and(|x| self.type_name_starter(x));
        let returned = if is_type {
            if kind == ModernKind::Generic
                && let Some(token) = token
            {
                self.extension(
                    Feature::GenericTypeOperand,
                    "type-controlling _Generic",
                    token,
                );
            }
            self.rd_child(ParseFrame::TypeName(TypeNameFrame::new(
                self.hard_error_count,
            )))?
        } else {
            let mode = if kind == ModernKind::Assertion
                || matches!(kind, ModernKind::Operand { constant: true, .. })
            {
                ExpressionMode::ConstantExpression
            } else if kind == ModernKind::Generic {
                ExpressionMode::AssignmentExpression
            } else {
                ExpressionMode::Expression
            };
            let boundary = if matches!(
                kind,
                ModernKind::Operand {
                    constant: false,
                    ..
                }
            ) {
                ExpressionBoundary::ClosingParenthesis
            } else {
                ExpressionBoundary::Argument
            };
            self.rd_child(ParseFrame::Expression(ExpressionFrame::new(
                self.arena,
                mode,
                boundary,
                self.hard_error_count,
            )))?
        };
        let operand = match returned {
            | ParseValue::TypeName(x) => SyntaxOperand::Type(x),
            | other => SyntaxOperand::Expression(any_expression_value(Some(other))),
        };
        self.context.merge_into(&mut source, operand.source());
        let mut message = None;
        let mut associations = ArenaVec::new_in(self.arena);
        self.pools.modern_associations.lend(&mut associations);
        let parsed = if matches!(kind, ModernKind::Operand { .. }) {
            Some(())
        } else if kind == ModernKind::Assertion {
            message = self.rd_assertion_message(kind, &mut source);
            Some(())
        } else {
            self.rd_generic_associations(kind, &mut source, &mut associations)
        };
        let value = parsed.map(|()| {
            self.rd_iso_punctuation(
                kind,
                &mut source,
                OperatorTokenType::ClosingParenthesis,
                "`)` in ISO construct",
            );
            if kind == ModernKind::Assertion {
                self.rd_iso_punctuation(
                    kind,
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "`;` after static assertion",
                );
            }
            self.rd_iso_value(
                kind,
                source,
                operand,
                message,
                &mut associations,
                starting_errors,
            )
        });
        self.pools.modern_associations.reclaim(&mut associations);
        value
    }

    /// The optional message of a static assertion. C23 permits none.
    /// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163.
    fn rd_assertion_message(
        &mut self,
        kind: ModernKind,
        source: &mut Option<SourceVectors>,
    ) -> Option<Token> {
        let token = self.cursor.current();
        if let Some(comma) = token
            && matches!(comma.kind, TokenType::Operator(OperatorTokenType::Comma))
        {
            self.rd_iso_own(kind, source, comma);
            if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::String(_))
            {
                self.rd_iso_own(kind, source, token);
                return Some(token);
            }
            iso_expected(
                self,
                self.cursor.current(),
                "string literal in static assertion",
            );
        } else if !self.context.configuration.is_native(Feature::C23Keywords)
            && let Some(token) = token
        {
            self.extension(
                Feature::C23Keywords,
                "static assertion without message",
                token,
            );
        }
        None
    }

    /// Parses `, generic-assoc-list`.
    /// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
    fn rd_generic_associations(
        &mut self,
        kind: ModernKind,
        source: &mut Option<SourceVectors>,
        associations: &mut ArenaVec<'p, GenericAssociation<'tu>>,
    ) -> Option<()> {
        self.rd_iso_punctuation(
            kind,
            source,
            OperatorTokenType::Comma,
            "`,` before generic associations",
        );
        loop {
            let type_name = if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::Keyword(KeywordTokenType::Default))
            {
                self.rd_iso_own(kind, source, token);
                None
            } else {
                let returned = self.rd_child(ParseFrame::TypeName(TypeNameFrame::new(
                    self.hard_error_count,
                )))?;
                let ParseValue::TypeName(x) = returned else {
                    unexpected_return!("generic association type protocol: {returned:?}")
                };
                self.context.merge_into(source, x.source_vectors);
                Some(x)
            };
            self.rd_iso_punctuation(
                kind,
                source,
                OperatorTokenType::Colon,
                "`:` in generic association",
            );
            let returned = self.rd_child(ParseFrame::Expression(ExpressionFrame::new(
                self.arena,
                ExpressionMode::AssignmentExpression,
                ExpressionBoundary::Argument,
                self.hard_error_count,
            )))?;
            let expression = any_expression_value(Some(returned));
            self.context.merge_into(source, expression.source_vectors);
            associations.push(GenericAssociation {
                type_name,
                expression,
            });
            if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::Operator(OperatorTokenType::Comma))
            {
                self.rd_iso_own(kind, source, token);
            } else {
                return Some(());
            }
        }
    }

    fn rd_iso_value(
        &mut self,
        kind: ModernKind,
        source: Option<SourceVectors>,
        operand: SyntaxOperand<'tu>,
        message: Option<Token>,
        associations: &mut ArenaVec<'p, GenericAssociation<'tu>>,
        starting_errors: usize,
    ) -> ModernValue<'tu> {
        let source_vectors = if kind == ModernKind::Attributes {
            self.cursor.source_since(0)
        } else {
            source.unwrap_or_default()
        };
        let recovered = self.hard_error_count > starting_errors;
        match kind {
            | ModernKind::Operand { .. } => ModernValue::Operand(operand, source_vectors),
            | ModernKind::Generic => {
                let associations = self.alloc_syntax_list(associations);
                ModernValue::Generic(self.alloc_syntax(GenericSelection {
                    source_vectors,
                    recovered,
                    controlling: operand,
                    associations,
                }))
            },
            | ModernKind::Assertion => {
                let SyntaxOperand::Expression(expression) = operand else {
                    unreachable!("assertion uses expressions")
                };
                ModernValue::Assertion(self.alloc_syntax(StaticAssertion {
                    expression,
                    message,
                    source_vectors,
                    recovered,
                }))
            },
            | ModernKind::Attributes => {
                // Only reached when an attribute owner starts at end of input.
                let tokens = self.cursor.retain_since(0, self.tree);
                let mut arguments = ArenaVec::new_in(self.arena);
                let arguments = self.alloc_syntax_list(&mut arguments);
                ModernValue::Attributes(self.alloc_syntax(AttributeSpecifier {
                    syntax: AttributeSyntax::Standard,
                    tokens,
                    arguments,
                    source_vectors,
                    recovered,
                }))
            },
        }
    }

    fn rd_iso_own(&mut self, kind: ModernKind, source: &mut Option<SourceVectors>, token: Token) {
        if kind != ModernKind::Attributes {
            self.merge_source(source, token);
        }
        self.rd_consume();
    }

    fn rd_iso_punctuation(
        &mut self,
        kind: ModernKind,
        source: &mut Option<SourceVectors>,
        operator: OperatorTokenType,
        position: &'static str,
    ) {
        let token = self.cursor.current();
        if let Some(token) = token
            && matches!(token.kind, TokenType::Operator(actual) if actual == operator)
        {
            self.rd_iso_own(kind, source, token);
        } else {
            iso_expected(self, token, position);
        }
    }

    /// Parses a standard `[[...]]`, GNU `__attribute__((...))`, or MSVC
    /// `__declspec(...)` specifier as balanced tokens, with the recognized
    /// alignment-like operands as expressions. `token` is its introducer.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    /// MSVC extension: Microsoft Learn, "declspec".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
    /// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn rd_attribute_specifier(
        &mut self,
        token: Token,
        starting_errors: usize,
    ) -> Option<&'tu AttributeSpecifier<'tu>> {
        let mut state = AttributeState {
            syntax:         AttributeSyntax::Standard,
            position:       Position::Opening,
            start:          self.cursor.consumed,
            argument_index: 0,
            tokens:         ArenaVec::new_in(self.arena),
            delimiters:     ArenaVec::new_in(self.arena),
            arguments:      ArenaVec::new_in(self.arena),
        };
        self.pools.opaque_tokens.lend(&mut state.tokens);
        self.pools.attribute_arguments.lend(&mut state.arguments);
        self.pools.delimiters.lend(&mut state.delimiters);
        let openers = match token.kind {
            | TokenType::Keyword(KeywordTokenType::Declspec) => {
                state.syntax = AttributeSyntax::Msvc;
                1
            },
            | TokenType::Keyword(KeywordTokenType::Attribute) => {
                state.syntax = AttributeSyntax::Gnu;
                2
            },
            | _ => {
                self.extension(Feature::Attributes, "[[...]]", token);
                state
                    .delimiters
                    .push(OperatorTokenType::ClosingSquareBracket);
                0
            },
        };
        state.tokens.push(token);
        self.rd_consume();
        let parsed = self.rd_attribute_body(&mut state, openers);
        let value = parsed.map(|()| {
            let source_vectors = self.cursor.source_since(state.start);
            let recovered = self.hard_error_count > starting_errors;
            let tokens = self.cursor.retain_since(state.start, self.tree);
            let arguments = self.alloc_syntax_list(&mut state.arguments);
            self.alloc_syntax(AttributeSpecifier {
                syntax: state.syntax,
                tokens,
                arguments,
                source_vectors,
                recovered,
            })
        });
        self.pools.opaque_tokens.reclaim(&mut state.tokens);
        self.pools.attribute_arguments.reclaim(&mut state.arguments);
        self.pools.delimiters.reclaim(&mut state.delimiters);
        value
    }

    fn rd_attribute_body(
        &mut self,
        state: &mut AttributeState<'tu, 'p>,
        openers: usize,
    ) -> Option<()> {
        for _ in 0..openers {
            state.position = Position::Name;
            if let Some(token) = self.cursor.current()
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                )
            {
                state.tokens.push(token);
                state.delimiters.push(OperatorTokenType::ClosingParenthesis);
                self.rd_consume();
            } else {
                attribute_expected(
                    self,
                    state.syntax,
                    self.cursor.current(),
                    if state.syntax == AttributeSyntax::Msvc {
                        "`(` in MSVC declspec specifier"
                    } else {
                        "`(` in GNU attribute specifier"
                    },
                );
                return Some(());
            }
        }
        loop {
            match self.rd_attribute_token(state) {
                | AttributeStep::Token => {},
                | AttributeStep::Finish => return Some(()),
                | AttributeStep::Argument =>
                    if !self.rd_attribute_argument(state)? {
                        return Some(());
                    },
            }
        }
    }

    /// Consumes one balanced attribute token, checking the specifier's
    /// outer name, prefix, argument and separator structure.
    /// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn rd_attribute_token(&mut self, state: &mut AttributeState<'tu, 'p>) -> AttributeStep {
        let Some(token) = self.cursor.current() else {
            attribute_expected(self, state.syntax, None, closer_component(state.syntax));
            return AttributeStep::Finish;
        };
        // Leave caller boundaries unconsumed, but keep semicolons
        // within standard balanced-token arguments (C23 §6.7.13.2p1).
        if self.rd_attribute_caller_owns(state, token) {
            attribute_expected(
                self,
                state.syntax,
                Some(token),
                closer_component(state.syntax),
            );
            return AttributeStep::Finish;
        }
        let mut argument = false;
        if state.delimiters.len() <= outer_depth(state.syntax) {
            use Position::{
                AfterArguments,
                AfterName,
                AfterPrefixedName,
                Closing,
                Name,
                Opening,
                PrefixedName,
                SecondColon,
            };
            let kind = token.kind;
            let identifier = matches!(kind, TokenType::Identifier | TokenType::Keyword(_));
            let closer = if state.syntax == AttributeSyntax::Standard {
                OperatorTokenType::ClosingSquareBracket
            } else {
                OperatorTokenType::ClosingParenthesis
            };
            let position = state.position;
            let valid = match position {
                | Opening => {
                    state.position = Name;
                    matches!(
                        kind,
                        TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                    )
                },
                | Name | PrefixedName if identifier => {
                    state.position = if matches!(position, Name) {
                        AfterName
                    } else {
                        AfterPrefixedName
                    };
                    true
                },
                | AfterName | AfterArguments
                    if identifier && state.syntax == AttributeSyntax::Msvc =>
                {
                    state.position = AfterName;
                    true
                },
                | Name | AfterName | AfterPrefixedName | AfterArguments
                    if matches!(kind, TokenType::Operator(OperatorTokenType::Comma)) =>
                {
                    state.position = Name;
                    true
                },
                | Name | AfterName | AfterPrefixedName | AfterArguments if matches!(kind, TokenType::Operator(actual) if actual == closer) =>
                {
                    state.position = Closing;
                    true
                },
                | AfterName if matches!(kind, TokenType::Operator(OperatorTokenType::Colon)) => {
                    state.position = SecondColon;
                    true
                },
                | SecondColon if matches!(kind, TokenType::Operator(OperatorTokenType::Colon)) => {
                    state.position = PrefixedName;
                    true
                },
                | AfterName | AfterPrefixedName
                    if matches!(
                        kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    ) =>
                {
                    argument = self.rd_attribute_argument_starts(state);
                    state.position = AfterArguments;
                    true
                },
                | Closing if matches!(kind, TokenType::Operator(actual) if actual == closer) =>
                    true,
                | Closing => {
                    // Only the second closer can follow the first;
                    // anything else starts the attributed syntax.
                    attribute_expected(
                        self,
                        state.syntax,
                        Some(token),
                        closer_component(state.syntax),
                    );
                    return AttributeStep::Finish;
                },
                | _ => false,
            };
            if !valid {
                attribute_expected(
                    self,
                    state.syntax,
                    Some(token),
                    "attribute name, arguments, or separator",
                );
            }
        }
        if let TokenType::Operator(op) = token.kind {
            let closing = match op {
                | OperatorTokenType::OpeningParenthesis =>
                    Some(OperatorTokenType::ClosingParenthesis),
                | OperatorTokenType::OpeningSquareBracket =>
                    Some(OperatorTokenType::ClosingSquareBracket),
                | OperatorTokenType::OpeningCurlyBrace =>
                    Some(OperatorTokenType::ClosingCurlyBrace),
                | _ => None,
            };
            if let Some(closing) = closing {
                state.delimiters.push(closing);
            } else if matches!(
                op,
                OperatorTokenType::ClosingParenthesis
                    | OperatorTokenType::ClosingSquareBracket
                    | OperatorTokenType::ClosingCurlyBrace
            ) {
                if state.delimiters.last() == Some(&op) {
                    _ = state.delimiters.pop();
                } else {
                    attribute_expected(
                        self,
                        state.syntax,
                        Some(token),
                        "matching attribute argument delimiter",
                    );
                    return AttributeStep::Finish;
                }
            }
        }
        state.tokens.push(token);
        self.rd_consume();
        if state.delimiters.is_empty() {
            AttributeStep::Finish
        } else if argument {
            AttributeStep::Argument
        } else {
            AttributeStep::Token
        }
    }

    /// GCC aligned (Microsoft's `__declspec(align(N))` too), and the
    /// constructor/destructor priority, have a C integer constant expression
    /// operand. Keep unknown vendor-prefixed attributes opaque, even if
    /// their name is `aligned`. Records the operand's name index when the
    /// operand at the current `(` is parsed as an expression.
    /// C99: §6.6p6, p. 95; PDF p. 107 (GNU attribute extension).
    fn rd_attribute_argument_starts(&mut self, state: &mut AttributeState<'tu, 'p>) -> bool {
        let name_is = |parser: &Self, token: &Token, names: &[&str]| {
            let name = parser
                .context
                .string_cache
                .at(token.contents)
                .trim_matches('_');
            names.contains(&name)
        };
        if matches!(state.position, Position::AfterPrefixedName)
            && state
                .tokens
                .get(state.tokens.len().saturating_sub(4))
                .is_none_or(|namespace| {
                    let namespace = self
                        .context
                        .string_cache
                        .at(namespace.contents)
                        .trim_matches('_');
                    namespace != "gnu"
                        && !(namespace == "clang"
                            && state.tokens.last().is_some_and(|name| {
                                name_is(
                                    self,
                                    name,
                                    &[
                                        "pass_object_size",
                                        "pass_dynamic_object_size",
                                        "diagnose_if",
                                        "enable_if",
                                    ],
                                )
                            }))
                })
        {
            return false;
        }
        if state.tokens.last().is_some_and(|name| {
            name_is(
                self,
                name,
                &[
                    "aligned",
                    "constructor",
                    "destructor",
                    "nonnull",
                    "pass_object_size",
                    "pass_dynamic_object_size",
                    "diagnose_if",
                    "enable_if",
                ],
            ) || (name_is(self, name, &["align"]) && state.syntax == AttributeSyntax::Msvc)
        }) && self.cursor.following().is_some_and(|token| {
            !matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            )
        }) && self.cursor.alignment_argument_closes(self.arena)
        {
            state.argument_index = self.cursor.consumed - state.start - 1;
            return true;
        }
        false
    }

    /// Parses an integer operand, a nonnull index list, or a deferred call
    /// predicate together with its message and severity. Preserves the
    /// operand's original tokens alongside its parsed expression. Further
    /// comma-separated arguments stay balanced tokens, so semantic analysis
    /// reports their number as an argument error, as Clang does. Returns
    /// whether the specifier continues.
    /// C99: §6.6p6, p. 95; PDF p. 107 (GNU attribute extension).
    fn rd_attribute_argument(&mut self, state: &mut AttributeState<'tu, 'p>) -> Option<bool> {
        let list = state.tokens.iter().rev().nth(1).is_some_and(|token| {
            let name = self
                .context
                .string_cache
                .at(token.contents)
                .trim_matches('_');
            matches!(name, "nonnull" | "diagnose_if" | "enable_if")
        });
        if list
            && self.cursor.current().is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
            })
        {
            return Some(true);
        }
        let returned = self.rd_child(ParseFrame::Expression(ExpressionFrame::new(
            self.arena,
            if list {
                ExpressionMode::Expression
            } else {
                ExpressionMode::ConstantExpression
            },
            if list {
                ExpressionBoundary::ClosingParenthesis
            } else {
                ExpressionBoundary::Argument
            },
            self.hard_error_count,
        )))?;
        state.arguments.push(AttributeArgument {
            index:      state.argument_index,
            expression: any_expression_value(Some(returned)),
        });
        let token = self.cursor.current();
        if !token.is_some_and(|token| {
            matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::Comma
                )
            )
        }) {
            attribute_expected(self, state.syntax, token, closer_component(state.syntax));
            if token.is_none_or(|token| self.rd_attribute_caller_owns(state, token)) {
                return Some(false);
            }
        }
        Some(true)
    }

    /// Whether `token` ends the enclosing declaration rather than continuing
    /// the attribute: a `;` outside standard argument delimiters or in an
    /// argument that does not close, a `}` outside braces the attribute
    /// opened, or a `{` that cannot be an argument token. Standard balanced
    /// tokens allow semicolons, while GNU and MSVC arguments are expressions
    /// and retain semicolons as recovery boundaries.
    /// C23: balanced-token §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn rd_attribute_caller_owns(&self, state: &AttributeState<'tu, 'p>, token: Token) -> bool {
        let TokenType::Operator(op) = token.kind else {
            return false;
        };
        match op {
            | OperatorTokenType::Semicolon if state.syntax == AttributeSyntax::Standard =>
                state.delimiters.len() <= outer_depth(state.syntax)
                    || !self.rd_standard_argument_closes(state),
            | OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace => !state
                .delimiters
                .contains(&OperatorTokenType::ClosingCurlyBrace),
            | OperatorTokenType::OpeningCurlyBrace =>
                state.syntax != AttributeSyntax::Standard
                    || state.delimiters.len() <= outer_depth(state.syntax),
            | _ => false,
        }
    }

    /// Whether the open standard attribute argument closes after the current
    /// `;`. An unclosed argument leaves the `;` to the enclosing declaration,
    /// so malformed input cannot swallow the declarations that follow it. The
    /// scan is bounded; a longer argument is treated as unclosed.
    /// C23: balanced-token §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn rd_standard_argument_closes(&self, state: &AttributeState<'tu, 'p>) -> bool {
        const SCAN_LIMIT: usize = 1024;
        let mut open = &state.delimiters[outer_depth(state.syntax)..];
        let mut nested = 0_usize;
        for offset in 0..SCAN_LIMIT {
            let Some(token) = self.cursor.lookahead(offset) else {
                return false;
            };
            let TokenType::Operator(op) = token.kind else {
                continue;
            };
            match op {
                | OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
                | OperatorTokenType::OpeningCurlyBrace => nested += 1,
                | OperatorTokenType::ClosingParenthesis
                | OperatorTokenType::ClosingSquareBracket
                | OperatorTokenType::ClosingCurlyBrace =>
                    if nested > 0 {
                        nested -= 1;
                    } else if let Some((&closing, rest)) = open.split_last()
                        && closing == op
                    {
                        open = rest;
                        if open.is_empty() {
                            return true;
                        }
                    } else {
                        return false;
                    },
                | _ => {},
            }
        }
        false
    }
}

/// An attribute specifier under construction.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
struct AttributeState<'tu, 'p> {
    syntax:         AttributeSyntax,
    position:       Position,
    /// Consumed-token count before the introducer.
    start:          usize,
    argument_index: usize,
    tokens:         ArenaVec<'p, Token>,
    delimiters:     ArenaVec<'p, OperatorTokenType>,
    arguments:      ArenaVec<'p, AttributeArgument<'tu>>,
}

/// Positions in the outer attribute token grammar.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Clone, Copy)]
enum Position {
    Opening,
    Name,
    AfterName,
    SecondColon,
    PrefixedName,
    AfterPrefixedName,
    AfterArguments,
    Closing,
}

enum AttributeStep {
    Token,
    Argument,
    Finish,
}

fn outer_depth(syntax: AttributeSyntax) -> usize {
    if syntax == AttributeSyntax::Msvc {
        1
    } else {
        2
    }
}

fn closer_component(syntax: AttributeSyntax) -> &'static str {
    match syntax {
        | AttributeSyntax::Msvc => "`)` in MSVC declspec specifier",
        | AttributeSyntax::Gnu => "`))` in GNU attribute specifier",
        | AttributeSyntax::Standard => "`]]` in attribute specifier",
    }
}

#[cold]
#[inline(never)]
fn iso_expected(parser: &mut Parser<'_, '_, '_>, token: Option<Token>, position: &'static str) {
    parser.report(
        ParserErrorType::ExpectedIsoSyntax(position, token.map(|x| x.kind)),
        token,
    );
}

#[cold]
#[inline(never)]
fn attribute_expected(
    parser: &mut Parser<'_, '_, '_>,
    syntax: AttributeSyntax,
    token: Option<Token>,
    component: &'static str,
) {
    let error = match syntax {
        | AttributeSyntax::Msvc =>
            ParserErrorType::ExpectedMsSyntax(component, token.map(|x| x.kind)),
        | AttributeSyntax::Gnu =>
            ParserErrorType::ExpectedGnuSyntax(component, token.map(|x| x.kind)),
        | AttributeSyntax::Standard =>
            ParserErrorType::ExpectedIsoSyntax(component, token.map(|x| x.kind)),
    };
    parser.report(error, token);
}
