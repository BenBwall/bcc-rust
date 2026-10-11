//! Macro replacement reads arguments through the current frame stack,
//! substitutes parameters, applies stringification and token pasting, and
//! rescans the resulting tokens. Names already being replaced stay disabled
//! during rescanning. An argument's expanded tokens are cached for repeated
//! parameter uses; `#` and `##` read its written tokens instead. Directive
//! recognition belongs to the outer preprocessing loop.
//!
//! For `#define STR(x) #x` followed by `STR(a + b)`, replacement reads the
//! written argument and stringifies it to `"a + b"`. For `#define CAT(a,b) a ##
//! b`, `CAT(na,me)` pastes the two written tokens into `name`, which can then
//! be rescanned as another macro name.
//!
//! Read [`Expander::expand_macros`], [`Expander::handle_macro_argument`], and
//! [`Expander::handle_hash_hash_operator`] first. [`HashHash`] is the
//! pending-paste state; [`MacroDefinition`] and [`FunctionLikeMacroArgument`]
//! describe definitions and invocation arguments. The reader itself lives in
//! the parent entry file.
//!
//! Files by role:
//!
//! - Definitions and invocation lookahead: `macro_expansion/definitions.rs`,
//!   `macro_expansion/invocation.rs`.
//! - Argument substitution and dialect variants:
//!   `macro_expansion/arguments.rs`, `macro_expansion/variadic.rs`.
//! - Replacement operators: `macro_expansion/stringification.rs`,
//!   `macro_expansion/paste.rs`.
//!
//! C99: macro replacement, §6.10.3, pp. 151-153; PDF pp. 163-165.
//!
//! C99: argument substitution, §6.10.3.1, p. 153; PDF p. 165.
//!
//! C99: stringification, §6.10.3.2, p. 153; PDF p. 165;
//! token pasting, §6.10.3.3, p. 154; PDF p. 166.
//!
//! C99: rescanning, §6.10.3.4, p. 155; PDF p. 167.
//! The order of `#` and `##` evaluation is unspecified (§6.10.3.2 paragraph 2,
//! p. 153; PDF p. 165; §6.10.3.3 paragraph 3, p. 154; PDF p. 166).
//! Here stringification precedes an adjacent paste.

// Definitions and invocation lookahead.
mod definitions;
mod invocation;

// Argument substitution and dialect variants.
mod arguments;
mod variadic;

// Replacement operators.
mod paste;
mod stringification;

use std::{
    cell::OnceCell,
    fmt::Debug,
    mem::{
        replace,
        take,
    },
    ops::{
        ControlFlow,
        RangeBounds,
    },
};

pub(super) use arguments::find_argument;
pub(crate) use arguments::{
    FunctionLikeMacroArgument,
    MacroArguments,
};
pub(crate) use definitions::MacroDefinition;

use super::{
    Expander,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    runtime::{
        TokenizerFrame,
        TokenizerFrameType,
        spell_string_literal,
    },
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        GetPosition,
        SetPosition,
        SourceVectors,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl Expander<'_, '_, '_, '_> {
    /// Reads the next token from the frame stack, popping exhausted frames.
    /// A replacement list ends at its directive's new-line, and inside an
    /// argument a new-line is whitespace.
    ///
    /// C99: §6.10.3 paragraph 10, p. 152; PDF p. 164.
    fn expand_macros<const SHOULD_IGNORE_WHITESPACE: bool>(&mut self) -> Option<PreprocessorToken> {
        'base: loop {
            if self.tokenizer_stack.is_empty() {
                std::hint::cold_path();
                break 'base None;
            }
            if self.tokenizer_stack.len() < self.operand_fence.max(self.expansion_fence) {
                break 'base None;
            }
            match self.tokenizer.next_item(self.context) {
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
                                self.pop_tokenizer_frame();
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
                            let variadic = argument.variadic;
                            let has_generated_token = *has_generated_token;
                            let next_paren_depth = match paren_depth {
                                | Some(depth) => self
                                    .update_macro_argument_paren_depth(token, variadic, depth)
                                    .map(Some),
                                | None => Some(None),
                            };
                            if let Some(paren_depth) = next_paren_depth {
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
                                // An argument of only whitespace is empty.
                                *has_generated_token |= !matches!(
                                    token.kind,
                                    PreprocessorTokenType::Whitespace
                                        | PreprocessorTokenType::Newline
                                );
                            } else {
                                self.pop_tokenizer_frame();
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(Self::placeholder(self.context));
                                }
                                continue 'base;
                            }
                            if token.kind == PreprocessorTokenType::Newline {
                                if SHOULD_IGNORE_WHITESPACE {
                                    continue 'base;
                                }
                                token.kind = PreprocessorTokenType::Whitespace;
                                token.contents = self.context.string_cache.intern(" ");
                            }
                        },
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::Rescan { .. } | TokenizerFrameType::DeferredQuery,
                            ..
                        } => {
                            // A recovered directive boundary must return its
                            // source frame before raw conditional-group
                            // skipping starts. C99:
                            // §6.10.1p6, p. 149; PDF p. 161.
                            if token.kind == PreprocessorTokenType::Newline
                                && self.tokenizer.is_exhausted_replay()
                            {
                                self.pop_tokenizer_frame();
                            }
                        },
                        | TokenizerFrame {
                            frame_type: TokenizerFrameType::SourceFile { .. },
                            ..
                        } => (),
                    }
                    break Some(token);
                },
                | None => {
                    // A replayed operand ends with its tokens, and an empty
                    // one is a placeholder like any other (C99 §6.10.3.3p2).
                    let is_empty_replay = matches!(
                        self.tokenizer_stack.last(),
                        Some(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                                paren_depth: None,
                                has_generated_token: false,
                                ..
                            },
                            ..
                        })
                    );
                    self.pop_tokenizer_frame();
                    if self.generate_placeholders && is_empty_replay {
                        break 'base Some(Self::placeholder(self.context));
                    }
                    continue 'base;
                },
            }
        }
    }
}

/// A `##` paste waiting for an operand that an argument frame is still
/// producing. `Lhs` holds the left operand, to paste with the right
/// argument's first token; `Rhs` holds the right operand, to paste with the
/// left argument's last token; `Empty` marks a paste whose left argument is
/// still being read.
///
/// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum HashHash {
    Lhs(PreprocessorToken),
    Rhs(PreprocessorToken),
    Empty,
}
