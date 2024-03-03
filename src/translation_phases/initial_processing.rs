use thiserror::Error;

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileName,
    GetSourceVectors,
    NonZeroExt,
    SetPosition,
    SetSourceFileName,
    SourceFile,
    SourcePosition,
    SourceVector,
    SourceVectors,
    StrExt,
    TranslationPhase,
    ONE,
};
use crate::util::shared::{
    SharedPath,
    SharedString,
};

enum HandleNewline {
    Newline,
    EscapedNewline,
    Other,
}

#[derive(Debug, Error)]
pub(crate) enum InitialProcessorError {
    #[error("missing final newline")]
    MissingFinalNewline(SourceVector),
}

impl GetPosition for InitialProcessorError {
    #[allow(clippy::inline_always)]
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
                vector.source_file.clone(),
                vector.length,
            ),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub(crate) struct InitialProcessor {
    last_was_newline:            bool,
    source_file:                 SourceFile,
    current_char_start_position: SourcePosition,
}

impl GetPosition for InitialProcessor {
    #[allow(clippy::inline_always)]
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
    #[allow(clippy::inline_always)]
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

impl GetSourceFileName for InitialProcessor {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn source_file_name(&self) -> SharedPath {
        self.source_file.name.clone()
    }
}

impl SetSourceFileName for InitialProcessor {
    fn set_source_file_name(&mut self, _context: &mut Context, name: SharedPath) {
        self.source_file.name = name;
    }
}

impl InitialProcessor {
    pub(crate) fn new(source_name: SharedPath, source: SharedString) -> Self {
        Self {
            last_was_newline:            false,
            source_file:                 SourceFile::new(source_name, source),
            current_char_start_position: SourcePosition::default(),
        }
    }

    pub(crate) fn current_char_start_position(&self) -> SourcePosition {
        self.current_char_start_position
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
                self.source_file.column.saturating_add_assign(1);
                return '?';
            },
        };
        // Step over `next_next`.
        self.source_file.index += 1;
        self.source_file.column.saturating_add_assign(3);
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
        self.source_file.column.saturating_add_assign(2);
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
                    self.source_file.column.saturating_add_assign(1);
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
        self.source_file.column.saturating_add_assign(2);
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
                    self.source_file.column.saturating_add_assign(2);
                    // Step over `next`.
                    self.source_file.index += 1;
                    return ' ';
                },
                | ('\r', Some('\n')) => {
                    self.last_was_newline = true;
                    self.source_file.column = ONE;
                    self.source_file.line += 1;
                    // Step over `next`.
                    self.source_file.index += 1;
                },
                | ('\n' | '\r', _) => {
                    self.last_was_newline = true;
                    self.source_file.column = ONE;
                    self.source_file.line += 1;
                },
                | (_, Some(_)) => {
                    self.last_was_newline = false;
                    self.source_file.column.saturating_add_assign(1);
                },
                | (_, None) => {
                    self.last_was_newline = false;
                    self.source_file.column.saturating_add_assign(1);
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
            self.source_file.column = ONE;
            self.source_file.line += 1;
            self.last_was_newline = true;
            // Discard 'next'.
            return HandleNewline::Newline;
        }

        // Handle Unix- and MacOS-style newlines.
        if matches!(curr, '\n' | '\r') {
            self.source_file.column = ONE;
            self.source_file.line += 1;
            self.source_file.index = next_index;
            self.last_was_newline = true;
            return HandleNewline::Newline;
        }

        // Handle escaped newlines.
        if curr == '\\' {
            match (next, next_next) {
                | (Some('\r'), Some('\n')) => {
                    self.source_file.column = ONE;
                    self.source_file.line += 1;
                    // Step over `next_next`.
                    self.source_file.index += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | (Some('\n' | '\r'), _) => {
                    self.source_file.column = ONE;
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
            index:       self.source_file.index,
            column:      self.source_file.column,
            line:        self.source_file.line,
            source_file: self.source_file.name.clone(),
            length:      0,
        });
    }

    fn impl_(&mut self, context: &mut Context) -> Option<char> {
        // We need to look three characters ahead to handle translation phases 1 and 2.
        // If we don't consume all three characters, we backtrack.
        // Translation phases 1 and 2 are handled in the same iterator for performance
        // reasons, because otherwise we would to store characters we don't consume with
        // source positions.

        // Index should be pointing at the start of the next token at the start of every
        // loop iteration.
        loop {
            self.current_char_start_position.index = self.source_file.index;
            self.current_char_start_position.column = self.source_file.column;
            self.current_char_start_position.line = self.source_file.line;

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
                    self.source_file.column.saturating_add_assign(1);
                    self.last_was_newline = false;
                    c
                },
            });
        }
    }
}

impl TranslationPhase for InitialProcessor {
    type Item = char;

    fn next_item(&mut self, context: &mut Context) -> Option<char> {
        // let ret = self.impl_(context);
        // eprintln!(
        //     "{ret:?} from {:?} to {:?}",
        //     self.current_char_start_position,
        //     self.position(context),
        // );
        // ret
        self.impl_(context)
    }
}
