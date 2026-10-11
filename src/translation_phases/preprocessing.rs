//! Preprocessing reads a stack of source files and macro replacements. It
//! executes directives, substitutes macro arguments, and rescans replacements.
//! Surviving tokens have their escapes decoded, adjacent strings joined, and
//! their kinds converted for the parser. Temporary expansion memory is reused
//! when reading reaches a point with no active macro expansion. The parser
//! consumes the finished token array; syntax and semantic analysis belong to
//! later modules.
//!
//! For `#define TWICE(x) x + x` followed by `TWICE(2) "a" "b"`, the directive
//! stores a definition. The reader collects `2`, substitutes it twice, and
//! rescans `2 + 2`. Conversion turns `2 + 2` into three parser tokens and
//! joins the two following string literals into a fourth, one string
//! containing `ab`.
//!
//! Read [`Preprocessor::preprocess_into_arena`], [`Preprocessor::run`], and
//! [`Expander::next_iterator_item`] first. Then follow
//! [`Expander::next_preprocessor_token`] for the macro-replacing reader.
//! [`Preprocessor`] and [`Expander`] hold the state those loops run on.
//!
//! Files by role:
//!
//! - Input and working state: `command_line.rs`, `runtime.rs`, and `runtime/`.
//! - Directive handling: `directives.rs` and `directives/`; conditional
//!   inclusion is in `conditional.rs`, with its integer evaluator in
//!   `expression.rs`.
//! - Macro replacement: `macro_expansion.rs` and `macro_expansion/`.
//! - Dialect queries and embedding: `language_features.rs`.
//! - Parser tokens and literal conversion: `token.rs`, `token_conversion.rs`.
//! - Diagnostics and regression coverage: `errors.rs`, `tests.rs`, and
//!   `tests/`.
//!
//! C99: translation phases 4-7, §5.1.1.2 paragraph 1 items 4-7, p. 10; PDF p.
//! 22.
//!
//! C99: preprocessing directives, §6.10, pp. 145-162; PDF pp. 157-174;
//! grammar summary §A.3, pp. 416-418; PDF pp. 428-430.
//!
//! C99: adjacent string literals, §6.4.5 paragraph 4, p. 62; PDF p. 74.
//!
//! C99: preprocessing-token conversion, §6.4 paragraphs 2-3, p. 49; PDF p. 61.
//! Phases 1-3 belong to `initial_processing` and `preprocessor_tokenizer`.
//! The syntax and semantics of phase 7 belong to the parser and semantic
//! analysis.

// Input and working state.
mod command_line;
mod runtime;

// Directives and conditional inclusion.
mod conditional;
mod directives;
mod expression;

// Macro replacement and dialect features.
mod language_features;
mod macro_expansion;

// Parser tokens and literal conversion.
mod token;
mod token_conversion;

// Diagnostics.
mod errors;

use std::{
    cell::OnceCell,
    ops::ControlFlow,
};

pub(crate) use errors::{
    PreprocessorError,
    PreprocessorErrorType,
};
use expression::PreprocessorExpressionParser;
pub(crate) use macro_expansion::HashHash;
use macro_expansion::{
    FunctionLikeMacroArgument,
    MacroArguments,
    MacroDefinition,
};
use runtime::{
    OutputPurpose,
    PreprocessorState,
    QueryExpansion,
    RESET_EXPANSIONS_AFTER_FRAMES,
    Resting,
    TokenizerFrame,
    TokenizerFrameType,
};
pub(crate) use token::{
    CharacterTokenType,
    FloatTokenType,
    IntegerTokenType,
    KeywordTokenType,
    LiteralId,
    LiteralUnit,
    OperatorTokenType,
    StringTokenType,
    Token,
    TokenType,
};

use crate::{
    translation_phases::{
        Context,
        GetSourceFileIndex,
        SetPosition,
        SourcePosition,
        SourceVector,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        region_vec::RegionVec,
    },
};

impl<'tu, 'pp> Preprocessor<'tu, 'pp> {
    /// Appends phase-6 output to the caller's token buffer before parsing
    /// starts. Diagnostics stay pending in `context`; retained provenance
    /// survives compaction of preprocessor working storage between tokens.
    pub(crate) fn preprocess_into_arena(
        &mut self,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
        tokens: &mut RegionVec<Token>,
    ) -> Option<Token> {
        self.collect_with_limit(context, source_segment_limit, |token| tokens.push(token))
    }

    /// Stops after the first token that exceeds the configured provenance
    /// budget, letting the parser report the existing resource diagnostic.
    fn collect_with_limit(
        &mut self,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
        mut push: impl FnMut(Token),
    ) -> Option<Token> {
        self.resting_mut().source_segment_limit = source_segment_limit;
        self.run(context, |preprocessor| {
            let Some(mut token) = preprocessor.next_iterator_item() else {
                return ControlFlow::Break(None);
            };
            let context = &mut *preprocessor.context;
            token.source_vectors = context.retain_token_source(token.source_vectors);
            push(token);
            if context.source_segment_count() > source_segment_limit {
                let mut limit_token = preprocessor.pending_parser_token.unwrap_or(token);
                limit_token.source_vectors =
                    context.retain_token_source(limit_token.source_vectors);
                context.append_pending_errors(preprocessor.pending_parser_errors.drain(..));
                return ControlFlow::Break(Some(limit_token));
            }
            ControlFlow::Continue(())
        })
    }

    /// Calls `visit` with each phase-6 token. Preprocessor provenance is
    /// compacted before each token is produced, so `visit` must retain the
    /// provenance it keeps.
    pub(crate) fn for_each_iterator_item(
        &mut self,
        context: &mut Context<'tu>,
        mut visit: impl FnMut(&mut Context<'tu>, Token),
    ) {
        self.run(context, |preprocessor| {
            match preprocessor.next_iterator_item() {
                | Some(token) => {
                    visit(preprocessor.context, token);
                    ControlFlow::Continue(())
                },
                | None => ControlFlow::Break(()),
            }
        });
    }

    /// Calls `step` until it breaks. Between calls, once enough frames were
    /// pushed and no expansion is active, the expansion arena is reset.
    ///
    /// When `step` breaks inside an expansion, that expansion's state is
    /// released with its arena and preprocessing cannot resume.
    fn run<B>(
        &mut self,
        context: &mut Context<'tu>,
        mut step: impl for<'c, 'x> FnMut(&mut Expander<'c, 'tu, 'pp, 'x>) -> ControlFlow<B>,
    ) -> B {
        loop {
            // The previous expander was suspended or dropped; resting state
            // retains no expansion allocations or raw pointers, and output
            // and diagnostics have been copied into longer-lived storage.
            self.expansion.reset();
            let resting = self
                .resting
                .take()
                .expect("preprocessing does not resume after stopping inside an expansion");
            let mut expander = Expander::resume(resting, context, &self.expansion);
            let stopped = loop {
                if let ControlFlow::Break(value) = step(&mut expander) {
                    break Some(value);
                }
                if expander.pushed_frames >= RESET_EXPANSIONS_AFTER_FRAMES
                    && expander.is_between_expansions()
                {
                    break None;
                }
            };
            self.end = (expander.position(), expander.source_file_index());
            if expander.is_between_expansions() {
                self.resting = Some(expander.suspend());
            }
            if let Some(value) = stopped {
                return value;
            }
        }
    }

    /// Runs translation phases 4 through 6 without the parser's resource
    /// budget, for direct preprocessing tests.
    #[cfg(test)]
    pub(crate) fn preprocess_all(&mut self, context: &mut Context<'tu>) -> RegionVec<Token> {
        let mut tokens = RegionVec::new();
        let _ = self.preprocess_into_arena(context, usize::MAX, &mut tokens);
        tokens
    }

    /// Calls `visit` with each phase-6 token as [`Expander::next_item`]
    /// produces it, without compacting provenance.
    #[cfg(test)]
    pub(crate) fn for_each_item(
        &mut self,
        context: &mut Context<'tu>,
        mut visit: impl FnMut(&mut Context<'tu>, Token),
    ) {
        self.run(context, |preprocessor| match preprocessor.next_item() {
            | Some(token) => {
                visit(preprocessor.context, token);
                ControlFlow::Continue(())
            },
            | None => ControlFlow::Break(()),
        });
    }
}

impl Expander<'_, '_, '_, '_> {
    /// Produces the next iterator item while keeping buffered provenance alive.
    ///
    /// Adjacent-string concatenation may already have mapped a later token or
    /// EOF diagnostic. Source-vector compaction therefore belongs to the
    /// producer that owns that buffered work, not to each iterator consumer.
    fn next_iterator_item(&mut self) -> Option<Token> {
        if self.next_iterator_item_compacts() {
            self.context.compact_preprocessor_vectors();
        }
        self.next_item()
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl<'x> Expander<'_, '_, '_, 'x> {
    /// Returns the next completely macro-replaced preprocessing token.
    ///
    /// Placemarkers left by `##` are dropped here (C99: §6.10.3.4 paragraph
    /// 1, p. 155; PDF p. 167), and a name met while its macro is being
    /// replaced is marked unavailable for good (§6.10.3.4 paragraph 2).
    fn next_preprocessor_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
    ) -> Option<PreprocessorToken> {
        self.last_was_newline = self.current_is_newline;
        let ret = 'base: loop {
            self.generate_placeholders = true;
            let Some((mut token, mut is_pasted)) =
                self.handle_hash_hash_operator::<SHOULD_IGNORE_WHITESPACE>()
            else {
                break 'base None;
            };
            self.generate_placeholders = false;
            if !self.hash_hash_stack.is_empty() {
                'merge: loop {
                    let next_is_end = self.macro_argument_is_at_end();
                    let argument_continues = matches!(
                        self.tokenizer_stack.last(),
                        Some(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroArgument { .. },
                            ..
                        })
                    ) && !next_is_end;
                    if matches!(
                        token.kind,
                        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                    ) {
                        // Whitespace is never a `##` operand, only the
                        // tokens around it.
                        if next_is_end
                            || matches!(self.hash_hash_stack.last(), Some(HashHash::Lhs(_)))
                        {
                            continue 'base;
                        }
                        break 'merge;
                    }
                    let new = match self.hash_hash_stack.last() {
                        | None | Some(HashHash::Empty) => None,
                        | Some(HashHash::Lhs(lhs)) => {
                            let lhs = *lhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(lhs, token)
                        },
                        | Some(HashHash::Rhs(rhs)) => {
                            // Paste the last token of the left argument.
                            if argument_continues {
                                break 'merge;
                            }
                            let rhs = *rhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(token, rhs)
                        },
                    };
                    if (next_is_end || token.kind == PreprocessorTokenType::Placeholder)
                        && let Some(x @ HashHash::Empty) = self.hash_hash_stack.last_mut()
                    {
                        *x = HashHash::Lhs(if let Some(new) = new { new } else { token });
                        continue 'base;
                    }
                    if let Some(new) = new {
                        token = new;
                        is_pasted = true;
                    } else {
                        break 'merge;
                    }
                }
            }
            if token.kind == PreprocessorTokenType::Placeholder && !self.state.retain_placeholders {
                continue 'base;
            }
            if !token.kind.is_identifier()
                || matches!(
                    token.kind,
                    PreprocessorTokenType::UnavailableIdentifier
                        | PreprocessorTokenType::UnavailableUniversalIdentifier
                )
            {
                break 'base Some(token);
            }
            // C23: §6.10.5p5, p. 178; PDF p. 191. Valid optional
            // replacements have already been consumed by prepare_variadic_body,
            // and a replacement list was checked where it was defined.
            if token.identifier_id(self.context) == self.state.va_opt_name
                && !matches!(
                    self.tokenizer_stack.last().map(|frame| &frame.frame_type),
                    Some(
                        TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                            | TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                    )
                )
            {
                self.check_va_args_use(token);
            }
            // C99 §6.10.3.1: parameters are replaced before the replacement
            // list is rescanned, so a parameter hides a macro of its name.
            // `##` runs after replacement, so its result names no parameter.
            if !is_pasted && let Some(frame) = self.handle_macro_argument(token) {
                self.push_tokenizer_frame(frame);
                continue;
            }
            if self.is_reading_operand() {
                // A `#` or `##` operand is not macro-replaced (C99
                // §6.10.3.1p1).
                break 'base Some(token);
            }

            // C99 §6.10.3.4p2: a name met while its macro is being replaced
            // stays unreplaced, even when it is rescanned later.
            if self.macro_is_disabled(token.identifier_id(self.context)) {
                token.kind = if token.kind == PreprocessorTokenType::UniversalIdentifier {
                    PreprocessorTokenType::UnavailableUniversalIdentifier
                } else {
                    PreprocessorTokenType::UnavailableIdentifier
                };
                break 'base Some(token);
            }
            if let Some(md) = self
                .state
                .macro_definitions
                .get(&token.identifier_id(self.context))
                .cloned()
            {
                match md {
                    | MacroDefinition::ObjectLike { tokenizer } => {
                        self.warn_deprecated_macro(token);
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation {
                                invocation_end: self.expansion_end().unwrap_or_else(|| {
                                    self.context
                                        .get_source_vectors(token.source_vectors)
                                        .last()
                                        .cloned()
                                        .unwrap_or_default()
                                }),
                                invocation:     self.invocation_location(token),
                                spelling:       self.spelling_location(token),
                                name:           token.identifier_id(self.context),
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(frame);
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        tokenizer,
                        is_variadic,
                        variadic_alias,
                    } => {
                        let variadic_name = is_variadic.then(|| {
                            variadic_alias
                                .unwrap_or_else(|| self.context.string_cache.intern("__VA_ARGS__"))
                        });
                        if !matches!(
                            self.tokenizer_stack.last().map(|f| &f.frame_type),
                            Some(TokenizerFrameType::SourceFile { .. })
                        ) {
                            let Some((arguments, invocation_end)) =
                                self.capture_cross_frame_call(token, argument_names, variadic_name)
                            else {
                                break 'base Some(token);
                            };
                            self.warn_deprecated_macro(token);
                            let (tokenizer, arguments) = if is_variadic {
                                self.prepare_variadic_body(token, tokenizer, arguments)
                            } else {
                                (tokenizer, arguments)
                            };
                            self.push_tokenizer_frame(TokenizerFrame {
                                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                    invocation_end,
                                    invocation: self.invocation_location(token),
                                    spelling: self.spelling_location(token),
                                    name: token.identifier_id(self.context),
                                    arguments,
                                    is_variadic,
                                },
                                tokenizer,
                            });
                            continue;
                        }
                        let position = self.position();
                        // The name was read from a source file, where a
                        // newline is whitespace between a function macro's
                        // name and `(` (C99 §6.10.3p10). Calls read from
                        // other frames were captured above.
                        loop {
                            match self.tokenizer.next_item(self.context) {
                                | Some(brace)
                                    if matches!(
                                        brace.kind,
                                        PreprocessorTokenType::Whitespace
                                            | PreprocessorTokenType::Newline
                                    ) =>
                                    continue,
                                | Some(brace)
                                    if brace.kind == PreprocessorTokenType::OpeningParenthesis =>
                                    break,
                                // C99 §6.10.3p10: without a following `(` the
                                // name is not an invocation and stays as is.
                                | Some(_) | None => {
                                    self.set_position(position);
                                    break 'base Some(token);
                                },
                            }
                        }
                        let mut i = 0;
                        self.warn_deprecated_macro(token);
                        let enclosing_arguments = self.get_arguments();
                        let disabled_macros = self.disabled_macros();
                        let mut arguments = ArenaVec::with_capacity_in(
                            argument_names.len() + usize::from(is_variadic),
                            self.scratch,
                        );
                        // Excess arguments share a recovery map key, so map
                        // length cannot give the invocation's argument count.
                        let mut argument_count = 0;
                        let mut paren_depth = 1isize;
                        // Where the named arguments of a variadic macro met
                        // the closing parenthesis, leaving `...` without one.
                        let mut closed_at = None;
                        macro_rules! at {
                            () => {
                                argument_names.get(i).copied().unwrap_or_else(|| {
                                    self.context.string_cache.intern("<undefined>")
                                })
                            };
                        }
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let tokenizer = self.tokenizer.clone();
                            let mut has_argument_token = false;
                            loop {
                                let before = is_variadic.then(|| self.position());
                                match self.tokenizer.next_item(self.context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            closed_at = before;
                                            // F() supplies no arguments when
                                            // F has no parameters, but one
                                            // empty argument when it has one
                                            // (C99 §6.10.3p4).
                                            argument_count = if i == 0
                                                && argument_names.is_empty()
                                                && !has_argument_token
                                            {
                                                0
                                            } else {
                                                i + 1
                                            };
                                            if argument_count != 0 {
                                                if !has_argument_token {
                                                    self.context.report_extension(crate::configuration::Feature::EmptyMacroArguments, "empty macro argument", token.source_vectors);
                                                }
                                                arguments.push(FunctionLikeMacroArgument {
                                                    variadic: false,
                                                    substituted: false,
                                                    expanded: self.scratch.alloc(OnceCell::new()),
                                                    omitted: false,
                                                    name: at!(),
                                                    tokenizer,
                                                    enclosing_arguments,
                                                    disabled_macros,
                                                });
                                            }
                                            break 'outer;
                                        }
                                        paren_depth -= 1;
                                    },
                                    // C99 §6.10.3p11: commas inside inner
                                    // parentheses do not separate arguments.
                                    | Some(token)
                                        if token.kind == PreprocessorTokenType::Comma
                                            && paren_depth == 1 =>
                                    {
                                        if !has_argument_token {
                                            self.report_empty_macro_argument(token.source_vectors);
                                        }
                                        arguments.push(FunctionLikeMacroArgument {
                                            variadic: false,
                                            substituted: false,
                                            expanded: self.scratch.alloc(OnceCell::new()),
                                            omitted: false,
                                            name: at!(),
                                            tokenizer,
                                            enclosing_arguments,
                                            disabled_macros,
                                        });
                                        i += 1;
                                        continue 'outer;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        has_argument_token = true;
                                        continue;
                                    },
                                    | Some(token) => {
                                        has_argument_token |= !matches!(
                                            token.kind,
                                            PreprocessorTokenType::Whitespace
                                                | PreprocessorTokenType::Newline
                                        );
                                        continue;
                                    },
                                    | None => {
                                        self.context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.set_position(position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }

                        // C99 §6.10.3p4: one argument per parameter, and more
                        // than the named parameters of a macro with `...`.
                        let missing_named_arguments = if is_variadic {
                            closed_at.is_some() && argument_count < argument_names.len()
                        } else {
                            argument_count != argument_names.len()
                        };
                        if missing_named_arguments {
                            self.context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      argument_count,
                                    },
                                    source_vectors: token.source_vectors,
                                },
                            );
                        } else if closed_at.is_some() {
                            // C99 §6.10.3p4 requires an argument for `...`;
                            // omitting it is a common extension (§4p6), which
                            // the extension policy governs.
                            if !self
                                .context
                                .configuration
                                .accepts(crate::configuration::Feature::MsVaArgs)
                            {
                                self.context.preprocessor_extension(
                                    crate::configuration::Feature::OmittedVariadicArguments,
                                    crate::translation_phases::DiagnosticPolicy::Extension,
                                    token.source_vectors,
                                    PreprocessorErrorType::MissingVariadicArgument,
                                );
                            }
                        }
                        if is_variadic {
                            // The trailing arguments, commas included, form
                            // the one argument `__VA_ARGS__` stands for (C99
                            // §6.10.3p12, §6.10.3.1p2). Without an argument,
                            // `__VA_ARGS__` is empty: it reads only the
                            // closing parenthesis.
                            let va_args_tokenizer = match closed_at {
                                | Some(position) => {
                                    let mut closing = self.tokenizer.clone();
                                    closing.set_position(position);
                                    closing
                                },
                                | None => self.tokenizer.clone(),
                            };
                            arguments.push(FunctionLikeMacroArgument {
                                variadic: true,
                                substituted: false,
                                expanded: self.scratch.alloc(OnceCell::new()),
                                name: variadic_name.expect("variadic parameter name"),
                                omitted: closed_at.is_some(),
                                tokenizer: va_args_tokenizer,
                                enclosing_arguments,
                                disabled_macros,
                            });
                            let mut paren_depth = 1isize;
                            let mut has_va_argument = false;

                            if closed_at.is_none() {
                                loop {
                                    match self.tokenizer.next_item(self.context) {
                                        | Some(token)
                                            if token.kind
                                                == PreprocessorTokenType::ClosingParenthesis =>
                                        {
                                            if paren_depth == 1 {
                                                if !has_va_argument
                                                    && argument_names.is_empty()
                                                    && self.context.configuration.gnu_extensions()
                                                {
                                                    arguments
                                                        .last_mut()
                                                        .expect("variadic argument")
                                                        .omitted = true;
                                                } else if !has_va_argument {
                                                    self.report_empty_macro_argument(
                                                        token.source_vectors,
                                                    );
                                                }
                                                break;
                                            }
                                            paren_depth -= 1;
                                        },
                                        | Some(token)
                                            if token.kind
                                                == PreprocessorTokenType::OpeningParenthesis =>
                                        {
                                            paren_depth += 1;
                                            has_va_argument = true;
                                            continue;
                                        },
                                        | Some(token) => {
                                            has_va_argument |= !matches!(
                                                token.kind,
                                                PreprocessorTokenType::Whitespace
                                                    | PreprocessorTokenType::Newline
                                            );
                                            continue;
                                        },
                                        | None => {
                                            self.context.preprocessor_error(PreprocessorError {
                                                error_type:
                                                    PreprocessorErrorType::UnexpectedEndOfInput(
                                                        "parsing function-like macro invocation",
                                                    ),
                                                source_vectors: token.source_vectors,
                                            });
                                            self.set_position(position);
                                            break 'base Some(token);
                                        },
                                    }
                                }
                            }
                        }
                        let arguments: MacroArguments<'x> = arguments.leak();
                        let (tokenizer, arguments) = if is_variadic {
                            self.prepare_variadic_body(token, tokenizer, arguments)
                        } else {
                            (tokenizer, arguments)
                        };
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                invocation_end: SourceVector::new(
                                    self.position(),
                                    self.source_file_index(),
                                    0,
                                ),
                                invocation: self.invocation_location(token),
                                spelling: self.spelling_location(token),
                                name: token.identifier_id(self.context),
                                arguments,
                                is_variadic,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(frame);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => {
                        if let Some(result) = self.expand_builtin(token) {
                            break 'base Some(result);
                        }
                        continue 'base;
                    },
                }
            }
            break 'base Some(token);
        };
        self.generate_placeholders = false;
        self.current_is_newline = ret.is_none_or(|t| t.kind == PreprocessorTokenType::Newline);
        ret
    }
}

/// Translation phases 4 through 6 over one translation unit.
///
/// C99: §5.1.1.2 paragraph 1 items 4-6, p. 10; PDF p. 22. Each preprocessing
/// token that survives phase 4 is converted to a token here as well, ahead of
/// the phase-7 analysis the parser performs.
///
/// Macro expansion working memory comes from an expansion arena. It is reset
/// at points where no expansion is active, so per-invocation data does not
/// accumulate in the preprocessing arena for the whole phase. This is the
/// chunking seam's expansion arena, at the granularity of top-level
/// expansions.
pub(crate) struct Preprocessor<'tu, 'pp> {
    /// `None` only after a caller stopped preprocessing inside an expansion.
    resting:   Option<Resting<'tu, 'pp>>,
    expansion: Bump,
    /// Where reading stopped, and the presumed source file there.
    end:       (SourcePosition, u32),
}

/// The preprocessor while it reads input: its long-lived state plus the
/// expansion state, whose memory comes from the expansion arena `'x`.
struct Expander<'c, 'tu, 'pp: 'x, 'x> {
    /// The translation context, borrowed while this expander runs.
    context:               &'c mut Context<'tu>,
    state:                 PreprocessorState<'pp>,
    /// Source and include frames remain between expansions. Macro frames
    /// share this stack until the current expansion finishes.
    tokenizer_stack:       ArenaVec<'x, TokenizerFrame<'x>>,
    tokenizer:             TokenSource<'x>,
    /// Expansion working memory, reset between top-level expansions.
    scratch:               &'x Bump,
    hash_hash_stack:       ArenaVec<'x, HashHash>,
    current_is_newline:    bool,
    /// Collect use-site hint metadata only when a language parser will consume
    /// the output.
    output_purpose:        OutputPurpose,
    last_was_newline:      bool,
    generate_placeholders: bool,
    /// C23 §6.10.4.2p3: queries in embed operands wait for limit evaluation.
    query_expansion:       QueryExpansion,
    /// The tokenizer-stack depth of the `#` or `##` operand being replaced,
    /// at which reading stops when that operand ends, or 0 outside such
    /// replacement.
    operand_fence:         usize,
    /// Argument prescan stops here without suppressing expansion within it.
    expansion_fence:       usize,
    /// The tokenizer-stack depth from which no name is macro-replaced, or 0.
    /// A `__VA_OPT__` result is substituted but not rescanned.
    verbatim_fence:        usize,
    expression_parser:     PreprocessorExpressionParser<'pp>,
    pending_parser_token:  Option<Token>,
    pending_parser_errors: ArenaVec<'pp, TranslationError<'tu>>,
    /// Parser-supplied provenance budget, checked within string concatenation
    /// as well as between completed output tokens.
    source_segment_limit:  usize,
    /// Frames pushed since the expansion arena was last reset.
    pushed_frames:         usize,
}

// Tests.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
