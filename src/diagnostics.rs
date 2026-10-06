//! User-facing diagnostics: a small builder for messages with labelled source
//! ranges, notes, and help, plus a renderer that prints them as annotated
//! source snippets in the style of `rustc`:
//!
//! ```text
//! error: expected `;` after the declarator, found string literal `"abc"`
//!  --> example.c:1:7
//!   |
//! 1 | int x "abc";
//!   |       ^^^^^ expected `,`, `=`, `;`, or a function body
//!   |
//!   = note: C99 §6.7: a declaration ends with `;`
//! ```
//!
//! Everything printed comes from the C source and fixed wording; no internal
//! representation (arena indices, interned-string handles, Rust `Debug`
//! output) may appear in a diagnostic.
//! C99: required diagnostics for syntax and constraint violations §5.1.1.3p1,
//! p. 11; PDF p. 23. This renderer carries locations from translation phases
//! 1-7 (§5.1.1.2, pp. 9-10; PDF pp. 21-22).

use std::fmt::{
    self,
    Display,
    Write as _,
};

use owo_colors::{
    OwoColorize,
    Style,
};

use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
        SourceVector,
        SourceVectors,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// Width a tab occupies when a source line is echoed.
const TAB_WIDTH: usize = 4;

/// Longest token spelling quoted inline in a message before it is shortened.
const MAX_QUOTED_SPELLING: usize = 40;

/// Formats text into an arena, like `format!` into a `String`; the result
/// borrows the arena. Text without arguments is returned without copying.
macro_rules! format_in {
    ($arena:expr, $($arguments:tt)*) => {
        $crate::diagnostics::format_arguments_in($arena, format_args!($($arguments)*))
    };
}
pub(crate) use format_in;

/// The function behind [`format_in!`].
pub(crate) fn format_arguments_in<'d>(arena: &'d Bump, arguments: fmt::Arguments<'_>) -> &'d str {
    if let Some(text) = arguments.as_str() {
        return text;
    }
    let mut text = ArenaString::new_in(arena);
    let _ = text.write_fmt(arguments);
    text.into_str()
}

/// A diagnostic ready to be rendered. Its text and labels live in the arena
/// it was built in.
#[derive(Debug)]
pub(crate) struct Diagnostic<'d> {
    pub(crate) severity: ErrorSeverity,
    pub(crate) message:  &'d str,
    labels:              ArenaVec<'d, Label<'d>>,
    notes:               ArenaVec<'d, &'d str>,
    help:                ArenaVec<'d, &'d str>,
}

/// A source range the diagnostic points at, optionally with a short message
/// printed under it.
#[derive(Debug)]
struct Label<'d> {
    source:  LabelSource<'d>,
    message: Option<&'d str>,
    primary: bool,
}

/// Where a label points: a provenance range, or explicit segments when only
/// part of a range is relevant.
#[derive(Debug, Clone, Copy)]
enum LabelSource<'d> {
    Range(SourceVectors),
    Segments(&'d [SourceVector]),
}

impl<'d> LabelSource<'d> {
    fn vectors<'a>(self, context: &'a Context<'_>) -> &'a [SourceVector]
    where
        'd: 'a,
    {
        match self {
            | Self::Range(source) => context.get_source_vectors(source),
            | Self::Segments(segments) => segments,
        }
    }
}

impl<'d> Diagnostic<'d> {
    /// Adds the range the diagnostic is about. Its first location becomes the
    /// `-->` location.
    pub(crate) fn primary(mut self, source: SourceVectors, label: Option<&'d str>) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: label,
            primary: true,
        });
        self
    }

    /// Adds a related range, drawn with `-` instead of `^`.
    pub(crate) fn secondary(mut self, source: SourceVectors, label: &'d str) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: Some(label),
            primary: false,
        });
        self
    }

    /// Folds a follow-on diagnostic reported at the same place into this
    /// one, keeping only its related ranges: one mistake, one error.
    pub(crate) fn absorb(&mut self, other: Self, context: &Context<'_>) {
        for label in other.labels {
            let duplicate = self.labels.iter().any(|existing| {
                existing.message == label.message
                    && existing.source.vectors(context) == label.source.vectors(context)
            });
            if !label.primary && !duplicate {
                self.labels.push(label);
            }
        }
    }

    /// Adds a related range given as explicit segments.
    pub(crate) fn secondary_segments(
        mut self,
        segments: &'d [SourceVector],
        label: &'d str,
    ) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Segments(segments),
            message: Some(label),
            primary: false,
        });
        self
    }
}

/// Conversion of a phase's error value into a user-facing diagnostic.
///
/// `source` is the error's primary range, already materialized by
/// [`GetSourceVectors`](crate::translation_phases::GetSourceVectors).
pub(crate) trait ToDiagnostic {
    /// Builds the diagnostic in `arena`.
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d>;

    /// The diagnostic with owned text, for tests to inspect.
    #[cfg(test)]
    fn to_diagnostic(&self, context: &Context<'_>, source: SourceVectors) -> OwnedDiagnostic {
        self.diagnostic_in(context, source, context.tu_arena())
            .to_owned_diagnostic()
    }
}

/// The location-independent part of a diagnostic: what went wrong, a short
/// label for the primary range, and any notes and help, all in one arena.
pub(crate) struct Explanation<'d> {
    arena:              &'d Bump,
    pub(crate) message: &'d str,
    pub(crate) label:   Option<&'d str>,
    pub(crate) notes:   ArenaVec<'d, &'d str>,
    pub(crate) help:    ArenaVec<'d, &'d str>,
}

impl<'d> Explanation<'d> {
    pub(crate) fn new(arena: &'d Bump, message: &'d str) -> Self {
        Self {
            arena,
            message,
            label: None,
            notes: ArenaVec::new_in(arena),
            help: ArenaVec::new_in(arena),
        }
    }

    pub(crate) fn label(mut self, label: &'d str) -> Self {
        self.label = Some(label);
        self
    }

    pub(crate) fn note(mut self, note: &'d str) -> Self {
        self.notes.push(note);
        self
    }

    pub(crate) fn help(mut self, help: &'d str) -> Self {
        self.help.push(help);
        self
    }

    /// Attaches the explanation to the source it is about.
    pub(crate) fn at(self, severity: ErrorSeverity, source: SourceVectors) -> Diagnostic<'d> {
        Diagnostic {
            severity,
            message: self.message,
            labels: ArenaVec::new_in(self.arena),
            notes: self.notes,
            help: self.help,
        }
        .primary(source, self.label)
    }

    /// The explanation with owned text, for tests to inspect.
    #[cfg(test)]
    #[expect(
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn to_owned_explanation(&self) -> OwnedExplanation {
        OwnedExplanation {
            message: self.message.to_owned(),
            label:   self.label.map(str::to_owned),
            notes:   self.notes.iter().map(|&note| note.to_owned()).collect(),
            help:    self.help.iter().map(|&help| help.to_owned()).collect(),
        }
    }
}

/// An [`Explanation`] with owned text, for tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct OwnedExplanation {
    pub(crate) message: String,
    pub(crate) label:   Option<String>,
    pub(crate) notes:   Vec<String>,
    pub(crate) help:    Vec<String>,
}

/// A [`Diagnostic`] with owned text, for tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedDiagnostic {
    pub(crate) severity: ErrorSeverity,
    pub(crate) message:  String,
    labels:              Vec<(OwnedLabelSource, Option<String>, bool)>,
    notes:               Vec<String>,
    help:                Vec<String>,
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Test-only owned copies, compiled only under `cfg(test)`."
)]
#[derive(Debug, Clone, PartialEq)]
enum OwnedLabelSource {
    Range(SourceVectors),
    Segments(Vec<SourceVector>),
}

#[cfg(test)]
impl Diagnostic<'_> {
    #[expect(
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn to_owned_diagnostic(&self) -> OwnedDiagnostic {
        OwnedDiagnostic {
            severity: self.severity,
            message:  self.message.to_owned(),
            labels:   self
                .labels
                .iter()
                .map(|label| {
                    let source = match label.source {
                        | LabelSource::Range(source) => OwnedLabelSource::Range(source),
                        | LabelSource::Segments(segments) =>
                            OwnedLabelSource::Segments(segments.to_vec()),
                    };
                    (source, label.message.map(str::to_owned), label.primary)
                })
                .collect(),
            notes:    self.notes.iter().map(|&note| note.to_owned()).collect(),
            help:     self.help.iter().map(|&help| help.to_owned()).collect(),
        }
    }
}

#[cfg(test)]
impl OwnedDiagnostic {
    /// The diagnostic, borrowing this one's text, with its lists in `arena`.
    #[expect(
        clippy::disallowed_types,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    fn borrowed<'d>(&'d self, arena: &'d Bump) -> Diagnostic<'d> {
        let mut labels = ArenaVec::new_in(arena);
        labels.extend(self.labels.iter().map(|(source, message, primary)| Label {
            source:  match source {
                | OwnedLabelSource::Range(source) => LabelSource::Range(*source),
                | OwnedLabelSource::Segments(segments) => LabelSource::Segments(segments),
            },
            message: message.as_deref(),
            primary: *primary,
        }));
        let mut notes = ArenaVec::new_in(arena);
        notes.extend(self.notes.iter().map(String::as_str));
        let mut help = ArenaVec::new_in(arena);
        help.extend(self.help.iter().map(String::as_str));
        Diagnostic {
            severity: self.severity,
            message: &self.message,
            labels,
            notes,
            help,
        }
    }
}

/// Formats `count noun`, pluralizing the noun with `s` when needed.
pub(crate) fn count_of(count: usize, noun: &str) -> impl Display {
    fmt::from_fn(move |f| {
        if count == 1 {
            write!(f, "{count} {noun}")
        } else {
            write!(f, "{count} {noun}s")
        }
    })
}

/// Returns the candidate closest to `word` when it is a plausible typo
/// (at most two single-character edits, and fewer than the word's length).
/// The comparison works in `scratch`.
pub(crate) fn closest_match<'a>(
    scratch: &Bump,
    word: &str,
    candidates: &[&'a str],
) -> Option<&'a str> {
    let distance = |a: &str, b: &str| {
        let b = scratch.alloc_slice_fill_iter(b.chars());
        let row = scratch.alloc_slice_fill_iter(0..=b.len());
        for (i, ca) in a.chars().enumerate() {
            let mut diagonal = row[0];
            row[0] = i + 1;
            for (j, &cb) in b.iter().enumerate() {
                let above = row[j + 1];
                row[j + 1] = (above + 1)
                    .min(row[j] + 1)
                    .min(diagonal + usize::from(ca != cb));
                diagonal = above;
            }
        }
        row[b.len()]
    };
    candidates
        .iter()
        .map(|&candidate| (distance(word, candidate), candidate))
        .filter(|&(distance, _)| distance > 0 && distance <= 2 && distance < word.chars().count())
        .min_by_key(|&(distance, _)| distance)
        .map(|(_, candidate)| candidate)
}

/// Quotes a token spelling for use inside a message, shortening long ones.
pub(crate) fn quote_spelling(spelling: &str) -> impl Display {
    let spelling = spelling.trim_end_matches('\0');
    fmt::from_fn(move |f| {
        match spelling.char_indices().nth(MAX_QUOTED_SPELLING - 1) {
            // Longer than the limit: keep the characters before the last one
            // that would fit.
            | Some((cut, _)) if spelling.chars().nth(MAX_QUOTED_SPELLING).is_some() =>
                write!(f, "`{}…`", &spelling[..cut]),
            | _ => write!(f, "`{spelling}`"),
        }
    })
}

/// Writes `value` as C source text inside the given quotes, escaping
/// characters that would otherwise be invisible or ambiguous.
pub(crate) fn write_c_quoted(
    out: &mut impl fmt::Write,
    prefix: &str,
    quote: char,
    value: &str,
) -> fmt::Result {
    out.write_str(prefix)?;
    out.write_char(quote)?;
    for c in value.chars() {
        match c {
            | '\\' => out.write_str("\\\\")?,
            | '\n' => out.write_str("\\n")?,
            | '\t' => out.write_str("\\t")?,
            | '\r' => out.write_str("\\r")?,
            | '\x07' => out.write_str("\\a")?,
            | '\x08' => out.write_str("\\b")?,
            | '\x0B' => out.write_str("\\v")?,
            | '\x0C' => out.write_str("\\f")?,
            // A fixed-width escape cannot absorb a following octal digit.
            | '\0' => out.write_str("\\000")?,
            | c if c == quote => {
                out.write_char('\\')?;
                out.write_char(c)?;
            },
            | c if u32::from(c) < 0x20 || c == '\x7F' => write!(out, "\\{:03o}", u32::from(c))?,
            | c if c.is_control() => write!(out, "\\u{:04X}", u32::from(c))?,
            | c => out.write_char(c)?,
        }
    }
    out.write_char(quote)
}

/// How rendered text is decorated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorChoice {
    Plain,
    Ansi,
}

impl ColorChoice {
    /// Colors output only for a terminal, honoring `NO_COLOR`
    /// (<https://no-color.org>) and `CLICOLOR_FORCE`.
    #[expect(
        clippy::disallowed_methods,
        reason = "Reads `NO_COLOR` and `CLICOLOR_FORCE` once while the CLI sets up its renderer; \
                  std returns environment values owned."
    )]
    pub(crate) fn for_stderr() -> Self {
        use std::io::IsTerminal as _;
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("NO_COLOR") {
            Self::Plain
        } else if set("CLICOLOR_FORCE") || std::io::stderr().is_terminal() {
            Self::Ansi
        } else {
            Self::Plain
        }
    }
}

/// Renders diagnostics with reusable scratch storage.
pub(crate) struct Renderer {
    color:   ColorChoice,
    scratch: Bump,
}

struct Painted<'a> {
    text:  &'a str,
    style: Style,
    color: ColorChoice,
}

impl Display for Painted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.color {
            | ColorChoice::Plain => f.write_str(self.text),
            | ColorChoice::Ansi => write!(f, "{}", self.text.style(self.style)),
        }
    }
}

/// One underline on one physical source line.
#[derive(Debug, Clone)]
struct Mark<'a> {
    file:    u32,
    line:    usize,
    /// Byte range within the file, clipped to the line.
    start:   usize,
    end:     usize,
    primary: bool,
    label:   Option<&'a str>,
    /// Where the mark was made among the diagnostic's marks.
    order:   usize,
}

impl Renderer {
    pub(crate) fn new(color: ColorChoice) -> Self {
        Self {
            color,
            scratch: Bump::new(),
        }
    }

    fn paint<'a>(&self, text: &'a str, style: Style) -> Painted<'a> {
        Painted {
            text,
            style,
            color: self.color,
        }
    }

    fn paint_into(&self, out: &mut ArenaString<'_>, text: &str, style: Style) {
        let _ = write!(out, "{}", self.paint(text, style));
    }

    fn severity_style(severity: ErrorSeverity) -> Style {
        match severity {
            | ErrorSeverity::Error => Style::new().bright_red().bold(),
            | ErrorSeverity::Warning => Style::new().bright_yellow().bold(),
            | ErrorSeverity::Note => Style::new().bright_green().bold(),
        }
    }

    fn gutter_style() -> Style {
        Style::new().bright_blue().bold()
    }

    /// Returns the 1-based physical line containing `offset` and the byte
    /// range of that line, excluding its terminator.
    fn locate(starts: &[usize], text: &str, offset: usize) -> (usize, usize, usize) {
        let line = starts.partition_point(|&start| start <= offset);
        let start = starts[line - 1];
        let end = starts
            .get(line)
            .map_or(text.len(), |&next| next - 1)
            .max(start);
        let end = if text[start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        (line, start, end)
    }

    fn marks<'a, 'scratch>(
        diagnostic: &'a Diagnostic<'_>,
        context: &Context<'_>,
        scratch: &'scratch Bump,
    ) -> ArenaVec<'scratch, Mark<'a>> {
        let mut marks: ArenaVec<'_, Mark<'a>> = ArenaVec::new_in(scratch);
        for label in &diagnostic.labels {
            let mut label_text = label.message;
            for vector in label.source.vectors(context) {
                let file = vector.source_file_index;
                let Some(text) = context.source_text(file) else {
                    continue;
                };
                let starts = context
                    .source_line_starts(file)
                    .expect("recorded source text has a line index");
                let offset = (vector.index as usize).min(text.len());
                let (line, line_start, line_end) = Self::locate(starts, text, offset);
                let start = offset.max(line_start);
                let end = vector.end().clamp(start, line_end);
                // Adjacent segments of one range on one line form one mark.
                if let Some(last) = marks.last_mut()
                    && last.file == file
                    && last.line == line
                    && last.primary == label.primary
                    && label_text.is_none()
                {
                    last.start = last.start.min(start);
                    last.end = last.end.max(end);
                    continue;
                }
                marks.push(Mark {
                    file,
                    line,
                    start,
                    end,
                    primary: label.primary,
                    label: label_text.take(),
                    order: marks.len(),
                });
            }
        }
        marks
    }

    /// Renders one diagnostic, ending with a blank line. The text lives in
    /// the renderer's scratch arena until the next diagnostic is rendered.
    pub(crate) fn render_text(
        &mut self,
        diagnostic: &Diagnostic<'_>,
        context: &Context<'_>,
    ) -> &str {
        self.scratch.reset();
        self.render_in_scratch(diagnostic, context)
    }

    /// Renders a diagnostic with owned text, for tests.
    #[cfg(test)]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn render(&mut self, diagnostic: &OwnedDiagnostic, context: &Context<'_>) -> String {
        self.scratch.reset();
        let diagnostic = diagnostic.borrowed(&self.scratch);
        self.render_in_scratch(&diagnostic, context).to_owned()
    }

    fn render_in_scratch(&self, diagnostic: &Diagnostic<'_>, context: &Context<'_>) -> &str {
        let scratch = &self.scratch;
        let mut out = ArenaString::new_in(scratch);
        let severity_style = Self::severity_style(diagnostic.severity);
        let severity = match diagnostic.severity {
            | ErrorSeverity::Error => "error:",
            | ErrorSeverity::Warning => "warning:",
            | ErrorSeverity::Note => "note:",
        };
        let _ = writeln!(
            out,
            "{} {}",
            self.paint(severity, severity_style),
            self.paint(diagnostic.message, Style::new().bold()),
        );

        let marks = Self::marks(diagnostic, context, scratch);
        let max_line = marks.iter().map(|mark| mark.line + 1).max().unwrap_or(0);
        let gutter_width = if max_line == 0 {
            1
        } else {
            max_line.ilog10() as usize + 1
        };
        let mut pad = ArenaString::new_in(scratch);
        for _ in 0..gutter_width {
            pad.push(' ');
        }
        let bar = self.paint("|", Self::gutter_style());

        // Files in order of first appearance, primary file first.
        let mut files = ArenaVec::new_in(scratch);
        for mark in marks.iter().filter(|mark| mark.primary).chain(&marks) {
            if !files.contains(&mark.file) {
                files.push(mark.file);
            }
        }
        if files.is_empty() {
            // No quotable source: still say where, when a location exists.
            if let Some(vector) = diagnostic
                .labels
                .iter()
                .find_map(|label| label.source.vectors(context).first())
            {
                let _ = writeln!(
                    out,
                    "{}{} {}:{}:{}",
                    pad,
                    self.paint("-->", Self::gutter_style()),
                    context.get_source_file(vector.source_file_index).display(),
                    vector.line,
                    vector.column,
                );
            }
        }
        for (file_number, &file) in files.iter().enumerate() {
            let text = context.source_text(file).unwrap_or_default();
            let first = diagnostic
                .labels
                .iter()
                .filter(|label| file_number != 0 || label.primary)
                .flat_map(|label| label.source.vectors(context))
                .find(|vector| vector.source_file_index == file)
                .cloned();
            let arrow = if file_number == 0 { "-->" } else { ":::" };
            let (line, column) = first.map_or((1, 1), |vector| (vector.line, vector.column));
            let _ = writeln!(
                out,
                "{}{} {}:{}:{}",
                pad,
                self.paint(arrow, Self::gutter_style()),
                context.get_source_file(file).display(),
                line,
                column,
            );
            let _ = writeln!(out, "{pad} {bar}");
            let starts = context.source_line_starts(file).unwrap_or(&[0]);
            self.render_file_lines(
                &mut out,
                &marks,
                (file, text, starts),
                diagnostic.severity,
                (gutter_width, pad.as_str()),
                scratch,
            );
        }

        let has_snippet = !files.is_empty();
        if has_snippet && (!diagnostic.notes.is_empty() || !diagnostic.help.is_empty()) {
            let _ = writeln!(out, "{pad} {bar}");
        }
        for (heading, items) in [("note:", &diagnostic.notes), ("help:", &diagnostic.help)] {
            for item in items {
                let mut continuation = ArenaString::new_in(scratch);
                for _ in 0..=(gutter_width + 3 + heading.len()) {
                    continuation.push(' ');
                }
                let mut lines = item.lines();
                let _ = writeln!(
                    out,
                    "{pad} {} {} {}",
                    self.paint("=", Self::gutter_style()),
                    self.paint(heading, Style::new().bold()),
                    lines.next().unwrap_or_default(),
                );
                for line in lines {
                    let _ = writeln!(out, "{continuation}{line}");
                }
            }
        }
        out.push('\n');
        out.into_str()
    }

    fn render_file_lines(
        &self,
        out: &mut ArenaString<'_>,
        marks: &[Mark<'_>],
        (file, text, starts): (u32, &str, &[usize]),
        severity: ErrorSeverity,
        (gutter_width, pad): (usize, &str),
        scratch: &Bump,
    ) {
        let bar = self.paint("|", Self::gutter_style());
        // Sort once and walk the marks line by line: rescanning every mark
        // for each line made a label spanning many lines quadratic.
        let mut file_marks = ArenaVec::new_in(scratch);
        file_marks.extend(marks.iter().filter(|mark| mark.file == file));
        // Equal marks keep the order they were made in. An unstable sort with
        // that tiebreak needs no buffer, where a stable sort of many marks
        // would allocate one.
        file_marks.sort_unstable_by_key(|mark| (mark.line, mark.start, !mark.primary, mark.order));
        let line_text = |line: usize| -> &str {
            let start = starts.get(line - 1).copied().unwrap_or(text.len());
            let end = starts
                .get(line)
                .map_or(text.len(), |&next| next - 1)
                .max(start);
            text[start..end].trim_end_matches('\r')
        };
        let mut previous: Option<usize> = None;
        for on_line in file_marks.chunk_by(|left, right| left.line == right.line) {
            let line = on_line[0].line;
            if let Some(previous) = previous {
                if line == previous + 2 {
                    self.write_source_line(
                        out,
                        previous + 1,
                        line_text(previous + 1),
                        gutter_width,
                        scratch,
                    );
                } else if line > previous + 2 {
                    let _ = writeln!(out, "{}", self.paint("...", Self::gutter_style()));
                }
            }
            previous = Some(line);
            let source = line_text(line);
            self.write_source_line(out, line, source, gutter_width, scratch);
            let line_start = starts[line - 1];

            // Underline row: secondary marks first so primary ones win.
            let column_of = |offset: usize| {
                display_width(&source[..offset.saturating_sub(line_start).min(source.len())])
            };
            let mut cells = ArenaVec::new_in(scratch);
            for primary in [false, true] {
                for mark in on_line.iter().filter(|mark| mark.primary == primary) {
                    let from = column_of(mark.start);
                    let to = column_of(mark.end).max(from + 1);
                    if cells.len() < to {
                        cells.resize(to, None);
                    }
                    for cell in &mut cells[from..to] {
                        *cell = Some(primary);
                    }
                }
            }
            let underline_style = |primary: bool| {
                if primary {
                    Self::severity_style(severity)
                } else {
                    Self::gutter_style()
                }
            };
            let mut underline = ArenaString::new_in(scratch);
            let mut run: Option<(bool, ArenaString<'_>)> = None;
            let flush = |run: &mut Option<(bool, ArenaString<'_>)>,
                         underline: &mut ArenaString<'_>| {
                if let Some((primary, text)) = run.take() {
                    self.paint_into(underline, &text, underline_style(primary));
                }
            };
            for cell in &cells {
                match (cell, &mut run) {
                    | (None, _) => {
                        flush(&mut run, &mut underline);
                        underline.push(' ');
                    },
                    | (Some(primary), Some((current, text))) if current == primary =>
                        text.push(if *primary { '^' } else { '-' }),
                    | (Some(primary), _) => {
                        flush(&mut run, &mut underline);
                        let mut text = ArenaString::new_in(scratch);
                        text.push(if *primary { '^' } else { '-' });
                        run = Some((*primary, text));
                    },
                }
            }
            flush(&mut run, &mut underline);

            let mut labelled = ArenaVec::new_in(scratch);
            labelled.extend(on_line.iter().filter_map(|mark| {
                mark.label
                    .map(|label| (column_of(mark.start), mark.primary, label))
            }));
            match labelled.split_last() {
                | None => {
                    let _ = writeln!(out, "{pad} {bar} {underline}");
                },
                | Some((&(_, primary, label), rest)) => {
                    let _ = writeln!(
                        out,
                        "{pad} {bar} {underline} {}",
                        self.paint(label, underline_style(primary))
                    );
                    // Remaining labels hang below their marks, rightmost first.
                    for (index, &(column, primary, label)) in rest.iter().enumerate().rev() {
                        let connectors = |upto: usize| {
                            let mut row = ArenaString::new_in(scratch);
                            let mut width = 0;
                            for &(column, primary, _) in &rest[..upto] {
                                for _ in width..column {
                                    row.push(' ');
                                }
                                self.paint_into(&mut row, "|", underline_style(primary));
                                width = column + 1;
                            }
                            (row, width)
                        };
                        let (row, _) = connectors(index + 1);
                        let _ = writeln!(out, "{pad} {bar} {row}");
                        let (mut row, width) = connectors(index);
                        for _ in width..column {
                            row.push(' ');
                        }
                        self.paint_into(&mut row, label, underline_style(primary));
                        let _ = writeln!(out, "{pad} {bar} {row}");
                    }
                },
            }
        }
    }

    fn write_source_line(
        &self,
        out: &mut ArenaString<'_>,
        line: usize,
        source: &str,
        gutter_width: usize,
        scratch: &Bump,
    ) {
        let mut number = ArenaString::new_in(scratch);
        let _ = write!(number, "{line:>gutter_width$}");
        let visible = visible_source(source, scratch);
        let _ = writeln!(
            out,
            "{} {} {}",
            self.paint(&number, Self::gutter_style()),
            self.paint("|", Self::gutter_style()),
            visible.as_str().trim_end(),
        );
    }
}

/// Expands tabs and shows other control characters as their one-column
/// Unicode control pictures, so a snippet never emits raw control bytes
/// and carets stay aligned.
fn visible_source<'scratch>(text: &str, scratch: &'scratch Bump) -> ArenaString<'scratch> {
    let mut visible = ArenaString::new_in(scratch);
    for c in text.chars() {
        if c == '\t' {
            for _ in 0..TAB_WIDTH {
                visible.push(' ');
            }
        } else {
            visible.push(match u32::from(c) {
                | code @ 0..0x20 => char::from_u32(0x2400 + code).unwrap_or(c),
                | 0x7F => '\u{2421}',
                | _ => c,
            });
        }
    }
    visible
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use std::path::Path;

    use super::*;

    fn with_context<R>(text: &str, inspect: impl FnOnce(&mut Context<'_>, u32) -> R) -> R {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let file = context.intern_source_file(Path::new("example.c"));
        context.record_source_text(file, text);
        inspect(&mut context, file)
    }

    fn span(context: &mut Context<'_>, file: u32, text: &str, needle: &str) -> SourceVectors {
        let index = text.find(needle).expect("needle occurs in text");
        let line = text[..index].matches('\n').count() + 1;
        let column = index - text[..index].rfind('\n').map_or(0, |newline| newline + 1) + 1;
        context.create_source_vectors(
            crate::translation_phases::SourcePosition {
                index,
                line: u32::try_from(line).unwrap(),
                column: u32::try_from(column).unwrap(),
            },
            file,
            needle.len(),
        )
    }

    #[test]
    fn renders_primary_and_secondary_labels_with_notes() {
        let text = "int x \"abc\";\n";
        with_context(text, |context, file| {
            let primary = span(context, file, text, "\"abc\"");
            let secondary = span(context, file, text, "x");
            let arena = Bump::new();
            let diagnostic = Explanation::new(&arena, "expected `;`, found string literal")
                .label("expected `;`")
                .note("a declaration ends with `;`")
                .help("add `;` after `x`")
                .at(ErrorSeverity::Error, primary)
                .secondary(secondary, "declarator");

            let mut renderer = Renderer::new(ColorChoice::Plain);
            let actual = renderer.render_text(&diagnostic, context).to_owned();

            assert_eq!(
                actual,
                "error: expected `;`, found string literal\n --> example.c:1:7\n  |\n1 | int x \
                 \"abc\";\n  |     - ^^^^^ expected `;`\n  |     |\n  |     declarator\n  |\n  = \
                 note: a declaration ends with `;`\n  = help: add `;` after `x`\n\n"
            );
            assert_eq!(renderer.render_text(&diagnostic, context), actual);
        });
    }

    #[test]
    fn reused_renderer_uses_the_current_contexts_line_starts() {
        let mut renderer = Renderer::new(ColorChoice::Plain);
        with_context("first line", |context, file| {
            let source = span(context, file, "first line", "first");
            let arena = Bump::new();
            let diagnostic = Explanation::new(&arena, "first").at(ErrorSeverity::Error, source);
            _ = renderer.render_text(&diagnostic, context);
        });
        with_context("x\ny\n", |context, file| {
            let source = span(context, file, "x\ny\n", "y");
            let arena = Bump::new();
            let diagnostic = Explanation::new(&arena, "second").at(ErrorSeverity::Error, source);
            let output = renderer.render_text(&diagnostic, context);
            assert!(output.contains("2 | y"), "{output}");
        });
    }

    #[test]
    fn per_diagnostic_scratch_does_not_scale_with_source_line_count() {
        let scratch_for = |lines| {
            with_context(&"int x;\n".repeat(lines), |context, file| {
                let source = span(context, file, "int x;\n", "x");
                let arena = Bump::new();
                let diagnostic =
                    Explanation::new(&arena, "example").at(ErrorSeverity::Error, source);
                let mut renderer = Renderer::new(ColorChoice::Plain);
                let first = renderer.render_text(&diagnostic, context).to_owned();
                let used = context.tu_arena().used();
                for _ in 0..16 {
                    assert_eq!(renderer.render_text(&diagnostic, context), first);
                }
                assert_eq!(context.tu_arena().used(), used);
                renderer.scratch.high_water()
            })
        };
        assert_eq!(scratch_for(32), scratch_for(8_192));
    }

    #[test]
    fn replaced_source_text_uses_its_own_line_index() {
        with_context("first line", |context, file| {
            let mut renderer = Renderer::new(ColorChoice::Plain);
            let arena = Bump::new();
            let source = span(context, file, "first line", "first");
            let diagnostic = Explanation::new(&arena, "first").at(ErrorSeverity::Error, source);
            _ = renderer.render_text(&diagnostic, context);

            let text = "x\r\ny\rz\n";
            context.record_source_text(file, text);
            let source = context.create_source_vectors(
                crate::translation_phases::SourcePosition {
                    index:  5,
                    line:   3,
                    column: 1,
                },
                file,
                1,
            );
            let diagnostic = Explanation::new(&arena, "second").at(ErrorSeverity::Error, source);
            let output = renderer.render_text(&diagnostic, context);
            assert_eq!(
                output,
                "error: second\n --> example.c:3:1\n  |\n3 | z\n  | ^\n\n"
            );
        });
    }

    #[test]
    fn zero_length_ranges_point_after_the_line() {
        let text = "int x";
        with_context(text, |context, file| {
            let end = context.create_source_vectors(
                crate::translation_phases::SourcePosition {
                    index:  5,
                    line:   1,
                    column: 6,
                },
                file,
                0,
            );
            let arena = Bump::new();
            let diagnostic = Explanation::new(&arena, "no newline at end of file")
                .at(ErrorSeverity::Warning, end);

            let mut renderer = Renderer::new(ColorChoice::Plain);
            let output = renderer.render_text(&diagnostic, context);

            assert!(output.contains("1 | int x\n  |      ^\n"), "{output}");
        });
    }

    fn c_quoted(prefix: &str, quote: char, value: &str) -> String {
        let mut out = String::new();
        write_c_quoted(&mut out, prefix, quote, value).expect("writing to a string cannot fail");
        out
    }

    #[test]
    fn quoted_strings_use_c_escapes() {
        assert_eq!(c_quoted("L", '"', "a\n\"\u{1b}"), "L\"a\\n\\\"\\033\"");
        assert_eq!(c_quoted("", '\'', "'"), "'\\''");
        assert_eq!(quote_spelling("12\0").to_string(), "`12`");
        let long = "a".repeat(MAX_QUOTED_SPELLING);
        assert_eq!(quote_spelling(&long).to_string(), format!("`{long}`"));
        assert_eq!(
            quote_spelling(&format!("{long}é")).to_string(),
            format!("`{}…`", &long[1..])
        );
        assert_eq!(count_of(1, "error").to_string(), "1 error");
        assert_eq!(count_of(2, "error").to_string(), "2 errors");
    }
}
