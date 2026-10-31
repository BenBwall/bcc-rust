//! The preprocessed translation unit as C source text, as `-E` writes it:
//! each phase-4 token with its spelling, on the line where the reader was in
//! its source file, with GCC's line markers unless `-P` drops them. The
//! command line streams it (`cli/preprocessed.rs`); the explorer keeps it.
//!
//! A token that does not touch the previous one in its source gets a space
//! before it, so no two tokens paste together. Adjacent string literals are
//! written apart, as phase 4 leaves them; both callers keep phase 6 from
//! joining them. A token goes on the line where the reader was in its
//! source file, so the text a macro expands to stays on its invocation's
//! line. Line markers (`# LINE "FILE" FLAGS`) name the physical file; flag 3
//! marks a system header.
//!
//! C99: translation phases 4-6, §5.1.1.2 paragraph 1 items 4-6, p. 10;
//! PDF p. 22; adjacent string literals, §6.4.5 paragraph 4, p. 62; PDF p. 74.
//! The text form is the GCC and Clang `-E` convention.

use std::fmt::Write as _;

use crate::{
    translation_phases::{
        Context,
        preprocessing::{
            ReadAt,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// The pragmas bcc keeps for later phases, which the text writes back where
/// they were: `#pragma pack` with its replaced operands and `#pragma weak`
/// with its name. Other pragmas affect only preprocessing, or nothing in
/// bcc, and are not written.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171.
#[derive(Default)]
pub(crate) struct Pragmas {
    pack: usize,
    weak: usize,
}

impl Pragmas {
    /// Writes the pragmas that came before output token `ordinal`.
    pub(crate) fn write_before(
        &mut self,
        context: &Context<'_>,
        ordinal: usize,
        writer: &mut TextWriter<'_>,
    ) {
        loop {
            let pack = context
                .pack_pragmas
                .get(self.pack)
                .filter(|p| p.position <= ordinal);
            let weak = context
                .weak_pragmas
                .get(self.weak)
                .filter(|w| w.0 <= ordinal);
            match (pack, weak) {
                | (Some(pack), weak) if weak.is_none_or(|weak| pack.position <= weak.0) => {
                    writer.start_directive();
                    writer.text.push_str("#pragma pack");
                    for operand in pack.operands {
                        writer.text.push(' ');
                        writer.text.push_str(
                            context
                                .string_cache
                                .at(operand.spelling)
                                .trim_end_matches('\0'),
                        );
                    }
                    writer.text.push('\n');
                    self.pack += 1;
                },
                | (_, Some(&(_, name))) => {
                    writer.start_directive();
                    writer.text.push_str("#pragma weak ");
                    writer
                        .text
                        .push_str(context.string_cache.at(name).trim_end_matches('\0'));
                    writer.text.push('\n');
                    self.weak += 1;
                },
                | _ => return,
            }
        }
    }
}

/// What `-E` writes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextOptions {
    /// `-P`: no line markers, and no blank lines kept between tokens.
    pub(crate) no_line_markers: bool,
    /// `-dM`: the macro definitions in effect at the end instead of the
    /// text.
    pub(crate) macros:          bool,
    /// Spell each token as its preprocessing token was spelled, as phase 4
    /// leaves it, rather than as the token phases 5 and 6 made of it.
    pub(crate) as_written:      bool,
}

/// More blank lines than this are replaced by a line marker, as GCC does.
const MAX_BLANK_LINES: u32 = 8;

/// The preprocessed text as it is written, and where it has got to.
pub(crate) struct TextWriter<'a> {
    pub(crate) text: ArenaString<'a>,
    scratch:         Bump,
    options:         TextOptions,
    /// The source file and line the output is on, once a token was written.
    line:            Option<(u32, u32)>,
    /// Where the last token ended in its own source: its file and byte
    /// offset, and its line.
    end:             Option<(u32, usize)>,
    end_line:        u32,
    /// The first and last bytes of the last token's spelling.
    last:            (Option<u8>, Option<u8>),
    /// The main source file, whose line marker starts the text.
    main_file:       u32,
    /// A directive ended the line: the next token starts one with a line
    /// marker.
    resync:          bool,
}

/// What separated two tokens in the source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gap {
    /// Nothing: the second began where the first ended.
    Touching,
    /// Whitespace or a comment on the same line.
    Space,
    /// They come from different places, such as a macro's definition and
    /// its argument.
    Unknown,
}

/// Whether writing `next` right after a token whose spelling began and ended
/// with `last` could read as other tokens, as GCC's `cpp_avoid_paste`
/// decides: two identifier or number characters, a number and what would
/// continue it, a prefix and a quote, and the punctuator pairs that form a
/// longer punctuator or a comment.
/// C99: preprocessing tokens are the longest sequence that can form one,
/// §6.4 paragraph 4, p. 49; PDF p. 61; pp-numbers §6.4.8, p. 65; PDF p. 77.
pub(crate) fn would_paste(last: (Option<u8>, Option<u8>), next: Option<u8>) -> bool {
    let (Some(first), Some(end), Some(start)) = (last.0, last.1, next) else {
        return false;
    };
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80;
    let number = first.is_ascii_digit() || (first == b'.' && end != b'.' && end.is_ascii_digit());
    if word(end) && (word(start) || start == b'"' || start == b'\'') {
        return true;
    }
    if number
        && (start == b'.'
            || word(start)
            || (matches!(start, b'+' | b'-') && matches!(end, b'e' | b'E' | b'p' | b'P')))
    {
        return true;
    }
    match (end, start) {
        | (b'.', b'.' | b'0'..=b'9')
        | (b'+', b'+' | b'=')
        | (b'-', b'-' | b'=' | b'>')
        | (b'<', b'<' | b'=' | b':' | b'%')
        | (b'>', b'>' | b'=')
        | (b'&', b'&' | b'=')
        | (b'|', b'|' | b'=')
        | (b'=' | b'!' | b'*' | b'^', b'=')
        | (b'/', b'=' | b'*' | b'/')
        | (b'%', b'=' | b'>' | b':')
        | (b'#', b'#' | b'%')
        | (b':', b'>') => true,
        | _ => false,
    }
}

impl<'a> TextWriter<'a> {
    pub(crate) fn new(arena: &'a Bump, options: TextOptions, main_file: u32) -> Self {
        Self {
            text: ArenaString::new_in(arena),
            scratch: Bump::new(),
            options,
            line: None,
            end: None,
            end_line: 0,
            last: (None, None),
            main_file,
            resync: false,
        }
    }

    /// Writes `token`, starting a new line or a line marker first when it
    /// belongs to a later line or another file. Returns the place it was
    /// written for: a source file index, which `#line` may have renamed,
    /// and a byte offset there.
    pub(crate) fn token(
        &mut self,
        context: &Context<'_>,
        token: Token,
        at: Option<ReadAt>,
    ) -> Option<(u32, usize)> {
        let spelled = context.get_source_vectors(token.source_vectors).first();
        // Where the token belongs: where the outermost invocation is for a
        // token a macro produced, even when the macro is defined in the same
        // file, otherwise its own place if the reader's file spells it, and
        // failing both the line the reader is on.
        let (file, line, column, offset) = match (spelled, at) {
            | (
                _,
                Some(ReadAt {
                    file,
                    expansion: Some(invocation),
                    ..
                }),
            ) if invocation.source_file_index == file => (
                file,
                invocation.line,
                invocation.column,
                Some(invocation.index as usize),
            ),
            | (
                Some(vector),
                Some(ReadAt {
                    file,
                    expansion: None,
                    ..
                }),
            ) if vector.source_file_index == file => (
                file,
                vector.line,
                vector.column,
                Some(vector.index as usize),
            ),
            | (_, Some(ReadAt { file, position, .. })) =>
                (file, position.line, 0, Some(position.index)),
            | (Some(vector), None) => (
                vector.source_file_index,
                vector.line,
                vector.column,
                Some(vector.index as usize),
            ),
            | (None, None) => (0, 0, 0, None),
        };
        let gap = match (self.end, spelled) {
            // Tokens from one place in the source keep its spacing.
            | (Some((end_file, end)), Some(vector)) if end_file == vector.source_file_index => {
                let start = vector.index as usize;
                if start == end {
                    Gap::Touching
                } else if start > end && vector.line == self.end_line {
                    Gap::Space
                } else {
                    Gap::Unknown
                }
            },
            | _ => Gap::Unknown,
        };
        // Like GCC's PREV_WHITE: whitespace before the token where it is
        // spelled, such as in a macro's replacement list.
        let white_before = spelled.is_some_and(|vector| {
            context
                .source_text(vector.source_file_index)
                .and_then(|text| text.as_bytes().get((vector.index as usize).checked_sub(1)?))
                .is_some_and(u8::is_ascii_whitespace)
        });
        let next = first_byte(context, token);
        if self.resync {
            self.resync = false;
            if !self.options.no_line_markers {
                self.marker(context, file, line);
            }
            self.line = Some((file, line));
            self.indent(column);
        } else {
            match self.line {
                | Some((current_file, current_line)) if current_file == file =>
                    if line > current_line {
                        if self.options.no_line_markers || line - current_line > MAX_BLANK_LINES {
                            self.text.push('\n');
                            if !self.options.no_line_markers {
                                self.marker(context, file, line);
                            }
                        } else {
                            for _ in current_line..line {
                                self.text.push('\n');
                            }
                        }
                        self.line = Some((file, line));
                        self.indent(column);
                    } else if gap == Gap::Space
                        || (gap == Gap::Unknown && (white_before || would_paste(self.last, next)))
                    {
                        self.text.push(' ');
                    },
                | previous => {
                    if !self.options.no_line_markers {
                        match previous {
                            | Some(_) => self.text.push('\n'),
                            | None => self.marker(context, self.main_file, 1),
                        }
                        if file != self.main_file || line > 1 {
                            self.marker(context, file, line);
                        }
                    } else if previous.is_some() {
                        self.text.push('\n');
                    }
                    self.line = Some((file, line));
                    self.indent(column);
                },
            }
        }
        self.scratch.reset();
        let spelling = if self.options.as_written {
            context
                .string_cache
                .at(token.contents)
                .trim_end_matches('\0')
        } else {
            spelling(context, &self.scratch, token)
        };
        self.text.push_str(spelling);
        self.last = (
            spelling.as_bytes().first().copied(),
            spelling.as_bytes().last().copied(),
        );
        self.end = spelled.map(|vector| (vector.source_file_index, vector.end()));
        self.end_line = spelled.map_or(0, |vector| vector.line);
        offset.map(|offset| (file, offset))
    }

    /// Writes `# LINE "FILE"` and a new line, with flag 3 for a system
    /// header.
    fn marker(&mut self, context: &Context<'_>, file: u32, line: u32) {
        let path = context.get_source_file(file);
        _ = write!(self.text, "# {line} ");
        _ = crate::diagnostics::write_c_quoted(&mut self.text, "", '"', &path.to_string_lossy());
        if context.has_system_header_part(file) {
            self.text.push_str(" 3");
        }
        self.text.push('\n');
    }

    /// Indents the first token of a line to its source column, as GCC does.
    fn indent(&mut self, column: u32) {
        for _ in 1..column {
            self.text.push(' ');
        }
    }

    /// Ends the current line for a directive; the next token starts a line
    /// with a line marker.
    fn start_directive(&mut self) {
        if let Some((file, _)) = self.line {
            self.text.push('\n');
            self.line = Some((file, u32::MAX));
            self.resync = true;
        }
    }

    /// Ends the last line.
    pub(crate) fn finish(&mut self) {
        if self.line.is_some() {
            self.text.push('\n');
        }
    }
}

/// The first byte of the token's spelling.
fn first_byte(context: &Context<'_>, token: Token) -> Option<u8> {
    match token.kind {
        | TokenType::Operator(operator) => operator.spelling().bytes().next(),
        | TokenType::String(StringTokenType::EncodedString(_, encoding)) =>
            encoding.prefix().bytes().next().or(Some(b'"')),
        | TokenType::String(StringTokenType::WideString(_)) => Some(b'L'),
        | TokenType::String(StringTokenType::String(_)) => Some(b'"'),
        | _ => context.string_cache.at(token.contents).bytes().next(),
    }
}

/// The token's spelling, in `scratch` for a string literal.
fn spelling<'s>(context: &'s Context<'_>, scratch: &'s Bump, token: Token) -> &'s str {
    match token.kind {
        | TokenType::Operator(operator) => operator.spelling(),
        | TokenType::String(StringTokenType::EncodedString(contents, encoding)) =>
            context.literal_spelling_in(scratch, scratch, contents, encoding.prefix()),
        | TokenType::String(StringTokenType::String(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, ""),
        | TokenType::String(StringTokenType::WideString(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, "L"),
        | _ => context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0'),
    }
}
