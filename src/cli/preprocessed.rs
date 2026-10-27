//! Prints the preprocessed translation unit as C source text (`-E`), with
//! GCC's line markers unless `-P` drops them, or the macro definitions in
//! effect at its end (`-dM`). The text itself, shared with the explorer, is
//! written by `pipeline/preprocessed_text.rs`; this file streams it to the
//! output and reports diagnostics in order.
//!
//! C99: translation phases 4-6, §5.1.1.2 paragraph 1 items 4-6, p. 10;
//! PDF p. 22; adjacent string literals, §6.4.5 paragraph 4, p. 62; PDF p. 74.
//! The text form is the GCC and Clang `-E` convention.

use std::{
    io::{
        self,
        Write,
    },
    path::Path,
};

use crate::{
    diagnostics::{
        ColorChoice as RenderColor,
        DiagnosticReporter,
    },
    headers::HeaderSearch,
    pipeline::{
        Pragmas,
        TextOptions,
        TextWriter,
        with_preprocessor,
    },
    translation_phases::{
        Context,
        TranslationError,
    },
    util::{
        bump::Bump,
        region_vec::RegionVec,
    },
};

/// Preprocesses the input and writes its text to `out`. Diagnostics go to
/// stderr. Returns whether an error was reported.
pub(super) fn print_preprocessed<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    options: TextOptions,
    out: &mut dyn Write,
) -> io::Result<bool> {
    let tu = context.tu_arena();
    let (items, macros) = with_preprocessor(
        context,
        source_filename,
        input_string,
        search,
        |mut preprocessor, context, _pp| {
            let mut tokens = RegionVec::new();
            preprocessor.for_each_iterator_item_read_at(context, |context, mut token, at| {
                token.source_vectors = context.retain_token_source(token.source_vectors);
                tokens.push((token, at, context.pending_error_count()));
            });
            let macros = options
                .macros
                .then(|| preprocessor.macro_listing_in(context, tu));
            (tokens, macros)
        },
    );
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(&reporter_arena, tu, RenderColor::for_stderr());
    let stderr = &mut io::stderr();
    let text_arena = Bump::new();
    let main_file = context.intern_source_file(source_filename);
    let mut writer = TextWriter::new(&text_arena, options, main_file);
    let mut diagnosed = 0;
    let mut pragmas = Pragmas::default();
    for (ordinal, (token, at, errors_before)) in items.into_iter().enumerate() {
        while diagnosed < errors_before {
            report_next(&mut reporter, context);
            diagnosed += 1;
        }
        if macros.is_none() {
            pragmas.write_before(context, ordinal, &mut writer);
            writer.token(context, token, at);
            if writer.text.len() >= FLUSH_BYTES {
                out.write_all(writer.text.as_bytes())?;
                writer.text.clear();
            }
        }
    }
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, context);
    }
    if let Some(macros) = macros {
        out.write_all(macros.as_bytes())?;
    } else {
        pragmas.write_before(context, usize::MAX, &mut writer);
        writer.finish();
        out.write_all(writer.text.as_bytes())?;
    }
    out.flush()?;
    drop(reporter.finish(context, stderr));
    Ok(reporter.errors != 0)
}

/// Reports the next pending diagnostic.
fn report_next(reporter: &mut DiagnosticReporter<'_, '_>, context: &mut Context<'_>) {
    let error: TranslationError<'_> = context
        .pop_pending_error()
        .expect("counted diagnostics are pending");
    reporter.report(&error, context);
}

/// Text accumulated before it is written out.
const FLUSH_BYTES: usize = 1 << 16;
