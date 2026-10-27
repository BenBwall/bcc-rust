//! Jumps and cleanup scopes. A jump may leave a GNU `cleanup` variable's
//! region, which runs the cleanup, but may not enter it: that would bypass
//! the variable's initialization. A goto in the variable's own initializer
//! is already inside its scope, as Clang's `JumpScopeChecker` treats it, and
//! may reach a label after the declaration; the cleanup then runs when the
//! scope ends. An indirect goto may neither enter nor leave a cleanup
//! region: Clang rejects both, so no cleanup runs on an indirect jump.
//! These checks read the same retained control regions and transitions as
//! the variably modified checks; lowering calls the cleanups of the regions
//! a transition leaves.
//! GNU extension: GCC manual, "Common Variable Attributes", cleanup, and
//! "Labels as Values". Clang: `JumpScopeChecker`.
//! C99: §6.8.6.1p1, p. 137; PDF p. 149; §6.8.4.2p2, p. 134; PDF p. 146
//! (the corresponding constraints for variably modified identifiers).

use super::super::{
    Analyzer,
    SemanticErrorKind,
    facts::{
        ControlPoint,
        RegionId,
        RegionKind,
        ancestry::ancestor,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        parsing::syntax::Statement,
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// A goto into a cleanup variable's scope is an error at the goto,
    /// unless the goto is in that variable's initializer.
    pub(in crate::translation_phases::semantic_analysis) fn check_cleanup_entry(
        &mut self,
        statement: &'tu Statement<'tu>,
        control: Option<super::super::JumpId>,
        source: SourceVectors,
        points: Option<(ControlPoint, ControlPoint)>,
    ) {
        let key = std::ptr::from_ref(statement).addr();
        let entered = if let Some((from, to)) = points
            && self
                .attributes
                .initializing_jumps
                .iter()
                .any(|&(jump, _)| jump == key)
        {
            self.entered_cleanup_from_initializer(key, from.region, to.region)
        } else {
            let Some(jump) = control else {
                return;
            };
            let transition = self.control.jumps[jump.0].transition;
            self.control
                .ancestry
                .as_ref()
                .expect("control finalized")
                .first_entered_cleanup(&self.control.regions, transition)
        };
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

    /// The outermost cleanup region that the goto `jump`, written in the
    /// initializers of the variables it records, enters on its way from
    /// `from` to `to` other than those variables' own regions.
    fn entered_cleanup_from_initializer(
        &self,
        jump: usize,
        from: RegionId,
        to: RegionId,
    ) -> Option<RegionId> {
        let regions = &self.control.regions;
        let mut entered = None;
        let mut cleanup = regions[to.0].nearest_cleanup_declaration;
        while let Some(region) = cleanup.filter(|&region| !ancestor(regions, region, from)) {
            let own = matches!(
                regions[region.0].kind,
                RegionKind::CleanupDeclaration(variable)
                    if self.attributes.initializing_jumps.contains(&(jump, variable.0))
            );
            if !own {
                entered = Some(region);
            }
            cleanup = regions[region.0]
                .parent
                .and_then(|parent| regions[parent.0].nearest_cleanup_declaration);
        }
        entered
    }

    /// Records that the goto `statement` is in the initializers of the
    /// cleanup variables whose regions are still to open.
    pub(in crate::translation_phases::semantic_analysis) fn record_initializing_jump(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        let key = std::ptr::from_ref(statement).addr();
        let mut variables = ArenaVec::new_in(self.scratch);
        variables.extend(self.attributes.initializing_cleanups());
        for variable in variables {
            self.attributes.initializing_jumps.push((key, variable));
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

    /// The innermost cleanup scope that a jump from `from` to `to` enters,
    /// if any: the target's nearest cleanup declaration when it does not
    /// enclose `from`, as for variably modified scopes.
    pub(in crate::translation_phases::semantic_analysis) fn cleanup_entered(
        &self,
        from: RegionId,
        to: RegionId,
    ) -> Option<RegionId> {
        let regions = &self.control.regions;
        regions[to.0]
            .nearest_cleanup_declaration
            .filter(|&cleanup| !ancestor(regions, cleanup, from))
    }
}
