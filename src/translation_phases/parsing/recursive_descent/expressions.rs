//! Recursive phase-7 expression grammar with precedence climbing.
//!
//! C99: §6.5.1-§6.5.17, pp. 69-94; PDF pp. 81-106. Prefix and cast
//! operands recurse; postfix chains and left associative operators iterate.
//! Conditional middle operands parse full expressions (§6.5.15 paragraph 1,
//! p. 90; PDF p. 102). Type names and initializer lists retain their frames;
//! operand constraints and constant evaluation belong to semantic analysis.

use super::StudyMode;
use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                StructOrUnion,
                TypeName,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            frames::{
                expression::{
                    ExpressionBoundary,
                    ExpressionFrame,
                    ExpressionMode,
                },
                expression_operators::{
                    LanguageExpressionOperator,
                    binary_operator,
                    is_assignment_operator,
                    is_operator,
                    is_postfix_starter,
                    prefix_operator,
                },
                initializer::InitializerFrame,
                type_name::TypeNameFrame,
            },
            machine::{
                ParseFrame,
                ParseFrameKind,
                ParseValue,
            },
            recovery::ExpressionTerminator,
            syntax::{
                BinaryOperator,
                ConditionalExpression,
                Constant,
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
            TokenType,
        },
    },
};

impl<'tu> Parser<'_, 'tu, '_> {
    pub(super) fn rd_expression(
        &mut self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<&'tu Expression<'tu>> {
        self.rd_precedence(mode_limit(mode), mode, boundary)
            .map(|operand| operand.expression)
    }

    /// C99: precedence and associativity follow §6.5 paragraph 3 and footnote
    /// 74, p. 67; PDF p. 79. Lower table levels bind more tightly.
    #[inline(never)]
    fn rd_precedence(
        &mut self,
        limit: u32,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        if self.study_fallback()? {
            return self.rd_expression_fallback(limit, mode, boundary, true);
        }
        if self.study.mode == StudyMode::Growing {
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
                self.rd_precedence_entered(limit, mode, boundary)
            })
        } else {
            self.rd_precedence_entered(limit, mode, boundary)
        }
    }

    fn rd_precedence_entered(
        &mut self,
        limit: u32,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        let marker = 0_u8;
        let address = if self.study.stack_samples {
            std::ptr::from_ref(&marker).addr()
        } else {
            0
        };
        let active = self.study_enter(ParseFrameKind::Expression, address);
        self.study.stats.expression_entries += 1;
        let result = self.rd_precedence_body(limit, mode, boundary);
        self.study_leave(active);
        result
    }

    fn rd_precedence_body(
        &mut self,
        limit: u32,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        let errors = self.hard_error_count;
        let mut left = self.rd_prefix(mode, boundary)?;
        while let Some(token) = self.cursor.current() {
            if limit >= 13 && is_operator(Some(token), OperatorTokenType::QuestionMark) {
                self.rd_consume();
                let omitted = is_operator(self.cursor.current(), OperatorTokenType::Colon);
                let middle = if omitted {
                    if let Some(colon) = self.cursor.current() {
                        self.extension(
                            Feature::OmittedConditionalOperand,
                            "omitted conditional operand",
                            colon,
                        );
                    }
                    left.expression
                } else {
                    self.rd_expression(
                        ExpressionMode::Expression,
                        ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                    )?
                };
                let colon = self.rd_expect(OperatorTokenType::Colon, "conditional expression");
                let right = self
                    .rd_precedence(13, ExpressionMode::ConstantExpression, boundary)?
                    .expression;
                let mut source = self
                    .context
                    .merge_vectors(left.expression.source_vectors, token.source_vectors);
                if !omitted {
                    source = self.context.merge_vectors(source, middle.source_vectors);
                }
                source = self.context.merge_vectors(source, colon);
                source = self.context.merge_vectors(source, right.source_vectors);
                let operators = self.context.merge_vectors(token.source_vectors, colon);
                let operands = self.alloc_syntax_part(ConditionalExpression {
                    condition_expression: left.expression,
                    then_expression:      middle,
                    else_expression:      right,
                });
                left = Operand::other(self.store_expression(
                    if omitted {
                        ExpressionType::OmittedConditional(operands)
                    } else {
                        ExpressionType::Conditional(operands)
                    },
                    source,
                    Some(operators),
                    self.hard_error_count > errors,
                ));
                continue;
            }
            let Some(operator) = binary_operator(token.kind) else {
                break;
            };
            let precedence = LanguageExpressionOperator::Binary {
                operator,
                source_vectors: token.source_vectors,
            }
            .precedence();
            if precedence > limit {
                break;
            }
            let invalid_assignment = is_assignment_operator(operator) && !left.unary;
            if invalid_assignment {
                self.report(
                    ParserErrorType::ExpectedStatementExpression(
                        "unary-expression left operand of assignment",
                        Some(token.kind),
                    ),
                    Some(token),
                );
            }
            self.rd_consume();
            let right_limit = if is_assignment_operator(operator) {
                precedence
            } else {
                precedence - 1
            };
            let right = self
                .rd_precedence(right_limit, ExpressionMode::CastExpression, boundary)?
                .expression;
            // The frame reducer retains the error state across both operands
            // of an invalid assignment production (§6.5.16 paragraph 1).
            let right = if invalid_assignment {
                self.mark_expression_recovered(right)
            } else {
                right
            };
            let source = self
                .context
                .merge_vectors(left.expression.source_vectors, token.source_vectors);
            let source = self.context.merge_vectors(source, right.source_vectors);
            left = Operand::other(self.store_expression(
                ExpressionType::Binary {
                    operator,
                    left_expression: left.expression,
                    right_expression: right,
                },
                source,
                Some(token.source_vectors),
                self.hard_error_count > errors,
            ));
        }
        if self.hard_error_count > errors {
            left.expression = self.mark_expression_recovered(left.expression);
        }
        Some(left)
    }

    /// C99: unary-expression §6.5.3, p. 78; PDF p. 90; cast-expression
    /// §6.5.4, p. 81; PDF p. 93; postfix-expression §6.5.2, p. 69; PDF p. 81.
    fn rd_prefix(
        &mut self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        let Some(token) = self.cursor.current() else {
            return self.rd_expression_fallback(2, mode, boundary, true);
        };
        if let Some((operator, child_mode)) = prefix_operator(token.kind) {
            self.rd_consume();
            if operator == UnaryOperator::Extension {
                self.pedantic_suppression += 1;
            }
            let operand = self.rd_expression(child_mode, boundary);
            if operator == UnaryOperator::Extension && !self.resource_limit_reported {
                self.pedantic_suppression -= 1;
            }
            let operand = operand?;
            let source = self
                .context
                .merge_vectors(token.source_vectors, operand.source_vectors);
            return Some(Operand::unary(self.store_expression(
                ExpressionType::Unary {
                    operator,
                    operand_expression: operand,
                },
                source,
                Some(token.source_vectors),
                false,
            )));
        }
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Sizeof)) {
            self.rd_consume();
            if is_operator(self.cursor.current(), OperatorTokenType::OpeningParenthesis)
                && ExpressionFrame::parenthesized_type_name_follows(self)
            {
                let (ty, type_source) = self.rd_type_name()?;
                let (kind, operand_source) =
                    if is_operator(self.cursor.current(), OperatorTokenType::OpeningCurlyBrace) {
                        let compound = self.rd_compound_literal(ty, type_source)?;
                        let operand = self.rd_postfix(compound, boundary)?.expression;
                        (ExpressionType::SizeofExpr(operand), operand.source_vectors)
                    } else {
                        (ExpressionType::SizeofType(ty), type_source)
                    };
                let source = self
                    .context
                    .merge_vectors(token.source_vectors, operand_source);
                return Some(Operand::unary(self.store_expression(
                    kind,
                    source,
                    Some(token.source_vectors),
                    ty.recovered,
                )));
            }
            let operand = self.rd_expression(ExpressionMode::UnaryExpression, boundary)?;
            let source = self
                .context
                .merge_vectors(token.source_vectors, operand.source_vectors);
            return Some(Operand::unary(self.store_expression(
                ExpressionType::SizeofExpr(operand),
                source,
                Some(token.source_vectors),
                false,
            )));
        }
        if is_operator(Some(token), OperatorTokenType::OpeningParenthesis) {
            if ExpressionFrame::parenthesized_type_name_follows(self) {
                return self.rd_typed_operand(mode, boundary);
            }
            if is_operator(
                self.cursor.following(),
                OperatorTokenType::OpeningCurlyBrace,
            ) {
                return self.rd_expression_fallback(2, mode, boundary, false);
            }
            self.rd_consume();
            let child = self.rd_expression(
                ExpressionMode::Expression,
                ExpressionBoundary::ClosingParenthesis,
            )?;
            let closing =
                self.rd_expect(OperatorTokenType::ClosingParenthesis, "grouped expression");
            let source = self
                .context
                .merge_vectors(token.source_vectors, child.source_vectors);
            let source = self.context.merge_vectors(source, closing);
            let operators = self.context.merge_vectors(token.source_vectors, closing);
            let expression = self.store_expression(
                ExpressionType::Parenthesized { expression: child },
                source,
                Some(operators),
                false,
            );
            return self.rd_postfix(expression, boundary);
        }
        let kind = match token.kind {
            | TokenType::Identifier => ExpressionType::Identifier(Identifier::from_token(token)),
            | TokenType::Integer(value) => ExpressionType::Constant(Constant::Integer(value)),
            | TokenType::Float(value) => {
                if matches!(
                    value,
                    FloatTokenType::Float128(_) | FloatTokenType::ImaginaryFloat128(_)
                ) {
                    self.extension(Feature::Float128, "binary128 floating constant", token);
                }
                ExpressionType::Constant(Constant::Float(value))
            },
            | TokenType::Character(value) => ExpressionType::Constant(Constant::Char(value)),
            | TokenType::String(value) => ExpressionType::StringLiteral(value),
            | TokenType::Keyword(KeywordTokenType::True) => ExpressionType::Boolean(true),
            | TokenType::Keyword(KeywordTokenType::False) => ExpressionType::Boolean(false),
            | TokenType::Keyword(KeywordTokenType::Nullptr) => ExpressionType::Nullptr,
            // Reserved modern/GNU operand syntax and malformed operands retain
            // frame recovery; the surrounding operators still use this parser.
            | _ => return self.rd_expression_fallback(2, mode, boundary, true),
        };
        self.rd_consume();
        let expression = self.store_expression(kind, token.source_vectors, None, false);
        self.rd_postfix(expression, boundary)
    }

    fn rd_type_name(&mut self) -> Option<(&'tu TypeName<'tu>, SourceVectors)> {
        let opening = self
            .cursor
            .current()
            .expect("typed operand starts at an opening parenthesis");
        self.rd_consume();
        let mut frame = TypeNameFrame::new(self.hard_error_count);
        if ExpressionFrame::type_name_is_compound_literal(self) {
            frame = frame.with_storage();
        }
        let Some(ParseValue::TypeName(ty)) = self.study_frame(ParseFrame::TypeName(frame), false)
        else {
            return None;
        };
        let close = self.rd_expect(OperatorTokenType::ClosingParenthesis, "type name");
        let source = self
            .context
            .merge_vectors(opening.source_vectors, ty.source_vectors);
        Some((ty, self.context.merge_vectors(source, close)))
    }

    fn rd_compound_literal(
        &mut self,
        ty: &'tu TypeName<'tu>,
        source: SourceVectors,
    ) -> Option<&'tu Expression<'tu>> {
        if let Some(token) = self.cursor.current() {
            self.extension(Feature::CompoundLiterals, "compound literal", token);
        }
        let frame = InitializerFrame::new(self.arena, self.hard_error_count, true, true);
        let Some(ParseValue::Initializer(result)) =
            self.study_frame(ParseFrame::Initializer(frame), false)
        else {
            return None;
        };
        let merged = self
            .context
            .merge_vectors(source, result.initializer.source_vectors);
        Some(self.store_expression(
            ExpressionType::CompoundLiteral {
                type_name:   ty,
                initializer: result.initializer,
            },
            merged,
            Some(source),
            result.recovered,
        ))
    }

    fn rd_typed_operand(
        &mut self,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        let (ty, source) = self.rd_type_name()?;
        if is_operator(self.cursor.current(), OperatorTokenType::OpeningCurlyBrace) {
            let expression = self.rd_compound_literal(ty, source)?;
            return self.rd_postfix(expression, boundary);
        }
        if mode == ExpressionMode::UnaryExpression {
            let token = self.cursor.current();
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "compound literal initializer",
                    token.map(|x| x.kind),
                ),
                token,
            );
        }
        if matches!(ty.declaration_specifiers.type_specifiers, TypeSpecifiers::StructOrUnion(x) if x.struct_or_union == StructOrUnion::Union)
        {
            self.extension_source(Feature::UnionCasts, "cast to union", ty.source_vectors);
        }
        let operand = self.rd_expression(ExpressionMode::CastExpression, boundary)?;
        let merged = self.context.merge_vectors(source, operand.source_vectors);
        Some(Operand::other(self.store_expression(
            ExpressionType::Cast {
                target_type:        ty,
                operand_expression: operand,
            },
            merged,
            Some(source),
            ty.recovered,
        )))
    }

    fn rd_postfix(
        &mut self,
        mut base: &'tu Expression<'tu>,
        _boundary: ExpressionBoundary,
    ) -> Option<Operand<'tu>> {
        while let Some(token) = self.cursor.current() {
            if !is_postfix_starter(token.kind) {
                break;
            }
            let TokenType::Operator(operator) = token.kind else {
                unreachable!("postfix starters are punctuators")
            };
            let kind;
            let mut source = if operator == OperatorTokenType::OpeningParenthesis {
                SourceVectors::default()
            } else {
                self.context
                    .merge_vectors(base.source_vectors, token.source_vectors)
            };
            let mut operators = token.source_vectors;
            match operator {
                | OperatorTokenType::OpeningSquareBracket => {
                    self.rd_consume();
                    let child = self.rd_expression(
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingSquareBracket,
                    )?;
                    let close =
                        self.rd_expect(OperatorTokenType::ClosingSquareBracket, "subscript");
                    source = self.context.merge_vectors(source, child.source_vectors);
                    source = self.context.merge_vectors(source, close);
                    operators = self.context.merge_vectors(operators, close);
                    kind = ExpressionType::Binary {
                        operator:         BinaryOperator::Subscript,
                        left_expression:  base,
                        right_expression: child,
                    };
                },
                | OperatorTokenType::OpeningParenthesis => {
                    self.rd_consume();
                    let mut call = self.pools.take_call();
                    call.source_vectors
                        .extend([base.source_vectors, token.source_vectors]);
                    call.operator_sources.push(token.source_vectors);
                    if !is_operator(self.cursor.current(), OperatorTokenType::ClosingParenthesis) {
                        loop {
                            let argument = self.rd_expression(
                                ExpressionMode::AssignmentExpression,
                                ExpressionBoundary::Argument,
                            )?;
                            call.source_vectors.push(argument.source_vectors);
                            call.arguments.push(argument);
                            if let Some(comma) = self
                                .cursor
                                .current()
                                .filter(|token| is_operator(Some(*token), OperatorTokenType::Comma))
                            {
                                call.source_vectors.push(comma.source_vectors);
                                call.operator_sources.push(comma.source_vectors);
                                self.rd_consume();
                            } else {
                                break;
                            }
                        }
                    }
                    let close =
                        self.rd_expect(OperatorTokenType::ClosingParenthesis, "function call");
                    call.source_vectors.push(close);
                    call.operator_sources.push(close);
                    source = self.context.merge_vector_list(&call.source_vectors);
                    operators = self.context.merge_vector_list(&call.operator_sources);
                    kind = ExpressionType::Call {
                        function_expression: base,
                        arguments:           self.alloc_syntax_list(&mut call.arguments),
                    };
                    call.source_vectors.clear();
                    call.operator_sources.clear();
                    self.pools.calls.reclaim(call);
                },
                | OperatorTokenType::Period | OperatorTokenType::Arrow => {
                    self.rd_consume();
                    let member = self.cursor.current();
                    if let Some(member) =
                        member.filter(|token| matches!(token.kind, TokenType::Identifier))
                    {
                        self.rd_consume();
                        source = self.context.merge_vectors(source, member.source_vectors);
                        kind = if operator == OperatorTokenType::Period {
                            ExpressionType::DirectMember {
                                base_expression: base,
                                member:          Identifier::from_token(member),
                            }
                        } else {
                            ExpressionType::IndirectMember {
                                base_expression: base,
                                member:          Identifier::from_token(member),
                            }
                        };
                    } else {
                        self.report(
                            ParserErrorType::ExpectedMemberIdentifier(member.map(|x| x.kind)),
                            member,
                        );
                        kind = ExpressionType::Error;
                    }
                },
                | OperatorTokenType::PlusPlus | OperatorTokenType::MinusMinus => {
                    self.rd_consume();
                    kind = ExpressionType::Unary {
                        operator:           if operator == OperatorTokenType::PlusPlus {
                            UnaryOperator::PostIncrement
                        } else {
                            UnaryOperator::PostDecrement
                        },
                        operand_expression: base,
                    };
                },
                | _ => break,
            }
            base = self.store_expression(kind, source, Some(operators), false);
        }
        Some(Operand {
            expression: base,
            unary:      true,
        })
    }

    fn rd_expression_fallback(
        &mut self,
        limit: u32,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
        suspend: bool,
    ) -> Option<Operand<'tu>> {
        let fallback_mode = if limit > 2 {
            ExpressionMode::Expression
        } else {
            mode
        };
        let frame =
            ExpressionFrame::new(self.arena, fallback_mode, boundary, self.hard_error_count)
                .with_study_precedence(limit);
        let result = self.study_frame(ParseFrame::Expression(frame), suspend)?;
        let expression = match result {
            | ParseValue::Expression(result) => result.expression,
            | ParseValue::ConstantExpression(result) => result.expression.0,
            | _ => unreachable!("expression fallback returns an expression"),
        };
        let unary = match expression.kind {
            | ExpressionType::Cast { .. }
            | ExpressionType::Conditional(_)
            | ExpressionType::OmittedConditional(_) => false,
            | ExpressionType::Binary { operator, .. } => operator == BinaryOperator::Subscript,
            | _ => true,
        };
        Some(Operand { expression, unary })
    }

    pub(super) fn rd_consume(&mut self) {
        if let Some(token) = self.cursor.current() {
            self.suppress_token_diagnostic(token);
            self.cursor.consume();
        }
    }

    pub(super) fn rd_expect(
        &mut self,
        operator: OperatorTokenType,
        owner: &'static str,
    ) -> SourceVectors {
        let token = self.cursor.current();
        if let Some(token) = token.filter(|token| is_operator(Some(*token), operator)) {
            self.rd_consume();
            token.source_vectors
        } else {
            let found = token.map(|token| token.kind);
            let error = match operator {
                | OperatorTokenType::OpeningParenthesis =>
                    ParserErrorType::ExpectedOpeningParenthesisInStatement(owner, found),
                | OperatorTokenType::ClosingParenthesis =>
                    ParserErrorType::ExpectedClosingParenthesisInStatement(owner, found),
                | OperatorTokenType::ClosingSquareBracket =>
                    ParserErrorType::ExpectedClosingSquareBracketInSubscript(found),
                | OperatorTokenType::Semicolon =>
                    ParserErrorType::ExpectedSemicolonInStatement(owner, found),
                | OperatorTokenType::Colon => ParserErrorType::ExpectedColonInLabel(owner, found),
                | OperatorTokenType::OpeningCurlyBrace =>
                    ParserErrorType::ExpectedOpeningCurlyBraceInCompoundStatement(found),
                | OperatorTokenType::ClosingCurlyBrace =>
                    ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(found),
                | _ => ParserErrorType::ExpectedIsoSyntax(owner, found),
            };
            self.report(error, token);
            SourceVectors::default()
        }
    }
}

#[derive(Clone, Copy)]
struct Operand<'tu> {
    expression: &'tu Expression<'tu>,
    unary:      bool,
}

impl<'tu> Operand<'tu> {
    fn unary(expression: &'tu Expression<'tu>) -> Self {
        Self {
            expression,
            unary: true,
        }
    }

    fn other(expression: &'tu Expression<'tu>) -> Self {
        Self {
            expression,
            unary: false,
        }
    }
}

fn mode_limit(mode: ExpressionMode) -> u32 {
    match mode {
        | ExpressionMode::Expression => 15,
        | ExpressionMode::AssignmentExpression => 14,
        | ExpressionMode::ConstantExpression => 13,
        | ExpressionMode::CastExpression | ExpressionMode::UnaryExpression => 2,
    }
}
