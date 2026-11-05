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
    translation_phases::{
        Context,
        preprocessor_tokenizer::OutputSpacing,
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

/// The pragmas a text of phase 4 writes back where they were, as GCC and
/// Clang write them: each `#pragma` line, and each `_Pragma` operator's, on a
/// line of its own, spelled as written, unless the preprocessor alone
/// carries it out.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171; §6.10.9 paragraph 1,
/// p. 161; PDF p. 173.
#[derive(Default)]
pub(crate) struct Pragmas {
    next: usize,
}

impl Pragmas {
    /// Writes the pragmas that came before output token `ordinal`.
    pub(crate) fn write_before(
        &mut self,
        context: &Context<'_>,
        ordinal: usize,
        writer: &mut TextWriter<'_>,
    ) {
        while let Some(pragma) = context.text_pragmas.get(self.next)
            && pragma.position <= ordinal
        {
            writer.pragma(context, pragma.file, pragma.line, pragma.text);
            self.next += 1;
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
    /// The class of the last token on the line.
    last:            PasteClass,
    /// The main source file, whose line marker starts the text.
    main_file:       u32,
    /// The current line holds a pragma: a token for it or an earlier line
    /// starts one of its own, with a line marker.
    after_pragma:    bool,
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
    /// A prefixed character constant or string literal, which pastes with
    /// nothing, as a GCC `CPP_WSTRING` does not.
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
            | [b'0'..=b'9', ..] | [b'.', b'0'..=b'9', ..] => Self::Number(
                bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | 0x80..)),
            ),
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
/// number, since `.2` could be read with it.
/// C99: preprocessing tokens are the longest sequence that can form one,
/// §6.4 paragraph 4, p. 49; PDF p. 61; pp-numbers §6.4.8, p. 65; PDF p. 77.
pub(crate) fn avoid_paste(last: PasteClass, next: PasteClass, first: u8) -> bool {
    let operator = next.operator(first);
    let c = |set: &[u8]| operator.is_some_and(|c| set.contains(&c));
    match last {
        | PasteClass::None | PasteClass::Character | PasteClass::String | PasteClass::Prefixed =>
            false,
        | PasteClass::Name => matches!(
            next,
            PasteClass::Name
                | PasteClass::Number(true)
                | PasteClass::Character
                | PasteClass::String
        ),
        | PasteClass::Number(_) =>
            matches!(next, PasteClass::Number(_) | PasteClass::Name | PasteClass::Character)
                || c(b".+-"),
        | PasteClass::Backslash => next == PasteClass::Name,
        | PasteClass::Punctuator(a, 0) =>
            (matches!(a, b'=' | b'!' | b'>' | b'<' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^')
                && operator == Some(b'='))
                || match a {
                    | b'>' => c(b">"),
                    | b'<' => c(b"<%:"),
                    | b'+' => c(b"+"),
                    | b'-' => c(b"->"),
                    | b'/' => c(b"/*"),
                    | b'%' => c(b":>"),
                    | b'&' => c(b"&"),
                    | b'|' => c(b"|"),
                    | b':' => c(b":>"),
                    | b'.' => c(b".%") || matches!(next, PasteClass::Number(_)),
                    | b'#' => c(b"#%"),
                    | _ => false,
                },
        | PasteClass::Punctuator(b'>', b'>') | PasteClass::Punctuator(b'<', b'<') =>
            operator == Some(b'='),
        | PasteClass::Punctuator(b'-', b'>') => c(b"*"),
        | PasteClass::Punctuator(b'<', b'=') => c(b">"),
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
        if std::mem::take(&mut self.after_pragma)
            && !later
            && self.line.is_some_and(|(f, _)| f == file)
        {
            self.text.push('\n');
            if !self.options.no_line_markers {
                self.marker(context, file, line);
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
        match self.line {
            | Some((current_file, current_line)) if current_file == file && line <= current_line =>
            {
                self.text.push('\n');
                if !self.options.no_line_markers {
                    self.marker(context, file, line);
                }
                self.line = Some((file, line));
            },
            | _ => self.advance(context, file, line),
        }
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
            },
            | previous =>
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
                },
        }
        self.line = Some((file, line));
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
    pub(crate) fn finish(&mut self) {
        if self.line.is_some() {
            self.text.push('\n');
        }
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
