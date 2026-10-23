//! Decodes quoted symbol names and `module asm` text into parser scratch;
//! module declarations copy their decoded names into the IR arena.

use super::{
    ParseError,
    ParseErrorKind,
    Token,
    TokenKind,
};
use crate::util::bump::{
    ArenaString,
    Bump,
};

impl<'t> Token<'t> {
    /// A symbol's decoded name. The IR accepts UTF-8 directly and quotes
    /// names with delimiters or controls; escape validation is always on.
    ///
    /// C99: §6.4.2.1 paragraph 2, p. 51; PDF p. 63 (identifier characters).
    pub(super) fn text_name<'a>(&'a self, scratch: &'a Bump) -> Result<&'a str, ParseError<'t>> {
        let TokenKind::QuotedSymbol(name) = self.kind else {
            return Ok(match self.kind {
                | TokenKind::Symbol(name) => name,
                | _ => self.text,
            });
        };
        self.decode(name, scratch)
    }

    /// A `module asm` string's decoded text, with the escapes of a quoted
    /// symbol name.
    pub(super) fn asm_text<'a>(&'a self, scratch: &'a Bump) -> Result<&'a str, ParseError<'t>> {
        let TokenKind::String(text) = self.kind else {
            return Err(self.expected("a string"));
        };
        self.decode(text, scratch)
    }

    /// Decodes `\\`, `\"` and `\u{...}` escapes; NUL is never valid.
    fn decode<'a>(&self, name: &str, scratch: &'a Bump) -> Result<&'a str, ParseError<'t>> {
        let mut decoded = ArenaString::new_in(scratch);
        let mut chars = name.chars();
        let bad_escape = || self.error(ParseErrorKind::BadSymbolEscape);
        while let Some(c) = chars.next() {
            if c != '\\' {
                if c == '\0' {
                    return Err(bad_escape());
                }
                decoded.push(c);
                continue;
            }
            let c = match chars.next() {
                | Some('"') => '"',
                | Some('\\') => '\\',
                | Some('u') if chars.next() == Some('{') => {
                    let mut scalar = 0_u32;
                    let mut digits = 0;
                    loop {
                        let c = chars.next().ok_or_else(bad_escape)?;
                        if c == '}' {
                            break;
                        }
                        let digit = c.to_digit(16).ok_or_else(bad_escape)?;
                        scalar = scalar
                            .checked_mul(16)
                            .and_then(|s| s.checked_add(digit))
                            .ok_or_else(bad_escape)?;
                        digits += 1;
                        if digits > 6 {
                            return Err(bad_escape());
                        }
                    }
                    if digits == 0 || scalar == 0 {
                        return Err(bad_escape());
                    }
                    char::from_u32(scalar).ok_or_else(bad_escape)?
                },
                | _ => return Err(bad_escape()),
            };
            decoded.push(c);
        }
        Ok(decoded.into_str())
    }
}
