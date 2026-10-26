//! Prints the preprocessed translation unit as C source text (`-E`), with
//! GCC's line markers unless `-P` drops them, or the macro definitions in
//! effect at its end (`-dM`).
//!
//! Each token is written with its spelling; a token that does not touch the
//! previous one in its source gets a space before it, so no two tokens paste
//! together. Strings are written as phase 6 joined them. A token goes on the
//! line where the reader was in its source file, so the text a macro expands
//! to stays on its invocation's line. Line markers (`# LINE "FILE" FLAGS`)
//! name the physical file; flag 3 marks a system header.
//!
//! C99: translation phases 4-6, §5.1.1.2 paragraph 1 items 4-6, p. 10;
//! PDF p. 22; adjacent string literals, §6.4.5 paragraph 4, p. 62; PDF p. 74.
//! The text form is the GCC and Clang `-E` convention.

use std::{
    fmt::Write as _,
    io::{
        self,
        Write,
    },
    path::Path,
};

use super::diagnostic_reporter::DiagnosticReporter;
use crate::{
    diagnostics::ColorChoice as RenderColor,
    headers::HeaderSearch,
    pipeline::with_preprocessor,
    translation_phases::{
        Context,
        SourcePosition,
        TranslationError,
        preprocessing::{
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        bump::{
            ArenaString,
            Bump,
        },
        region_vec::RegionVec,
    },
};

/// Preprocesses the input and writes its text to `out`. Diagnostics go to
/// stderr. Returns whether an error was reported.
pub(super) fn print_preprocessed<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    options: TextOptions,
    out: &mut dyn Write,
) -> io::Result<bool> {
    let tu = context.tu_arena();
    let (items, macros) = with_preprocessor(
        context,
        source_filename,
        input_string,
        search,
        |mut preprocessor, context, _pp| {
            let mut tokens = RegionVec::new();
            preprocessor.for_each_iterator_item_read_at(context, |context, mut token, at| {
                token.source_vectors = context.retain_token_source(token.source_vectors);
                tokens.push((token, at, context.pending_error_count()));
            });
            let macros = options
                .macros
                .then(|| preprocessor.macro_listing_in(context, tu));
            (tokens, macros)
        },
    );
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(&reporter_arena, tu, RenderColor::for_stderr());
    let stderr = &mut io::stderr();
    let text_arena = Bump::new();
    let main_file = context.intern_source_file(source_filename);
    let mut writer = TextWriter::new(&text_arena, options, main_file);
    let mut diagnosed = 0;
    let mut pragmas = Pragmas::default();
    for (ordinal, (token, at, errors_before)) in items.into_iter().enumerate() {
        while diagnosed < errors_before {
            report_next(&mut reporter, context);
            diagnosed += 1;
        }
        if macros.is_none() {
            pragmas.write_before(context, ordinal, &mut writer);
            writer.token(context, token, at);
            if writer.text.len() >= FLUSH_BYTES {
                out.write_all(writer.text.as_bytes())?;
                writer.text.clear();
            }
        }
    }
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, context);
    }
    if let Some(macros) = macros {
        out.write_all(macros.as_bytes())?;
    } else {
        pragmas.write_before(context, usize::MAX, &mut writer);
        writer.finish();
        out.write_all(writer.text.as_bytes())?;
    }
    out.flush()?;
    drop(reporter.finish(context, stderr));
    Ok(reporter.errors != 0)
}

/// Reports the next pending diagnostic.
fn report_next(reporter: &mut DiagnosticReporter<'_, '_>, context: &mut Context<'_>) {
    let error: TranslationError<'_> = context
        .pop_pending_error()
        .expect("counted diagnostics are pending");
    reporter.report(&error, context);
}

/// The pragmas bcc keeps for later phases, which the text writes back where
/// they were: `#pragma pack` with its replaced operands and `#pragma weak`
/// with its name. Other pragmas affect only preprocessing, or nothing in
/// bcc, and are not written.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171.
#[derive(Default)]
struct Pragmas {
    pack: usize,
    weak: usize,
}

impl Pragmas {
    /// Writes the pragmas that came before output token `ordinal`.
    fn write_before(&mut self, context: &Context<'_>, ordinal: usize, writer: &mut TextWriter<'_>) {
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
pub(super) struct TextOptions {
    /// `-P`: no line markers, and no blank lines kept between tokens.
    pub(super) no_line_markers: bool,
    /// `-dM`: the macro definitions in effect at the end instead of the
    /// text.
    pub(super) macros:          bool,
}

/// Text accumulated before it is written out.
const FLUSH_BYTES: usize = 1 << 16;

/// More blank lines than this are replaced by a line marker, as GCC does.
const MAX_BLANK_LINES: u32 = 8;

/// The preprocessed text as it is written, and where it has got to.
struct TextWriter<'a> {
    text:      ArenaString<'a>,
    scratch:   Bump,
    options:   TextOptions,
    /// The source file and line the output is on, once a token was written.
    line:      Option<(u32, u32)>,
    /// Where the last token ended in its own source: its file and byte
    /// offset, and its line.
    end:       Option<(u32, usize)>,
    end_line:  u32,
    /// The first and last bytes of the last token's spelling.
    last:      (Option<u8>, Option<u8>),
    /// The main source file, whose line marker starts the text.
    main_file: u32,
    /// A directive ended the line: the next token starts one with a line
    /// marker.
    resync:    bool,
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
fn would_paste(last: (Option<u8>, Option<u8>), next: Option<u8>) -> bool {
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
    fn new(arena: &'a Bump, options: TextOptions, main_file: u32) -> Self {
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
    /// belongs to a later line or another file.
    fn token(&mut self, context: &Context<'_>, token: Token, at: Option<(u32, SourcePosition)>) {
        let spelled = context.get_source_vectors(token.source_vectors).first();
        // Where the token belongs: its own place if the reader's file spells
        // it, otherwise the line the reader is on.
        let (file, line, column) = match (spelled, at) {
            | (Some(vector), Some((file, _))) if vector.source_file_index == file =>
                (file, vector.line, vector.column),
            | (_, Some((file, position))) => (file, position.line, 0),
            | (Some(vector), None) => (vector.source_file_index, vector.line, vector.column),
            | (None, None) => (0, 0, 0),
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
        let spelling = spelling(context, &self.scratch, token);
        self.text.push_str(spelling);
        self.last = (
            spelling.as_bytes().first().copied(),
            spelling.as_bytes().last().copied(),
        );
        self.end = spelled.map(|vector| (vector.source_file_index, vector.end()));
        self.end_line = spelled.map_or(0, |vector| vector.line);
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
    fn finish(&mut self) {
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
