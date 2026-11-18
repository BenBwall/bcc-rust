//! Where GCC and Clang put a fix-it hint in a file: the bytes of a token
//! range, or the place before or after it, followed out of the macro
//! expansions that produced its tokens as far as each compiler follows them.
//!
//! C99: macro replacement, §6.10.3 paragraph 10, p. 152; PDF p. 164;
//! argument substitution, §6.10.3.1 paragraph 1, p. 153; PDF p. 165.

use super::Context;
use crate::{
    diagnostics::FixItPlace,
    translation_phases::provenance::{
        SourceArena,
        SourceVector,
        SourceVectors,
    },
};

impl Context<'_> {
    /// Where Clang's edit at `place` falls in a file: the bytes from the
    /// start of the range's first token to the end of its last, or the
    /// empty place before the first or after the last. `None` when Clang's
    /// `edit::Commit` refuses the edit, which drops every hint of the
    /// diagnostic (`mergeFixits` in `DiagnosticRenderer.cpp`).
    ///
    /// A token a file spells stays where it is. A token a macro expansion
    /// produced moves out to the start of the outermost invocation when it
    /// begins the expansion, and to the end of the invocation when it ends
    /// it (`Lexer::isAtStartOfMacroExpansion`, `isAtEndOfMacroExpansion`);
    /// otherwise a token of an argument stays where the invocation spells
    /// it (`SourceManager::getTopMacroCallerLoc`), and a replacement keeps
    /// both ends there only when one substitution of an argument produced
    /// them (`Lexer::makeFileCharRange`). A token a replacement list wrote
    /// anywhere else has no place, nor has any place in a system header.
    ///
    /// An insertion after a range is placed before the edit is made, after
    /// the last token (`Lexer::getLocForEndOfToken`): a token a macro
    /// expansion produced has that place only at the end of the expansion,
    /// and otherwise the hint is left out alone ([`FixItPlacement::Dropped`]).
    ///
    /// Only the outermost invocation of a token is recorded, so a token at
    /// the start of an inner expansion within an outer one counts as being
    /// at the start of the outer one only if it begins that too.
    pub(crate) fn clang_fix_it_bytes(&self, place: FixItPlace) -> FixItPlacement {
        // `getLocForEndOfToken` follows a token a macro expansion produced
        // out only from the expansion's end.
        if let FixItPlace::After(source) = place
            && source.length() != 0
            && self
                .invocation_span(Self::last_source_vector(source))
                .is_some()
            && !self.at_expansion_edge(Self::last_source_vector(source), true)
        {
            return FixItPlacement::Dropped;
        }
        self.clang_fix_it_place(place)
            .map_or(FixItPlacement::Refused, FixItPlacement::Placed)
    }

    /// [`Self::clang_fix_it_bytes`] for a hint that has a place.
    fn clang_fix_it_place(&self, place: FixItPlace) -> Option<FixItBytes> {
        let (source, start, end) = Self::fix_it_edges(place);
        if source.length() == 0 {
            return None;
        }
        let first = Self::first_token_vector(source);
        let last = Self::last_source_vector(source);
        let replace = start && end;
        let (from, to) = match (start, end) {
            | (true, false) => {
                let at = self.clang_edit_point(first, false, None)?;
                (at, at)
            },
            | (false, true) => {
                let at = self.clang_edit_point(last, true, None)?;
                (at, at)
            },
            | _ => (
                self.clang_edit_point(first, false, replace.then_some(last))?,
                self.clang_edit_point(last, true, replace.then_some(first))?,
            ),
        };
        (from.file == to.file && from.start <= to.start).then_some(FixItBytes {
            end: to.start,
            ..from
        })
    }

    /// Where the edge of the one-token range `token` falls for Clang's
    /// edit: its start, or with `after` its end. `other` is the token at the
    /// other end of a range the edit replaces, whose argument substitution
    /// must then be this one's for an argument's token to keep its place.
    fn clang_edit_point(
        &self,
        token: SourceVectors,
        after: bool,
        other: Option<SourceVectors>,
    ) -> Option<FixItBytes> {
        let vector = self.first_source_vector(token);
        let place = |vector: &SourceVector| {
            (!self.in_system_header(vector)).then(|| {
                let byte = if after {
                    vector.end()
                } else {
                    vector.index as usize
                };
                FixItBytes {
                    file:  vector.source_file_index,
                    start: byte,
                    end:   byte,
                    line:  vector.line,
                }
            })
        };
        let Some((name, last)) = self.invocation_span(token) else {
            return place(vector);
        };
        // A replacement takes the whole invocation only when its other end
        // reaches the invocation's other end too.
        let edge = self.at_expansion_edge(token, after)
            && other.is_none_or(|other| self.at_expansion_edge(other, !after));
        if edge {
            return place(if after { last } else { name });
        }
        let same_argument = other.is_none_or(|other| {
            self.argument_substitution(other) == self.argument_substitution(token)
                && self.invocation_span(other) == Some((name, last))
        });
        if self.argument_substitution(token).is_some()
            && !self.written_by_macro_body(token)
            && same_argument
        {
            return place(vector);
        }
        None
    }

    /// Whether the outermost invocation that produced the one-token range
    /// `token` produced no token before it (or with `after`, after it) in
    /// the parser's tokens. A token of another arena, which holds copies
    /// out of order, is taken to be inside its expansion.
    fn at_expansion_edge(&self, token: SourceVectors, after: bool) -> bool {
        let (arena, index) = SourceArena::decode(token);
        if arena != SourceArena::ParserTokens {
            return false;
        }
        let neighbor = if after {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        let Some(neighbor) =
            neighbor.filter(|&neighbor| (neighbor as usize) < self.parser_token_vectors.len())
        else {
            return true;
        };
        let neighbor = arena.encode(neighbor, neighbor + 1);
        self.invocation_span(neighbor) != self.invocation_span(token)
    }

    /// Where GCC's fix-it hint at `place` falls in a file, as
    /// [`Self::clang_fix_it_bytes`] gives Clang's: the tokens where the file
    /// spells them. `None` when a macro expansion produced either end, a
    /// virtual location GCC's `rich_location::reject_impossible_fixit`
    /// refuses, which drops every hint of the diagnostic.
    pub(crate) fn gcc_fix_it_bytes(&self, place: FixItPlace) -> FixItPlacement {
        self.gcc_fix_it_place(place)
            .map_or(FixItPlacement::Refused, FixItPlacement::Placed)
    }

    /// [`Self::gcc_fix_it_bytes`] for a hint GCC keeps.
    fn gcc_fix_it_place(&self, place: FixItPlace) -> Option<FixItBytes> {
        let (source, start, end) = Self::fix_it_edges(place);
        if source.length() == 0 {
            return None;
        }
        let first = Self::first_token_vector(source);
        let last = Self::last_source_vector(source);
        if self.produced_by_macro_expansion(first) || self.produced_by_macro_expansion(last) {
            return None;
        }
        let (first, last) = (
            self.first_source_vector(first),
            self.first_source_vector(last),
        );
        let (from, to) = match (start, end) {
            | (true, false) => (first.index as usize, first.index as usize),
            | (false, true) => (last.end(), last.end()),
            | _ => (first.index as usize, last.end()),
        };
        let line = if start { first.line } else { last.line };
        (first.source_file_index == last.source_file_index && from <= to).then_some(FixItBytes {
            file: first.source_file_index,
            start: from,
            end: to,
            line,
        })
    }

    /// The range of `place` and which of its edges it holds: the start for
    /// an insertion before it, the end for one after it, both for a
    /// replacement.
    const fn fix_it_edges(place: FixItPlace) -> (SourceVectors, bool, bool) {
        match place {
            | FixItPlace::Before(source) => (source, true, false),
            | FixItPlace::After(source) => (source, false, true),
            | FixItPlace::Replace(source) => (source, true, true),
        }
    }
}

/// Where a fix-it hint falls in a file: the bytes `start..end` of `file`,
/// the source file as its locations name it, which `#line` may have
/// renamed, and the presumed line its first byte is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixItBytes {
    pub(crate) file:  u32,
    pub(crate) start: usize,
    pub(crate) end:   usize,
    pub(crate) line:  u32,
}

/// What a compiler does with a fix-it hint: place it, leave it out alone
/// (`Dropped`), or refuse it and with it every hint of its diagnostic
/// (`Refused`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixItPlacement {
    Placed(FixItBytes),
    Dropped,
    Refused,
}
