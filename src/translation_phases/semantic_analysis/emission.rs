//! Phase-7 model of what Clang's code generation emits without optimizing,
//! for the forms the statement walk cannot decide on its own. Each sets the
//! walk's `dead` flag (`functions.rs`, `State::dead`), which approximates
//! Clang's insertion state: a reference in code Clang skips creates no LLVM
//! value, so it ranks no function in Clang's creation order and selects no
//! DLL import. Semantic analysis still runs in source order; what these
//! rules learn late they apply to the references already recorded, which
//! only marks more code skipped.
//!
//! - **Folding.** One evaluator stands for Clang's
//!   `ConstantFoldsToSimpleInteger`, which decides every condition the flag
//!   folds: `if`, `switch`, scalar `?:`, `&&` and `||` (`folding.rs`).
//! - **Folded switches.** A switch whose condition folds emits only the
//!   statements `CollectStatementsForCase` gathers from the selected label
//!   (`switches.rs`), with the labels and breaks each statement holds,
//!   statement expressions included (`labels.rs`).
//! - **Calls that do not return.** A call through a `noreturn` function or
//!   function type leaves a block without predecessors, which ends its
//!   expression statement's code; an expression that joins two paths leaves a
//!   block with predecessors again (`noreturn.rs`, `insertion.rs`).
//!
//! For `switch (0) { case 1: h(0); break; default: g(0); }`, the condition
//! folds to 0. The walk analyzes `case 1` as in any switch; at `default`,
//! which no case precedes as a match, Clang's collection from `default`
//! succeeds, so the reference to `h` recorded before it is marked skipped
//! and `default` resumes the code around the switch. For `die(), g(0);
//! h(0);` with `die` declared `noreturn`, `g` is still referenced, but `h`
//! is not.
//!
//! Read [`State`], then `switches.rs`, `insertion.rs`, `noreturn.rs`,
//! `folding.rs` and `labels.rs`.
//!
//! C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146 (switch selection);
//! §6.8.6.3 paragraph 2, p. 138; PDF p. 150 (break); §6.8.3 paragraph 2,
//! p. 132; PDF p. 144 (expression statements); §6.6 paragraph 10, p. 96;
//! PDF p. 108 (other constant forms).
//! C11: §6.7.4 paragraph 8, p. 126; PDF p. 144 (`_Noreturn`).
//! GNU extension: GCC manual, "Common Function Attributes", `noreturn`.
//! <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>

// Rules
mod folding;
mod insertion;
mod noreturn;
mod switches;

// Statement facts
mod labels;

use rustc_hash::FxBuildHasher;

use crate::{
    translation_phases::parsing::{
        declaration_syntax::Initializer,
        syntax::Expression,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// The emission model's state for the translation unit, kept beside the
/// walk's `dead` flag.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact of the emission model's state."
)]
pub(super) struct State<'tu, 's> {
    /// Code generation emits the code being analyzed into a block without
    /// predecessors, which a call that does not return created during the
    /// expression statement numbered here or a later one.
    pub(super) orphan:       Option<usize>,
    /// The number of expression statements begun so far.
    pub(super) serial:       usize,
    /// The number of expression statements begun and not yet ended.
    open_statements:         usize,
    /// The joins in `sizeof`, `alignof` and `typeof` operands of the
    /// expression statements being analyzed, each with the number of its
    /// statement and its sizeof and reference contexts, which tell once the
    /// operands are analyzed whether code generation emits them.
    context_joins:           ArenaVec<'s, (usize, Option<usize>, Option<usize>)>,
    /// The value statements of the statement expressions being analyzed,
    /// by address, which Clang emits even after a jump.
    value_statements:        ArenaMap<'s, usize, ()>,
    /// Bindings that call nothing that returns: a function declared
    /// `noreturn`, or an object, parameter or typedef whose function type
    /// is, directly or by an earlier declaration of the same entity. The
    /// value says whether the fact belongs to the type (GNU, `gnu::`,
    /// Microsoft, or Clang's builtin type), which `__typeof__` and typedefs
    /// carry, rather than to the declaration (`_Noreturn`, `[[noreturn]]`).
    pub(super) noreturn:     ArenaMap<'s, usize, bool>,
    /// Bindings whose function type's result is a pointer to a `noreturn`
    /// function type: a function, a pointer to one, an array of such
    /// pointers, or a typedef of one.
    noreturn_results:        ArenaMap<'s, usize, ()>,
    /// Members whose function type does not return, by tag and name.
    noreturn_fields:         ArenaMap<'s, (usize, StringCacheId), ()>,
    /// File-scope typedefs named `jmp_buf` and `sigjmp_buf` were declared,
    /// which Clang's `longjmp` builtins need.
    jmp_buf:                 bool,
    sigjmp_buf:              bool,
    /// The switches being analyzed whose condition folds, by switch index.
    folds:                   ArenaMap<'s, usize, switches::FoldedSwitch<'tu>>,
    /// The skipped state a folded switch's selected statements ended in,
    /// passed from the end of its body to the code after it.
    pub(super) switch_after: Option<bool>,
    /// What each walked statement holds, by address ([`labels`]).
    holds:                   ArenaMap<'s, usize, labels::Holds>,
    /// The case labels of each walked switch, by address.
    switches:                ArenaMap<'s, usize, labels::SwitchLabels<'tu, 's>>,
    /// The position of each walked case label among its switch's labels.
    label_positions:         ArenaMap<'s, usize, usize>,
    /// The functions walked so far, by address.
    walked:                  ArenaMap<'s, usize, ()>,
    /// The values the evaluator found, by expression address, with the
    /// depth of a query that `folding::DEPTH` cut short and the weak
    /// horizon of one that read a weak name.
    folded:                  ArenaMap<'s, usize, folding::Memo<Option<folding::Value>>>,
    /// A conditional's arm boundaries and insertion states, resolved after
    /// its result type distinguishes scalar from complex/aggregate emission.
    conditional_marks:       ArenaMap<'s, usize, [(usize, Option<usize>, usize); 3]>,
    /// The evaluation in progress was cut short by `folding::DEPTH`.
    depth_cut:               bool,
    /// Whether an expression holds a label, by address.
    labeled_expressions:     ArenaMap<'s, usize, bool>,
    /// Whether Clang's evaluator may fold an expression differently from
    /// semantic analysis, by address, with the weak horizon of an answer
    /// that read a weak name.
    refolds:                 ArenaMap<'s, usize, (bool, Option<usize>)>,
    /// The names that a declaration anywhere in the unit declares `weak`
    /// or `weak_import`, or that `#pragma weak` names, each with the
    /// output-token ordinal from which it is weak (`folding.rs`).
    weak_names:              ArenaMap<'s, StringCacheId, usize>,
    /// The output-token ordinal past the root being analyzed.
    pub(super) root_end:     usize,
    /// The weak names that apply to the code being analyzed: those before
    /// this ordinal. Clang emits an external function that no declaration
    /// makes inline where it is defined, and every other body after the
    /// whole unit, so this is the root's end in such a function and
    /// unbounded elsewhere.
    pub(super) weak_horizon: usize,
    /// The evaluation in progress read whether a name in `weak_names`
    /// applies, so that its result holds only at the same horizon.
    weak_read:               bool,
    /// The initializers of `const` non-volatile objects, which Clang's
    /// evaluator may read.
    constant_initializers:   ArenaMap<'s, usize, &'tu Expression<'tu>>,
    /// The initializers of `const` non-volatile arrays and structures, by
    /// binding, whose retained plans Clang's evaluator may read.
    constant_aggregates:     ArenaMap<'s, usize, &'tu Initializer<'tu>>,
    /// The byte ranges of each read aggregate and the operation that stores
    /// each, by binding, with the depth of a query `folding::DEPTH` cut
    /// short (`folding.rs`).
    plan_covers:             ArenaMap<'s, usize, folding::Memo<Option<folding::Cover<'s>>>>,
    /// Cached constant-array strlen queries, including failed scans, with
    /// the same depth and weak-horizon dependencies as other folds.
    array_lengths: ArenaMap<'s, (usize, u64, super::TypeId), folding::Memo<Option<folding::Value>>>,
    /// The elements of each string literal the evaluator reads, by literal
    /// and encoding.
    literal_units:           ArenaMap<'s, (u32, u8), &'s [u32]>,
    /// The block items on the work stack that a variably modified lookahead
    /// has seen: each one's position and address, and whether it or an
    /// item below it may declare something of a variably modified type.
    variably_modified:       ArenaVec<'s, (usize, usize, bool)>,
    /// The lowest length the work stack has had since the last variably
    /// modified lookahead, below which that lookahead's answers hold.
    work_low:                usize,
    /// Whether each block item a variably modified lookahead examined, or
    /// an item inside it, may declare something of a variably modified
    /// type, by address.
    variably_modified_items: ArenaMap<'s, usize, bool>,
    /// Whether an expression or one in it may join paths, by address
    /// (`insertion.rs`).
    may_join:                ArenaMap<'s, usize, bool>,
    /// The functions whose block-scope typedefs have been read, by address.
    vm_typedef_functions:    ArenaMap<'s, usize, ()>,
    /// The names a block-scope typedef may give a variably modified type,
    /// by function address and name.
    vm_typedefs:             ArenaMap<'s, (usize, StringCacheId), ()>,
    /// The state [`State::orphan`] had where a call of a builtin that
    /// leaves its operands unemitted began, by the address of its callee.
    unevaluated_calls:       ArenaMap<'s, usize, Option<usize>>,
}

impl<'s> State<'_, 's> {
    /// Notes that the work stack has `length` items after a pop.
    pub(super) fn work_popped(&mut self, length: usize) {
        self.work_low = self.work_low.min(length);
    }

    pub(super) fn new(scratch: &'s Bump) -> Self {
        Self {
            orphan:                  None,
            serial:                  0,
            open_statements:         0,
            context_joins:           ArenaVec::new_in(scratch),
            value_statements:        ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            noreturn:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            noreturn_results:        ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            noreturn_fields:         ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            jmp_buf:                 false,
            sigjmp_buf:              false,
            folds:                   ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            switch_after:            None,
            holds:                   ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            switches:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            label_positions:         ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            walked:                  ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            folded:                  ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            conditional_marks:       ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            depth_cut:               false,
            refolds:                 ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            labeled_expressions:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            weak_names:              ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            root_end:                usize::MAX,
            weak_horizon:            usize::MAX,
            weak_read:               false,
            constant_initializers:   ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            constant_aggregates:     ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            plan_covers:             ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            array_lengths:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            literal_units:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            variably_modified:       ArenaVec::new_in(scratch),
            work_low:                0,
            variably_modified_items: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            may_join:                ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            vm_typedef_functions:    ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            vm_typedefs:             ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            unevaluated_calls:       ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        }
    }
}
