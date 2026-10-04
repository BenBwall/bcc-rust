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
            PreprocessedTranslationUnit,
        },
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::{
        bump::Bump,
        region_vec::RegionVec,
    },
};

/// Completes preprocessing before constructing the parser. The parser-facing
/// token stream owns its region and lives until phase 7 ends.
pub(crate) fn parse_translation_unit<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
) -> ParsedTranslationUnit {
    let preprocessed = with_preprocessor(
        context,
        source_filename,
        source,
        quote_include,
        system_include,
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context),
    );
    let parse = Bump::new();
    parse_with_arena(preprocessed, context, &parse)
}

/// Keeps phase-4 working storage within preprocessing. The arena is passed to
/// the phase callback so stage 6 can move its state without changing callers.
pub(crate) fn with_preprocessor<'tu, R>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
    run: impl for<'pp> FnOnce(Preprocessor<'tu, 'pp>, &mut Context<'tu>, &'pp Bump) -> R,
) -> R {
    let pp = Bump::new();
    let preprocessor = Preprocessor::new_with_arena_source(
        &pp,
        context,
        source_filename,
        source,
        quote_include,
        system_include,
    );
    run(preprocessor, context, &pp)
}

/// Keeps phase-7 working storage scoped to parsing: the parser's frames,
/// their pools, its scopes, and its recovery state come from `parse`, which
/// the caller frees when parsing ends.
pub(crate) fn parse_with_arena(
    preprocessed: PreprocessedTranslationUnit,
    context: &mut Context<'_>,
    parse: &Bump,
) -> ParsedTranslationUnit {
    Parser::from_preprocessed(preprocessed, parse).parse_translation_unit(context)
}

/// Runs translation phases 4 through 6 over the whole translation unit and
/// returns its parser-facing tokens with their diagnostics, each diagnostic
/// before the token whose production reported it.
///
/// Token provenance is retained in the translation-unit context as each
/// token is produced, and pending diagnostics keep theirs across
/// preprocessor-arena compaction, so every item stays renderable afterwards.
pub(crate) fn preprocess_with_diagnostics<'tu>(
    mut preprocessor: Preprocessor<'tu, '_>,
    context: &mut Context<'tu>,
) -> RegionVec<Result<Token, TranslationError<'tu>>> {
    let mut tokens = RegionVec::new();
    preprocessor.for_each_iterator_item(context, |context, mut token| {
        token.source_vectors = context.retain_token_source(token.source_vectors);
        tokens.push((token, context.pending_error_count()));
    });
    let mut items = RegionVec::new();
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
