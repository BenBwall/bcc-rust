use std::{
    borrow::BorrowMut,
    cell::{
        Ref,
        RefCell,
        RefMut,
    },
};

use super::{
    Context,
    SourcePosition,
};

enum HandleNewline {
    Newline,
    EscapedNewline,
    Other,
}

pub(crate) struct InitialProcessor<'ctx> {
    last_was_newline: bool,
    current_position: SourcePosition,
    context:          &'ctx RefCell<Context>,
}

impl<'ctx> InitialProcessor<'ctx> {
    pub(crate) fn new(context: &'ctx RefCell<Context>, current_position: SourcePosition) -> Self {
        Self {
            last_was_newline: false,
            current_position,
            context,
        }
    }

    #[inline(never)]
    #[cold]
    fn handle_trigraph_graph(
        &mut self,
        mut borrow: RefMut<'_, Context>,
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
                borrow.source.index = next_index;
                borrow.source.column += 1;
                return '?';
            },
        };
        borrow.source.index += 1;
        borrow.source.column += 3;
        to_yield
    }

    #[inline(never)]
    #[cold]
    fn handle_line_comment(&mut self, mut borrow: RefMut<'_, Context>) -> Option<char> {
        //! Marked as cold and inline(never) because it's only called when the
        //! current character is the start of a line comment, which is
        //! obviously quite rare.
        //!
        //! TODO: Benchmark this function to see if the cold attribute is
        //! net-positive on typical workloads.
        borrow.source.column += 2;
        loop {
            let Some(c) = borrow.next_char() else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    borrow.missing_final_newline();
                    return Some('\n');
                }
                return None;
            };
            borrow.source.index += c.len_utf8();
            let next = borrow.next_char();
            if let Some(next) = next {
                borrow.source.index += next.len_utf8();
            }
            let next_next = borrow.next_char();
            match self.handle_newline(borrow, curr, next, next_next, next_index) {
                | HandleNewline::Newline => return Some('\n'),
                | HandleNewline::EscapedNewline => (),
                | HandleNewline::Other => {
                    borrow.source.column += 1;
                    borrow.source.index = next_index
                },
            }
        }
    }

    #[inline(never)]
    #[cold]
    fn handle_block_comment(&mut self, mut borrow: RefMut<'_, Context>) {
        //! Marked as cold and inline(never) because it's only called when the
        //! current character is the start of a block comment, which is
        //! obviously quite rare.
        //!
        //! TODO: Benchmark this function to see if the cold attribute is
        //! net-positive on typical workloads.
        borrow.source.column += 2;
        loop {
            let Some(c) = borrow.next_char() else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    borrow.missing_final_newline();
                    return;
                }
                return None;
            };
            borrow.source.index += c.len_utf8();
            let next = borrow.next_char();
            match (curr, next) {
                | ('*', Some('/')) => {
                    borrow.source.column += 2;
                    borrow.source.index += 1;
                    return;
                },
                | ('\r', Some('\n')) => {
                    self.last_was_newline = true;
                    borrow.source.column = ONE;
                    borrow.source.line += 1;
                    borrow.source.index += 1;
                },
                | ('\n' | '\r', _) => {
                    self.last_was_newline = true;
                    borrow.source.column = ONE;
                    borrow.source.line += 1;
                },
                | (_, Some(_)) => {
                    self.last_was_newline = false;
                    borrow.source.column += 1;
                },
                | (_, None) => {
                    self.last_was_newline = false;
                    borrow.source.column += 1;
                    return;
                },
            }
        }
    }

    /// Assumes that context.source.index is pointing at next_next.
    fn handle_newline(
        &mut self,
        mut borrow: RefMut<'_, Context>,
        curr: char,
        next: Option<char>,
        next_next: Option<char>,
        next_index: u32,
    ) -> HandleNewline {
        // Canonicalize and track line endings.
        // Handle windows-style newlines.
        if curr == '\r' && next == Some('\n') {
            borrow.source.column = ONE;
            borrow.source.line += 1;
            self.last_was_newline = true;
            // Discard 'next'.
            return HandleNewline::Newline;
        }

        // Handle Unix- and MacOS-style newlines.
        if matches!(curr, b'\n' | b'\r') {
            borrow.source.column = ONE;
            borrow.source.line += 1;
            borrow.source.index = next_index;
            self.last_was_newline = true;
            return HandleNewline::Newline;
        }

        // Handle escaped newlines.
        if curr == '\\' {
            match (next, next_next) {
                | (Some('\r'), Some('\n')) => {
                    borrow.source.column = ONE;
                    borrow.source.line += 1;
                    borrow.source.index += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | (Some('\n' | '\r'), _) => {
                    borrow.source.column = ONE;
                    borrow.source.line += 1;
                    self.last_was_newline = true;
                    return HandleNewline::EscapedNewline;
                },
                | _ => (),
            }
        }
        HandleNewline::Other
    }
}

impl<'ctx> Iterator for InitialProcessor<'ctx> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        // We need to look three characters ahead to handle translation phases 1 and 2.
        // If we don't consume all three characters, we backtrack.
        // Translation phases 1 and 2 are handled in the same iterator for performance
        // reasons, because otherwise we would to store characters we don't consume with
        // source positions.
        let mut borrow = self.context.borrow_mut();
        let mut start_index = borrow.source.index;
        loop {
            self.current_position.index = borrow.source.index;
            self.current_position.column = borrow.source.column;
            self.current_position.line = borrow.source.line;

            let Some(curr) = borrow.next_char() else {
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    borrow.missing_final_newline();
                    return Some('\n');
                }
                return None;
            };

            borrow.source.index += curr.len_utf8();
            let next_index = borrow.source.index;

            let next = borrow.next_char();
            if let Some(next) = next {
                borrow.source.index += next.len_utf8();
            }

            let next_next_index = borrow.source.index;
            let next_next = borrow.next_char();

            match self.handle_newline(borrow, curr, next, next_next, next_index) {
                | HandleNewline::Newline => {
                    return Some('\n');
                },
                | HandleNewline::EscapedNewline => {
                    continue;
                },
                | HandleNewline::Other => (),
            }

            match (curr, next, next_next) {
                | ('/', Some('/'), _) => {
                    return self.handle_line_comment(borrow);
                },
                | ('/', Some('*'), _) => {
                    self.handle_block_comment(borrow);
                    continue;
                },
                | ('?', Some('?'), Some(to_map)) => {
                    self.last_was_newline = false;
                    return Some(self.handle_trigraph_graph(borrow, to_map, next_index));
                },
                | (c, _, _) => {
                    borrow.source.index = next_index;
                    borrow.source.column += 1;
                    self.last_was_newline = false;
                    return Some(c);
                },
            }
        }
    }
}
