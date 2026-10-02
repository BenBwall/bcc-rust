//! Tokenizer frame stack and the main preprocessing-token loop.

use std::{
    fmt::Debug,
    mem::take,
    ops::ControlFlow,
    path::PathBuf,
    rc::Rc,
};

use chrono::Local;

use super::{
    Preprocessor,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    macro_expansion::{
        FunctionLikeMacroArgument,
        HashHash,
        MacroDefinition,
    },
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            PreprocessorTokenizer,
        },
    },
    util::{
        HashMap,
        string_cache::StringCacheId,
    },
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType {
    SourceFile,
    ObjectLikeMacroInvocation {
        name: StringCacheId,
    },
    FunctionLikeMacroInvocation {
        name:        StringCacheId,
        arguments:   Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        is_variadic: bool,
    },
    FunctionLikeMacroArgument {
        argument:            Box<FunctionLikeMacroArgument>,
        paren_depth:         usize,
        has_generated_token: bool,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame {
    pub(super) frame_type: TokenizerFrameType,
    pub(super) tokenizer:  PreprocessorTokenizer,
}

/// The date and time of translation, spelled as C99 §6.10.8p1 requires.
#[derive(Debug)]
pub(super) struct TranslationTimestamp {
    date: String,
    time: String,
}

impl TranslationTimestamp {
    /// Honors `SOURCE_DATE_EPOCH` (reproducible-builds.org, also used by GCC
    /// and Clang) so builds can pin the expansion; otherwise uses local time.
    fn now() -> Self {
        let pinned = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|seconds| seconds.trim().parse::<i64>().ok())
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| time.naive_utc());
        let time = pinned.unwrap_or_else(|| Local::now().naive_local());
        Self {
            date: time.format("%b %e %Y").to_string(),
            time: time.format("%H:%M:%S").to_string(),
        }
    }
}

/// Spells `value` as a narrow C string literal whose evaluated contents are
/// exactly `value`.
fn string_literal_spelling(value: &str) -> String {
    let mut spelling = String::with_capacity(value.len() + 2);
    spelling.push('"');
    for c in value.chars() {
        match c {
            | '\\' | '"' => {
                spelling.push('\\');
                spelling.push(c);
            },
            | '\n' => spelling.push_str("\\n"),
            | _ => spelling.push(c),
        }
    }
    spelling.push('"');
    spelling
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl Preprocessor {
    pub(super) fn skip_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.tokenizer.next_item(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    pub(super) fn skip_and_expand_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.next_preprocessor_token::<true>(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    pub(super) fn push_tokenizer_frame(&mut self, _context: &mut Context, frame: TokenizerFrame) {
        self.tokenizer_stack.last_mut().unwrap().tokenizer = take(&mut self.tokenizer);
        self.tokenizer = frame.tokenizer.clone();
        self.tokenizer_stack.push(frame);
    }

    pub(super) fn pop_tokenizer_frame(&mut self, _context: &mut Context) {
        let f = self.tokenizer_stack.pop();
        drop(f);
        if let Some(last) = self.tokenizer_stack.last() {
            self.tokenizer = last.tokenizer.clone();
        }
    }

    pub(super) fn expect_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_token_with_rewind::<SHOULD_IGNORE_WHITESPACE>(
            context,
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            true,
        )
    }

    pub(super) fn expect_token_without_rewind<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_token_with_rewind::<SHOULD_IGNORE_WHITESPACE>(
            context,
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            false,
        )
    }

    fn expect_token_with_rewind<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
        rewind_on_error: bool,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.position(context);
            match self.next_preprocessor_token::<SHOULD_IGNORE_WHITESPACE>(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            if rewind_on_error {
                                self.set_position(context, start);
                            }
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    if rewind_on_error {
                        self.set_position(context, start);
                    }
                    let source_vectors =
                        context.create_source_vectors(start, self.source_file_index(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    pub(super) fn expect_token_from_previous_phase<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.position(context);
            match self.tokenizer.next_item(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            self.set_position(context, start);
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    self.set_position(context, start);
                    let source_vectors =
                        context.create_source_vectors(start, self.source_file_index(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    pub(super) fn next_preprocessor_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        self.last_was_newline = self.current_is_newline;
        let ret = 'base: loop {
            self.generate_placeholders = true;
            let Some(mut token) =
                self.handle_hash_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)
            else {
                break 'base None;
            };
            self.generate_placeholders = false;
            if !self.hash_hash_stack.is_empty() {
                'merge: loop {
                    let next_is_end = self.macro_argument_is_at_end(context);
                    let argument_continues = matches!(
                        self.tokenizer_stack.last(),
                        Some(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroArgument { .. },
                            ..
                        })
                    ) && !next_is_end;
                    let new = match self.hash_hash_stack.last() {
                        | None | Some(HashHash::Empty) => None,
                        | Some(HashHash::Lhs(lhs)) => {
                            let lhs = *lhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, lhs, token)
                        },
                        | Some(HashHash::Rhs(rhs)) => {
                            // Paste the last token of the left argument.
                            if argument_continues {
                                break 'merge;
                            }
                            let rhs = *rhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, token, rhs)
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
                    } else {
                        break 'merge;
                    }
                }
            }
            if token.kind == PreprocessorTokenType::Placeholder {
                continue 'base;
            }
            if token.kind != PreprocessorTokenType::Identifier {
                break 'base Some(token);
            }

            if self.macro_is_disabled(token.contents) {
                break 'base Some(token);
            }
            if let Some(md) = self.macro_definitions.get(&token.contents).cloned() {
                match md {
                    | MacroDefinition::ObjectLike { tokenizer } => {
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation {
                                name: token.contents,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        tokenizer,
                        is_variadic,
                    } => {
                        let position = self.position(context);
                        loop {
                            match self.tokenizer.next_item(context) {
                                | Some(brace) if brace.kind == PreprocessorTokenType::Whitespace =>
                                    continue,
                                | Some(brace)
                                    if brace.kind == PreprocessorTokenType::OpeningParenthesis =>
                                    break,
                                // C99 §6.10.3p10: without a following `(` the
                                // name is not an invocation and stays as is.
                                | Some(_) | None => {
                                    self.set_position(context, position);
                                    break 'base Some(token);
                                },
                            }
                        }
                        let mut i = 0;
                        let enclosing_arguments = self.get_arguments(context);
                        let disabled_macros = self.disabled_macros();
                        let mut arguments = HashMap::default();
                        let mut paren_depth = 1isize;
                        macro_rules! at {
                            () => {
                                argument_names
                                    .get(i)
                                    .copied()
                                    .unwrap_or(context.string_cache.intern("<undefined>"))
                            };
                        }
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let tokenizer = self.tokenizer.clone();
                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            drop(arguments.insert(
                                                at!(),
                                                FunctionLikeMacroArgument {
                                                    name: at!(),
                                                    tokenizer,
                                                    enclosing_arguments:
                                                        enclosing_arguments.clone(),
                                                    disabled_macros: disabled_macros.clone(),
                                                },
                                            ));
                                            break 'outer;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token) if token.kind == PreprocessorTokenType::Comma => {
                                        drop(arguments.insert(
                                            at!(),
                                            FunctionLikeMacroArgument {
                                                name: at!(),
                                                tokenizer,
                                                enclosing_arguments: enclosing_arguments.clone(),
                                                disabled_macros: disabled_macros.clone(),
                                            },
                                        ));
                                        i += 1;
                                        continue 'outer;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.set_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }

                        if arguments.len() != argument_names.len() && !is_variadic {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      arguments.len(),
                                    },
                                    source_vectors: token.source_vectors,
                                },
                            );
                        }
                        if is_variadic {
                            drop(arguments.insert(
                                context.string_cache.intern("__VA_ARGS__"),
                                FunctionLikeMacroArgument {
                                    name:                context.string_cache.intern("__VA_ARGS__"),
                                    tokenizer:           self.tokenizer.clone(),
                                    enclosing_arguments: enclosing_arguments.clone(),
                                    disabled_macros:     disabled_macros.clone(),
                                },
                            ));
                            let mut paren_depth = 1isize;

                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            break;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.set_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                name: token.contents,
                                arguments: Rc::new(arguments),
                                is_variadic,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match context.string_cache.at(token.contents) {
                        // C99 §6.10.8p1: each built-in expands to an ordinary
                        // token spelled as C source, located at the invocation.
                        | "__FILE__" => {
                            let spelling = string_literal_spelling(
                                &context.source_files[self.source_file_index()].to_string_lossy(),
                            );
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | "__LINE__" => {
                            // Number spellings carry the trailing NUL that
                            // numeric conversion expects.
                            let spelling = format!("{}\0", self.line(context));
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | name @ ("__DATE__" | "__TIME__") => {
                            let is_date = name == "__DATE__";
                            let timestamp = self
                                .translation_timestamp
                                .get_or_insert_with(TranslationTimestamp::now);
                            let spelling = string_literal_spelling(if is_date {
                                &timestamp.date
                            } else {
                                &timestamp.time
                            });
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | "_Pragma" => {
                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::OpeningParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            let Some(string_token) = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::String,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingStringLiteralInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            ) else {
                                continue 'base;
                            };

                            let input =
                                self.prepare_pragma_operator_string(context, string_token.contents);

                            let tokenizer = take(&mut self.tokenizer);
                            // Each operator gets its own identity: diagnostics
                            // rendered later must quote this payload, not the
                            // most recent one.
                            let pragma_string = context.add_synthetic_source_file(
                                PathBuf::from("<pragma string>").into_boxed_path(),
                                input.clone(),
                            );
                            self.tokenizer = PreprocessorTokenizer::new(pragma_string, input);
                            self.parse_pragma_directive(context, string_token);
                            if self.tokenizer.next_item(context).is_some() {
                                let source_vectors = context.create_source_vectors(
                                    self.position(context),
                                    self.source_file_index(),
                                    0,
                                );
                                context.preprocessor_error(PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::ExtraTokensAfterPragmaOperator,
                                    source_vectors,
                                });
                            }
                            self.tokenizer = tokenizer;

                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            continue 'base;
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            }
            if let Some(frame) = self.handle_macro_argument(context, token) {
                self.push_tokenizer_frame(context, frame);
                continue;
            }
            break 'base Some(token);
        };
        self.generate_placeholders = false;
        self.current_is_newline = ret.is_none_or(|t| t.kind == PreprocessorTokenType::Newline);
        ret
    }

    pub(super) fn next_ignore_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(t) if t.kind == PreprocessorTokenType::Whitespace => continue,
                | Some(t) => return Some(t),
                | None => return None,
            }
        }
    }

    pub(super) fn next_treat_newlines_as_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
        last_was_whitespace: &mut bool,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(mut t) => {
                    match t.kind {
                        | PreprocessorTokenType::Whitespace => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                        },
                        | PreprocessorTokenType::Newline => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                            t.contents = context.string_cache.intern(" ");
                            t.kind = PreprocessorTokenType::Whitespace;
                        },
                        | _ => *last_was_whitespace = false,
                    }
                    return Some(t);
                },
                | None => return None,
            }
        }
    }
}
