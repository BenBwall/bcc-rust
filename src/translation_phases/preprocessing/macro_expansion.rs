//! Macro definitions, argument collection, and `#`/`##` operators.

use std::{
    fmt::Debug,
    ops::{
        ControlFlow,
        RangeBounds,
    },
    rc::Rc,
};

use super::{
    Preprocessor,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SourceVectors,
        TokenString,
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
pub(crate) enum MacroDefinition {
    ObjectLike {
        tokenizer: PreprocessorTokenizer,
    },
    FunctionLike {
        argument_names: Rc<[StringCacheId]>,
        tokenizer:      PreprocessorTokenizer,
        is_variadic:    bool,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum HashHash {
    Lhs(PreprocessorToken),
    Rhs(PreprocessorToken),
    Empty,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct FunctionLikeMacroArgument {
    pub(super) name:                StringCacheId,
    pub(super) tokenizer:           PreprocessorTokenizer,
    pub(super) enclosing_arguments: Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>>,
    pub(super) disabled_macros:     Rc<[StringCacheId]>,
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl Preprocessor {
    pub(super) fn get_arguments(
        &self,
        _context: &Context,
    ) -> Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>> {
        match self.tokenizer_stack.last().map(|frame| &frame.frame_type) {
            // Argument tokens belong to the invocation's caller. Looking them
            // up in the callee's map can make a same-named parameter expand itself.
            | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                argument.enclosing_arguments.clone(),
            | Some(TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. }) =>
                Some(arguments.clone()),
            | _ => None,
        }
    }

    pub(super) fn macro_argument_is_at_end(&mut self, context: &mut Context) -> bool {
        let Some(TokenizerFrame {
            frame_type:
                TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    paren_depth,
                    ..
                },
            ..
        }) = self.tokenizer_stack.last()
        else {
            return false;
        };
        let name = argument.name;
        let depth = *paren_depth;
        let position = self.position(context);
        let next_is_end = loop {
            match self.tokenizer.next_item(context) {
                | Some(token)
                    if matches!(
                        token.kind,
                        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                    ) =>
                    continue,
                | Some(token) =>
                    break self
                        .update_macro_argument_paren_depth(context, token, name, depth)
                        .is_none(),
                | None => break true,
            }
        };
        self.set_position(context, position);
        next_is_end
    }

    pub(super) fn macro_is_disabled(&self, name: StringCacheId) -> bool {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    return argument.disabled_macros.contains(&name);
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name: active }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name: active, .. }
                    if *active == name =>
                    return true,
                | _ => (),
            }
        }
        false
    }

    pub(super) fn disabled_macros(&self) -> Rc<[StringCacheId]> {
        let mut names = Vec::new();
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    names.extend_from_slice(&argument.disabled_macros);
                    break;
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name, .. } => names.push(*name),
                | _ => (),
            }
        }
        Rc::from(names)
    }

    pub(super) fn handle_macro_argument(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        if let Some(arguments) = self.get_arguments(context)
            && let Some(arg) = arguments.get(&token.contents)
        {
            let frame = TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                    argument:            Box::new(arg.clone()),
                    paren_depth:         1,
                    has_generated_token: false,
                },
                tokenizer:  arg.tokenizer.clone(),
            };
            return Some(frame);
        }

        None
    }

    fn parse_hash_hash_operator(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        let lhs_frame = self.handle_macro_argument(context, lhs);
        let rhs_frame = self.handle_macro_argument(context, rhs);
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = rhs_frame {
            self.push_tokenizer_frame(context, frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs);
            false
        };
        if let Some(frame) = lhs_frame {
            self.push_tokenizer_frame(context, frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs);
        } else {
            _ = self.hash_hash_stack.pop();
            return self.merge_tokens(context, lhs, rhs);
        }
        None
    }

    fn parse_hash_operator(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> PreprocessorToken {
        let position = self.position(context);
        let Some(argument_name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing '#' operator in function-like macro invocation.",
        ) else {
            self.set_position(context, position);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::GeneratedString,
                contents:       context.string_cache.intern(""),
                source_vectors: token.source_vectors,
            };
        };
        let (mut token_tokenizer, argument_id) = match self.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match arguments.get(&argument_name.contents) {
                | None => {
                    self.set_position(context, position);
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                context.string_cache.at(argument_name.contents).to_owned(),
                            ),
                        source_vectors: argument_name.source_vectors,
                    });
                    return PreprocessorToken {
                        kind:           PreprocessorTokenType::GeneratedString,
                        contents:       context.string_cache.intern(""),
                        source_vectors: token.source_vectors,
                    };
                },
                | Some(v) => (v.tokenizer.clone(), v.name),
            },
            | _ => unreachable!(),
        };
        let mut last_was_whitespace = true;
        let mut synthetic_contents = String::new();
        let mut paren_depth = 1;
        'base: loop {
            let Some(token) = Self::next_treat_newlines_as_whitespace(
                &mut token_tokenizer,
                context,
                &mut last_was_whitespace,
            ) else {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing '#' operator in function-like macro invocation",
                    ),
                    source_vectors: argument_name.source_vectors,
                });
                break 'base;
            };
            match self.update_macro_argument_paren_depth(context, token, argument_id, paren_depth) {
                | Some(depth) => paren_depth = depth,
                | None => break,
            }
            let mut s = context.string_cache.at(token.contents);
            if token.kind == PreprocessorTokenType::Number {
                // Remove trailing null byte.
                s = &s[..s.len() - 1];
            }
            synthetic_contents.push_str(s);
        }
        PreprocessorToken {
            kind:           PreprocessorTokenType::GeneratedString,
            contents:       context.string_cache.intern(&synthetic_contents),
            source_vectors: token.source_vectors,
        }
    }

    fn merge_token_contents(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        self.merge_token_contents_with_ranges(context, lhs, rhs, .., .., result_token_type)
    }

    fn merge_token_contents_with_ranges(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        lhs_range: impl RangeBounds<usize>,
        rhs_range: impl RangeBounds<usize>,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        _ = self;
        let mut new_contents = TokenString::new();
        new_contents.push_str(
            &context.string_cache.at(lhs.contents)[(
                lhs_range.start_bound().cloned(),
                lhs_range.end_bound().cloned(),
            )],
        );
        new_contents.push_str(
            &context.string_cache.at(rhs.contents)[(
                rhs_range.start_bound().cloned(),
                rhs_range.end_bound().cloned(),
            )],
        );
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: result_token_type,
            contents: context.string_cache.intern(&new_contents),
            source_vectors,
        }
    }

    fn create_merge_error(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        _ = self;
        let lhs_contents = context.string_cache.at(lhs.contents).to_string();
        let rhs_contents = context.string_cache.at(rhs.contents).to_string();
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    pub(super) fn merge_tokens(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        match (lhs.kind, rhs.kind) {
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(rhs),
            | (_, PreprocessorTokenType::Placeholder) => Some(lhs),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                let rhs_contents = context.string_cache.at(rhs.contents);
                if rhs_contents.contains('.') {
                    return Some(self.create_merge_error(context, lhs, rhs));
                }
                let rhs_range = if rhs.kind == PreprocessorTokenType::Number {
                    0..rhs_contents.len() - 1
                } else {
                    0..rhs_contents.len()
                };
                let new = self.merge_token_contents_with_ranges(
                    context,
                    lhs,
                    rhs,
                    ..,
                    rhs_range,
                    PreprocessorTokenType::Identifier,
                );
                let kind = if context.string_cache.at(new.contents) == "defined" {
                    PreprocessorTokenType::Defined
                } else {
                    PreprocessorTokenType::Identifier
                };
                Some(PreprocessorToken {
                    kind,
                    contents: new.contents,
                    source_vectors: new.source_vectors,
                })
            },
            | (
                PreprocessorTokenType::Identifier,
                PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString,
            ) => {
                // C99 §6.10.3.3p3: `L ## "x"` forms the wide literal `L"x"`,
                // so the right-hand side must still be a narrow literal.
                if context.string_cache.at(lhs.contents) == "L"
                    && !context.string_cache.at(rhs.contents).starts_with('L')
                {
                    Some(self.merge_token_contents(
                        context,
                        lhs,
                        rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    ))
                } else {
                    Some(self.create_merge_error(context, lhs, rhs))
                }
            },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            )
            | (PreprocessorTokenType::Number, PreprocessorTokenType::Period) => {
                let lhs_contents = context.string_cache.at(lhs.contents);
                let lhs_range = if lhs.kind == PreprocessorTokenType::Number {
                    0..lhs_contents.len() - 1
                } else {
                    0..lhs_contents.len()
                };
                // We don't remove the trailing null byte from the right-hand
                // side because we're generating a new number
                // token, and number tokens should always have a
                // trailing null byte.
                Some(self.merge_token_contents_with_ranges(
                    context,
                    lhs,
                    rhs,
                    lhs_range,
                    ..,
                    PreprocessorTokenType::Number,
                ))
            },
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusMinus),
            ),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusEquals),
            ),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusEquals),
            ),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::AsteriskEquals),
            ),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ForwardSlashEquals,
                )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PercentEquals),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThan,
                )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThan,
                )),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::LessThanEquals),
            ),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanEquals,
                )),
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThanEquals,
                )),
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandEquals,
                )),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::CaretEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipeEquals),
            ),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ExclamationMarkEquals,
                )),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::EqualsEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipePipe)),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandAmpersand,
                )),

            | _ => Some(self.create_merge_error(context, lhs, rhs)),
        }
    }

    fn expand_macros<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            if self.tokenizer_stack.is_empty() {
                std::hint::cold_path();
                break 'base None;
            }
            match self.tokenizer.next_item(context) {
                | Some(token)
                    if token.kind == PreprocessorTokenType::Whitespace
                        && SHOULD_IGNORE_WHITESPACE =>
                {
                    continue 'base;
                },
                | Some(mut token) => {
                    match self.tokenizer_stack.last_mut().unwrap() {
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                                | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                            ..
                        } =>
                            if token.kind == PreprocessorTokenType::Newline {
                                self.pop_tokenizer_frame(context);
                                continue 'base;
                            },
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroArgument {
                                    paren_depth,
                                    argument,
                                    has_generated_token,
                                },
                            ..
                        } => {
                            let paren_depth = *paren_depth;
                            let argument = argument.clone();
                            let has_generated_token = *has_generated_token;
                            if let Some(paren_depth) = self.update_macro_argument_paren_depth(
                                context,
                                token,
                                argument.name,
                                paren_depth,
                            ) {
                                let TokenizerFrame {
                                    frame_type:
                                        TokenizerFrameType::FunctionLikeMacroArgument {
                                            paren_depth: p,
                                            has_generated_token,
                                            ..
                                        },
                                    ..
                                } = self.tokenizer_stack.last_mut().unwrap()
                                else {
                                    unreachable!();
                                };
                                *p = paren_depth;
                                *has_generated_token = true;
                            } else {
                                self.pop_tokenizer_frame(context);
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(PreprocessorToken {
                                        kind:           PreprocessorTokenType::Placeholder,
                                        contents:       context.string_cache.intern(""),
                                        source_vectors: SourceVectors::default(),
                                    });
                                }
                                continue 'base;
                            }
                            if token.kind == PreprocessorTokenType::Newline {
                                if SHOULD_IGNORE_WHITESPACE {
                                    continue 'base;
                                }
                                token.kind = PreprocessorTokenType::Whitespace;
                                token.contents = context.string_cache.intern(" ");
                            }
                        },
                        | TokenizerFrame {
                            frame_type: TokenizerFrameType::SourceFile,
                            ..
                        } => (),
                    }
                    break Some(token);
                },
                | None => {
                    self.pop_tokenizer_frame(context);
                    continue 'base;
                },
            }
        }
    }

    fn update_macro_argument_paren_depth(
        &self,
        context: &Context,
        token: PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        if (token.kind == PreprocessorTokenType::Comma
            && context.string_cache.at(argument_name) != "__VA_ARGS__")
            || (token.kind == PreprocessorTokenType::ClosingParenthesis && paren_depth == 1)
        {
            return None;
        }
        if token.kind == PreprocessorTokenType::OpeningParenthesis {
            return Some(paren_depth + 1);
        }
        if token.kind == PreprocessorTokenType::ClosingParenthesis {
            return Some(paren_depth - 1);
        }
        Some(paren_depth)
    }

    pub(super) fn current_is_header(&self, _context: &Context) -> bool {
        // The first in the tokenizer stack is the original source file.
        for frame in self.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile => return true,
                | _ => (),
            }
        }
        false
    }

    #[expect(
        dead_code,
        clippy::type_complexity,
        reason = "Macro-frame inspection is retained for pending expansion paths."
    )]
    fn current_function_like_macro(
        &self,
        _context: &Context,
    ) -> Option<(
        u32,
        PreprocessorTokenizer,
        Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        bool,
    )> {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation {
                        arguments,
                        is_variadic,
                        ..
                    },
                tokenizer,
            }) => Some((
                tokenizer.source_file_index(),
                tokenizer.clone(),
                arguments.clone(),
                *is_variadic,
            )),
            | _ => None,
        }
    }

    #[expect(
        dead_code,
        reason = "Macro-frame inspection is retained for pending expansion paths."
    )]
    fn current_is_function_like_macro(&self, _context: &Context) -> bool {
        matches!(
            self.tokenizer_stack.last(),
            Some(TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                ..
            })
        )
    }

    fn handle_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        let token = self.expand_macros::<SHOULD_IGNORE_WHITESPACE>(context)?;

        match token.kind {
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    Some(self.parse_hash_operator(context, token))
                } else {
                    Some(token)
                }
            },
            | _ => Some(token),
        }
    }

    pub(super) fn handle_hash_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            let lhs = self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)?;
            let replacement_list = match self.tokenizer_stack.last().map(|frame| &frame.frame_type)
            {
                | Some(
                    TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                    | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                ) => true,
                | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                    argument.enclosing_arguments.is_some(),
                | _ => false,
            };
            let hash_hash = if replacement_list {
                let save = self.position(context);
                context.set_ignore_tokenizer_errors(true);
                let hash_hash = Self::next_ignore_whitespace(&mut self.tokenizer, context);
                context.set_ignore_tokenizer_errors(false);
                self.set_position(context, save);
                if hash_hash.is_some_and(|v| v.kind == PreprocessorTokenType::HashHash) {
                    Self::next_ignore_whitespace(&mut self.tokenizer, context)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(h) = hash_hash {
                let Some(rhs) = self.handle_hash_hash_operator::<true>(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.tokenizer.position(context),
                        self.tokenizer.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing hash-hash operator. Hash hash operator must be followed by a \
                             preprocessor token on the same line.",
                        ),
                        source_vectors,
                    });
                    return Some(lhs);
                };
                if let Some(r) = self.parse_hash_hash_operator(context, lhs, h, rhs) {
                    return Some(r);
                }
                continue 'base;
            }
            return Some(lhs);
        }
    }
}
