use thiserror::Error;

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourceFile,
    SourcePosition,
    SourceVector,
    SourceVectors,
    StrExt,
    TranslationPhase,
};
use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
    },
    util::shared::SharedString,
};

enum HandleNewline {
    Newline,
    EscapedNewline,
    Other,
}

#[derive(Debug, Error)]
pub(crate) enum InitialProcessorError {
    #[error("no newline at end of file")]
    MissingFinalNewline(SourceVector),
}

impl ToDiagnostic for InitialProcessorError {
    fn to_diagnostic(&self, _context: &Context, source: SourceVectors) -> Diagnostic {
        match self {
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
            | Self::MissingFinalNewline(vector) => vector.position(context),
        }
    }
}

impl GetSeverity for InitialProcessorError {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::MissingFinalNewline(_) => ErrorSeverity::Warning,
        }
    }
}

impl GetSourceVectors for InitialProcessorError {
    fn source_vectors(&self, context: &mut Context) -> SourceVectors {
        match self {
            | Self::MissingFinalNewline(vector) => context.create_source_vectors(
                vector.position(context),
                vector.source_file_index,
                vector.length,
            ),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub(crate) struct InitialProcessor {
    last_was_newline: bool,
    source_file:      SourceFile,
}

impl GetPosition for InitialProcessor {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.source_file.index,
            column: self.source_file.column,
            line:   self.source_file.line,
        }
    }
}

impl SetPosition for InitialProcessor {
    #[inline(always)]
    fn set_position(&mut self, _context: &mut Context, position: SourcePosition) {
        let SourcePosition {
            index,
            column,
            line,
        } = position;
        self.source_file.index = index;
        self.source_file.column = column;
        self.source_file.line = line;
    }
}

impl GetSourceFileIndex for InitialProcessor {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.source_file.source_file_index
    }
}

impl SetSourceFileIndex for InitialProcessor {
    fn set_source_file_index(&mut self, _context: &mut Context, source_file_index: u32) {
        self.source_file.source_file_index = source_file_index;
    }
}

impl InitialProcessor {
    pub(crate) fn new(source_file_index: u32, source: SharedString) -> Self {
        Self {
            last_was_newline: false,
            source_file:      SourceFile::new(source_file_index, source),
        }
    }

    fn next_char(&mut self, _context: &mut Context) -> Option<char> {
        self.source_file.source.char_at(self.source_file.index)
    }

    #[inline(never)]
    #[cold]
    fn handle_trigraph_graph(
        &mut self,
        _context: &mut Context,
        to_map: char,
        next_index: usize,
    ) -> char {
        //! Marked as cold and inline(never) because it's only called when the
        //! current character is the start of a trigraph sequence, which is
        //! obviously quite rare.
        //!
        //! TODO: Benchmark this function to see if the cold attribute is
        //! net-positive on typical workloads.
        let to_yield = match to_map {
            | '=' => '#',
            | ')' => ']',
            | '!' => '|',
            | '(' => '[',
            | '\'' => '^',
            | '>' => '}',
            | '/' => '\\',
            | '<' => '{',
            | '-' => '~',
            | _ => {
                self.source_file.index = next_index;
                self.source_file.column += 1;
                return '?';
            },
        };
        // Step over `next_next`.
        self.source_file.index += 1;
        self.source_file.column += 3;
        to_yield
    }

    #[inline(never)]
    #[cold]
    fn handle_line_comment(&mut self, context: &mut Context) -> char {
        //! Marked as cold and inline(never) because it's only called when the
        //! current character is the start of a line comment, which is
        //! obviously quite rare.
        //!
        //! TODO: Benchmark this function to see if the cold attribute is
        //! net-positive on typical workloads.
        self.source_file.column += 2;
        loop {
            let Some(curr) = self.next_char(context) else {
                return ' ';
            };
            self.source_file.index += curr.len_utf8();
            let next_index = self.source_file.index;
            let next = self.next_char(context);
            if let Some(next) = next {
                self.source_file.index += next.len_utf8();
            }
            let next_next = self.next_char(context);
            match self.handle_newline(context, curr, next, next_next, next_index) {
                | HandleNewline::Newline => return ' ',
                | HandleNewline::EscapedNewline => (),
                | HandleNewline::Other => {
                    self.source_file.column += 1;
                    self.source_file.index = next_index;
                },
            }
        }
    }

    #[inline(never)]
    #[cold]
    fn handle_block_comment(&mut self, context: &mut Context) -> char {
        //! Marked as cold and inline(never) because it's only called when the
        //! current character is the start of a block comment, which is
        //! obviously quite rare.
        //!
        //! TODO: Benchmark this function to see if the cold attribute is
        //! net-positive on typical workloads.
        self.source_file.column += 1;
        loop {
            let Some(curr) = self.next_char(context) else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    self.missing_final_newline(context);
                    return ' ';
                }
                return ' ';
            };
            self.source_file.index += curr.len_utf8();
            let next = self.next_char(context);
            match (curr, next) {
                | ('*', Some('/')) => {
                    self.source_file.column += 2;
                    // Step over `next`.
                    self.source_file.index += 1;
                    return ' ';
                },
                | ('\r', Some('\n')) => {
                    self.last_was_newline = true;
                    self.source_file.column = 1;
                    self.source_file.line += 1;
                    // Step over `next`.
                    self.source_file.index += 1;
                },
                | ('\n' | '\r', _) => {
                    self.last_was_newline = true;
                    self.source_file.column = 1;
                    self.source_file.line += 1;
                },
                | (_, Some(_)) => {
                    self.last_was_newline = false;
                    self.source_file.column += 1;
                },
                | (_, None) => {
                    self.last_was_newline = false;
                    self.source_file.column += 1;
                    return ' ';
                },
            }
        }
    }

    /// Assumes that context.source.index is pointing at `next_next`.
    fn handle_newline(
        &mut self,
        _context: &mut Context,
        curr: char,
        next: Option<char>,
        next_next: Option<char>,
        next_index: usize,
    ) -> HandleNewline {
        // Canonicalize and track line endings.
        // Handle windows-style newlines.
        if curr == '\r' && next == Some('\n') {
            self.source_file.column = 1;
            self.source_file.line += 1;
            self.last_was_newline = true;
            // Discard 'next'.
            return HandleNewline::Newline;
        }

        // Handle Unix- and MacOS-style newlines.
        if matches!(curr, '\n' | '\r') {
            self.source_file.column = 1;
            self.source_file.line += 1;
            self.source_file.index = next_index;
            self.last_was_newline = true;
            return HandleNewline::Newline;
        }

        // Handle escaped newlines.
        if curr == '\\' {
            match (next, next_next) {
                | (Some('\r'), Some('\n')) => {
                    self.source_file.column = 1;
                    self.source_file.line += 1;
                    // Step over `next_next`.
                    self.source_file.index += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | (Some('\n' | '\r'), _) => {
                    self.source_file.column = 1;
                    self.source_file.line += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | _ => (),
            }
        }
        HandleNewline::Other
    }

    #[inline(never)]
    #[cold]
    fn missing_final_newline(&mut self, context: &mut Context) {
        context.missing_final_newline(SourceVector {
            index:             self.source_file.index,
            column:            self.source_file.column,
            line:              self.source_file.line,
            source_file_index: self.source_file.source_file_index,
            length:            0,
        });
    }
}

impl TranslationPhase for InitialProcessor {
    type Item = char;

    fn next_item(&mut self, context: &mut Context) -> Option<char> {
        // We need to look three characters ahead to handle translation phases 1
        // and 2. If we don't consume all three characters, we
        // backtrack. Translation phases 1 and 2 are handled in the same
        // iterator for performance reasons, because otherwise we would
        // to store characters we don't consume with source positions.

        // Index should be pointing at the start of the next token at the start
        // of every loop iteration.
        loop {
            let Some(curr) = self.next_char(context) else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    self.missing_final_newline(context);
                    return Some('\n');
                }
                return None;
            };

            self.source_file.index += curr.len_utf8();
            let next_index = self.source_file.index;

            let next = self.next_char(context);
            if let Some(next) = next {
                self.source_file.index += next.len_utf8();
            }

            let next_next = self.next_char(context);

            match self.handle_newline(context, curr, next, next_next, next_index) {
                | HandleNewline::Newline => {
                    return Some('\n');
                },
                | HandleNewline::EscapedNewline => {
                    continue;
                },
                | HandleNewline::Other => (),
            }

            break Some(match (curr, next, next_next) {
                | ('/', Some('/'), _) => self.handle_line_comment(context),
                | ('/', Some('*'), _) => self.handle_block_comment(context),
                | ('?', Some('?'), Some(to_map)) => {
                    self.last_was_newline = false;
                    self.handle_trigraph_graph(context, to_map, next_index)
                },
                | (c, _, _) => {
                    self.source_file.index = next_index;
                    self.source_file.column += 1;
                    self.last_was_newline = false;
                    c
                },
            });
        }
    }
}
