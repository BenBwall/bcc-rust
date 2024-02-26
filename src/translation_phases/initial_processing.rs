use super::{
    Context,
    GetPosition,
    NonZeroExt,
    SourcePosition,
    TranslationPhase,
};

enum HandleNewline {
    Newline,
    EscapedNewline,
    Other,
}

pub(crate) enum InitialProcessingError {
    MissingFinalNewline(SourcePosition),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub(crate) struct InitialProcessor {
    last_was_newline: bool,
    current_position: SourcePosition,
}

impl GetPosition for InitialProcessor {
    fn position(&self) -> SourcePosition {
        self.current_position
    }
}

impl InitialProcessor {
    pub(crate) fn new() -> Self {
        Self {
            last_was_newline: false,
            current_position: SourcePosition::default(),
        }
    }

    #[inline(never)]
    #[cold]
    fn handle_trigraph_graph(
        &mut self,
        context: &mut Context,
        to_map: char,
        next_index: u32,
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
            | c => {
                context.source.index = next_index;
                context.source.column.saturating_add_assign(1);
                return '?';
            },
        };
        context.source.index += 1;
        context.source.column.saturating_add_assign(3);
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
        context.source.column.saturating_add_assign(2);
        loop {
            let start_index = context.source.index;
            let start_column = context.source.column;
            let start_line = context.source.line;
            let Some(c) = context.next_char() else {
                return ' ';
            };
            context.source.index += c.len_utf8();
            let next = context.next_char();
            if let Some(next) = next {
                context.source.index += next.len_utf8();
            }
            let next_next = context.next_char();
            match self.handle_newline(context, curr, next, next_next, next_index) {
                | HandleNewline::Newline => {
                    context.source.index = start_index;
                    context.source.column = start_column;
                    context.source.line = start_line;
                    return ' ';
                },
                | HandleNewline::EscapedNewline => (),
                | HandleNewline::Other => {
                    context.source.column.saturating_add_assign(1);
                    context.source.index = next_index
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
        context.source.column.saturating_add_assign(2);
        loop {
            let Some(c) = context.next_char() else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    context.missing_final_newline();
                    return ' ';
                }
                return ' ';
            };
            context.source.index += c.len_utf8();
            let next = context.next_char();
            match (curr, next) {
                | ('*', Some('/')) => {
                    context.source.column.saturating_add_assign(2);
                    context.source.index += 1;
                    return ' ';
                },
                | ('\r', Some('\n')) => {
                    self.last_was_newline = true;
                    context.source.column = ONE;
                    context.source.line += 1;
                    context.source.index += 1;
                },
                | ('\n' | '\r', _) => {
                    self.last_was_newline = true;
                    context.source.column = ONE;
                    context.source.line += 1;
                },
                | (_, Some(_)) => {
                    self.last_was_newline = false;
                    context.source.column.saturating_add_assign(1);
                },
                | (_, None) => {
                    self.last_was_newline = false;
                    context.source.column.saturating_add_assign(1);
                    return ' ';
                },
            }
        }
    }

    /// Assumes that context.source.index is pointing at next_next.
    fn handle_newline(
        &mut self,
        context: &mut Context,
        curr: char,
        next: Option<char>,
        next_next: Option<char>,
        next_index: u32,
    ) -> HandleNewline {
        // Canonicalize and track line endings.
        // Handle windows-style newlines.
        if curr == '\r' && next == Some('\n') {
            context.source.column = ONE;
            context.source.line += 1;
            self.last_was_newline = true;
            // Discard 'next'.
            return HandleNewline::Newline;
        }

        // Handle Unix- and MacOS-style newlines.
        if matches!(curr, b'\n' | b'\r') {
            context.source.column = ONE;
            context.source.line += 1;
            context.source.index = next_index;
            self.last_was_newline = true;
            return HandleNewline::Newline;
        }

        // Handle escaped newlines.
        if curr == '\\' {
            match (next, next_next) {
                | (Some('\r'), Some('\n')) => {
                    context.source.column = ONE;
                    context.source.line += 1;
                    context.source.index += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | (Some('\n' | '\r'), _) => {
                    context.source.column = ONE;
                    context.source.line += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | _ => (),
            }
        }
        HandleNewline::Other
    }
}

impl<'ctx> TranslationPhase for InitialProcessor<'ctx> {
    type Item = char;

    fn next_item(&mut self, context: &mut Context) -> Option<char> {
        // We need to look three characters ahead to handle translation phases 1 and 2.
        // If we don't consume all three characters, we backtrack.
        // Translation phases 1 and 2 are handled in the same iterator for performance
        // reasons, because otherwise we would to store characters we don't consume with
        // source positions.
        let mut start_index = context.source.index;
        loop {
            self.current_position.index = context.source.index;
            self.current_position.column = context.source.column;
            self.current_position.line = context.source.line;

            let Some(curr) = context.next_char() else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    context.missing_final_newline();
                    return Some('\n');
                }
                return None;
            };

            context.source.index += curr.len_utf8();
            let next_index = context.source.index;

            let next = context.next_char();
            if let Some(next) = next {
                context.source.index += next.len_utf8();
            }

            let next_next_index = context.source.index;
            let next_next = context.next_char();

            match self.handle_newline(context, curr, next, next_next, next_index) {
                | HandleNewline::Newline => {
                    return Some('\n');
                },
                | HandleNewline::EscapedNewline => {
                    continue;
                },
                | HandleNewline::Other => (),
            }

            Some(match (curr, next, next_next) {
                | ('/', Some('/'), _) => self.handle_line_comment(context),
                | ('/', Some('*'), _) => self.handle_block_comment(context),
                | ('?', Some('?'), Some(to_map)) => {
                    self.last_was_newline = false;
                    self.handle_trigraph_graph(context, to_map, next_index)
                },
                | (c, _, _) => {
                    context.source.index = next_index;
                    context.source.column.saturating_add_assign(1);
                    self.last_was_newline = false;
                    c
                },
            })
        }
    }
}
