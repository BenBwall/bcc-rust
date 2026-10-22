//! Lowering-only measurements with untimed front-end preparation and IR
//! inspection. The compilation callback keeps every arena on one worker.

use std::{
    path::Path,
    time::Instant,
};

use super::input::benchmark_context;
use crate::{
    headers::HeaderSearch,
    ir::{
        Profile,
        verify_module,
    },
    lowering::{
        LoweringOptions,
        lower_with_options,
    },
    pipeline::{
        analyze_translation_unit,
        parse_translation_unit,
    },
    util::{
        bump::Bump,
        vm::accounting,
    },
};

/// Prepares the front end, then measures lowering in isolation. Begin/end
/// hooks delimit exactly the lowering interval for allocation counters.
#[doc(hidden)]
#[must_use]
#[expect(
    clippy::disallowed_macros,
    reason = "Study results own untimed IR/diagnostic inspection after compilation arenas die."
)]
pub fn measure_lowering_source(
    source: &str,
    recursive: bool,
    sample_stack: bool,
    inspect: bool,
    begin: impl FnOnce(),
    end: impl FnOnce(),
) -> LoweringMeasurement {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let mut context = benchmark_context(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<lowering-study>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    let ir = Bump::new();
    let mut scratch = Bump::new();
    let mut options = LoweringOptions::default_for_build(true);
    options.recursive = recursive;
    options.sample_stack = sample_stack;
    let live = accounting::live();
    accounting::reset_peak();
    begin();
    let start = Instant::now();
    let lowered = lower_with_options(
        &context,
        &syntax,
        &sema,
        context.configuration.target(),
        &ir,
        &mut scratch,
        &mut options,
    );
    let nanoseconds = start.elapsed().as_nanos();
    end();
    let peak = accounting::peak();
    let mut functions = 0;
    let mut errors = 0;
    let inspection = match lowered {
        | Ok(unit) => {
            functions = unit.module.functions().count();
            inspect.then(|| {
                let verifier = Bump::new();
                let failures = verify_module(&unit.module, Profile::PreAbi, &verifier);
                assert!(failures.is_empty(), "lowered IR must verify: {failures:?}");
                format!(
                    "{}\n=== sources ===\n{:?}\n=== call diagnostics ===\n{:?}",
                    unit.module, unit.function_sources, unit.call_diagnostics
                )
            })
        },
        | Err(failures) => {
            errors = failures.errors.len();
            inspect.then(|| format!("lowering errors: {:?}", failures.errors))
        },
    };
    LoweringMeasurement {
        nanoseconds,
        functions,
        errors,
        diagnostics: context.pending_error_count(),
        scratch_bytes: scratch.high_water(),
        ir_bytes: ir.high_water(),
        peak_commit_bytes: peak.committed.saturating_sub(live.committed),
        entries: options.stats.entries,
        max_depth: options.stats.max_depth,
        stack_span: options.stats.high.saturating_sub(options.stats.low),
        inspection,
    }
}

/// Lowering-only time and arena requests. Stack sampling and inspection must
/// be selected separately from timing runs.
#[doc(hidden)]
#[derive(Debug)]
#[expect(
    clippy::disallowed_types,
    reason = "Owned inspection crosses the study worker boundary."
)]
pub struct LoweringMeasurement {
    pub nanoseconds:       u128,
    pub functions:         usize,
    pub diagnostics:       usize,
    pub errors:            usize,
    pub scratch_bytes:     usize,
    pub ir_bytes:          usize,
    pub peak_commit_bytes: usize,
    pub entries:           usize,
    pub max_depth:         usize,
    pub stack_span:        usize,
    pub inspection:        Option<String>,
}
