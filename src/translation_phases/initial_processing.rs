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
    util::{
        byte_scan,
        shared::SharedString,
    },
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

#[derive(Debug, Default, PartialEq, Eq, Hash, Clone)]
pub(crate) struct InitialProcessor {
    last_was_newline:         bool,
    /// Reading-order state: rewinding a cursor alone must not duplicate EOF
    /// diagnostics.
    terminal_splice_reported: bool,
    source_file:              SourceFile,
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
    /// Physical tail spelling, before trigraph replacement and line splicing.
    pub(crate) fn terminal_splice_length(source: &str) -> Option<usize> {
        ["\\\r\n", "??/\r\n", "\\\n", "??/\n", "\\\r", "??/\r"]
            .into_iter()
            .find(|suffix| source.ends_with(suffix))
            .map(str::len)
    }

    pub(crate) fn new(source_file_index: u32, source: SharedString) -> Self {
        Self {
            last_was_newline:         source.is_empty(),
            terminal_splice_reported: false,
            source_file:              SourceFile::new(source_file_index, source),
        }
    }

    /// Returns where the character `c` just returned by
    /// [`TranslationPhase::next_item`] starts, past any line splice deleted
    /// before it, and how many source bytes spell it.
    ///
    /// Only valid immediately after `c` was returned as an ordinary or
    /// trigraph character, not as a newline or comment replacement.
    /// Whether reading at the end of input returns `None` rather than
    /// supplying a missing final newline: true when the last character read,
    /// in reading order, was a newline.
    pub(crate) fn final_newline_withheld(&self) -> bool {
        self.last_was_newline
    }

    pub(crate) fn set_final_newline_withheld(&mut self, withheld: bool) {
        self.last_was_newline = withheld;
    }

    pub(crate) fn last_char_start(&self, c: char) -> (SourcePosition, usize) {
        let end = self.source_file.index;
        let trigraph = match c {
            | '#' => Some("??="),
            | ']' => Some("??)"),
            | '|' => Some("??!"),
            | '[' => Some("??("),
            | '^' => Some("??'"),
            | '}' => Some("??>"),
            | '\\' => Some("??/"),
            | '{' => Some("??<"),
            | '~' => Some("??-"),
            | _ => None,
        }
        .filter(|spelling| {
            end.checked_sub(spelling.len())
                .and_then(|start| self.source_file.source.get(start..end))
                == Some(*spelling)
        });
        let (bytes, columns) = trigraph.map_or((c.len_utf8(), 1), |spelling| (spelling.len(), 3));
        let start = SourcePosition {
            index:  end - bytes,
            column: self.source_file.column - columns,
            line:   self.source_file.line,
        };
        (start, bytes)
    }

    fn next_char(&mut self, _context: &mut Context) -> Option<char> {
        self.source_file.source.char_at(self.source_file.index)
    }

    /// The unread source bytes, before phases 1 and 2 apply to them.
    #[inline(always)]
    pub(crate) fn raw_remaining(&self) -> &[u8] {
        &self.source_file.source.as_bytes()[self.source_file.index..]
    }

    /// Consumes the next `length` raw bytes in one step and returns them.
    ///
    /// The bytes must be ones phases 1 and 2 pass through unchanged and must
    /// not end a line, so a byte-class scan such as
    /// [`byte_scan::raw_verbatim_run`] can measure the run.
    #[inline(always)]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Columns are 32-bit like the rest of source provenance."
    )]
    pub(crate) fn consume_verbatim(&mut self, length: usize) -> &str {
        let start = self.source_file.index;
        let end = start + length;
        let text = &self.source_file.source[start..end];
        debug_assert!(
            !text
                .bytes()
                .any(|byte| matches!(byte, b'\\' | b'?' | b'\r' | b'\n')),
            "verbatim runs exclude bytes that phases 1 and 2 rewrite"
        );
        if length != 0 {
            self.source_file.index = end;
            self.source_file.column += byte_scan::count_chars(text.as_bytes()) as u32;
            self.last_was_newline = false;
            self.terminal_splice_reported = false;
        }
        text
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

    /// Deletes the line ending after a `??/` trigraph, which phase 2 splices
    /// like a backslash (C99 §5.1.1.2p1). Returns whether one was deleted.
    #[inline(never)]
    #[cold]
    fn splice_after_trigraph(&mut self) -> bool {
        let length = match self.raw_remaining() {
            | [b'\r', b'\n', ..] => 2,
            | [b'\n' | b'\r', ..] => 1,
            | _ => return false,
        };
        self.source_file.index += length;
        self.source_file.column = 1;
        self.source_file.line += 1;
        true
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

        // Handle escaped newlines. A splice deletes characters, so whether
        // the file ends in a newline still depends on what preceded it.
        if curr == '\\' {
            match (next, next_next) {
                | (Some('\r'), Some('\n')) => {
                    self.source_file.column = 1;
                    self.source_file.line += 1;
                    // Step over `next_next`.
                    self.source_file.index += 1;
                    return HandleNewline::EscapedNewline;
                },
                | (Some('\n' | '\r'), _) => {
                    self.source_file.column = 1;
                    self.source_file.line += 1;
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
        context.missing_final_newline(SourceVector::new(
            self.source_file.position(context),
            self.source_file.source_file_index,
            0,
        ));
    }
}

impl TranslationPhase for InitialProcessor {
    type Item = char;

    /// Returns the next character after translation phases 1 and 2:
    /// trigraphs replaced, line endings canonicalized to `'\n'`, and line
    /// splices deleted. Comments are recognized by the tokenizer in phase 3,
    /// after splicing, so `/\` + newline + `*` still opens one and string
    /// literals never contain one.
    fn next_item(&mut self, context: &mut Context) -> Option<char> {
        // We need to look three characters ahead to handle translation phases 1
        // and 2. If we don't consume all three characters, we
        // backtrack. Translation phases 1 and 2 are handled in the same
        // iterator for performance reasons, because otherwise we would
        // to store characters we don't consume with source positions.

        // Index should be pointing at the start of the next token at the start
        // of every loop iteration.
        loop {
            // Most source bytes are ASCII that phases 1 and 2 leave alone; skip
            // the three-character lookahead for them.
            if let Some(&byte) = self.raw_remaining().first()
                && byte.is_ascii()
                && !matches!(byte, b'\\' | b'?' | b'\r' | b'\n')
            {
                self.source_file.index += 1;
                self.source_file.column += 1;
                self.last_was_newline = false;
                self.terminal_splice_reported = false;
                return Some(char::from(byte));
            }

            let Some(curr) = self.next_char(context) else {
                let terminal_splice = Self::terminal_splice_length(&self.source_file.source);
                if let Some(length) = terminal_splice
                    && !self.terminal_splice_reported
                {
                    self.terminal_splice_reported = true;
                    let index = self.source_file.source.len() - length;
                    let line_start = self.source_file.source[..index]
                        .rfind(['\r', '\n'])
                        .map_or(0, |i| i + 1);
                    let position = SourcePosition {
                        index,
                        line: self.source_file.line.saturating_sub(1),
                        column: u32::try_from(
                            self.source_file.source[line_start..index].chars().count() + 1,
                        )
                        .unwrap_or(u32::MAX),
                    };
                    context.escaped_final_newline(SourceVector::new(
                        position,
                        self.source_file.source_file_index,
                        length,
                    ));
                }
                if !self.last_was_newline {
                    self.last_was_newline = true;
                    if terminal_splice.is_none() {
                        self.missing_final_newline(context);
                    }
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
                    self.terminal_splice_reported = false;
                    return Some('\n');
                },
                | HandleNewline::EscapedNewline => {
                    continue;
                },
                | HandleNewline::Other => (),
            }

            break Some(match (curr, next, next_next) {
                | ('?', Some('?'), Some(to_map)) => {
                    let c = self.handle_trigraph_graph(context, to_map, next_index);
                    if c == '\\' && self.splice_after_trigraph() {
                        continue;
                    }
                    self.last_was_newline = false;
                    self.terminal_splice_reported = false;
                    c
                },
                | (c, _, _) => {
                    self.source_file.index = next_index;
                    self.source_file.column += 1;
                    self.last_was_newline = false;
                    self.terminal_splice_reported = false;
                    c
                },
            });
        }
    }
}
