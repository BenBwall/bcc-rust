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

use super::Renderer;
use crate::{
    configuration::DiagnosticFormat,
    translation_phases::{
        Context,
        Inclusion,
        ReadingPosition,
        SourceVector,
        SourceVectors,
    },
    util::bump::ArenaString,
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
    /// in `segment`. A related note (`note`) gets GCC's include stack but no
    /// function line, since GCC names the function being compiled when the
    /// note was given, and none of Clang's, which a note only resets.
    pub(super) fn write_context_lines(
        &self,
        out: &mut ArenaString<'_>,
        context: &Context<'_>,
        vector: &SourceVector,
        segment: Option<u32>,
        note: bool,
    ) {
        let Some(segment) = segment else {
            return;
        };
        let map = &context.source_map;
        let (opening, file) = map.opening_of(segment);
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
            FileName {
                path: context.get_source_file(at.file),
                here: includer.here,
            },
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
        let here = context.configuration.diagnostic_format() == DiagnosticFormat::Clang
            && segment.is_some_and(|segment| context.source_map.opening_of(segment).1.here);
        FileName {
            path: context.get_source_file(vector.source_file_index),
            here,
        }
    }
}

/// A file's name as a location writes it.
pub(super) struct FileName<'a> {
    path: &'a Path,
    here: bool,
}

impl Display for FileName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.here {
            f.write_str("./")?;
        }
        write!(f, "{}", self.path.display())
    }
}
