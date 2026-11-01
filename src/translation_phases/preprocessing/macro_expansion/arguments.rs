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
    /// `#` or `##` operand, pushing it to be rescanned. Returns whether
    /// `token` was such a parameter.
    ///
    /// Out of line, as the frames built here would otherwise enlarge the
    /// reader's own frame, which every argument prescan nests once more.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn handle_macro_argument(
        &mut self,
        token: PreprocessorToken,
    ) -> bool {
        let Some(argument) = self
            .get_arguments()
            .and_then(|arguments| find_argument(arguments, token.identifier_id(self.context)))
        else {
            return false;
        };
        // Prescan is isolated from the replacement list. Rescanning the result
        // then uses the callee's disabled-name set, not the caller's. This
        // frame, which nested arguments stack up, holds neither the replay
        // nor the frame that reads it.
        self.prescan_argument(token, argument);
        self.rescan_argument(argument);
        true
    }

    /// Reads the replay of `argument`'s prescan, which
    /// [`Self::prescan_argument`] kept, in place of its parameter.
    #[inline(never)]
    fn rescan_argument(&mut self, argument: &'x FunctionLikeMacroArgument<'x>) {
        let tokenizer = argument
            .expanded
            .get()
            .expect("the argument was prescanned")
            .clone();
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan { argument: true },
            tokenizer,
        });
    }

    pub(in crate::translation_phases::preprocessing) fn get_arguments(
        &self,
    ) -> Option<MacroArguments<'x>> {
        match self.tokenizer_stack.last().map(|frame| &frame.frame_type) {
            // Argument tokens belong to the invocation's caller. Looking them
            // up in the callee's map can make a same-named parameter expand
            // itself.
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
                        self.update_macro_argument_paren_depth(token.kind, variadic, depth)
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
        self.prescan_argument(token, argument);
        argument
            .expanded
            .get()
            .expect("the argument was prescanned")
            .clone()
    }

    /// Prescans `argument` for its parameter, here `token`, unless that was
    /// done: [`Self::expanded_argument`] without the replay. Inlined, as
    /// one frame fewer for each argument prescan that nests leaves room for
    /// deeper nesting.
    #[inline(always)]
    fn prescan_argument(
        &mut self,
        token: PreprocessorToken,
        argument: &'x FunctionLikeMacroArgument<'x>,
    ) {
        if argument.expanded.get().is_some() {
            return;
        }
        // Only the parameter's place outlives the prescan, so the frames of
        // the prescans that nest do not hold the whole token.
        let source_vectors = token.source_vectors;
        let tokens = self.read_argument(argument, true);
        self.keep_expansion(source_vectors, argument, &tokens);
    }

    /// Keeps `tokens`, the prescan of `argument`, as a replay for each use
    /// of its parameter, here the one at `source_vectors`. Out of line, so
    /// that the frames of the prescans that nest do not hold the replay.
    #[inline(never)]
    fn keep_expansion(
        &mut self,
        source_vectors: SourceVectors,
        argument: &'x FunctionLikeMacroArgument<'x>,
        tokens: &[PreprocessorToken],
    ) {
        let location = self.context.first_source_vector_or_default(source_vectors);
        let tokenizer = TokenSource::replay(self.context, self.scratch, &[tokens], location);
        drop(argument.expanded.set(tokenizer));
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
                    released:            false,
                    operand:             false,
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
            operand,
            ..
        } = &mut frame.frame_type
        else {
            unreachable!("macro arguments are read by argument frames");
        };
        *operand = true;
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

    /// Reads `argument` to its end through the reader, macro-replacing it
    /// with `expand`, and returns its tokens without leading or trailing
    /// white space.
    ///
    /// Inlined: a prescan nests the reader once per argument nested in an
    /// argument, and each frame fewer per level lets the explorer's
    /// WebAssembly stack hold deeper nesting.
    #[inline(always)]
    pub(super) fn read_argument(
        &mut self,
        argument: &'x FunctionLikeMacroArgument<'x>,
        expand: bool,
    ) -> ArenaVec<'x, PreprocessorToken> {
        let hash_hash_stack = replace(&mut self.hash_hash_stack, ArenaVec::new_in(self.scratch));
        let generate_placeholders = self.generate_placeholders;
        let newlines = (self.last_was_newline, self.current_is_newline);
        self.push_argument_frame(argument);
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

    /// Whether an argument's prescan is reading (§6.10.3.1 paragraph 1),
    /// in which a pragma operator is left in the argument's replacement,
    /// to be carried out where the argument is rescanned in place. Its
    /// argument's frame is the one [`Self::read_argument`] fenced.
    pub(in crate::translation_phases::preprocessing) fn in_prescan(&self) -> bool {
        self.operand_fence == 0
            && self
                .expansion_fence
                .checked_sub(1)
                .and_then(|index| self.tokenizer_stack.get(index))
                .is_some_and(|frame| {
                    matches!(
                        frame.frame_type,
                        TokenizerFrameType::FunctionLikeMacroArgument { .. }
                    )
                })
    }

    /// Pushes a frame that reads `argument` as written. Out of line, so that
    /// the frames of the prescans that nest do not hold it.
    #[inline(never)]
    fn push_argument_frame(&mut self, argument: &'x FunctionLikeMacroArgument<'x>) {
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                argument,
                paren_depth: (!argument.substituted).then_some(1),
                has_generated_token: false,
                released: false,
                operand: false,
            },
            tokenizer:  argument.tokenizer.clone(),
        });
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
        kind: PreprocessorTokenType,
        variadic: bool,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        // C99 §6.10.3p11: only a comma outside inner parentheses ends a
        // named argument.
        if paren_depth == 1
            && ((kind == PreprocessorTokenType::Comma && !variadic)
                || kind == PreprocessorTokenType::ClosingParenthesis)
        {
            return None;
        }
        if kind == PreprocessorTokenType::OpeningParenthesis {
            return Some(paren_depth + 1);
        }
        if kind == PreprocessorTokenType::ClosingParenthesis {
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
pub(super) fn find_argument(
    arguments: MacroArguments<'_>,
    name: StringCacheId,
) -> Option<&FunctionLikeMacroArgument<'_>> {
    arguments
        .iter()
        .rev()
        .find(|argument| argument.name == name)
}
