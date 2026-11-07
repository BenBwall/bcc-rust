//! What GCC and Clang say of a function whose end its control can reach:
//! `-Wreturn-type`, and of one declared `noreturn`. C99 makes reaching the end
//! of a non-void function undefined only if the caller uses the value, so each
//! is a warning §5.1.1.3p1 permits. Both compilers ask their control-flow
//! graphs, which [`Analyzer::flow_statement`] follows. They differ in a
//! `switch` that covers its enumeration, which only Clang takes as complete,
//! and in what GCC's front end says first of a `static` function with no
//! `return`, which it may not lower at all.
//! GCC: `finish_function` in `c-decl.cc`, `pass_warn_function_return`.
//! Clang: `CheckFallThroughForBody` in `AnalysisBasedWarnings.cpp`.
//! C99: §6.9.1p12, p. 142; PDF p. 154; §5.1.2.2.3p1, p. 13; PDF p. 25.

use super::{
    super::{
        Linkage,
        SemanticErrorKind,
        types::{
            Scalar,
            TypeKind,
        },
    },
    Analyzer,
    Flow,
    S,
};
use crate::{
    configuration::{
        CStandard,
        CompilerIdentity,
        DiagnosticGroup as G,
    },
    translation_phases::{
        SourceVectors,
        context::Context,
        parsing::syntax::BlockItem,
    },
};

impl Analyzer<'_, '_, '_> {
    /// Checks the end of the current function, when its analysis is done.
    /// C99: §6.9.1p12, p. 142; PDF p. 154.
    pub(in crate::translation_phases::semantic_analysis) fn check_return_flow(&mut self) {
        let Some(function) = self.functions.current else {
            return;
        };
        if !self.unused.return_flow || self.tainted {
            return;
        }
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        // Clang's analysis stops at the unit's first error.
        if !gcc && self.context.error_count() != 0 {
            return;
        }
        let Some(binding) = function.binding else {
            return;
        };
        let noreturn = self.functions.emission.noreturn.contains_key(&binding);
        let result = self.types.non_atomic(function.result).unqualified();
        let void = matches!(
            self.types.nodes[result.index],
            TypeKind::Scalar(Scalar::Void)
        );
        if (void && !noreturn) || self.types.unanalyzed(result) || self.declared_naked(binding) {
            return;
        }
        // Nothing after a last `return` can run, which most functions end in.
        let body = function.syntax.body;
        if !noreturn
            && let S::Compound { items } = body.kind
            && matches!(
                items.iter().next_back(),
                Some(BlockItem::Statement(last)) if matches!(last.kind, S::Return(_))
            )
        {
            return;
        }
        let mut flow = Flow::new(self.scratch);
        let end_reached = loop {
            flow.changed = false;
            let live = self.flow_statement(&mut flow, body, true);
            if !flow.changed {
                break live;
            }
        };
        let end = Context::last_source_vector(body.source_vectors);
        if gcc {
            self.gcc_return_flow(binding, noreturn, end_reached, end, &flow);
        } else {
            self.clang_return_flow(binding, noreturn, end_reached, end, &flow);
        }
    }

    fn gcc_return_flow(
        &mut self,
        binding: usize,
        noreturn: bool,
        end_reached: bool,
        end: SourceVectors,
        flow: &Flow<'_>,
    ) {
        if noreturn {
            let site = if end_reached {
                Some(end)
            } else {
                flow.exits.live_return
            };
            if let Some(site) = site {
                self.return_flow_warning(G::GccNoreturn, site, "'noreturn' function does return");
            }
            return;
        }
        let data = self.bindings[binding];
        let main = self.context.string_cache.at(data.name.name) == "main";
        let internal = data.linkage == Linkage::Internal;
        let start = self
            .functions
            .current
            .map_or(end, |function| function.syntax.source_vectors);
        let implicit_int = self
            .functions
            .current
            .is_some_and(|function| function.syntax.declaration_specifiers.implicit_int);
        if internal && !main && !implicit_int && !flow.exits.any_return && !flow.exits.abnormal {
            self.return_flow_warning(
                G::ReturnType,
                start,
                "no return statement in function returning non-void",
            );
            return;
        }
        // GCC lowers neither of these unless something calls it.
        if (internal && self.inline_declared(binding)) || self.is_inline_definition(binding) {
            return;
        }
        let exempt = main && self.context.configuration.standard() as u8 >= CStandard::C99 as u8;
        if end_reached && !exempt {
            self.return_flow_warning(
                G::ReturnType,
                end,
                "control reaches end of non-void function",
            );
        }
    }

    fn clang_return_flow(
        &mut self,
        binding: usize,
        noreturn: bool,
        end_reached: bool,
        end: SourceVectors,
        flow: &Flow<'_>,
    ) {
        if !end_reached {
            return;
        }
        if noreturn {
            self.return_flow_warning(
                G::InvalidNoreturn,
                end,
                "function declared 'noreturn' should not return",
            );
            return;
        }
        let main = self
            .context
            .string_cache
            .at(self.bindings[binding].name.name)
            == "main";
        if main && self.context.configuration.hosted() {
            return;
        }
        // A path that leaves by a call counts as one that does not fall
        // through.
        let text = if flow.exits.live_return.is_some() || flow.exits.abnormal {
            "non-void function does not return a value in all control paths"
        } else {
            "non-void function does not return a value"
        };
        self.return_flow_warning(G::ReturnType, end, text);
    }

    /// A `return` in a function declared `noreturn`, which C11 §6.7.4p8 makes
    /// undefined if it runs.
    /// C11: §6.7.4p8, p. 126; PDF p. 144.
    pub(in crate::translation_phases::semantic_analysis) fn check_noreturn_return(
        &mut self,
        site: SourceVectors,
    ) {
        let Some(function) = self.functions.current else {
            return;
        };
        let Some(binding) = function.binding else {
            return;
        };
        if !self.unused.return_flow || !self.functions.emission.noreturn.contains_key(&binding) {
            return;
        }
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            self.return_flow_warning(
                G::GccNoreturn,
                site,
                "function declared 'noreturn' has a 'return' statement",
            );
        } else {
            let name = self
                .context
                .string_cache
                .at(self.bindings[binding].name.name);
            let text = crate::diagnostics::format_in!(
                self.scratch,
                "function '{name}' declared 'noreturn' should not return"
            );
            self.return_flow_warning(G::InvalidNoreturn, site, text);
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn return_flow_warning(
        &mut self,
        group: G,
        site: SourceVectors,
        text: &str,
    ) {
        let name = self.context.string_cache.intern(text);
        self.spelled_group_diagnostic(
            group,
            SemanticErrorKind::ReturnFlow(group),
            site,
            Some(name),
            site,
        );
    }
}
