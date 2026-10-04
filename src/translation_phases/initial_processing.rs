//! Diagnostics of translation phases 1 and 2 (C99 §5.1.1.2p1): the source
//! file's final newline. The phases themselves run over whole buffers in
//! [`super::preprocessor_tokenizer`].

use thiserror::Error;

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceVectors,
    SourcePosition,
    SourceVector,
    SourceVectors,
};
use crate::diagnostics::{
    Diagnostic,
    Explanation,
    ToDiagnostic,
};

#[derive(Debug, Error)]
pub(crate) enum InitialProcessorError {
    #[error("no newline at end of file")]
    MissingFinalNewline(SourceVector),
    #[error("final newline is escaped")]
    EscapedFinalNewline(SourceVector),
}

impl ToDiagnostic for InitialProcessorError {
    fn to_diagnostic(&self, _context: &Context, source: SourceVectors) -> Diagnostic {
        match self {
            | Self::EscapedFinalNewline(_) => Explanation::new(self.to_string())
                .label("this splice removes the final physical newline")
                .note(
                    "C99 5.1.1.2p2: the final newline shall not be immediately preceded by a \
                     backslash before splicing",
                )
                .help("add an unescaped newline at the end of the file")
                .at(self.severity(), source),
            | Self::MissingFinalNewline(_) => Explanation::new(self.to_string())
                .label("the file ends without a newline")
                .note("C99 §5.1.1.2p2: a nonempty source file shall end in a new-line character")
                .help("add a newline at the end of the file")
                .at(self.severity(), source),
        }
    }
}

impl GetPosition for InitialProcessorError {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        match self {
            | Self::MissingFinalNewline(vector) | Self::EscapedFinalNewline(vector) =>
                vector.position(context),
        }
    }
}

impl GetSeverity for InitialProcessorError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::MissingFinalNewline(_) | Self::EscapedFinalNewline(_) => ErrorSeverity::Warning,
        }
    }
}

impl GetSourceVectors for InitialProcessorError {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors {
        match self {
            | Self::MissingFinalNewline(vector) | Self::EscapedFinalNewline(vector) => context
                .create_source_vectors(
                    vector.position(context),
                    vector.source_file_index,
                    vector.length as usize,
                ),
        }
    }
}

/// The length of the line splice that escapes the final newline of
/// `source`, spelled before trigraph replacement and line splicing.
pub(crate) fn terminal_splice_length(source: &str) -> Option<usize> {
    ["\\\r\n", "??/\r\n", "\\\n", "??/\n", "\\\r", "??/\r"]
        .into_iter()
        .find(|suffix| source.ends_with(suffix))
        .map(str::len)
}
