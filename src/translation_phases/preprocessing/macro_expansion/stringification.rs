//! Replace hash operands with stringified written arguments.
//!
//! C99: translation phase 4; stringification, §6.10.3.2, p. 153; PDF p. 165.
//! Pasting and subsequent rescanning belong to their separate readers.

use std::{
    mem::take,
    ops::ControlFlow,
};

use super::find_argument;
use crate::{
    translation_phases::{
        Context,
        preprocessing::{
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
        },
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::{
        bump::{
            ArenaString,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl Expander<'_, '_, '_, '_> {
    /// Reads the next token, applying `#` when it is one. `#` is an operator
    /// only in the replacement list of a function-like macro.
    ///
    /// C99: §6.10.3.2 paragraph 1, p. 153; PDF p. 165.
    pub(in crate::translation_phases::preprocessing) fn handle_hash_operator<
        const SHOULD_IGNORE_WHITESPACE: bool,
    >(
        &mut self,
    ) -> Option<PreprocessorToken> {
        let token = self.expand_macros::<SHOULD_IGNORE_WHITESPACE>()?;

        match token.kind {
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    Some(self.parse_hash_operator(token))
                } else {
                    Some(token)
                }
            },
            | _ => Some(token),
        }
    }

    /// Replaces `#` and the parameter after it with a character string
    /// literal spelling the argument as written: inner whitespace becomes one
    /// space, outer whitespace is dropped, and an empty argument gives `""`.
    ///
    /// C99: §6.10.3.2 paragraphs 1-2, p. 153; PDF p. 165.
    fn parse_hash_operator(&mut self, token: PreprocessorToken) -> PreprocessorToken {
        let position = self.position();
        let Some(argument_name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing '#' operator in function-like macro invocation.",
        ) else {
            self.set_position(position);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::String,
                contents:       self.context.string_cache.intern("\"\""),
                source_vectors: token.source_vectors,
            };
        };
        let argument = match self.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match find_argument(arguments, argument_name.identifier_id(self.context)) {
                | None => {
                    self.set_position(position);
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                self.context.diagnostic_text(
                                    self.context.string_cache.at(argument_name.contents),
                                ),
                            ),
                        source_vectors: argument_name.source_vectors,
                    });
                    return PreprocessorToken {
                        kind:           PreprocessorTokenType::String,
                        contents:       self.context.string_cache.intern("\"\""),
                        source_vectors: token.source_vectors,
                    };
                },
                | Some(v) => v,
            },
            | _ => unreachable!(),
        };
        if argument.enclosing_arguments.is_some() {
            let tokens = self.replace_operand_argument(argument);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::String,
                contents:       Self::stringify(self.context, self.scratch, &tokens),
                source_vectors: token.source_vectors,
            };
        }
        let variadic = argument.variadic;
        let mut token_tokenizer = argument.tokenizer.clone();
        let mut last_was_whitespace = true;
        let mut synthetic_contents = ArenaString::new_in(self.scratch);
        synthetic_contents.push('"');
        let mut pending_space = false;
        let mut paren_depth = 1;
        'base: loop {
            let Some(token) = Self::next_treat_newlines_as_whitespace(
                &mut token_tokenizer,
                self.context,
                &mut last_was_whitespace,
            ) else {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing '#' operator in function-like macro invocation",
                    ),
                    source_vectors: argument_name.source_vectors,
                });
                break 'base;
            };
            match self.update_macro_argument_paren_depth(token, variadic, paren_depth) {
                | Some(depth) => paren_depth = depth,
                | None => break,
            }
            // C99 6.10.3.2p2: whitespace separates argument tokens, but
            // trailing whitespace is not part of the resulting string.
            if token.kind == PreprocessorTokenType::Whitespace {
                pending_space = true;
                continue;
            }
            if take(&mut pending_space) {
                synthetic_contents.push(' ');
            }
            Self::append_stringified_token(self.context, &mut synthetic_contents, token);
        }
        synthetic_contents.push('"');
        PreprocessorToken {
            kind:           PreprocessorTokenType::String,
            contents:       self.context.string_cache.intern(&*synthetic_contents),
            source_vectors: token.source_vectors,
        }
    }

    /// Spells replaced operand tokens as `#` does (C99 §6.10.3.2p2), with
    /// each run of whitespace as one space.
    pub(in crate::translation_phases::preprocessing) fn stringify(
        context: &mut Context<'_>,
        scratch: &Bump,
        tokens: &[PreprocessorToken],
    ) -> StringCacheId {
        let mut spelling = ArenaString::new_in(scratch);
        spelling.push('"');
        let mut space = false;
        for token in tokens {
            match token.kind {
                | PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline => {
                    space = true;
                    continue;
                },
                | PreprocessorTokenType::Placeholder => continue,
                | _ =>
                    if take(&mut space) {
                        spelling.push(' ');
                    },
            }
            Self::append_stringified_token(context, &mut spelling, *token);
        }
        spelling.push('"');
        context.string_cache.intern(&*spelling)
    }

    /// C99 6.10.3.2p2: escape quotes/backslashes only inside literal tokens.
    /// A backslash that was an Other token participates in phase-5 escapes.
    ///
    /// Whether a `\` is inserted before the `\` that begins a universal
    /// character name is implementation-defined. Inside a literal one is
    /// inserted, as before any other `\`; an identifier's spelling is copied
    /// as it is.
    ///
    /// C99: §6.10.3.2 paragraph 2, p. 153; PDF p. 165.
    fn append_stringified_token(
        context: &Context<'_>,
        spelling: &mut ArenaString<'_>,
        token: PreprocessorToken,
    ) {
        let contents = context.string_cache.at(token.contents);
        let mut escaped = |c: char| {
            if matches!(c, '\\' | '"') {
                spelling.push('\\');
            }
            spelling.push(c);
        };
        match token.kind {
            | PreprocessorTokenType::Number =>
                spelling.push_str(contents.strip_suffix('\0').unwrap_or(contents)),
            | PreprocessorTokenType::GeneratedString => spell_string_literal(contents, escaped),
            | PreprocessorTokenType::WideGeneratedString => {
                let prefix_length = if contents.starts_with("u8") { 2 } else { 1 };
                contents[..prefix_length].chars().for_each(&mut escaped);
                spell_string_literal(&contents[prefix_length..], escaped);
            },
            | PreprocessorTokenType::String | PreprocessorTokenType::Character =>
                contents.chars().for_each(escaped),
            | _ => spelling.push_str(contents),
        }
    }
}
