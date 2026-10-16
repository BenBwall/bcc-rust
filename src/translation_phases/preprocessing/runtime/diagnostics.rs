//! Macro deprecation warnings and their expansion notes.
//!
//! C99: translation phase 4; implementation-defined pragma behavior,
//! §6.10.6 paragraph 1, p. 159; PDF p. 171; replacement rescanning,
//! §6.10.3.4 paragraph 1, p. 155; PDF p. 167.
//! Deprecation is a Clang extension and does not alter macro replacement.

use std::fmt::Debug;

use super::{
    super::{
        Expander,
        PreprocessorError,
        PreprocessorErrorType,
        errors::{
            DeprecatedMacroDiagnostic,
            MacroExpansionNote,
        },
    },
    TokenizerFrameType,
};
use crate::{
    translation_phases::{
        SourceVector,
        preprocessor_tokenizer::PreprocessorToken,
    },
    util::bump::ArenaVec,
};

impl Expander<'_, '_, '_, '_> {
    /// Clang's implementation-defined deprecation is reported at the outer
    /// replacement invocation, or the argument being prescanned. Suppression
    /// therefore follows the use rather than a system-header spelling.
    /// C99: §6.10.6p1, p. 159; PDF p. 171; rescanning §6.10.3.4p1,
    /// p. 155; PDF p. 167.
    pub(in crate::translation_phases::preprocessing) fn warn_deprecated_macro(
        &mut self,
        token: PreprocessorToken,
    ) {
        let Some(deprecation) = self
            .state
            .deprecated_macros
            .get(&token.identifier_id(self.context))
            .cloned()
        else {
            return;
        };
        self.report_deprecated_macro(token, deprecation);
    }

    /// Every expansion looks the macro up; only a deprecated one reaches the
    /// diagnostic, which stays out of line.
    #[cold]
    #[inline(never)]
    fn report_deprecated_macro(
        &mut self,
        token: PreprocessorToken,
        deprecation: MacroDeprecation<'_>,
    ) {
        let mut spelling = self.spelling_location(token);
        let mut invocation = spelling.clone();
        let mut expansions = ArenaVec::new_in(self.scratch);
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation {
                    name,
                    invocation: location,
                    spelling: parent_spelling,
                    ..
                }
                | TokenizerFrameType::FunctionLikeMacroInvocation {
                    name,
                    invocation: location,
                    spelling: parent_spelling,
                    ..
                } => {
                    // __VA_OPT__ uses an internal replacement frame, not an
                    // additional source macro expansion (C23 §6.10.5.1p4).
                    if *name == self.state.va_opt_name {
                        continue;
                    }
                    expansions.push(MacroExpansionNote {
                        name:     self
                            .context
                            .diagnostic_text(self.context.string_cache.at(*name)),
                        location: spelling,
                    });
                    spelling = parent_spelling.clone();
                    invocation = location.clone();
                },
                | TokenizerFrameType::SourceFile { .. }
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::Rescan { argument: true } => break,
                | TokenizerFrameType::Rescan { argument: false }
                | TokenizerFrameType::DeferredQuery => {},
            }
        }
        let source_vectors = self.context.push_source_vectors(&[invocation]);
        self.context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::DeprecatedMacro(
                self.context.tu_arena().alloc(DeprecatedMacroDiagnostic {
                    name:       self
                        .context
                        .diagnostic_text(self.context.string_cache.at(token.contents)),
                    message:    deprecation
                        .message
                        .map(|text| self.context.diagnostic_text(text)),
                    marked_at:  deprecation.location,
                    expansions: self
                        .context
                        .tu_arena()
                        .alloc_slice_fill_iter(expansions.iter().rev().cloned()),
                }),
            ),
            source_vectors,
        });
    }
}

/// Clang's macro deprecation message and the end of its pragma payload.
/// Own the location across preprocessor provenance compaction.
/// C99: implementation-defined pragma behavior, §6.10.6p1, p. 159; PDF p. 171.
#[derive(Debug, Clone)]
pub(in crate::translation_phases::preprocessing) struct MacroDeprecation<'pp> {
    pub(in crate::translation_phases::preprocessing) message:  Option<&'pp str>,
    pub(in crate::translation_phases::preprocessing) location: SourceVector,
}
