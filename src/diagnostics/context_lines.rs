//! The lines GCC and Clang print before a diagnostic to say where it is:
//! the `#include`s that led to its file, and, in GCC's format, the function
//! it is in. Each is printed only when it differs from what the last
//! diagnostic said.
//!
//! ```text
//! In file included from a.h:1,
//!                  from main.c:2:
//! b.h: In function 'f':
//! b.h:3:5: error: ...
//! ```
//!
//! GCC manual, "Diagnostic Message Formatting Options" (and its
//! `diagnostic_report_current_module`); Clang's
//! `DiagnosticRenderer::emitIncludeStack`.
//!
//! C99: the form of a diagnostic is implementation-defined, §5.1.1.3
//! paragraph 1, p. 11; PDF p. 23; includes, §6.10.2 paragraph 3, p. 149;
//! PDF p. 161.

use std::{
    fmt::{
        self,
        Display,
        Write as _,
    },
    path::Path,
};

use super::{
    Diagnostic,
    Renderer,
    model::{
        Label,
        LabelSource,
    },
};
use crate::{
    configuration::DiagnosticFormat,
    translation_phases::{
        Context,
        Inclusion,
        ReadingPosition,
        SourceVector,
        SourceVectors,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// What GCC and Clang last said about where diagnostics are.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ContextLines {
    /// The function GCC last named, `None` at top level.
    function: Option<u32>,
    /// The opening whose include stack Clang printed last, `None` for the
    /// main file (Clang's `LastIncludeLoc`).
    included: Option<u32>,
    /// Where the last located diagnostic or note was read.
    place:    Option<ReadingPosition>,
}

impl Renderer {
    /// Writes the context lines for a diagnostic at `vector`, which was read
    /// in `segment`, with the include stack of stretch `stack`: GCC's for a
    /// diagnostic in a macro's definition is the definition's. A related
    /// note (`note`) gets GCC's include stack but no function line, since
    /// GCC names the function being compiled when the note was given, and
    /// none of Clang's, which a note only resets.
    pub(super) fn write_context_lines(
        &self,
        out: &mut ArenaString<'_>,
        context: &Context<'_>,
        (vector, segment): (&SourceVector, Option<u32>),
        stack: Option<u32>,
        note: bool,
    ) {
        let (Some(segment), Some(stack)) = (segment, stack) else {
            return;
        };
        let map = &context.source_map;
        let (opening, file) = map.opening_of(stack);
        let mut state = self.context_lines.get();
        match context.configuration.diagnostic_format() {
            | DiagnosticFormat::Gcc => {
                // GCC 14 writes each `#include` once in a compilation:
                // the chain stops at the first it wrote before (its
                // `includes_seen_p`).
                let mut seen = self.included_seen.borrow_mut();
                let mut inclusion = file.includer.map(|at| (opening, at));
                let mut first = true;
                while let Some((opening, at)) = inclusion
                    && !seen.contains(opening as usize)
                {
                    seen.insert(opening as usize);
                    let _ = write!(
                        out,
                        "{}{} {}:{}",
                        if first { "" } else { ",\n" },
                        if first {
                            "In file included from"
                        } else {
                            "                 from"
                        },
                        context.get_source_file(at.file).display(),
                        at.line,
                    );
                    first = false;
                    inclusion = map
                        .opening(at.opening)
                        .includer
                        .map(|outer| (at.opening, outer));
                }
                if !first {
                    out.push_str(":\n");
                }
                let function = map.function_at((segment, vector.index));
                if !note && function != state.function {
                    state.function = function;
                    let name =
                        function.map(|index| context.string_cache.at(map.function(index).name));
                    let _ = match name {
                        | Some(name) => writeln!(
                            out,
                            "{}: In function '{name}':",
                            context.get_source_file(vector.source_file_index).display()
                        ),
                        | None => writeln!(
                            out,
                            "{}: At top level:",
                            context.get_source_file(vector.source_file_index).display()
                        ),
                    };
                }
            },
            | DiagnosticFormat::Clang => {
                let included = file.includer.map(|_| opening);
                if state.included != included {
                    state.included = included;
                    if !note && let Some(at) = file.includer {
                        Self::write_clang_include(out, context, at.opening, at);
                    }
                }
            },
            | _ => {},
        }
        self.context_lines.set(state);
    }

    /// Writes Clang's `In file included from` lines for the `#include` `at`,
    /// in `opening`, outermost first.
    fn write_clang_include(
        out: &mut ArenaString<'_>,
        context: &Context<'_>,
        opening: u32,
        at: Inclusion,
    ) {
        let includer = context.source_map.opening(opening);
        if let Some(outer) = includer.includer {
            Self::write_clang_include(out, context, outer.opening, outer);
        }
        let _ = writeln!(
            out,
            "In file included from {}:{}:",
            FileName::new(context, at.file, includer.here),
            at.line,
        );
    }

    /// The stretch of source `vector` was read in: the one the first of
    /// `sources` that is a parser token's range was read in, or else the
    /// one that holds `vector` after the place the last diagnostic was in.
    /// `sources` are the ranges a diagnostic was reported about, which say
    /// which reading of a file read twice it is in.
    pub(super) fn segment_of(
        &self,
        context: &Context<'_>,
        vector: &SourceVector,
        sources: [Option<SourceVectors>; 2],
    ) -> Option<u32> {
        let mut state = self.context_lines.get();
        let file = context.physical_source_file(vector.source_file_index);
        let segment = sources
            .into_iter()
            .flatten()
            .find_map(|source| context.reading_segment(source, file, vector.index))
            .or_else(|| {
                context
                    .source_map
                    .segment_at(file, vector.index, state.place)
            })?;
        state.place = Some((segment, vector.index));
        self.context_lines.set(state);
        Some(segment)
    }

    /// The name a location gives the file `vector` is in, read in `segment`:
    /// Clang begins a header found beside an includer in the working
    /// directory with `./`.
    pub(super) fn file_name<'c>(
        context: &'c Context<'_>,
        vector: &SourceVector,
        segment: Option<u32>,
    ) -> FileName<'c> {
        Self::file_name_of(context, vector.source_file_index, segment)
    }

    /// The name a location gives the source file `file`, read in
    /// `segment`, as [`Self::file_name`] gives it.
    pub(super) fn file_name_of<'c>(
        context: &'c Context<'_>,
        file: u32,
        segment: Option<u32>,
    ) -> FileName<'c> {
        let here = context.configuration.diagnostic_format() == DiagnosticFormat::Clang
            && segment.is_some_and(|segment| context.source_map.opening_of(segment).1.here);
        FileName::new(context, file, here)
    }
}

/// A diagnostic about a token a macro's replacement list wrote: where the
/// list wrote it, where the outermost invocation that produced it begins,
/// the macro whose definition holds the token, and the invoked one's name.
pub(super) struct MacroExpansion<'c> {
    pub(super) spelling: SourceVector,
    site:                SourceVector,
    defined:             &'c str,
    invoked:             &'c str,
}

impl Renderer {
    /// The macro expansion that wrote the token `origin`, a diagnostic's
    /// range as reported, starts with, when the diagnostic's place
    /// `location` is the invocation (GCC manual, "Diagnostic Message
    /// Formatting Options", `-ftrack-macro-expansion`).
    pub(super) fn macro_expansion<'c>(
        context: &'c Context<'_>,
        origin: SourceVectors,
        location: &SourceVector,
    ) -> Option<MacroExpansion<'c>> {
        let (spelling, site) = context.macro_body_expansion(origin)?;
        if site.source_file_index != location.source_file_index || site.index != location.index {
            return None;
        }
        let invoked = context.outermost_macro_name(origin)?;
        let defined = macro_defined_at(context, spelling)?;
        Some(MacroExpansion {
            spelling: spelling.clone(),
            site: site.clone(),
            defined,
            invoked,
        })
    }

    /// GCC's `in expansion of macro 'X'` note at the invocation, or Clang's
    /// `expanded from macro 'X'` at the replacement list. Only the
    /// outermost invocation and the innermost list are known, so a nested
    /// expansion's other levels are not noted. Clang draws the ranges
    /// `diagnostic` names that the same expansion wrote where its list
    /// wrote them, beside a caret that is then a point.
    pub(super) fn render_expansion_note(
        &self,
        out: &mut ArenaString<'_>,
        expansion: &MacroExpansion<'_>,
        diagnostic: &Diagnostic<'_>,
        context: &Context<'_>,
        scratch: &Bump,
    ) {
        let gcc = context.configuration.diagnostic_format() == DiagnosticFormat::Gcc;
        let (place, message) = if gcc {
            (
                &expansion.site,
                crate::diagnostics::format_in!(
                    scratch,
                    "in expansion of macro '{}'",
                    expansion.invoked
                ),
            )
        } else {
            (
                &expansion.spelling,
                crate::diagnostics::format_in!(
                    scratch,
                    "expanded from macro '{}'",
                    expansion.defined
                ),
            )
        };
        let mut highlights = ArenaVec::new_in(scratch);
        let mut highlighted = false;
        for label in diagnostic
            .labels
            .iter()
            .filter(|label| !gcc && label.is_highlight())
        {
            highlighted = true;
            let Some(source) = label.source.range() else {
                continue;
            };
            // A range that ends outside the expansion, in the file after
            // the invocation, cannot be brought into the definition, and
            // Clang's note draws none of it (`mapDiagnosticRanges`).
            if let Some((_, site)) = context.macro_body_expansion(source)
                && site.source_file_index == expansion.site.source_file_index
                && site.index == expansion.site.index
                && context.invocation_span(Context::last_source_vector(source))
                    == context.invocation_span(source)
            {
                // The definition holds the range; what the arguments wrote
                // of it is in the invocation, which Clang's note does not
                // draw.
                let invocation = context
                    .invocation_span(source)
                    .map(|(start, end)| (start.source_file_index, start.index..=end.index));
                let written = scratch.alloc_slice_fill_iter(
                    context
                        .get_source_vectors(source)
                        .iter()
                        .filter(|vector| {
                            invocation.as_ref().is_none_or(|(file, span)| {
                                vector.source_file_index != *file || !span.contains(&vector.index)
                            })
                        })
                        .cloned(),
                );
                highlights.push(Label {
                    source:  LabelSource::Segments(written),
                    message: None,
                    primary: false,
                });
            }
        }
        let label = Label {
            source:  LabelSource::Segments(scratch.alloc_slice_fill_iter([place.clone()])),
            message: Some(message),
            primary: false,
        };
        let point = highlighted || (!gcc && diagnostic.point);
        self.render_related_note(
            out,
            (&label, &highlights, &[]),
            point,
            context,
            scratch,
            !gcc,
        );
    }
}

/// The name of the macro whose definition holds `vector`: the identifier
/// after `#define` (or `%:define`) on the logical line it is on. The
/// preprocessor's macro table is gone by the time diagnostics are written, so
/// the name is read again from the source text.
/// C99: §6.10.3 paragraph 10, p. 152; PDF p. 164; §6.4.6 paragraph 3, p. 64;
/// PDF p. 76.
pub(crate) fn macro_defined_at<'c>(
    context: &'c Context<'_>,
    vector: &SourceVector,
) -> Option<&'c str> {
    let file = vector.source_file_index;
    let text = context.source_text(file)?;
    let starts = context.source_line_starts(file)?;
    let offset = (vector.index as usize).min(text.len());
    let mut line = starts
        .partition_point(|&start| start <= offset)
        .checked_sub(1)?;
    // The lines a backslash continues belong to the line before.
    while line > 0
        && text[starts[line - 1]..starts[line]]
            .trim_end_matches(['\n', '\r'])
            .ends_with('\\')
    {
        line -= 1;
    }
    let identifier = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let rest = skip_blanks(&text[starts[line]..]);
    let rest = skip_blanks(rest.strip_prefix('#').or_else(|| rest.strip_prefix("%:"))?);
    let rest = rest.strip_prefix("define")?;
    if rest.starts_with(identifier) {
        return None;
    }
    let rest = skip_blanks(rest);
    let end = rest.find(|c: char| !identifier(c)).unwrap_or(rest.len());
    (end != 0).then(|| &rest[..end])
}

/// `text` after the blanks that may separate a directive's tokens: spaces
/// and tabs, comments, which translation phase 3 replaces with a space, and
/// line splices, which phase 2 deletes.
/// C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22; §6.10 paragraph 5,
/// p. 147; PDF p. 159.
fn skip_blanks(mut text: &str) -> &str {
    loop {
        let blank = text.trim_start_matches([' ', '\t', '\x0b', '\x0c']);
        text = if let Some(comment) = blank.strip_prefix("/*") {
            comment.find("*/").map_or("", |end| &comment[end + 2..])
        } else if let Some(spliced) = blank
            .strip_prefix("\\\n")
            .or_else(|| blank.strip_prefix("\\\r\n"))
        {
            spliced
        } else {
            return blank;
        };
    }
}

/// A file's name as a location writes it.
pub(super) struct FileName<'a> {
    path: &'a Path,
    here: bool,
}

impl<'a> FileName<'a> {
    /// The name of source file `file`, which Clang begins with `./` when
    /// `here` says its header was found beside an includer in the working
    /// directory: Clang's `FileManager` names that directory `.` and
    /// `HeaderSearch` appends the name to it with the host's separator. A
    /// name `#line` gave is the presumed name exactly as written.
    /// C99: §6.10.4 paragraph 4, p. 158; PDF p. 170.
    fn new(context: &'a Context<'_>, file: u32, here: bool) -> Self {
        Self {
            path: context.get_source_file(file),
            here: here && context.physical_source_file(file) == file,
        }
    }
}

impl Display for FileName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.here {
            write!(f, ".{}", std::path::MAIN_SEPARATOR)?;
        }
        write!(f, "{}", self.path.display())
    }
}
