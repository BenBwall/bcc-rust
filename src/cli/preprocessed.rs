//! Prints the preprocessed translation unit as C source text (`-E`), with
//! GCC's line markers unless `-P` drops them, or the macro definitions in
//! effect at its end (`-dM`). The text itself, shared with the explorer, is
//! written by `pipeline/preprocessed_text.rs`; this file writes it to the
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
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Preprocesses the input and writes its text to `out`. Diagnostics go to
/// stderr. Returns whether an error was reported.
///
/// Each token's text is formatted into one buffer as the token is produced,
/// while its provenance is still current, so no token outlives its turn;
/// the buffer holds the whole text. Diagnostics are reported once
/// preprocessing ends, as the preprocessor may still withdraw or suppress
/// one it queued, and only then is the text written to `out`: [`Step`]
/// records where each diagnostic goes between the 64 KiB pieces of text, so
/// stdout and stderr interleave as they did when every token was kept.
pub(super) fn print_preprocessed<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    options: TextOptions,
    out: &mut dyn Write,
) -> io::Result<bool> {
    let tu = context.tu_arena();
    let text_arena = Bump::new();
    let steps_arena = Bump::new();
    let mut steps = ArenaVec::new_in(&steps_arena);
    let (writer, macros) = with_preprocessor(
        context,
        source_filename,
        input_string,
        search,
        |mut preprocessor, context, _pp| {
            let main_file = context.intern_source_file(source_filename);
            let mut writer = TextWriter::new(&text_arena, options, main_file);
            let mut pragmas = Pragmas::default();
            let mut ordinal = 0;
            let mut diagnosed = 0;
            let mut flushed = 0;
            preprocessor.for_each_iterator_item_read_at(context, |context, token, at| {
                let errors_before = context.pending_error_count();
                if errors_before != diagnosed {
                    steps.push(Step::Report(errors_before));
                    diagnosed = errors_before;
                }
                if !options.macros {
                    pragmas.write_before(context, ordinal, &mut writer);
                    _ = writer.token(context, token, at);
                    if writer.text.len() - flushed >= FLUSH_BYTES {
                        flushed = writer.text.len();
                        steps.push(Step::Flush(flushed));
                    }
                }
                ordinal += 1;
            });
            if !options.macros {
                pragmas.write_before(context, usize::MAX, &mut writer);
                writer.finish();
            }
            let macros = options
                .macros
                .then(|| preprocessor.macro_listing_in(context, tu));
            (writer, macros)
        },
    );
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(&reporter_arena, tu, RenderColor::for_stderr());
    let stderr = &mut io::stderr();
    let mut diagnosed = 0;
    let mut written = 0;
    for step in steps {
        match step {
            | Step::Report(errors_before) =>
                while diagnosed < errors_before {
                    report_next(&mut reporter, context);
                    diagnosed += 1;
                },
            | Step::Flush(end) => {
                out.write_all(&writer.text.as_bytes()[written..end])?;
                written = end;
            },
        }
    }
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, context);
    }
    match macros {
        | Some(macros) => out.write_all(macros.as_bytes())?,
        | None => out.write_all(&writer.text.as_bytes()[written..])?,
    }
    out.flush()?;
    drop(reporter.finish(context, stderr));
    Ok(reporter.errors != 0)
}

/// What [`print_preprocessed`] does at a point in the token stream once
/// preprocessing has ended, in order.
#[derive(Clone, Copy)]
enum Step {
    /// Report diagnostics until this many have been reported: those queued
    /// before the next token was produced.
    Report(usize),
    /// Write the text up to this byte offset.
    Flush(usize),
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
