//! Recursive MSVC extensions to translation phase 7: structured exception
//! handling statements, `__leave`, and `__asm` blocks.
//!
//! C99: vendor extensions to statements §6.8, p. 131; PDF p. 143. Microsoft
//! Learn specifies SEH and inline assembly grammar. Target assembly, ABI,
//! layout and control-flow constraints belong to later analysis.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            errors::ParserErrorType,
            extensions::msvc::{
                MsAsm,
                Seh,
                new_line,
            },
            grammar::{
                ProductionKind,
                expressions::{
                    ExpressionBoundary,
                    ExpressionMode,
                },
            },
            syntax::{
                ExpressionSlot,
                Statement,
                StatementType,
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

impl<'tu> Parser<'_, 'tu, '_> {
    /// Parses Microsoft assembly, SEH handlers and `__leave` statements,
    /// starting at `keyword`.
    /// MSVC extension: Microsoft Learn, "__asm".
    /// <https://learn.microsoft.com/en-us/cpp/assembler/inline/asm>
    /// MSVC extension: Microsoft Learn, "try-except statement".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement>
    /// MSVC extension: Microsoft Learn, "try-finally statement".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/try-finally-statement>
    pub(super) fn rd_msvc(
        &mut self,
        keyword: KeywordTokenType,
        starting_errors: usize,
    ) -> Option<&'tu Statement<'tu>> {
        let entry = self.enter_production(ProductionKind::Msvc)?;
        let mut tokens = ArenaVec::new_in(self.arena);
        self.pools.opaque_tokens.lend(&mut tokens);
        let mut source = None;
        if let Some(token) = self.cursor.current() {
            // The reserved `__asm` alias defers its origin until the
            // statement owner distinguishes GNU and MSVC grammar.
            if token.uses_ambiguous_asm(self.context.configuration) {
                self.extension(Feature::MsAsm, "__asm", token);
            }
            if keyword == KeywordTokenType::MsAsm {
                tokens.push(token);
            }
            self.merge_source(&mut source, token);
            self.rd_consume();
        }
        let result = match keyword {
            | KeywordTokenType::Try => self.rd_seh(source, starting_errors),
            | KeywordTokenType::Leave => Some(self.rd_seh_leave(source, starting_errors)),
            | _ => Some(self.rd_ms_asm(&mut tokens, source, starting_errors)),
        };
        self.pools.opaque_tokens.reclaim(&mut tokens);
        self.leave_production(entry);
        result
    }

    /// `__try compound (__except ( expression ) | __finally) compound`.
    /// C99: extension to §6.8, p. 131; PDF p. 143.
    /// MSVC extension: Microsoft Learn, "try-except statement".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement>
    /// MSVC extension: Microsoft Learn, "try-finally statement".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/try-finally-statement>
    fn rd_seh(
        &mut self,
        mut source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> Option<&'tu Statement<'tu>> {
        let body = self.rd_seh_compound(&mut source)?;
        let mut filter = None;
        let mut handler_keyword = None;
        let handler = if let Some(token) = self.cursor.current()
            && matches!(
                token.kind,
                TokenType::Keyword(KeywordTokenType::Except | KeywordTokenType::Finally)
            ) {
            handler_keyword = Some(token);
            self.merge_source(&mut source, token);
            self.rd_consume();
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Except)) {
                self.rd_seh_punctuation(
                    &mut source,
                    OperatorTokenType::OpeningParenthesis,
                    "`(` before SEH filter",
                );
                let expression = self
                    .rd_expression_ending_at(
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                    )?
                    .expression;
                filter = Some(expression);
                self.context
                    .merge_into(&mut source, expression.source_vectors);
                self.rd_seh_punctuation(
                    &mut source,
                    OperatorTokenType::ClosingParenthesis,
                    "`)` after SEH filter",
                );
            }
            self.rd_seh_compound(&mut source)?
        } else {
            ms_expected(
                self,
                self.cursor.current(),
                "`__except` or `__finally` after SEH body",
            );
            self.rd_seh_missing(source)
        };
        let source_vectors = source.unwrap_or_default();
        let recovered = self.hard_error_count > starting_errors;
        let kind = StatementType::Seh(self.alloc_syntax(Seh {
            body,
            filter,
            handler,
            handler_keyword,
        }));
        Some(self.alloc_syntax(Statement {
            kind,
            source_vectors,
            recovered,
        }))
    }

    fn rd_seh_compound(
        &mut self,
        source: &mut Option<SourceVectors>,
    ) -> Option<&'tu Statement<'tu>> {
        if self.cursor.current().is_some_and(|x| {
            matches!(
                x.kind,
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            )
        }) {
            let child = self.rd_compound(false)?;
            self.context.merge_into(source, child.source_vectors);
            Some(child)
        } else {
            ms_expected(
                self,
                self.cursor.current(),
                "compound statement in SEH construct",
            );
            Some(self.rd_seh_missing(*source))
        }
    }

    /// A recovered placeholder for a missing SEH compound.
    fn rd_seh_missing(&mut self, source: Option<SourceVectors>) -> &'tu Statement<'tu> {
        self.alloc_syntax(Statement {
            kind:           StatementType::Expression(ExpressionSlot::Missing(
                source.unwrap_or_default(),
            )),
            source_vectors: source.unwrap_or_default(),
            recovered:      true,
        })
    }

    /// MSVC extension: Microsoft Learn, "try-except statement", `__leave`.
    /// <https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement>
    fn rd_seh_leave(
        &mut self,
        mut source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> &'tu Statement<'tu> {
        if let Some(token) = self.cursor.current()
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Semicolon)
            )
        {
            self.merge_source(&mut source, token);
            self.rd_consume();
        } else {
            ms_expected(self, self.cursor.current(), "`;` after `__leave`");
        }
        let source_vectors = source.unwrap_or_default();
        let recovered = self.hard_error_count > starting_errors;
        self.alloc_syntax(Statement {
            kind: StatementType::SehLeave,
            source_vectors,
            recovered,
        })
    }

    /// A braced `__asm { ... }` block, or the tokens of one `__asm`
    /// instruction up to the end of its source line, kept as opaque balanced
    /// tokens.
    /// C99: extension to §6.8, p. 131; PDF p. 143.
    /// MSVC extension: Microsoft Learn, "__asm".
    /// <https://learn.microsoft.com/en-us/cpp/assembler/inline/asm>
    fn rd_ms_asm(
        &mut self,
        tokens: &mut ArenaVec<'_, Token>,
        mut source: Option<SourceVectors>,
        starting_errors: usize,
    ) -> &'tu Statement<'tu> {
        let mut delimiters = ArenaVec::new_in(self.arena);
        self.pools.delimiters.lend(&mut delimiters);
        let opening = self.cursor.current().filter(|x| {
            matches!(
                x.kind,
                TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
            )
        });
        let braced = opening.is_some();
        // Assembly operands are not C syntax, so C extension
        // diagnostics about their spelling do not apply.
        self.pedantic_suppression += 1;
        if let Some(opening) = opening {
            delimiters.push(OperatorTokenType::ClosingCurlyBrace);
            tokens.push(opening);
            self.merge_source(&mut source, opening);
            self.rd_consume();
        }
        loop {
            let token = self.cursor.current();
            let boundary = token.is_none_or(|next| {
                if braced {
                    return false;
                }
                if matches!(next.kind, TokenType::Keyword(KeywordTokenType::MsAsm))
                    || matches!(
                        next.kind,
                        TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                    ) && delimiters.is_empty()
                {
                    return true;
                }
                let previous = self.cursor.previous.expect("asm introducer consumed");
                new_line(self, previous, next)
            });
            if boundary {
                if !delimiters.is_empty() {
                    ms_expected(self, token, "matching delimiter in MSVC assembly");
                }
                if !braced && tokens.len() == 1 {
                    ms_expected(self, token, "assembly instruction after `__asm`");
                }
                break;
            }
            let token = token.expect("non-boundary asm token");
            let mut finished = false;
            if let TokenType::Operator(op) = token.kind {
                let closer = match op {
                    | OperatorTokenType::OpeningCurlyBrace =>
                        Some(OperatorTokenType::ClosingCurlyBrace),
                    | OperatorTokenType::OpeningParenthesis =>
                        Some(OperatorTokenType::ClosingParenthesis),
                    | OperatorTokenType::OpeningSquareBracket =>
                        Some(OperatorTokenType::ClosingSquareBracket),
                    | _ => None,
                };
                if let Some(closer) = closer {
                    delimiters.push(closer);
                } else if matches!(
                    op,
                    OperatorTokenType::ClosingCurlyBrace
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket
                ) {
                    let mut stray = false;
                    if delimiters.last() != Some(&op) {
                        ms_expected(self, Some(token), "matching delimiter in MSVC assembly");
                        if !braced {
                            break;
                        }
                        // A braced block owns everything up to its `}`:
                        // a closer it opened closes the unclosed inner
                        // delimiters too, and a stray one stays a token.
                        if delimiters.contains(&op) {
                            while delimiters.last() != Some(&op) {
                                _ = delimiters.pop();
                            }
                        } else {
                            stray = true;
                        }
                    }
                    if !stray {
                        _ = delimiters.pop();
                        finished = braced && delimiters.is_empty();
                    }
                }
            }
            tokens.push(token);
            self.merge_source(&mut source, token);
            self.rd_consume();
            if finished {
                break;
            }
        }
        self.pools.delimiters.reclaim(&mut delimiters);
        let source_vectors = source.unwrap_or_default();
        let recovered = self.hard_error_count > starting_errors;
        self.pedantic_suppression -= 1;
        let tokens = self.alloc_syntax_list(tokens);
        let kind = StatementType::MsAsm(self.alloc_syntax(MsAsm {
            tokens,
            braced,
            source_vectors,
            recovered,
        }));
        self.alloc_syntax(Statement {
            kind,
            source_vectors,
            recovered,
        })
    }

    fn rd_seh_punctuation(
        &mut self,
        source: &mut Option<SourceVectors>,
        operator: OperatorTokenType,
        component: &'static str,
    ) {
        let token = self.cursor.current();
        if let Some(token) = token
            && matches!(token.kind, TokenType::Operator(actual) if actual == operator)
        {
            self.merge_source(source, token);
            self.rd_consume();
        } else {
            ms_expected(self, token, component);
        }
    }
}

#[cold]
#[inline(never)]
fn ms_expected(parser: &mut Parser<'_, '_, '_>, token: Option<Token>, component: &'static str) {
    parser.report(
        ParserErrorType::ExpectedMsSyntax(component, token.map(|x| x.kind)),
        token,
    );
}
