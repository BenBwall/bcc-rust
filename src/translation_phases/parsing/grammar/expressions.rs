//! Recursive phase-7 expression grammar with a Double-E precedence reducer.
//!
//! C99: §6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106; §A.2.1, pp. 409-411;
//! PDF pp. 421-423. Each expression alternates between expecting an
//! operand and expecting an operator, keeping both on pooled parse-arena
//! stacks; binary operators reduce by precedence (§6.5 paragraph 3 and
//! footnote 74, p. 67; PDF p. 79). Grouping, subscripts, call arguments,
//! prefix, cast and conditional operands recurse. The caller chooses which
//! delimiters end the expression and keeps ownership of them. Type names,
//! initializers and extension operands call their own productions;
//! operand constraints and constant evaluation belong to semantic analysis.

// Recovery lookahead
mod lookahead;

pub(in crate::translation_phases::parsing) use lookahead::{
    brace_group_closes_before_parenthesis,
    brace_group_continues_expression,
    brace_group_is_block,
    closer_follows_stray_run,
    is_owning_boundary_for,
    is_strong_grammar_boundary_for,
    parenthesized_compound_literal_follows,
    parenthesized_type_name_follows,
    type_name_is_compound_literal,
};

use crate::{
    configuration::{
        Feature,
        FeatureOrigin,
    },
    translation_phases::{
        GetPosition,
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                StructOrUnion,
                TypeName,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            grammar::{
                ProductionKind,
                expression_operators::{
                    LanguageExpressionOperator,
                    binary_operator,
                    is_assignment_operator,
                    is_expression_operand_starter,
                    is_operator,
                    is_postfix_starter,
                    prefix_operator,
                },
                gnu::{
                    GnuKind,
                    GnuValue,
                },
                initializers::InitializerResult,
                modern::{
                    ModernKind,
                    ModernValue,
                },
                statements::is_statement_keyword,
            },
            modern::SyntaxOperand,
            recovery::ExpressionTerminator,
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
        },
        preprocessing::{
            FloatTokenType,
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

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses an expression of `mode` that `boundary` ends, for a caller
    /// whose own recovery boundary is the same.
    pub(super) fn parse_expression_ending_at(
        &mut self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<ExpressionResult<'tu>> {
        self.parse_expression(mode, boundary, boundary)
    }

    /// Parses a `constant-expression` that `boundary` ends and owns.
    ///
    /// C99: §6.6 paragraph 1, p. 95; PDF p. 107.
    pub(super) fn parse_constant_expression(
        &mut self,
        boundary: ExpressionBoundary,
    ) -> Option<ConstantExpressionResult<'tu>> {
        let result =
            self.parse_expression_ending_at(ExpressionMode::ConstantExpression, boundary)?;
        Some(ConstantExpressionResult {
            expression: ConstantExpression(result.expression),
            recovered:  result.recovered,
        })
    }

    /// C99: `expression` and its subordinate nonterminals, §6.5, pp. 67-94;
    /// PDF pp. 79-106. `recovery` is the enclosing owner's boundary, which
    /// nested operands inherit for their recovery decisions.
    #[inline(never)]
    pub(super) fn parse_expression(
        &mut self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        recovery: ExpressionBoundary,
    ) -> Option<ExpressionResult<'tu>> {
        let active = self.enter_production(ProductionKind::Expression)?;
        let mut expression = ExpressionParts {
            mode,
            boundary,
            recovery,
            operators: ArenaVec::new_in(self.arena),
            operands: ArenaVec::new_in(self.arena),
            operand: true,
            errors: self.hard_error_count,
            pending_sizeof: None,
            stray_error_operand: false,
        };
        self.pools.operators.lend(&mut expression.operators);
        self.pools.operands.lend(&mut expression.operands);
        let result = self.parse_expression_body(&mut expression);
        self.pools.operators.reclaim(&mut expression.operators);
        self.pools.operands.reclaim(&mut expression.operands);
        self.leave_production(active);
        result
    }

    fn parse_expression_body(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
    ) -> Option<ExpressionResult<'tu>> {
        loop {
            let finished = if e.operand {
                self.parse_operand(e)?
            } else {
                self.parse_operator(e)?
            };
            if finished {
                return Some(self.finish_expression(e));
            }
        }
    }

    /// One step in operand position. Returns whether the expression ends.
    ///
    /// C99: `primary-expression` §6.5.1, p. 69; PDF p. 81; `unary-expression`
    /// §6.5.3, p. 78; PDF p. 90; `cast-expression` §6.5.4, p. 81; PDF p. 93.
    fn parse_operand(&mut self, e: &mut ExpressionParts<'tu, 'p>) -> Option<bool> {
        let token = self.cursor.current();
        if let Some(token) = token
            && is_closing_delimiter(token.kind)
            && e.consumes_unowned_closer(token.kind)
        {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "expression operand",
                    Some(token.kind),
                ),
                Some(token),
            );
            // The stray closer stands in for the missing operand.
            self.push_error_operand(e, Some(token));
            self.consume_token();
            return Some(true);
        }
        let Some(token) = token else {
            self.report(
                ParserErrorType::ExpectedStatementExpression("expression operand", None),
                None,
            );
            self.push_error_operand(e, None);
            return Some(true);
        };
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
            let GnuValue::Builtin(builtin) =
                self.parse_gnu(GnuKind::Builtin(keyword), self.hard_error_count)?
            else {
                unreachable!("builtin child protocol")
            };
            let expression = self.store_expression(
                ExpressionType::Builtin(builtin),
                builtin.source_vectors,
                None,
                builtin.recovered,
            );
            e.push(expression, true, true);
            return Some(false);
        }
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) {
            self.pedantic_suppression += 1;
            self.consume_token();
            self.parse_prefix(
                e,
                UnaryOperator::Extension,
                token.source_vectors,
                ExpressionMode::CastExpression,
            )?;
            return Some(false);
        }
        if is_operator(Some(token), OperatorTokenType::AmpersandAmpersand) {
            self.extension(Feature::LabelsAsValues, "label address", token);
            self.consume_token();
            self.parse_label_address(e, token.source_vectors);
            return Some(false);
        }
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Countof)) {
            self.consume_token();
            self.parse_countof_or_alignof(e, KeywordTokenType::Countof, token.source_vectors)?;
            return Some(false);
        }
        if let TokenType::Keyword(
            keyword @ (KeywordTokenType::Generic | KeywordTokenType::Alignof),
        ) = token.kind
        {
            if keyword == KeywordTokenType::Alignof {
                // GCC takes the operand of every spelling as `__alignof__`'s:
                // a parenthesized type name or a unary expression. The
                // parsed operand, a compound literal included, decides
                // whether an ISO spelling took an expression.
                self.consume_token();
                self.parse_countof_or_alignof(e, KeywordTokenType::Alignof, token.source_vectors)?;
                if e.operands.last().is_some_and(|operand| {
                    matches!(operand.expression.kind, ExpressionType::AlignofExpr(_))
                }) {
                    report_alignof_expression(self, token);
                }
                return Some(false);
            }
            let kind = if keyword == KeywordTokenType::Generic {
                ModernKind::Generic
            } else {
                ModernKind::Operand {
                    type_only: false,
                    constant:  false,
                }
            };
            self.parse_modern_operand(e, keyword, token.source_vectors, kind, None)?;
            return Some(false);
        }
        // A brace group directly inside a call's or statement header's `(`
        // and closed before its `)` is one error operand even when it holds
        // statements: skip it whole so the parenthesis keeps its `)`. A `({`
        // that opens an expression is a GNU statement expression and never
        // reaches here.
        if is_operator(Some(token), OperatorTokenType::OpeningCurlyBrace)
            && is_operator(self.cursor.previous, OperatorTokenType::OpeningParenthesis)
            && brace_group_closes_before_parenthesis(self)
        {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "expression operand",
                    Some(token.kind),
                ),
                Some(token),
            );
            self.consume_token();
            return Some(self.skip_brace_group(e, token.source_vectors));
        }
        // A visible typedef spelling is still a syntactically valid primary
        // expression. At the first initializer operand, do not reinterpret
        // declaration-shaped lookahead as recovery evidence.
        let following_kind = self.cursor.following().map(|following| following.kind);
        let identifier_is_unambiguous_recovery_boundary =
            matches!(token.kind, TokenType::Identifier)
                && (matches!(
                    following_kind,
                    Some(TokenType::Operator(OperatorTokenType::Colon))
                ) || e.recovery == ExpressionBoundary::Initializer
                    && matches!(following_kind, Some(TokenType::Identifier)));
        let identifier_is_operand = matches!(token.kind, TokenType::Identifier)
            && !identifier_is_unambiguous_recovery_boundary;
        let owned_boundary = is_owning_boundary_for(e.boundary, token.kind)
            || e.recovery != e.boundary && is_owning_boundary_for(e.recovery, token.kind);
        if owned_boundary || !identifier_is_operand && self.at_strong_boundary(e, token) {
            // A declaration-specifier keyword inside this expression's own
            // delimiters, such as `f(int)` or `(static int)`, is a stray token
            // when the closer follows: skip to the closer instead of
            // abandoning the delimiters to a new declaration.
            let stray_run = if !owned_boundary
                && matches!(token.kind, TokenType::Keyword(_))
                && let Some(closer) = e.closer()
            {
                closer_follows_stray_run(self, closer, false)
            } else {
                None
            };
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "expression operand",
                    Some(token.kind),
                ),
                Some(token),
            );
            self.push_error_operand(e, Some(token));
            if let Some(stray_run) = stray_run {
                self.skip_stray_tokens(stray_run);
            }
            return Some(true);
        }
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Sizeof)) {
            self.consume_token();
            self.parse_sizeof(e, token.source_vectors)?;
            return Some(false);
        }
        if let Some((operator, child_mode)) = prefix_operator(token.kind) {
            if operator == UnaryOperator::Extension {
                self.pedantic_suppression += 1;
            }
            self.consume_token();
            self.parse_prefix(e, operator, token.source_vectors, child_mode)?;
            return Some(false);
        }
        if is_operator(Some(token), OperatorTokenType::OpeningParenthesis) {
            if is_operator(
                self.cursor.following(),
                OperatorTokenType::OpeningCurlyBrace,
            ) {
                self.extension(Feature::StatementExpressions, "statement expression", token);
                self.consume_token();
                self.parse_statement_expression(e, token.source_vectors)?;
                return Some(false);
            }
            if parenthesized_type_name_follows(self) {
                let usage = if e.mode == ExpressionMode::UnaryExpression {
                    TypeNameUse::UnaryCompoundLiteral
                } else {
                    TypeNameUse::Cast
                };
                self.consume_token();
                self.parse_typed_operand(e, token.source_vectors, usage)?;
                return Some(false);
            }
            self.consume_token();
            self.parse_grouped(e, token.source_vectors)?;
            return Some(false);
        }
        let kind = match token.kind {
            | TokenType::Keyword(KeywordTokenType::True) => ExpressionType::Boolean(true),
            | TokenType::Keyword(KeywordTokenType::False) => ExpressionType::Boolean(false),
            | TokenType::Keyword(KeywordTokenType::Nullptr) => ExpressionType::Nullptr,
            | TokenType::Identifier => ExpressionType::Identifier(Identifier::from_token(token)),
            | TokenType::Integer(value) => ExpressionType::Constant(Constant::Integer(value)),
            | TokenType::Float(value) => {
                let binary128 = matches!(
                    value,
                    FloatTokenType::Float128(_) | FloatTokenType::ImaginaryFloat128(_)
                );
                // `f128` names `_Float128`, the type `q` names.
                let interchange_binary128 = binary128
                    && self
                        .context
                        .string_cache
                        .at(token.contents)
                        .trim_end_matches('\0')
                        .ends_with("128");
                if binary128 && !interchange_binary128 {
                    self.extension(Feature::Float128, "binary128 floating constant", token);
                }
                if interchange_binary128
                    || matches!(
                        value,
                        FloatTokenType::Float32(_)
                            | FloatTokenType::Float64(_)
                            | FloatTokenType::Float32x(_)
                            | FloatTokenType::Float64x(_)
                    )
                {
                    self.extension(
                        Feature::InterchangeFloatTypes,
                        "_FloatN floating constant suffix",
                        token,
                    );
                }
                // GCC reports the suffix before C23 under `-pedantic`; Clang
                // does not.
                if matches!(value, FloatTokenType::Float16(_))
                    && self.context.configuration.compiler_identity()
                        == crate::configuration::CompilerIdentity::Gcc
                {
                    self.extension(Feature::Float16, "_Float16 floating constant suffix", token);
                }
                if matches!(value, FloatTokenType::BFloat16(_)) {
                    self.extension(Feature::BFloat16, "__bf16 floating constant suffix", token);
                }
                ExpressionType::Constant(Constant::Float(value))
            },
            | TokenType::Character(value) => ExpressionType::Constant(Constant::Char(value)),
            | TokenType::String(value) => ExpressionType::StringLiteral(value),
            | _ => {
                self.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "expression operand",
                        Some(token.kind),
                    ),
                    Some(token),
                );
                if is_operator(Some(token), OperatorTokenType::OpeningCurlyBrace) {
                    self.consume_token();
                    return Some(self.skip_brace_group(e, token.source_vectors));
                }
                self.push_error_operand(e, Some(token));
                if binary_operator(token.kind).is_some()
                    || is_operator(Some(token), OperatorTokenType::QuestionMark)
                {
                    // Only the left operand is missing: the operator and its
                    // right operand still parse normally.
                    return Some(false);
                }
                e.stray_error_operand = true;
                self.consume_token();
                return Some(false);
            },
        };
        let expression = self.store_expression(
            kind,
            token.source_vectors,
            None,
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, true);
        self.consume_token();
        Some(false)
    }

    /// One step in operator position. Returns whether the expression ends.
    ///
    /// C99: postfix suffixes §6.5.2, p. 69; PDF p. 81; binary operators
    /// §6.5.5-§6.5.17, pp. 82-94; PDF pp. 94-106.
    fn parse_operator(&mut self, e: &mut ExpressionParts<'tu, 'p>) -> Option<bool> {
        let token = self.cursor.current();
        if let Some(sizeof_source) = e.pending_sizeof
            && !token.is_some_and(|token| is_postfix_starter(token.kind))
        {
            e.pending_sizeof = None;
            let operand = e.pop().expression;
            let source = self
                .context
                .merge_vectors(sizeof_source, operand.source_vectors);
            let expression = self.store_expression(
                ExpressionType::SizeofExpr(operand),
                source,
                Some(sizeof_source),
                self.hard_error_count > e.errors,
            );
            e.push(expression, true, false);
            return Some(false);
        }
        if let Some(token) = token
            && is_closing_delimiter(token.kind)
            && e.consumes_unowned_closer(token.kind)
        {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "operator in expression",
                    Some(token.kind),
                ),
                Some(token),
            );
            self.consume_token();
            return Some(true);
        }
        if e.is_boundary(token.map(|token| token.kind)) {
            return Some(true);
        }
        let Some(token) = token else {
            return Some(true);
        };
        // A brace list right after a `)` this expression consumed reads as a
        // compound literal whose type name did not parse, as in `(a b){1, 2}`.
        // Skip the list with the operand as one error operand instead of
        // leaking its `}` to the enclosing statement. A statement block, as in
        // `while (f(x) { ... }`, still ends the expression so its owner names
        // the missing `)`.
        if is_operator(Some(token), OperatorTokenType::OpeningCurlyBrace)
            && is_operator(self.cursor.previous, OperatorTokenType::ClosingParenthesis)
            && !brace_group_is_block(self)
        {
            let operand = e.pop().expression;
            if !operand.recovered {
                self.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "operator before brace list",
                        Some(token.kind),
                    ),
                    Some(token),
                );
            }
            let source = self
                .context
                .merge_vectors(operand.source_vectors, token.source_vectors);
            self.consume_token();
            return Some(self.skip_brace_group(e, source));
        }
        // C99 §6.5.2p1: a postfix suffix applies only to a
        // `postfix-expression`. Keep the suffix in this expression, applied
        // to the operand and marked recovered, rather than leaking each of
        // its tokens into the enclosing production.
        if is_postfix_starter(token.kind)
            && !e
                .operands
                .last()
                .is_some_and(|operand| operand.postfix_expression)
        {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "postfix operator after a non-postfix expression",
                    Some(token.kind),
                ),
                Some(token),
            );
        }
        if let TokenType::Operator(operator) = token.kind {
            match operator {
                | OperatorTokenType::OpeningSquareBracket => {
                    let base = e.pop().expression;
                    self.consume_token();
                    self.parse_subscript(e, base, token.source_vectors)?;
                    return Some(false);
                },
                | OperatorTokenType::OpeningParenthesis => {
                    let base = e.pop().expression;
                    self.consume_token();
                    self.parse_call(e, base, token.source_vectors)?;
                    return Some(false);
                },
                | OperatorTokenType::Period | OperatorTokenType::Arrow => {
                    let base = e.pop().expression;
                    self.consume_token();
                    self.parse_member(
                        e,
                        base,
                        operator == OperatorTokenType::Arrow,
                        token.source_vectors,
                    );
                    return Some(false);
                },
                | OperatorTokenType::PlusPlus | OperatorTokenType::MinusMinus => {
                    let base = e.pop().expression;
                    let source = self
                        .context
                        .merge_vectors(base.source_vectors, token.source_vectors);
                    let expression = self.store_expression(
                        ExpressionType::Unary {
                            operator:           if operator == OperatorTokenType::PlusPlus {
                                UnaryOperator::PostIncrement
                            } else {
                                UnaryOperator::PostDecrement
                            },
                            operand_expression: base,
                        },
                        source,
                        Some(token.source_vectors),
                        self.hard_error_count > e.errors,
                    );
                    e.push(expression, true, true);
                    self.consume_token();
                    return Some(false);
                },
                | _ => {},
            }
        }
        // C99 §6.5.15p1: `?` follows a `logical-OR-expression`, so every
        // tighter operator (levels below 13) reduces first.
        if is_operator(Some(token), OperatorTokenType::QuestionMark)
            && !matches!(
                e.mode,
                ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
            )
        {
            while e
                .operators
                .last()
                .is_some_and(|operator| operator.precedence() < 13)
            {
                self.reduce_operator(e);
            }
            self.consume_token();
            self.parse_conditional(e, token.source_vectors)?;
            return Some(false);
        }
        // C99: only `expression` absorbs a comma (§6.5.17p1), a
        // `constant-expression` takes no assignment (§6.6p1), and a cast or
        // unary operand takes no binary operator (§6.5.3p1, §6.5.4p1).
        if let Some(operator) = binary_operator(token.kind)
            && !(operator == BinaryOperator::Comma && e.mode != ExpressionMode::Expression)
            && !(is_assignment_operator(operator) && e.mode == ExpressionMode::ConstantExpression)
            && !matches!(
                e.mode,
                ExpressionMode::CastExpression | ExpressionMode::UnaryExpression
            )
        {
            let incoming = LanguageExpressionOperator::Binary {
                operator,
                source_vectors: token.source_vectors,
            };
            while e
                .operators
                .last()
                .is_some_and(|stacked| stacked.has_precedence_over(incoming))
            {
                self.reduce_operator(e);
            }
            // C99 §6.5.16p1: the left operand of an assignment operator is a
            // `unary-expression`.
            if is_assignment_operator(operator)
                && !e
                    .operands
                    .last()
                    .is_some_and(|operand| operand.unary_expression)
            {
                self.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "unary-expression left operand of assignment",
                        Some(token.kind),
                    ),
                    Some(token),
                );
            }
            e.operators.push(incoming);
            e.operand = true;
            e.stray_error_operand = false;
            self.consume_token();
            return Some(false);
        }
        if let Some(closer) = e.closer() {
            // Inside delimiters this expression owns, skip a stray run to the
            // closer that follows it. Without such a closer, the closer
            // itself is missing: finish quietly so its owner names the
            // missing `)`, `]`, or `:` at this token.
            if let Some(stray_run) = closer_follows_stray_run(self, closer, false) {
                if !e.stray_error_operand {
                    self.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "operator in delimited expression",
                            Some(token.kind),
                        ),
                        Some(token),
                    );
                }
                self.skip_stray_tokens(stray_run);
            }
            return Some(true);
        }
        // The statement that owns the terminator reports the missing `;`
        // before a statement keyword, and a `;` never continues an
        // expression: every owner of an expression that does not end at `;`
        // (an enumerator value) names what it expected there instead.
        if is_statement_keyword(token.kind)
            && matches!(
                e.boundary,
                ExpressionBoundary::Statement(
                    ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon
                )
            )
            || is_operator(Some(token), OperatorTokenType::Semicolon)
        {
            return Some(true);
        }
        self.report(
            ParserErrorType::ExpectedStatementExpression(
                "operator in expression",
                Some(token.kind),
            ),
            Some(token),
        );
        Some(true)
    }

    /// Discards `remaining` already-diagnosed tokens before the closer that
    /// ends a malformed delimited expression.
    fn skip_stray_tokens(&mut self, mut remaining: usize) {
        while remaining != 0 && self.cursor.current().is_some() {
            self.consume_token();
            remaining -= 1;
        }
    }

    /// Skips a brace group whose `{` was just consumed as one error operand.
    /// Returns whether the expression ends at the end of input.
    fn skip_brace_group(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        source: SourceVectors,
    ) -> bool {
        let mut depth = 1_u32;
        let mut source = source;
        loop {
            let Some(token) = self.cursor.current() else {
                self.push_error_operand_with_source(e, source, None);
                return true;
            };
            source = self.context.merge_vectors(source, token.source_vectors);
            match token.kind {
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => depth += 1,
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => depth -= 1,
                | _ => {},
            }
            self.consume_token();
            if depth == 0 {
                self.push_error_operand_with_source(e, source, None);
                return false;
            }
        }
    }

    /// GNU extension: `&& identifier`, the address of a label.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    fn parse_label_address(&mut self, e: &mut ExpressionParts<'tu, 'p>, source: SourceVectors) {
        let token = self.cursor.current();
        if let Some(token) = token.filter(|token| matches!(token.kind, TokenType::Identifier)) {
            let merged = self.context.merge_vectors(source, token.source_vectors);
            let expression = self.store_expression(
                ExpressionType::LabelAddress(Identifier::from_token(token)),
                merged,
                Some(source),
                false,
            );
            e.push(expression, true, false);
            self.consume_token();
            return;
        }
        self.report(
            ParserErrorType::ExpectedGnuSyntax(
                "label identifier after `&&`",
                token.map(|x| x.kind),
            ),
            token,
        );
        self.push_error_operand(e, token);
    }

    /// C2y `_Countof` and GNU `__alignof__`: a parenthesized type name or a
    /// `unary-expression`, including a compound literal, like `sizeof` in
    /// C99 §6.5.3 paragraph 1, p. 78; PDF p. 90.
    fn parse_countof_or_alignof(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        keyword: KeywordTokenType,
        source: SourceVectors,
    ) -> Option<()> {
        if is_operator(self.cursor.current(), OperatorTokenType::OpeningParenthesis)
            && parenthesized_type_name_follows(self)
            && !parenthesized_compound_literal_follows(self)
        {
            let kind = ModernKind::Operand {
                type_only: true,
                constant:  false,
            };
            return self.parse_modern_operand(e, keyword, source, kind, Some(source));
        }
        let operand = self
            .parse_expression(ExpressionMode::UnaryExpression, e.boundary, e.recovery)?
            .expression;
        let merged = self.context.merge_vectors(source, operand.source_vectors);
        let kind = if keyword == KeywordTokenType::Countof {
            ExpressionType::Countof(SyntaxOperand::Expression(operand))
        } else {
            ExpressionType::AlignofExpr(operand)
        };
        let expression =
            self.store_expression(kind, merged, Some(source), self.hard_error_count > e.errors);
        e.push(expression, true, false);
        Some(())
    }

    /// C11 `_Generic` (§6.5.1.1, p. 78; PDF p. 96) and the parenthesized
    /// operands of `_Alignof` and `_Countof`, parsed by the ISO construct
    /// grammar. `opened` is the keyword's provenance when the caller already
    /// consumed it and the construct starts at `(`.
    fn parse_modern_operand(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        keyword: KeywordTokenType,
        keyword_source: SourceVectors,
        construct: ModernKind,
        opened: Option<SourceVectors>,
    ) -> Option<()> {
        let (kind, source) = match self.parse_modern(construct, opened, self.hard_error_count)? {
            | ModernValue::Generic(x) => (ExpressionType::Generic(x), x.source_vectors),
            | ModernValue::Operand(operand, source) => (
                match (keyword, operand) {
                    | (KeywordTokenType::Alignof, SyntaxOperand::Type(x)) =>
                        ExpressionType::AlignofType(x),
                    | (KeywordTokenType::Alignof, SyntaxOperand::Expression(x)) =>
                        ExpressionType::AlignofExpr(x),
                    | (_, operand) => ExpressionType::Countof(operand),
                },
                source,
            ),
            | returned => unreachable!("ISO expression protocol: {returned:?}"),
        };
        let expression = self.store_expression(
            kind,
            source,
            Some(keyword_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, keyword == KeywordTokenType::Generic);
        Some(())
    }

    /// GNU extension: `( compound-statement )` as an operand.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    fn parse_statement_expression(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        source: SourceVectors,
    ) -> Option<()> {
        let statement = self.parse_compound(false)?;
        let mut merged = self.context.merge_vectors(source, statement.source_vectors);
        let token = self.cursor.current();
        let consume = is_operator(token, OperatorTokenType::ClosingParenthesis);
        if consume {
            merged = self
                .context
                .merge_vectors(merged, token.expect("closer exists").source_vectors);
        } else {
            self.report(
                ParserErrorType::ExpectedGnuSyntax(
                    "`)` after statement expression",
                    token.map(|x| x.kind),
                ),
                token,
            );
        }
        let expression = self.store_expression(
            ExpressionType::StatementExpression(statement),
            merged,
            Some(source),
            statement.recovered || !consume,
        );
        e.push(expression, true, true);
        if consume {
            self.consume_token();
        }
        Some(())
    }

    /// C99 §6.5.1p1: `( expression )`.
    fn parse_grouped(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        opening: SourceVectors,
    ) -> Option<()> {
        let child = self
            .parse_expression(
                ExpressionMode::Expression,
                ExpressionBoundary::ClosingParenthesis,
                e.recovery,
            )?
            .expression;
        let mut operator_source = opening;
        let mut source = self.context.merge_vectors(opening, child.source_vectors);
        let token = self.cursor.current();
        let consume = if let Some(close) =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::ClosingParenthesis))
        {
            source = self.context.merge_vectors(source, close.source_vectors);
            operator_source = self
                .context
                .merge_vectors(operator_source, close.source_vectors);
            true
        } else {
            self.report(
                ParserErrorType::ExpectedClosingParenthesisInStatement(
                    "grouped expression",
                    token.map(|token| token.kind),
                ),
                token,
            );
            false
        };
        let expression = self.store_expression(
            ExpressionType::Parenthesized { expression: child },
            source,
            Some(operator_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, true);
        if consume {
            self.consume_token();
        }
        Some(())
    }

    /// C99 §6.5.2p1: `postfix-expression [ expression ]`.
    fn parse_subscript(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        base: &'tu Expression<'tu>,
        opening: SourceVectors,
    ) -> Option<()> {
        let child = self
            .parse_expression(
                ExpressionMode::Expression,
                ExpressionBoundary::ClosingSquareBracket,
                e.recovery,
            )?
            .expression;
        let mut operator_source = opening;
        let mut source = self.context.merge_vectors(base.source_vectors, opening);
        source = self.context.merge_vectors(source, child.source_vectors);
        let token = self.cursor.current();
        let consume = if let Some(close) =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::ClosingSquareBracket))
        {
            source = self.context.merge_vectors(source, close.source_vectors);
            operator_source = self
                .context
                .merge_vectors(operator_source, close.source_vectors);
            true
        } else {
            self.report(
                ParserErrorType::ExpectedClosingSquareBracketInSubscript(
                    token.map(|token| token.kind),
                ),
                token,
            );
            false
        };
        let expression = self.store_expression(
            ExpressionType::Binary {
                operator:         BinaryOperator::Subscript,
                left_expression:  base,
                right_expression: child,
            },
            source,
            Some(operator_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, true);
        if consume {
            self.consume_token();
        }
        Some(())
    }

    /// C99 §6.5.2p1: `postfix-expression ( argument-expression-list(opt) )`.
    /// Each argument is an `assignment-expression`, so a top-level comma
    /// separates arguments.
    fn parse_call(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        base: &'tu Expression<'tu>,
        opening: SourceVectors,
    ) -> Option<()> {
        let mut call = self.pools.take_call();
        call.source_vectors.extend([base.source_vectors, opening]);
        call.operator_sources.push(opening);
        let mut consume = is_operator(self.cursor.current(), OperatorTokenType::ClosingParenthesis);
        if !consume {
            loop {
                let Some(argument) = self.parse_expression(
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::Argument,
                    e.recovery,
                ) else {
                    call.arguments.clear();
                    call.source_vectors.clear();
                    call.operator_sources.clear();
                    self.pools.calls.reclaim(call);
                    return None;
                };
                call.arguments.push(argument.expression);
                call.source_vectors.push(argument.expression.source_vectors);
                let token = self.cursor.current();
                if let Some(separator) =
                    token.filter(|token| is_operator(Some(*token), OperatorTokenType::Comma))
                {
                    call.source_vectors.push(separator.source_vectors);
                    call.operator_sources.push(separator.source_vectors);
                    self.consume_token();
                    continue;
                }
                // A child may return without consuming an enclosing grammar
                // boundary (for example a label after a malformed
                // initializer). Avoid retrying the boundary as another call
                // argument.
                if let Some(token) = token
                    && is_expression_operand_starter(token.kind)
                    && !self.at_strong_boundary(e, token)
                {
                    self.report(
                        ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionCall(Some(
                            token.kind,
                        )),
                        Some(token),
                    );
                    continue;
                }
                consume = is_operator(token, OperatorTokenType::ClosingParenthesis);
                if !consume {
                    self.report(
                        ParserErrorType::ExpectedClosingParenthesisInStatement(
                            "function call",
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                }
                break;
            }
        }
        if consume {
            let close = self.cursor.current().expect("closing parenthesis exists");
            call.source_vectors.push(close.source_vectors);
            call.operator_sources.push(close.source_vectors);
        }
        let arguments = self.alloc_syntax_list(&mut call.arguments);
        let source_vectors = self.context.merge_vector_list(&call.source_vectors);
        let operator_sources = self.context.merge_vector_list(&call.operator_sources);
        let expression = self.store_expression(
            ExpressionType::Call {
                function_expression: base,
                arguments,
            },
            source_vectors,
            Some(operator_sources),
            self.hard_error_count > e.errors,
        );
        call.source_vectors.clear();
        call.operator_sources.clear();
        self.pools.calls.reclaim(call);
        e.push(expression, true, true);
        if consume {
            self.consume_token();
        }
        Some(())
    }

    /// C99 §6.5.2p1: `.` and `->` take an `identifier`.
    fn parse_member(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        base: &'tu Expression<'tu>,
        indirect: bool,
        operator_source: SourceVectors,
    ) {
        let token = self.cursor.current();
        let source = self
            .context
            .merge_vectors(base.source_vectors, operator_source);
        let Some(member) = token.filter(|token| matches!(token.kind, TokenType::Identifier)) else {
            self.report(
                ParserErrorType::ExpectedMemberIdentifier(token.map(|token| token.kind)),
                token,
            );
            self.push_error_operand_with_source(e, source, Some(operator_source));
            return;
        };
        let source = self.context.merge_vectors(source, member.source_vectors);
        let kind = if indirect {
            ExpressionType::IndirectMember {
                base_expression: base,
                member:          Identifier::from_token(member),
            }
        } else {
            ExpressionType::DirectMember {
                base_expression: base,
                member:          Identifier::from_token(member),
            }
        };
        let expression = self.store_expression(
            kind,
            source,
            Some(operator_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, true);
        self.consume_token();
    }

    /// C99 §6.5.3p1: a prefix operator and its operand, whose `operand_mode`
    /// the operator selects. GNU `__extension__` suppresses extension
    /// diagnostics from its keyword through its operand.
    fn parse_prefix(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        operator: UnaryOperator,
        operator_source: SourceVectors,
        operand_mode: ExpressionMode,
    ) -> Option<()> {
        let operand = self
            .parse_expression(operand_mode, e.boundary, e.recovery)?
            .expression;
        if operator == UnaryOperator::Extension {
            self.pedantic_suppression -= 1;
        }
        let source = self
            .context
            .merge_vectors(operator_source, operand.source_vectors);
        let expression = self.store_expression(
            ExpressionType::Unary {
                operator,
                operand_expression: operand,
            },
            source,
            Some(operator_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, false);
        Some(())
    }

    /// C99 §6.5.3p1: `sizeof ( type-name )` when a type name follows `(`,
    /// otherwise `sizeof unary-expression`.
    fn parse_sizeof(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        sizeof_source: SourceVectors,
    ) -> Option<()> {
        if let Some(opening) = self
            .cursor
            .current()
            .filter(|token| is_operator(Some(*token), OperatorTokenType::OpeningParenthesis))
            && parenthesized_type_name_follows(self)
        {
            self.consume_token();
            return self.parse_typed_operand(
                e,
                opening.source_vectors,
                TypeNameUse::Sizeof(sizeof_source),
            );
        }
        let operand = self
            .parse_expression(ExpressionMode::UnaryExpression, e.boundary, e.recovery)?
            .expression;
        let source = self
            .context
            .merge_vectors(sizeof_source, operand.source_vectors);
        let expression = self.store_expression(
            ExpressionType::SizeofExpr(operand),
            source,
            Some(sizeof_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, true, false);
        Some(())
    }

    /// The operand a parenthesized `type-name` introduces after its `(` was
    /// consumed: a cast (§6.5.4, p. 81; PDF p. 93), `sizeof ( type-name )`
    /// (§6.5.3, p. 78; PDF p. 90), or a compound literal (§6.5.2.5, p. 75;
    /// PDF p. 87).
    fn parse_typed_operand(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        opening: SourceVectors,
        usage: TypeNameUse,
    ) -> Option<()> {
        let compound_literal = type_name_is_compound_literal(self);
        let type_name = self.parse_type_name(self.hard_error_count, compound_literal)?;
        let mut source = self
            .context
            .merge_vectors(opening, type_name.source_vectors);
        let token = self.cursor.current();
        let consume = if let Some(close) =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::ClosingParenthesis))
        {
            source = self.context.merge_vectors(source, close.source_vectors);
            true
        } else {
            self.report(
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
            && is_operator(
                self.cursor.following(),
                OperatorTokenType::OpeningCurlyBrace,
            )
            && !self.cursor.lookahead(1).is_some_and(|first| {
                !matches!(first.kind, TokenType::Identifier)
                    && (self.declaration_starter(first) || is_statement_keyword(first.kind))
            });
        let mut cast = false;
        if !starts_compound_literal {
            match usage {
                | TypeNameUse::Sizeof(sizeof_source) => {
                    let source = self.context.merge_vectors(sizeof_source, source);
                    let expression = self.store_expression(
                        ExpressionType::SizeofType(type_name),
                        source,
                        Some(sizeof_source),
                        self.hard_error_count > e.errors,
                    );
                    e.push(expression, true, false);
                },
                | TypeNameUse::Cast => cast = true,
                | TypeNameUse::UnaryCompoundLiteral if consume => {
                    // A unary operator's operand cannot be a cast, so the `{`
                    // of a compound literal is missing after the `)`. Report
                    // it there and keep the operand as a recovered cast.
                    let following = self.cursor.following();
                    self.report(
                        ParserErrorType::ExpectedStatementExpression(
                            "compound literal initializer",
                            following.map(|following| following.kind),
                        ),
                        following,
                    );
                    if following
                        .is_some_and(|following| is_expression_operand_starter(following.kind))
                    {
                        cast = true;
                    } else {
                        self.push_error_operand(e, following);
                    }
                },
                | TypeNameUse::UnaryCompoundLiteral => self.push_error_operand(e, token),
            }
        }
        if consume {
            self.consume_token();
        }
        if starts_compound_literal {
            return self.parse_compound_literal(e, type_name, source, usage);
        }
        if cast {
            return self.parse_cast(e, type_name, source);
        }
        Some(())
    }

    /// C99 §6.5.2.5p1: `( type-name ) { initializer-list }`.
    fn parse_compound_literal(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        type_name: &'tu TypeName<'tu>,
        type_source: SourceVectors,
        usage: TypeNameUse,
    ) -> Option<()> {
        if let Some(token) = self.cursor.current() {
            self.extension(Feature::CompoundLiterals, "compound literal", token);
        }
        let InitializerResult { initializer, .. } = self.parse_initializer(
            self.hard_error_count,
            e.closing_parenthesis_is_boundary(),
            e.closing_square_bracket_is_boundary(),
        )?;
        let source = self
            .context
            .merge_vectors(type_source, initializer.source_vectors);
        let compound = self.store_expression(
            ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            },
            source,
            Some(type_source),
            self.hard_error_count > e.errors,
        );
        if let TypeNameUse::Sizeof(sizeof_source) = usage {
            e.pending_sizeof = Some(sizeof_source);
        }
        e.push(compound, true, true);
        Some(())
    }

    /// C99 §6.5.4p1: `( type-name ) cast-expression`.
    fn parse_cast(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        type_name: &'tu TypeName<'tu>,
        type_source: SourceVectors,
    ) -> Option<()> {
        if matches!(type_name.declaration_specifiers.type_specifiers, TypeSpecifiers::StructOrUnion(x) if x.struct_or_union == StructOrUnion::Union)
        {
            self.extension_source(
                Feature::UnionCasts,
                "cast to union",
                type_name.source_vectors,
            );
        }
        let operand = self
            .parse_expression(ExpressionMode::CastExpression, e.boundary, e.recovery)?
            .expression;
        let source = self
            .context
            .merge_vectors(type_source, operand.source_vectors);
        let expression = self.store_expression(
            ExpressionType::Cast {
                target_type:        type_name,
                operand_expression: operand,
            },
            source,
            Some(type_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, false, false);
        Some(())
    }

    /// C99 §6.5.15p1: `logical-OR-expression ? expression :
    /// conditional-expression`, after the `?` was consumed. The middle
    /// operand is a full `expression`; the last is parsed in
    /// constant-expression mode (§6.6p1), so assignment and comma end it.
    fn parse_conditional(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        question_source: SourceVectors,
    ) -> Option<()> {
        let token = self.cursor.current();
        let (middle, colon_source) = if let Some(colon) =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::Colon))
        {
            self.extension(
                Feature::OmittedConditionalOperand,
                "omitted conditional operand",
                colon,
            );
            self.consume_token();
            (None, Some(colon.source_vectors))
        } else {
            let middle = self
                .parse_expression(
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                    e.recovery,
                )?
                .expression;
            let token = self.cursor.current();
            if let Some(colon) =
                token.filter(|token| is_operator(Some(*token), OperatorTokenType::Colon))
            {
                self.consume_token();
                (Some(middle), Some(colon.source_vectors))
            } else {
                self.report(
                    ParserErrorType::ExpectedColonInLabel(
                        "conditional expression",
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                (Some(middle), None)
            }
        };
        let final_expression = self
            .parse_expression(ExpressionMode::ConstantExpression, e.boundary, e.recovery)?
            .expression;
        let condition = e.pop().expression;
        let omitted = middle.is_none();
        let middle = middle.unwrap_or(condition);
        let mut source = self
            .context
            .merge_vectors(condition.source_vectors, question_source);
        if !omitted {
            source = self.context.merge_vectors(source, middle.source_vectors);
        }
        if let Some(colon) = colon_source {
            source = self.context.merge_vectors(source, colon);
        }
        source = self
            .context
            .merge_vectors(source, final_expression.source_vectors);
        let operands = self.alloc_syntax_part(ConditionalExpression {
            condition_expression: condition,
            then_expression:      middle,
            else_expression:      final_expression,
        });
        let operator_source = colon_source.map_or(question_source, |colon| {
            self.context.merge_vectors(question_source, colon)
        });
        let expression = self.store_expression(
            if omitted {
                ExpressionType::OmittedConditional(operands)
            } else {
                ExpressionType::Conditional(operands)
            },
            source,
            Some(operator_source),
            self.hard_error_count > e.errors,
        );
        e.push(expression, false, false);
        Some(())
    }

    fn reduce_operator(&mut self, e: &mut ExpressionParts<'tu, 'p>) {
        let LanguageExpressionOperator::Binary {
            operator,
            source_vectors,
        } = e
            .operators
            .pop()
            .expect("a pending expression operator exists");
        let right = self.pop_operand_or_error(e).expression;
        let left = self.pop_operand_or_error(e).expression;
        let source = self
            .context
            .merge_vectors(left.source_vectors, source_vectors);
        let source = self.context.merge_vectors(source, right.source_vectors);
        let expression = self.store_expression(
            ExpressionType::Binary {
                operator,
                left_expression: left,
                right_expression: right,
            },
            source,
            Some(source_vectors),
            self.hard_error_count > e.errors,
        );
        e.push(expression, false, false);
    }

    fn pop_operand_or_error(&mut self, e: &mut ExpressionParts<'tu, 'p>) -> ExpressionOperand<'tu> {
        if let Some(operand) = e.operands.pop() {
            return operand;
        }
        self.push_error_operand(e, None);
        e.pop()
    }

    /// Reduces every pending operator and returns the expression. Whether a
    /// `constant-expression` (§6.6 paragraph 1, p. 95; PDF p. 107) is in fact
    /// constant (§6.6 paragraphs 3-10, pp. 95-96; PDF pp. 107-108) is left to
    /// semantic analysis.
    fn finish_expression(&mut self, e: &mut ExpressionParts<'tu, 'p>) -> ExpressionResult<'tu> {
        while !e.operators.is_empty() {
            self.reduce_operator(e);
        }
        let operand = self.pop_operand_or_error(e);
        let recovered = self.hard_error_count > e.errors;
        let expression = if recovered {
            self.mark_expression_recovered(operand.expression)
        } else {
            operand.expression
        };
        ExpressionResult {
            expression,
            recovered,
        }
    }

    #[cold]
    #[inline(never)]
    fn push_error_operand(&mut self, e: &mut ExpressionParts<'tu, 'p>, anchor: Option<Token>) {
        let source_vectors = match anchor {
            | Some(token) if token.source_vectors.length() != 0 => {
                let mut combined = SourceVectors::default();
                for index in 0..token.source_vectors.length() as usize {
                    // Creating an anchor appends to the retained provenance,
                    // so the token's vectors are read again each time.
                    let source = &self.context.get_source_vectors(token.source_vectors)[index];
                    let (position, source_file_index) =
                        (source.position(self.context), source.source_file_index);
                    let anchor =
                        self.context
                            .create_retained_source_vectors(position, source_file_index, 0);
                    combined = self.context.merge_vectors(combined, anchor);
                }
                combined
            },
            | _ => self.missing_syntax_source(),
        };
        self.push_error_operand_with_source(e, source_vectors, None);
    }

    #[cold]
    #[inline(never)]
    fn push_error_operand_with_source(
        &mut self,
        e: &mut ExpressionParts<'tu, 'p>,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
    ) {
        let expression = self.store_expression(
            ExpressionType::Error,
            source_vectors,
            operator_source_vectors,
            true,
        );
        // An error operand stands for whatever was meant, so operators that
        // need a unary or postfix operand do not diagnose it a second time.
        e.push(expression, true, true);
    }

    fn at_strong_boundary(&mut self, e: &ExpressionParts<'tu, 'p>, token: Token) -> bool {
        let identifier_precedes_conditional_colon = e.boundary
            == ExpressionBoundary::Statement(ExpressionTerminator::Colon)
            && matches!(token.kind, TokenType::Identifier)
            && is_operator(self.cursor.following(), OperatorTokenType::Colon);
        // A brace in a statement-level expression ends it only when the group
        // reads as a statement block, as in `if (value { return; }`. A
        // misplaced brace list such as `a = {1, 2};` is skipped as one error
        // operand instead.
        is_operator(Some(token), OperatorTokenType::OpeningCurlyBrace)
            && matches!(
                e.recovery,
                ExpressionBoundary::Statement(_) | ExpressionBoundary::ClosingParenthesis
            )
            && brace_group_is_block(self)
            && !brace_group_continues_expression(self)
            || is_strong_grammar_boundary_for(self, token, e.boundary)
            || e.recovery != e.boundary
                && !identifier_precedes_conditional_colon
                && is_strong_grammar_boundary_for(self, token, e.recovery)
    }

    pub(super) fn consume_token(&mut self) {
        if let Some(token) = self.cursor.current() {
            self.suppress_token_diagnostic(token);
            self.cursor.consume();
        }
    }
}

/// One expression's Double-E state: the expected position, the pending
/// operands and operators, and its recovery flags.
///
/// C99: §6.5, pp. 67-94; PDF pp. 79-106.
struct ExpressionParts<'tu, 'p> {
    mode:                ExpressionMode,
    boundary:            ExpressionBoundary,
    recovery:            ExpressionBoundary,
    operators:           ArenaVec<'p, LanguageExpressionOperator>,
    operands:            ArenaVec<'p, ExpressionOperand<'tu>>,
    /// Whether an operand, rather than an operator, comes next.
    operand:             bool,
    errors:              usize,
    /// `sizeof` applied to a compound literal waits for its postfix suffixes.
    pending_sizeof:      Option<SourceVectors>,
    /// The top operand is an error operand that replaced a stray token
    /// already diagnosed, so the tokens after it need no second report.
    stray_error_operand: bool,
}

/// What a parenthesized `type-name` in operand position introduces.
///
/// C99: `Cast` is `( type-name ) cast-expression` (§6.5.4 paragraph 1,
/// p. 81; PDF p. 93) or a compound literal; `Sizeof` is
/// `sizeof ( type-name )` (§6.5.3 paragraph 1, p. 78; PDF p. 90) or `sizeof`
/// applied to a compound literal; `UnaryCompoundLiteral` is the
/// `unary-expression` operand of `++`, `--`, or `sizeof`, which cannot be a
/// cast and so must be a compound literal (§6.5.2 paragraph 1, p. 69;
/// PDF p. 81).
#[derive(Clone, Copy)]
enum TypeNameUse {
    Cast,
    Sizeof(SourceVectors),
    UnaryCompoundLiteral,
}

impl<'tu> ExpressionParts<'tu, '_> {
    fn push(
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
        self.operand = false;
        self.stray_error_operand = false;
    }

    fn pop(&mut self) -> ExpressionOperand<'tu> {
        self.operands
            .pop()
            .expect("operator position has an expression operand")
    }

    fn closing_parenthesis_is_boundary(&self) -> bool {
        [self.boundary, self.recovery].into_iter().any(|boundary| {
            matches!(
                boundary,
                ExpressionBoundary::ClosingParenthesis
                    | ExpressionBoundary::Argument
                    | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
            )
        })
    }

    fn closing_square_bracket_is_boundary(&self) -> bool {
        [self.boundary, self.recovery].into_iter().any(|boundary| {
            matches!(
                boundary,
                ExpressionBoundary::ClosingSquareBracket
                    | ExpressionBoundary::ArrayBound
                    | ExpressionBoundary::Designator
            )
        })
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
        is_owning_boundary_for(self.boundary, token)
    }

    fn consumes_unowned_closer(&self, token: TokenType) -> bool {
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

/// Arguments and provenance of the postfix call an expression is building.
///
/// C99: `postfix-expression ( argument-expression-list(opt) )`, §6.5.2,
/// pp. 69-70; PDF pp. 81-82; function calls §6.5.2.2, p. 71; PDF p. 83.
#[derive(Debug)]
pub(in crate::translation_phases::parsing) struct CallState<'tu, 'p> {
    pub(in crate::translation_phases::parsing) arguments:        ArenaVec<'p, &'tu Expression<'tu>>,
    pub(in crate::translation_phases::parsing) source_vectors:   ArenaVec<'p, SourceVectors>,
    pub(in crate::translation_phases::parsing) operator_sources: ArenaVec<'p, SourceVectors>,
}

/// The expression nonterminal a production parses, which bounds the
/// operators it may absorb.
///
/// C99: `Expression` is `expression` (§6.5.17 paragraph 1, p. 94;
/// PDF p. 106); `AssignmentExpression` is `assignment-expression` (§6.5.16
/// paragraph 1, p. 91; PDF p. 103); `ConstantExpression` is
/// `constant-expression`, syntactically a `conditional-expression` (§6.6
/// paragraph 1, p. 95; PDF p. 107); `CastExpression` is `cast-expression`
/// (§6.5.4 paragraph 1, p. 81; PDF p. 93); `UnaryExpression` is
/// `unary-expression` (§6.5.3 paragraph 1, p. 78; PDF p. 90).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::parsing) enum ExpressionMode {
    Expression,
    AssignmentExpression,
    ConstantExpression,
    CastExpression,
    UnaryExpression,
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
pub(in crate::translation_phases::parsing) enum ExpressionBoundary {
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
pub(in crate::translation_phases::parsing) struct ExpressionOperand<'tu> {
    pub(in crate::translation_phases::parsing) expression:         &'tu Expression<'tu>,
    pub(in crate::translation_phases::parsing) unary_expression:   bool,
    pub(in crate::translation_phases::parsing) postfix_expression: bool,
}

impl<'p> CallState<'_, 'p> {
    pub(in crate::translation_phases::parsing) fn new_in(arena: &'p Bump) -> Self {
        Self {
            arguments:        ArenaVec::new_in(arena),
            source_vectors:   ArenaVec::new_in(arena),
            operator_sources: ArenaVec::new_in(arena),
        }
    }
}

/// Diagnoses `_Alignof` or `alignof` applied to an expression, which GCC
/// and Clang accept from `__alignof__`, parenthesized or not (GCC also
/// without parentheses after the standard spellings), a compound literal
/// included. The standard spellings take only a parenthesized type name.
/// The caller reports once the operand is parsed.
/// C11: §6.5.3 paragraph 1, p. 88; PDF p. 106. Extension: GNU, GCC manual,
/// "Determining the Alignment of Functions, Types or Variables",
/// <https://gcc.gnu.org/onlinedocs/gcc/Alignment.html>.
pub(in crate::translation_phases::parsing) fn report_alignof_expression(
    parser: &mut Parser<'_, '_, '_>,
    keyword: Token,
) {
    let gnu_spelling = KeywordTokenType::classify(keyword.contents, parser.context.configuration)
        .is_some_and(|x| x.origin == Some(FeatureOrigin::Gnu));
    if gnu_spelling || parser.pedantic_suppression != 0 {
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
    parser
        .context
        .report_extension(Feature::AlignofExpression, spelling, keyword.source_vectors);
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpressionResult<'tu> {
    pub(super) expression: &'tu Expression<'tu>,
    pub(super) recovered:  bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ConstantExpressionResult<'tu> {
    pub(super) expression: ConstantExpression<'tu>,
    pub(super) recovered:  bool,
}
