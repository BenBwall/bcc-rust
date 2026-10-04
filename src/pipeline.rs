//! Runs the translation phases over one translation unit in order: each
//! source file is lexed completely when it is opened, the whole unit is
//! preprocessed, and only then is it parsed.

#[cfg(test)]
mod tests;

use crate::{
    translation_phases::{
        Context,
        TranslationError,
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Runs translation phases 4 through 6 over the whole translation unit and
/// returns its parser-facing tokens with their diagnostics, each diagnostic
/// before the token whose production reported it.
///
/// Token provenance is copied to the token arena as each token is produced,
/// and pending diagnostics keep theirs across preprocessor-arena compaction,
/// so every item stays renderable afterwards.
pub(crate) fn preprocess_with_diagnostics<'tok>(
    mut preprocessor: Preprocessor,
    context: &mut Context<'_>,
    tok: &'tok Bump,
) -> ArenaVec<'tok, Result<Token, TranslationError>> {
    let mut tokens = ArenaVec::new_in(tok);
    while let Some(mut token) = preprocessor.next_iterator_item(context) {
        token.source_vectors = context.retain_token_source(token.source_vectors);
        tokens.push((token, context.pending_error_count()));
    }
    let mut items = ArenaVec::new_in(tok);
    let mut reported = 0;
    for (token, errors_before) in tokens {
        while reported < errors_before {
            items.push(Err(context
                .pop_pending_error()
                .expect("counted diagnostics are pending")));
            reported += 1;
        }
        items.push(Ok(token));
    }
    items.extend(std::iter::from_fn(|| context.pop_pending_error()).map(Err));
    items
}
