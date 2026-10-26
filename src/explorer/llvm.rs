//! LLVM IR for the site's prebuilt demonstrations, which the WebAssembly
//! explorer cannot produce: the text `--emit=llvm` prints, with `-O2` when
//! the options optimize.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "The explorer hands owned text and vectors across its interface, outside every arena."
)]

use std::fmt;

use super::{
    ExplorerDiagnostic,
    ExplorerOptions,
    compilation::{
        Failure,
        Stages,
        Views,
        compile_views,
    },
};
use crate::{
    backend::llvm,
    ir::Profile,
    optimizer::OptimizerOptions,
    pipeline::{
        InlineCopies,
        lower_translation_unit_with_locals,
        with_optional_retention,
    },
    translation_phases::{
        parsing::ParsedTranslationUnit,
        semantic_analysis::SemanticTranslationUnit,
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// The LLVM IR `bcc-rust --emit=llvm` prints for `source` with the same
/// target, language mode and optimization (`-O2` or none).
///
/// # Errors
///
/// Returns the compilation's diagnostics when it produced no LLVM IR.
pub fn llvm_ir(source: &str, options: &ExplorerOptions) -> Result<String, LlvmIrError> {
    let run = std::panic::AssertUnwindSafe(|| {
        crate::with_compilation_stack(crate::cli::COMPILATION_STACK_BYTES, || {
            compile_views(source, options, Views::Llvm)
        })
    });
    let (output, text) = match std::panic::catch_unwind(run) {
        | Ok(Ok(result)) => result,
        | Ok(Err(error)) => (
            super::compilation::failed(format_args!(
                "cannot reserve the compilation stack: {error}"
            )),
            None,
        ),
        | Err(_) => (
            super::compilation::failed(format_args!("internal compiler error")),
            None,
        ),
    };
    text.ok_or(LlvmIrError {
        diagnostics: output.diagnostics,
    })
}

/// A compilation that produced no LLVM IR, with its diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlvmIrError {
    /// Why, as [`ExplorerOutput::diagnostics`](super::ExplorerOutput) would
    /// say.
    pub diagnostics: Vec<ExplorerDiagnostic>,
}

impl<'tu> Stages<'_, '_, 'tu> {
    /// `--emit=llvm`, whose inline copies follow the LLVM back end's choice
    /// at `-O2` or `-O0`.
    pub(super) fn llvm_ir(
        &mut self,
        unit: &ParsedTranslationUnit<'tu>,
        sema: &SemanticTranslationUnit<'tu>,
        arena: &Bump,
        optimizer: Option<&OptimizerOptions<'_>>,
        optimize: bool,
    ) -> Option<String> {
        let context = &*self.context;
        let target = context.configuration.target();
        let code = context.configuration.code_options();
        let copies = if optimize {
            InlineCopies::Keep
        } else {
            InlineCopies::KeepAlwaysInline
        };
        let printed = with_optional_retention(true, |retain_locals| {
            let diagnosed = lower_translation_unit_with_locals(
                context,
                unit,
                sema,
                arena,
                optimizer,
                Profile::PostAbi,
                (copies, retain_locals),
            )
            .map_err(Failure::MiddleEnd)?;
            let mut text = ArenaString::new_in(arena);
            llvm::emit_module_with(&diagnosed.module, target, code, &mut text)
                .map_err(|error| Failure::Message(error.to_string()))?;
            Ok(text.to_string())
        });
        self.accept(printed)
    }
}

impl fmt::Display for LlvmIrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for diagnostic in &self.diagnostics {
            f.write_str(&diagnostic.rendered)?;
        }
        Ok(())
    }
}

impl std::error::Error for LlvmIrError {}
