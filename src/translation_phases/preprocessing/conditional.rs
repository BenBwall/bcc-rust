//! Conditional inclusion and skipping of excluded groups.

use std::{
    fmt::Debug,
    ops::ControlFlow,
};

use super::{
    Preprocessor,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::translation_phases::{
    Context,
    TranslationPhase,
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
    },
};

/// How far [`Preprocessor::skip_over_dead_code`] skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkipMode {
    /// A group whose condition was false: stop at the matching `#elif` whose
    /// condition holds, at `#else`, or at `#endif`.
    FalseGroup,
    /// The groups after a translated one: skip through the matching `#endif`.
    ToEndif,
}

impl Preprocessor {
    /// Skips the lines of a conditional group that is not being translated
    /// (C99 §6.10.1p6). Only conditional directives are recognized inside
    /// skipped lines; every other line, including malformed directives, is
    /// ignored.
    ///
    /// `at_line_start` says whether the directive that started the skip has
    /// already consumed its terminating newline.
    fn skip_over_dead_code(
        &mut self,
        context: &mut Context,
        mut at_line_start: bool,
        mode: SkipMode,
    ) {
        let depth = self.open_conditionals.len();
        context.set_ignore_tokenizer_errors(true);
        'lines: while depth > 0 && self.open_conditionals.len() >= depth {
            if !at_line_start {
                loop {
                    match self.tokenizer.next_item(context) {
                        | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                        | Some(_) => {},
                        | None => break 'lines,
                    }
                }
            }
            at_line_start = false;
            let Some(first) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                break 'lines;
            };
            match first.kind {
                | PreprocessorTokenType::Newline => {
                    at_line_start = true;
                    continue 'lines;
                },
                | PreprocessorTokenType::Hash => {},
                | _ => continue 'lines,
            }
            let Some(name) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                break 'lines;
            };
            match name.kind {
                | PreprocessorTokenType::Newline => {
                    at_line_start = true;
                    continue 'lines;
                },
                | PreprocessorTokenType::Identifier => {},
                | _ => continue 'lines,
            }
            let innermost = self.open_conditionals.len() == depth;
            match context.string_cache.at(name.contents) {
                | "if" | "ifdef" | "ifndef" => self
                    .open_conditionals
                    .push(context.get_source_vectors(name.source_vectors).into()),
                | "endif" => {
                    drop(self.open_conditionals.pop());
                },
                | "elif" if innermost && mode == SkipMode::FalseGroup => {
                    context.set_ignore_tokenizer_errors(false);
                    let taken = self.eval_preprocessor_expression(
                        context,
                        PreprocessorErrorType::NoConditionInElifDirective,
                    );
                    if taken {
                        self.last_was_newline = true;
                        self.current_is_newline = true;
                        return;
                    }
                    context.set_ignore_tokenizer_errors(true);
                    at_line_start = true;
                },
                | "else" if innermost && mode == SkipMode::FalseGroup => break 'lines,
                | _ => {},
            }
        }
        context.set_ignore_tokenizer_errors(false);
        if !at_line_start {
            self.skip_until_newline(context);
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    pub(super) fn parse_if_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        self.open_conditionals
            .push(context.get_source_vectors(directive.source_vectors).into());
        if self
            .eval_preprocessor_expression(context, PreprocessorErrorType::NoConditionInIfDirective)
        {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context, true, SkipMode::FalseGroup);
        }
    }

    /// `#elif` and `#else` reached while translating a group end that group:
    /// the rest of the conditional is skipped through its `#endif`.
    pub(super) fn parse_elif_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        self.skip_remaining_groups(
            context,
            directive,
            PreprocessorErrorType::ElifDirectiveWithoutIfDirective,
        );
    }

    pub(super) fn parse_else_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        self.skip_remaining_groups(
            context,
            directive,
            PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
        );
    }

    fn skip_remaining_groups(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
        unmatched_error: PreprocessorErrorType,
    ) {
        if self.open_conditionals.is_empty() {
            context.preprocessor_error(PreprocessorError {
                error_type:     unmatched_error,
                source_vectors: directive.source_vectors,
            });
            self.skip_until_newline(context);
            return;
        }
        self.skip_over_dead_code(context, false, SkipMode::ToEndif);
    }

    pub(super) fn parse_endif_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        if self.open_conditionals.pop().is_none() {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                source_vectors: directive.source_vectors,
            });
        }
        self.skip_until_newline(context);
    }

    pub(super) fn parse_ifdef_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        self.parse_macro_test_directive(context, directive, true);
    }

    pub(super) fn parse_ifndef_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
    ) {
        self.parse_macro_test_directive(context, directive, false);
    }

    /// Handles `#ifdef` (`wants_defined`) and `#ifndef`. A missing macro name
    /// is diagnosed and the group is skipped, as GCC and Clang do.
    fn parse_macro_test_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
        wants_defined: bool,
    ) {
        self.open_conditionals
            .push(context.get_source_vectors(directive.source_vectors).into());
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     if wants_defined {
                        PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(token.kind)
                    } else {
                        PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(token.kind)
                    },
                    source_vectors: token.source_vectors,
                })
            },
            if wants_defined {
                "parsing ifdef directive"
            } else {
                "parsing ifndef directive"
            },
        ) else {
            self.skip_over_dead_code(context, false, SkipMode::FalseGroup);
            return;
        };
        if self
            .expect_token_from_previous_phase::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     if wants_defined {
                            PreprocessorErrorType::ExtraTokensAfterIfdefDirective
                        } else {
                            PreprocessorErrorType::ExtraTokensAfterIfndefDirective
                        },
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing conditional directive",
            )
            .is_none()
        {
            self.skip_until_newline(context);
        }
        if self.macro_definitions.contains_key(&name.contents) == wants_defined {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context, true, SkipMode::FalseGroup);
        }
    }
}
