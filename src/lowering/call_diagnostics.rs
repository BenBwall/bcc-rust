//! GNU function-call diagnostics follow opaque IDs attached to IR calls.
//! After optimization, only calls in reachable function bodies retain their
//! messages; inlining copies IDs, and removing a call removes its diagnostic.
//! C99: §5.1.1.3p1, p. 11; PDF p. 23, extended by GNU function attributes.

use crate::{
    ir::{
        Entity,
        GlobalInit,
        InstData,
        Linkage,
        Module,
        RelocationTarget,
        Symbol,
    },
    translation_phases::{
        Context,
        SourceVectors,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Selects messages belonging to surviving calls. The IR keeps only numeric
/// IDs; source locations and diagnostic strings remain in the frontend.
/// GNU extension: GCC Common Function Attributes, error and warning.
pub(crate) fn retained_call_diagnostics<'ir>(
    module: &Module<'_>,
    diagnostics: &[CallDiagnostic],
    arena: &'ir Bump,
    scratch: &Bump,
) -> &'ir [CallDiagnostic] {
    if diagnostics.is_empty() {
        return &[];
    }
    let mut seen = ArenaVec::new_in(scratch);
    seen.resize(module.functions().count(), false);
    let mut keep = ArenaVec::new_in(scratch);
    keep.resize(diagnostics.len(), 0_usize);
    let mut pending = ArenaVec::new_in(scratch);
    for (func, function) in module.functions() {
        if function.linkage != Linkage::Internal || function.attributes.used {
            pending.push(func);
        }
    }
    pending.extend(module.init_functions().iter().map(|init| init.func));
    for (_, global) in module.globals() {
        if let Some(GlobalInit::Bytes { relocations, .. }) = global.init {
            pending.extend(relocations.iter().filter_map(|r| match r.target {
                | RelocationTarget::Symbol(Symbol::Function(func))
                | RelocationTarget::Label { func, .. }
                | RelocationTarget::LabelDifference { func, .. } => Some(func),
                | _ => None,
            }));
        }
    }
    let mut seen_blocks = ArenaVec::new_in(scratch);
    let mut pending_blocks = ArenaVec::new_in(scratch);
    while let Some(func) = pending.pop() {
        let func = module.function_definition(func);
        if seen[func.index()] {
            continue;
        }
        seen[func.index()] = true;
        let Some(body) = &module.function(func).body else {
            continue;
        };
        seen_blocks.clear();
        seen_blocks.resize(body.block_count(), false);
        pending_blocks.clear();
        pending_blocks.extend(body.entry_block());
        while let Some(block) = pending_blocks.pop() {
            if seen_blocks[block.index()] {
                continue;
            }
            seen_blocks[block.index()] = true;
            for &inst in body.block_insts(block) {
                if let Some(id) = body.call_diagnostic(inst) {
                    keep[id as usize] += 1;
                }
                if let InstData::Call { func, .. } | InstData::FuncAddr { func } = *body.inst(inst)
                {
                    pending.push(func);
                }
            }
            if let Some(terminator) = body.terminator(block) {
                body.visit_destinations(terminator, |dest| {
                    pending_blocks.push(body.block_call(dest).0);
                });
            }
        }
    }
    let mut retained = ArenaVec::new_in(arena);
    for (&diagnostic, count) in diagnostics.iter().zip(keep) {
        retained.extend(std::iter::repeat_n(diagnostic, count));
    }
    retained.leak()
}

/// One GNU error/warning diagnostic attached to an emitted direct call.
/// GNU extension: GCC Common Function Attributes, error and warning.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallDiagnostic {
    pub(crate) message:     crate::util::string_cache::StringCacheId,
    pub(crate) declaration: SourceVectors,
    pub(crate) source:      SourceVectors,
    pub(crate) warning:     bool,
}

impl CallDiagnostic {
    pub(crate) fn report(self, context: &mut Context<'_>) {
        let Some(severity) = self.severity(context) else {
            return;
        };
        context.append_pending_errors([crate::translation_phases::TranslationError::Semantic(
            crate::translation_phases::semantic_analysis::SemanticError {
                kind: if self.warning {
                    crate::translation_phases::semantic_analysis::SemanticErrorKind::CallAttributeWarning
                } else {
                    crate::translation_phases::semantic_analysis::SemanticErrorKind::CallAttribute
                },
                extension_severity: Some(severity),
                source_vectors:     self.source,
                name:               Some(self.message),
                previous:           Some(self.declaration),
            },
        )]);
    }

    /// The output preparation boundary must honor promotion and suppression
    /// before an artifact is committed, without reporting a failed attempt.
    pub(crate) fn is_error(self, context: &Context<'_>) -> bool {
        self.severity(context) == Some(crate::translation_phases::ErrorSeverity::Error)
    }

    fn severity(self, context: &Context<'_>) -> Option<crate::translation_phases::ErrorSeverity> {
        let severity = if self.warning {
            context.group_severity(
                crate::configuration::DiagnosticGroup::AttributeWarning,
                self.source,
            )?
        } else {
            crate::translation_phases::ErrorSeverity::Error
        };
        (!context.withholds(severity, false, self.source)).then_some(severity)
    }
}
