//! Analyzer initialization and continuation-stack invariants for translation
//! phase 7. Recovered syntax taints only its scheduled work; declaration and
//! expression constraints remain in their concern modules.
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.

#[cfg(test)]
use std::cell::Cell;

use rustc_hash::FxBuildHasher;

use super::{
    Analyzer,
    Scope,
    ScopeKind,
    Work,
    collection::Collection,
    functions,
    statements,
    types::{
        TypeId,
        TypeInterner,
    },
};
use crate::{
    translation_phases::Context,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

impl<'c, 'tu, 's> Analyzer<'c, 'tu, 's> {
    pub(super) fn new(context: &'c mut Context<'tu>, scratch: &'s Bump) -> Self {
        let tu = context.tu_arena();
        let target = context.configuration.target().layout();
        let mut analyzer = Self {
            #[cfg(test)]
            review_steps: Cell::new([0; 3]),
            context,
            scratch,
            types: TypeInterner::new(tu, scratch, &target),
            bindings: ArenaVec::new_in(tu),
            definitions: ArenaVec::new_in(tu),
            scopes: ArenaVec::new_in(tu),
            type_names: ArenaVec::new_in(tu),
            parameter_lists: ArenaVec::new_in(tu),
            scope: 0,
            entries: ArenaVec::new_in(scratch),
            visible: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            external: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            scope_entries: ArenaVec::new_in(scratch),
            parameters: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            work: ArenaVec::new_in(scratch),
            values: ArenaVec::new_in(scratch),
            integers: ArenaVec::new_in(scratch),
            tainted: false,
            function_name: None,
            old_parameter_mode: false,
            runtime_bound: false,
            semantic_errors: 0,
            member_indices: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            member_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            tag_declarations: ArenaVec::new_in(tu),
            resolved_type_names: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            integer_models: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            expressions: ArenaVec::new_in(tu),
            expression_indices: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            conversions: ArenaVec::new_in(tu),
            const_members: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            register_bindings: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            ice_operands: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            functions: functions::State::new(scratch),
            va_list_type: None,
            statements: statements::State::new(scratch),
            defining: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            enum_ranges: ArenaMap::with_hasher_in(FxBuildHasher, scratch),
        };
        analyzer.scopes.push(Scope {
            parent: None,
            kind:   ScopeKind::File,
        });
        analyzer
            .scope_entries
            .push(scratch.alloc(Collection::new()));
        analyzer
    }

    /// Every external declaration starts and ends with empty continuation and
    /// value stacks, so an unbalanced task cannot silently feed the next root.
    pub(super) fn assert_balanced(&self) {
        debug_assert!(self.work.is_empty(), "no continuation outlives its root");
        debug_assert!(self.values.is_empty(), "no type value outlives its root");
        debug_assert!(
            self.integers.is_empty(),
            "no integer value outlives its root"
        );
    }

    pub(super) fn take_type(&mut self) -> TypeId {
        debug_assert!(!self.values.is_empty(), "a type continuation has a value");
        self.values.pop().unwrap_or_else(|| self.types.unknown())
    }

    pub(super) fn taint(&mut self, recovered: bool) {
        self.work.push(Work::RestoreTaint(self.tainted));
        self.tainted |= recovered;
    }

    #[cfg(test)]
    pub(super) fn review_step(&self, kind: usize) {
        let mut steps = self.review_steps.get();
        steps[kind] += 1;
        self.review_steps.set(steps);
    }
}
