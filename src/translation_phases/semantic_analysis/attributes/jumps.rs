//! Jumps and cleanup scopes. A jump may leave a GNU `cleanup` variable's
//! region, which runs the cleanup, but may not enter it: that would bypass
//! the variable's initialization. An indirect goto may neither enter nor
//! leave one, because bcc, like Clang, does not run cleanups on indirect
//! jumps. These checks read the same retained control regions and
//! transitions as the variably modified checks and lowering.
//! GNU extension: GCC manual, "Common Variable Attributes", cleanup, and
//! "Labels as Values". Clang: `JumpScopeChecker`.
//! C99: §6.8.6.1p1, p. 137; PDF p. 149; §6.8.4.2p2, p. 134; PDF p. 146
//! (the corresponding constraints for variably modified identifiers).

use super::super::{
    Analyzer,
    SemanticErrorKind,
    facts::{
        ControlPoint,
        LabelId,
        RegionKind,
        ancestry::ancestor,
    },
};
use crate::translation_phases::{
    SourceVectors,
    parsing::syntax::Statement,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// A goto into a cleanup variable's scope is an error at the goto.
    pub(in crate::translation_phases::semantic_analysis) fn check_cleanup_entry(
        &mut self,
        statement: &'tu Statement<'tu>,
        source: SourceVectors,
    ) {
        let Some(jump) = self
            .control
            .statement_id(statement)
            .and_then(|id| self.control.statements[id.0].jump)
        else {
            return;
        };
        let transition = self.control.jumps[jump.0].transition;
        let entered = self
            .control
            .ancestry
            .as_ref()
            .expect("control finalized")
            .first_entered_cleanup(&self.control.regions, transition);
        if let Some(region) = entered {
            let declaration = self.control.regions[region.0].source;
            self.error(
                SemanticErrorKind::JumpIntoCleanupScope,
                source,
                None,
                Some(declaration),
            );
        }
    }

    /// Switch dispatch from `from` to a case label at `to` may not enter a
    /// cleanup variable's scope.
    pub(in crate::translation_phases::semantic_analysis) fn check_cleanup_dispatch(
        &mut self,
        from: ControlPoint,
        to: ControlPoint,
        source: SourceVectors,
    ) {
        let regions = &self.control.regions;
        if let Some(region) = regions[to.region.0]
            .nearest_cleanup_declaration
            .filter(|&cleanup| !ancestor(regions, cleanup, from.region))
        {
            let declaration = regions[region.0].source;
            self.error(
                SemanticErrorKind::SwitchIntoCleanupScope,
                source,
                None,
                Some(declaration),
            );
        }
    }

    /// Records an indirect goto for the check at translation-unit end.
    pub(in crate::translation_phases::semantic_analysis) fn record_indirect_jump(
        &mut self,
        source: SourceVectors,
    ) {
        if let Some(point) = self.control.point() {
            self.attributes.indirect_jumps.push((point, source));
        }
    }

    /// Records a label whose address is taken, a possible indirect target.
    pub(in crate::translation_phases::semantic_analysis) fn record_address_label(
        &mut self,
        label: LabelId,
    ) {
        self.attributes.address_labels.push(label);
    }

    /// Every indirect goto of a function may reach every label whose
    /// address the function takes; none of those edges may cross a cleanup
    /// scope. Functions without cleanup variables cost nothing here.
    pub(in crate::translation_phases::semantic_analysis) fn check_indirect_cleanup_jumps(
        &mut self,
    ) {
        if self.attributes.indirect_jumps.is_empty()
            || !self
                .control
                .regions
                .iter()
                .any(|region| matches!(region.kind, RegionKind::CleanupDeclaration(_)))
        {
            return;
        }
        for jump in 0..self.attributes.indirect_jumps.len() {
            let (from, source) = self.attributes.indirect_jumps[jump];
            for target in 0..self.attributes.address_labels.len() {
                let label = self.control.labels[self.attributes.address_labels[target].0];
                let Some(to) = label.point.filter(|to| to.function == from.function) else {
                    continue;
                };
                let regions = &self.control.regions;
                let ancestry = self.control.ancestry.as_ref().expect("control finalized");
                let transition = ancestry.transition(regions, from.region, to.region);
                let common = regions[transition.left.stop_before.0].preorder;
                let leaves = regions[from.region.0]
                    .nearest_cleanup_declaration
                    .is_some_and(|cleanup| regions[cleanup.0].preorder > common);
                let enters = ancestry
                    .first_entered_cleanup(regions, transition)
                    .is_some();
                if leaves || enters {
                    let target = label.definition.map_or(source, |s| s.source_vectors);
                    self.error(
                        SemanticErrorKind::IndirectJumpCleanupScope,
                        source,
                        None,
                        Some(target),
                    );
                    break;
                }
            }
        }
    }
}
