//! Conditional inclusion and skipping of excluded groups.

use std::{
    fmt::Debug,
    ops::ControlFlow,
};

use super::{
    Preprocessor,
    driver::TokenizerFrameType,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVector,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::bump::Bump,
};

/// One source-file-local conditional, including whether its final arm began.
#[derive(Debug)]
pub(super) struct ConditionalGroup<'pp> {
    pub(super) source: &'pp [SourceVector],
    saw_else:          bool,
}

impl<'pp> ConditionalGroup<'pp> {
    fn new(pp: &'pp Bump, context: &Context<'_>, directive: PreprocessorToken) -> Self {
        Self {
            source:   pp.alloc_slice_fill_iter(
                context
                    .get_source_vectors(directive.source_vectors)
                    .iter()
                    .cloned(),
            ),
            saw_else: false,
        }
    }
}

/// How far [`Preprocessor::skip_over_dead_code`] skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkipMode {
    /// A group whose condition was false: stop at the matching `#elif` whose
    /// condition holds, at `#else`, or at `#endif`.
    FalseGroup,
    /// The groups after a translated one: skip through the matching `#endif`.
    ToEndif,
}

impl Preprocessor<'_, '_> {
    /// Physical source frames own conditional groups. Presumed filenames
    /// changed by #line do not change the frame's boundary.
    fn current_file_conditional_base(&self) -> usize {
        self.state
            .tokenizer_stack
            .iter()
            .rev()
            .find_map(|frame| match &frame.frame_type {
                | TokenizerFrameType::SourceFile {
                    conditional_base, ..
                } => Some(*conditional_base),
                | _ => None,
            })
            .unwrap_or(0)
    }

    /// Skips the lines of a conditional group that is not being translated
    /// (C99 §6.10.1p6). Only conditional directives are recognized inside
    /// skipped lines; every other line, including malformed directives, is
    /// ignored.
    ///
    /// `at_line_start` says whether the directive that started the skip has
    /// already consumed its terminating newline.
    fn skip_over_dead_code(
        &mut self,
        context: &mut Context<'_>,
        mut at_line_start: bool,
        mode: SkipMode,
    ) {
        let depth = self.state.open_conditionals.len();
        context.set_ignore_tokenizer_errors(true);
        'lines: while depth > 0 && self.state.open_conditionals.len() >= depth {
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
                | PreprocessorTokenType::Identifier
                | PreprocessorTokenType::UniversalIdentifier => {},
                | _ => continue 'lines,
            }
            let innermost = self.state.open_conditionals.len() == depth;
            match context.string_cache.at(name.contents) {
                | "if" | "ifdef" | "ifndef" => self
                    .state
                    .open_conditionals
                    .push(ConditionalGroup::new(self.state.arena, context, name)),
                | "endif" => {
                    let _ = self.state.open_conditionals.pop();
                    if innermost {
                        context.set_ignore_tokenizer_errors(false);
                        self.finish_conditional_directive(context, "endif");
                        at_line_start = true;
                    }
                },
                | "elif" if innermost => {
                    if !self.check_conditional_arm(context, name, false)
                        || mode == SkipMode::ToEndif
                    {
                        continue 'lines;
                    }
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
                | "else" if innermost => {
                    let valid = self.check_conditional_arm(context, name, true);
                    context.set_ignore_tokenizer_errors(false);
                    self.finish_conditional_directive(context, "else");
                    at_line_start = true;
                    if valid && mode == SkipMode::FalseGroup {
                        break 'lines;
                    }
                    context.set_ignore_tokenizer_errors(true);
                },
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
        context: &mut Context<'_>,
        directive: PreprocessorToken,
    ) {
        self.state.open_conditionals.push(ConditionalGroup::new(
            self.state.arena,
            context,
            directive,
        ));
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
        context: &mut Context<'_>,
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
        context: &mut Context<'_>,
        directive: PreprocessorToken,
    ) {
        self.skip_remaining_groups(
            context,
            directive,
            PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
        );
    }

    fn skip_remaining_groups<'tu>(
        &mut self,
        context: &mut Context<'tu>,
        directive: PreprocessorToken,
        unmatched_error: PreprocessorErrorType<'tu>,
    ) {
        if self.state.open_conditionals.len() <= self.current_file_conditional_base() {
            context.preprocessor_error(PreprocessorError {
                error_type:     unmatched_error,
                source_vectors: directive.source_vectors,
            });
            self.skip_until_newline(context);
            return;
        }
        let is_else = context.string_cache.at(directive.contents) == "else";
        _ = self.check_conditional_arm(context, directive, is_else);
        if is_else {
            self.finish_conditional_directive(context, "else");
        }
        self.skip_over_dead_code(context, is_else, SkipMode::ToEndif);
    }

    fn check_conditional_arm(
        &mut self,
        context: &mut Context<'_>,
        directive: PreprocessorToken,
        is_else: bool,
    ) -> bool {
        let Some(group) = self.state.open_conditionals.last_mut() else {
            return false;
        };
        if group.saw_else {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ConditionalArmAfterElse(if is_else {
                    "else"
                } else {
                    "elif"
                }),
                source_vectors: directive.source_vectors,
            });
            return false;
        }
        group.saw_else = is_else;
        true
    }

    fn finish_conditional_directive(&mut self, context: &mut Context<'_>, name: &'static str) {
        if let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, context)
            && token.kind != PreprocessorTokenType::Newline
        {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ExtraTokensAfterConditionalDirective(name),
                source_vectors: token.source_vectors,
            });
            self.skip_until_newline(context);
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    pub(super) fn parse_endif_directive(
        &mut self,
        context: &mut Context<'_>,
        directive: PreprocessorToken,
    ) {
        if self.state.open_conditionals.len() <= self.current_file_conditional_base() {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                source_vectors: directive.source_vectors,
            });
        } else {
            let _ = self.state.open_conditionals.pop();
        }
        self.finish_conditional_directive(context, "endif");
    }

    pub(super) fn parse_ifdef_directive(
        &mut self,
        context: &mut Context<'_>,
        directive: PreprocessorToken,
    ) {
        self.parse_macro_test_directive(context, directive, true);
    }

    pub(super) fn parse_ifndef_directive(
        &mut self,
        context: &mut Context<'_>,
        directive: PreprocessorToken,
    ) {
        self.parse_macro_test_directive(context, directive, false);
    }

    /// Handles `#ifdef` (`wants_defined`) and `#ifndef`. A missing macro name
    /// is diagnosed and the group is skipped, as GCC and Clang do.
    fn parse_macro_test_directive(
        &mut self,
        context: &mut Context<'_>,
        directive: PreprocessorToken,
        wants_defined: bool,
    ) {
        self.state.open_conditionals.push(ConditionalGroup::new(
            self.state.arena,
            context,
            directive,
        ));
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind.is_identifier(),
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
        if self
            .state
            .macro_definitions
            .contains_key(&name.identifier_id(context))
            == wants_defined
        {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context, true, SkipMode::FalseGroup);
        }
    }
}
