//! Replay of preprocessing tokens that phase 4 already produced.
//!
//! C99: translation phase 4 §5.1.1.2p4, p. 10; PDF p. 22; macro rescanning
//! §6.10.3.4p2, p. 155; PDF p. 167.

use super::{
    PreprocessorToken,
    PreprocessorTokenType,
};
use crate::{
    translation_phases::{
        Context,
        SourcePosition,
        SourceVector,
        SourceVectors,
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// A rewindable cursor over preprocessing tokens that phase 4 produced
/// itself, such as a macro argument whose enclosing parameters were already
/// replaced.
///
/// Each token owns its provenance because the preprocessor provenance arena
/// is cleared between iterator items; reading a token pushes it again. A
/// position's `index` counts tokens, so rewinding is exact even when one
/// source token is replayed twice. Its line and column, the current file,
/// and diagnostic locations come from the next token's own provenance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReplayCursor<'a> {
    tokens: &'a [ReplayedToken<'a>],
    /// The zero-length location just after the last token.
    end:    SourceVector,
    next:   usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayedToken<'a> {
    kind:           PreprocessorTokenType,
    contents:       StringCacheId,
    source_vectors: ReplaySources<'a>,
}

/// Most replayed tokens have one contiguous source segment. Keep that
/// segment inline instead of making an allocation for every token.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ReplaySources<'a> {
    Single(SourceVector),
    Multiple(&'a [SourceVector]),
}

impl<'a> ReplaySources<'a> {
    fn new(arena: &'a Bump, sources: &[SourceVector]) -> Self {
        match sources {
            | [source] => Self::Single(source.clone()),
            | _ => Self::Multiple(arena.alloc_slice_fill_iter(sources.iter().cloned())),
        }
    }

    fn as_slice(&self) -> &[SourceVector] {
        match self {
            | Self::Single(source) => std::slice::from_ref(source),
            | Self::Multiple(sources) => sources,
        }
    }
}

impl<'a> ReplayCursor<'a> {
    /// Replays the tokens of `parts` in order, keeping them in `arena`;
    /// `empty_location` locates an empty replay.
    pub(super) fn new(
        context: &Context<'_>,
        arena: &'a Bump,
        parts: &[&[PreprocessorToken]],
        empty_location: SourceVector,
    ) -> Self {
        // The capacity is exact, so allocating a token's sources while the
        // vector fills never moves it.
        let mut tokens =
            ArenaVec::with_capacity_in(parts.iter().map(|part| part.len()).sum(), arena);
        for token in parts.iter().flat_map(|part| part.iter()) {
            tokens.push(ReplayedToken {
                kind:           token.kind,
                contents:       token.contents,
                source_vectors: ReplaySources::new(
                    arena,
                    context.get_source_vectors(token.source_vectors),
                ),
            });
        }
        let tokens: &'a [ReplayedToken<'a>] = tokens.leak();
        let end = tokens
            .iter()
            .rev()
            .find_map(|token| token.source_vectors.as_slice().last())
            .map_or(empty_location, |last| SourceVector {
                index:             last.index + last.length,
                column:            last.column + last.length,
                line:              last.line,
                source_file_index: last.source_file_index,
                length:            0,
            });
        Self {
            tokens,
            end: SourceVector { length: 0, ..end },
            next: 0,
        }
    }

    /// The same replay, with its tokens copied into `arena`.
    pub(super) fn copy_into<'b>(&self, arena: &'b Bump) -> ReplayCursor<'b> {
        let mut tokens = ArenaVec::with_capacity_in(self.tokens.len(), arena);
        for token in self.tokens {
            tokens.push(ReplayedToken {
                kind:           token.kind,
                contents:       token.contents,
                source_vectors: ReplaySources::new(arena, token.source_vectors.as_slice()),
            });
        }
        ReplayCursor {
            tokens: tokens.leak(),
            end:    self.end.clone(),
            next:   self.next,
        }
    }

    /// The zero-length location of the token at `index`, or of the end.
    fn start_of(&self, index: usize) -> SourceVector {
        self.tokens
            .get(index)
            .and_then(|token| token.source_vectors.as_slice().first())
            .map_or_else(
                || self.end.clone(),
                |first| SourceVector {
                    length: 0,
                    ..first.clone()
                },
            )
    }

    pub(super) fn position(&self) -> SourcePosition {
        let start = self.start_of(self.next);
        SourcePosition {
            index:  self.next,
            line:   start.line,
            column: start.column,
        }
    }

    pub(super) fn set_position(&mut self, position: SourcePosition) {
        self.next = position.index.min(self.tokens.len());
    }

    pub(super) fn source_file_index(&self) -> u32 {
        self.start_of(self.next).source_file_index
    }

    /// A zero-length diagnostic location at a position of this cursor.
    pub(super) fn location_at(
        &self,
        context: &mut Context<'_>,
        position: SourcePosition,
    ) -> SourceVectors {
        context.push_source_vectors(&[self.start_of(position.index)])
    }

    pub(super) fn next_item(&mut self, context: &mut Context<'_>) -> Option<PreprocessorToken> {
        let token = self.tokens.get(self.next)?;
        self.next += 1;
        Some(PreprocessorToken {
            kind:           token.kind,
            contents:       token.contents,
            source_vectors: context.push_source_vectors(token.source_vectors.as_slice()),
        })
    }
}
