//! The differential tests. The default sizes keep a test run short; the
//! environment variables in the module map, or the ignored sweep, check
//! more.

use super::{
    super::interpreter::Trap,
    generate::generate,
    native::check_native,
    oracle::observe_text,
    sweep,
};

/// The first seed of the default sweeps; any seed works.
const DEFAULT_SEED: u64 = 0xBCC0_D1FF;

/// The interpreter cases a default run checks.
const DEFAULT_CASES: u64 = 300;

/// The cases a default run compiles with clang, two clang runs each.
const DEFAULT_NATIVE_CASES: u64 = 6;

/// Reads a number from the environment, in decimal or with `0x`.
fn setting(name: &str, default: u64) -> u64 {
    let Ok(text) = std::env::var(name) else {
        return default;
    };
    let text = text.trim();
    let parsed = match text.strip_prefix("0x") {
        | Some(hex) => u64::from_str_radix(hex, 16),
        | None => text.parse(),
    };
    parsed.unwrap_or_else(|error| panic!("{name}={text}: {error}"))
}

fn first_seed() -> u64 {
    setting("BCC_DIFFTEST_SEED", DEFAULT_SEED)
}

/// Each seed gives the same text every time, and its probe variant differs
/// from it only by three lines per `freeze`.
#[test]
fn generation_is_deterministic() {
    for seed in 0..50 {
        let text = generate(seed, false);
        assert_eq!(text, generate(seed, false), "seed {seed}");
        let probe = generate(seed, true);
        let probes = probe.lines().count() - text.lines().count();
        assert_eq!(probes, 3 * text.matches("freeze").count(), "seed {seed}");
    }
}

#[test]
fn optimizer_preserves_generated_programs() {
    check_optimizer_sweep(first_seed(), setting("BCC_DIFFTEST_CASES", DEFAULT_CASES));
}

/// A long sweep for occasional runs: `cargo test -- --ignored difftest`.
#[test]
#[ignore = "takes minutes; run explicitly"]
fn optimizer_preserves_many_generated_programs() {
    check_optimizer_sweep(first_seed().wrapping_add(1 << 32), 20_000);
}

fn check_optimizer_sweep(first: u64, cases: u64) {
    let summary = sweep(first, cases);
    assert!(
        summary.failures.is_empty(),
        "{} of {cases} cases miscompiled; the first:\n{}",
        summary.failures.len(),
        summary.failures[0]
    );
    let kept = summary.kept.len() as u64;
    eprintln!(
        "{kept} of {cases} cases kept, discarded {:?}, {} returning poison, {} rewrites",
        summary.discarded, summary.poison_results, summary.rewrites
    );
    // A generator that drifts into mostly undefined programs would make the
    // oracle vacuous; this catches it.
    assert!(kept * 2 >= cases, "only {kept} of {cases} cases were kept");
    assert!(summary.rewrites >= kept, "the optimizer barely ran");
    assert!(
        summary.poison_results * 4 <= kept,
        "{} of {kept} kept cases return poison",
        summary.poison_results
    );
}

#[test]
fn llvm_matches_the_interpreter_on_generated_programs() {
    check_native_sweep(
        first_seed(),
        setting("BCC_DIFFTEST_NATIVE_CASES", DEFAULT_NATIVE_CASES),
    );
}

/// A long native sweep for occasional runs.
#[test]
#[ignore = "takes minutes; run explicitly"]
fn llvm_matches_the_interpreter_on_many_generated_programs() {
    check_native_sweep(first_seed().wrapping_add(1 << 33), 200);
}

/// Checks the first `cases` kept cases from `first` natively.
fn check_native_sweep(first: u64, cases: u64) {
    if cases == 0 {
        return;
    }
    let mut summary = sweep(first, cases * 2);
    assert!(
        summary.failures.is_empty(),
        "the optimizer miscompiled a case; the first:\n{}",
        summary.failures[0]
    );
    summary
        .kept
        .truncate(usize::try_from(cases).expect("the case count fits"));
    let failures = check_native(&summary.kept);
    assert!(
        failures.is_empty(),
        "{} of {} native runs disagreed; the first:\n{}",
        failures.len(),
        summary.kept.len() * 2,
        failures[0]
    );
}

/// `main` returning `value` (an `i32` constant or `poison`) after printing
/// `letter`.
fn program(letter: char, value: &str) -> String {
    let result = if value == "poison" {
        "v2 = poison.i32".to_string()
    } else {
        format!("v2 = iconst.i32 {value}")
    };
    format!(
        "function @putchar(i32) -> i32 external

function @main() -> i32 external {{
block0:
    v0 = iconst.i32 {}
    v1 = call @putchar(v0)
    {result}
    return v2
}}
",
        u32::from(letter)
    )
}

#[test]
fn refinement_allows_any_result_only_for_poison() {
    let five = observe_text(&program('a', "5"));
    let six = observe_text(&program('a', "6"));
    let poison = observe_text(&program('a', "poison"));
    let other_output = observe_text(&program('b', "5"));
    assert_eq!(five.refines(&five), Ok(()));
    assert_eq!(five.refines(&poison), Ok(()));
    assert_eq!(poison.refines(&poison), Ok(()));
    assert!(six.refines(&five).is_err());
    assert!(poison.refines(&five).is_err());
    assert!(other_output.refines(&five).is_err());
    assert!(other_output.refines(&poison).is_err());
}

#[test]
fn refinement_rejects_a_trap() {
    let five = observe_text(&program('a', "5"));
    let trap = observe_text(
        "function @main() -> i32 external {
block0:
    v0 = poison.i1
    brif v0, block1, block1
block1:
    v1 = iconst.i32 5
    return v1
}
",
    );
    assert!(matches!(trap.result, Err(Trap::UndefinedBehavior(..))));
    assert!(trap.refines(&five).is_err());
}
