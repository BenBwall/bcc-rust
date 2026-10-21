//! Phase-7 measurements on caller-provided source, with untimed preparation
//! and optional iterative syntax inspection for differential checks.

use std::{
    path::Path,
    time::Instant,
};

use super::input::benchmark_context;
use crate::{
    pipeline::with_preprocessor,
    translation_phases::{
        Context,
        parsing::{
            InspectionOptions,
            ParsedTranslationUnit,
            Parser,
            PreprocessedTranslationUnit,
        },
    },
    util::{
        bump::Bump,
        vm::accounting,
    },
};

/// Preprocesses `source` before invoking a phase-7 measurement callback.
#[doc(hidden)]
pub fn with_study_source<R>(
    source: &str,
    callback: impl FnOnce(StudyPrepared<'_, '_, '_>) -> R,
) -> R {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = benchmark_context(&tu);
    let preprocessed = with_preprocessor(
        &mut context,
        Path::new("<study>"),
        source,
        crate::headers::HeaderSearch::default(),
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context),
    );
    let parse = Bump::new();
    callback(StudyPrepared {
        context: &mut context,
        preprocessed,
        parse: &parse,
        tu: &tu,
    })
}

impl StudyPrepared<'_, '_, '_> {
    /// Times parser construction and parsing, excluding preparation,
    /// inspection, and destruction. Hooks delimit exactly that interval for
    /// allocation and hardware-counter instrumentation in the executable.
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "Optional benchmark inspection is copied after the timed interval."
    )]
    pub fn measure(
        self,
        inspect: bool,
        begin: impl FnOnce(),
        end: impl FnOnce(),
    ) -> StudyMeasurement {
        let tu_before = self.tu.high_water();
        let live_before = accounting::live();
        accounting::reset_peak();
        begin();
        let start = Instant::now();
        let unit = parser_study_phase7(self.preprocessed, self.context, self.parse);
        let elapsed = start.elapsed();
        end();
        let peak = accounting::peak();
        let inspection = inspect.then(|| {
            let arena = Bump::new();
            unit.inspect(
                &arena,
                self.context,
                InspectionOptions {
                    show_locations: true,
                },
            )
            .to_owned()
        });
        StudyMeasurement {
            nanoseconds: elapsed.as_nanos(),
            roots: unit.external_declarations().len(),
            diagnostics: self.context.pending_error_count(),
            parse_bytes: self.parse.high_water(),
            tu_bytes: self.tu.high_water() - tu_before,
            peak_commit_bytes: peak.committed - live_before.committed,
            peak_regions: peak.regions - live_before.regions,
            inspection,
        }
    }
}

/// Prepared token array and live arenas for one phase-7 measurement.
#[doc(hidden)]
pub struct StudyPrepared<'a, 'tu, 'p> {
    context:      &'a mut Context<'tu>,
    preprocessed: PreprocessedTranslationUnit,
    parse:        &'p Bump,
    tu:           &'tu Bump,
}

/// Parser-only result and exact arena storage use. Inspection is untimed.
#[doc(hidden)]
#[derive(Debug)]
#[expect(
    clippy::disallowed_types,
    reason = "Benchmark output owns optional untimed inspection text."
)]
pub struct StudyMeasurement {
    /// Wall time of parser construction and phase 7.
    pub nanoseconds:       u128,
    /// Retained external declarations.
    pub roots:             usize,
    /// Diagnostics after preprocessing and parsing.
    pub diagnostics:       usize,
    /// Parse-arena high-water mark in bytes, including alignment and spare
    /// capacity.
    pub parse_bytes:       usize,
    /// Translation-unit arena growth during phase 7.
    pub tu_bytes:          usize,
    /// Additional peak arena commit during phase 7 (not process RSS).
    pub peak_commit_bytes: usize,
    /// Additional peak virtual-memory regions during phase 7.
    pub peak_regions:      usize,
    /// Full iterative inspection with source locations, when requested.
    pub inspection:        Option<String>,
}

// Keep a named call boundary for Callgrind's --toggle-collect pattern.
#[inline(never)]
fn parser_study_phase7<'tu>(
    preprocessed: PreprocessedTranslationUnit,
    context: &mut Context<'tu>,
    parse: &Bump,
) -> ParsedTranslationUnit<'tu> {
    Parser::from_preprocessed(preprocessed, context, parse).parse_translation_unit()
}

impl std::fmt::Debug for StudyPrepared<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudyPrepared").finish_non_exhaustive()
    }
}
