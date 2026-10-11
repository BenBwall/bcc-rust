//! What a caller can ask of the optimizer: which passes to run, whether to
//! verify between them, where to print, and where to stop.

use std::{
    cell::RefCell,
    fmt,
};

use thiserror::Error;

/// How [`optimize`](super::optimize) runs. `Default` is the full pipeline,
/// verified after every pass in debug and test builds.
pub(crate) struct OptimizerOptions<'a> {
    /// The passes to run once each, in order, instead of the default
    /// pipeline (from `--passes=fold,dce,simplify-cfg`).
    pub(crate) passes:          Option<PassList>,
    /// Run the IR verifier on every function after every pass that changed
    /// it, and before the first pass. A failure is a compiler bug and
    /// panics with the errors and the function's text.
    pub(crate) verify_each:     bool,
    /// Where to print each function after every pass
    /// (`--print-after-all`). It is a `RefCell` so that the options stay
    /// shared while the optimizer writes.
    pub(crate) print_after_all: Option<&'a RefCell<dyn fmt::Write + 'a>>,
    /// The number of rewrites to apply before stopping, in pipeline order
    /// (`--opt-bisect-limit`). With `Some(n)`, the first `n` rewrites happen
    /// and every later one is skipped, so a miscompile can be bisected on `n`.
    pub(crate) bisect_limit:    Option<u64>,
    /// A pass injected after each real pass, to test that verification
    /// catches a broken one.
    #[cfg(test)]
    pub(super) injected:        Option<fn(&mut super::draft::Draft<'_, '_>) -> bool>,
}

impl Default for OptimizerOptions<'_> {
    fn default() -> Self {
        Self {
            passes:                None,
            verify_each:           cfg!(debug_assertions),
            print_after_all:       None,
            bisect_limit:          None,
            #[cfg(test)]
            injected:              None,
        }
    }
}

/// A transformation the optimizer can run.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Pass {
    /// Constant folding and algebraic simplification.
    Fold,
    /// Dead code elimination.
    Dce,
    /// Control-flow graph simplification.
    SimplifyCfg,
    /// Global value numbering over the dominator tree.
    Gvn,
    /// Loop-invariant code motion.
    Licm,
    /// Stack-slot promotion (mem2reg).
    Promote,
    /// Sparse conditional constant propagation.
    Sccp,
}

impl Pass {
    /// Every pass, in default pipeline order.
    pub(crate) const ALL: [Self; Self::COUNT] = [
        Self::Promote,
        Self::Fold,
        Self::SimplifyCfg,
        Self::Sccp,
        Self::Gvn,
        Self::Licm,
        Self::Dce,
    ];
    pub(crate) const COUNT: usize = 7;

    /// The name `--passes` and reports use.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Fold => "fold",
            | Self::Dce => "dce",
            | Self::SimplifyCfg => "simplify-cfg",
            | Self::Gvn => "gvn",
            | Self::Licm => "licm",
            | Self::Promote => "promote",
            | Self::Sccp => "sccp",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pass| pass.name() == name)
    }

    /// A dense index for per-pass tables.
    pub(crate) const fn index(self) -> usize {
        match self {
            | Self::Fold => 0,
            | Self::Dce => 1,
            | Self::SimplifyCfg => 2,
            | Self::Gvn => 3,
            | Self::Licm => 4,
            | Self::Promote => 5,
            | Self::Sccp => 6,
        }
    }
}

impl fmt::Display for Pass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An explicit list of passes: a short, fixed-capacity array, so that parsing
/// one allocates nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PassList {
    passes: [Pass; Self::CAPACITY],
    len:    usize,
}

impl PassList {
    /// The most passes a list holds.
    pub(crate) const CAPACITY: usize = 32;

    pub(crate) fn as_slice(&self) -> &[Pass] {
        &self.passes[..self.len]
    }

    /// The list of `passes`, or `None` if there are more than
    /// [`PassList::CAPACITY`].
    pub(crate) fn new(passes: &[Pass]) -> Option<Self> {
        let mut list = Self {
            passes: [Pass::Fold; Self::CAPACITY],
            len:    0,
        };
        list.passes.get_mut(..passes.len())?.copy_from_slice(passes);
        list.len = passes.len();
        Some(list)
    }
}

/// Reads a `--passes` value: pass names separated by commas, such as
/// `fold,dce,simplify-cfg`. A pass may repeat. The empty string is the empty
/// list, which runs nothing.
pub(crate) fn parse_pass_list(text: &str) -> Result<PassList, PassListError<'_>> {
    let mut passes = [Pass::Fold; PassList::CAPACITY];
    let mut len = 0;
    if !text.trim().is_empty() {
        for name in text.split(',') {
            let name = name.trim();
            let pass = Pass::from_name(name).ok_or(PassListError::Unknown(name))?;
            *passes.get_mut(len).ok_or(PassListError::TooMany)? = pass;
            len += 1;
        }
    }
    Ok(PassList { passes, len })
}

/// A `--passes` value that is not a list of known passes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub(crate) enum PassListError<'a> {
    /// A name that is empty or not a pass; the known passes are `fold`,
    /// `dce`, `simplify-cfg`, `gvn`, `licm`, `promote` and `sccp`.
    #[error(
        "unknown pass `{0}` (the passes are fold, dce, simplify-cfg, gvn, licm, promote and sccp)"
    )]
    Unknown(&'a str),
    #[error("a pass list holds at most {} passes", PassList::CAPACITY)]
    TooMany,
}

impl fmt::Debug for OptimizerOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OptimizerOptions")
            .field("passes", &self.passes)
            .field("verify_each", &self.verify_each)
            .field("print_after_all", &self.print_after_all.is_some())
            .field("bisect_limit", &self.bisect_limit)
            .finish_non_exhaustive()
    }
}
