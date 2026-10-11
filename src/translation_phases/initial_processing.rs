//! Diagnostics of translation phases 1 and 2: the source file's final
//! newline. The phases themselves run over whole buffers in
//! [`super::preprocessor_tokenizer`].
//!
//! C99: §5.1.1.2p1, p. 9; PDF p. 21; the final-newline rule is §5.1.1.2p2,
//! p. 10; PDF p. 22.
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
