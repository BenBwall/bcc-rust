//! Capture macro calls across frame boundaries and track disabled macro names.
//!
//! C99: translation phase 4; replacement rescanning, §6.10.3.4 paragraphs 1-2,
//! p. 155; PDF p. 167. Lookahead commits only when it recognizes a call;
//! parameter substitution belongs to the argument reader.

use std::cell::OnceCell;

use super::{
    FunctionLikeMacroArgument,
    MacroArguments,
    find_argument,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        GetPosition,
        SetPosition,
        TranslationPhase,
        preprocessing::{
            Expander,
            errors::{
                PreprocessorError,
                PreprocessorErrorType,
            },
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

impl<'x> Expander<'_, '_, '_, 'x> {
    /// Capture an invocation whose opening, arguments, or closing delimiter
    /// can come from different replacement/argument/source frames.
    ///
    /// C99: §6.10.3.4 paragraph 1, p. 155; PDF p. 167; argument separation
    /// and count, §6.10.3 paragraphs 4 and 11-12, pp. 151-153; PDF
    /// pp. 163-165.
    pub(in crate::translation_phases::preprocessing) fn capture_cross_frame_call(
        &mut self,
        invocation: PreprocessorToken,
        names: &[StringCacheId],
        variadic_name: Option<StringCacheId>,
    ) -> Option<(MacroArguments<'x>, crate::translation_phases::SourceVector)> {
        let variadic = variadic_name.is_some();
        let mut cursor = MacroCallCursor::new(self);
        let opening = loop {
            let token = cursor.next(self)?;
            if !matches!(
                token.kind,
                PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
            ) {
                break token;
            }
        };
        if opening.kind != PreprocessorTokenType::OpeningParenthesis {
            return None;
        }
        // Argument tokens in order, and where each argument's tokens end.
        let mut grouped = ArenaVec::new_in(self.scratch);
        let mut group_ends = ArenaVec::new_in(self.scratch);
        let mut depth = 1usize;
        let closing = loop {
            let Some(mut token) = cursor.next(self) else {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing function-like macro invocation",
                    ),
                    source_vectors: invocation.source_vectors,
                });
                return None;
            };
            match token.kind {
                | PreprocessorTokenType::OpeningParenthesis => depth += 1,
                | PreprocessorTokenType::ClosingParenthesis => {
                    depth -= 1;
                    if depth == 0 {
                        break token;
                    }
                },
                | PreprocessorTokenType::Comma
                    if depth == 1 && (!variadic || group_ends.len() < names.len()) =>
                {
                    group_ends.push(grouped.len());
                    continue;
                },
                | _ => (),
            }
            // C99 §6.10.3.4p2: retain the paint of tokens taken from a
            // replacement, even when the call closes in the rest of source.
            if matches!(
                token.kind,
                PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier
            ) && Self::macro_is_disabled_in(
                cursor.active_frames(),
                token.identifier_id(self.context),
            ) {
                token.kind = if token.kind == PreprocessorTokenType::UniversalIdentifier {
                    PreprocessorTokenType::UnavailableUniversalIdentifier
                } else {
                    PreprocessorTokenType::UnavailableIdentifier
                };
            }
            grouped.push(token);
        };
        group_ends.push(grouped.len());
        let empty = group_ends.len() == 1
            && grouped.iter().all(|token: &PreprocessorToken| {
                matches!(
                    token.kind,
                    PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                )
            });
        let count = if names.is_empty() && !variadic && empty {
            0
        } else {
            group_ends.len()
        };
        if count != 0
            && !(variadic
                && names.is_empty()
                && empty
                && self.context.configuration.gnu_extensions())
        {
            let mut start = 0;
            for &end in &group_ends {
                if grouped[start..end].iter().all(|t| {
                    matches!(
                        t.kind,
                        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                    )
                }) {
                    self.report_empty_macro_argument(invocation.source_vectors);
                }
                start = end;
            }
        }
        if (!variadic && count != names.len()) || (variadic && count < names.len()) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:
                    PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                        expected: names.len(),
                        found:    count,
                    },
                source_vectors: invocation.source_vectors,
            });
        } else if variadic && count == names.len() {
            // C99 §6.10.3p4 requires an argument for `...`; omitting it is
            // an extension (§4p6), which the extension policy governs.
            if !self.context.configuration.accepts(Feature::MsVaArgs) {
                self.context.preprocessor_extension(
                    Feature::OmittedVariadicArguments,
                    crate::translation_phases::DiagnosticPolicy::Extension,
                    invocation.source_vectors,
                    PreprocessorErrorType::MissingVariadicArgument,
                );
            }
        }
        // The closing delimiter belongs to the invocation's caller. Keep
        // exhausted frames on the real stack for replacement rescanning, but
        // do not let them disable macros in arguments from later tokens.
        let disabled_macros = self.disabled_macros_in(cursor.active_frames());
        let location = self
            .context
            .first_source_vector_or_default(closing.source_vectors);
        let parameters = names.len() + usize::from(variadic);
        let mut arguments = ArenaVec::with_capacity_in(parameters, self.scratch);
        for (i, name) in names.iter().copied().chain(variadic_name).enumerate() {
            // A normal argument cursor is terminated by a closing parenthesis.
            // Keep that sentinel so all existing raw #/## readers share bounds.
            let group = match i {
                | _ if i >= group_ends.len() => &[][..],
                | 0 => &grouped[..group_ends[0]],
                | _ => &grouped[group_ends[i - 1]..group_ends[i]],
            };
            let tokenizer = TokenSource::replay(
                self.context,
                self.scratch,
                &[group, std::slice::from_ref(&closing)],
                location.clone(),
            );
            arguments.push(FunctionLikeMacroArgument {
                variadic: variadic && i == names.len(),
                substituted: false,
                expanded: self.scratch.alloc(OnceCell::new()),
                omitted: i >= group_ends.len()
                    || (variadic
                        && names.is_empty()
                        && empty
                        && self.context.configuration.gnu_extensions()),
                name,
                tokenizer,
                enclosing_arguments: None,
                disabled_macros,
            });
        }
        let invocation_end = match cursor.index.map(|index| &cursor.frames[index].frame_type) {
            | Some(
                TokenizerFrameType::ObjectLikeMacroInvocation { invocation_end, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation_end, .. },
            ) => invocation_end.clone(),
            | _ => location.clone(),
        };
        cursor.commit(self, location);
        Some((arguments.leak(), invocation_end))
    }

    /// Whether `name` is a macro being replaced where the current token is
    /// read, so that the name is not replaced again.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    pub(in crate::translation_phases::preprocessing) fn macro_is_disabled(
        &self,
        name: StringCacheId,
    ) -> bool {
        Self::macro_is_disabled_in(&self.tokenizer_stack, name)
    }

    /// Whether `name` is disabled in the frames owning a collected token.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    fn macro_is_disabled_in(frames: &[TokenizerFrame<'x>], name: StringCacheId) -> bool {
        for frame in frames.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    return argument.disabled_macros.contains(&name);
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name: active, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name: active, .. }
                    if *active == name =>
                    return true,
                | _ => (),
            }
        }
        false
    }

    /// The macros being replaced where an argument is collected; nested
    /// replacements within the argument do not replace them either.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    pub(in crate::translation_phases::preprocessing) fn disabled_macros(
        &self,
    ) -> &'x [StringCacheId] {
        self.disabled_macros_in(&self.tokenizer_stack)
    }

    /// The disabled set of the frames supplying an invocation's arguments.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    fn disabled_macros_in(&self, frames: &[TokenizerFrame<'x>]) -> &'x [StringCacheId] {
        if let Some(frame) = frames.last() {
            match &frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return &[],
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } =>
                    return argument.disabled_macros,
                | _ => (),
            }
        }
        let mut names = ArenaVec::new_in(self.scratch);
        for frame in frames.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    names.extend_from_slice(argument.disabled_macros);
                    break;
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name, .. } => names.push(*name),
                | _ => (),
            }
        }
        names.leak()
    }
}

/// Transactional logical cursor over replacement lists and their continuations.
/// Exhausted frames remain present until normal rescan unwinds them, preserving
/// disabled macro names. A failed lookahead changes no real source cursor.
///
/// C99: §6.10.3.4 paragraph 1, p. 155; PDF p. 167: a replacement is rescanned
/// along with the rest of the source, so an invocation's `(` and arguments
/// may follow the replacement list that ends with the macro's name.
struct MacroCallCursor<'x> {
    frames:       ArenaVec<'x, TokenizerFrame<'x>>,
    index:        Option<usize>,
    floor:        usize,
    /// Consumed prefix stays in place until the queue empties.
    pending:      ArenaVec<'x, PreprocessorToken>,
    pending_head: usize,
}

impl<'x> MacroCallCursor<'x> {
    fn next(&mut self, preprocessor: &mut Expander<'_, '_, '_, 'x>) -> Option<PreprocessorToken> {
        loop {
            if let Some(token) = self.pending.get(self.pending_head).copied() {
                self.pending_head += 1;
                return Some(token);
            }
            self.pending.clear();
            self.pending_head = 0;
            let index = self.index.filter(|index| *index >= self.floor)?;
            let frame = &mut self.frames[index];
            let position = frame.tokenizer.position(preprocessor.context);
            let token = frame.tokenizer.next_item(preprocessor.context);
            let ends = match (&mut frame.frame_type, token) {
                | (TokenizerFrameType::SourceFile { .. }, None) => return None,
                | (_, None) => true,
                | (
                    TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                    | TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                    Some(token),
                ) => token.kind == PreprocessorTokenType::Newline,
                | (
                    TokenizerFrameType::FunctionLikeMacroArgument {
                        argument,
                        paren_depth: Some(depth),
                        ..
                    },
                    Some(token),
                ) => {
                    match preprocessor.update_macro_argument_paren_depth(
                        token,
                        argument.variadic,
                        *depth,
                    ) {
                        | Some(next) => {
                            *depth = next;
                            false
                        },
                        | None => true,
                    }
                },
                | _ => false,
            };
            if ends {
                frame.tokenizer.set_position(position);
                self.index = index.checked_sub(1);
                continue;
            }
            let mut token = token?;
            let arguments = match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. } =>
                    Some(*arguments),
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } =>
                    argument.enclosing_arguments,
                | _ => None,
            };
            let replacement = matches!(
                frame.frame_type,
                TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                    | TokenizerFrameType::FunctionLikeMacroInvocation { .. }
            ) || arguments.is_some();
            if replacement && token.kind == PreprocessorTokenType::Hash {
                let position = frame.tokenizer.position(preprocessor.context);
                let operand =
                    Expander::next_ignore_whitespace(&mut frame.tokenizer, preprocessor.context);
                if let Some(operand) = operand
                    && let Some(argument) = arguments.and_then(|arguments| {
                        find_argument(arguments, operand.identifier_id(preprocessor.context))
                    })
                {
                    let raw = preprocessor.read_argument(argument, false);
                    token.contents =
                        Expander::stringify(preprocessor.context, preprocessor.scratch, &raw);
                    token.kind = PreprocessorTokenType::String;
                    token.source_vectors = preprocessor
                        .context
                        .merge_vectors(token.source_vectors, operand.source_vectors);
                } else {
                    frame.tokenizer.set_position(position);
                }
            }
            let mut tokens = ArenaVec::new_in(preprocessor.scratch);
            let mut pasted = false;
            if replacement {
                loop {
                    let position = frame.tokenizer.position(preprocessor.context);
                    let next = Expander::next_ignore_whitespace(
                        &mut frame.tokenizer,
                        preprocessor.context,
                    );
                    if !next.is_some_and(|token| token.kind == PreprocessorTokenType::HashHash) {
                        frame.tokenizer.set_position(position);
                        break;
                    }
                    let Some(rhs) = Expander::next_ignore_whitespace(
                        &mut frame.tokenizer,
                        preprocessor.context,
                    )
                    .filter(|token| token.kind != PreprocessorTokenType::Newline) else {
                        frame.tokenizer.set_position(position);
                        break;
                    };
                    if !pasted {
                        tokens = preprocessor.paste_operand(arguments, token);
                    }
                    let right = preprocessor.paste_operand(arguments, rhs);
                    preprocessor.paste_onto(&mut tokens, right);
                    pasted = true;
                }
            }
            if pasted {
                self.pending.extend(tokens);
                continue;
            }
            if token.kind.is_identifier()
                && let Some(argument) = arguments.and_then(|arguments| {
                    find_argument(arguments, token.identifier_id(preprocessor.context))
                })
            {
                let mut expanded = preprocessor.expanded_argument(token, argument);
                while let Some(token) = expanded.next_item(preprocessor.context) {
                    self.pending.push(token);
                }
                continue;
            }
            return Some(token);
        }
    }

    fn commit(
        mut self,
        preprocessor: &mut Expander<'_, '_, '_, 'x>,
        location: crate::translation_phases::SourceVector,
    ) {
        if self.pending_head < self.pending.len() {
            let tokenizer = TokenSource::replay(
                preprocessor.context,
                preprocessor.scratch,
                &[&self.pending[self.pending_head..]],
                location,
            );
            self.frames.insert(
                self.index.unwrap() + 1,
                TokenizerFrame {
                    frame_type: TokenizerFrameType::Rescan { argument: false },
                    tokenizer,
                },
            );
        }
        preprocessor.tokenizer = self.frames.last().unwrap().tokenizer.clone();
        preprocessor.tokenizer_stack.clear();
        preprocessor.tokenizer_stack.extend(self.frames);
    }

    fn new(preprocessor: &Expander<'_, '_, '_, 'x>) -> Self {
        let mut frames = ArenaVec::new_in(preprocessor.scratch);
        frames.extend(preprocessor.tokenizer_stack.iter().cloned());
        frames.last_mut().unwrap().tokenizer = preprocessor.tokenizer.clone();
        Self {
            index: frames.len().checked_sub(1),
            frames,
            floor: preprocessor
                .operand_fence
                .max(preprocessor.expansion_fence)
                .saturating_sub(1),
            pending: ArenaVec::new_in(preprocessor.scratch),
            pending_head: 0,
        }
    }

    /// The frames enclosing the token just collected, excluding replacement
    /// frames that lookahead has already exhausted.
    ///
    /// C99: §6.10.3.4 paragraphs 1-2, p. 155; PDF p. 167.
    fn active_frames(&self) -> &[TokenizerFrame<'x>] {
        &self.frames[..self.index.map_or(0, |index| index + 1)]
    }
}
