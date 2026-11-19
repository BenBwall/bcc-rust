//! The preprocessed translation unit as C source text, as `-E` writes it:
//! each phase-4 token with its spelling, on the line where the reader was in
//! its source file, with GCC's line markers unless `-P` drops them. The
//! command line streams it (`cli/preprocessed.rs`); the explorer keeps it.
//!
//! A token gets a space before it where white space came before it where it
//! was written, or, around a macro's expansion and its arguments, where the
//! preprocessor's padding says so or the two tokens could paste together, as
//! GCC prints them (`c-ppoutput.cc`, `scan_translation_unit`; see
//! `preprocessor_tokenizer/spacing.rs`). Adjacent string literals are
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
    configuration::CompilerIdentity,
    translation_phases::{
        Context,
        TextEventKind,
        preprocessing::{
            ReadAt,
            StringTokenType,
            Token,
            TokenType,
        },
        preprocessor_tokenizer::OutputSpacing,
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// What a text of phase 4 writes between its tokens, as GCC and Clang write
/// it. Each `#pragma` line, and each `_Pragma` operator's, stands on a line of
/// its own, spelled as written, unless the preprocessor alone carries it out.
/// Each file entered or returned to gets a line marker, even when no token
/// lies between two of them.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171; §6.10.9 paragraph 1,
/// p. 161; PDF p. 173.
#[derive(Default)]
pub(crate) struct TextEvents {
    next: usize,
}

impl TextEvents {
    /// Writes what came before output token `ordinal`.
    pub(crate) fn write_before(
        &mut self,
        context: &Context<'_>,
        ordinal: usize,
        writer: &mut TextWriter<'_>,
    ) {
        while let Some(&event) = context.text_events.get(self.next)
            && event.position <= ordinal
        {
            self.next += 1;
            if writer.stopped {
                continue;
            }
            match event.kind {
                | TextEventKind::Pragma(text) =>
                    writer.pragma(context, event.file, event.line, text),
                | TextEventKind::FileEntered if writer.cl => writer.cl_enter(context, event.file),
                | TextEventKind::FileEntered =>
                    writer.file_change(context, event.file, event.line, Flag::Entered),
                | TextEventKind::FileReturned if writer.cl => {
                    writer.cl_end_file(context);
                    writer.cl_marker(context, event.file, event.line);
                },
                | TextEventKind::FileReturned =>
                    writer.file_change(context, event.file, event.line, Flag::Returned),
                | TextEventKind::IncludeLine if writer.cl =>
                    writer.cl_reach(context, event.file, event.line),
                | TextEventKind::ConditionalEnded if writer.cl => {
                    writer.cl_reach(context, event.file, event.line);
                    writer.cl_marker(context, event.file, event.line + 1);
                },
                | TextEventKind::Stopped if writer.cl => writer.cl_stop(context),
                | TextEventKind::IncludeLine
                | TextEventKind::ConditionalEnded
                | TextEventKind::Stopped => {},
            }
        }
    }
}

/// What `-E` writes.
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each is a separate command-line choice."
)]
pub(crate) struct TextOptions {
    /// `-P`: no line markers, and no blank lines kept between tokens.
    pub(crate) no_line_markers: bool,
    /// `-dM`: the macro definitions in effect at the end instead of the
    /// text.
    pub(crate) macros:          bool,
    /// Spell each token as its preprocessing token was spelled, as phase 4
    /// leaves it, rather than as the token phases 5 and 6 made of it.
    pub(crate) as_written:      bool,
    /// `cl`'s layout: a line for each source line, and `#line` markers, or
    /// empty lines in their place under `-P` (`cl /EP`).
    pub(crate) cl_layout:       bool,
}

/// More blank lines than this are replaced by a line marker, as GCC does.
const MAX_BLANK_LINES: u32 = 8;

/// The preprocessed text as it is written, and where it has got to.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact about the line being written."
)]
pub(crate) struct TextWriter<'a> {
    pub(crate) text: ArenaString<'a>,
    scratch:         Bump,
    options:         TextOptions,
    /// The source file and line the output is on, once a token was written.
    line:            Option<(u32, u32)>,
    /// The class of the last token on the line.
    last:            PasteClass,
    /// The main source file, whose line marker starts the text.
    main_file:       u32,
    /// The current line holds a pragma: a token for it or an earlier line
    /// starts one of its own, with a line marker.
    after_pragma:    bool,
    /// A file change's marker was just written, so the next token or marker
    /// starts its line with no new line first, whatever line it is on.
    at_line_start:   bool,
    /// `cl`'s layout ([`Self::cl_reach`]), under the `cl` identity.
    cl:              bool,
    /// In `cl`'s layout, nothing is written on the current line yet.
    fresh:           bool,
    /// `cl` stopped its text at a fatal error.
    stopped:         bool,
}

/// The flag on a line marker for a change of file.
#[derive(Clone, Copy)]
enum Flag {
    Entered,
    Returned,
}

/// A token's spelling as far as it decides what it could paste with: GCC's
/// `cpp_avoid_paste` looks at the token's type and, of an operator, at its
/// first character.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum PasteClass {
    /// Nothing comes before the token on its line.
    #[default]
    None,
    /// An identifier.
    Name,
    /// A pp-number, and whether all of it could continue an identifier.
    Number(bool),
    /// An unprefixed character constant.
    Character,
    /// An unprefixed string literal.
    String,
    /// A prefixed character constant or string literal. GCC's
    /// `cpp_avoid_paste` takes no account of them (`CPP_WSTRING`,
    /// `CPP_WCHAR`), and so changes `ID(return)L"x"`'s tokens; bcc does.
    Prefixed,
    /// The other tokens: a punctuator, as it is written, with its digraph
    /// `%:` written `#`; the three-character ones paste with nothing.
    Punctuator(u8, u8),
    /// A lone `\`, which would begin a universal character name.
    Backslash,
}

impl PasteClass {
    /// The class of the token spelled `spelling`.
    pub(crate) fn of(spelling: &str) -> Self {
        let bytes = spelling.as_bytes();
        match bytes {
            | [] => Self::None,
            | [b'\\'] => Self::Backslash,
            | [b'"', ..] => Self::String,
            | [b'\'', ..] => Self::Character,
            | [b'0'..=b'9', ..] | [b'.', b'0'..=b'9', ..] =>
                Self::Number(bytes.iter().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | 0x80..)
                })),
            | [first, .., b'"' | b'\''] if first.is_ascii_alphabetic() => Self::Prefixed,
            | [first, ..]
                if first.is_ascii_alphabetic() || matches!(first, b'_' | b'$' | b'\\' | 0x80..) =>
                Self::Name,
            | [b'%', b':'] => Self::Punctuator(b'#', 0),
            | [first] => Self::Punctuator(*first, 0),
            | [first, second] => Self::Punctuator(*first, *second),
            | [first, ..] => Self::Punctuator(*first, 1),
        }
    }

    /// The first character of the operator this is, by GCC's `c`.
    const fn operator(self, spelling: u8) -> Option<u8> {
        match self {
            | Self::Punctuator(..) => Some(spelling),
            | _ => None,
        }
    }
}

/// Whether a token of class `next`, spelled from `first`, written right after
/// one of class `last`, could read as other tokens, as GCC's `cpp_avoid_paste`
/// (libcpp `lex.cc`) decides. It is conservative on purpose: a number followed
/// by `+`, `-` or `.` is kept apart even without an exponent, and `.` from a
/// number, since `.2` could be read with it. Unlike GCC it also keeps a name or
/// number from a prefixed literal after it, since `return L"x"` would otherwise
/// read as the name `returnL` and a string, as Clang's printer knows.
/// C99: preprocessing tokens are the longest sequence that can form one,
/// §6.4 paragraph 4, p. 49; PDF p. 61; pp-numbers §6.4.8, p. 65; PDF p. 77.
pub(crate) fn avoid_paste(last: PasteClass, next: PasteClass, first: u8) -> bool {
    let operator = next.operator(first);
    let c = |set: &[u8]| operator.is_some_and(|c| set.contains(&c));
    match last {
        | PasteClass::Name => matches!(
            next,
            PasteClass::Name
                | PasteClass::Number(true)
                | PasteClass::Character
                | PasteClass::String
                | PasteClass::Prefixed
        ),
        | PasteClass::Number(_) =>
            matches!(
                next,
                PasteClass::Number(_)
                    | PasteClass::Name
                    | PasteClass::Character
                    | PasteClass::Prefixed
            ) || c(b".+-"),
        | PasteClass::Backslash => matches!(next, PasteClass::Name | PasteClass::Prefixed),
        | PasteClass::Punctuator(a, 0) =>
            (matches!(
                a,
                b'=' | b'!' | b'>' | b'<' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^'
            ) && operator == Some(b'='))
                || match a {
                    | b'>' => c(b">"),
                    | b'<' => c(b"<%:"),
                    | b'+' => c(b"+"),
                    | b'-' => c(b"->"),
                    | b'/' => c(b"/*"),
                    | b'%' | b':' => c(b":>"),
                    | b'&' => c(b"&"),
                    | b'|' => c(b"|"),
                    | b'.' => c(b".%") || matches!(next, PasteClass::Number(_)),
                    | b'#' => c(b"#%"),
                    | _ => false,
                },
        | PasteClass::Punctuator(b'>', b'>') | PasteClass::Punctuator(b'<', b'<') =>
            operator == Some(b'='),
        | PasteClass::Punctuator(b'-', b'>') => c(b"*"),
        | PasteClass::Punctuator(b'<', b'=') => c(b">"),
        | PasteClass::None
        | PasteClass::Character
        | PasteClass::String
        | PasteClass::Prefixed
        | PasteClass::Punctuator(..) => false,
    }
}

impl<'a> TextWriter<'a> {
    pub(crate) fn new(arena: &'a Bump, options: TextOptions, main_file: u32) -> Self {
        Self {
            text: ArenaString::new_in(arena),
            scratch: Bump::new(),
            options,
            line: None,
            last: PasteClass::None,
            main_file,
            after_pragma: false,
            at_line_start: false,
            cl: options.cl_layout,
            fresh: false,
            stopped: false,
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
        if self.stopped {
            return None;
        }
        let spelled = context.get_source_vectors(token.source_vectors).first();
        // A token without a place to read it from is one a macro made.
        let spacing = at.as_ref().map_or(
            OutputSpacing {
                space:       false,
                avoid_paste: true,
            },
            |at| at.spacing,
        );
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
        let classified = paste_spelling(context, token, self.options.as_written);
        let class = PasteClass::of(classified);
        let first = classified.as_bytes().first().copied().unwrap_or(0);
        let later = matches!(self.line, Some((current_file, current_line))
            if current_file == file && line > current_line);
        if self.cl {
            if self.fresh || later || self.line.is_none_or(|(current, _)| current != file) {
                self.cl_reach(context, file, line);
                self.indent(column);
                self.begin_line(column, spacing, class);
            } else if spacing.space || (spacing.avoid_paste && avoid_paste(self.last, class, first))
            {
                self.text.push(' ');
            }
            self.fresh = false;
        } else if self.at_line_start {
            self.advance(context, file, line);
            self.at_line_start = false;
            self.indent(column);
            self.begin_line(column, spacing, class);
        } else if std::mem::take(&mut self.after_pragma)
            && !later
            && self.line.is_some_and(|(f, _)| f == file)
        {
            self.text.push('\n');
            if !self.options.no_line_markers {
                self.marker(context, file, line, None);
            }
            self.line = Some((file, line));
            self.indent(column);
            self.begin_line(column, spacing, class);
        } else if self
            .line
            .is_some_and(|(current_file, _)| current_file == file)
            && !later
        {
            if spacing.space || (spacing.avoid_paste && avoid_paste(self.last, class, first)) {
                self.text.push(' ');
            }
        } else {
            self.advance(context, file, line);
            self.indent(column);
            self.begin_line(column, spacing, class);
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
        self.last = class;
        offset.map(|offset| (file, offset))
    }

    /// Writes a pragma's line, `text`, for line `line` of `file`: on that
    /// line when the output has not reached it, otherwise after a new line
    /// and a line marker, as for a `_Pragma` operator between two tokens of
    /// a line. The next token starts a line of its own.
    pub(crate) fn pragma(&mut self, context: &Context<'_>, file: u32, line: u32, text: &str) {
        if self.cl {
            if self.stopped {
                return;
            }
            self.cl_reach(context, file, line);
            if !self.fresh {
                self.text.push(' ');
            }
            self.text.push_str(text);
            self.fresh = false;
            self.last = PasteClass::None;
            return;
        }
        match self.line {
            | Some((current_file, current_line))
                if current_file == file && line <= current_line && !self.at_line_start =>
            {
                self.text.push('\n');
                if !self.options.no_line_markers {
                    self.marker(context, file, line, None);
                }
                self.line = Some((file, line));
            },
            | _ => self.advance(context, file, line),
        }
        self.at_line_start = false;
        self.text.push_str(text);
        self.after_pragma = true;
        self.last = PasteClass::None;
    }

    /// Starts output line `line` of `file`, which is later in the same file
    /// than the current line, or in another: with new lines up to it, or a
    /// line marker after too many or for another file, or with the text's
    /// first line marker.
    fn advance(&mut self, context: &Context<'_>, file: u32, line: u32) {
        match self.line {
            | Some((current_file, current_line)) if current_file == file => {
                if self.options.no_line_markers
                    || line.saturating_sub(current_line) > MAX_BLANK_LINES
                {
                    self.end_line();
                    if !self.options.no_line_markers {
                        self.marker(context, file, line, None);
                    }
                } else {
                    for _ in current_line..line {
                        self.text.push('\n');
                    }
                }
            },
            | previous =>
                if !self.options.no_line_markers {
                    match previous {
                        | Some(_) => self.end_line(),
                        | None => self.marker(context, self.main_file, 1, None),
                    }
                    if file != self.main_file || line > 1 {
                        self.marker(context, file, line, None);
                    }
                } else if previous.is_some() {
                    self.end_line();
                },
        }
        self.line = Some((file, line));
    }

    /// Records that the reader changed file: a line marker for it at `line`,
    /// on a line of its own, and without markers just the end of the line, so
    /// that the tokens of two files never join.
    fn file_change(&mut self, context: &Context<'_>, file: u32, line: u32, flag: Flag) {
        if self.line.is_some() {
            self.end_line();
        } else if !self.options.no_line_markers {
            self.marker(context, self.main_file, 1, None);
        }
        if !self.options.no_line_markers {
            self.marker(context, file, line, Some(flag));
        }
        self.line = Some((file, line));
        self.at_line_start = true;
        self.after_pragma = false;
        self.last = PasteClass::None;
    }

    /// Ends the current line, unless a marker just did.
    fn end_line(&mut self) {
        if !self.at_line_start {
            self.text.push('\n');
        }
    }

    /// Writes `# LINE "FILE"` and a new line, with GCC's flags as each
    /// identity's compiler writes them: 1 entering a file, 2 returning to one,
    /// 3 in a system header, and from GCC 4 there too, on the targets where a
    /// system header is implicitly `extern "C"`. `cl` writes no flags, so
    /// `flag` stays off there.
    fn marker(&mut self, context: &Context<'_>, file: u32, line: u32, flag: Option<Flag>) {
        let path = context.get_source_file(file);
        _ = write!(self.text, "# {line} ");
        _ = crate::diagnostics::write_c_quoted(&mut self.text, "", '"', &path.to_string_lossy());
        let identity = context.configuration.compiler_identity();
        match flag {
            | _ if identity == CompilerIdentity::Msvc => (),
            | Some(Flag::Entered) => self.text.push_str(" 1"),
            | Some(Flag::Returned) => self.text.push_str(" 2"),
            | None => (),
        }
        if context.has_system_header_part(file) {
            self.text.push_str(if identity == CompilerIdentity::Gcc {
                " 3 4"
            } else {
                " 3"
            });
        }
        self.text.push('\n');
    }

    /// Separates the first token of a line, at `column`, from the start of
    /// the line when a macro made it: white space may stand before it where
    /// its column does not show that, and a `#` must not read as beginning a
    /// directive.
    fn begin_line(&mut self, column: u32, spacing: OutputSpacing, class: PasteClass) {
        if column <= 1
            && (spacing.space || (spacing.avoid_paste && class == PasteClass::Punctuator(b'#', 0)))
        {
            self.text.push(' ');
        }
    }

    /// Indents the first token of a line to its source column, as GCC does.
    fn indent(&mut self, column: u32) {
        for _ in 1..column {
            self.text.push(' ');
        }
    }

    /// Ends the last line.
    pub(crate) fn finish(&mut self, context: &Context<'_>) {
        if self.cl {
            if self.line.is_none() {
                self.cl_marker(context, self.main_file, 1);
            }
            if !self.stopped {
                self.cl_end_file(context);
            }
        } else if self.line.is_some() {
            self.end_line();
        }
    }

    /// Moves `cl`'s layout to the start of `line` of `file`, with an empty
    /// line for each source line passed: `cl /E` writes a line for each
    /// line of a source file, directives and blank lines included, and
    /// starts the text with the main file's `#line` marker. Another file
    /// gets a marker of its own.
    fn cl_reach(&mut self, context: &Context<'_>, file: u32, line: u32) {
        match self.line {
            | Some((current_file, current)) if current_file == file =>
                if line > current {
                    for _ in current..line {
                        self.text.push('\n');
                    }
                    self.line = Some((file, line));
                    self.fresh = true;
                },
            | None => {
                self.cl_marker(context, self.main_file, 1);
                self.cl_reach(context, file, line);
            },
            | Some(_) => self.cl_marker(context, file, line),
        }
    }

    /// Writes the marker of `file`, which the reader entered, in `cl`'s
    /// layout: on the `#include` line [`Self::cl_reach`] reached, or after
    /// the main file's marker for a file the command line forces.
    fn cl_enter(&mut self, context: &Context<'_>, file: u32) {
        if self.line.is_none() {
            self.cl_marker(context, self.main_file, 1);
        }
        self.cl_marker(context, file, 1);
    }

    /// Stops `cl`'s text at a fatal error, as `cl` does, after the main
    /// file's marker and the line in progress.
    fn cl_stop(&mut self, context: &Context<'_>) {
        if self.line.is_none() {
            self.cl_marker(context, self.main_file, 1);
        } else if !self.fresh {
            self.text.push('\n');
        }
        self.stopped = true;
    }

    /// Writes the rest of the file being read in `cl`'s layout: an empty
    /// line for each source line not reached, to its end.
    fn cl_end_file(&mut self, context: &Context<'_>) {
        let Some((file, _)) = self.line else {
            return;
        };
        let text = context.source_text(file).unwrap_or_default();
        let lines = text.bytes().filter(|&byte| byte == b'\n').count()
            + usize::from(!text.is_empty() && !text.ends_with('\n'));
        self.cl_reach(
            context,
            file,
            u32::try_from(lines).unwrap_or(u32::MAX).saturating_add(1),
        );
    }

    /// Writes `cl`'s marker for `line` of `file`, `#line LINE "FILE"`, on a
    /// line of its own, or an empty line under `-P`. `cl` names an included
    /// file by its absolute path, from the working directory when relative,
    /// and the main file as the command line does.
    fn cl_marker(&mut self, context: &Context<'_>, file: u32, line: u32) {
        if self.line.is_some() && !self.fresh {
            self.text.push('\n');
        }
        if !self.options.no_line_markers {
            let path = context.get_source_file(file);
            let mut name = ArenaString::new_in(&self.scratch);
            if file != self.main_file
                && path.is_relative()
                && let Ok(directory) = std::env::current_dir()
            {
                _ = write!(name, "{}{}", directory.display(), std::path::MAIN_SEPARATOR);
            }
            _ = write!(name, "{}", path.display());
            _ = write!(self.text, "#line {line} ");
            _ = crate::diagnostics::write_c_quoted(&mut self.text, "", '"', &name);
        }
        self.text.push('\n');
        self.scratch.reset();
        self.line = Some((file, line));
        self.fresh = true;
        self.last = PasteClass::None;
    }
}

/// The token's spelling as far as [`PasteClass::of`] looks at it, so a string
/// literal needs no spelling out.
fn paste_spelling<'s>(context: &'s Context<'_>, token: Token, as_written: bool) -> &'s str {
    match token.kind {
        | _ if as_written => context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0'),
        | TokenType::Operator(operator) => operator.spelling(),
        | TokenType::String(StringTokenType::String(_)) => "\"\"",
        | TokenType::String(_) => "L\"\"",
        | _ => context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0'),
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
