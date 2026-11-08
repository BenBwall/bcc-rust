//! Stage drivers scope preprocessing and parser work arenas to their phases
//! and retain output token ranges before temporary provenance is compacted.

use std::path::Path;

use crate::{
    configuration::DiagnosticFormat,
    headers::HeaderSearch,
    translation_phases::{
        Context,
        TranslationError,
        parsing::{
            ExternalDeclaration,
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

/// Keeps phase-4 working storage within preprocessing: the preprocessing
/// arena is dropped when `run` returns. `run` also receives that arena, so a
/// measurement can read its high-water mark.
pub(crate) fn with_preprocessor<'tu, R>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    search: HeaderSearch<'_>,
    run: impl for<'pp> FnOnce(Preprocessor<'tu, 'pp>, &mut Context<'tu>, &'pp Bump) -> R,
) -> R {
    let pp = Bump::new();
    let preprocessor =
        Preprocessor::new_with_arena_source(&pp, context, source_filename, source, search);
    run(preprocessor, context, &pp)
}

/// As [`with_preprocessor`], recording a trace of phase 4 in `trace`, an
/// arena that holds nothing else, if there is one.
#[cfg(feature = "explorer")]
pub(crate) fn with_traced_preprocessor<'tu, R>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    source: &'tu str,
    search: HeaderSearch<'_>,
    trace: Option<&Bump>,
    run: impl for<'pp> FnOnce(Preprocessor<'tu, 'pp>, &mut Context<'tu>, &'pp Bump) -> R,
) -> R {
    let pp = Bump::new();
    let mut preprocessor =
        Preprocessor::new_with_arena_source(&pp, context, source_filename, source, search);
    if let Some(trace) = trace {
        preprocessor.record_trace(trace);
    }
    run(preprocessor, context, &pp)
}

/// Keeps phase-7 working storage scoped to parsing: the parser's scratch
/// pools, its scopes, and its recovery state come from `parse`, which
/// the caller frees when parsing ends. The syntax tree goes into the
/// translation-unit arena.
pub(crate) fn parse_with_arena<'tu>(
    preprocessed: PreprocessedTranslationUnit,
    context: &mut Context<'tu>,
    parse: &Bump,
) -> ParsedTranslationUnit<'tu> {
    let unit = Parser::from_preprocessed(preprocessed, context, parse).parse_translation_unit();
    record_function_spans(context, &unit);
    unit
}

/// Records where each function definition is, from just past its name
/// through its closing brace, for the `In function 'f':` line GCC prints
/// before the first diagnostic in it (GCC manual, "Diagnostic Message
/// Formatting Options"). GCC's other diagnostics about the declarator, such
/// as `-Wmain`'s or an unused function's, come at top level.
/// C99: function definitions, §6.9.1 paragraph 1, p. 141; PDF p. 153.
fn record_function_spans(context: &mut Context<'_>, unit: &ParsedTranslationUnit<'_>) {
    if context.configuration.diagnostic_format() != DiagnosticFormat::Gcc {
        return;
    }
    let mut last = None;
    for root in unit.external_declarations() {
        let (ExternalDeclaration::FunctionDefinition(definition)
        | ExternalDeclaration::RecoveredFunctionDefinition(definition)) = root
        else {
            continue;
        };
        let Some(identifier) = definition.declarator.identifier() else {
            continue;
        };
        let body = definition.body.source_vectors;
        if identifier.source_vectors.length() == 0 || body.length() == 0 {
            continue;
        }
        let place = |context: &Context<'_>, vector: &crate::translation_phases::SourceVector| {
            (
                context.physical_source_file(vector.source_file_index),
                vector.index,
            )
        };
        let name_at = place(context, context.file_vector(identifier.source_vectors));
        let close = place(
            context,
            context.file_vector(Context::last_source_vector(body)),
        );
        context
            .source_map
            .record_function(identifier.name, name_at, close, &mut last);
    }
}
