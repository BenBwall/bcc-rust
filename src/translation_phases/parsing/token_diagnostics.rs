use std::cell::Cell;

use super::{
    ParseFrameKind,
    Parser,
};
use crate::{
    translation_phases::{
        SourceVector,
        preprocessing::{
            KeywordTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaQueue,
    },
};

impl Parser<'_, '_, '_> {
    /// Advances the diagnostic occurrence cursor even on recovery skips.
    pub(super) fn suppress_token_diagnostic(&mut self, token: Token) {
        if self.token_diagnostics.is_empty() {
            return;
        }
        match token.kind {
            | TokenType::Keyword(_) => {
                self.suppress_diagnostic_occurrence(token, None);
            },
            | TokenType::Integer(_) | TokenType::Float(_) => {
                for spelling in [
                    "imaginary constant",
                    "long long integer constant",
                    "hexadecimal floating constant",
                    "binary integer constant",
                ]
                .into_iter()
                .chain(super::msvc::CONSTANT_DIAGNOSTICS)
                {
                    self.suppress_diagnostic_occurrence(token, Some(spelling));
                }
            },
            | _ => {},
        }
    }

    pub(super) fn suppress_diagnostic_occurrence(&mut self, token: Token, spelling: Option<&str>) {
        let spelling = spelling.unwrap_or_else(|| self.context.string_cache.at(token.contents));
        let vectors = self.context.get_source_vectors(token.source_vectors);
        let user_end = self.context.user_source_end(token.source_vectors);
        // A diagnostic about a macro argument can lack the invocation that
        // the token carries; the argument's spelling already places it.
        let occurrence = [user_end, vectors.last().cloned()]
            .into_iter()
            .find_map(|user_end| {
                self.token_diagnostics
                    .get_mut(&DiagnosticLookup {
                        spelling,
                        vectors,
                        user_end,
                    })
                    .and_then(ArenaQueue::pop_front)
            });
        match occurrence {
            | Some(TokenDiagnostic::Extension(suppressed))
                if self.pedantic_suppression != 0
                    || matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) =>
                self.context.suppress_extension(suppressed),
            | Some(TokenDiagnostic::Constant(index)) if self.active_frame == ParseFrameKind::Msvc =>
                self.context.withdraw_pending_error(index),
            | _ => {},
        }
    }
}

/// Each spelling/provenance pair owns its FIFO of diagnostic occurrences.
/// Macro replacements can share provenance, so occurrences must not be merged.
/// C99: §5.1.1.3, p. 11; PDF p. 23. GNU extension markers suppress only
/// diagnostics in their owning parser frame, without reordering the queue.
pub(super) type TokenDiagnostics<'tu, 'p> = ArenaMap<
    'p,
    (&'tu str, &'p [SourceVector], Option<SourceVector>),
    ArenaQueue<'p, TokenDiagnostic<'tu>>,
>;

/// One pending diagnostic that the frame consuming its token may withdraw.
#[derive(Debug, Clone, Copy)]
pub(super) enum TokenDiagnostic<'tu> {
    /// An extension diagnostic, suppressed through its marker.
    Extension(&'tu Cell<bool>),
    /// A phase-7 constant-conversion error at its pending-queue position,
    /// which MSVC assembly withdraws (see
    /// [`super::msvc::constant_diagnostic`]).
    Constant(usize),
}

/// A borrowed lookup keeps temporary context borrows out of stored key
/// lifetimes.
#[derive(Hash)]
struct DiagnosticLookup<'a> {
    spelling: &'a str,
    vectors:  &'a [SourceVector],
    user_end: Option<SourceVector>,
}

impl hashbrown::Equivalent<(&str, &[SourceVector], Option<SourceVector>)> for DiagnosticLookup<'_> {
    fn equivalent(&self, key: &(&str, &[SourceVector], Option<SourceVector>)) -> bool {
        self.spelling == key.0 && self.vectors == key.1 && self.user_end == key.2
    }
}
