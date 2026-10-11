//! Public compilation entry points report the time spent reading,
//! preprocessing, and parsing files to a caller-provided callback.

use std::{
    io::{
        self,
        Write,
    },
    path::Path,
};

use clap::Parser;

use super::{
    Cli,
    arguments::normalize_language_arguments,
    diagnostic_reporter::DiagnosticReporter,
};
use crate::{
    configuration::CompilerConfiguration,
    diagnostics::ColorChoice as RenderColor,
    headers::HeaderSearch,
    pipeline::parse_translation_unit,
    translation_phases::Context,
    util::bump::Bump,
};

/// Measures compilation with CLI language and startup preprocessing arguments.
/// Argument parsing runs before either measured interval. Include-path and
/// output options are not applied by this diagnostic measurement adapter.
///
/// # Errors
///
/// Invalid CLI arguments, reading `path`, or writing to `out` can fail.
#[doc(hidden)]
#[expect(
    clippy::disallowed_types,
    reason = "CLI argument normalization owns OS strings before measured compiler intervals."
)]
pub fn compile_file_with_arguments_measured(
    path: &Path,
    arguments: &[&str],
    out: &mut dyn Write,
    measure: impl FnMut(CompileStep, &mut dyn FnMut()),
) -> io::Result<()> {
    let args = Cli::try_parse_from(normalize_language_arguments(
        ["bcc-rust", "--input", ""]
            .into_iter()
            .chain(arguments.iter().copied())
            .map(std::ffi::OsString::from),
    ))
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    compile_file_configured_measured(path, args.configuration(), out, measure, |context| {
        args.configure_preprocessing(context);
    })
}

/// Compiles the file at `path` in the library's default mode, writing its
/// diagnostics and their summary to `out` without color. Each step runs
/// inside `measure`, so a caller can observe one step alone; the allocation
/// tests count the global allocations each makes.
///
/// # Errors
///
/// Reading `path` or writing to `out` can fail.
#[doc(hidden)]
pub fn compile_file_measured(
    path: &Path,
    out: &mut dyn Write,
    measure: impl FnMut(CompileStep, &mut dyn FnMut()),
) -> io::Result<()> {
    compile_file_configured_measured(path, CompilerConfiguration::default(), out, measure, |_| {})
}

fn compile_file_configured_measured(
    path: &Path,
    configuration: CompilerConfiguration,
    out: &mut dyn Write,
    mut measure: impl FnMut(CompileStep, &mut dyn FnMut()),
    configure: impl FnOnce(&mut Context<'_>),
) -> io::Result<()> {
    let tu = Bump::new();
    let source = tu.read_to_str_lossy(path)?;
    let mut context = Context::with_configuration(&tu, configuration);
    configure(&mut context);
    measure(CompileStep::Parse, &mut || {
        let unit = parse_translation_unit(&mut context, path, source, HeaderSearch::default());
        let _semantic = crate::pipeline::analyze_translation_unit(&mut context, &unit);
    });
    let mut result = Ok(());
    measure(CompileStep::Report, &mut || {
        let reporter_arena = Bump::new();
        let mut reporter = DiagnosticReporter::new(&reporter_arena, &tu, RenderColor::Plain);
        result = reporter
            .report_pending(&mut context, out)
            .and_then(|()| reporter.finish(&context, out));
    });
    result
}

/// A step of [`compile_file_measured`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileStep {
    /// Preprocessing and parsing, which record the diagnostics.
    Parse,
    /// Building, folding, ordering, and rendering the recorded diagnostics.
    Report,
}
