//! Fix-it hints as GCC and Clang print them: the line of suggested text
//! under the source line a hint changes, and the `fix-it:` lines of
//! `-fdiagnostics-parseable-fixits`.
//!
//! For `b = ~b;`, Clang replaces the `~` with `!`: it underlines the
//! replaced range beside its caret and writes `!` under the `~`. GCC
//! suggests inserting `!` before the `~`, which it writes at the same
//! column; a replacement it writes under a row of `-` over the replaced
//! range unless its caret line drew exactly that range.

use std::fmt::Write as _;

use super::{
    Renderer,
    model::FixIt,
};
use crate::{
    configuration::DiagnosticFormat,
    translation_phases::Context,
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl Renderer {
    /// The edits of the hints that `note` owns (`None`: the diagnostic
    /// itself), placed in files as `format`'s compiler places them, in
    /// order, an edit that starts where the one before it ends joined to
    /// it; none when that compiler would refuse one of them, which drops
    /// them all (Clang's `mergeFixits` and `EditedSource::applyRewrites`,
    /// GCC's `rich_location::maybe_add_fixit`).
    pub(super) fn place_fix_its<'a>(
        fix_its: &[FixIt<'a>],
        note: Option<usize>,
        format: DiagnosticFormat,
        context: &Context<'_>,
        scratch: &'a Bump,
    ) -> &'a [Edit<'a>] {
        let mut edits = ArenaVec::new_in(scratch);
        for fix_it in fix_its.iter().filter(|fix_it| fix_it.note == note) {
            let placed = match format {
                | DiagnosticFormat::Clang => context.clang_fix_it_bytes(fix_it.place),
                | DiagnosticFormat::Gcc => context.gcc_fix_it_bytes(fix_it.place),
                | _ => return &[],
            };
            let Some(bytes) = placed else {
                return &[];
            };
            edits.push((
                edits.len(),
                Edit {
                    file:  bytes.file,
                    start: bytes.start,
                    end:   bytes.end,
                    line:  bytes.line,
                    text:  fix_it.text,
                },
            ));
        }
        // Edits at one place keep the order they were made in.
        edits.sort_unstable_by_key(|&(order, edit)| (edit.file, edit.start, order));
        let mut joined: ArenaVec<'a, Edit<'a>> = ArenaVec::new_in(scratch);
        for (_, edit) in edits {
            match joined.last_mut() {
                | Some(last) if last.file == edit.file && last.end == edit.start => {
                    last.text = super::format_in!(scratch, "{}{}", last.text, edit.text);
                    last.end = edit.end;
                },
                | _ => joined.push(edit),
            }
        }
        scratch.alloc_slice_fill_iter(joined.iter().copied())
    }

    /// Writes the fix-it lines under the source line that holds the bytes
    /// `line_start..line_end` of the file `file`, whose display columns
    /// `column_of` gives. `lead` stands before Clang's line, and GCC's
    /// margin before each of its lines; `shown` are the byte ranges GCC's
    /// caret line drew there.
    #[expect(
        clippy::too_many_arguments,
        reason = "The line's place, its columns and what stands before it are each needed."
    )]
    pub(super) fn write_fix_it_lines(
        &self,
        out: &mut ArenaString<'_>,
        edits: &[Edit<'_>],
        format: DiagnosticFormat,
        (file, source, line_start, line_end): (u32, &str, usize, usize),
        column_of: &dyn Fn(usize) -> usize,
        (lead, margin): (&str, &str),
        shown: &[(usize, usize)],
        scratch: &Bump,
    ) {
        let on_line = |edit: &&Edit<'_>| {
            edit.file == file
                && edit.start >= line_start
                && edit.start <= line_end
                && !edit.text.contains(['\n', '\r'])
        };
        let mut text = ArenaString::new_in(scratch);
        match format {
            | DiagnosticFormat::Clang => {
                Self::clang_insertion_line(&mut text, edits.iter().filter(on_line), column_of);
                if !text.is_empty() {
                    let _ = writeln!(out, "{lead}{}", self.paint(&text, Self::fix_it_style()));
                }
            },
            | DiagnosticFormat::Gcc => {
                let edits = edits
                    .iter()
                    .filter(on_line)
                    .filter(|edit| edit.end <= line_end);
                let corrections =
                    Self::gcc_corrections(edits, source, line_start, column_of, scratch);
                self.write_gcc_corrections(out, &corrections, shown, margin, scratch);
            },
            | _ => {},
        }
    }

    /// Writes each edit as `-fdiagnostics-parseable-fixits` asks, after the
    /// source of the diagnostic that suggests it:
    /// `fix-it:"file":{line:column-line:column}:"text"`, the range half
    /// open with its columns in bytes, the file's presumed name and the
    /// text with `\`, `"`, tabs, newlines and other unprintable bytes
    /// escaped. GCC counts presumed lines and Clang physical ones (Clang's
    /// `TextDiagnostic::emitParseableFixits`, GCC's `print_parseable_fixits`
    /// in `diagnostic.cc`). `segment` is the reading the diagnostic's
    /// location is in.
    pub(super) fn write_parseable_fix_its(
        out: &mut ArenaString<'_>,
        edits: &[Edit<'_>],
        format: DiagnosticFormat,
        context: &Context<'_>,
        segment: Option<u32>,
        scratch: &Bump,
    ) {
        for edit in edits {
            let file = context.physical_source_file(edit.file);
            let (Some(text), Some(starts)) =
                (context.source_text(file), context.source_line_starts(file))
            else {
                continue;
            };
            let place = |offset: usize| {
                let (line, line_start, _) = Self::locate(starts, text, offset);
                (line, offset - line_start + 1)
            };
            let ((line, column), (end_line, end_column)) = (place(edit.start), place(edit.end));
            // GCC counts presumed lines, Clang physical ones.
            let (line, end_line) = if format == DiagnosticFormat::Gcc {
                let presumed = edit.line as usize;
                (presumed, presumed + end_line - line)
            } else {
                (line, end_line)
            };
            let name = super::format_in!(
                scratch,
                "{}",
                Self::file_name_of(context, edit.file, segment)
            );
            out.push_str("fix-it:");
            write_escaped(out, name);
            let _ = write!(out, ":{{{line}:{column}-{end_line}:{end_column}}}:");
            write_escaped(out, edit.text);
            out.push('\n');
        }
    }

    /// Clang's line of inserted text (`buildFixItInsertionLine` in
    /// `TextDiagnostic.cpp`): each edit's text at the column where it
    /// starts, or one column past the text before it when that text reaches
    /// further. A removal writes nothing.
    fn clang_insertion_line<'e>(
        line: &mut ArenaString<'_>,
        edits: impl Iterator<Item = &'e Edit<'e>>,
        column_of: &dyn Fn(usize) -> usize,
    ) {
        let mut previous_end = 0;
        for edit in edits.filter(|edit| !edit.text.is_empty()) {
            let mut column = column_of(edit.start);
            if column < previous_end {
                column = previous_end + 1;
            }
            // Clang counts the line in bytes and the columns in cells.
            let size = line.len() + (column - previous_end);
            while line.len() < size {
                line.push(' ');
            }
            line.push_str(edit.text);
            previous_end = column + super::render::text_width(edit.text);
        }
    }

    /// GCC's plan for printing the edits on one line (`line_corrections` in
    /// `diagnostic-show-locus.cc`): an edit whose printed text would touch or
    /// overlap the one before it is joined to it, with the source between
    /// them, so the two print as one.
    fn gcc_corrections<'a, 'e>(
        edits: impl Iterator<Item = &'e Edit<'e>>,
        source: &str,
        line_start: usize,
        column_of: &dyn Fn(usize) -> usize,
        scratch: &'a Bump,
    ) -> ArenaVec<'a, Correction<'a>> {
        let mut corrections: ArenaVec<'a, Correction<'a>> = ArenaVec::new_in(scratch);
        for edit in edits {
            let start = column_of(edit.start) + 1;
            let width = super::render::text_width(edit.text);
            let insertion = edit.start == edit.end;
            // The columns the edit affects, with `finish` one before `start`
            // for an insertion, and those its text and range take when
            // printed, both 1-based and inclusive.
            let finish = if insertion {
                start - 1
            } else {
                column_of(edit.end)
            };
            let printed_finish = if insertion {
                start + width - 1
            } else {
                finish.max(start + width - 1)
            };
            if let Some(last) = corrections.last_mut()
                && start <= last.printed.1
                && last.bytes.1 <= edit.start
                && let Some(between) =
                    source.get(last.bytes.1 - line_start..edit.start - line_start)
            {
                let previous = super::render::text_width(&last.text);
                let mut text = ArenaString::new_in(scratch);
                text.push_str(&last.text);
                text.push_str(between);
                text.push_str(edit.text);
                last.text = text;
                last.bytes.1 = edit.end;
                last.columns.1 = finish;
                last.printed.1 += super::render::text_width(&last.text) - previous;
                continue;
            }
            let mut text = ArenaString::new_in(scratch);
            text.push_str(edit.text);
            corrections.push(Correction {
                bytes: (edit.start, edit.end),
                columns: (start, finish),
                printed: (start, printed_finish),
                text,
            });
        }
        corrections
    }

    /// Writes GCC's corrections for one line (`print_trailing_fixits`): an
    /// insertion's text where it goes; a replacement's text where its range
    /// starts, after a row of `-` under that range unless the caret line
    /// drew exactly that range (`shown`, in 1-based display columns); a
    /// removal's `-` alone. Text that would go left of what is already
    /// written starts a new line.
    fn write_gcc_corrections(
        &self,
        out: &mut ArenaString<'_>,
        corrections: &[Correction<'_>],
        shown: &[(usize, usize)],
        margin: &str,
        scratch: &Bump,
    ) {
        if corrections.is_empty() {
            return;
        }
        out.push_str(margin);
        let mut column = 0;
        let move_to = |out: &mut ArenaString<'_>, column: &mut usize, to: usize| {
            if *column > to {
                out.push('\n');
                out.push_str(margin);
                *column = 0;
            }
            while *column < to {
                out.push(' ');
                *column += 1;
            }
        };
        for correction in corrections {
            let (start, finish) = correction.columns;
            let insertion = correction.bytes.0 == correction.bytes.1;
            if !insertion && (!shown.contains(&(start, finish)) || correction.text.is_empty()) {
                move_to(out, &mut column, start);
                let mut dashes = ArenaString::new_in(scratch);
                while column <= finish {
                    dashes.push('-');
                    column += 1;
                }
                self.paint_into(out, &dashes, Self::fix_it_delete_style());
            }
            if !correction.text.is_empty() {
                move_to(out, &mut column, correction.printed.0);
                self.paint_into(out, &correction.text, Self::fix_it_style());
                column += super::render::text_width(&correction.text);
            }
        }
        out.push('\n');
    }
}

/// Writes `text` in double quotes with `\`, `"`, tabs and newlines escaped,
/// and every other unprintable byte as three octal digits, as LLVM's
/// `raw_ostream::write_escaped` and GCC's `print_escaped_string` do.
fn write_escaped(out: &mut ArenaString<'_>, text: &str) {
    out.push('"');
    for &byte in text.as_bytes() {
        match byte {
            | b'\\' => out.push_str("\\\\"),
            | b'\t' => out.push_str("\\t"),
            | b'\n' => out.push_str("\\n"),
            | b'"' => out.push_str("\\\""),
            | 0x20..0x7F => out.push(char::from(byte)),
            | _ => {
                let _ = write!(out, "\\{byte:03o}");
            },
        }
    }
    out.push('"');
}

/// One fix-it hint placed in a file: the bytes `start..end` of `file`, as
/// its locations name it, give way to `text`; an insertion when they are
/// empty.
#[derive(Debug, Clone, Copy)]
pub(super) struct Edit<'a> {
    pub(super) file:  u32,
    pub(super) start: usize,
    pub(super) end:   usize,
    /// The presumed line of `start`.
    pub(super) line:  u32,
    pub(super) text:  &'a str,
}

/// One correction GCC prints on a line: the byte range it changes, its
/// affected and printed display columns, and its text.
struct Correction<'a> {
    bytes:   (usize, usize),
    columns: (usize, usize),
    printed: (usize, usize),
    text:    ArenaString<'a>,
}
