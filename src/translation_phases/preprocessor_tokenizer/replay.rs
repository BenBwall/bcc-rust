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

impl<'a> ReplayCursor<'a> {
    pub(super) fn next_item(&mut self, context: &mut Context<'_>) -> Option<PreprocessorToken> {
        let token = self.get(self.next())?;
        self.next += 1;
        Some(token.read(context))
    }

    /// Reads the next token without recording its provenance, which stays
    /// with the token in this replay; see [`SharedToken`].
    pub(super) fn next_shared(&mut self) -> Option<SharedToken<'a>> {
        let token = match self.next().checked_sub(self.tokens.len()) {
            | Some(index) => SharedToken {
                tokens: self.tail(),
                index,
            },
            | None => SharedToken {
                tokens: self.tokens,
                index:  self.next(),
            },
        };
        if token.index >= token.tokens.len() {
            return None;
        }
        self.next += 1;
        Some(token)
    }

    /// Moves past the tokens after `open`, the token just read with
    /// [`Self::next_shared`], up to the one `offset` tokens after it, and
    /// returns them; `None`, reading nothing, unless that token is among
    /// `tokens`.
    pub(super) fn skip_shared(
        &mut self,
        open: SharedToken<'a>,
        offset: usize,
    ) -> Option<SharedRun<'a>> {
        let end = open.index + offset;
        if !std::ptr::eq(open.tokens, self.tokens)
            || self.next() != open.index + 1
            || end >= self.tokens.len()
        {
            return None;
        }
        self.next = position_index(end);
        Some(SharedRun {
            tokens: self.tokens,
            start: open.index + 1,
            end,
        })
    }

    /// The kind and spelling of each token left, without reading it.
    pub(super) fn spellings(
        &self,
    ) -> impl Iterator<Item = (PreprocessorTokenType, StringCacheId)> + '_ {
        let start = self.next().min(self.tokens.len());
        let tail = self.tail();
        let tail_start = self
            .next()
            .saturating_sub(self.tokens.len())
            .min(tail.len());
        self.tokens[start..]
            .iter()
            .chain(&tail[tail_start..])
            .map(|token| (token.kind, token.contents))
    }
}

/// A rewindable cursor over preprocessing tokens that phase 4 produced
/// itself, such as a macro argument whose enclosing parameters were already
/// replaced.
///
/// Each token owns its provenance because the preprocessor provenance arena
/// is cleared between iterator items; reading a token pushes it again. A
/// position's `index` counts tokens, so rewinding is exact even when one
/// source token is replayed twice. Its line and column, the current file,
/// and diagnostic locations come from the next token's own provenance.
///
/// Replays share tokens rather than copy them where they can: a macro
/// argument collected from another replay is a slice of that replay's
/// tokens, with its call's closing parenthesis in `tail`. An argument nested
/// in an argument then costs its call's delimiters, not its length.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReplayCursor<'a> {
    tokens: &'a [ReplayedToken<'a>],
    /// Replayed after `tokens`. One token at most, so that the cursor, which
    /// every frame of the reader holds, stays as small as one without it.
    tail:   Option<&'a ReplayedToken<'a>>,
    /// The zero-length location just after the last token.
    end:    SourceVector,
    next:   u32,
}

// Every reader frame holds a cursor and frames are copied as calls are
// collected, so sharing tokens must not make the cursor larger.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<ReplayCursor<'static>>() == 48,
    "a replay cursor stays six words"
);

/// One token of a replay, with the provenance that reading it records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReplayedToken<'a> {
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

/// A token read from a replay without recording its provenance. It names
/// its place in the replay, so that a macro call collected from replayed
/// tokens can share them instead of copying them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SharedToken<'a> {
    tokens: &'a [ReplayedToken<'a>],
    index:  usize,
}

/// Consecutive tokens of one replay, shared by reference.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SharedRun<'a> {
    tokens: &'a [ReplayedToken<'a>],
    start:  usize,
    end:    usize,
}

/// Tokens read with their provenance, copied as they come into a run of
/// their own.
pub(crate) struct CopiedRun<'a> {
    tokens: ArenaVec<'a, ReplayedToken<'a>>,
}

impl<'a> ReplayCursor<'a> {
    /// The zero-length location of the token at `index`, or of the end.
    fn start_of(&self, index: usize) -> SourceVector {
        self.get(index)
            .and_then(|token| token.source_vectors.as_slice().first())
            .map_or_else(
                || self.end.clone(),
                |first| SourceVector {
                    length: 0,
                    ..first.clone()
                },
            )
    }

    /// The token at `index`, counting `tail` after `tokens`.
    #[inline(always)]
    fn get(&self, index: usize) -> Option<&'a ReplayedToken<'a>> {
        match self.tokens.get(index) {
            | Some(token) => Some(token),
            | None => self.tail().get(index - self.tokens.len()),
        }
    }

    /// The token `tail` holds, as a slice.
    fn tail(&self) -> &'a [ReplayedToken<'a>] {
        self.tail.map_or(&[], std::slice::from_ref)
    }

    /// The index of the next token.
    fn next(&self) -> usize {
        self.next as usize
    }

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
            tokens.push(ReplayedToken::new(context, arena, token));
        }
        Self::shared(tokens.leak(), &[], empty_location)
    }

    /// Replays `runs` in order and then `tail`. A single run is replayed
    /// where it is; several are copied into `arena` together, each token
    /// sharing its provenance. `empty_location` locates an empty replay.
    pub(super) fn of_runs(
        arena: &'a Bump,
        runs: &[SharedRun<'a>],
        tail: SharedRun<'a>,
        empty_location: SourceVector,
    ) -> Self {
        let tokens = match runs {
            | [] => &[][..],
            | [run] => run.as_slice(),
            | _ => {
                let mut tokens = ArenaVec::with_capacity_in(
                    runs.iter().map(|run| run.end - run.start).sum(),
                    arena,
                );
                for run in runs {
                    tokens.extend_from_slice(run.as_slice());
                }
                tokens.leak()
            },
        };
        Self::shared(tokens, tail.as_slice(), empty_location)
    }

    /// Replays `tokens` and then `tail`, one token at most, where they are;
    /// `empty_location` locates an empty replay.
    fn shared(
        tokens: &'a [ReplayedToken<'a>],
        tail: &'a [ReplayedToken<'a>],
        empty_location: SourceVector,
    ) -> Self {
        debug_assert!(
            tail.len() <= 1,
            "a replay's tail is one closing parenthesis"
        );
        let end = tail
            .iter()
            .rev()
            .chain(tokens.iter().rev())
            .find_map(|token| token.source_vectors.as_slice().last())
            .map_or(empty_location, SourceVector::end_anchor);
        Self {
            tokens,
            tail: tail.first(),
            end: SourceVector { length: 0, ..end },
            next: 0,
        }
    }

    /// The same replay, with its tokens copied into `arena`.
    pub(super) fn copy_into<'b>(&self, arena: &'b Bump) -> ReplayCursor<'b> {
        let mut tokens = ArenaVec::with_capacity_in(self.len(), arena);
        for token in self.tokens.iter().chain(self.tail) {
            tokens.push(ReplayedToken {
                kind:           token.kind,
                contents:       token.contents,
                source_vectors: ReplaySources::new(arena, token.source_vectors.as_slice()),
            });
        }
        ReplayCursor {
            tokens: tokens.leak(),
            tail:   None,
            end:    self.end.clone(),
            next:   self.next,
        }
    }

    fn len(&self) -> usize {
        self.tokens.len() + usize::from(self.tail.is_some())
    }

    pub(super) fn is_exhausted(&self) -> bool {
        self.next() == self.len()
    }

    pub(super) fn position(&self) -> SourcePosition {
        let start = self.start_of(self.next());
        SourcePosition {
            index:  self.next(),
            line:   start.line,
            column: start.column,
        }
    }

    pub(super) fn set_position(&mut self, position: SourcePosition) {
        self.next = position_index(position.index.min(self.len()));
    }

    pub(super) fn source_file_index(&self) -> u32 {
        self.start_of(self.next()).source_file_index
    }

    /// A zero-length diagnostic location at a position of this cursor.
    pub(super) fn location_at(
        &self,
        context: &mut Context<'_>,
        position: SourcePosition,
    ) -> SourceVectors {
        context.push_source_vectors(&[self.start_of(position.index)])
    }
}

/// A replay position as its cursor keeps it.
fn position_index(index: usize) -> u32 {
    u32::try_from(index).expect("a replay holds fewer than 2^32 tokens")
}

impl<'a> ReplayedToken<'a> {
    /// `token`, with its provenance copied into `arena`.
    fn new(context: &Context<'_>, arena: &'a Bump, token: &PreprocessorToken) -> Self {
        Self {
            kind:           token.kind,
            contents:       token.contents,
            source_vectors: ReplaySources::new(
                arena,
                context.get_source_vectors(token.source_vectors),
            ),
        }
    }

    /// The token, with its provenance pushed again.
    fn read(&self, context: &mut Context<'_>) -> PreprocessorToken {
        PreprocessorToken {
            kind:           self.kind,
            contents:       self.contents,
            source_vectors: context.push_source_vectors(self.source_vectors.as_slice()),
        }
    }
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

impl SharedToken<'_> {
    pub(crate) fn kind(self) -> PreprocessorTokenType {
        self.tokens[self.index].kind
    }

    /// The token's spelling, as [`PreprocessorToken::contents`].
    pub(crate) fn contents(self) -> StringCacheId {
        self.tokens[self.index].contents
    }

    /// The token as reading it from its replay gives it, recording its
    /// provenance.
    pub(crate) fn read(self, context: &mut Context<'_>) -> PreprocessorToken {
        self.tokens[self.index].read(context)
    }

    /// Where the token is in memory, which identifies it whichever replay
    /// shares it.
    pub(crate) fn address(self) -> usize {
        std::ptr::from_ref(&self.tokens[self.index]).addr()
    }
}

impl<'a> SharedRun<'a> {
    /// The run of `token` alone.
    pub(crate) fn new(token: SharedToken<'a>) -> Self {
        Self {
            tokens: token.tokens,
            start:  token.index,
            end:    token.index + 1,
        }
    }

    /// The run without its last token, and that token as a run of its own.
    pub(crate) fn split_last(self) -> (Self, Self) {
        let last = self.end - 1;
        (
            Self { end: last, ..self },
            Self {
                start: last,
                ..self
            },
        )
    }

    pub(crate) fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Adds `token` when it follows the run in the same replay.
    pub(crate) fn extend(&mut self, token: SharedToken<'a>) -> bool {
        let follows = std::ptr::eq(self.tokens, token.tokens) && token.index == self.end;
        self.end += usize::from(follows);
        follows
    }

    /// Adds `run` when it follows this one in the same replay.
    pub(crate) fn append(&mut self, run: Self) -> bool {
        let follows = std::ptr::eq(self.tokens, run.tokens) && run.start == self.end;
        if follows {
            self.end = run.end;
        }
        follows
    }

    /// How many tokens after `open` the run has `close`, when it has both.
    pub(crate) fn distance(self, open: SharedToken<'a>, close: SharedToken<'a>) -> Option<usize> {
        let has = |token: SharedToken<'a>| {
            std::ptr::eq(self.tokens, token.tokens) && (self.start..self.end).contains(&token.index)
        };
        (has(open) && has(close) && open.index < close.index).then(|| close.index - open.index)
    }

    /// The kind of each token of the run.
    pub(crate) fn kinds(self) -> impl Iterator<Item = PreprocessorTokenType> + 'a {
        self.as_slice().iter().map(|token| token.kind)
    }

    fn as_slice(self) -> &'a [ReplayedToken<'a>] {
        &self.tokens[self.start..self.end]
    }
}

impl<'a> CopiedRun<'a> {
    pub(crate) fn new(arena: &'a Bump) -> Self {
        Self {
            tokens: ArenaVec::new_in(arena),
        }
    }

    /// Copies `token` and its provenance.
    pub(crate) fn push(&mut self, context: &Context<'_>, token: &PreprocessorToken) {
        let arena = *self.tokens.allocator();
        self.tokens.push(ReplayedToken::new(context, arena, token));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The tokens copied, as a run. Capacity left over returns to the arena
    /// when nothing was allocated after the tokens.
    pub(crate) fn finish(mut self) -> SharedRun<'a> {
        self.tokens.shrink_to_fit();
        let tokens: &'a [ReplayedToken<'a>] = self.tokens.leak();
        SharedRun {
            tokens,
            start: 0,
            end: tokens.len(),
        }
    }
}
