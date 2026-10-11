//! Renders annotated source lines, gutters, labels, notes, and help using
//! per-diagnostic scratch storage.

use std::fmt::{
    self,
    Display,
    Write as _,
};

use owo_colors::{
    OwoColorize,
    Style,
};

#[cfg(test)]
use super::OwnedDiagnostic;
use super::{
    ColorChoice,
    Diagnostic,
    Renderer,
};
use crate::{
    translation_phases::{
        Context,
        ErrorSeverity,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl Renderer {
    pub(super) fn render_in_scratch(
        &self,
        diagnostic: &Diagnostic<'_>,
        context: &Context<'_>,
    ) -> &str {
        let scratch = &self.scratch;
        let mut out = ArenaString::new_in(scratch);
        let severity_style = Self::severity_style(diagnostic.severity);
        let severity = match diagnostic.severity {
            | ErrorSeverity::Error => "error:",
            | ErrorSeverity::Warning => "warning:",
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
        }
    }

    fn gutter_style() -> Style {
        Style::new().bright_blue().bold()
    }

    /// Renders a diagnostic with owned text, for tests.
    #[cfg(test)]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "Test-only owned copies, compiled only under `cfg(test)`."
    )]
    pub(crate) fn render(&mut self, diagnostic: &OwnedDiagnostic, context: &Context<'_>) -> String {
        // Previous output was copied into owned text; no scratch pointer
        // survives between renders.
        self.scratch.reset();
        let diagnostic = diagnostic.borrowed(&self.scratch);
        self.render_in_scratch(&diagnostic, context).to_owned()
    }

    pub(crate) fn new(color: ColorChoice) -> Self {
        Self {
            color,
            scratch: Bump::new(),
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

struct Painted<'a> {
    text:  &'a str,
    style: Style,
    color: ColorChoice,
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

/// Width a tab occupies when a source line is echoed.
const TAB_WIDTH: usize = 4;

impl Display for Painted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.color {
            | ColorChoice::Plain => f.write_str(self.text),
            | ColorChoice::Ansi => write!(f, "{}", self.text.style(self.style)),
        }
    }
}
