//! What an optimizer run did: the transformation counter that bisecting
//! uses, and per-pass counts and times.

use std::{
    fmt,
    time::Duration,
};

use super::options::Pass;

/// The outcome of [`optimize`](super::optimize), and the counter that every
/// rewrite consults first.
#[derive(Clone, Debug)]
pub(crate) struct OptimizationReport {
    passes:               [PassStats; Pass::COUNT],
    current:              Option<Pass>,
    applied:              u64,
    limit:                Option<u64>,
    limit_reached:        bool,
    /// Functions with a body that were optimized.
    pub(crate) functions: u32,
    /// Rounds of the default pipeline, summed over functions.
    pub(crate) rounds:    u32,
}

/// What one pass did over a whole run.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct PassStats {
    /// How many times the pass ran, once per function and round.
    pub(crate) runs:    u32,
    /// Rewrites it applied.
    pub(crate) changes: u64,
    /// Time spent in it, lifting and verifying excluded.
    pub(crate) time:    Duration,
}

impl OptimizationReport {
    /// A report whose [`allow`](Self::allow) grants `limit` rewrites, or
    /// all of them without a limit.
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            passes: [PassStats::default(); Pass::COUNT],
            current: None,
            applied: 0,
            limit,
            limit_reached: false,
            functions: 0,
            rounds: 0,
        }
    }

    /// Asks permission for one rewrite. Every rewrite calls this first and
    /// makes the change only if it returns `true`; once the bisect limit is
    /// reached it returns `false` forever, so the rewrites that did happen
    /// are a prefix of the full run.
    pub(crate) fn allow(&mut self) -> bool {
        if self.limit.is_some_and(|limit| self.applied >= limit) {
            self.limit_reached = true;
            return false;
        }
        self.applied += 1;
        if let Some(pass) = self.current {
            self.passes[pass.index()].changes += 1;
        }
        true
    }

    /// The rewrites applied so far, in all passes.
    pub(crate) const fn transformations(&self) -> u64 {
        self.applied
    }

    /// Whether a rewrite has been refused because of the bisect limit.
    pub(crate) const fn limit_reached(&self) -> bool {
        self.limit_reached
    }

    pub(crate) fn stats(&self, pass: Pass) -> PassStats {
        self.passes[pass.index()]
    }

    /// The time spent in all passes.
    pub(crate) fn total_time(&self) -> Duration {
        self.passes.iter().map(|stats| stats.time).sum()
    }

    /// Starts attributing rewrites to `pass`.
    pub(super) fn begin_pass(&mut self, pass: Pass) {
        self.current = Some(pass);
        self.passes[pass.index()].runs += 1;
    }

    /// Stops attributing rewrites to the running pass, charging it `time`.
    pub(super) fn end_pass(&mut self, pass: Pass, time: Duration) {
        self.current = None;
        self.passes[pass.index()].time += time;
    }
}

impl fmt::Display for OptimizationReport {
    /// A table of the passes with their runs, rewrites and times.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} functions, {} rounds, {} rewrites{}",
            self.functions,
            self.rounds,
            self.applied,
            if self.limit_reached {
                " (bisect limit reached)"
            } else {
                ""
            }
        )?;
        for pass in Pass::ALL {
            let stats = self.stats(pass);
            writeln!(
                f,
                "  {:<13}{:>6} runs{:>8} rewrites{:>12.3?}",
                pass.name(),
                stats.runs,
                stats.changes,
                stats.time
            )?;
        }
        Ok(())
    }
}
