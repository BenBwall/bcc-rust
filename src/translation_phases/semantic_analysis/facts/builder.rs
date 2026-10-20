//! Traversal-time region cursor and compact edge production. The analyzer's
//! existing work stack enters/leaves regions; no auxiliary syntax walk exists.
//! C99: §6.2.4p6, p. 32; PDF p. 44; §6.8.6.1p1, p. 137; PDF p. 149.

use super::{
    BindingId,
    CaseFacts,
    CleanupKind,
    ControlFacts,
    ControlPoint,
    ControlTarget,
    FunctionControl,
    FunctionId,
    JumpFacts,
    JumpId,
    LabelFacts,
    LabelId,
    LoopFacts,
    Region,
    RegionId,
    RegionKind,
    ScopeId,
    StatementFacts,
    StatementId,
    SwitchFacts,
    Transition,
    ancestry::{
        Ancestry,
        ancestor,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            Statement,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

pub(in crate::translation_phases::semantic_analysis) struct Builder<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) regions: ArenaVec<'s, Region>,
    pub(in crate::translation_phases::semantic_analysis) current: Option<RegionId>,
    pub(in crate::translation_phases::semantic_analysis) scope_regions:
        ArenaVec<'s, Option<RegionId>>,
    pub(in crate::translation_phases::semantic_analysis) statements:
        ArenaVec<'s, StatementFacts<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) labels: ArenaVec<'s, LabelFacts<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) jumps: ArenaVec<'s, JumpFacts>,
    pub(in crate::translation_phases::semantic_analysis) loops: ArenaVec<'s, LoopFacts>,
    pub(in crate::translation_phases::semantic_analysis) switches: ArenaVec<'s, SwitchFacts<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) cases: ArenaVec<'s, CaseFacts<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) case_runs:
        ArenaVec<'s, std::ops::Range<usize>>,
    pub(in crate::translation_phases::semantic_analysis) targets:
        ArenaVec<'s, (StatementId, Option<usize>)>,
    pub(in crate::translation_phases::semantic_analysis) deferred_vm_labels:
        ArenaVec<'s, (usize, RegionId, RegionId)>,
    loop_bodies: ArenaMap<'s, usize, usize>,
    statement_indices: ArenaMap<'s, usize, StatementId>,
    expression_points: ArenaMap<'s, usize, ControlPoint>,
    vla_regions: ArenaMap<'s, BindingId, RegionId>,
    address_taken: ArenaMap<'s, FunctionId, ArenaVec<'s, LabelId>>,
    label_addresses: ArenaMap<'s, usize, LabelId>,
    label_differences: ArenaMap<'s, usize, (LabelId, LabelId)>,
    normal_destinations: ArenaVec<'s, (RegionId, RegionId)>,
    pub(in crate::translation_phases::semantic_analysis) ancestry: Option<Ancestry<'s>>,
}

impl<'tu, 's> Builder<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) fn open(
        &mut self,
        scope: usize,
        kind: RegionKind,
        cleanup: CleanupKind,
        source: SourceVectors,
    ) -> Option<RegionId> {
        let parent = self.current?;
        let id = RegionId(self.regions.len());
        let previous = self.regions[parent.0];
        self.regions.push(Region {
            parent: Some(parent),
            lexical_scope: ScopeId(scope),
            function: previous.function,
            kind,
            cleanup,
            source,
            preorder: id.0,
            subtree_end: usize::MAX,
            nearest_vm: if matches!(kind, RegionKind::VmDeclaration(_)) {
                Some(id)
            } else {
                previous.nearest_vm
            },
            nearest_cleanup: if cleanup == CleanupKind::None {
                previous.nearest_cleanup
            } else {
                Some(id)
            },
            nearest_cleanup_declaration: if matches!(kind, RegionKind::CleanupDeclaration(_)) {
                Some(id)
            } else {
                previous.nearest_cleanup_declaration
            },
            nearest_statement_expression: if matches!(kind, RegionKind::StatementExpression(_)) {
                Some(id)
            } else {
                previous.nearest_statement_expression
            },
        });
        if let CleanupKind::AutomaticVla(binding) = cleanup {
            _ = self.vla_regions.insert(binding, id);
        }
        self.current = Some(id);
        Some(id)
    }

    pub(in crate::translation_phases::semantic_analysis) fn start_function(
        &mut self,
        function: usize,
        scope: usize,
        source: SourceVectors,
    ) -> FunctionControl {
        let root = RegionId(self.regions.len());
        let id = FunctionId(function);
        self.regions.push(Region {
            parent: None,
            lexical_scope: ScopeId(scope),
            function: id,
            kind: RegionKind::Lexical,
            cleanup: CleanupKind::None,
            source,
            preorder: root.0,
            subtree_end: usize::MAX,
            nearest_vm: None,
            nearest_cleanup: None,
            nearest_cleanup_declaration: None,
            nearest_statement_expression: None,
        });
        self.current = Some(root);
        FunctionControl { id, root }
    }

    pub(in crate::translation_phases::semantic_analysis) fn close_to(
        &mut self,
        previous: Option<RegionId>,
    ) {
        while let Some(current) = self.current {
            if Some(current) == previous {
                break;
            }
            self.regions[current.0].subtree_end = self.regions.len();
            self.current = self.regions[current.0].parent;
        }
        self.current = previous;
    }

    pub(in crate::translation_phases::semantic_analysis) fn point(&self) -> Option<ControlPoint> {
        self.current.map(|region| ControlPoint {
            region,
            function: self.regions[region.0].function,
        })
    }

    pub(in crate::translation_phases::semantic_analysis) fn statement(
        &mut self,
        syntax: &'tu Statement<'tu>,
    ) -> Option<StatementId> {
        let entry = self.point()?;
        if let Some(&index) = self.loop_bodies.get(&std::ptr::from_ref(syntax).addr()) {
            self.loops[index].body = entry;
        }
        let key = std::ptr::from_ref(syntax).addr();
        if let Some(&id) = self.statement_indices.get(&key) {
            return Some(id);
        }
        let id = StatementId(self.statements.len());
        self.statements.push(StatementFacts {
            generated: super::GeneratedControl::Runtime,
            elided: false,
            id,
            syntax,
            entry,
            exit: Transition::empty(entry.region),
            label: None,
            jump: None,
            switch: None,
        });
        self.normal_destinations.push((entry.region, entry.region));
        _ = self.statement_indices.insert(key, id);
        Some(id)
    }

    pub(in crate::translation_phases::semantic_analysis) fn statement_id(
        &self,
        syntax: &Statement<'_>,
    ) -> Option<StatementId> {
        self.statement_indices
            .get(&std::ptr::from_ref(syntax).addr())
            .copied()
    }

    pub(in crate::translation_phases::semantic_analysis) fn loop_body(
        &mut self,
        syntax: &Statement<'_>,
        index: usize,
    ) {
        _ = self
            .loop_bodies
            .insert(std::ptr::from_ref(syntax).addr(), index);
    }

    pub(in crate::translation_phases::semantic_analysis) fn expression(
        &mut self,
        syntax: &Expression<'_>,
    ) {
        if let Some(point) = self.point() {
            _ = self
                .expression_points
                .insert(std::ptr::from_ref(syntax).addr(), point);
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn normal_exit(
        &mut self,
        statement: &Statement<'_>,
        destination: RegionId,
    ) {
        if let Some(id) = self.statement_id(statement) {
            self.normal_destinations[id.0] =
                (self.current.expect("statement has a region"), destination);
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn jump(
        &mut self,
        statement: &Statement<'_>,
        target: ControlTarget,
        destination: ControlPoint,
    ) {
        let Some(id) = self.statement_id(statement) else {
            return;
        };
        let source = self.point().expect("statement has a region");
        let jump = JumpId(self.jumps.len());
        self.jumps.push(JumpFacts {
            source,
            target,
            destination,
            transition: Transition::empty(source.region),
            provenance: statement.source_vectors,
        });
        self.statements[id.0].jump = Some(jump);
    }

    /// Records that `expression` takes the address of `label`, numbering the
    /// label among its function's address-taken labels on first use.
    /// GNU extension: GCC "Labels as Values".
    pub(in crate::translation_phases::semantic_analysis) fn take_address(
        &mut self,
        expression: &Expression<'_>,
        label: LabelId,
        scratch: &'s Bump,
    ) {
        _ = self
            .label_addresses
            .insert(std::ptr::from_ref(expression).addr(), label);
        let facts = &mut self.labels[label.0];
        if facts.address.is_some() {
            return;
        }
        let taken = self
            .address_taken
            .entry(facts.function)
            .or_insert_with(|| ArenaVec::new_in(scratch));
        facts.address = Some(u32::try_from(taken.len()).expect("fewer than 2^32 labels"));
        taken.push(label);
    }

    /// The label an `&&label` expression names, once taken.
    pub(in crate::translation_phases::semantic_analysis) fn address_label(
        &self,
        expression: &Expression<'_>,
    ) -> Option<LabelId> {
        self.label_addresses
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    /// The labels of a constant label difference recorded for `expression`.
    pub(in crate::translation_phases::semantic_analysis) fn difference_labels(
        &self,
        expression: &Expression<'_>,
    ) -> Option<(LabelId, LabelId)> {
        self.label_differences
            .get(&std::ptr::from_ref(expression).addr())
            .copied()
    }

    /// Records that `expression` is the constant label difference
    /// `label - base`.
    pub(in crate::translation_phases::semantic_analysis) fn record_difference(
        &mut self,
        expression: &Expression<'_>,
        labels: (LabelId, LabelId),
    ) {
        _ = self
            .label_differences
            .insert(std::ptr::from_ref(expression).addr(), labels);
    }

    /// Every address-taken label of `function`, in address order.
    pub(in crate::translation_phases::semantic_analysis) fn taken(
        &self,
        function: FunctionId,
    ) -> &[LabelId] {
        self.address_taken
            .get(&function)
            .map_or(&[], |labels| labels.as_slice())
    }

    pub(in crate::translation_phases::semantic_analysis) fn forbidden_vm(
        &self,
        from: RegionId,
        to: RegionId,
    ) -> Option<RegionId> {
        self.regions[to.0]
            .nearest_vm
            .filter(|&vm| !ancestor(&self.regions, vm, from))
    }

    pub(in crate::translation_phases::semantic_analysis) fn forbidden_expression(
        &self,
        from: RegionId,
        to: RegionId,
    ) -> Option<SourceVectors> {
        self.regions[to.0]
            .nearest_statement_expression
            .filter(|&expression| !ancestor(&self.regions, expression, from))
            .map(|expression| self.regions[expression.0].source)
    }

    pub(in crate::translation_phases::semantic_analysis) fn finalize(
        &mut self,
        scratch: &'s Bump,
        tu: &'tu Bump,
    ) {
        let ancestry = Ancestry::new(&self.regions, scratch);
        for (i, &(from, to)) in self.normal_destinations.iter().enumerate() {
            self.statements[i].exit = ancestry.transition(&self.regions, from, to);
        }
        for jump in &mut self.jumps {
            if let ControlTarget::Label(id) = jump.target
                && let Some(point) = self.labels[id.0].point
                && point.function == jump.source.function
            {
                jump.destination = point;
            }
            jump.transition =
                ancestry.transition(&self.regions, jump.source.region, jump.destination.region);
        }
        for (index, switch) in self.switches.iter_mut().enumerate() {
            let mut cases = ArenaVec::new_in(scratch);
            for &case in &self.cases[self.case_runs[index].clone()] {
                let mut case = case;
                case.dispatch = case.dispatch.map(|_| {
                    ancestry.transition(&self.regions, switch.dispatch.region, case.point.region)
                });
                cases.push(case);
            }
            switch.cases = tu.alloc_slice_copy(&cases);
            switch.default_dispatch = switch
                .default
                .and_then(|label| self.labels[label.0].point)
                .map(|point| {
                    ancestry.transition(&self.regions, switch.dispatch.region, point.region)
                });
            switch.no_match =
                ancestry.transition(&self.regions, switch.dispatch.region, switch.exit.region);
        }
        self.ancestry = Some(ancestry);
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(
        self,
        tu: &'tu Bump,
    ) -> ControlFacts<'tu> {
        let copy_map = |source: ArenaMap<'s, usize, StatementId>| {
            let mut result = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
            result.extend(source);
            result
        };
        let mut expressions = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
        expressions.extend(self.expression_points);
        let mut vlas = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
        vlas.extend(self.vla_regions);
        let mut address_taken = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
        address_taken.extend(
            self.address_taken
                .iter()
                .map(|(&function, labels)| (function, &*tu.alloc_slice_copy(labels))),
        );
        let mut label_addresses = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
        label_addresses.extend(self.label_addresses);
        let mut label_differences = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tu);
        label_differences.extend(self.label_differences);
        ControlFacts {
            regions: tu.alloc_slice_copy(&self.regions),
            statements: tu.alloc_slice_copy(&self.statements),
            labels: tu.alloc_slice_copy(&self.labels),
            jumps: tu.alloc_slice_copy(&self.jumps),
            loops: tu.alloc_slice_copy(&self.loops),
            switches: tu.alloc_slice_copy(&self.switches),
            statement_indices: copy_map(self.statement_indices),
            expression_points: expressions,
            vla_regions: vlas,
            address_taken,
            label_addresses,
            label_differences,
            #[cfg(test)]
            construction_work: self.ancestry.as_ref().map_or(0, |a| a.construction_work),
            #[cfg(test)]
            query_work: self.ancestry.as_ref().map_or(0, |a| a.query_work.get()),
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn new(scratch: &'s Bump) -> Self {
        let mut scope_regions = ArenaVec::new_in(scratch);
        scope_regions.push(None);
        Self {
            loop_bodies: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            regions: ArenaVec::new_in(scratch),
            current: None,
            scope_regions,
            statements: ArenaVec::new_in(scratch),
            labels: ArenaVec::new_in(scratch),
            jumps: ArenaVec::new_in(scratch),
            loops: ArenaVec::new_in(scratch),
            switches: ArenaVec::new_in(scratch),
            cases: ArenaVec::new_in(scratch),
            case_runs: ArenaVec::new_in(scratch),
            targets: ArenaVec::new_in(scratch),
            deferred_vm_labels: ArenaVec::new_in(scratch),
            normal_destinations: ArenaVec::new_in(scratch),
            ancestry: None,
            statement_indices: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            expression_points: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            vla_regions: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            address_taken: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            label_addresses: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            label_differences: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
        }
    }
}
