//! Lowering tests: golden IR for each construct family (`golden.rs`),
//! programs whose lowering must verify (`programs.rs`), floating-point and
//! bit-field programs run in the interpreter and clang (`numeric_programs.rs`),
//! the constant conversions (`floats.rs`), and the constructs
//! lowering refuses (`refusals.rs`).

mod call_diagnostics;
mod complex;
mod correspondence;
mod effects;
mod entities;
mod extensions;
mod floats;
mod fortify;
mod golden;
mod inline;
mod layout;
mod numeric_programs;
pub(crate) mod programs;
mod ranges;
mod refusals;
mod registry;
mod resolved;
mod review;

use std::path::Path;

use super::*;
use crate::{
    headers::HeaderSearch,
    ir::{
        parse_module,
        verify_module,
    },
    pipeline::{
        analyze_translation_unit,
        parse_translation_unit,
    },
};

/// Runs phases 1-7 and lowering on `source` for `target`. A lowered module
/// is verified and must survive a print-parse round trip; it is returned
/// printed. Otherwise the lowering errors are returned as text.
fn lower_for(source: &str, target: Target) -> Result<String, Vec<String>> {
    lower_with(
        source,
        crate::configuration::CompilerConfiguration::default().with_target(target),
    )
}

/// [`lower_for`] with a whole configuration, which names the target.
fn lower_with(
    source: &str,
    configuration: crate::configuration::CompilerConfiguration,
) -> Result<String, Vec<String>> {
    #[cfg(feature = "recursive-lowering-study")]
    {
        // Test-only overrides/counters belong to the originating test thread.
        // Carry them explicitly rather than silently resetting test policy.
        let minimum = initializers::REPLICATION_MINIMUM.get();
        let replay = initializers::REPLAY_STEPS.get();
        let effects = crate::test_support::EFFECT_RECIPE_STEPS.get();
        let (result, replay, effects) = crate::with_compilation_stack(1024_usize.pow(3), || {
            initializers::REPLICATION_MINIMUM.set(minimum);
            initializers::REPLAY_STEPS.set(replay);
            crate::test_support::EFFECT_RECIPE_STEPS.set(effects);
            let result = lower_with_here(source, configuration);
            (
                result,
                initializers::REPLAY_STEPS.get(),
                crate::test_support::EFFECT_RECIPE_STEPS.get(),
            )
        })
        .expect("reserved compilation worker");
        initializers::REPLAY_STEPS.set(replay);
        crate::test_support::EFFECT_RECIPE_STEPS.set(effects);
        result
    }
    #[cfg(not(feature = "recursive-lowering-study"))]
    lower_with_here(source, configuration)
}

fn lower_with_here(
    source: &str,
    configuration: crate::configuration::CompilerConfiguration,
) -> Result<String, Vec<String>> {
    let target = configuration.target();
    let tu = Bump::new();
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::with_configuration(&tu, configuration);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let arena = Bump::new();
    let mut scratch = Bump::new();
    #[cfg(feature = "recursive-lowering-study")]
    {
        let replay = initializers::REPLAY_STEPS.get();
        let effects = crate::test_support::EFFECT_RECIPE_STEPS.get();
        let baseline_ir = Bump::new();
        let mut baseline_scratch = Bump::new();
        let mut options = LoweringOptions::default_for_build(true);
        options.recursive = false;
        let baseline = lower_with_options(
            &context,
            &unit,
            &sema,
            target,
            &baseline_ir,
            &mut baseline_scratch,
            &mut options,
        );
        let recursive_ir = Bump::new();
        let mut recursive_scratch = Bump::new();
        options.recursive = true;
        options.plain_inline = true;
        let recursive = lower_with_options(
            &context,
            &unit,
            &sema,
            target,
            &recursive_ir,
            &mut recursive_scratch,
            &mut options,
        );
        let describe = |result: Result<LoweredUnit<'_>, LoweringErrors<'_>>| match result {
            | Ok(unit) => format!(
                "{}\n{:?}\n{:?}",
                unit.module, unit.function_sources, unit.call_diagnostics
            ),
            | Err(errors) => format!("{:?}", errors.errors),
        };
        initializers::REPLAY_STEPS.set(replay);
        crate::test_support::EFFECT_RECIPE_STEPS.set(effects);
        pretty_assertions::assert_eq!(
            describe(baseline),
            describe(recursive),
            "task/recursive IR, provenance and diagnostics identity"
        );
    }
    match lower_translation_unit(&context, &unit, &sema, target, &arena, &mut scratch) {
        | Ok(module) => {
            let verifier = Bump::new();
            let errors: Vec<String> = verify_module(&module, Profile::PreAbi, &verifier)
                .iter()
                .map(ToString::to_string)
                .collect();
            let text = module.to_string();
            assert!(errors.is_empty(), "{errors:?}\n{text}");
            let reparsed = Bump::new();
            let parsed =
                parse_module(&reparsed, &text).unwrap_or_else(|error| panic!("{error}\n{text}"));
            pretty_assertions::assert_eq!(parsed.to_string(), text, "the printed IR round-trips");
            Ok(text)
        },
        | Err(errors) => Err(errors.errors.iter().map(ToString::to_string).collect()),
    }
}

/// [`lower_for`] on the default Linux target.
fn lower(source: &str) -> Result<String, Vec<String>> {
    lower_for(source, Target::LinuxGnu)
}

/// The IR of `source`, panicking with the errors if it does not lower.
#[track_caller]
fn lowered(source: &str) -> String {
    lower(source).unwrap_or_else(|errors| panic!("lowering failed: {errors:?}\n{source}"))
}

/// Only the functions of a printed module, without its target lines.
fn functions(text: &str) -> String {
    text.split("\n\n")
        .filter(|section| !section.starts_with("target "))
        .collect::<Vec<_>>()
        .join("\n\n")
}
