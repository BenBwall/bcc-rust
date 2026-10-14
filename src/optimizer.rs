//! The optimizer: a fixed pipeline of passes over the bcc IR.
//!
//! [`optimize`] takes a [`Module`] and rewrites every defined function in
//! turn, in bottom-up call-graph order when inlining is enabled. There is no
//! pass-manager framework: the pipeline is one list of passes run in one
//! function, as in QBE and Cranelift, and each pass recomputes the analyses it
//! needs ([`Draft::use_counts`], reachability, predecessors) instead of keeping
//! them. The IR has no use lists and stores each block's instructions
//! contiguously, so a function is *lifted* into a [`Draft`], an editable copy
//! with a value-replacement map. The passes edit the draft, and it is *lowered*
//! back into a fresh, densely numbered body once the function is done (or
//! earlier, when the verifier or a printer asks to see the function after a
//! pass).
//!
//! The default pipeline is `inline`, `promote`, `fold`, `simplify-cfg`, `sccp`,
//! `known-bits`, `gvn`, `licm`, `dce` (promotion first, so the others see SSA
//! values instead of loads and stores; value numbering before code motion, so
//! one copy of a repeated invariant moves; and dead code last, so it sweeps
//! what the others leave). Inlining runs once per function, using already
//! optimized callees; the remaining passes repeat until a round changes nothing
//! or [`MAX_ROUNDS`] rounds have run. An explicit `--passes=` list runs once,
//! in the order given. Every rewrite first asks [`OptimizationReport::allow`],
//! which counts rewrites globally and refuses them all once the bisect limit is
//! reached, so `--opt-bisect-limit=N` applies exactly the first `N` rewrites
//! and a miscompile can be bisected on `N`. With `verify_each`, the verifier
//! checks a function before the first pass and after every pass that changed
//! it.
//!
//! For a function whose entry branches on a constant,
//!
//! ```text
//! block0:                         block0:
//!     v0 = iconst.i32 1               v0 = iconst.i32 1
//!     v1 = iconst.i32 0               return v0
//!     v2 = icmp.i32 ne v0, v1
//!     brif v2, block1, block2
//! block1:
//!     return v0
//! block2:
//!     return v1
//! ```
//!
//! `fold` decides the comparison and turns the `brif` into `jump block1`,
//! `simplify-cfg` removes `block2` and merges `block1` into the entry, and
//! `dce` removes the comparison and the unused `iconst`.
//!
//! Read [`optimize`] first, then [`Draft`] to see what a pass edits, then the
//! passes.
//!
//! - Pipeline: `options.rs` holds [`OptimizerOptions`], the pass names and the
//!   `--passes` parser; `report.rs` the transformation counter and the per-pass
//!   counts and times.
//! - Working copy: `draft.rs` lifts and edits a function; `draft/analysis.rs`
//!   computes reachability, ordering, predecessors, use counts and the
//!   control-flow graph; `draft/lower.rs` writes the body back.
//! - Analyses over the IR's dominator tree of that graph: `loops.rs` finds
//!   natural loops and their nesting, `frontiers.rs` dominance frontiers;
//!   `memory.rs` tracks scoped memory generations and conservative aliases.
//! - Passes: `fold.rs` (with `fold/eval.rs`, exact integer evaluation),
//!   `dce.rs`, `simplify_cfg.rs` (with `simplify_cfg/select.rs`, bounded select
//!   formation), `inline.rs` (module call graph and direct-call expansion),
//!   `promote.rs` (stack-slot promotion), `sccp.rs` (sparse conditional
//!   constant propagation), `known_bits.rs` (integer facts from dominating
//!   conditions and division rewrites), `gvn.rs` (value numbering over the
//!   dominator tree, including load reuse and store forwarding) and `licm.rs`
//!   (loop-invariant code motion).
//! - `tests.rs` and `tests/` exercise each pass on textual IR, the pipeline as
//!   a whole, and run programs in the interpreter before and after
//!   optimization.
//!
//! The optimizer implements the IR's semantics, not C's; where a rule exists
//! because of C (signed overflow is undefined, so `nsw` results may be
//! assumed not to overflow), the item that uses it cites the clause. The
//! prototype omits floating-point folding.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (a conforming
//! implementation must preserve the observable behaviour, and may change
//! anything else).

// Pipeline
mod options;
mod report;

// Working copy
mod draft;

// Analyses
mod frontiers;
mod loops;
mod memory;

// Passes
mod dce;
mod fold;
mod gvn;
mod inline;
mod known_bits;
mod licm;
mod promote;
mod sccp;
mod simplify_cfg;
mod sroa;

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;

use std::{
    fmt,
    time::Instant,
};

use draft::Draft;
pub(crate) use options::{
    OptimizerOptions,
    Pass,
    PassList,
    PassListError,
    parse_pass_list,
};
pub(crate) use report::OptimizationReport;

use crate::{
    ir::{
        FuncId,
        Module,
        Profile,
        verify_function,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The most rounds of the default pipeline one function gets.
const MAX_ROUNDS: u32 = 8;

/// Optimizes every defined function of `module` and reports what it did.
///
/// The module must verify for the pre-ABI profile. `scratch` holds the
/// module-lifetime call graph and ordering; the caller resets it between calls.
/// A separate arena holds function work and is reset after every function.
pub(crate) fn optimize(
    module: &mut Module<'_>,
    options: &OptimizerOptions<'_>,
    scratch: &Bump,
) -> OptimizationReport {
    let mut function_scratch = Bump::new();
    optimize_with_function_scratch(module, options, scratch, &mut function_scratch)
}

/// Runs the module pipeline with independently owned function scratch.
fn optimize_with_function_scratch(
    module: &mut Module<'_>,
    options: &OptimizerOptions<'_>,
    scratch: &Bump,
    function_scratch: &mut Bump,
) -> OptimizationReport {
    let mut report = OptimizationReport::new(options.bisect_limit);
    let mut functions = ArenaVec::new_in(scratch);
    functions.extend(
        module
            .functions()
            .filter(|(_, function)| function.body.is_some())
            .map(|(func, _)| func),
    );
    let graph = inline::CallGraph::compute(module, scratch);
    let inline_enabled = options
        .passes
        .as_ref()
        .is_none_or(|list| list.as_slice().contains(&Pass::Inline));
    if inline_enabled {
        functions.clear();
        functions.extend_from_slice(&graph.order);
    }
    for func in functions {
        optimize_function(module, func, options, &mut report, &graph, function_scratch);
        function_scratch.reset();
        report.functions += 1;
    }
    report
}

/// The state of one function's run: whether the module holds a body that
/// matches the draft.
struct Materialized {
    /// The module holds a body lowered from the draft at some point.
    installed: bool,
    /// The draft changed after that body was lowered.
    stale:     bool,
}

/// Runs the passes over one function.
fn optimize_function(
    module: &mut Module<'_>,
    func: FuncId,
    options: &OptimizerOptions<'_>,
    report: &mut OptimizationReport,
    graph: &inline::CallGraph<'_>,
    scratch: &Bump,
) {
    if options.verify_each {
        assert_verified(module, func, format_args!("before optimization"), scratch);
    }
    let Some(body) = module.function_mut(func).body.take() else {
        return;
    };
    let mut draft = Draft::lift(&body, scratch);
    let mut state = Materialized {
        installed: false,
        stale:     false,
    };
    let (passes, rounds) = match &options.passes {
        | Some(list) => (list.as_slice(), 1),
        | None => (&Pass::ALL[..], MAX_ROUNDS),
    };
    for round in 0..rounds {
        let mut changed = false;
        for &pass in passes {
            if pass == Pass::Inline && options.passes.is_none() && round != 0 {
                continue;
            }
            changed |= run_pass(
                pass, &mut draft, module, func, options, report, &mut state, graph, scratch,
            );
        }
        report.rounds += 1;
        if !changed || report.limit_reached() {
            break;
        }
    }
    if state.stale || !state.installed {
        if state.stale {
            draft.lower(module, func);
        } else {
            drop(draft);
            module.function_mut(func).body = Some(body);
        }
    }
}

/// Runs one pass on the draft, then verifies or prints the function if asked.
/// Returns whether the draft changed.
#[expect(
    clippy::too_many_arguments,
    reason = "The pass loop's state is threaded through one call per pass."
)]
fn run_pass(
    pass: Pass,
    draft: &mut Draft<'_, '_>,
    module: &mut Module<'_>,
    func: FuncId,
    options: &OptimizerOptions<'_>,
    report: &mut OptimizationReport,
    state: &mut Materialized,
    graph: &inline::CallGraph<'_>,
    scratch: &Bump,
) -> bool {
    report.begin_pass(pass);
    let start = Instant::now();
    let changed = match pass {
        | Pass::Inline => inline::run(draft, module, graph, report),
        | Pass::Fold => fold::run(draft, module.function_signature(func), report),
        | Pass::Dce => dce::run(draft, report),
        | Pass::SimplifyCfg => simplify_cfg::run(draft, report),
        | Pass::Gvn => gvn::run(draft, report),
        | Pass::Licm => licm::run(draft, report),
        | Pass::Sroa => sroa::run(draft, report),
        | Pass::Promote => promote::run(draft, report),
        | Pass::Sccp => sccp::run(draft, report),
        | Pass::KnownBits => known_bits::run(draft, report),
    };
    report.end_pass(pass, start.elapsed());
    #[cfg(test)]
    let changed = options.injected.is_some_and(|injected| injected(draft)) | changed;
    state.stale |= changed;
    let verify = options.verify_each && changed;
    if verify || options.print_after_all.is_some() {
        if state.stale || !state.installed {
            draft.lower(module, func);
            state.stale = false;
            state.installed = true;
        }
        if verify {
            assert_verified(module, func, format_args!("after {pass}"), scratch);
        }
        if let Some(sink) = options.print_after_all {
            let mut sink = sink.borrow_mut();
            _ = writeln!(
                sink,
                "*** IR after {pass} of @{} ***",
                module.function(func).name
            );
            _ = write!(sink, "{}", module.display_function(func));
        }
    }
    changed
}

/// Panics with the verifier's errors and the function's text if the function
/// does not verify: a broken pass is a compiler bug, not bad input.
fn assert_verified(module: &Module<'_>, func: FuncId, when: fmt::Arguments<'_>, scratch: &Bump) {
    let errors = verify_function(module, func, Profile::PreAbi, scratch);
    if errors.is_empty() {
        return;
    }
    panic!(
        "the IR of @{} does not pass the verifier {when}:\n{}{}",
        module.function(func).name,
        fmt::from_fn(|f| errors.iter().try_for_each(|error| writeln!(f, "  {error}"))),
        module.display_function(func),
    );
}
