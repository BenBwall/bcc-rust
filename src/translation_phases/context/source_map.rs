//! Where each stretch of the source was read, for the lines GCC and Clang
//! print before a diagnostic: `In file included from a.h:1,` for each
//! `#include` that led to it, and GCC's `In function 'f':` when the function
//! it is in differs from the last one named (GCC manual, "Diagnostic Message
//! Formatting Options"; Clang's `DiagnosticRenderer::emitIncludeStack`).
//!
//! A source file read twice is two openings, each with the `#include` that
//! opened it. An opening is read in stretches, split where it includes
//! another file, as GCC's line maps split it; a diagnostic's file and byte
//! offset find the stretch it was read in.
//!
//! C99: includes, §6.10.2 paragraph 3, p. 149; PDF p. 161; the form of a
//! diagnostic is implementation-defined, §5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23.

use super::Context;
use crate::{
    translation_phases::{
        SourceVectors,
        provenance::SourceArena,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl Context<'_> {
    /// Records that the `#include` at `directive` (its file as named, byte
    /// offset and line), or the command line when `None`, opened physical
    /// file `file`. `here` is [`FileOpening::here`].
    pub(crate) fn enter_source_file(
        &mut self,
        file: u32,
        directive: Option<(u32, u32, u32)>,
        here: bool,
    ) {
        let tokens = Self::token_count(self.parser_token_vectors.len());
        self.source_map.enter(file, directive, here, tokens);
    }

    /// Records that the end of physical file `file` was read.
    pub(crate) fn leave_source_file(&mut self, file: u32) {
        let tokens = Self::token_count(self.parser_token_vectors.len());
        self.source_map.leave(file, tokens);
    }

    /// The stretch of source that `source`, a diagnostic's range, was read
    /// in when it is a parser token's, whose order says which reading of a
    /// file read twice it is, and when that stretch holds `offset` of
    /// physical file `file`, where the diagnostic points.
    pub(crate) fn reading_segment(
        &self,
        source: SourceVectors,
        file: u32,
        offset: u32,
    ) -> Option<u32> {
        if source.length() == 0 {
            return None;
        }
        let (arena, start) = SourceArena::decode(source);
        if arena != SourceArena::ParserTokens {
            return None;
        }
        self.source_map
            .segment_of_token(start)
            .filter(|&segment| self.source_map.holds(segment, file, offset))
    }

    /// Whether two diagnostic ranges can be one mistake: not when they are
    /// parser tokens read in different readings of a file.
    pub(crate) fn same_reading(&self, left: SourceVectors, right: SourceVectors) -> bool {
        let segment = |source: SourceVectors| {
            if source.length() == 0 {
                return None;
            }
            let (arena, start) = SourceArena::decode(source);
            (arena == SourceArena::ParserTokens)
                .then(|| self.source_map.segment_of_token(start))
                .flatten()
                .map(|segment| self.source_map.opening_of(segment).0)
        };
        match (segment(left), segment(right)) {
            | (Some(left), Some(right)) => left == right,
            | _ => true,
        }
    }

    fn token_count(length: usize) -> u32 {
        u32::try_from(length).unwrap_or(u32::MAX)
    }
}

/// The openings of the translation unit's source files, the stretches of
/// text read from each, and the span of each function definition.
pub(crate) struct SourceMap<'tu> {
    openings:  ArenaVec<'tu, FileOpening>,
    segments:  ArenaVec<'tu, FileSegment>,
    /// The openings being read, innermost last.
    open:      ArenaVec<'tu, u32>,
    functions: ArenaVec<'tu, FunctionSpan>,
}

/// One opening of a source file: the main file, or a file an `#include`
/// entered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FileOpening {
    /// The physical source file read.
    pub(crate) file:     u32,
    /// The `#include` that opened it, `None` for the main file and the
    /// headers the command line includes.
    pub(crate) includer: Option<Inclusion>,
    /// Found beside an includer in the working directory, or one reached
    /// from it that way, which Clang names `./a.h`.
    pub(crate) here:     bool,
}

/// Where an `#include` directive is: the opening that holds it, and its file
/// (as `#line` names it) and line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Inclusion {
    pub(crate) opening: u32,
    pub(crate) file:    u32,
    pub(crate) line:    u32,
    /// The directive's byte offset, where reading the includer resumes.
    resume:             u32,
}

/// Bytes `start..end` of an opening's file, read without entering another
/// file.
#[derive(Clone, Copy, Debug)]
struct FileSegment {
    opening:     u32,
    start:       u32,
    end:         u32,
    /// How many parser-token vectors came before the first token read here.
    first_token: u32,
}

/// A place in reading order: the stretch it was read in, and its byte offset
/// in that stretch's file. Later places compare greater.
pub(crate) type ReadingPosition = (u32, u32);

/// A function definition, from just past its name, which GCC reports at top
/// level, through its closing brace.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FunctionSpan {
    pub(crate) name: StringCacheId,
    after:           ReadingPosition,
    through:         ReadingPosition,
}

impl<'tu> SourceMap<'tu> {
    /// The stretch that holds byte `offset` of physical file `file`, read
    /// after `last`, the place found before. A file read more than once
    /// holds the offset in several stretches; diagnostics come in reading
    /// order, so a place at or before `last` in its stretch is taken to be
    /// in a later reading of the file when there is one.
    pub(crate) fn segment_at(
        &self,
        file: u32,
        offset: u32,
        last: Option<ReadingPosition>,
    ) -> Option<u32> {
        let holds = |&index: &usize| {
            u32::try_from(index).is_ok_and(|index| self.holds(index, file, offset))
        };
        let (hint, last_offset) = last.map_or((0, None), |(segment, offset)| {
            ((segment as usize).min(self.segments.len()), Some(offset))
        });
        let later = |&index: &usize| index > hint || last_offset.is_none_or(|last| offset > last);
        (hint..self.segments.len())
            .filter(later)
            .find(holds)
            .or_else(|| (hint..self.segments.len()).find(holds))
            .or_else(|| (0..hint).rev().find(holds))
            .map(|index| u32::try_from(index).expect("segments are counted in u32"))
    }

    /// The stretch the parser token whose first vector is `token` was read
    /// in: the last to begin before it.
    fn segment_of_token(&self, token: u32) -> Option<u32> {
        let index = self
            .segments
            .partition_point(|segment| segment.first_token <= token)
            .checked_sub(1)?;
        u32::try_from(index).ok()
    }

    /// Whether stretch `segment` holds byte `offset` of physical file `file`.
    fn holds(&self, segment: u32, file: u32, offset: u32) -> bool {
        let segment = &self.segments[segment as usize];
        self.openings[segment.opening as usize].file == file
            && segment.start <= offset
            && offset < segment.end
    }

    /// The opening a stretch was read in, with its index.
    pub(crate) fn opening_of(&self, segment: u32) -> (u32, FileOpening) {
        let opening = self.segments[segment as usize].opening;
        (opening, self.openings[opening as usize])
    }

    pub(crate) fn opening(&self, opening: u32) -> FileOpening {
        self.openings[opening as usize]
    }

    /// The function definition that holds `position`, by its index.
    pub(crate) fn function_at(&self, position: ReadingPosition) -> Option<u32> {
        let index = self
            .functions
            .partition_point(|function| function.after < position)
            .checked_sub(1)?;
        (self.functions[index].through >= position)
            .then(|| u32::try_from(index).expect("functions are counted in u32"))
    }

    pub(crate) fn function(&self, index: u32) -> FunctionSpan {
        self.functions[index as usize]
    }

    /// Records a function definition named `name` whose name is at `name_at`
    /// and whose closing brace is at `close`, each a physical file and byte
    /// offset. Definitions are recorded in reading order; `last` carries the
    /// place found last from one to the next.
    pub(crate) fn record_function(
        &mut self,
        name: StringCacheId,
        name_at: (u32, u32),
        close: (u32, u32),
        last: &mut Option<ReadingPosition>,
    ) {
        let Some(after) = self.segment_at(name_at.0, name_at.1, *last) else {
            return;
        };
        let Some(through) = self.segment_at(close.0, close.1, Some((after, name_at.1))) else {
            return;
        };
        *last = Some((through, close.1));
        let span = FunctionSpan {
            name,
            after: (after, name_at.1),
            through: (through, close.1),
        };
        // Nested and recovered definitions can break the order; such a
        // definition is not named.
        if self
            .functions
            .last()
            .is_none_or(|last| last.through < span.after)
            && span.after <= span.through
        {
            self.functions.push(span);
        }
    }

    /// Starts reading the main source file, `file`.
    pub(crate) fn open_main(&mut self, file: u32) {
        if self.openings.is_empty() {
            self.openings.push(FileOpening {
                file,
                includer: None,
                here: false,
            });
            self.segments.push(FileSegment {
                opening:     0,
                start:       0,
                end:         u32::MAX,
                first_token: 0,
            });
            self.open.push(0);
        }
    }

    /// Starts reading `file`, which the `#include` at `directive` (its file
    /// as named, byte offset and line) opened, or the command line when
    /// `None`. `here` is [`FileOpening::here`]; `tokens` counts the
    /// parser-token vectors so far.
    fn enter(&mut self, file: u32, directive: Option<(u32, u32, u32)>, here: bool, tokens: u32) {
        let includer = directive.and_then(|(directive_file, offset, line)| {
            let &opening = self.open.last()?;
            if let Some(segment) = self
                .segments
                .iter_mut()
                .rev()
                .find(|segment| segment.opening == opening)
            {
                segment.end = segment.end.min(offset).max(segment.start);
            }
            Some(Inclusion {
                opening,
                file: directive_file,
                line,
                resume: offset,
            })
        });
        let opening = u32::try_from(self.openings.len()).expect("openings are counted in u32");
        self.openings.push(FileOpening {
            file,
            includer,
            here,
        });
        self.segments.push(FileSegment {
            opening,
            start: 0,
            end: u32::MAX,
            first_token: tokens,
        });
        self.open.push(opening);
    }

    /// Whether the innermost opening read now was found beside the working
    /// directory, as [`FileOpening::here`] says.
    pub(crate) fn reading_here(&self) -> bool {
        self.open
            .last()
            .is_some_and(|&opening| self.openings[opening as usize].here)
    }

    /// Stops reading `file`, a physical source file whose end was reached,
    /// and resumes its includer after the `#include`.
    fn leave(&mut self, file: u32, tokens: u32) {
        let Some(&opening) = self.open.last() else {
            return;
        };
        if self.open.len() < 2 || self.openings[opening as usize].file != file {
            return;
        }
        _ = self.open.pop();
        if let Some(includer) = self.openings[opening as usize].includer {
            self.segments.push(FileSegment {
                opening:     includer.opening,
                start:       includer.resume,
                end:         u32::MAX,
                first_token: tokens,
            });
        }
    }

    pub(crate) fn new_in(tu: &'tu Bump) -> Self {
        Self {
            openings:  ArenaVec::new_in(tu),
            segments:  ArenaVec::new_in(tu),
            open:      ArenaVec::new_in(tu),
            functions: ArenaVec::new_in(tu),
        }
    }
}
