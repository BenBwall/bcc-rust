//! Recursive initializer and designation grammar for the phase-7 study.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `initializer`, `initializer-list`, `designation`, `designator-list`, and
//! `designator` (C99: §6.7.8 paragraph 1, p. 125; PDF p. 137; §A.2.2,
//! pp. 414-415; PDF pp. 426-427), for init-declarators and for the braced
//! list of a compound literal (§6.5.2.5 paragraph 4, p. 75; PDF p. 87).
//!
//! Diagnosed here: syntax the grammar rejects, including an empty `{}`,
//! which C99 does not allow. Left to semantic analysis: the constraints of
//! §6.7.8 paragraphs 2-7, p. 125; PDF p. 137, and the current-object,
//! ordering, and implicit-initialization semantics of paragraphs 8-23,
//! pp. 126-128; PDF pp. 138-140. Nested braced lists recurse natively.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                BracedInitializerList,
                Designation,
                Designator,
                DesignatorType,
                Initializer,
                InitializerElement,
                InitializerType,
                RangeDesignator,
            },
            errors::ParserErrorType,
            grammar::{
                ProductionKind,
                expression_operators::{
                    binary_operator,
                    is_expression_operand_starter,
                    is_postfix_starter,
                },
                expressions::{
                    ExpressionBoundary,
                    ExpressionMode,
                    ExpressionResult,
                    is_strong_grammar_boundary_for,
                },
                statements::is_statement_keyword,
            },
            recovery::DelimiterDepth,
            scratch_pool::PoolBox,
            syntax::{
                ConstantExpression,
                ExpressionType,
                Identifier,
            },
        },
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

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses one `initializer`: an `assignment-expression` or a braced
    /// `initializer-list` with an optional trailing comma. `)` and `]` end
    /// the initializer when the caller owns them.
    ///
    /// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
    pub(super) fn rd_initializer(
        &mut self,
        starting_error_count: usize,
        closing_parenthesis_is_caller_boundary: bool,
        closing_square_bracket_is_caller_boundary: bool,
    ) -> Option<InitializerResult<'tu>> {
        let entry = self.enter_production(ProductionKind::Initializer)?;
        let result = if let Some(opening) = self
            .cursor
            .current()
            .filter(|token| is_operator_token(*token, OperatorTokenType::OpeningCurlyBrace))
        {
            let mut list = BracedList {
                elements: ArenaVec::new_in(self.arena),
                source_vectors: ArenaVec::new_in(self.arena),
                opening_brace_source_vectors: Some(opening.source_vectors),
                closing_brace_source_vectors: None,
                designation: self.pools.take_designation(),
                range_lower: None,
                closing_parenthesis_is_caller_boundary,
                closing_square_bracket_is_caller_boundary,
            };
            self.pools.initializers.lend(&mut list.elements);
            self.pools.source_vectors.lend(&mut list.source_vectors);
            list.source_vectors.push(opening.source_vectors);
            self.rd_consume();
            let result = self.rd_braced_initializer(&mut list, starting_error_count);
            self.pools.initializers.reclaim(&mut list.elements);
            self.pools.source_vectors.reclaim(&mut list.source_vectors);
            let mut designation = list.designation;
            designation.reset();
            self.pools.designations.reclaim(designation);
            result
        } else {
            self.rd_scalar_initializer(starting_error_count)
        };
        self.leave_production(entry);
        result
    }

    /// C99 §6.7.8p1: a scalar initializer is an `assignment-expression`.
    fn rd_scalar_initializer(
        &mut self,
        starting_error_count: usize,
    ) -> Option<InitializerResult<'tu>> {
        let expression = self
            .rd_expression_ending_at(
                ExpressionMode::AssignmentExpression,
                ExpressionBoundary::Initializer,
            )?
            .expression;
        let recovered = self.hard_error_count > starting_error_count;
        let initializer = self.alloc_syntax(Initializer {
            kind: InitializerType::AssignmentExpression(expression),
            source_vectors: expression.source_vectors,
            recovered,
        });
        Some(InitializerResult { initializer })
    }

    /// Parses the elements and the closing `}` of a braced list whose `{`
    /// was consumed.
    ///
    /// C99: `{ initializer-list }` and `{ initializer-list , }`, §6.7.8
    /// paragraph 1, p. 125; PDF p. 137.
    fn rd_braced_initializer(
        &mut self,
        list: &mut BracedList<'tu, 'p>,
        starting_error_count: usize,
    ) -> Option<InitializerResult<'tu>> {
        let mut next = ListStep::ElementOrClose;
        loop {
            next = match next {
                | ListStep::ElementOrClose => self.rd_element_or_close(list)?,
                | ListStep::SkipMalformedElement => {
                    self.rd_skip_malformed_element(list);
                    ListStep::Separator
                },
                | ListStep::Separator => self.rd_initializer_separator(list),
                | ListStep::Finish => break,
            };
        }
        let elements = self.alloc_syntax_list(&mut list.elements);
        let braced = self.alloc_syntax_part(BracedInitializerList {
            elements,
            opening_brace_source_vectors: list.opening_brace_source_vectors,
            closing_brace_source_vectors: list.closing_brace_source_vectors,
        });
        let source_vectors = self.context.merge_vector_list(&list.source_vectors);
        let recovered = self.hard_error_count > starting_error_count;
        let initializer = self.alloc_syntax(Initializer {
            kind: InitializerType::InitializerList(braced),
            source_vectors,
            recovered,
        });
        Some(InitializerResult { initializer })
    }

    fn rd_element_or_close(&mut self, list: &mut BracedList<'tu, 'p>) -> Option<ListStep> {
        loop {
            let token = self.cursor.current();
            if let Some(close) = token
                .filter(|token| is_operator_token(*token, OperatorTokenType::ClosingCurlyBrace))
            {
                // C99 §6.7.8p1: an initializer-list has at least one
                // initializer; `{}` is not C99.
                if list.elements.is_empty() {
                    self.extension(Feature::EmptyInitializers, "empty initializer", close);
                }
                list.closing_brace_source_vectors = Some(close.source_vectors);
                list.source_vectors.push(close.source_vectors);
                self.rd_consume();
                return Some(ListStep::Finish);
            }
            if list.is_unowned_closing_delimiter(token) {
                self.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "initializer element",
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                list.source_vectors.push(
                    token
                        .expect("an unowned closing delimiter exists")
                        .source_vectors,
                );
                self.rd_consume();
                continue;
            }
            match self.rd_list_boundary(list, token, true) {
                | ListBoundary::None => {},
                | ListBoundary::MissingClose => {
                    self.report(
                        ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    return Some(ListStep::Finish);
                },
                | ListBoundary::MalformedElement => {
                    // C99 §6.7.8p1: an initializer is an
                    // assignment-expression or a braced list.
                    self.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "initializer element",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                    return Some(ListStep::SkipMalformedElement);
                },
            }
            list.designation.reset();
            self.rd_designation(list)?;
            let element = self.rd_initializer(
                self.hard_error_count,
                list.closing_parenthesis_is_caller_boundary,
                list.closing_square_bracket_is_caller_boundary,
            )?;
            self.rd_push_element(list, element.initializer);
            return Some(ListStep::Separator);
        }
    }

    /// Parses an optional designation before one element, including its
    /// `=` or the GNU forms without it.
    ///
    /// C99: `designation` is `designator-list =`, §6.7.8 paragraph 1,
    /// p. 125; PDF p. 137.
    fn rd_designation(&mut self, list: &mut BracedList<'tu, 'p>) -> Option<()> {
        loop {
            let token = self.cursor.current();
            if let Some(token) = token
                && matches!(token.kind, TokenType::Identifier)
                && self.cursor.following().is_some_and(|x| {
                    matches!(x.kind, TokenType::Operator(OperatorTokenType::Colon))
                })
            {
                self.extension(Feature::GnuDesignators, "old-style field designator", token);
                list.merge_designation_source(self, token.source_vectors);
                list.designation.current_designators.push(Designator {
                    kind: DesignatorType::GnuField(Identifier::from_token(token)),
                    operator_source_vectors: token.source_vectors,
                    closing_bracket_source_vectors: None,
                    source_vectors: token.source_vectors,
                    recovered: false,
                });
                self.rd_consume();
                if let Some(colon) = self.cursor.current() {
                    list.merge_designation_source(self, colon.source_vectors);
                    let designator = list
                        .designation
                        .current_designators
                        .last_mut()
                        .expect("old field has a designator");
                    designator.source_vectors = self
                        .context
                        .merge_vectors(designator.source_vectors, colon.source_vectors);
                }
                self.rd_finish_designation(list);
                self.rd_consume();
                return Some(());
            }
            if let Some(period) =
                token.filter(|token| is_operator_token(*token, OperatorTokenType::Period))
            {
                self.extension(
                    Feature::DesignatedInitializers,
                    "designated initializer",
                    period,
                );
                list.merge_designation_source(self, period.source_vectors);
                list.designation.current_designator_source = Some(period.source_vectors);
                self.rd_consume();
                self.rd_field_designator(list);
                continue;
            }
            if let Some(opening) = token
                .filter(|token| is_operator_token(*token, OperatorTokenType::OpeningSquareBracket))
            {
                self.extension(
                    Feature::DesignatedInitializers,
                    "designated initializer",
                    opening,
                );
                list.merge_designation_source(self, opening.source_vectors);
                list.designation.current_designator_source = Some(opening.source_vectors);
                self.rd_consume();
                self.rd_array_designator(list)?;
                continue;
            }
            if !list.designation.current_designators.is_empty() {
                self.rd_designation_equals(list);
            }
            return Some(());
        }
    }

    /// C99 §6.7.8p1: designator `. identifier`, after its `.`.
    fn rd_field_designator(&mut self, list: &mut BracedList<'tu, 'p>) {
        let token = self.cursor.current();
        let Some(identifier) = token.filter(|token| matches!(token.kind, TokenType::Identifier))
        else {
            self.report(
                ParserErrorType::ExpectedMemberIdentifier(token.map(|token| token.kind)),
                token,
            );
            let operator_source_vectors = list
                .designation
                .current_designator_source
                .take()
                .unwrap_or_default();
            list.designation.current_designators.push(Designator {
                kind: DesignatorType::Error,
                operator_source_vectors,
                closing_bracket_source_vectors: None,
                source_vectors: operator_source_vectors,
                recovered: true,
            });
            list.designation.current_designation_recovered = true;
            return;
        };
        list.merge_designation_source(self, identifier.source_vectors);
        let operator_source_vectors = list
            .designation
            .current_designator_source
            .take()
            .unwrap_or_default();
        let source_vectors = self
            .context
            .merge_vectors(operator_source_vectors, identifier.source_vectors);
        list.designation.current_designators.push(Designator {
            kind: DesignatorType::Field(Identifier::from_token(identifier)),
            operator_source_vectors,
            closing_bracket_source_vectors: None,
            source_vectors,
            recovered: false,
        });
        self.rd_consume();
    }

    /// Parses `constant-expression ]` after a designator's `[`, continuing
    /// into a GNU range when `...` follows the first expression, as in
    /// `[1 ... 3]`.
    ///
    /// C99: designator `[ constant-expression ]`, §6.7.8 paragraph 1,
    /// p. 125; PDF p. 137; ranges are an extension to it.
    fn rd_array_designator(&mut self, list: &mut BracedList<'tu, 'p>) -> Option<()> {
        let (expression, expression_recovered) = loop {
            let ExpressionResult {
                expression,
                recovered,
            } = self.rd_expression(
                ExpressionMode::ConstantExpression,
                ExpressionBoundary::Designator,
                ExpressionBoundary::Initializer,
            )?;
            let expression = ConstantExpression(expression);
            let Some(ellipsis) = self
                .cursor
                .current()
                .filter(|token| is_operator_token(*token, OperatorTokenType::Ellipsis))
            else {
                break (expression, recovered);
            };
            if list.range_lower.is_some() {
                self.report(
                    ParserErrorType::ExpectedGnuSyntax(
                        "one range per designator",
                        Some(ellipsis.kind),
                    ),
                    Some(ellipsis),
                );
            } else {
                list.range_lower = Some(expression);
            }
            self.extension(Feature::GnuDesignators, "array range designator", ellipsis);
            let lower_source = expression.expression().source_vectors;
            list.merge_designation_source(self, lower_source);
            list.merge_designation_source(self, ellipsis.source_vectors);
            let opening = list
                .designation
                .current_designator_source
                .unwrap_or_default();
            let lower = self.context.merge_vectors(opening, lower_source);
            list.designation.current_designator_source =
                Some(self.context.merge_vectors(lower, ellipsis.source_vectors));
            self.rd_consume();
        };
        let expression_source = expression.expression().source_vectors;
        let operator_source_vectors = list
            .designation
            .current_designator_source
            .unwrap_or_default();
        let mut source_vectors = self
            .context
            .merge_vectors(operator_source_vectors, expression_source);
        list.merge_designation_source(self, expression_source);
        let token = self.cursor.current();
        if let Some(close) =
            token.filter(|token| is_operator_token(*token, OperatorTokenType::ClosingSquareBracket))
        {
            source_vectors = self
                .context
                .merge_vectors(source_vectors, close.source_vectors);
            list.merge_designation_source(self, close.source_vectors);
            self.rd_push_array_designator(
                list,
                expression,
                source_vectors,
                Some(close.source_vectors),
                expression_recovered,
            );
            self.rd_consume();
            return Some(());
        }
        self.report(
            ParserErrorType::ExpectedClosingSquareBracketInArrayDesignator(
                token.map(|token| token.kind),
            ),
            token,
        );
        let closing_bracket_follows = closing_bracket_follows(self, token);
        let mut depth = DelimiterDepth::default();
        loop {
            let token = self.cursor.current();
            if let Some(close) = token
                .filter(|token| is_operator_token(*token, OperatorTokenType::ClosingSquareBracket))
            {
                source_vectors = self
                    .context
                    .merge_vectors(source_vectors, close.source_vectors);
                list.merge_designation_source(self, close.source_vectors);
                if depth.brackets == 0 {
                    self.rd_push_array_designator(
                        list,
                        expression,
                        source_vectors,
                        Some(close.source_vectors),
                        true,
                    );
                    self.rd_consume();
                    return Some(());
                }
                depth.brackets -= 1;
                self.rd_consume();
                continue;
            }
            // Nested delimiters cannot transfer a comma, `=`, or weak grammar
            // boundary to the surrounding initializer. Owning outer
            // boundaries still stop the scan despite an unmatched sibling
            // delimiter.
            let Some(token) = token.filter(|&token| {
                !self.rd_at_array_designator_sync_boundary(
                    list,
                    token,
                    depth,
                    closing_bracket_follows,
                )
            }) else {
                self.rd_push_array_designator(list, expression, source_vectors, None, true);
                return Some(());
            };
            match token.kind {
                | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                    depth.parentheses += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                    if depth.parentheses > 0 =>
                {
                    depth.parentheses -= 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                    depth.brackets += 1;
                },
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    depth.braces += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if depth.braces > 0 => {
                    depth.braces -= 1;
                },
                | _ => {},
            }
            source_vectors = self
                .context
                .merge_vectors(source_vectors, token.source_vectors);
            list.merge_designation_source(self, token.source_vectors);
            self.rd_consume();
        }
    }

    /// Ends a designator-list at its `=`, or accepts the GNU omission of
    /// `=` after one array designator.
    ///
    /// C99: `designation` is `designator-list =`, §6.7.8 paragraph 1,
    /// p. 125; PDF p. 137.
    fn rd_designation_equals(&mut self, list: &mut BracedList<'tu, 'p>) {
        let token = self.cursor.current();
        let consume = if let Some(equals) =
            token.filter(|token| is_operator_token(*token, OperatorTokenType::Equals))
        {
            list.designation.designation_equals_source_vectors = Some(equals.source_vectors);
            list.merge_designation_source(self, equals.source_vectors);
            true
        } else if matches!(
            list.designation.current_designators.as_slice(),
            [Designator {
                kind: DesignatorType::Array(_) | DesignatorType::Range(_),
                ..
            }]
        ) && token.is_some_and(|x| {
            is_expression_operand_starter(x.kind)
                || matches!(
                    x.kind,
                    TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                )
        }) {
            // GNU omission applies only to one array designator, including
            // a range; fields use the separate `field:` form.
            self.extension(
                Feature::GnuDesignators,
                "designator without equals",
                token.expect("initializer exists"),
            );
            false
        } else {
            self.report(
                ParserErrorType::ExpectedEqualsAfterInitializerDesignation(
                    token.map(|token| token.kind),
                ),
                token,
            );
            list.designation.current_designation_recovered = true;
            false
        };
        self.rd_finish_designation(list);
        if consume {
            self.rd_consume();
        }
    }

    /// Skips a malformed element that a declaration or statement keyword
    /// starts while this list's own `}` still follows, and stores it as an
    /// error element.
    fn rd_skip_malformed_element(&mut self, list: &mut BracedList<'tu, 'p>) {
        let mut nesting = 0_u32;
        let mut source = None;
        loop {
            let token = self.cursor.current();
            let stops = token.is_none_or(|token| match token.kind {
                | TokenType::Operator(OperatorTokenType::Semicolon) => true,
                | TokenType::Operator(
                    OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace,
                ) => nesting == 0,
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
                ) => nesting == 0 && list.at_caller_boundary(Some(token)),
                | _ => false,
            });
            if stops {
                break;
            }
            let token = token.expect("a skipped token exists");
            match token.kind {
                | TokenType::Operator(
                    OperatorTokenType::OpeningParenthesis
                    | OperatorTokenType::OpeningSquareBracket
                    | OperatorTokenType::OpeningCurlyBrace,
                ) => nesting = nesting.saturating_add(1),
                | TokenType::Operator(
                    OperatorTokenType::ClosingParenthesis
                    | OperatorTokenType::ClosingSquareBracket
                    | OperatorTokenType::ClosingCurlyBrace,
                ) => nesting = nesting.saturating_sub(1),
                | _ => {},
            }
            self.context.merge_into(&mut source, token.source_vectors);
            self.rd_consume();
        }
        let source_vectors = source.unwrap_or_else(|| self.missing_syntax_source());
        let expression = self.store_expression(ExpressionType::Error, source_vectors, None, true);
        let element = self.alloc_syntax(Initializer {
            kind: InitializerType::AssignmentExpression(expression),
            source_vectors,
            recovered: true,
        });
        self.rd_push_element(list, element);
    }

    /// C99 §6.7.8p1: `,` continues the list, and a `,` before `}` is the
    /// trailing-comma form.
    fn rd_initializer_separator(&mut self, list: &mut BracedList<'tu, 'p>) -> ListStep {
        loop {
            let token = self.cursor.current();
            if let Some(comma) =
                token.filter(|token| is_operator_token(*token, OperatorTokenType::Comma))
            {
                list.elements
                    .last_mut()
                    .expect("a separator follows one initializer element")
                    .comma_source_vectors = Some(comma.source_vectors);
                list.source_vectors.push(comma.source_vectors);
                self.rd_consume();
                return ListStep::ElementOrClose;
            }
            if let Some(close) = token
                .filter(|token| is_operator_token(*token, OperatorTokenType::ClosingCurlyBrace))
            {
                list.closing_brace_source_vectors = Some(close.source_vectors);
                list.source_vectors.push(close.source_vectors);
                self.rd_consume();
                return ListStep::Finish;
            }
            if list.is_unowned_closing_delimiter(token) {
                self.report(
                    ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                list.source_vectors.push(
                    token
                        .expect("an unowned closing delimiter exists")
                        .source_vectors,
                );
                // The element before the stray closer is complete, so a
                // following `,` or `}` is still this list's separator.
                self.rd_consume();
                continue;
            }
            let boundary = self.rd_list_boundary(list, token, false);
            self.report(
                ParserErrorType::ExpectedClosingCurlyBraceInInitializerList(
                    token.map(|token| token.kind),
                ),
                token,
            );
            return match boundary {
                | ListBoundary::MissingClose => ListStep::Finish,
                // The `,` is missing before a malformed element; skip it as
                // one element so it is not diagnosed again.
                | ListBoundary::MalformedElement => ListStep::SkipMalformedElement,
                | ListBoundary::None => ListStep::ElementOrClose,
            };
        }
    }

    /// Appends one element, with any designation that preceded it.
    fn rd_push_element(&mut self, list: &mut BracedList<'tu, 'p>, element: &'tu Initializer<'tu>) {
        let initializer_source = element.source_vectors;
        let current_designation = list.designation.current_designation.take();
        let source_vectors = current_designation.map_or(initializer_source, |designation| {
            self.context
                .merge_vectors(designation.source_vectors, initializer_source)
        });
        list.elements.push(InitializerElement {
            designation: current_designation,
            initializer: element,
            comma_source_vectors: None,
            source_vectors,
        });
        list.source_vectors.push(source_vectors);
    }

    fn rd_push_array_designator(
        &mut self,
        list: &mut BracedList<'tu, 'p>,
        expression: ConstantExpression<'tu>,
        source_vectors: SourceVectors,
        closing_bracket_source_vectors: Option<SourceVectors>,
        recovered: bool,
    ) {
        let kind = if let Some(lower) = list.range_lower.take() {
            DesignatorType::Range(self.alloc_syntax(RangeDesignator {
                lower,
                upper: expression,
            }))
        } else {
            DesignatorType::Array(expression)
        };
        let designation = &mut *list.designation;
        let operator_source_vectors = designation
            .current_designator_source
            .take()
            .unwrap_or_default();
        designation.current_designators.push(Designator {
            kind,
            operator_source_vectors,
            closing_bracket_source_vectors,
            source_vectors,
            recovered,
        });
        designation.current_designation_recovered |= recovered;
    }

    fn rd_finish_designation(&mut self, list: &mut BracedList<'tu, 'p>) {
        let designation = &mut list.designation;
        let recovered = designation.current_designation_recovered
            || designation
                .current_designators
                .iter()
                .any(|designator| designator.recovered);
        let designators = self.alloc_syntax_list(&mut designation.current_designators);
        let index = self.alloc_syntax(Designation {
            designators,
            equals_source_vectors: designation.designation_equals_source_vectors.take(),
            source_vectors: designation
                .designation_source_vectors
                .take()
                .unwrap_or_default(),
            recovered,
        });
        designation.current_designation = Some(index);
    }

    fn rd_at_array_designator_sync_boundary(
        &mut self,
        list: &BracedList<'tu, 'p>,
        token: Token,
        depth: DelimiterDepth,
        closing_bracket_follows: bool,
    ) -> bool {
        let at_top_level = depth.is_top_level();
        at_top_level
            && !closing_bracket_follows
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Equals | OperatorTokenType::Comma)
            )
            || matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace
                )
            )
            || list.closing_parenthesis_is_caller_boundary
                && depth.parentheses == 0
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
            || at_top_level
                && (is_statement_keyword(token.kind)
                    || is_strong_grammar_boundary_for(self, token, ExpressionBoundary::Initializer))
    }

    /// Classifies a token that may end this list. `at_element` is whether an
    /// element or the closing `}` is expected, rather than a separator.
    fn rd_list_boundary(
        &mut self,
        list: &BracedList<'tu, 'p>,
        token: Option<Token>,
        at_element: bool,
    ) -> ListBoundary {
        if list.at_caller_boundary(token) {
            return ListBoundary::MissingClose;
        }
        let Some(token) = token else {
            return ListBoundary::MissingClose;
        };
        if matches!(token.kind, TokenType::Identifier)
            && self.cursor.following().is_some_and(|following| {
                matches!(
                    following.kind,
                    TokenType::Operator(OperatorTokenType::Colon)
                )
            })
        {
            return if at_element && gnu_designator_element_follows(self) {
                ListBoundary::None
            } else {
                ListBoundary::MissingClose
            };
        }
        let identifier_continues_initializer = matches!(token.kind, TokenType::Identifier)
            && self.cursor.following().is_some_and(|following| {
                binary_operator(following.kind).is_some() || is_postfix_starter(following.kind)
            });
        let starts_declaration_or_statement = is_statement_keyword(token.kind)
            || self.declaration_recovery_starts_here(token)
                && !identifier_continues_initializer
                && !matches!(
                    self.cursor.following().map(|following| following.kind),
                    Some(TokenType::Operator(
                        OperatorTokenType::Comma
                            | OperatorTokenType::Semicolon
                            | OperatorTokenType::ClosingCurlyBrace
                    ))
                );
        if !starts_declaration_or_statement {
            ListBoundary::None
        } else if closing_brace_follows(self) {
            ListBoundary::MalformedElement
        } else {
            ListBoundary::MissingClose
        }
    }
}

/// One braced initializer list under construction.
///
/// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
struct BracedList<'tu, 'p> {
    elements: ArenaVec<'p, InitializerElement<'tu>>,
    source_vectors: ArenaVec<'p, SourceVectors>,
    opening_brace_source_vectors: Option<SourceVectors>,
    closing_brace_source_vectors: Option<SourceVectors>,
    /// Designation of the element being parsed, reused across elements.
    designation: PoolBox<'p, DesignationState<'tu, 'p>>,
    range_lower: Option<ConstantExpression<'tu>>,
    /// Whether `)` belongs to an enclosing expression or `for` header.
    closing_parenthesis_is_caller_boundary: bool,
    /// Whether `]` belongs to an enclosing expression.
    closing_square_bracket_is_caller_boundary: bool,
}

/// The next part of a braced list to parse.
#[derive(Clone, Copy)]
enum ListStep {
    ElementOrClose,
    SkipMalformedElement,
    Separator,
    Finish,
}

/// How a token that cannot continue an initializer list relates to it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListBoundary {
    /// The token belongs to the list.
    None,
    /// The token belongs to the caller or to a following declaration or
    /// statement, so the list's `}` is missing.
    MissingClose,
    /// The token looks like a declaration or statement, but this list's
    /// `}` follows before any `;`, so it starts one malformed element.
    MalformedElement,
}

impl BracedList<'_, '_> {
    fn merge_designation_source(&mut self, parser: &mut Parser<'_, '_, '_>, source: SourceVectors) {
        parser
            .context
            .merge_into(&mut self.designation.designation_source_vectors, source);
    }

    fn at_caller_boundary(&self, token: Option<Token>) -> bool {
        token.is_none_or(|token| {
            matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Semicolon)
            ) || self.closing_parenthesis_is_caller_boundary
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
                || self.closing_square_bracket_is_caller_boundary
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                    )
        })
    }

    fn is_unowned_closing_delimiter(&self, token: Option<Token>) -> bool {
        token.is_some_and(|token| {
            !self.closing_parenthesis_is_caller_boundary
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
                || !self.closing_square_bracket_is_caller_boundary
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                    )
        })
    }
}

fn is_operator_token(token: Token, operator: OperatorTokenType) -> bool {
    matches!(token.kind, TokenType::Operator(found) if found == operator)
}

/// Designators and provenance of the designation preceding one initializer
/// element.
///
/// C99: `designation` and `designator-list`, §6.7.8 paragraph 1, p. 125;
/// PDF p. 137.
#[derive(Debug)]
pub(in crate::translation_phases::parsing) struct DesignationState<'tu, 'p> {
    pub(in crate::translation_phases::parsing) current_designators: ArenaVec<'p, Designator<'tu>>,
    pub(in crate::translation_phases::parsing) current_designation: Option<&'tu Designation<'tu>>,
    pub(in crate::translation_phases::parsing) designation_source_vectors: Option<SourceVectors>,
    pub(in crate::translation_phases::parsing) designation_equals_source_vectors:
        Option<SourceVectors>,
    pub(in crate::translation_phases::parsing) current_designator_source: Option<SourceVectors>,
    pub(in crate::translation_phases::parsing) current_designation_recovered:     bool,
}

impl<'p> DesignationState<'_, 'p> {
    /// Clears the state for the next element while keeping list capacity.
    pub(in crate::translation_phases::parsing) fn reset(&mut self) {
        self.current_designators.clear();
        self.current_designation = None;
        self.designation_source_vectors = None;
        self.designation_equals_source_vectors = None;
        self.current_designator_source = None;
        self.current_designation_recovered = false;
    }

    pub(in crate::translation_phases::parsing) fn new_in(arena: &'p Bump) -> Self {
        Self {
            current_designators:               ArenaVec::new_in(arena),
            current_designation:               None,
            designation_source_vectors:        None,
            designation_equals_source_vectors: None,
            current_designator_source:         None,
            current_designation_recovered:     false,
        }
    }
}

/// Whether this array designator's `]` follows `token` before the list,
/// declaration, or an enclosing delimiter ends.
///
/// C99 §6.7.8p1: a designator is `[ constant-expression ]`, so a
/// top-level `,` or `=` inside one is a single syntax error when a `]`
/// still closes it, e.g. `[1, 2] = 3`. Without that `]`, as in
/// `{ [1 = 2 }`, the `,` or `=` is where the bracket went missing. The
/// scan is bounded so repeated errors in one long list stay linear.
pub(in crate::translation_phases::parsing) fn closing_bracket_follows(
    parser: &mut Parser<'_, '_, '_>,
    token: Option<Token>,
) -> bool {
    const SCAN_LIMIT: usize = 32;
    let mut depth = DelimiterDepth::default();
    let mut next = token;
    for index in 0..SCAN_LIMIT {
        let Some(token) = next else {
            return false;
        };
        let at_top_level = depth.is_top_level();
        match token.kind {
            | TokenType::Operator(OperatorTokenType::ClosingSquareBracket) => {
                if depth.brackets == 0 {
                    return at_top_level;
                }
                depth.brackets -= 1;
            },
            | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                depth.brackets += 1;
            },
            | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => {
                depth.parentheses += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis) => {
                if depth.parentheses == 0 {
                    return false;
                }
                depth.parentheses -= 1;
            },
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                depth.braces += 1;
            },
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                if depth.braces == 0 {
                    return false;
                }
                depth.braces -= 1;
            },
            | TokenType::Operator(OperatorTokenType::Semicolon) => return false,
            | kind if at_top_level
                && (is_statement_keyword(kind)
                    || !matches!(kind, TokenType::Identifier)
                        && parser.declaration_starter(token)) =>
            {
                return false;
            },
            | _ => {},
        }
        next = parser.cursor.lookahead(index);
    }
    false
}

/// Whether the current `identifier :` begins a GNU `field: value`
/// element rather than a label after an unclosed list: its value ends at
/// a `,` or `}` before any `;` or statement keyword at depth zero.
/// A scan that runs out of lookahead keeps the element, so long values
/// stay initializers.
///
/// C99: labels are §6.8.1 paragraph 1, p. 131; PDF p. 143. The
/// initializer-list grammar (§6.7.8 paragraph 1, p. 125; PDF p. 137)
/// has no top-level `;`. GNU `field:` designators and statement-expression
/// values extend that grammar; nested delimiters may contain statements.
pub(in crate::translation_phases::parsing) fn gnu_designator_element_follows(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    const SCAN_LIMIT: usize = 64;
    let mut parentheses = 0_u32;
    let mut braces = 0_u32;
    // Lookahead 0 is the colon.
    for index in 1..SCAN_LIMIT {
        let Some(token) = parser.cursor.lookahead(index) else {
            return false;
        };
        match token.kind {
            | TokenType::Operator(
                OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
            ) => parentheses += 1,
            | TokenType::Operator(
                OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
            ) => {
                if parentheses == 0 {
                    return false;
                }
                parentheses -= 1;
            },
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => braces += 1,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                if braces == 0 {
                    return parentheses == 0;
                }
                braces -= 1;
            },
            | TokenType::Operator(OperatorTokenType::Comma) if parentheses == 0 && braces == 0 =>
                return true,
            | TokenType::Operator(OperatorTokenType::Semicolon)
                if parentheses == 0 && braces == 0 =>
                return false,
            | kind if parentheses == 0 && braces == 0 && is_statement_keyword(kind) =>
                return false,
            | _ => {},
        }
    }
    true
}

/// Whether this list's `}` follows the current token before a `;`, a
/// statement keyword, or an unmatched closer.
///
/// C99 §6.7.8p1: no initializer contains a `;`, so a keyword that would
/// otherwise start a following declaration or statement is a malformed
/// element when the list's own `}` comes first, as in `{ 1, int 0 }`.
/// The scan is bounded so repeated errors in one long list stay linear.
pub(in crate::translation_phases::parsing) fn closing_brace_follows(
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
            | kind if is_statement_keyword(kind) => return false,
            | _ => {},
        }
    }
    false
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InitializerResult<'tu> {
    pub(super) initializer: &'tu Initializer<'tu>,
}
