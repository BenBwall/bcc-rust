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

use std::{
    collections::HashMap,
    fmt::Write as _,
};

use owo_colors::{
    OwoColorize,
    Style,
};

use crate::translation_phases::{
    Context,
    ErrorSeverity,
    SourceVector,
    SourceVectors,
};

/// Width a tab occupies when a source line is echoed.
const TAB_WIDTH: usize = 4;

/// Longest token spelling quoted inline in a message before it is shortened.
const MAX_QUOTED_SPELLING: usize = 40;

/// A diagnostic ready to be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub(crate) severity: ErrorSeverity,
    pub(crate) message:  String,
    labels:              Vec<Label>,
    notes:               Vec<String>,
    help:                Vec<String>,
}

/// A source range the diagnostic points at, optionally with a short message
/// printed under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Label {
    source:  LabelSource,
    message: Option<String>,
    primary: bool,
}

/// Where a label points: a provenance range, or explicit segments when only
/// part of a range is relevant.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LabelSource {
    Range(SourceVectors),
    Segments(Vec<SourceVector>),
}

impl LabelSource {
    fn vectors<'a>(&'a self, context: &'a Context<'_>) -> &'a [SourceVector] {
        match self {
            | Self::Range(source) => context.get_source_vectors(*source),
            | Self::Segments(segments) => segments,
        }
    }
}

impl Diagnostic {
    pub(crate) fn new(severity: ErrorSeverity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    /// Adds the range the diagnostic is about. Its first location becomes the
    /// `-->` location.
    pub(crate) fn primary(mut self, source: SourceVectors, label: Option<String>) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: label,
            primary: true,
        });
        self
    }

    /// Adds a related range, drawn with `-` instead of `^`.
    pub(crate) fn secondary(mut self, source: SourceVectors, label: impl Into<String>) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Range(source),
            message: Some(label.into()),
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
        segments: Vec<SourceVector>,
        label: impl Into<String>,
    ) -> Self {
        self.labels.push(Label {
            source:  LabelSource::Segments(segments),
            message: Some(label.into()),
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
    fn to_diagnostic(&self, context: &Context<'_>, source: SourceVectors) -> Diagnostic;
}

/// The location-independent part of a diagnostic: what went wrong, a short
/// label for the primary range, and any notes and help.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Explanation {
    pub(crate) message: String,
    pub(crate) label:   Option<String>,
    pub(crate) notes:   Vec<String>,
    pub(crate) help:    Vec<String>,
}

impl Explanation {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    pub(crate) fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub(crate) fn help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }

    /// Attaches the explanation to the source it is about.
    pub(crate) fn at(self, severity: ErrorSeverity, source: SourceVectors) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(severity, self.message).primary(source, self.label);
        diagnostic.notes = self.notes;
        diagnostic.help = self.help;
        diagnostic
    }
}

/// Formats `count noun`, pluralizing the noun with `s` when needed.
pub(crate) fn count_of(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Returns the candidate closest to `word` when it is a plausible typo
/// (at most two single-character edits, and fewer than the word's length).
pub(crate) fn closest_match<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let distance = |a: &str, b: &str| {
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
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
pub(crate) fn quote_spelling(spelling: &str) -> String {
    let spelling = spelling.trim_end_matches('\0');
    if spelling.chars().count() <= MAX_QUOTED_SPELLING {
        format!("`{spelling}`")
    } else {
        let prefix: String = spelling.chars().take(MAX_QUOTED_SPELLING - 1).collect();
        format!("`{prefix}…`")
    }
}

/// Spells a string as C source text inside the given quotes, escaping
/// characters that would otherwise be invisible or ambiguous.
pub(crate) fn c_quoted(prefix: &str, quote: char, value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2 + prefix.len());
    out.push_str(prefix);
    out.push(quote);
    for c in value.chars() {
        match c {
            | '\\' => out.push_str("\\\\"),
            | '\n' => out.push_str("\\n"),
            | '\t' => out.push_str("\\t"),
            | '\r' => out.push_str("\\r"),
            | '\x07' => out.push_str("\\a"),
            | '\x08' => out.push_str("\\b"),
            | '\x0B' => out.push_str("\\v"),
            | '\x0C' => out.push_str("\\f"),
            // A fixed-width escape cannot absorb a following octal digit.
            | '\0' => out.push_str("\\000"),
            | c if c == quote => {
                out.push('\\');
                out.push(c);
            },
            | c if u32::from(c) < 0x20 || c == '\x7F' => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            },
            | c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            },
            | c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Byte offsets at which each physical line of `text` starts. Like initial
/// processing, LF, CRLF, and a lone CR each end a line; every terminator is
/// one byte before the next start once a CRLF's CR is trimmed.
fn physical_line_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    std::iter::once(0)
        .chain(
            bytes
                .iter()
                .enumerate()
                .filter(|&(index, &byte)| {
                    byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
                })
                .map(|(index, _)| index + 1),
        )
        .collect()
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

/// Renders diagnostics, caching each file's line starts across calls.
pub(crate) struct Renderer {
    color:       ColorChoice,
    line_starts: HashMap<u32, Vec<usize>>,
}

/// One underline on one physical source line.
#[derive(Debug, Clone)]
struct Mark {
    file:    u32,
    line:    usize,
    /// Byte range within the file, clipped to the line.
    start:   usize,
    end:     usize,
    primary: bool,
    label:   Option<String>,
}

impl Renderer {
    pub(crate) fn new(color: ColorChoice) -> Self {
        Self {
            color,
            line_starts: HashMap::new(),
        }
    }

    fn paint(&self, text: &str, style: Style) -> String {
        match self.color {
            | ColorChoice::Plain => text.to_owned(),
            | ColorChoice::Ansi => text.style(style).to_string(),
        }
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
    fn locate(&mut self, file: u32, text: &str, offset: usize) -> (usize, usize, usize) {
        let starts = self
            .line_starts
            .entry(file)
            .or_insert_with(|| physical_line_starts(text));
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

    fn marks(&mut self, diagnostic: &Diagnostic, context: &Context<'_>) -> Vec<Mark> {
        let mut marks: Vec<Mark> = Vec::new();
        for label in &diagnostic.labels {
            let mut label_text = label.message.clone();
            for vector in label.source.vectors(context) {
                let file = vector.source_file_index;
                let Some(text) = context.source_text(file) else {
                    continue;
                };
                let offset = (vector.index as usize).min(text.len());
                let (line, line_start, line_end) = self.locate(file, text, offset);
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
                });
            }
        }
        marks
    }

    /// Renders one diagnostic, ending with a blank line.
    pub(crate) fn render(&mut self, diagnostic: &Diagnostic, context: &Context<'_>) -> String {
        let mut out = String::new();
        let severity_style = Self::severity_style(diagnostic.severity);
        let _ = writeln!(
            out,
            "{}{}",
            self.paint(&format!("{}:", diagnostic.severity), severity_style),
            self.paint(&format!(" {}", diagnostic.message), Style::new().bold()),
        );

        let marks = self.marks(diagnostic, context);
        let max_line = marks.iter().map(|mark| mark.line + 1).max().unwrap_or(0);
        let gutter_width = max_line.to_string().len().max(1);
        let pad = " ".repeat(gutter_width);
        let bar = self.paint("|", Self::gutter_style());

        // Files in order of first appearance, primary file first.
        let mut files: Vec<u32> = Vec::new();
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
            let starts = self.line_starts.get(&file).map_or(&[0][..], Vec::as_slice);
            self.render_file_lines(
                &mut out,
                &marks,
                (file, text, starts),
                diagnostic.severity,
                gutter_width,
            );
        }

        let has_snippet = !files.is_empty();
        if has_snippet && (!diagnostic.notes.is_empty() || !diagnostic.help.is_empty()) {
            let _ = writeln!(out, "{pad} {bar}");
        }
        for (kind, items) in [("note", &diagnostic.notes), ("help", &diagnostic.help)] {
            for item in items {
                let heading = format!("{kind}:");
                let continuation = " ".repeat(gutter_width + 3 + heading.len() + 1);
                let mut lines = item.lines();
                let _ = writeln!(
                    out,
                    "{pad} {} {} {}",
                    self.paint("=", Self::gutter_style()),
                    self.paint(&heading, Style::new().bold()),
                    lines.next().unwrap_or_default(),
                );
                for line in lines {
                    let _ = writeln!(out, "{continuation}{line}");
                }
            }
        }
        out.push('\n');
        out
    }

    fn render_file_lines(
        &self,
        out: &mut String,
        marks: &[Mark],
        (file, text, starts): (u32, &str, &[usize]),
        severity: ErrorSeverity,
        gutter_width: usize,
    ) {
        let pad = " ".repeat(gutter_width);
        let bar = self.paint("|", Self::gutter_style());
        // Sort once and walk the marks line by line: rescanning every mark
        // for each line made a label spanning many lines quadratic.
        let mut file_marks: Vec<&Mark> = marks.iter().filter(|mark| mark.file == file).collect();
        file_marks.sort_by_key(|mark| (mark.line, mark.start, !mark.primary));
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
                    );
                } else if line > previous + 2 {
                    let _ = writeln!(out, "{}", self.paint("...", Self::gutter_style()));
                }
            }
            previous = Some(line);
            let source = line_text(line);
            self.write_source_line(out, line, source, gutter_width);
            let line_start = starts[line - 1];

            // Underline row: secondary marks first so primary ones win.
            let column_of = |offset: usize| {
                display_width(&source[..offset.saturating_sub(line_start).min(source.len())])
            };
            let mut cells: Vec<Option<bool>> = Vec::new();
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
            let mut underline = String::new();
            let mut run: Option<(bool, String)> = None;
            let flush = |run: &mut Option<(bool, String)>, underline: &mut String| {
                if let Some((primary, text)) = run.take() {
                    underline.push_str(&self.paint(&text, underline_style(primary)));
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
                        run = Some((*primary, String::from(if *primary { '^' } else { '-' })));
                    },
                }
            }
            flush(&mut run, &mut underline);

            let labelled: Vec<(usize, bool, &str)> = on_line
                .iter()
                .filter_map(|mark| {
                    mark.label
                        .as_deref()
                        .map(|label| (column_of(mark.start), mark.primary, label))
                })
                .collect();
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
                            let mut row = String::new();
                            let mut width = 0;
                            for &(column, primary, _) in &rest[..upto] {
                                row.push_str(&" ".repeat(column - width));
                                row.push_str(&self.paint("|", underline_style(primary)));
                                width = column + 1;
                            }
                            (row, width)
                        };
                        let (row, _) = connectors(index + 1);
                        let _ = writeln!(out, "{pad} {bar} {row}");
                        let (mut row, width) = connectors(index);
                        row.push_str(&" ".repeat(column - width));
                        row.push_str(&self.paint(label, underline_style(primary)));
                        let _ = writeln!(out, "{pad} {bar} {row}");
                    }
                },
            }
        }
    }

    fn write_source_line(&self, out: &mut String, line: usize, source: &str, gutter_width: usize) {
        let number = format!("{line:>gutter_width$}");
        let _ = writeln!(
            out,
            "{} {} {}",
            self.paint(&number, Self::gutter_style()),
            self.paint("|", Self::gutter_style()),
            visible_source(source).trim_end(),
        );
    }
}

/// Expands tabs and shows other control characters as their one-column
/// Unicode control pictures, so a snippet never emits raw control bytes
/// and carets stay aligned.
fn visible_source(text: &str) -> String {
    expand_tabs(text)
        .chars()
        .map(|c| match u32::from(c) {
            | code @ 0..0x20 => char::from_u32(0x2400 + code).unwrap_or(c),
            | 0x7F => '\u{2421}',
            | _ => c,
        })
        .collect()
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

fn expand_tabs(text: &str) -> String {
    text.replace('\t', &" ".repeat(TAB_WIDTH))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn with_context<R>(text: &str, inspect: impl FnOnce(&mut Context<'_>, u32) -> R) -> R {
        let tu = crate::util::bump::Bump::new();
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
            let diagnostic = Explanation::new("expected `;`, found string literal")
                .label("expected `;`")
                .note("a declaration ends with `;`")
                .help("add `;` after `x`")
                .at(ErrorSeverity::Error, primary)
                .secondary(secondary, "declarator");

            let rendered = Renderer::new(ColorChoice::Plain).render(&diagnostic, context);

            assert_eq!(
                rendered,
                "error: expected `;`, found string literal\n --> example.c:1:7\n  |\n1 | int x \
                 \"abc\";\n  |     - ^^^^^ expected `;`\n  |     |\n  |     declarator\n  |\n  = \
                 note: a declaration ends with `;`\n  = help: add `;` after `x`\n\n"
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
            let diagnostic =
                Explanation::new("no newline at end of file").at(ErrorSeverity::Warning, end);

            let rendered = Renderer::new(ColorChoice::Plain).render(&diagnostic, context);

            assert!(rendered.contains("1 | int x\n  |      ^\n"), "{rendered}");
        });
    }

    #[test]
    fn quoted_strings_use_c_escapes() {
        assert_eq!(c_quoted("L", '"', "a\n\"\u{1b}"), "L\"a\\n\\\"\\033\"");
        assert_eq!(c_quoted("", '\'', "'"), "'\\''");
        assert_eq!(quote_spelling("12\0"), "`12`");
    }
}
