//! The final physical newline is checked before line splicing removes it.
//! [`terminal_splice_length`] recognizes a backslash or enabled `??/` trigraph
//! followed by LF, CRLF, or CR at the end of the source. Whole-buffer rewriting
//! and token formation run in [`super::preprocessor_tokenizer`].
//!
//! For example, a source ending in a backslash and LF has a terminal splice of
//! two bytes. The lexer saves that physical span and reports an escaped final
//! newline when the token source reads the end of the file.
//!
//! Read [`terminal_splice_length`] first, then [`InitialProcessorError`] for
//! the missing-newline and escaped-newline diagnostics.
//!
//! Files by role:
//! - Final-newline recognition: `initial_processing.rs`.
//! - Diagnostic types and rendering: `preprocessor_tokenizer/errors.rs`,
//!   re-exported here as [`InitialProcessorError`].
//! - Source mapping and lexing: `preprocessor_tokenizer/splicing.rs` and
//!   `preprocessor_tokenizer.rs`.
//!
//! C99: §5.1.1.2 paragraph 1 (phases 1-2), pp. 9-10; PDF pp. 21-22;
//! the final-newline rule is in phase 2, p. 10; PDF p. 22.
//! Trigraph replacement is §5.2.1.1 paragraph 1, p. 18; PDF p. 30.

pub(crate) use super::preprocessor_tokenizer::errors::InitialProcessorError;

/// The length of the line splice that escapes the final newline of
/// `source`, spelled before trigraph replacement and line splicing.
/// C99: trigraph replacement §5.2.1.1p1, p. 18; PDF p. 30; line splicing
/// §5.1.1.2p2, p. 10; PDF p. 22.
pub(crate) fn terminal_splice_length(source: &str, trigraphs: bool) -> Option<usize> {
    ["\\\r\n", "??/\r\n", "\\\n", "??/\n", "\\\r", "??/\r"]
        .into_iter()
        .find(|suffix| (trigraphs || !suffix.starts_with("??")) && source.ends_with(suffix))
        .map(str::len)
}
