//! Quoted tokens, whitespace, and comments in translation phase 3.
//! The scanners retain newlines and replace horizontal whitespace and comments
//! with a space. Quoted-token scanning records spellings and recovers partial
//! tokens; escape validity and literal values are checked after lexing.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! comments §6.4.9 paragraphs 1-2, p. 66; PDF p. 78;
//! character constants §6.4.4.4 paragraph 1, p. 59; PDF p. 71;
//! string literals §6.4.5 paragraph 1, p. 62; PDF p. 74.

use super::{
    super::{
        Lexer,
        PreprocessorTokenType,
        PreprocessorTokenizerErrorType,
        SourcePosition,
    },
    Lexed,
};
use crate::{
    configuration::Feature,
    util::byte_scan,
};

impl Lexer<'_, '_, '_, '_> {
    /// Continues a whitespace token from `end`; comments join it.
    /// C99: comment replacement and whitespace choice §5.1.1.2p3, p. 10;
    /// PDF p. 22; comments §6.4.9p1-2, p. 66; PDF p. 78.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn lex_whitespace(
        &mut self,
        mut end: usize,
    ) -> Lexed {
        loop {
            end += byte_scan::horizontal_space_run(&self.bytes[end..]);
            match self.peek(end) {
                | Some(b'/') => match self.peek(end + 1) {
                    | Some(b'/') if self.context.configuration.accepts(Feature::LineComments) => {
                        self.record_spliced_extension(Feature::LineComments, "//", end, end + 2);
                        end = self.skip_line_comment(end + 2);
                    },
                    | Some(b'*') => end = self.skip_block_comment(end + 2),
                    | _ => break,
                },
                | Some(0x80..) if self.char_at(end).is_whitespace() =>
                    end += self.char_at(end).len_utf8(),
                | _ => break,
            }
        }
        self.respelled(end, PreprocessorTokenType::Whitespace, " ")
    }

    /// Returns the offset of the line ending that ends a `//` comment.
    /// C99: §6.4.9p2, p. 66; PDF p. 78.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn skip_line_comment(
        &mut self,
        body: usize,
    ) -> usize {
        let end = body + byte_scan::find_line_feed(&self.bytes[body..]);
        _ = self.peek(end);
        end
    }

    /// Returns the offset after the `*/` that ends a block comment, or the
    /// end of input.
    /// C99: §6.4.9p1, p. 66; PDF p. 78; partial-comment constraint §5.1.1.2p3,
    /// p. 10; PDF p. 22.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn skip_block_comment(
        &mut self,
        mut end: usize,
    ) -> usize {
        let body = end;
        loop {
            end += byte_scan::block_comment_run(&self.bytes[end..]);
            match self.peek(end) {
                // Unterminated: the token ends past any trailing splice.
                | None => {
                    let start = self.tracker.advance_past_deletions(body - 2);
                    self.diagnose_to_eof(
                        PreprocessorTokenizerErrorType::UnterminatedBlockComment,
                        start,
                    );
                    return end;
                },
                | Some(b'*') => {
                    end += 1;
                    if self.peek(end) == Some(b'/') {
                        return end + 1;
                    }
                },
                | Some(_) if end < self.bytes.len() => end += 1,
                // The supplied final newline.
                | Some(_) => {},
            }
        }
    }

    /// Lexes a string or character literal whose body starts at `body`.
    /// C99: `character-constant` and `escape-sequence` §6.4.4.4p1, p. 59;
    /// PDF p. 71; `string-literal` §6.4.5p1, p. 62; PDF p. 74. Escape validity
    /// and value are checked after lexing.
    pub(in crate::translation_phases::preprocessor_tokenizer) fn lex_quoted(
        &mut self,
        start: usize,
        position: SourcePosition,
        body: usize,
        quote: u8,
    ) -> Lexed {
        use PreprocessorTokenType as T;
        use PreprocessorTokenizerErrorType as E;
        let (kind, unterminated, newline) = match quote {
            | b'"' => (T::String, E::UnterminatedString, E::NewlineInString),
            | _ => (
                T::Character,
                E::UnterminatedCharacter,
                E::NewlineInCharacter,
            ),
        };
        let mut end = body;
        loop {
            end += byte_scan::literal_run(&self.bytes[end..]);
            match self.peek(end) {
                | None => {
                    self.diagnose_to_eof(unterminated, position);
                    return self.spelled(start, end, kind);
                },
                | Some(b'\\') => {
                    end += 1;
                    match self.peek(end) {
                        | Some(b'\n') | None => {},
                        | Some(_) => end += self.char_at(end).len_utf8(),
                    }
                },
                | Some(b'\n') => {
                    self.diagnose_to(newline, position, end);
                    // Close the literal for recovery.
                    let quote = [quote];
                    let quote = std::str::from_utf8(&quote).expect("quotes are ASCII");
                    self.pos = end;
                    return Lexed {
                        kind:     Some(kind),
                        contents: self
                            .context
                            .string_cache
                            .intern_concat(&[&self.text[start..end], quote]),
                    };
                },
                | Some(byte) if byte == quote => return self.spelled(start, end + 1, kind),
                | Some(_) => end += 1,
            }
        }
    }
}
