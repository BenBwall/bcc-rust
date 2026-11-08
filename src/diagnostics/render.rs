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
    model::Label,
};
use crate::{
    configuration::{
        DiagnosticFormat,
        DiagnosticPresentation,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
    },
    util::{
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
        raw_bytes,
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
            | ErrorSeverity::Error if diagnostic.fatal => "fatal error:",
            | ErrorSeverity::Error => "error:",
            | ErrorSeverity::Warning => "warning:",
        };
        let format = context.configuration.diagnostic_format();
        let compiler_snippet = matches!(format, DiagnosticFormat::Gcc | DiagnosticFormat::Clang);
        let presentation = if compiler_snippet {
            context.configuration.diagnostic_presentation()
        } else {
            DiagnosticPresentation::default()
        };
        // GCC, Clang and MSVC lead with the primary location.
        let located = diagnostic
            .labels
            .iter()
            .filter(|label| label.primary)
            .chain(&diagnostic.labels)
            .find_map(|label| Some((label.source.vectors(context).first()?, label.source)));
        let location = located.map(|(vector, _)| vector);
        let segment = located
            .filter(|_| compiler_snippet)
            .and_then(|(vector, source)| {
                self.segment_of(context, vector, [diagnostic.reading, source.range()])
            });
        if let Some(vector) = location
            && compiler_snippet
        {
            self.write_context_lines(&mut out, context, vector, segment, false);
        }
        if let Some(vector) = location
            && format != DiagnosticFormat::Bcc
        {
            let file = Self::file_name(context, vector, segment);
            let _ = match format {
                | DiagnosticFormat::Msvc { column: true, .. } =>
                    write!(out, "{file}({},{}): ", vector.line, vector.column),
                | DiagnosticFormat::Msvc { column: false, .. } =>
                    write!(out, "{file}({}): ", vector.line),
                | DiagnosticFormat::Tcc => write!(out, "{file}:{}: ", vector.line),
                | _ if !presentation.column => write!(out, "{file}:{}: ", vector.line),
                | _ => write!(
                    out,
                    "{file}:{}:{}: ",
                    vector.line,
                    Self::location_column(context, format, vector)
                ),
            };
        }
        // TCC names no warning options, so a message drops the `[-W…]`
        // suffix bcc appended as GCC and Clang print it, and only that. Those
        // two drop it on request.
        let drop_suffix = format == DiagnosticFormat::Tcc || !presentation.option_names;
        let message = if drop_suffix && diagnostic.option_suffix {
            diagnostic
                .message
                .rfind(" [-W")
                .map_or(diagnostic.message, |at| &diagnostic.message[..at])
        } else {
            diagnostic.message
        };
        // `cl` writes its code between the severity and the message:
        // `error C2065: 'y': undeclared identifier`.
        let severity: &str = match diagnostic.code {
            | Some(code) if matches!(format, DiagnosticFormat::Msvc { .. }) =>
                crate::diagnostics::format_in!(
                    scratch,
                    "{} {code}:",
                    severity.trim_end_matches(':')
                ),
            | _ => severity,
        };
        let _ = writeln!(
            out,
            "{} {}",
            self.paint(severity, severity_style),
            self.paint(message, Style::new().bold()),
        );
        // TCC writes the message line alone.
        if format == DiagnosticFormat::Tcc {
            return out.into_str();
        }
        if matches!(format, DiagnosticFormat::Msvc { caret: false, .. }) {
            for item in diagnostic.notes.iter().chain(&diagnostic.help) {
                for line in item.lines() {
                    let _ = writeln!(out, "        {line}");
                }
            }
            return out.into_str();
        }

        // GCC and Clang share one snippet: a gutter five wide, nothing around
        // the source line and its caret, and what relates to the message in
        // `note:` lines of its own.
        let mut marks = ArenaVec::new_in(scratch);
        for label in diagnostic
            .labels
            .iter()
            .filter(|label| presentation.caret && (!compiler_snippet || label.primary))
        {
            Self::push_label_marks(label, context, &mut marks);
        }
        if compiler_snippet {
            for mark in &mut marks {
                mark.label = None;
            }
        }
        let max_line = marks.iter().map(|mark| mark.line + 1).max().unwrap_or(0);
        let gutter_width = if max_line == 0 {
            1
        } else {
            max_line.ilog10() as usize + 1
        };
        let gutter_width = if compiler_snippet {
            gutter_width.max(5)
        } else {
            gutter_width
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
        if files.is_empty() && format == DiagnosticFormat::Bcc {
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
            // The other formats named the primary file on the first line.
            if file_number != 0 || format == DiagnosticFormat::Bcc {
                let _ = writeln!(
                    out,
                    "{}{} {}:{}:{}",
                    pad,
                    self.paint(arrow, Self::gutter_style()),
                    context.get_source_file(file).display(),
                    line,
                    column,
                );
            }
            if !compiler_snippet {
                let _ = writeln!(out, "{pad} {bar}");
            }
            let starts = context.source_line_starts(file).unwrap_or(&[0]);
            self.render_file_lines(
                &mut out,
                &marks,
                (file, text, starts),
                diagnostic.severity,
                &Gutter {
                    width: gutter_width,
                    pad: pad.as_str(),
                    compiler_snippet,
                    without_numbers: Self::without_numbers(format, presentation),
                },
                scratch,
            );
        }
        if compiler_snippet {
            for label in diagnostic
                .labels
                .iter()
                .filter(|label| !label.primary && label.message.is_some())
            {
                self.render_related_note(&mut out, label, context, scratch);
            }
            return out.into_str();
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

    /// Writes `label`, a secondary place in a diagnostic, as GCC and Clang do:
    /// a `note:` of its own, at the place's reported location, over its source
    /// lines when there are any to quote.
    fn render_related_note(
        &self,
        out: &mut ArenaString<'_>,
        label: &Label<'_>,
        context: &Context<'_>,
        scratch: &Bump,
    ) {
        let Some(vector) = label.source.vectors(context).first() else {
            return;
        };
        let format = context.configuration.diagnostic_format();
        let presentation = context.configuration.diagnostic_presentation();
        let segment = self.segment_of(context, vector, [label.source.range(), None]);
        self.write_context_lines(out, context, vector, segment, true);
        let _ = write!(
            out,
            "{}:{}:",
            Self::file_name(context, vector, segment),
            vector.line
        );
        if presentation.column {
            let _ = write!(out, "{}:", Self::location_column(context, format, vector));
        }
        let _ = writeln!(
            out,
            " {} {}",
            self.paint("note:", Style::new().bold()),
            label.message.unwrap_or_default(),
        );
        if !presentation.caret {
            return;
        }
        let mut marks = ArenaVec::new_in(scratch);
        Self::push_label_marks(label, context, &mut marks);
        for mark in &mut marks {
            mark.primary = true;
            mark.label = None;
        }
        let mut files = ArenaVec::new_in(scratch);
        for mark in &marks {
            if !files.contains(&mark.file) {
                files.push(mark.file);
            }
        }
        let last_line = marks.iter().map(|mark| mark.line + 1).max().unwrap_or(1);
        let gutter_width = (last_line.ilog10() as usize + 1).max(5);
        let mut pad = ArenaString::new_in(scratch);
        for _ in 0..gutter_width {
            pad.push(' ');
        }
        for file in files {
            let text = context.source_text(file).unwrap_or_default();
            let starts = context.source_line_starts(file).unwrap_or(&[0]);
            self.render_file_lines(
                out,
                &marks,
                (file, text, starts),
                ErrorSeverity::Warning,
                &Gutter {
                    width:            gutter_width,
                    pad:              pad.as_str(),
                    compiler_snippet: true,
                    without_numbers:  Self::without_numbers(format, presentation),
                },
                scratch,
            );
        }
    }

    /// What stands before quoted source without line numbers: GCC leaves one
    /// space there and Clang none. `None` keeps the numbers.
    const fn without_numbers(
        format: DiagnosticFormat,
        presentation: DiagnosticPresentation,
    ) -> Option<&'static str> {
        match (presentation.line_numbers, format) {
            | (true, _) => None,
            | (false, DiagnosticFormat::Gcc) => Some(" "),
            | (false, _) => Some(""),
        }
    }

    fn render_file_lines(
        &self,
        out: &mut ArenaString<'_>,
        marks: &[Mark<'_>],
        (file, text, starts): (u32, &str, &[usize]),
        severity: ErrorSeverity,
        gutter: &Gutter<'_>,
        scratch: &Bump,
    ) {
        // GCC and Clang mark the rest of a range with tildes.
        let tail = if gutter.compiler_snippet { '~' } else { '^' };
        let bar = self.paint("|", Self::gutter_style());
        // What stands before an underline.
        let lead = match gutter.without_numbers {
            | Some(lead) => lead,
            | None => crate::diagnostics::format_in!(scratch, "{} {bar} ", gutter.pad),
        };
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
                        gutter,
                        scratch,
                    );
                } else if line > previous + 2 {
                    let _ = writeln!(out, "{}", self.paint("...", Self::gutter_style()));
                }
            }
            previous = Some(line);
            let source = line_text(line);
            self.write_source_line(out, line, source, gutter, scratch);
            let line_start = starts[line - 1];

            // Underline row: secondary marks first so primary ones win.
            let column_of = |offset: usize| {
                display_width(
                    &source[..offset.saturating_sub(line_start).min(source.len())],
                    Tabs::of(gutter.compiler_snippet),
                )
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
                        text.push(if *primary { tail } else { '-' }),
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
                    let _ = writeln!(out, "{lead}{underline}");
                },
                | Some((&(_, primary, label), rest)) => {
                    let _ = writeln!(
                        out,
                        "{lead}{underline} {}",
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
                        let _ = writeln!(out, "{lead}{row}");
                        let (mut row, width) = connectors(index);
                        for _ in width..column {
                            row.push(' ');
                        }
                        self.paint_into(&mut row, label, underline_style(primary));
                        let _ = writeln!(out, "{lead}{row}");
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
        gutter: &Gutter<'_>,
        scratch: &Bump,
    ) {
        let visible = visible_source(source, Tabs::of(gutter.compiler_snippet), scratch);
        if let Some(lead) = gutter.without_numbers {
            let _ = writeln!(out, "{lead}{}", visible.as_str().trim_end());
            return;
        }
        let mut number = ArenaString::new_in(scratch);
        let width = gutter.width;
        let _ = write!(number, "{line:>width$}");
        let _ = writeln!(
            out,
            "{} {} {}",
            self.paint(&number, Self::gutter_style()),
            self.paint("|", Self::gutter_style()),
            visible.as_str().trim_end(),
        );
    }

    /// Adds the marks of `label`: one on each physical line it covers, the
    /// first holding its message.
    fn push_label_marks<'a>(
        label: &'a Label<'_>,
        context: &Context<'_>,
        marks: &mut ArenaVec<'_, Mark<'a>>,
    ) {
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

    /// Returns the 1-based physical line containing `offset` and the byte
    /// range of that line, excluding its terminator.
    /// The column a location prints: GCC counts display columns, its
    /// `-fdiagnostics-column-unit=display` default since GCC 11, in which a
    /// tab moves to the next tab stop and a character is one column however
    /// many bytes spell it (GCC manual, "Diagnostic Message Formatting
    /// Options"). Clang and MSVC count bytes, as the vector does.
    fn location_column(
        context: &Context<'_>,
        format: DiagnosticFormat,
        vector: &crate::translation_phases::SourceVector,
    ) -> usize {
        let bytes = vector.column as usize;
        if format != DiagnosticFormat::Gcc {
            return bytes;
        }
        let file = vector.source_file_index;
        let (Some(text), Some(starts)) =
            (context.source_text(file), context.source_line_starts(file))
        else {
            return bytes;
        };
        let offset = (vector.index as usize).min(text.len());
        let (_, line_start, _) = Self::locate(starts, text, offset);
        text.get(line_start..offset).map_or(bytes, |prefix| {
            display_width(prefix, Tabs::Stops(COMPILER_TAB_STOP)) + 1
        })
    }

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
            context_lines: std::cell::Cell::default(),
            included_seen: std::cell::RefCell::new(crate::util::region_bit_set::RegionBitSet::new()),
        }
    }
}

/// What stands to the left of quoted source.
struct Gutter<'a> {
    width:            usize,
    pad:              &'a str,
    compiler_snippet: bool,
    /// See [`Renderer::without_numbers`].
    without_numbers:  Option<&'static str>,
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
fn visible_source<'scratch>(
    text: &str,
    tabs: Tabs,
    scratch: &'scratch Bump,
) -> ArenaString<'scratch> {
    let mut visible = ArenaString::new_in(scratch);
    let mut width = 0;
    for c in text.chars() {
        if c == '\t' {
            let advance = tabs.advance(width);
            for _ in 0..advance {
                visible.push(' ');
            }
            width += advance;
            continue;
        }
        width += char_width(c);
        if let Some(byte) = raw_bytes::raw_byte(c) {
            // A byte of malformed UTF-8 shows as Clang shows it.
            let _ = write!(visible, "<{byte:02X}>");
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

fn display_width(text: &str, tabs: Tabs) -> usize {
    text.chars().fold(0, |width, c| {
        if c == '\t' {
            width + tabs.advance(width)
        } else {
            width + char_width(c)
        }
    })
}

/// Columns a character other than a tab occupies when a line is echoed.
fn char_width(c: char) -> usize {
    if raw_bytes::raw_byte(c).is_some() {
        RAW_BYTE_WIDTH
    } else {
        1
    }
}

/// How a tab is laid out when a source line is echoed. GCC and Clang move to
/// the next multiple of their tab stop, 8 unless `-ftabstop=` says otherwise
/// (GCC manual, "Diagnostic Message Formatting Options"; Clang's
/// `-ftabstop`); bcc's own format gives every tab four columns.
#[derive(Clone, Copy)]
enum Tabs {
    Stops(usize),
    Fixed(usize),
}

impl Tabs {
    /// The columns a tab that starts at column `width` occupies.
    const fn advance(self, width: usize) -> usize {
        match self {
            | Self::Stops(stop) => stop - width % stop,
            | Self::Fixed(columns) => columns,
        }
    }

    const fn of(compiler_snippet: bool) -> Self {
        if compiler_snippet {
            Self::Stops(COMPILER_TAB_STOP)
        } else {
            Self::Fixed(BCC_TAB_WIDTH)
        }
    }
}

/// Width bcc's own format gives a tab when a source line is echoed.
const BCC_TAB_WIDTH: usize = 4;

/// GCC's and Clang's default tab stop.
const COMPILER_TAB_STOP: usize = 8;

/// Width of a malformed-UTF-8 byte echoed as `<FF>`.
const RAW_BYTE_WIDTH: usize = 4;

impl Display for Painted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.color {
            | ColorChoice::Plain => f.write_str(self.text),
            | ColorChoice::Ansi => write!(f, "{}", self.text.style(self.style)),
        }
    }
}
