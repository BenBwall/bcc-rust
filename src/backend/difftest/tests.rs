//! The differential tests. The default sizes keep a test run short; the
//! environment variables in the module map, or the ignored sweep, check
//! more.

use super::{
    super::interpreter::Trap,
    generate::generate,
    native::{
        check_native,
        check_native_backend,
    },
    oracle::{
        Pipeline,
        observe_text,
        optimized,
    },
    sweep,
};
use crate::{
    abi::lower_abi,
    backend::native::emit_assembly,
    ir::parse_module,
    optimizer::Pass,
    target::Target,
    util::bump::Bump,
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

/// The cases a default run compiles with the native back end, two arms
/// each.
const DEFAULT_NATIVE_BACKEND_CASES: u64 = 20;

#[test]
fn native_backend_matches_the_interpreter_on_generated_programs() {
    check_native_backend_sweep(
        first_seed(),
        setting(
            "BCC_DIFFTEST_NATIVE_BACKEND_CASES",
            DEFAULT_NATIVE_BACKEND_CASES,
        ),
    );
}

/// A long native back end sweep for occasional runs.
#[test]
#[ignore = "takes minutes; run explicitly"]
fn native_backend_matches_the_interpreter_on_many_generated_programs() {
    check_native_backend_sweep(first_seed().wrapping_add(1 << 34), 500);
}

/// Checks the first `cases` kept cases from `first` with the native back
/// end, which must accept every one of them: the generator writes only
/// integer, pointer, `f32` and `f64` code.
fn check_native_backend_sweep(first: u64, cases: u64) {
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
    let (failures, refused) = check_native_backend(&summary.kept);
    eprintln!(
        "{} native back end runs, {refused} refused",
        summary.kept.len() * 2
    );
    assert!(
        failures.is_empty(),
        "{} of {} native back end runs disagreed; the first:\n{}",
        failures.len(),
        summary.kept.len() * 2,
        failures[0]
    );
    assert_eq!(refused, 0, "generated programs are refused");
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

#[test]
fn noundef_poison_uses_the_existing_ub_oracle_rule() {
    let defined = observe_text(&program('a', "5"));
    let poison_return = program('a', "poison")
        .replace("function @main() -> i32", "function @main() -> i32 noundef");
    let trapped = observe_text(&poison_return);
    assert!(matches!(trapped.result, Err(Trap::UndefinedBehavior(..))));
    assert!(trapped.refines(&defined).is_err());
    assert!((0..50).any(|seed| generate(seed, false).contains(" noundef")));
}

/// Generated programs exercise atomic read-modify-writes, compare-exchange
/// and fences, so the sweeps compare every back end's atomics with the
/// interpreter.
#[test]
fn generated_programs_exercise_atomics() {
    let programs: Vec<String> = (0..100).map(|seed| generate(seed, false)).collect();
    for operation in [
        " load ",
        " store ",
        " compare_exchange ",
        " nand ",
        " umax ",
        "_fence ",
    ] {
        let count = programs
            .iter()
            .filter(|text| {
                text.lines()
                    .any(|line| line.contains("atomic.") && line.contains(operation))
            })
            .count();
        assert!(count >= 2, "{operation}: {count} of 100 programs");
    }
}

/// The record slots give `sroa` structures to split in many programs.
#[test]
fn generated_records_exercise_scalar_replacement() {
    let split = (0..50)
        .filter(|&seed| {
            optimized(&generate(seed, false), Pipeline::Single(Pass::Sroa), None).rewrites > 0
        })
        .count();
    assert!(split >= 10, "{split} of 50 programs split a record");
}

/// Record copies and fills take every form: between two records at one
/// offset and at different ones, within one record, to and from `@g1`, and
/// partial fills; pointer fields are stored and loaded through.
#[test]
fn generated_records_copy_across_offsets_and_hold_pointers() {
    let programs: Vec<String> = (0..100).map(|seed| generate(seed, false)).collect();
    for form in [
        "; records",
        "; shifted",
        "; within a record",
        "; to @g1",
        "; from @g1",
        "; partial fill",
        "store.ptr",
    ] {
        let count = programs
            .iter()
            .filter(|program| program.contains(form))
            .count();
        assert!(count >= 5, "{count} of 100 programs have `{form}`");
    }
}

/// Range checks and lowered `&&` and `||` give `combine` work in many
/// programs.
#[test]
fn generated_conditions_exercise_combining() {
    let combined = (0..50)
        .filter(|&seed| {
            optimized(
                &generate(seed, false),
                Pipeline::Single(Pass::Combine),
                None,
            )
            .rewrites
                > 0
        })
        .count();
    assert!(
        combined >= 10,
        "{combined} of 50 programs combined a condition"
    );
}

/// Conversions between `i128` and `f32` or `f64`, which the generator does
/// not write: the native back end calls the runtime's `__floattidf`,
/// `__fixsfti` and their unsigned forms, and both arms must print what the
/// interpreter prints.
const WIDE_CONVERSIONS: &str = "\
global @fmt_x internal constant size 5, align 1 = bytes \"5b25785d00\"
function @printf(ptr, ...) -> i32 external

function @show(i64) -> i32 internal {
block0(v0: i64):
    v1 = global_addr @fmt_x
    v2 = trunc.i32 v0
    v3 = call @printf(v1, v2)
    v4 = iconst.i64 32
    v5 = lshr.i64 v0, v4
    v6 = trunc.i32 v5
    v7 = call @printf(v1, v6)
    return v7
}

function @show_wide(i128) -> i32 internal {
block0(v0: i128):
    v1 = trunc.i64 v0
    v2 = call @show(v1)
    v3 = iconst.i128 64
    v4 = lshr.i128 v0, v3
    v5 = trunc.i64 v4
    v6 = call @show(v5)
    return v6
}

function @convert(i128) -> i32 internal {
block0(v0: i128):
    v1 = sitofp.f64 v0
    v2 = sitofp.f32 v0
    v3 = bitcast.i64 v1
    v4 = call @show(v3)
    v5 = bitcast.i32 v2
    v6 = zext.i64 v5
    v7 = call @show(v6)
    v8 = fptosi.i128 v1
    v9 = call @show_wide(v8)
    v10 = fptosi.i128 v2
    v11 = call @show_wide(v10)
    v12 = iconst.i128 0
    v13 = icmp.i128 slt v0, v12
    v14 = isub.i128 v12, v0
    v15 = select.i128 v13, v14, v0
    v16 = uitofp.f64 v15
    v17 = uitofp.f32 v15
    v18 = bitcast.i64 v16
    v19 = call @show(v18)
    v20 = fptoui.i128 v16
    v21 = call @show_wide(v20)
    v22 = fptoui.i128 v17
    v23 = call @show_wide(v22)
    return v23
}

function @main() -> i32 external {
block0:
    v0 = iconst.i128 0
    v1 = call @convert(v0)
    v2 = iconst.i128 -1000000007
    v3 = call @convert(v2)
    v4 = iconst.i128 1512366075204170929049582354406559215
    v5 = call @convert(v4)
    v6 = iconst.i128 -39614081257132168796771975169
    v7 = call @convert(v6)
    v8 = iconst.i128 9007199254740993
    v9 = call @convert(v8)
    v10 = iconst.i128 170141173319264429905852091742258462719
    v11 = uitofp.f32 v10
    v12 = fptoui.i128 v11
    v13 = call @show_wide(v12)
    v14 = iconst.i32 0
    return v14
}
";

/// The native back end accepts the conversions for every target once they
/// are lowered for its ABI, and the host's executable prints what the
/// interpreter prints. Win64 passes the `i128` to `__floattidf` and
/// `__floattisf` by reference and receives it from `__fixdfti` in
/// `%xmm0`.
#[test]
fn native_backend_matches_the_interpreter_on_wide_conversions() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let arena = Bump::new();
        let mut module = parse_module(&arena, WIDE_CONVERSIONS).unwrap();
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        let mut assembly = String::new();
        emit_assembly(&module, target, &mut assembly)
            .unwrap_or_else(|error| panic!("{target:?}: {error}"));
        for routine in [
            "__floattidf",
            "__floattisf",
            "__floatuntidf",
            "__floatuntisf",
            "__fixdfti",
            "__fixsfti",
            "__fixunsdfti",
            "__fixunssfti",
        ] {
            assert!(
                assembly.contains(routine),
                "{target:?} does not call {routine}:\n{assembly}"
            );
        }
    }
    let reference = super::oracle::Reference {
        seed:        0,
        text:        WIDE_CONVERSIONS.to_string(),
        observation: observe_text(WIDE_CONVERSIONS),
        rewrites:    0,
    };
    assert!(
        reference.observation.result.is_ok(),
        "{:?}",
        reference.observation
    );
    let (failures, refused) = check_native_backend(&[reference]);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(refused, 0, "the native back end refused the conversions");
}
