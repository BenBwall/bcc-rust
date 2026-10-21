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
    pub fn measure(
        self,
        inspect: bool,
        begin: impl FnOnce(),
        end: impl FnOnce(),
    ) -> StudyMeasurement {
        self.measure_with_mode(ParserStudyMode::Machine, false, inspect, begin, end)
    }

    /// Selects an experimental parser and optional untimed stack-address
    /// sampling. Recursive modes require `parser-recursive-descent-study`.
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "Optional benchmark inspection is copied after the timed interval."
    )]
    pub fn measure_with_mode(
        self,
        mode: ParserStudyMode,
        sample_stack: bool,
        inspect: bool,
        begin: impl FnOnce(),
        end: impl FnOnce(),
    ) -> StudyMeasurement {
        let tu_before = self.tu.high_water();
        let live_before = accounting::live();
        accounting::reset_peak();
        let rss_before = peak_rss();
        begin();
        let start = Instant::now();
        let (unit, stats) = parser_study_phase7(
            self.preprocessed,
            self.context,
            self.parse,
            mode,
            sample_stack,
        );
        let elapsed = start.elapsed();
        end();
        let rss_after = peak_rss();
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
            expression_entries: stats.expression_entries,
            statement_entries: stats.statement_entries,
            compound_entries: stats.compound_entries,
            max_depth: stats.max_depth,
            native_stack_span: stats.native_stack_span,
            fallback_count: stats.fallback_count,
            depth_limit_hits: stats.depth_limit_hits,
            peak_rss_before: rss_before,
            peak_rss_after: rss_after,
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
    pub nanoseconds:        u128,
    /// Retained external declarations.
    pub roots:              usize,
    /// Diagnostics after preprocessing and parsing.
    pub diagnostics:        usize,
    /// Parse-arena high-water mark in bytes, including alignment and spare
    /// capacity.
    pub parse_bytes:        usize,
    /// Translation-unit arena growth during phase 7.
    pub tu_bytes:           usize,
    /// Additional peak arena commit during phase 7 (not process RSS).
    pub peak_commit_bytes:  usize,
    /// Additional peak virtual-memory regions during phase 7.
    pub peak_regions:       usize,
    /// Full iterative inspection with source locations, when requested.
    pub inspection:         Option<String>,
    /// Number of recursive expression entries.
    pub expression_entries: usize,
    /// Number of recursive statement entries.
    pub statement_entries:  usize,
    /// Number of recursive compound-block entries.
    pub compound_entries:   usize,
    /// Maximum simultaneously active recursive grammar entries.
    pub max_depth:          usize,
    /// Sampled native stack address span, enabled separately from timings.
    pub native_stack_span:  usize,
    /// Iterative fallback entries caused by the hybrid threshold.
    pub fallback_count:     usize,
    /// Terminal recursive depth limit events.
    pub depth_limit_hits:   usize,
    /// Process peak resident bytes after preparation, before parsing.
    pub peak_rss_before:    usize,
    /// Process peak resident bytes immediately after parsing, before
    /// inspection.
    pub peak_rss_after:     usize,
}

fn peak_rss() -> usize {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage writes a complete rusage to a valid output pointer.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            // SAFETY: successful getrusage initialized the output above.
            let usage = unsafe { usage.assume_init() };
            let scale = if cfg!(target_os = "macos") { 1 } else { 1024 };
            return usize::try_from(usage.ru_maxrss).unwrap_or(0) * scale;
        }
    }
    0
}

/// Opt-in parser algorithms; the production default is always `Machine`.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserStudyMode {
    /// Production explicit-stack parser.
    Machine,
    /// Precedence climbing expressions and recursive statements.
    Recursive,
    /// Recursive parser guarded by `stacker::maybe_grow`.
    Growing,
    /// Iterative fallback after this many recursive entries.
    Hybrid {
        /// Maximum active recursive entries before fallback.
        depth: usize,
    },
    /// Terminal resource diagnostic after this many recursive entries.
    Limited {
        /// Maximum active recursive entries.
        depth: usize,
    },
}

#[derive(Default)]
struct Counters {
    expression_entries: usize,
    statement_entries:  usize,
    compound_entries:   usize,
    max_depth:          usize,
    native_stack_span:  usize,
    fallback_count:     usize,
    depth_limit_hits:   usize,
}

// Keep a named call boundary for Callgrind's --toggle-collect pattern.
#[inline(never)]
fn parser_study_phase7<'tu>(
    preprocessed: PreprocessedTranslationUnit,
    context: &mut Context<'tu>,
    parse: &Bump,
    mode: ParserStudyMode,
    sample_stack: bool,
) -> (ParsedTranslationUnit<'tu>, Counters) {
    let parser = Parser::from_preprocessed(preprocessed, context, parse);
    #[cfg(feature = "parser-recursive-descent-study")]
    {
        use crate::translation_phases::parsing::recursive_descent::StudyMode;
        let mode = match mode {
            | ParserStudyMode::Machine => StudyMode::Machine,
            | ParserStudyMode::Recursive => StudyMode::Recursive,
            | ParserStudyMode::Growing => StudyMode::Growing,
            | ParserStudyMode::Hybrid { depth } => StudyMode::Hybrid { depth },
            | ParserStudyMode::Limited { depth } => StudyMode::Limited { depth },
        };
        let (unit, stats) = parser
            .with_study_mode(mode)
            .with_study_stack_samples(sample_stack)
            .parse_study_translation_unit();
        (
            unit,
            Counters {
                expression_entries: stats.expression_entries,
                statement_entries:  stats.statement_entries,
                compound_entries:   stats.compound_entries,
                max_depth:          stats.max_depth,
                native_stack_span:  stats.native_stack_span,
                fallback_count:     stats.fallback_count,
                depth_limit_hits:   stats.depth_limit_hits,
            },
        )
    }
    #[cfg(not(feature = "parser-recursive-descent-study"))]
    {
        assert_eq!(
            mode,
            ParserStudyMode::Machine,
            "enable parser-recursive-descent-study for recursive modes"
        );
        assert!(
            !sample_stack,
            "enable parser-recursive-descent-study for stack sampling"
        );
        (parser.parse_translation_unit(), Counters::default())
    }
}

impl std::fmt::Debug for StudyPrepared<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudyPrepared").finish_non_exhaustive()
    }
}
