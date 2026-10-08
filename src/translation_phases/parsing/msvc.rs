//! MSVC extensions to translation phase 7, owned by explicit frames.
//!
//! C99: vendor extensions to declarators §6.7.5, p. 114; PDF p. 126 and
//! statements §6.8, p. 131; PDF p. 143. Microsoft Learn specifies SEH and
//! inline assembly grammar. Target assembly, ABI, layout and control-flow
//! constraints belong to later analysis.

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
        any_expression_value,
    },
    syntax::{
        Expression,
        Statement,
        StatementType,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaVec,
            Bump,
        },
    },
};

/// An opaque, balanced MSVC assembly token sequence, including its introducer.
/// C99: extension to §6.8, p. 131; PDF p. 143. Assembly interpretation is
/// deferred.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct MsAsm<'tu> {
    pub(crate) tokens:         ArenaList<'tu, Token>,
    pub(crate) braced:         bool,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// Guarded SEH compound, optional exception filter, and handler compound.
/// C99: extension to §6.8, p. 131; PDF p. 143. An absent filter denotes
/// finally; missing required syntax is represented by recovered children.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Seh<'tu> {
    pub(crate) body:            &'tu Statement<'tu>,
    pub(crate) filter:          Option<&'tu Expression<'tu>>,
    pub(crate) handler:         &'tu Statement<'tu>,
    pub(crate) handler_keyword: Option<Token>,
}

pub(super) fn calling_convention(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::Cdecl
            | KeywordTokenType::Stdcall
            | KeywordTokenType::Fastcall
            | KeywordTokenType::Vectorcall
            | KeywordTokenType::Thiscall
    )
}
pub(super) fn type_modifier(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::Ptr32
            | KeywordTokenType::Ptr64
            | KeywordTokenType::Unaligned
            | KeywordTokenType::W64
            | KeywordTokenType::Sptr
            | KeywordTokenType::Uptr
    )
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Start,
    AsmOpen,
    AsmTokens,
    TryBody,
    AwaitTryBody,
    Handler,
    FilterOpen,
    Filter,
    AwaitFilter,
    FilterClose,
    HandlerBody,
    AwaitHandlerBody,
    LeaveSemicolon,
    Finish,
}

/// Delimiter owner for MSVC statements; compound/expression children run on
/// the shared parser stack. C99: extension to §6.8, p. 131; PDF p. 143.
#[derive(Debug)]
pub(super) struct MsvcFrame<'tu, 'p> {
    keyword:           KeywordTokenType,
    phase:             Phase,
    starting_errors:   usize,
    source_vectors:    Option<SourceVectors>,
    pub(super) tokens: ArenaVec<'p, Token>,
    delimiters:        ArenaVec<'p, OperatorTokenType>,
    braced:            bool,
    body:              Option<&'tu Statement<'tu>>,
    filter:            Option<&'tu Expression<'tu>>,
    handler:           Option<&'tu Statement<'tu>>,
    handler_keyword:   Option<Token>,
}

impl<'tu, 'p> MsvcFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump, keyword: KeywordTokenType, errors: usize) -> Self {
        Self {
            keyword,
            phase: Phase::Start,
            starting_errors: errors,
            source_vectors: None,
            tokens: ArenaVec::new_in(arena),
            delimiters: ArenaVec::new_in(arena),
            braced: false,
            body: None,
            filter: None,
            handler: None,
            handler_keyword: None,
        }
    }

    fn own(&mut self, parser: &mut Parser<'_, 'tu, 'p>, token: Token) {
        if self.keyword == KeywordTokenType::MsAsm {
            self.tokens.push(token);
        }
        parser.merge_source(&mut self.source_vectors, token);
    }

    fn expected(parser: &mut Parser<'_, 'tu, 'p>, token: Option<Token>, component: &'static str) {
        parser.report(
            ParserErrorType::ExpectedMsSyntax(component, token.map(|x| x.kind)),
            token,
        );
    }

    fn missing(&self, parser: &mut Parser<'_, 'tu, 'p>) -> &'tu Statement<'tu> {
        parser.alloc_syntax(Statement {
            kind:           StatementType::Expression(super::syntax::ExpressionSlot::Missing(
                self.source_vectors.unwrap_or_default(),
            )),
            source_vectors: self.source_vectors.unwrap_or_default(),
            recovered:      true,
        })
    }

    /// Uses invocation endpoints for macros and ignores phase-2 splices.
    /// C99: phase 2 is §5.1.1.2 paragraph 1, p. 10; PDF p. 22.
    fn new_line(parser: &Parser<'_, 'tu, 'p>, previous: Token, next: Token) -> bool {
        let Some(a) = parser.context.user_source_end(previous.source_vectors) else {
            return false;
        };
        let Some(b) = parser.context.user_source_end(next.source_vectors) else {
            return false;
        };
        if a.source_file_index != b.source_file_index {
            return true;
        }
        if a.line == b.line || a.end() >= b.index as usize {
            return false;
        }
        let Some(text) = parser.context.source_text(a.source_file_index) else {
            return true;
        };
        let bytes = text.as_bytes();
        let end = b.index as usize;
        for index in a.end()..end {
            if bytes[index] != b'\n' && bytes[index] != b'\r' {
                continue;
            }
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                continue;
            }
            let before =
                if bytes[index] == b'\n' && bytes.get(index.wrapping_sub(1)) == Some(&b'\r') {
                    index - 1
                } else {
                    index
                };
            if before > 0 && bytes[before - 1] == b'\\' {
                continue;
            }
            if before >= 3
                && &bytes[before - 3..before] == b"??/"
                && parser
                    .context
                    .configuration
                    .accepts(crate::configuration::Feature::Trigraphs)
            {
                continue;
            }
            return true;
        }
        false
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | Phase::Start => {
                self.phase = match self.keyword {
                    | KeywordTokenType::Try => Phase::TryBody,
                    | KeywordTokenType::Leave => Phase::LeaveSemicolon,
                    | _ => Phase::AsmOpen,
                };
                if let Some(token) = token {
                    // The reserved `__asm` alias defers its origin until the
                    // statement owner distinguishes GNU and MSVC grammar.
                    if token.contents == KeywordTokenType::MsAsm.cache_id() {
                        parser.extension(crate::configuration::Feature::MsAsm, "__asm", token);
                    }
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    ParseAction::Continue
                }
            },
            | Phase::TryBody | Phase::HandlerBody => {
                let body = matches!(self.phase, Phase::TryBody);
                if token.is_some_and(|x| {
                    x.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                }) {
                    self.phase = if body {
                        Phase::AwaitTryBody
                    } else {
                        Phase::AwaitHandlerBody
                    };
                    ParseAction::Push(ParseFrame::CompoundStatement(CompoundStatementFrame::new(
                        parser.arena,
                        parser.hard_error_count,
                        false,
                    )))
                } else {
                    Self::expected(parser, token, "compound statement in SEH construct");
                    let missing = self.missing(parser);
                    if body {
                        self.body = Some(missing);
                        self.phase = Phase::Handler;
                    } else {
                        self.handler = Some(missing);
                        self.phase = Phase::Finish;
                    }
                    ParseAction::Continue
                }
            },
            | Phase::AwaitTryBody | Phase::AwaitHandlerBody => {
                let Some(ParseValue::CompoundStatement(child)) = returned else {
                    panic!("SEH compound child protocol")
                };
                self.source_vectors = Some(parser.context.merge_vectors(
                    self.source_vectors.unwrap_or_default(),
                    child.source_vectors,
                ));
                if matches!(self.phase, Phase::AwaitTryBody) {
                    self.body = Some(child);
                    self.phase = Phase::Handler;
                } else {
                    self.handler = Some(child);
                    self.phase = Phase::Finish;
                }
                ParseAction::Continue
            },
            | Phase::Handler => {
                if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Keyword(KeywordTokenType::Except | KeywordTokenType::Finally)
                    )
                {
                    self.handler_keyword = Some(token);
                    self.phase = if token.kind == TokenType::Keyword(KeywordTokenType::Except) {
                        Phase::FilterOpen
                    } else {
                        Phase::HandlerBody
                    };
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "`__except` or `__finally` after SEH body");
                    self.handler = Some(self.missing(parser));
                    self.phase = Phase::Finish;
                    ParseAction::Continue
                }
            },
            | Phase::FilterOpen | Phase::FilterClose => {
                let open = matches!(self.phase, Phase::FilterOpen);
                self.phase = if open {
                    Phase::Filter
                } else {
                    Phase::HandlerBody
                };
                let op = if open {
                    OperatorTokenType::OpeningParenthesis
                } else {
                    OperatorTokenType::ClosingParenthesis
                };
                if let Some(token) = token
                    && token.kind == TokenType::Operator(op)
                {
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(
                        parser,
                        token,
                        if open {
                            "`(` before SEH filter"
                        } else {
                            "`)` after SEH filter"
                        },
                    );
                    ParseAction::Reprocess
                }
            },
            | Phase::Filter => {
                self.phase = Phase::AwaitFilter;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::Expression,
                    ExpressionBoundary::ClosingParenthesis,
                    parser.hard_error_count,
                )))
            },
            | Phase::AwaitFilter => {
                let expression = any_expression_value(returned);
                self.filter = Some(expression);
                self.source_vectors = Some(parser.context.merge_vectors(
                    self.source_vectors.unwrap_or_default(),
                    expression.source_vectors,
                ));
                self.phase = Phase::FilterClose;
                ParseAction::Continue
            },
            | Phase::LeaveSemicolon => {
                self.phase = Phase::Finish;
                if let Some(token) = token
                    && token.kind == TokenType::Operator(OperatorTokenType::Semicolon)
                {
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "`;` after `__leave`");
                    ParseAction::Reprocess
                }
            },
            | Phase::AsmOpen => {
                self.braced = token.is_some_and(|x| {
                    x.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                });
                self.phase = Phase::AsmTokens;
                if self.braced {
                    self.delimiters.push(OperatorTokenType::ClosingCurlyBrace);
                    self.own(parser, token.expect("asm brace exists"));
                    ParseAction::Consume
                } else {
                    ParseAction::Continue
                }
            },
            | Phase::AsmTokens => {
                let boundary = token.is_none_or(|next| {
                    if self.braced {
                        return false;
                    }
                    if next.kind == TokenType::Keyword(KeywordTokenType::MsAsm)
                        || next.kind == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
                            && self.delimiters.is_empty()
                    {
                        return true;
                    }
                    let previous = parser.cursor.previous.expect("asm introducer consumed");
                    Self::new_line(parser, previous, next)
                });
                if boundary {
                    if !self.delimiters.is_empty() {
                        Self::expected(parser, token, "matching delimiter in MSVC assembly");
                    }
                    if !self.braced && self.tokens.len() == 1 {
                        Self::expected(parser, token, "assembly instruction after `__asm`");
                    }
                    self.phase = Phase::Finish;
                    return ParseAction::Continue;
                }
                let token = token.expect("non-boundary asm token");
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
                        self.delimiters.push(closer);
                    } else if matches!(
                        op,
                        OperatorTokenType::ClosingCurlyBrace
                            | OperatorTokenType::ClosingParenthesis
                            | OperatorTokenType::ClosingSquareBracket
                    ) {
                        if self.delimiters.last() != Some(&op) {
                            Self::expected(
                                parser,
                                Some(token),
                                "matching delimiter in MSVC assembly",
                            );
                            if !self.braced {
                                self.phase = Phase::Finish;
                                return ParseAction::Continue;
                            }
                            // A braced block owns everything up to its `}`:
                            // a closer it opened closes the unclosed inner
                            // delimiters too, and a stray one stays a token.
                            if self.delimiters.contains(&op) {
                                while self.delimiters.last() != Some(&op) {
                                    _ = self.delimiters.pop();
                                }
                            } else {
                                self.own(parser, token);
                                return ParseAction::Consume;
                            }
                        }
                        _ = self.delimiters.pop();
                        if self.braced && self.delimiters.is_empty() {
                            self.phase = Phase::Finish;
                        }
                    }
                }
                self.own(parser, token);
                ParseAction::Consume
            },
            | Phase::Finish => {
                let source_vectors = self.source_vectors.unwrap_or_default();
                let recovered = parser.hard_error_count > self.starting_errors;
                let kind = match self.keyword {
                    | KeywordTokenType::MsAsm => {
                        let tokens = parser.alloc_syntax_list(&mut self.tokens);
                        StatementType::MsAsm(parser.alloc_syntax(MsAsm {
                            tokens,
                            braced: self.braced,
                            source_vectors,
                            recovered,
                        }))
                    },
                    | KeywordTokenType::Leave => StatementType::SehLeave,
                    | _ => StatementType::Seh(parser.alloc_syntax(Seh {
                        body:            self.body.expect("SEH body completed"),
                        filter:          self.filter,
                        handler:         self.handler.expect("SEH handler completed"),
                        handler_keyword: self.handler_keyword,
                    })),
                };
                ParseAction::Reduce(ParseValue::Statement(parser.alloc_syntax(Statement {
                    kind,
                    source_vectors,
                    recovered,
                })))
            },
        }
    }
}
