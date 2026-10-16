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
        RegionId,
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
