//! Splits the textual form into tokens.
//!
//! Entity names (`v3`, `block1`, `slot0`) become their own tokens, and each
//! token records whether it starts its line, which is how the parser tells a
//! block header or a named result from an operand.

use super::{
    ParseError,
    ParseErrorKind,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// Splits `text` into tokens, ending with [`TokenKind::End`].
pub(super) fn lex<'t, 's>(
    text: &'t str,
    scratch: &'s Bump,
) -> Result<ArenaVec<'s, Token<'t>>, ParseError<'t>> {
    let mut lexer = Lexer {
        text,
        bytes: text.as_bytes(),
        pos: 0,
        line: 1,
        line_begin: 0,
        line_start: true,
    };
    let mut tokens = ArenaVec::new_in(scratch);
    loop {
        let token = lexer.next_token()?;
        tokens.push(token);
        if token.kind == TokenKind::End {
            return Ok(tokens);
        }
    }
}

/// One token with its source position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Token<'t> {
    pub(super) kind:       TokenKind<'t>,
    /// The token's source text; empty at the end of input.
    pub(super) text:       &'t str,
    pub(super) line:       u32,
    pub(super) column:     u32,
    /// No token precedes this one on its line.
    pub(super) line_start: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TokenKind<'t> {
    /// A keyword, opcode, type, flag or condition name.
    Ident(&'t str),
    /// `v<n>`.
    Value(u32),
    /// `block<n>`.
    Block(u32),
    /// `slot<n>`.
    Slot(u32),
    /// `@name`, holding the name without `@`.
    Symbol(&'t str),
    /// A decimal integer, with its sign apart from its magnitude.
    Int {
        negative:  bool,
        magnitude: u128,
    },
    /// A `0x` hexadecimal integer.
    Hex(u128),
    /// A double-quoted string without escapes, holding its contents.
    String(&'t str),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Equals,
    Arrow,
    Plus,
    Minus,
    Dot,
    Ellipsis,
    End,
}

struct Lexer<'t> {
    text:       &'t str,
    bytes:      &'t [u8],
    pos:        usize,
    line:       u32,
    line_begin: usize,
    line_start: bool,
}

impl<'t> Lexer<'t> {
    fn next_token(&mut self) -> Result<Token<'t>, ParseError<'t>> {
        self.skip_trivia();
        let start = self.pos;
        let line_start = self.line_start;
        self.line_start = false;
        let Some(&byte) = self.bytes.get(start) else {
            return Ok(self.token(TokenKind::End, start, line_start));
        };
        self.pos += 1;
        let kind = match byte {
            | b'(' => TokenKind::LParen,
            | b')' => TokenKind::RParen,
            | b'{' => TokenKind::LBrace,
            | b'}' => TokenKind::RBrace,
            | b'[' => TokenKind::LBracket,
            | b']' => TokenKind::RBracket,
            | b',' => TokenKind::Comma,
            | b':' => TokenKind::Colon,
            | b'=' => TokenKind::Equals,
            | b'+' => TokenKind::Plus,
            | b'.' if self.bytes[self.pos..].starts_with(b"..") => {
                self.pos += 2;
                TokenKind::Ellipsis
            },
            | b'.' => TokenKind::Dot,
            | b'-' if self.peek() == Some(b'>') => {
                self.pos += 1;
                TokenKind::Arrow
            },
            | b'-' if self.peek().is_some_and(|next| next.is_ascii_digit()) => {
                let magnitude = self.decimal(start)?;
                TokenKind::Int {
                    negative: true,
                    magnitude,
                }
            },
            | b'-' => TokenKind::Minus,
            | b'0' if matches!(self.peek(), Some(b'x' | b'X')) => {
                self.pos += 1;
                self.hexadecimal(start)?
            },
            | b'0'..=b'9' => {
                self.pos -= 1;
                let magnitude = self.decimal(start)?;
                TokenKind::Int {
                    negative: false,
                    magnitude,
                }
            },
            | b'"' => self.string(start)?,
            | b'@' => {
                let name = self.take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$')
                });
                if name.is_empty() {
                    return Err(self.error(start, ParseErrorKind::UnexpectedCharacter('@')));
                }
                TokenKind::Symbol(name)
            },
            | b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                self.pos -= 1;
                let word = self.take_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                self.word(word, start)?
            },
            | _ => {
                let character = self.text[start..].chars().next().unwrap_or('\0');
                return Err(self.error(start, ParseErrorKind::UnexpectedCharacter(character)));
            },
        };
        Ok(self.token(kind, start, line_start))
    }

    /// Skips whitespace and `;` comments, noting line breaks.
    fn skip_trivia(&mut self) {
        while let Some(&byte) = self.bytes.get(self.pos) {
            match byte {
                | b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_begin = self.pos;
                    self.line_start = true;
                },
                | b' ' | b'\t' | b'\r' => self.pos += 1,
                | b';' => _ = self.take_while(|byte| byte != b'\n'),
                | _ => return,
            }
        }
    }

    /// Classifies an identifier: an entity name or a plain word.
    fn word(&self, word: &'t str, start: usize) -> Result<TokenKind<'t>, ParseError<'t>> {
        for (index, prefix) in ["block", "slot", "v"].into_iter().enumerate() {
            if let Some(digits) = word.strip_prefix(prefix)
                && !digits.is_empty()
                && digits.bytes().all(|byte| byte.is_ascii_digit())
            {
                let number = digits
                    .parse()
                    .map_err(|_| self.error(start, ParseErrorKind::NumberTooLarge))?;
                return Ok(match index {
                    | 0 => TokenKind::Block(number),
                    | 1 => TokenKind::Slot(number),
                    | _ => TokenKind::Value(number),
                });
            }
        }
        Ok(TokenKind::Ident(word))
    }

    fn decimal(&mut self, start: usize) -> Result<u128, ParseError<'t>> {
        let digits = self.take_while(|byte| byte.is_ascii_digit());
        digits
            .parse()
            .map_err(|_| self.error(start, ParseErrorKind::NumberTooLarge))
    }

    fn hexadecimal(&mut self, start: usize) -> Result<TokenKind<'t>, ParseError<'t>> {
        let digits = self.take_while(|byte| byte.is_ascii_hexdigit());
        if digits.is_empty() {
            return Err(self.error(start, ParseErrorKind::UnexpectedCharacter('x')));
        }
        u128::from_str_radix(digits, 16)
            .map(TokenKind::Hex)
            .map_err(|_| self.error(start, ParseErrorKind::NumberTooLarge))
    }

    fn string(&mut self, start: usize) -> Result<TokenKind<'t>, ParseError<'t>> {
        let contents = self.take_while(|byte| byte != b'"' && byte != b'\n');
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(self.error(start, ParseErrorKind::UnterminatedString));
        }
        self.pos += 1;
        Ok(TokenKind::String(contents))
    }

    fn take_while(&mut self, mut keep: impl FnMut(u8) -> bool) -> &'t str {
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(|&byte| keep(byte)) {
            self.pos += 1;
        }
        &self.text[start..self.pos]
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn token(&self, kind: TokenKind<'t>, start: usize, line_start: bool) -> Token<'t> {
        Token {
            kind,
            text: &self.text[start..self.pos],
            line: self.line,
            column: self.column(start),
            line_start,
        }
    }

    fn error(&self, start: usize, kind: ParseErrorKind<'t>) -> ParseError<'t> {
        ParseError {
            line: self.line,
            column: self.column(start),
            kind,
        }
    }

    fn column(&self, start: usize) -> u32 {
        u32::try_from(start - self.line_begin + 1).unwrap_or(u32::MAX)
    }
}
