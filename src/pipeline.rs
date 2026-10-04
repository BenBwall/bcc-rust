//! Runs the translation phases over one translation unit in order: each
//! source file is lexed completely when it is opened, the whole unit is
//! preprocessed, and only then is it parsed.

use std::path::{
    Path,
    PathBuf,
};

#[cfg(test)]
mod tests;

use crate::{
    translation_phases::{
        Context,
        TranslationError,
        parsing::{
            ParsedTranslationUnit,
            Parser,
        },
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::bump::{
        Bump,
        RegionVec,
    },
};

/// Completes preprocessing before constructing the parser. The token arena
/// outlives phase 7 and is released before the translation-unit context.
pub(crate) fn parse_translation_unit<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
) -> ParsedTranslationUnit {
    let tok = Bump::new();
    let preprocessed = with_preprocessor(
        context,
        source_filename,
        source,
        quote_include,
        system_include,
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context, &tok),
    );
    let parse = Bump::new();
    let unit = parse_with_arena(Parser::from_preprocessed(preprocessed), context, &parse);
    drop(parse);
    drop(tok);
    unit
}

/// Keeps phase-4 working storage within preprocessing. The arena is passed to
/// the phase callback so stage 6 can move its state without changing callers.
pub(crate) fn with_preprocessor<'tu, R>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
    run: impl FnOnce(Preprocessor, &mut Context<'tu>, &Bump) -> R,
) -> R {
    let pp = Bump::new();
    let preprocessor = Preprocessor::new_with_arena_source(
        context,
        source_filename,
        source,
        quote_include,
        system_include,
    );
    run(preprocessor, context, &pp)
}

/// Keeps phase-7 working storage scoped to parsing. Stage 7 will allocate the
/// parser's frames and scopes from this arena.
pub(crate) fn parse_with_arena(
    parser: Parser<'_>,
    context: &mut Context<'_>,
    _parse: &Bump,
) -> ParsedTranslationUnit {
    parser.parse_translation_unit(context)
}

/// Runs translation phases 4 through 6 over the whole translation unit and
/// returns its parser-facing tokens with their diagnostics, each diagnostic
/// before the token whose production reported it.
///
/// Token provenance is copied to the token arena as each token is produced,
/// and pending diagnostics keep theirs across preprocessor-arena compaction,
/// so every item stays renderable afterwards.
pub(crate) fn preprocess_with_diagnostics(
    mut preprocessor: Preprocessor,
    context: &mut Context<'_>,
    _tok: &Bump,
) -> RegionVec<Result<Token, TranslationError>> {
    let mut tokens = RegionVec::new_in(Bump::new());
    while let Some(mut token) = preprocessor.next_iterator_item(context) {
        token.source_vectors = context.retain_token_source(token.source_vectors);
        tokens.push((token, context.pending_error_count()));
    }
    let mut items = RegionVec::new_in(Bump::new());
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
