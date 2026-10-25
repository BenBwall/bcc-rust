//! Recursive GNU extensions to translation phase 7: assembly statements and
//! declarator labels, builtin operands, and local-label declarations.
//!
//! C99: these extend expression, statement and declarator grammar (§6.5,
//! pp. 67-94; PDF pp. 79-106; §6.8, pp. 131-139; PDF pp. 143-151;
//! §6.7.5, p. 114; PDF p. 126). GCC's C Extensions and Extended Asm
//! manuals specify the vendor grammar. Types, constraints, assembly meaning,
//! builtin evaluation and label resolution belong to later analysis.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            errors::ParserErrorType,
            extensions::{
                gnu::{
                    Asm,
                    AsmOperand,
                    AsmQualifiers,
                    Builtin,
                    OffsetMember,
                },
                modern::SyntaxOperand,
            },
            grammar::{
                ProductionKind,
                expressions::{
                    ExpressionBoundary,
                    ExpressionMode,
                },
            },
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
    /// Parses GNU assembly operands, builtin arguments and local-label
    /// declarations, starting at the introducing keyword.
    /// GNU extension: GCC manual, "Extended Asm".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    pub(super) fn rd_gnu(
        &mut self,
        kind: GnuKind,
        starting_errors: usize,
    ) -> Option<GnuValue<'tu>> {
        let entry = self.enter_production(ProductionKind::Gnu)?;
        let mut source = None;
        if let Some(token) = self.cursor.current() {
            // Reserved `__asm` has a deferred GNU/MSVC origin when
            // both grammars are enabled; this owner chose GNU syntax.
            if token.uses_ambiguous_asm(self.context.configuration)
                && matches!(token.kind, TokenType::Keyword(KeywordTokenType::MsAsm))
            {
                self.extension(Feature::GnuAsm, "__asm", token);
            }
            self.rd_gnu_own(&mut source, token);
        }
        let result = match kind {
            | GnuKind::Asm { label } => self
                .rd_gnu_asm(label, source, starting_errors)
                .map(GnuValue::Asm),
            | GnuKind::Builtin(keyword) => self
                .rd_gnu_builtin(keyword, source, starting_errors)
                .map(GnuValue::Builtin),
            | GnuKind::LocalLabels => Some(self.rd_local_labels(source)),
        };
        self.leave_production(entry);
        result
    }

    /// `asm qualifiers ( template : outputs : inputs : clobbers : labels )`
    /// as a statement, or a declarator's `asm ( template )` label.
    /// C99: extension to §6.8, p. 131; PDF p. 143 and §6.7.5, p. 114; PDF p.
    /// 126. GNU extension: GCC manual, "Extended Asm".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    fn rd_gnu_asm(
        &mut self,
        label: bool,
        mut source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> Option<&'tu Asm<'tu>> {
        let mut asm = AsmState {
            qualifiers: AsmQualifiers::default(),
            template:   None,
            sections:   0,
            clobbers:   ArenaVec::new_in(self.arena),
            operands:   ArenaVec::new_in(self.arena),
            labels:     ArenaVec::new_in(self.arena),
        };
        self.pools.opaque_tokens.lend(&mut asm.clobbers);
        self.pools.asm_operands.lend(&mut asm.operands);
        self.pools.identifiers.lend(&mut asm.labels);
        if !label {
            self.rd_asm_qualifiers(&mut asm.qualifiers, &mut source);
        }
        self.rd_gnu_punctuation(
            &mut source,
            OperatorTokenType::OpeningParenthesis,
            "`(` in GNU construct",
        );
        if let Some(token) = self.cursor.current()
            && matches!(token.kind, TokenType::String(_))
        {
            asm.template = Some(token);
            self.rd_gnu_own(&mut source, token);
        } else {
            gnu_expected(
                self,
                self.cursor.current(),
                "string literal asm template or label",
            );
        }
        let parsed = self.rd_asm_sections(label, &mut asm, &mut source);
        let value = parsed.map(|()| {
            if !label && asm.qualifiers.goto != (asm.sections == 4) {
                gnu_expected(
                    self,
                    self.cursor.current(),
                    "four asm goto sections with goto qualifier",
                );
            }
            if asm.qualifiers.goto && asm.sections == 4 && asm.labels.is_empty() {
                gnu_expected(self, self.cursor.current(), "label in asm goto section");
            }
            self.rd_gnu_punctuation(
                &mut source,
                OperatorTokenType::ClosingParenthesis,
                "`)` in GNU construct",
            );
            if !label {
                self.rd_gnu_punctuation(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "`;` after GNU statement",
                );
            }
            let source_vectors = source.unwrap_or_default();
            let recovered = self.hard_error_count > starting_errors;
            let clobbers = self.alloc_syntax_list(&mut asm.clobbers);
            let operands = self.alloc_syntax_list(&mut asm.operands);
            let labels = self.alloc_syntax_list(&mut asm.labels);
            self.alloc_syntax(Asm {
                qualifiers: asm.qualifiers,
                template: asm.template,
                operands,
                clobbers,
                labels,
                sections: asm.sections,
                source_vectors,
                recovered,
            })
        });
        self.pools.opaque_tokens.reclaim(&mut asm.clobbers);
        self.pools.asm_operands.reclaim(&mut asm.operands);
        self.pools.identifiers.reclaim(&mut asm.labels);
        value
    }

    /// GNU extension: GCC manual, "Extended Asm", "Qualifiers".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    fn rd_asm_qualifiers(
        &mut self,
        qualifiers: &mut AsmQualifiers,
        source: &mut Option<SourceVectors>,
    ) {
        while let Some(token) = self.cursor.current() {
            let qualifier = match token.kind {
                | TokenType::Keyword(KeywordTokenType::Volatile) => &mut qualifiers.volatile,
                | TokenType::Keyword(KeywordTokenType::Inline) => &mut qualifiers.inline,
                | TokenType::Keyword(KeywordTokenType::Goto) => &mut qualifiers.goto,
                | _ => return,
            };
            if std::mem::replace(qualifier, true) {
                gnu_expected(self, Some(token), "distinct asm qualifier");
            }
            self.rd_gnu_own(source, token);
        }
    }

    /// Colon-separated output, input, clobber and label sections. Stops at
    /// the closing `)` or at an operand the sections cannot continue past.
    /// GNU extension: GCC manual, "Extended Asm".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    fn rd_asm_sections(
        &mut self,
        label: bool,
        asm: &mut AsmState<'tu, 'p>,
        source: &mut Option<SourceVectors>,
    ) -> Option<()> {
        'sections: while let Some(colon) = self.cursor.current()
            && matches!(colon.kind, TokenType::Operator(OperatorTokenType::Colon))
        {
            asm.sections = asm.sections.saturating_add(1);
            if asm.sections > 4 || label {
                gnu_expected(self, Some(colon), "`)` after asm sections");
            }
            self.rd_gnu_own(source, colon);
            let mut requires_operand = false;
            loop {
                let token = self.cursor.current();
                if token.is_none_or(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(
                            OperatorTokenType::Colon
                                | OperatorTokenType::ClosingParenthesis
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                }) {
                    if requires_operand {
                        gnu_expected(self, token, "asm operand after comma");
                    }
                    continue 'sections;
                }
                if !self.rd_asm_operand(asm, source)? {
                    return Some(());
                }
                if let Some(comma) = self.cursor.current()
                    && matches!(comma.kind, TokenType::Operator(OperatorTokenType::Comma))
                {
                    self.rd_gnu_own(source, comma);
                    requires_operand = true;
                } else {
                    continue 'sections;
                }
            }
        }
        Some(())
    }

    /// One goto label, clobber literal, or `[name] "constraint" (expression)`
    /// operand of the current section. Returns `false` when the operand is
    /// malformed, which ends the sections.
    /// GNU extension: GCC manual, "Extended Asm".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    fn rd_asm_operand(
        &mut self,
        asm: &mut AsmState<'tu, 'p>,
        source: &mut Option<SourceVectors>,
    ) -> Option<bool> {
        let token = self.cursor.current();
        if asm.sections >= 4 {
            if let Some(token) = token
                && matches!(token.kind, TokenType::Identifier)
            {
                asm.labels.push(Identifier::from_token(token));
                self.rd_gnu_own(source, token);
                return Some(true);
            }
            gnu_expected(self, token, "asm goto label");
            return Some(false);
        }
        if asm.sections == 3 {
            if let Some(token) = token
                && matches!(token.kind, TokenType::String(_))
            {
                asm.clobbers.push(token);
                self.rd_gnu_own(source, token);
                return Some(true);
            }
            gnu_expected(self, token, "string literal asm clobber");
            return Some(false);
        }
        let mut name = None;
        if let Some(token) = token
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
            )
        {
            self.rd_gnu_own(source, token);
            if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::Identifier)
            {
                name = Some(Identifier::from_token(token));
                self.rd_gnu_own(source, token);
            } else {
                gnu_expected(
                    self,
                    self.cursor.current(),
                    "identifier in asm operand name",
                );
            }
            self.rd_gnu_punctuation(
                source,
                OperatorTokenType::ClosingSquareBracket,
                "`]` in asm operand name",
            );
        }
        let Some(constraint) = self
            .cursor
            .current()
            .filter(|token| matches!(token.kind, TokenType::String(_)))
        else {
            gnu_expected(self, self.cursor.current(), "string literal asm constraint");
            return Some(false);
        };
        self.rd_gnu_own(source, constraint);
        if let Some(token) = self.cursor.current()
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            )
        {
            self.rd_gnu_own(source, token);
        } else {
            gnu_expected(
                self,
                self.cursor.current(),
                "`(` before asm operand expression",
            );
            return Some(false);
        }
        let expression = self.rd_gnu_expression(
            source,
            ExpressionMode::Expression,
            ExpressionBoundary::ClosingParenthesis,
        )?;
        asm.operands.push(AsmOperand {
            name,
            constraint,
            expression,
            output: asm.sections == 1,
        });
        self.rd_gnu_punctuation(
            source,
            OperatorTokenType::ClosingParenthesis,
            "`)` after asm operand expression",
        );
        Some(true)
    }

    /// Typed builtin operands, and the member designator of
    /// `__builtin_offsetof`.
    /// C99: extension to primary expressions §6.5.1, p. 69; PDF p. 81.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    /// GNU extension: GCC manual, "Offsetof".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Offsetof.html>
    fn rd_gnu_builtin(
        &mut self,
        keyword: KeywordTokenType,
        mut source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> Option<&'tu Builtin<'tu>> {
        let mut operands = ArenaVec::new_in(self.arena);
        let mut members = ArenaVec::new_in(self.arena);
        self.pools.builtin_operands.lend(&mut operands);
        self.pools.offset_members.lend(&mut members);
        let parsed = if self.cursor.current().is_none_or(|x| {
            !matches!(
                x.kind,
                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
            )
        }) {
            // Recover the GNU primary-expression extension without
            // operands, leaving the caller's statement boundary intact.
            // C99 §6.5.1p1, p. 69; PDF p. 81 and §6.8.3p1,
            // p. 132; PDF p. 144: the statement owns its semicolon.
            gnu_expected(self, self.cursor.current(), "`(` in GNU construct");
            Some(false)
        } else {
            self.rd_gnu_punctuation(
                &mut source,
                OperatorTokenType::OpeningParenthesis,
                "`(` in GNU construct",
            );
            self.rd_builtin_operands(keyword, &mut source, &mut operands, &mut members)
                .map(|()| true)
        };
        let value = parsed.map(|close| {
            if close {
                self.rd_gnu_punctuation(
                    &mut source,
                    OperatorTokenType::ClosingParenthesis,
                    "`)` in GNU construct",
                );
            }
            let source_vectors = source.unwrap_or_default();
            let recovered = self.hard_error_count > starting_errors;
            let operands = self.alloc_syntax_list(&mut operands);
            let members = self.alloc_syntax_list(&mut members);
            self.alloc_syntax(Builtin {
                keyword,
                operands,
                members,
                source_vectors,
                recovered,
            })
        });
        self.pools.builtin_operands.reclaim(&mut operands);
        self.pools.offset_members.reclaim(&mut members);
        value
    }

    fn rd_builtin_operands(
        &mut self,
        keyword: KeywordTokenType,
        source: &mut Option<SourceVectors>,
        operands: &mut ArenaVec<'p, SyntaxOperand<'tu>>,
        members: &mut ArenaVec<'p, OffsetMember<'tu>>,
    ) -> Option<()> {
        let count = match keyword {
            | KeywordTokenType::BuiltinChooseExpr => 3,
            | KeywordTokenType::BuiltinVaEnd => 1,
            | _ => 2,
        };
        loop {
            let type_operand = match keyword {
                | KeywordTokenType::BuiltinVaArg | KeywordTokenType::BuiltinConvertVector =>
                    operands.len() == 1,
                | KeywordTokenType::BuiltinBitCast => operands.is_empty(),
                | KeywordTokenType::BuiltinOffsetof | KeywordTokenType::BuiltinTypesCompatible =>
                    true,
                | _ => false,
            };
            let operand = if type_operand {
                SyntaxOperand::Type(self.rd_type_name_syntax(self.hard_error_count, false)?)
            } else {
                let mode = if keyword == KeywordTokenType::BuiltinChooseExpr && operands.is_empty()
                {
                    ExpressionMode::ConstantExpression
                } else {
                    ExpressionMode::AssignmentExpression
                };
                SyntaxOperand::Expression(
                    self.rd_expression_ending_at(mode, ExpressionBoundary::Argument)?
                        .expression,
                )
            };
            self.context.merge_into(source, operand.source());
            operands.push(operand);
            if operands.len() >= count {
                return Some(());
            }
            self.rd_gnu_punctuation(
                source,
                OperatorTokenType::Comma,
                "`,` between builtin operands",
            );
            if keyword == KeywordTokenType::BuiltinOffsetof {
                return self.rd_offsetof_members(source, members);
            }
        }
    }

    /// `identifier`, then `.identifier` and `[expression]` suffixes.
    /// GNU extension: GCC manual, "Offsetof".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Offsetof.html>
    /// C99: §7.17 paragraph 3, p. 254; PDF p. 266.
    fn rd_offsetof_members(
        &mut self,
        source: &mut Option<SourceVectors>,
        members: &mut ArenaVec<'p, OffsetMember<'tu>>,
    ) -> Option<()> {
        'field: loop {
            let Some(field) = self
                .cursor
                .current()
                .filter(|token| matches!(token.kind, TokenType::Identifier))
            else {
                gnu_expected(
                    self,
                    self.cursor.current(),
                    "member identifier in __builtin_offsetof",
                );
                return Some(());
            };
            members.push(OffsetMember::Field(Identifier::from_token(field)));
            self.rd_gnu_own(source, field);
            loop {
                let Some(token) = self.cursor.current() else {
                    return Some(());
                };
                match token.kind {
                    | TokenType::Operator(OperatorTokenType::Period) => {
                        self.rd_gnu_own(source, token);
                        continue 'field;
                    },
                    | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) => {
                        self.rd_gnu_own(source, token);
                        let expression = self.rd_gnu_expression(
                            source,
                            ExpressionMode::Expression,
                            ExpressionBoundary::ClosingSquareBracket,
                        )?;
                        members.push(OffsetMember::Index(expression));
                        self.rd_gnu_punctuation(
                            source,
                            OperatorTokenType::ClosingSquareBracket,
                            "`]` in offsetof member path",
                        );
                    },
                    | _ => return Some(()),
                }
            }
        }
    }

    /// `identifier-list ;` after `__label__`.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    fn rd_local_labels(&mut self, mut source: Option<SourceVectors>) -> GnuValue<'tu> {
        let mut labels = ArenaVec::new_in(self.arena);
        self.pools.identifiers.lend(&mut labels);
        loop {
            if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::Identifier)
            {
                labels.push(Identifier::from_token(token));
                self.rd_gnu_own(&mut source, token);
            } else {
                gnu_expected(
                    self,
                    self.cursor.current(),
                    "identifier in __label__ declaration",
                );
                break;
            }
            if let Some(token) = self.cursor.current()
                && matches!(token.kind, TokenType::Operator(OperatorTokenType::Comma))
            {
                self.rd_gnu_own(&mut source, token);
            } else {
                break;
            }
        }
        self.rd_gnu_punctuation(
            &mut source,
            OperatorTokenType::Semicolon,
            "`;` after GNU statement",
        );
        let value = GnuValue::LocalLabels(
            self.alloc_syntax_list(&mut labels),
            source.unwrap_or_default(),
        );
        self.pools.identifiers.reclaim(&mut labels);
        value
    }

    fn rd_gnu_expression(
        &mut self,
        source: &mut Option<SourceVectors>,
        mode: ExpressionMode,
        boundary: ExpressionBoundary,
    ) -> Option<&'tu Expression<'tu>> {
        let expression = self.rd_expression_ending_at(mode, boundary)?.expression;
        self.context.merge_into(source, expression.source_vectors);
        Some(expression)
    }

    fn rd_gnu_own(&mut self, source: &mut Option<SourceVectors>, token: Token) {
        self.merge_source(source, token);
        self.rd_consume();
    }

    fn rd_gnu_punctuation(
        &mut self,
        source: &mut Option<SourceVectors>,
        operator: OperatorTokenType,
        position: &'static str,
    ) {
        let token = self.cursor.current();
        if let Some(token) = token
            && matches!(token.kind, TokenType::Operator(actual) if actual == operator)
        {
            self.rd_gnu_own(source, token);
        } else {
            gnu_expected(self, token, position);
        }
    }
}

/// A GNU assembly statement or label under construction.
/// GNU extension: GCC manual, "Extended Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
struct AsmState<'tu, 'p> {
    qualifiers: AsmQualifiers,
    template:   Option<Token>,
    sections:   u8,
    clobbers:   ArenaVec<'p, Token>,
    operands:   ArenaVec<'p, AsmOperand<'tu>>,
    labels:     ArenaVec<'p, Identifier>,
}

#[cold]
#[inline(never)]
fn gnu_expected(parser: &mut Parser<'_, '_, '_>, token: Option<Token>, position: &'static str) {
    parser.report(
        ParserErrorType::ExpectedGnuSyntax(position, token.map(|x| x.kind)),
        token,
    );
}

#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::parsing) enum GnuValue<'tu> {
    Asm(&'tu Asm<'tu>),
    Builtin(&'tu Builtin<'tu>),
    LocalLabels(ArenaList<'tu, Identifier>, SourceVectors),
}

/// Selects GNU assembly, builtin operand or local-label grammar.
/// GNU extension: GCC manual, "Extended Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
/// GNU extension: GCC manual, "Other Builtins".
/// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
/// GNU extension: GCC manual, "Local Labels".
/// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::translation_phases::parsing) enum GnuKind {
    Asm { label: bool },
    Builtin(KeywordTokenType),
    LocalLabels,
}
