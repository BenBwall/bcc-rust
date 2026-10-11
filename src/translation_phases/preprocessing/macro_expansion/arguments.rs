//! Substitute macro arguments, caching prescans and preserving written
//! operands.
//!
//! C99: translation phase 4; invocation arguments, §6.10.3 paragraph 11,
//! p. 152; PDF p. 164; argument substitution, §6.10.3.1 paragraph 1, p. 153;
//! PDF p. 165. Stringification and pasting consume the resulting operand
//! frames.

use std::{
    cell::OnceCell,
    fmt::Debug,
    mem::replace,
};

use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        TranslationPhase,
        preprocessing::{
            Expander,
            runtime::{
                TokenizerFrame,
                TokenizerFrameType,
            },
        },
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl<'x> Expander<'_, '_, '_, 'x> {
    /// Substitutes the macro-replaced argument for a parameter that is not a
    /// `#` or `##` operand.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
    pub(in crate::translation_phases::preprocessing) fn handle_macro_argument(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame<'x>> {
        let arguments = self.get_arguments()?;
        let argument = find_argument(arguments, token.identifier_id(self.context))?;
        // Prescan is isolated from the replacement list. Rescanning the result
        // then uses the callee's disabled-name set, not the caller's.
        Some(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan { argument: true },
            tokenizer:  self.expanded_argument(token, argument),
        })
    }

    pub(in crate::translation_phases::preprocessing) fn get_arguments(
        &self,
    ) -> Option<MacroArguments<'x>> {
        match self.tokenizer_stack.last().map(|frame| &frame.frame_type) {
            // Argument tokens belong to the invocation's caller. Looking them
            // up in the callee's map can make a same-named parameter expand itself.
            | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                argument.enclosing_arguments,
            | Some(TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. }) =>
                Some(*arguments),
            | _ => None,
        }
    }

    pub(in crate::translation_phases::preprocessing) fn macro_argument_is_at_end(
        &mut self,
    ) -> bool {
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
        let variadic = argument.variadic;
        let depth = *paren_depth;
        let position = self.position();
        let next_is_end = loop {
            match self.tokenizer.next_item(self.context) {
                | Some(token)
                    if matches!(
                        token.kind,
                        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                    ) =>
                    continue,
                | Some(token) =>
                    break depth.is_some_and(|depth| {
                        self.update_macro_argument_paren_depth(token, variadic, depth)
                            .is_none()
                    }),
                | None => break true,
            }
        };
        self.set_position(position);
        next_is_end
    }

    /// Share prescan results with speculative invocation lookahead too:
    /// repeating replacement would also repeat its diagnostics and
    /// `_Pragma` effects.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165: the argument is
    /// replaced as if it formed the rest of the file, with no other tokens
    /// available.
    pub(super) fn expanded_argument(
        &mut self,
        token: PreprocessorToken,
        argument: &'x FunctionLikeMacroArgument<'x>,
    ) -> TokenSource<'x> {
        if let Some(tokenizer) = argument.expanded.get() {
            return tokenizer.clone();
        }
        let tokens = self.read_argument(argument, true);
        let location = self
            .context
            .first_source_vector_or_default(token.source_vectors);
        let tokenizer = TokenSource::replay(self.context, self.scratch, &[&tokens], location);
        drop(argument.expanded.set(tokenizer.clone()));
        tokenizer
    }

    fn raw_macro_argument_frame(&mut self, token: PreprocessorToken) -> Option<TokenizerFrame<'x>> {
        if let Some(arguments) = self.get_arguments()
            && let Some(arg) = find_argument(arguments, token.identifier_id(self.context))
        {
            let frame = TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                    argument:            arg,
                    paren_depth:         (!arg.substituted).then_some(1),
                    has_generated_token: false,
                },
                tokenizer:  arg.tokenizer.clone(),
            };
            return Some(frame);
        }

        None
    }

    /// Whether the token just read belongs to the `#` or `##` operand being
    /// replaced rather than to an argument substituted into it, or to a
    /// `__VA_OPT__` replacement, which is not rescanned.
    pub(in crate::translation_phases::preprocessing) fn is_reading_operand(&self) -> bool {
        let depth = self.tokenizer_stack.len();
        (self.operand_fence != 0 && depth == self.operand_fence)
            || (self.verbatim_fence != 0 && depth >= self.verbatim_fence)
            || matches!(
                self.tokenizer_stack.last().map(|frame| &frame.frame_type),
                Some(TokenizerFrameType::DeferredQuery)
            )
    }

    /// Returns the frame that reads `token`'s argument as a `##` operand.
    ///
    /// An argument written in a replacement list that has parameters of its
    /// own is replayed after those parameters are replaced, so the operand is
    /// what that enclosing replacement produced.
    pub(super) fn operand_frame(&mut self, token: PreprocessorToken) -> Option<TokenizerFrame<'x>> {
        let mut frame = self.raw_macro_argument_frame(token)?;
        let TokenizerFrameType::FunctionLikeMacroArgument {
            argument,
            paren_depth,
            ..
        } = &mut frame.frame_type
        else {
            unreachable!("macro arguments are read by argument frames");
        };
        if argument.enclosing_arguments.is_some() {
            let tokens = self.replace_operand_argument(argument);
            let empty_location = self
                .context
                .first_source_vector_or_default(token.source_vectors);
            let tokenizer =
                TokenSource::replay(self.context, self.scratch, &[&tokens], empty_location);
            // This frame reads its own replaced copy; the invocation keeps
            // the original argument.
            *argument = self.scratch.alloc(FunctionLikeMacroArgument {
                tokenizer: tokenizer.clone(),
                enclosing_arguments: None,
                ..argument.clone()
            });
            *paren_depth = None;
            frame.tokenizer = tokenizer;
        }
        Some(frame)
    }

    /// Reads `argument` as the `#` or `##` operand of the invocation on top
    /// of the stack, after replacing the parameters of the replacement list
    /// it was written in.
    ///
    /// C99 §6.10.3.1: those parameters are replaced by their fully
    /// macro-replaced arguments before the enclosing replacement list is
    /// rescanned, and only then does the nested invocation take its operand.
    /// The operand's own tokens are not macro-replaced. Leading and trailing
    /// whitespace is not part of the result.
    pub(super) fn replace_operand_argument(
        &mut self,
        argument: &'x FunctionLikeMacroArgument<'x>,
    ) -> ArenaVec<'x, PreprocessorToken> {
        self.read_argument(argument, false)
    }

    pub(super) fn read_argument(
        &mut self,
        argument: &'x FunctionLikeMacroArgument<'x>,
        expand: bool,
    ) -> ArenaVec<'x, PreprocessorToken> {
        let hash_hash_stack = replace(&mut self.hash_hash_stack, ArenaVec::new_in(self.scratch));
        let generate_placeholders = self.generate_placeholders;
        let newlines = (self.last_was_newline, self.current_is_newline);
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                argument,
                paren_depth: (!argument.substituted).then_some(1),
                has_generated_token: false,
            },
            tokenizer:  argument.tokenizer.clone(),
        });
        let depth = self.tokenizer_stack.len();
        let fence = replace(&mut self.operand_fence, if expand { 0 } else { depth });
        let expansion_fence = replace(&mut self.expansion_fence, depth);
        // A prescan replaces macros completely, even within a `__VA_OPT__`
        // replacement; an operand is read as it is.
        let verbatim_fence = if expand {
            replace(&mut self.verbatim_fence, 0)
        } else {
            self.verbatim_fence
        };
        let mut tokens = ArenaVec::new_in(self.scratch);
        while let Some(token) = self.next_preprocessor_token::<false>() {
            tokens.push(token);
        }
        self.operand_fence = fence;
        self.expansion_fence = expansion_fence;
        self.verbatim_fence = verbatim_fence;
        (self.last_was_newline, self.current_is_newline) = newlines;
        self.generate_placeholders = generate_placeholders;
        self.hash_hash_stack = hash_hash_stack;

        let is_whitespace = |token: &PreprocessorToken| {
            matches!(
                token.kind,
                PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
            )
        };
        let end = tokens
            .iter()
            .rposition(|token| !is_whitespace(token))
            .map_or(0, |i| i + 1);
        tokens.truncate(end);
        let start = tokens
            .iter()
            .position(|token| !is_whitespace(token))
            .unwrap_or(end);
        drop(tokens.drain(..start));
        tokens
    }

    /// The placemarker that stands for an empty `##` operand; it exists only
    /// within phase 4.
    ///
    /// C99: §6.10.3.3 paragraph 2 and footnote 151, p. 154; PDF p. 166.
    pub(super) fn placeholder(context: &mut Context<'_>) -> PreprocessorToken {
        PreprocessorToken {
            kind:           PreprocessorTokenType::Placeholder,
            contents:       context.string_cache.intern(""),
            source_vectors: SourceVectors::default(),
        }
    }

    pub(super) fn update_macro_argument_paren_depth(
        &self,
        token: PreprocessorToken,
        variadic: bool,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        // C99 §6.10.3p11: only a comma outside inner parentheses ends a
        // named argument.
        if paren_depth == 1
            && ((token.kind == PreprocessorTokenType::Comma && !variadic)
                || token.kind == PreprocessorTokenType::ClosingParenthesis)
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

    pub(in crate::translation_phases::preprocessing) fn current_is_header(&self) -> bool {
        // The first in the tokenizer stack is the original source file.
        for frame in self.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return true,
                | _ => (),
            }
        }
        false
    }
}

/// One argument of a function-like macro invocation. Its data lives in the
/// expansion arena and is shared by reference between the invocation and
/// the frames that read it.
///
/// C99: §6.10.3 paragraph 11, p. 152; PDF p. 164. The tokenizer reads the
/// argument as written, for `#` and `##` operands; `expanded` holds it
/// completely macro-replaced, for other parameter uses (§6.10.3.1 paragraph
/// 1, p. 153; PDF p. 165).
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct FunctionLikeMacroArgument<'x> {
    pub(in crate::translation_phases::preprocessing) omitted:             bool,
    pub(in crate::translation_phases::preprocessing) variadic:            bool,
    /// A `__VA_OPT__` result standing in for its replacement: the tokenizer
    /// replays the substituted tokens to their end, with no closing
    /// parenthesis, and they are never substituted again.
    /// C23: §6.10.5.1 paragraphs 4 and 7, pp. 179-180; PDF pp. 192-193.
    pub(in crate::translation_phases::preprocessing) substituted:         bool,
    pub(in crate::translation_phases::preprocessing) name:                StringCacheId,
    pub(in crate::translation_phases::preprocessing) tokenizer:           TokenSource<'x>,
    pub(in crate::translation_phases::preprocessing) enclosing_arguments:
        Option<MacroArguments<'x>>,
    pub(in crate::translation_phases::preprocessing) disabled_macros:     &'x [StringCacheId],
    /// Shared once-only argument prescan; raw #/## operands bypass it.
    pub(in crate::translation_phases::preprocessing) expanded: &'x OnceCell<TokenSource<'x>>,
}

/// The arguments of one invocation, each under its parameter's name.
pub(crate) type MacroArguments<'x> = &'x [FunctionLikeMacroArgument<'x>];

/// The argument for parameter `name`. Excess arguments kept for recovery
/// share one name; as with a map insert, the last one wins.
pub(in crate::translation_phases::preprocessing) fn find_argument(
    arguments: MacroArguments<'_>,
    name: StringCacheId,
) -> Option<&FunctionLikeMacroArgument<'_>> {
    arguments
        .iter()
        .rev()
        .find(|argument| argument.name == name)
}
