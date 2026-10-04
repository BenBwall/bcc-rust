//! Source positions, source-vector provenance ranges, and their arenas.

use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    ops::Range,
};

use super::{
    GetPosition,
    GetSourceVectors,
    context::Context,
};
use crate::util::vector_slice::VectorSlice;

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub(crate) struct SourcePosition {
    pub(crate) index:  usize,
    pub(crate) line:   u32,
    pub(crate) column: u32,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            index:  0,
            line:   1,
            column: 1,
        }
    }
}

/// One contiguous source segment. Byte offsets and lengths are `u32`, which
/// limits a source file to 4 GiB and keeps the many vectors the parser
/// retains at 20 bytes each.
#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub(crate) struct SourceVector {
    pub(crate) index:             u32,
    pub(crate) column:            u32,
    pub(crate) line:              u32,
    pub(crate) source_file_index: u32,
    pub(crate) length:            u32,
}

impl SourceVector {
    pub(crate) fn new(
        start_position: SourcePosition,
        source_file_index: u32,
        length: usize,
    ) -> Self {
        Self {
            index: source_offset(start_position.index),
            column: start_position.column,
            line: start_position.line,
            source_file_index,
            length: source_offset(length),
        }
    }

    /// The byte offset just past this segment.
    pub(crate) fn end(&self) -> usize {
        self.index as usize + self.length as usize
    }

    /// The source bytes this segment covers.
    pub(crate) fn range(&self) -> Range<usize> {
        self.index as usize..self.end()
    }
}

/// Converts a source byte offset or length to its stored width.
///
/// # Panics
///
/// If a source file exceeds 4 GiB.
pub(crate) fn source_offset(offset: usize) -> u32 {
    u32::try_from(offset).expect("source files larger than 4 GiB are not supported")
}

impl Default for SourceVector {
    fn default() -> Self {
        Self {
            index:             0,
            column:            1,
            line:              1,
            source_file_index: 0,
            length:            0,
        }
    }
}

impl GetPosition for SourceVector {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index as usize,
            line:   self.line,
            column: self.column,
        }
    }
}

pub(crate) type SourceVectors = VectorSlice<SourceVector>;

impl GetPosition for SourceVectors {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        let start = context.first_source_vector(*self);
        SourcePosition {
            index:  start.index as usize,
            line:   start.line,
            column: start.column,
        }
    }
}

impl GetSourceVectors for SourceVectors {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        *self
    }
}

impl GetPosition for SourcePosition {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        *self
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct SourceVectorStack(pub(crate) Vec<SourceVector>);

impl Display for SourceVectorStack {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        for (i, vector) in self.0.iter().enumerate() {
            writeln!(
                f,
                "SourceVector {}: index: {}, line: {}, column: {}, length: {}",
                i, vector.index, vector.line, vector.column, vector.length
            )?;
        }
        Ok(())
    }
}

/// Storage that a [`SourceVectors`] range indexes, encoded in the two high bits
/// of its `start_index`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SourceArena {
    /// Vectors produced by initial processing, tokenization, and
    /// preprocessing.
    Preprocessor,
    /// One copy of each phase-6 output token's vectors, in output order, so
    /// provenance of consecutive tokens is adjacent and merges in O(1).
    ParserTokens,
    /// Provenance that must survive compaction of the preprocessor arena
    /// without being a token copy: parser merges whose operands are not
    /// adjacent in one arena, parser-made locations, and the ranges of
    /// diagnostics still pending when the preprocessor arena is compacted.
    Retained,
}

impl SourceArena {
    const INDEX_BITS: u32 = 30;
    pub(super) const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    pub(super) fn decode(source_vectors: SourceVectors) -> (Self, u32) {
        let arena = match source_vectors.start_index() >> Self::INDEX_BITS {
            | 0 => Self::Preprocessor,
            | 1 => Self::ParserTokens,
            | 2 => Self::Retained,
            | _ => unreachable!("empty source ranges carry no arena"),
        };
        (arena, source_vectors.start_index() & Self::INDEX_MASK)
    }

    #[inline(always)]
    pub(super) fn encode(self, start: u32, end: u32) -> SourceVectors {
        assert!(end <= Self::INDEX_MASK, "source arena overflow");
        let tag = (self as u32) << Self::INDEX_BITS;
        SourceVectors::new(tag | start, tag | end)
    }
}
