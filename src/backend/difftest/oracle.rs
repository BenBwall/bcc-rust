//! The interpreter oracle: a generated program, run before and after the
//! optimizer, must behave the same, up to the refinement the IR's poison
//! rules allow.
//!
//! [`check_optimizer`] screens a case with its probe variant, runs the
//! original program as the reference, then optimizes a fresh copy under each
//! of [`PIPELINES`] and compares. A disagreement is minimized by bisecting
//! the optimizer's rewrite counter (`bisect_limit`) to the first rewrite
//! after which the program misbehaves, and reported with the seed and the
//! IR text on both sides of that rewrite.

use std::{
    fmt::Write as _,
    panic::{
        AssertUnwindSafe,
        catch_unwind,
    },
};

use super::generate::generate;
use crate::{
    backend::interpreter::{
        DefaultHost,
        Limits,
        Outcome,
        RuntimeValue,
        Termination,
        Trap,
        run_with_limits,
    },
    ir::{
        Module,
        Profile,
        parse_module,
        verify_module,
    },
    optimizer::{
        OptimizerOptions,
        Pass,
        PassList,
        optimize,
    },
    util::bump::Bump,
};

/// What checking one seed against the optimizer gave.
#[derive(Debug)]
pub(super) enum Verdict {
    /// The original program has undefined behaviour or freezes poison, so
    /// nothing can be compared.
    Discarded(&'static str),
    /// Every pipeline preserved the program's behaviour. `reference` is the
    /// original's text and behaviour, for the native oracle.
    Passed(Reference),
    /// A pipeline changed the behaviour; the report says how.
    Failed(String),
}

/// The original program of a kept case and what it did.
#[derive(Debug)]
pub(super) struct Reference {
    pub(super) seed:        u64,
    pub(super) text:        String,
    pub(super) observation: Observation,
    /// The rewrites the default pipeline applied.
    pub(super) rewrites:    u64,
}

impl Reference {
    /// Whether `main` returned poison, so that its result constrains nothing.
    pub(super) fn returns_poison(&self) -> bool {
        matches!(
            self.observation.result,
            Ok(Outcome {
                termination: Termination::Returned(Some(RuntimeValue::Poison)),
                ..
            })
        )
    }

    /// The `i32` that `main` returned, unless it is poison.
    pub(super) fn exit_value(&self) -> Option<i32> {
        match self.observation.result {
            | Ok(Outcome {
                termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
                ..
            }) => Some(low_i32(bits)),
            | _ => None,
        }
    }
}

/// Generates the case for `seed` and checks it against every pipeline.
pub(super) fn check_optimizer(seed: u64) -> Verdict {
    let text = generate(seed, false);
    let probe = generate(seed, true);
    assert_verifies(seed, &text);
    assert_verifies(seed, &probe);
    match observe_text(&probe).result {
        | Err(Trap::UndefinedBehavior(..)) => return Verdict::Discarded("undefined behaviour"),
        | Err(Trap::StepLimit) => return Verdict::Discarded("step limit"),
        | Err(trap) => panic!("seed {seed:#x}: the probe run trapped: {trap}\n{probe}"),
        | Ok(_) => {},
    }
    let observation = observe_text(&text);
    if let Err(trap) = observation.result {
        panic!("seed {seed:#x}: the program trapped where its probe did not: {trap}\n{text}");
    }
    let mut reference = Reference {
        seed,
        text,
        observation,
        rewrites: 0,
    };
    for pipeline in PIPELINES {
        let run = optimized(&reference.text, pipeline, None);
        if let Err(problem) = run.refines(&reference.observation) {
            return Verdict::Failed(minimize(&reference, pipeline, run.rewrites, &problem));
        }
        if matches!(pipeline, Pipeline::Default) {
            reference.rewrites = run.rewrites;
        }
    }
    Verdict::Passed(reference)
}

/// The optimizer configurations every case runs under.
#[derive(Clone, Copy, Debug)]
pub(super) enum Pipeline {
    /// The default pipeline, repeated to a fixed point.
    Default,
    /// One pass, once.
    Single(Pass),
}

pub(super) const PIPELINES: [Pipeline; 4] = [
    Pipeline::Default,
    Pipeline::Single(Pass::Fold),
    Pipeline::Single(Pass::SimplifyCfg),
    Pipeline::Single(Pass::Dce),
];

/// A program optimized with at most `limit` rewrites, and what it did.
#[derive(Debug)]
pub(super) struct Optimized {
    /// The optimized module's text, or the optimizer's panic message.
    pub(super) text:        Result<String, String>,
    pub(super) rewrites:    u64,
    pub(super) observation: Option<Observation>,
}

/// Optimizes `text` under `pipeline`, applying at most `limit` rewrites,
/// verifies the result, and runs it.
pub(super) fn optimized(text: &str, pipeline: Pipeline, limit: Option<u64>) -> Optimized {
    let arena = Bump::new();
    let mut module = parse_module(&arena, text).expect("generated IR parses");
    let mut options = OptimizerOptions::default();
    options.passes = match pipeline {
        | Pipeline::Default => None,
        | Pipeline::Single(pass) => PassList::new(&[pass]),
    };
    options.verify_each = true;
    options.bisect_limit = limit;
    let scratch = Bump::new();
    let report = catch_unwind(AssertUnwindSafe(|| {
        optimize(&mut module, &options, &scratch)
    }));
    let report = match report {
        | Ok(report) => report,
        | Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_default();
            return Optimized {
                text:        Err(format!("the optimizer panicked: {message}")),
                rewrites:    0,
                observation: None,
            };
        },
    };
    let text = module.to_string();
    let errors = verify_errors(&module, Profile::PreAbi);
    if !errors.is_empty() {
        return Optimized {
            text:        Err(format!(
                "the optimized module does not verify:\n{errors}{text}"
            )),
            rewrites:    report.transformations(),
            observation: None,
        };
    }
    Optimized {
        observation: Some(observe(&module)),
        text:        Ok(text),
        rewrites:    report.transformations(),
    }
}

impl Optimized {
    /// Checks that the optimized program refines the reference.
    fn refines(&self, reference: &Observation) -> Result<(), String> {
        if let Err(message) = &self.text {
            return Err(message.clone());
        }
        let observation = self.observation.as_ref().expect("a verified module ran");
        observation.refines(reference)
    }
}

/// Bisects the rewrite counter to the first rewrite after which the
/// program misbehaves, and writes the report. `rewrites` is the full run's
/// count, or 0 if the optimizer panicked before reporting one.
fn minimize(reference: &Reference, pipeline: Pipeline, rewrites: u64, problem: &str) -> String {
    let fails = |limit: u64| {
        optimized(&reference.text, pipeline, Some(limit))
            .refines(&reference.observation)
            .is_err()
    };
    let mut report = String::new();
    let _ = writeln!(
        report,
        "seed {:#x}, pipeline {pipeline:?}: {problem}\nrerun with BCC_DIFFTEST_SEED={:#x} \
         BCC_DIFFTEST_CASES=1",
        reference.seed, reference.seed
    );
    // The full run's count bounds the search, unless the optimizer panicked
    // before reporting one; then double the limit until the program fails.
    let mut bad = rewrites;
    let mut good = 0;
    if fails(0) {
        let _ = writeln!(
            report,
            "the program misbehaves even with no rewrites allowed"
        );
        bad = 0;
    } else {
        if bad == 0 || !fails(bad) {
            bad = 1;
            while !fails(bad) {
                good = bad;
                bad *= 2;
                assert!(
                    bad < 1 << 40,
                    "seed {:#x}: the failure does not reproduce",
                    reference.seed
                );
            }
        }
        while bad - good > 1 {
            let middle = good + (bad - good) / 2;
            if fails(middle) {
                bad = middle;
            } else {
                good = middle;
            }
        }
    }
    let _ = writeln!(
        report,
        "\nthe original program, which {}:\n{}",
        reference.observation, reference.text
    );
    if bad > 0 {
        let before = optimized(&reference.text, pipeline, Some(bad - 1));
        let _ = writeln!(
            report,
            "after the first {} rewrites, where it still behaves:\n{}",
            bad - 1,
            before.text.unwrap_or_else(|error| error)
        );
    }
    let after = optimized(&reference.text, pipeline, Some(bad));
    let behaviour = after
        .observation
        .as_ref()
        .map_or_else(String::new, ToString::to_string);
    let _ = writeln!(
        report,
        "the minimal failing module, after rewrite {bad}, which {behaviour}:\n{}",
        after.text.unwrap_or_else(|error| error)
    );
    report
}

/// The `i32` whose bits are the low 32 of `bits`.
fn low_i32(bits: u128) -> i32 {
    u32::try_from(bits & 0xFFFF_FFFF).expect("masked to 32 bits") as i32
}

/// What one run of `main` did: how it ended and what it printed.
#[derive(Debug)]
pub(super) struct Observation {
    pub(super) result: Result<Outcome, Trap>,
    pub(super) output: Vec<u8>,
}

impl Observation {
    /// Checks that `self` is a behaviour `reference` allows: the same
    /// output and ending, where a poison result allows any result.
    pub(super) fn refines(&self, reference: &Self) -> Result<(), String> {
        let (Ok(expected), Ok(actual)) = (&reference.result, &self.result) else {
            return Err(format!(
                "expected a program that {reference}, got one that {self}"
            ));
        };
        let result_matches = match (expected.termination, actual.termination) {
            | (
                Termination::Returned(Some(RuntimeValue::Poison)),
                Termination::Returned(Some(_)),
            ) => true,
            | (expected, actual) => expected == actual,
        };
        if result_matches && reference.output == self.output {
            Ok(())
        } else {
            Err(format!(
                "expected a program that {reference}, got one that {self}"
            ))
        }
    }
}

impl std::fmt::Display for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.result {
            | Ok(outcome) => match outcome.termination {
                | Termination::Returned(Some(RuntimeValue::Int(bits))) =>
                    write!(f, "returns {}", low_i32(bits))?,
                | termination => write!(f, "ends with {termination:?}")?,
            },
            | Err(trap) => write!(f, "traps ({trap})")?,
        }
        write!(f, " and prints {:?}", String::from_utf8_lossy(&self.output))
    }
}

/// The most instructions a generated program may run; the generator's cost
/// estimate keeps them far below.
const STEP_LIMIT: u64 = 2_000_000;

/// Parses `text` and runs it.
pub(super) fn observe_text(text: &str) -> Observation {
    let arena = Bump::new();
    let module = parse_module(&arena, text).expect("generated IR parses");
    observe(&module)
}

/// Runs `main` of `module` under the default host.
pub(super) fn observe(module: &Module<'_>) -> Observation {
    let arena = Bump::new();
    let mut host = DefaultHost::new_in(&arena);
    let limits = Limits {
        steps: STEP_LIMIT,
        ..Limits::default()
    };
    let result = run_with_limits(module, "main", &[], &mut host, limits);
    Observation {
        result,
        output: host.output().to_vec(),
    }
}

/// Panics with the seed, the errors and the text if `text` does not parse
/// or verify in either profile: a generator bug.
fn assert_verifies(seed: u64, text: &str) {
    let arena = Bump::new();
    let module = parse_module(&arena, text)
        .unwrap_or_else(|error| panic!("seed {seed:#x}: {error}\n{text}"));
    for profile in [Profile::PreAbi, Profile::PostAbi] {
        let errors = verify_errors(&module, profile);
        assert!(errors.is_empty(), "seed {seed:#x}: {errors}\n{text}");
    }
}

/// The verifier's errors for `module`, one per line.
pub(super) fn verify_errors(module: &Module<'_>, profile: Profile) -> String {
    let scratch = Bump::new();
    let mut errors = String::new();
    for error in &verify_module(module, profile, &scratch) {
        let _ = writeln!(errors, "  {error}");
    }
    errors
}
