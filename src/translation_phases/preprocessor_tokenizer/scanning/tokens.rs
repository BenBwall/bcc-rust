//! Phase-3 token selection. [`super::super::Lexer::lex_token`] dispatches on
//! the first byte, then extends identifiers and preprocessing numbers or
//! selects the longest punctuator. Header names belong to phase-4 include
//! handling; numeric values and conversion to C tokens belong to later phases.
//!
//! C99: §6.4 paragraphs 1-4, pp. 49-50; PDF pp. 61-62;
//! identifiers §6.4.2.1 paragraph 1, p. 51; PDF p. 63;
//! preprocessing numbers §6.4.8 paragraphs 1-4, p. 65; PDF p. 77.

use super::{
    super::{
        Lexer,
        PreprocessorTokenType,
    },
    Lexed,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        SourcePosition,
        SourceVector,
    },
    util::byte_scan,
};

impl Lexer<'_, '_, '_, '_> {
    /// Lexes the token starting with `byte` at `start`.
    /// C99: maximal munch §6.4p4, p. 50; PDF p. 62; `preprocessing-token`
    /// §6.4p1, p. 49; PDF p. 61. Header names are deferred to `#include`
    /// handling (§6.4.7, pp. 64-65; PDF pp. 76-77).
    #[inline(always)]
    pub(in crate::translation_phases::preprocessor_tokenizer) fn lex_token(
        &mut self,
        start: usize,
        position: SourcePosition,
        byte: u8,
    ) -> Lexed {
        use PreprocessorTokenType as T;
        match byte {
            | b'\n' => self.spelled(start, start + 1, T::Newline),
            | b'0'..=b'9' => self.lex_number(start, start + 1),
            | b'{' => self.spelled(start, start + 1, T::OpeningCurlyBrace),
            | b'}' => self.spelled(start, start + 1, T::ClosingCurlyBrace),
            | b'(' => self.spelled(start, start + 1, T::OpeningParenthesis),
            | b')' => self.spelled(start, start + 1, T::ClosingParenthesis),
            | b'[' => self.spelled(start, start + 1, T::OpeningSquareBracket),
            | b']' => self.spelled(start, start + 1, T::ClosingSquareBracket),
            | b'L' => match self.peek(start + 1) {
                | Some(quote @ (b'"' | b'\'')) =>
                    self.lex_quoted(start, position, start + 2, quote),
                | _ => self.lex_identifier(start, start + 1),
            },
            | b'u' | b'U'
                if self
                    .context
                    .configuration
                    .accepts(Feature::UnicodeLiteralPrefixes) =>
            {
                let mut end = start + 1;
                if byte == b'u' && self.peek(end) == Some(b'8') {
                    end += 1;
                }
                match self.peek(end) {
                    | Some(quote @ (b'"' | b'\''))
                        if end == start + 1
                            || quote == b'"'
                            || self
                                .context
                                .configuration
                                .accepts(Feature::Utf8CharacterConstants) =>
                        self.lex_quoted(start, position, end + 1, quote),
                    | _ => self.lex_identifier(start, end),
                }
            },
            | b'$' if self
                .context
                .configuration
                .accepts(Feature::DollarIdentifiers) =>
                self.lex_extended_identifier(start, start),
            | b'\\' if super::ucn::decode(&self.text[start..], true).is_some() =>
                self.lex_identifier(start, start),
            | b'A'..=b'Z' | b'a'..=b'z' | b'_' => self.lex_identifier(start, start + 1),
            | b'.' => match self.peek(start + 1) {
                | Some(b'.') => match self.peek(start + 2) {
                    | Some(b'.') => self.spelled(start, start + 3, T::Ellipsis),
                    | _ => self.spelled(start, start + 1, T::Period),
                },
                | Some(b'0'..=b'9') => self.lex_number(start, start + 2),
                | _ => self.spelled(start, start + 1, T::Period),
            },
            | b'"' | b'\'' => self.lex_quoted(start, position, start + 1, byte),
            | b'#' => self.one_of(start, T::Hash, &[(b'#', T::HashHash)]),
            | b' ' | b'\t' | b'\x0b' | b'\x0c' => self.lex_whitespace(start + 1),
            | b'/' => match self.peek(start + 1) {
                | Some(b'=') => self.spelled(start, start + 2, T::ForwardSlashEquals),
                | Some(b'/') if self.context.configuration.accepts(Feature::LineComments) => {
                    self.record_spliced_extension(Feature::LineComments, "//", start, start + 2);
                    let end = self.skip_line_comment(start + 2);
                    self.lex_whitespace(end)
                },
                | Some(b'*') => {
                    let end = self.skip_block_comment(start + 2);
                    self.lex_whitespace(end)
                },
                | _ => self.spelled(start, start + 1, T::ForwardSlash),
            },
            | b'%' => match self.peek(start + 1) {
                | Some(b'=') => self.spelled(start, start + 2, T::PercentEquals),
                | Some(b'>') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::ClosingCurlyBrace),
                | Some(b':') if self.context.configuration.accepts(Feature::Digraphs) =>
                    match self.peek(start + 2) {
                        | Some(b'%') => match self.peek(start + 3) {
                            | Some(b':') => self.digraph(start, start + 4, T::HashHash),
                            | _ => self.digraph(start, start + 2, T::Hash),
                        },
                        | _ => self.digraph(start, start + 2, T::Hash),
                    },
                | _ => self.spelled(start, start + 1, T::Percent),
            },
            | b'<' => match self.peek(start + 1) {
                | Some(b':') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::OpeningSquareBracket),
                | Some(b'%') if self.context.configuration.accepts(Feature::Digraphs) =>
                    self.digraph(start, start + 2, T::OpeningCurlyBrace),
                | Some(b'=') => self.spelled(start, start + 2, T::LessThanEquals),
                | Some(b'<') => match self.peek(start + 2) {
                    | Some(b'=') => self.spelled(start, start + 3, T::LessThanLessThanEquals),
                    | _ => self.spelled(start, start + 2, T::LessThanLessThan),
                },
                | _ => self.spelled(start, start + 1, T::LessThan),
            },
            | b'>' => match self.peek(start + 1) {
                | Some(b'>') => match self.peek(start + 2) {
                    | Some(b'=') => self.spelled(start, start + 3, T::GreaterThanGreaterThanEquals),
                    | _ => self.spelled(start, start + 2, T::GreaterThanGreaterThan),
                },
                | Some(b'=') => self.spelled(start, start + 2, T::GreaterThanEquals),
                | _ => self.spelled(start, start + 1, T::GreaterThan),
            },
            | b',' => self.spelled(start, start + 1, T::Comma),
            | b';' => self.spelled(start, start + 1, T::SemiColon),
            | b'?' => self.spelled(start, start + 1, T::QuestionMark),
            | b'~' => self.spelled(start, start + 1, T::Tilde),
            | b':' if self.peek(start + 1) == Some(b'>')
                && self.context.configuration.accepts(Feature::Digraphs) =>
                self.digraph(start, start + 2, T::ClosingSquareBracket),
            | b':' => self.spelled(start, start + 1, T::Colon),
            | b'+' => self.one_of(
                start,
                T::Plus,
                &[(b'+', T::PlusPlus), (b'=', T::PlusEquals)],
            ),
            | b'-' => self.one_of(
                start,
                T::Minus,
                &[
                    (b'-', T::MinusMinus),
                    (b'=', T::MinusEquals),
                    (b'>', T::Arrow),
                ],
            ),
            | b'*' => self.one_of(start, T::Asterisk, &[(b'=', T::AsteriskEquals)]),
            | b'^' => self.one_of(start, T::Caret, &[(b'=', T::CaretEquals)]),
            | b'&' => self.one_of(
                start,
                T::Ampersand,
                &[(b'&', T::AmpersandAmpersand), (b'=', T::AmpersandEquals)],
            ),
            | b'|' => self.one_of(
                start,
                T::Pipe,
                &[(b'|', T::PipePipe), (b'=', T::PipeEquals)],
            ),
            | b'!' => self.one_of(
                start,
                T::ExclamationMark,
                &[(b'=', T::ExclamationMarkEquals)],
            ),
            | b'=' => self.one_of(start, T::Equals, &[(b'=', T::EqualsEquals)]),
            | 0x80.. if self.char_at(start).is_alphabetic() =>
                self.lex_identifier(start, start + self.char_at(start).len_utf8()),
            | _ => self.lex_other(start),
        }
    }

    /// A one-character punctuator, or a two-character one when the next
    /// character is listed.
    /// C99: `punctuator` §6.4.6p1, p. 63; PDF p. 75.
    fn one_of(
        &mut self,
        start: usize,
        single: PreprocessorTokenType,
        doubles: &[(u8, PreprocessorTokenType)],
    ) -> Lexed {
        let next = self.peek(start + 1);
        match doubles.iter().find(|(second, _)| Some(*second) == next) {
            | Some(&(_, kind)) => self.spelled(start, start + 2, kind),
            | None => self.spelled(start, start + 1, single),
        }
    }

    /// Recognizes `identifier` spellings and the phase-4 `defined` operator.
    /// C99: §6.4.2.1p1, p. 51; PDF p. 63; §6.10.1p1, pp. 147-148;
    /// PDF pp. 159-160.
    fn lex_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        end += byte_scan::identifier_run(&self.bytes[end..]);
        let next = self.peek(end);
        if matches!(next, Some(b'\\' | 0x80..))
            || (next == Some(b'$')
                && self
                    .context
                    .configuration
                    .accepts(Feature::DollarIdentifiers))
        {
            return self.lex_extended_identifier(start, end);
        }
        let kind = if &self.bytes[start..end] == b"defined" {
            PreprocessorTokenType::Defined
        } else {
            PreprocessorTokenType::Identifier
        };
        self.spelled(start, end, kind)
    }

    /// Extends `identifier` across UCNs and accepted multibyte characters.
    /// C99: §6.4.2.1p1-3, p. 51; PDF p. 63; UCN form §6.4.3p1, p. 53;
    /// PDF p. 65.
    #[cold]
    fn lex_extended_identifier(&mut self, start: usize, mut end: usize) -> Lexed {
        let mut universal = false;
        loop {
            end += byte_scan::identifier_run(&self.bytes[end..]);
            match self.peek(end) {
                | Some(b'$')
                    if self
                        .context
                        .configuration
                        .accepts(Feature::DollarIdentifiers) =>
                {
                    let position = self.tracker.advance_past_deletions(end);
                    self.record_extension(Feature::DollarIdentifiers, "$", position, 1);
                    end += 1;
                },
                | Some(b'\\') if super::ucn::decode(&self.text[end..], end == start).is_some() => {
                    end += super::ucn::decode(&self.text[end..], end == start)
                        .unwrap()
                        .1;
                    universal = true;
                },
                | Some(0x80..) if self.char_at(end).is_alphanumeric() =>
                    end += self.char_at(end).len_utf8(),
                | _ => break,
            }
        }
        let kind = if &self.bytes[start..end] == b"defined" {
            PreprocessorTokenType::Defined
        } else {
            PreprocessorTokenType::Identifier
        };
        let mut token = self.spelled(start, end, kind);
        if universal {
            let (kind, contents) =
                super::ucn::identifier(self.context, self.scratch, token.contents);
            token.kind = Some(kind);
            token.contents = contents;
        }
        token
    }

    /// Extends a `pp-number`, including exponent sign and identifier suffix.
    /// C99: §6.4.8p1-4, p. 65; PDF p. 77; maximal munch §6.4p4, p. 50;
    /// PDF p. 62.
    #[inline(always)]
    fn lex_number(&mut self, start: usize, mut end: usize) -> Lexed {
        loop {
            let run = byte_scan::number_run(&self.bytes[end..]);
            end += run;
            if run != 0
                && matches!(self.bytes[end - 1], b'e' | b'E' | b'p' | b'P')
                && matches!(self.peek(end), Some(b'+' | b'-'))
            {
                end += 1;
                continue;
            }
            match self.peek(end) {
                | Some(b'\'')
                    if self.context.configuration.accepts(Feature::DigitSeparators)
                        && let Some(after) = self.number_separator_end(end) =>
                {
                    end = after;
                    if matches!(self.bytes[end - 1], b'e' | b'E' | b'p' | b'P')
                        && matches!(self.peek(end), Some(b'+' | b'-'))
                    {
                        end += 1;
                    }
                },
                | Some(b'$')
                    if self
                        .context
                        .configuration
                        .accepts(Feature::DollarIdentifiers) =>
                    end += 1,
                | Some(b'\\') if super::ucn::decode(&self.text[end..], false).is_some() =>
                    end += super::ucn::decode(&self.text[end..], false).unwrap().1,
                | Some(0x80..) if self.char_at(end).is_alphanumeric() =>
                    end += self.char_at(end).len_utf8(),
                | Some(_) => break,
                // Reading the end of input leaves the position past any
                // trailing splice.
                | None => {
                    self.reached_eof = true;
                    break;
                },
            }
        }
        // Number spellings carry the trailing NUL that numeric conversion
        // expects.
        self.pos = end;
        Lexed {
            kind:     Some(PreprocessorTokenType::Number),
            contents: self
                .context
                .string_cache
                .intern_concat(&[&self.text[start..end], "\0"]),
        }
    }

    /// C23: §6.4.8 paragraph 1, p. 70; PDF p. 83: a separator continues a
    /// pp-number only as `' digit` or `' nondigit`, and a `nondigit` is
    /// ASCII, so a universal character name, another character, or `$` after
    /// the `'` ends the pp-number before it. Conversion then requires digits
    /// of the radix. After `' nondigit`, the grammar's `e sign` rule still
    /// applies, so `0x1'e+1` is one pp-number, as GCC lexes it.
    fn number_separator_end(&mut self, apostrophe: usize) -> Option<usize> {
        let index = apostrophe + 1;
        let byte = self.peek(index)?;
        (byte.is_ascii_alphanumeric() || byte == b'_').then_some(index + 1)
    }

    fn lex_other(&mut self, start: usize) -> Lexed {
        let character = self.char_at(start);
        let position = self.tracker.advance_past_deletions(start);
        let length = if self.tracker.at_trigraph() {
            3
        } else {
            character.len_utf8()
        };
        let entry = Self::checked_entry_index(self.file.entries.len());
        self.file
            .other_locations
            .push((entry, SourceVector::new(position, 0, length)));
        self.spelled(
            start,
            start + character.len_utf8(),
            PreprocessorTokenType::Other,
        )
    }
}
