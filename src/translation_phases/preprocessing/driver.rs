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
    configuration::{
        CStandard,
        ExtensionPolicy,
    },
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SourcePosition,
        SourceVector,
        SourceVectors,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        HashMap,
        string_cache::StringCacheId,
    },
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType {
    /// Remainder of already substituted tokens in a boundary-crossing call.
    Rescan,
    SourceFile {
        /// Caller groups below this depth cannot be modified by this file.
        conditional_base:           usize,
        /// Physical file identity, unaffected by #line.
        physical_source_file_index: u32,
    },
    ObjectLikeMacroInvocation {
        name:           StringCacheId,
        invocation:     SourceVector,
        invocation_end: SourceVector,
    },
    FunctionLikeMacroInvocation {
        invocation:     SourceVector,
        invocation_end: SourceVector,
        name:           StringCacheId,
        arguments:      Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        is_variadic:    bool,
    },
    FunctionLikeMacroArgument {
        argument:            Box<FunctionLikeMacroArgument>,
        /// The parenthesis depth within an argument read from its invocation,
        /// or `None` for a replayed operand, which ends with its tokens.
        paren_depth:         Option<usize>,
        has_generated_token: bool,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame {
    pub(super) frame_type: TokenizerFrameType,
    pub(super) tokenizer:  TokenSource,
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
pub(super) fn string_literal_spelling(value: &str) -> String {
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
    pub(super) fn expansion_end(&self) -> Option<SourceVector> {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation { invocation_end, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation_end, .. } =>
                    return Some(invocation_end.clone()),
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::SourceFile { .. } => return None,
                | TokenizerFrameType::Rescan => (),
            }
        }
        None
    }

    fn invocation_location(&self, context: &Context, token: PreprocessorToken) -> SourceVector {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::ObjectLikeMacroInvocation { invocation, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation, .. } =>
                    return invocation.clone(),
                | TokenizerFrameType::FunctionLikeMacroArgument { .. }
                | TokenizerFrameType::SourceFile { .. } => break,
                | TokenizerFrameType::Rescan => (),
            }
        }
        context
            .get_source_vectors(token.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn physical_source_file_index(&self) -> u32 {
        self.tokenizer_stack
            .iter()
            .rev()
            .find_map(|frame| match frame.frame_type {
                | TokenizerFrameType::SourceFile {
                    physical_source_file_index,
                    ..
                } => Some(physical_source_file_index),
                | _ => None,
            })
            .unwrap_or_else(|| self.source_file_index())
    }

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

    /// A zero-length diagnostic location at the current input position.
    pub(super) fn current_location(&self, context: &mut Context) -> SourceVectors {
        self.location_at(context, self.position(context))
    }

    /// A zero-length diagnostic location at `position` of the current token
    /// source.
    pub(super) fn location_at(
        &self,
        context: &mut Context,
        position: SourcePosition,
    ) -> SourceVectors {
        self.tokenizer.location_at(context, position)
    }

    pub(super) fn push_tokenizer_frame(&mut self, _context: &mut Context, frame: TokenizerFrame) {
        self.tokenizer_stack.last_mut().unwrap().tokenizer = take(&mut self.tokenizer);
        self.tokenizer = frame.tokenizer.clone();
        self.tokenizer_stack.push(frame);
    }

    pub(super) fn pop_tokenizer_frame(&mut self, context: &mut Context) {
        let frame = self.tokenizer_stack.pop();
        if let Some(TokenizerFrame {
            frame_type:
                TokenizerFrameType::SourceFile {
                    conditional_base, ..
                },
            ..
        }) = frame
        {
            // File-local openings are owned copies, so their provenance
            // survives streaming token-arena compaction. Macro frame pops
            // leave conditional state untouched.
            let base = conditional_base.min(self.open_conditionals.len());
            for vectors in self.open_conditionals.split_off(base) {
                let source_vectors = context.push_source_vectors(&vectors.source);
                context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                    source_vectors,
                });
            }
        }
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
                    let source_vectors = self.location_at(context, start);
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
                    let source_vectors = self.location_at(context, start);
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
            let Some((mut token, mut is_pasted)) =
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
                        is_pasted = true;
                    } else {
                        break 'merge;
                    }
                }
            }
            if token.kind == PreprocessorTokenType::Placeholder {
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
            // C99 §6.10.3.1: parameters are replaced before the replacement
            // list is rescanned, so a parameter hides a macro of its name.
            // `##` runs after replacement, so its result names no parameter.
            if !is_pasted && let Some(frame) = self.handle_macro_argument(context, token) {
                self.push_tokenizer_frame(context, frame);
                continue;
            }
            if self.is_reading_operand() {
                // A `#` or `##` operand is not macro-replaced.
                break 'base Some(token);
            }

            if self.macro_is_disabled(token.identifier_id(context)) {
                token.kind = if token.kind == PreprocessorTokenType::UniversalIdentifier {
                    PreprocessorTokenType::UnavailableUniversalIdentifier
                } else {
                    PreprocessorTokenType::UnavailableIdentifier
                };
                break 'base Some(token);
            }
            if let Some(md) = self
                .macro_definitions
                .get(&token.identifier_id(context))
                .cloned()
            {
                match md {
                    | MacroDefinition::ObjectLike { tokenizer } => {
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation {
                                invocation_end: self.expansion_end().unwrap_or_else(|| {
                                    context
                                        .get_source_vectors(token.source_vectors)
                                        .last()
                                        .cloned()
                                        .unwrap_or_default()
                                }),
                                invocation:     self.invocation_location(context, token),
                                name:           token.identifier_id(context),
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
                        if !matches!(
                            self.tokenizer_stack.last().map(|f| &f.frame_type),
                            Some(TokenizerFrameType::SourceFile { .. })
                        ) {
                            let Some((arguments, invocation_end)) = self.capture_cross_frame_call(
                                context,
                                token,
                                &argument_names,
                                is_variadic,
                            ) else {
                                break 'base Some(token);
                            };
                            self.push_tokenizer_frame(
                                context,
                                TokenizerFrame {
                                    frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                        invocation_end,
                                        invocation: self.invocation_location(context, token),
                                        name: token.identifier_id(context),
                                        arguments: if arguments.is_empty() {
                                            self.empty_arguments.clone()
                                        } else {
                                            Rc::new(arguments)
                                        },
                                        is_variadic,
                                    },
                                    tokenizer,
                                },
                            );
                            continue;
                        }
                        let position = self.position(context);
                        // A source newline is whitespace between a function
                        // macro's name and `(`. In a replacement list it ends
                        // the frame and must not expose the definition's
                        // following source lines to this lookahead.
                        let source_file = matches!(
                            self.tokenizer_stack.last().map(|frame| &frame.frame_type),
                            Some(TokenizerFrameType::SourceFile { .. })
                        );
                        loop {
                            match self.tokenizer.next_item(context) {
                                | Some(brace)
                                    if brace.kind == PreprocessorTokenType::Whitespace
                                        || (source_file
                                            && brace.kind == PreprocessorTokenType::Newline) =>
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
                        // Excess arguments share a recovery map key, so map
                        // length cannot give the invocation's argument count.
                        let mut argument_count = 0;
                        let mut paren_depth = 1isize;
                        // Where the named arguments of a variadic macro met
                        // the closing parenthesis, leaving `...` without one.
                        let mut closed_at = None;
                        macro_rules! at {
                            () => {
                                argument_names
                                    .get(i)
                                    .copied()
                                    .unwrap_or_else(|| context.string_cache.intern("<undefined>"))
                            };
                        }
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let tokenizer = self.tokenizer.clone();
                            let mut has_argument_token = false;
                            loop {
                                let before = is_variadic.then(|| self.position(context));
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            closed_at = before;
                                            // F() supplies no arguments when
                                            // F has no parameters, but one
                                            // empty argument when it has one.
                                            argument_count = if i == 0
                                                && argument_names.is_empty()
                                                && !has_argument_token
                                            {
                                                0
                                            } else {
                                                i + 1
                                            };
                                            if argument_count != 0 {
                                                drop(arguments.insert(
                                                    at!(),
                                                    FunctionLikeMacroArgument {
                                                        expanded: Rc::default(),
                                                        name: at!(),
                                                        tokenizer,
                                                        enclosing_arguments:
                                                            enclosing_arguments.clone(),
                                                        disabled_macros: disabled_macros.clone(),
                                                    },
                                                ));
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
                                        drop(arguments.insert(
                                            at!(),
                                            FunctionLikeMacroArgument {
                                                expanded: Rc::default(),
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

                        let missing_named_arguments = if is_variadic {
                            closed_at.is_some() && argument_count < argument_names.len()
                        } else {
                            argument_count != argument_names.len()
                        };
                        if missing_named_arguments {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      argument_count,
                                    },
                                    source_vectors: token.source_vectors,
                                },
                            );
                        } else if closed_at.is_some() {
                            // C99 §6.10.3p4 requires an argument for `...`;
                            // omitting it is a common extension.
                            let extension_policy = match context.configuration.standard() {
                                | CStandard::C99 => context.configuration.extension_policy(),
                            };
                            if extension_policy != ExtensionPolicy::Allow {
                                context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::MissingVariadicArgument(
                                        extension_policy,
                                    ),
                                    source_vectors: token.source_vectors,
                                });
                            }
                        }
                        if is_variadic {
                            // Without an argument, `__VA_ARGS__` is empty: it
                            // reads only the closing parenthesis.
                            let va_args_tokenizer = match closed_at {
                                | Some(position) => {
                                    let mut closing = self.tokenizer.clone();
                                    closing.set_position(context, position);
                                    closing
                                },
                                | None => self.tokenizer.clone(),
                            };
                            drop(arguments.insert(
                                context.string_cache.intern("__VA_ARGS__"),
                                FunctionLikeMacroArgument {
                                    expanded:            Rc::default(),
                                    name:                context.string_cache.intern("__VA_ARGS__"),
                                    tokenizer:           va_args_tokenizer,
                                    enclosing_arguments: enclosing_arguments.clone(),
                                    disabled_macros:     disabled_macros.clone(),
                                },
                            ));
                            let mut paren_depth = 1isize;

                            if closed_at.is_none() {
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
                        }
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                invocation_end: SourceVector::new(
                                    self.position(context),
                                    self.source_file_index(),
                                    0,
                                ),
                                invocation: self.invocation_location(context, token),
                                name: token.identifier_id(context),
                                arguments: if arguments.is_empty() {
                                    self.empty_arguments.clone()
                                } else {
                                    Rc::new(arguments)
                                },
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
                            let invocation = self.invocation_location(context, token);
                            let spelling = string_literal_spelling(
                                &context.source_files[invocation.source_file_index]
                                    .to_string_lossy(),
                            );
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: context.push_source_vectors(&[invocation]),
                            });
                        },
                        | "__LINE__" => {
                            // Number spellings carry the trailing NUL that
                            // numeric conversion expects.
                            let invocation = self.invocation_location(context, token);
                            let spelling = format!("{}\0", invocation.line);
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: context.push_source_vectors(&[invocation]),
                            });
                        },
                        | name @ ("__STDC__"
                        | "__STDC_VERSION__"
                        | "__STDC_HOSTED__"
                        | "__STDC_MB_MIGHT_NEQ_WC__") => {
                            // C99 §6.10.8p1. This front end currently uses a
                            // freestanding execution model; the version must
                            // retain its prescribed long suffix.
                            // MB_MIGHT_NEQ_WC permits unequal codes; its 1
                            // does not assert that their values differ.
                            let spelling = if name == "__STDC_VERSION__" {
                                "199901L\0"
                            } else if name == "__STDC_HOSTED__" {
                                "0\0"
                            } else {
                                "1\0"
                            };
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       context.string_cache.intern(spelling),
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
                            self.tokenizer = TokenSource::new(context, pragma_string, input);
                            _ = self.parse_pragma_directive(context, string_token);
                            if self.tokenizer.next_item(context).is_some() {
                                let source_vectors = self.current_location(context);
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
            break 'base Some(token);
        };
        self.generate_placeholders = false;
        self.current_is_newline = ret.is_none_or(|t| t.kind == PreprocessorTokenType::Newline);
        ret
    }

    pub(super) fn next_ignore_whitespace(
        tokenizer: &mut TokenSource,
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
        tokenizer: &mut TokenSource,
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
