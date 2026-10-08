//! Conditional inclusion and skipping of excluded groups.
//!
//! C99: the `if-section` grammar of §6.10 paragraph 1, p. 145; PDF p. 157
//! (also §A.3, pp. 416-417; PDF pp. 428-429), and conditional inclusion,
//! §6.10.1 paragraphs 3-6, pp. 148-149; PDF pp. 160-161. The controlling
//! expressions of `#if` and `#elif` are evaluated in `expression`.
//!
//! Nesting depth is not limited; §5.2.4.1 paragraph 1, p. 20; PDF p. 32
//! requires at least 63 levels.

use std::{
    fmt::Debug,
    ops::ControlFlow,
};

use super::{
    Expander,
    driver::TokenizerFrameType,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    configuration::Feature,
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
///
/// C99: `if-section`, §6.10 paragraph 1, p. 145; PDF p. 157: an `if-group`,
/// any `elif-groups`, at most one `else-group`, and an `endif-line`.
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

/// The spelling of a conditional directive's name, for its diagnostics.
fn conditional_directive_name(context: &Context<'_>, directive: PreprocessorToken) -> &'static str {
    match context.string_cache.at(directive.contents) {
        | "else" => "else",
        | "elifdef" => "elifdef",
        | "elifndef" => "elifndef",
        | _ => "elif",
    }
}

/// How far [`Expander::skip_over_dead_code`] skips.
///
/// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161: only the first group whose
/// condition is true is processed, else the `#else` group if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkipMode {
    /// A group whose condition was false: stop at the matching `#elif` whose
    /// condition holds, at `#else`, or at `#endif`.
    FalseGroup,
    /// The groups after a translated one: skip through the matching `#endif`.
    ToEndif,
}

impl<'tu> Expander<'_, 'tu, '_, '_> {
    /// Physical source frames own conditional groups. Presumed filenames
    /// changed by #line do not change the frame's boundary.
    fn current_file_conditional_base(&self) -> usize {
        self.tokenizer_stack
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
    ///
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161, and the relaxed syntax
    /// of skipped groups, §6.10 paragraph 4, p. 147; PDF p. 159.
    fn skip_over_dead_code(&mut self, mut at_line_start: bool, mode: SkipMode) {
        let depth = self.state.open_conditionals.len();
        self.context.set_ignore_tokenizer_errors(true);
        'lines: while depth > 0 && self.state.open_conditionals.len() >= depth {
            if !at_line_start {
                loop {
                    match self.tokenizer.next_item(self.context) {
                        | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                        | Some(_) => {},
                        | None => break 'lines,
                    }
                }
            }
            at_line_start = false;
            let Some(first) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            else {
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
            let Some(name) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context) else {
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
            match self.context.string_cache.at(name.contents) {
                | "if" | "ifdef" | "ifndef" => self
                    .state
                    .open_conditionals
                    .push(ConditionalGroup::new(self.state.arena, self.context, name)),
                | "endif" => {
                    let _ = self.state.open_conditionals.pop();
                    if innermost {
                        self.context.set_ignore_tokenizer_errors(false);
                        self.finish_conditional_directive("endif");
                        at_line_start = true;
                    }
                },
                | "elif" if innermost => {
                    if !self.check_conditional_arm(name, false) || mode == SkipMode::ToEndif {
                        continue 'lines;
                    }
                    self.context.set_ignore_tokenizer_errors(false);
                    let taken = self.eval_preprocessor_expression(
                        PreprocessorErrorType::NoConditionInElifDirective,
                    );
                    if taken {
                        self.last_was_newline = true;
                        self.current_is_newline = true;
                        return;
                    }
                    self.context.set_ignore_tokenizer_errors(true);
                    at_line_start = true;
                },
                | "elifdef" | "elifndef"
                    if innermost && self.context.configuration.accepts(Feature::Elifdef) =>
                {
                    if !self.check_conditional_arm(name, false) || mode == SkipMode::ToEndif {
                        continue 'lines;
                    }
                    self.context.set_ignore_tokenizer_errors(false);
                    let directive = conditional_directive_name(self.context, name);
                    self.context.report_extension(
                        Feature::Elifdef,
                        if directive == "elifdef" {
                            "#elifdef"
                        } else {
                            "#elifndef"
                        },
                        name.source_vectors,
                    );
                    let taken = self.eval_macro_test(directive);
                    at_line_start = true;
                    if taken {
                        self.last_was_newline = true;
                        self.current_is_newline = true;
                        return;
                    }
                    self.context.set_ignore_tokenizer_errors(true);
                },
                | "else" if innermost => {
                    let valid = self.check_conditional_arm(name, true);
                    self.context.set_ignore_tokenizer_errors(false);
                    self.finish_conditional_directive("else");
                    at_line_start = true;
                    if valid && mode == SkipMode::FalseGroup {
                        break 'lines;
                    }
                    self.context.set_ignore_tokenizer_errors(true);
                },
                | _ => {},
            }
        }
        self.context.set_ignore_tokenizer_errors(false);
        if !at_line_start {
            self.skip_until_newline();
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    /// Opens a conditional and processes its group when the controlling
    /// expression is nonzero.
    ///
    /// C99: §6.10.1 paragraph 3, p. 148; PDF p. 160.
    pub(super) fn parse_if_directive(&mut self, directive: PreprocessorToken) {
        self.state.open_conditionals.push(ConditionalGroup::new(
            self.state.arena,
            self.context,
            directive,
        ));
        if self.eval_preprocessor_expression(PreprocessorErrorType::NoConditionInIfDirective) {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(true, SkipMode::FalseGroup);
        }
    }

    /// `#elif` and `#else` reached while translating a group end that group:
    /// the rest of the conditional is skipped through its `#endif`.
    ///
    /// C99: §6.10.1 paragraph 6, p. 149; PDF p. 161. The skipped `#elif`'s
    /// expression is not evaluated.
    pub(super) fn parse_elif_directive(&mut self, directive: PreprocessorToken) {
        let name = conditional_directive_name(self.context, directive);
        self.skip_remaining_groups(
            directive,
            PreprocessorErrorType::ElifDirectiveWithoutIfDirective(name),
        );
    }

    pub(super) fn parse_else_directive(&mut self, directive: PreprocessorToken) {
        self.skip_remaining_groups(
            directive,
            PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
        );
    }

    fn skip_remaining_groups(
        &mut self,
        directive: PreprocessorToken,
        unmatched_error: PreprocessorErrorType<'tu>,
    ) {
        if self.state.open_conditionals.len() <= self.current_file_conditional_base() {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     unmatched_error,
                source_vectors: directive.source_vectors,
            });
            self.skip_until_newline();
            return;
        }
        let is_else = self.context.string_cache.at(directive.contents) == "else";
        _ = self.check_conditional_arm(directive, is_else);
        if is_else {
            self.finish_conditional_directive("else");
        }
        self.skip_over_dead_code(is_else, SkipMode::ToEndif);
    }

    /// Whether an `#elif` or `#else` may follow the arms already seen: none
    /// may follow the `else-group`.
    ///
    /// C99: `if-section`, §6.10 paragraph 1, p. 145; PDF p. 157.
    fn check_conditional_arm(&mut self, directive: PreprocessorToken, is_else: bool) -> bool {
        let Some(group) = self.state.open_conditionals.last_mut() else {
            return false;
        };
        if group.saw_else {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ConditionalArmAfterElse(
                    conditional_directive_name(self.context, directive),
                ),
                source_vectors: directive.source_vectors,
            });
            return false;
        }
        group.saw_else = is_else;
        true
    }

    /// Ends an `#else` or `#endif` line, which takes no tokens.
    ///
    /// C99: §6.10 paragraph 1, p. 145; PDF p. 157, and footnote 147, p. 149;
    /// PDF p. 161.
    fn finish_conditional_directive(&mut self, name: &'static str) {
        if let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            && token.kind != PreprocessorTokenType::Newline
        {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::ExtraTokensAfterConditionalDirective(name),
                source_vectors: token.source_vectors,
            });
            self.skip_until_newline();
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    pub(super) fn parse_endif_directive(&mut self, directive: PreprocessorToken) {
        if self.state.open_conditionals.len() <= self.current_file_conditional_base() {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                source_vectors: directive.source_vectors,
            });
        } else {
            let _ = self.state.open_conditionals.pop();
        }
        self.finish_conditional_directive("endif");
    }

    pub(super) fn parse_ifdef_directive(&mut self, directive: PreprocessorToken) {
        self.parse_macro_test_directive(directive, "ifdef");
    }

    pub(super) fn parse_ifndef_directive(&mut self, directive: PreprocessorToken) {
        self.parse_macro_test_directive(directive, "ifndef");
    }

    /// Handles `#ifdef` (`wants_defined`) and `#ifndef`. A missing macro name
    /// is diagnosed and the group is skipped, as GCC and Clang do.
    ///
    /// C99: §6.10.1 paragraph 5, pp. 148-149; PDF pp. 160-161: the same
    /// tests as `#if defined identifier` and `#if !defined identifier`.
    fn parse_macro_test_directive(&mut self, directive: PreprocessorToken, name: &'static str) {
        self.state.open_conditionals.push(ConditionalGroup::new(
            self.state.arena,
            self.context,
            directive,
        ));
        if self.eval_macro_test(name) {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(true, SkipMode::FalseGroup);
        }
    }

    /// Reads the macro name that `#ifdef`, `#ifndef`, `#elifdef`, or
    /// `#elifndef` (named by `directive`) tests, and returns whether the
    /// test holds.
    ///
    /// C23: §6.10.2p16, p. 168; PDF p. 181: elifdef tests a macro name
    /// directly.
    fn eval_macro_test(&mut self, directive: &'static str) -> bool {
        let wants_defined = matches!(directive, "ifdef" | "elifdef");
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     if wants_defined {
                        PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(
                            directive, token.kind,
                        )
                    } else {
                        PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(
                            directive, token.kind,
                        )
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
            if !self.current_is_newline {
                self.skip_until_newline();
            }
            return false;
        };
        if self
            .expect_token_from_previous_phase::<true>(
                |_, t| t.kind == PreprocessorTokenType::Newline,
                |_, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     if wants_defined {
                            PreprocessorErrorType::ExtraTokensAfterIfdefDirective(directive)
                        } else {
                            PreprocessorErrorType::ExtraTokensAfterIfndefDirective(directive)
                        },
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing conditional directive",
            )
            .is_none()
        {
            self.skip_until_newline();
        }
        self.state
            .macro_definitions
            .contains_key(&name.identifier_id(self.context))
            == wants_defined
    }
}
