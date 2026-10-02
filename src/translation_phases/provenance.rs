//! Source positions, source-vector provenance ranges, and their arenas.

use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
};

use super::{
    GetPosition,
    GetSourceVectors,
    context::Context,
};
use crate::util::{
    shared::SharedString,
    vector_slice::VectorSlice,
};

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

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub(crate) struct SourceVector {
    pub(crate) index:             usize,
    pub(crate) column:            u32,
    pub(crate) line:              u32,
    pub(crate) source_file_index: u32,
    pub(crate) length:            usize,
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
            index:  self.index,
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
            index:  start.index,
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
#[expect(
    clippy::struct_field_names,
    reason = "I think using source and source_file_index as member names is fine here."
)]
pub(crate) struct SourceFile {
    pub(crate) source_file_index: u32,
    pub(crate) source:            SharedString,
    pub(crate) line:              u32,
    pub(crate) column:            u32,
    pub(crate) index:             usize,
}

impl Default for SourceFile {
    fn default() -> Self {
        Self {
            source_file_index: 0,
            source:            SharedString::default(),
            line:              1,
            column:            1,
            index:             0,
        }
    }
}

impl SourceFile {
    pub(crate) fn new(source_file_index: u32, source: SharedString) -> Self {
        Self {
            source_file_index,
            source,
            line: 1,
            column: 1,
            index: 0,
        }
    }
}

impl GetPosition for SourceFile {
    #[inline(always)]
    fn position(&self, _context: &Context) -> SourcePosition {
        SourcePosition {
            index:  self.index,
            line:   self.line,
            column: self.column,
        }
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
    /// One copy of each parser-fetched token's vectors, in fetch order, so
    /// provenance of consecutive tokens is adjacent and merges in O(1).
    ParserTokens,
    /// Parser merges whose operands are not adjacent in one arena.
    ParserMerges,
}

impl SourceArena {
    const INDEX_BITS: u32 = 30;
    pub(super) const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    pub(super) fn decode(source_vectors: SourceVectors) -> (Self, u32) {
        let arena = match source_vectors.start_index >> Self::INDEX_BITS {
            | 0 => Self::Preprocessor,
            | 1 => Self::ParserTokens,
            | 2 => Self::ParserMerges,
            | _ => unreachable!("empty source ranges carry no arena"),
        };
        (arena, source_vectors.start_index & Self::INDEX_MASK)
    }

    #[inline(always)]
    pub(super) fn encode(self, start: u32, end: u32) -> SourceVectors {
        assert!(end <= Self::INDEX_MASK, "source arena overflow");
        let tag = (self as u32) << Self::INDEX_BITS;
        SourceVectors::new(tag | start, tag | end)
    }
}
