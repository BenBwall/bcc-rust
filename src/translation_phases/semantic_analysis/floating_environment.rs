//! The floating-point environment the pragmas and attributes give each
//! function, which decides who computes its invalid operations
//! ([`super::constants::NanSource`]). bcc does not model a function whose
//! environment one of them might make strict: it computes the function's
//! invalid operations at run time, as it did before it folded their NaNs,
//! wherever the compiler it follows might.
//!
//! - **Clang's rules.** `#pragma STDC FENV_ACCESS ON`, Microsoft's
//!   `fenv_access(on)`, a strict or `maytrap` exception behavior (`clang fp
//!   exceptions`, `float_control(except, on)`, or `-ftrapping-math` that no
//!   pragma overrides), and a constant `FENV_ROUND` direction other than to
//!   nearest each make Clang compute invalid operations at run time, at least
//!   at `-O0`. `float_control(push)` and `(pop)` save and restore all of them.
//!   A pragma inside a function lasts to the end of its compound statement,
//!   which bcc widens to the whole function. `cl` reads the same pragmas, so
//!   they count for its identity too.
//! - **GCC's rules.** An `optimize` attribute or `#pragma GCC optimize` can
//!   turn trapping math back on for a function (`trapping-math`,
//!   `no-fast-math`, or an `-O` level after `-Ofast`), so a function any of its
//!   declarations has one on keeps its run-time NaNs.
//!
//! C99: §7.6.1 paragraph 2, p. 189; PDF p. 201; §F.7.4 paragraph 1, p.
//! 449; PDF p. 461. C23: §7.6.2 paragraph 2, p. 213; PDF p. 226. Clang
//! extension: Clang Language Extensions, "Extensions to specify
//! floating-point flags". GNU extension: GCC manual, "Function Specific
//! Option Pragmas".

use super::{
    Analyzer,
    facts::EntityId,
};
use crate::{
    configuration::{
        CompilerIdentity,
        RuleSet,
    },
    translation_phases::pragma_floating::{
        ExceptionBehavior,
        FloatingPragma,
    },
    util::bump::{
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

impl Analyzer<'_, '_, '_> {
    /// Enters the root whose output tokens run from `start` to `end`: reads
    /// the pragmas between the previous root and this one, which set the
    /// file-scope state, then those inside it. The root may have a strict
    /// environment if the file-scope state is strict or a pragma inside it
    /// can make it so, and is under `#pragma GCC optimize` if that is in
    /// effect at file scope or written inside it.
    pub(super) fn enter_floating_root(&mut self, start: usize, end: usize) {
        let pragmas = &self.context.floating_pragmas;
        let trapping = self.context.configuration.requests_trapping_math();
        let state = &mut self.functions.floating;
        while let Some(&(position, pragma)) = pragmas.get(state.next)
            && position <= start
        {
            state.file_scope(pragma);
            state.next += 1;
        }
        let root = &mut state.root;
        *root = RootFacts::default();
        root.add_if(
            RootFacts::STRICT,
            state.unread || state.clang.strict(trapping),
        );
        root.add_if(
            RootFacts::DIRECTED,
            state.unread || state.clang.directed_rounding,
        );
        root.add_if(RootFacts::OPTIMIZED, state.gcc_optimize);
        while let Some(&(position, pragma)) = pragmas.get(state.next)
            && position < end
        {
            let root = &mut state.root;
            root.add_if(
                RootFacts::DIRECTED,
                matches!(
                    pragma,
                    FloatingPragma::DirectedRounding(true) | FloatingPragma::Unread
                ),
            );
            match pragma {
                | FloatingPragma::Pop => {
                    let saved = &state.saved;
                    root.add_if(RootFacts::STRICT, saved.iter().any(|s| s.strict(trapping)));
                    root.add_if(
                        RootFacts::DIRECTED,
                        saved.iter().any(|s| s.directed_rounding),
                    );
                },
                | FloatingPragma::GccOptimize => root.add_if(RootFacts::OPTIMIZED, true),
                | FloatingPragma::GccPopOptions =>
                    root.add_if(RootFacts::OPTIMIZED, state.gcc_saved.contains(&true)),
                | pragma => root.add_if(RootFacts::STRICT, ClangFloating::makes_strict(pragma)),
            }
            state.next += 1;
        }
    }

    /// Whether the function analyzed now may have a strict exception
    /// behavior under Clang's rules (and `cl`'s pragmas), so that its
    /// invalid operations are computed at run time.
    pub(super) fn clang_strict_exceptions(&self) -> bool {
        self.functions.floating.root.contains(RootFacts::STRICT)
    }

    /// Whether [`Self::settle_strict_initializer`] may fold an automatic
    /// aggregate's NaNs in the function analyzed now: its rules are Clang's
    /// evaluator's under strict exceptions, to nearest or in a dynamic
    /// direction. Under a constant directed rounding, or a pragma bcc could
    /// not read, that evaluator refuses other operations (an exact
    /// subnormal result, for one), so every NaN stays at run time.
    pub(super) fn strict_aggregates_settle(&self) -> bool {
        !self.functions.floating.root.contains(RootFacts::DIRECTED)
    }

    /// Whether the function analyzed now computes its invalid operations at
    /// run time under GCC's rules although trapping math is off on the
    /// command line: an `optimize` attribute or `#pragma GCC optimize`
    /// reached it, or, for `cl`, one of the pragmas Clang's rules read.
    pub(super) fn gcc_function_runtime(&self) -> bool {
        let root = self.functions.floating.root;
        root.contains(RootFacts::OPTIMIZED)
            || root.contains(RootFacts::OPTIMIZED_FUNCTION)
            || (self.context.configuration.compiler_identity() == CompilerIdentity::Msvc
                && root.contains(RootFacts::STRICT))
    }

    /// Enters the definition of function `binding`, once its own
    /// declaration's attributes are applied: notes whether an `optimize`
    /// attribute or `#pragma GCC optimize` reached any of its declarations.
    /// A GNU nested function adds to the function around it.
    pub(super) fn enter_floating_function(&mut self, binding: Option<usize>) {
        let Some(binding) = binding else {
            return;
        };
        let entity = self.entity_of(binding);
        let floating = &mut self.functions.floating;
        let optimized = floating.optimized.contains(&entity);
        floating
            .root
            .add_if(RootFacts::OPTIMIZED_FUNCTION, optimized);
    }

    /// Notes that function `binding` is declared with an `optimize`
    /// attribute, or (`attribute` false) here, where `#pragma GCC optimize`
    /// may be in effect, under GCC's rules.
    pub(super) fn note_optimized_function(&mut self, binding: usize, attribute: bool) {
        let rules = self.context.configuration.compiler_identity().rules();
        let pragma = self.functions.floating.root.contains(RootFacts::OPTIMIZED);
        if rules != RuleSet::Gcc || !(attribute || pragma) {
            return;
        }
        let entity = self.entity_of(binding);
        _ = self.functions.floating.optimized.insert(entity);
    }
}

/// The pragma state as the roots are analyzed.
pub(super) struct FloatingEnvironment<'s> {
    /// Clang's state at file scope after the pragmas read so far.
    clang:        ClangFloating,
    /// What `float_control(push)` saved at file scope, innermost last.
    saved:        ArenaVec<'s, ClangFloating>,
    /// A pragma bcc could not read was seen at file scope, so any later
    /// function may be strict.
    unread:       bool,
    /// `#pragma GCC optimize` is in effect at file scope.
    gcc_optimize: bool,
    /// What `#pragma GCC push_options` saved, innermost last.
    gcc_saved:    ArenaVec<'s, bool>,
    /// The first pragma not yet read, in `Context::floating_pragmas`.
    next:         usize,
    /// What may hold in the root being analyzed.
    root:         RootFacts,
    /// The functions an `optimize` attribute or `#pragma GCC optimize`
    /// reached on any declaration.
    optimized:    ArenaSet<'s, EntityId>,
}

impl<'s> FloatingEnvironment<'s> {
    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            clang:        ClangFloating::default(),
            saved:        ArenaVec::new_in(scratch),
            unread:       false,
            gcc_optimize: false,
            gcc_saved:    ArenaVec::new_in(scratch),
            next:         0,
            root:         RootFacts::default(),
            optimized:    ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
        }
    }

    /// Applies a pragma read between roots.
    fn file_scope(&mut self, pragma: FloatingPragma) {
        use FloatingPragma as P;
        match pragma {
            | P::FenvAccess(on) => self.clang.fenv_access = on,
            | P::Exceptions(behavior) => self.clang.exceptions = Some(behavior),
            | P::DirectedRounding(directed) => self.clang.directed_rounding = directed,
            | P::Push => self.saved.push(self.clang),
            | P::Pop =>
                if let Some(saved) = self.saved.pop() {
                    self.clang = saved;
                },
            | P::Unread => self.unread = true,
            | P::GccOptimize => self.gcc_optimize = true,
            | P::GccPushOptions => self.gcc_saved.push(self.gcc_optimize),
            | P::GccPopOptions =>
                if let Some(saved) = self.gcc_saved.pop() {
                    self.gcc_optimize = saved;
                },
            | P::GccResetOptions => self.gcc_optimize = false,
        }
    }
}

/// What may hold in the root being analyzed, as a set of bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RootFacts(u8);

impl RootFacts {
    /// A constant directed rounding, or a pragma bcc could not read, may be
    /// in effect.
    const DIRECTED: Self = Self(1 << 1);
    /// `#pragma GCC optimize` may be in effect.
    const OPTIMIZED: Self = Self(1 << 2);
    /// The function defined in the root is in
    /// [`FloatingEnvironment::optimized`].
    const OPTIMIZED_FUNCTION: Self = Self(1 << 3);
    /// The root may have a strict environment under Clang's rules.
    const STRICT: Self = Self(1);

    const fn contains(self, fact: Self) -> bool {
        self.0 & fact.0 != 0
    }

    /// Adds `fact` when `condition` holds.
    const fn add_if(&mut self, fact: Self, condition: bool) {
        if condition {
            self.0 |= fact.0;
        }
    }
}

/// The state of Clang's floating-point pragmas.
#[derive(Clone, Copy, Debug, Default)]
struct ClangFloating {
    /// `FENV_ACCESS` (or `fenv_access`) is on.
    fenv_access:       bool,
    /// The exception behavior a pragma set; `None` keeps the command
    /// line's.
    exceptions:        Option<ExceptionBehavior>,
    /// `FENV_ROUND` may set a direction other than to nearest.
    directed_rounding: bool,
}

impl ClangFloating {
    /// Whether Clang computes invalid operations at run time here, at least
    /// at `-O0`; `trapping` is the command line's `-ftrapping-math`.
    fn strict(self, trapping: bool) -> bool {
        self.fenv_access
            || self.directed_rounding
            || match self.exceptions {
                | Some(behavior) => behavior != ExceptionBehavior::Ignore,
                | None => trapping,
            }
    }

    /// Whether `pragma` makes the state strict whatever it was before.
    fn makes_strict(pragma: FloatingPragma) -> bool {
        use FloatingPragma as P;
        match pragma {
            | P::FenvAccess(on) | P::DirectedRounding(on) => on,
            | P::Exceptions(behavior) => behavior != ExceptionBehavior::Ignore,
            | P::Unread => true,
            | P::Push
            | P::Pop
            | P::GccOptimize
            | P::GccPushOptions
            | P::GccPopOptions
            | P::GccResetOptions => false,
        }
    }
}
