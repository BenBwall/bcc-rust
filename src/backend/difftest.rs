//! Differential tests of the optimizer and the LLVM back end on random
//! programs, with the interpreter as the oracle. Test-only: nothing here
//! ships.
//!
//! [`sweep`] generates one program per seed, screens out the ones whose
//! behaviour the IR leaves open, and checks the rest: the interpreter runs
//! the original and each optimized version, which must refine it (the same
//! output and ending; a poison result allows any result). A kept case can
//! then be compiled with clang under the A0 and A1 arms and run natively,
//! which must match the interpreter too.
//!
//! A case for seed 7 goes like this: `generate(7, false)` writes a module
//! of a few functions and a `main` returning an `i32` checksum, and
//! `generate(7, true)` the same module with a probe before every `freeze`.
//! The probe run traps if the program has undefined behaviour or freezes
//! poison, and the case is discarded. Otherwise the original runs, then
//! `optimize` runs on a fresh copy under the default pipeline and under each
//! single pass, and each result runs. On a mismatch the harness bisects
//! `bisect_limit` to the first bad rewrite and panics with the seed and the
//! IR before and after it.
//!
//! Read [`sweep`] first, then `oracle.rs` for the comparison and
//! minimization, then `generate.rs` for the programs.
//!
//! - Programs: `generate.rs` writes a random, verifier-clean module for a seed.
//! - Oracles: `oracle.rs` checks the optimizer with the interpreter;
//!   `native.rs` checks the LLVM back end against the interpreter.
//! - `tests.rs` holds the tests, whose sizes the environment can raise:
//!   `BCC_DIFFTEST_CASES` (interpreter cases), `BCC_DIFFTEST_NATIVE_CASES`
//!   (cases compiled by clang) and `BCC_DIFFTEST_SEED` (the first seed). Two
//!   ignored tests sweep more: `cargo test -- --ignored difftest`.

// Programs
mod generate;

// Oracles
mod native;
mod oracle;

// Tests
mod tests;

use std::{
    collections::BTreeMap,
    thread,
};

use oracle::{
    Reference,
    Verdict,
    check_optimizer,
};

/// Checks the seeds `first..first + count` against the optimizer, several
/// threads at a time, and tallies the verdicts.
fn sweep(first: u64, count: u64) -> Summary {
    let parallelism = thread::available_parallelism().map_or(4, usize::from);
    let verdicts: Vec<Verdict> = thread::scope(|scope| {
        let workers: Vec<_> = (0..parallelism as u64)
            .map(|worker| {
                scope.spawn(move || {
                    (worker..count)
                        .step_by(parallelism)
                        .map(|index| check_optimizer(first.wrapping_add(index)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a sweep worker does not panic"))
            .collect()
    });
    let mut summary = Summary::default();
    for verdict in verdicts {
        match verdict {
            | Verdict::Discarded(reason) => *summary.discarded.entry(reason).or_default() += 1,
            | Verdict::Passed(reference) => {
                summary.poison_results += u64::from(reference.returns_poison());
                summary.rewrites += reference.rewrites;
                summary.kept.push(reference);
            },
            | Verdict::Failed(report) => summary.failures.push(report),
        }
    }
    summary.kept.sort_by_key(|reference| reference.seed);
    summary
}

/// What a sweep found.
#[derive(Debug, Default)]
struct Summary {
    /// The cases every pipeline preserved, by seed.
    kept:           Vec<Reference>,
    /// The cases screened out, by reason.
    discarded:      BTreeMap<&'static str, u64>,
    /// The kept cases whose `main` returned poison.
    poison_results: u64,
    /// The rewrites the default pipeline applied to the kept cases.
    rewrites:       u64,
    /// A report per case some pipeline miscompiled.
    failures:       Vec<String>,
}
