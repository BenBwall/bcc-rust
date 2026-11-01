//! Replace macro names, capture macro calls across frame boundaries, and
//! track disabled macro names.
//!
//! C99: translation phase 4; replacement rescanning, §6.10.3.4 paragraphs 1-2,
//! p. 155; PDF p. 167. Lookahead commits only when it recognizes a call;
//! parameter substitution belongs to the argument reader.

use std::cell::OnceCell;

use super::{
    FunctionLikeMacroArgument,
    MacroArguments,
    MacroDefinition,
    arguments::find_argument,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        GetPosition,
        SetPosition,
        SourceVectors,
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
            CopiedRun,
            PreprocessorToken,
            PreprocessorTokenType,
            SharedRun,
            SharedToken,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl<'x> Expander<'_, '_, '_, 'x> {
    /// Replaces `token`, a name that may be macro-replaced, when it names a
    /// macro: pushes the replacement of an object-like macro, or of a
    /// function-like one that the next tokens call, or reads a built-in.
    /// Returns the next token when that is the name itself or a built-in's
    /// result, or `None` to read on.
    ///
    /// Kept out of line so that the reader's own frame, which every argument
    /// prescan nests once more, does not hold the replacement frames built
    /// here.
    ///
    /// C99: §6.10.3 paragraphs 9-10, p. 152; PDF p. 164; §6.10.3.4
    /// paragraph 1, p. 155; PDF p. 167.
    #[inline(never)]
    pub(in crate::translation_phases::preprocessing) fn replace_macro_name(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        if let Some(md) = self
            .state
            .macro_definitions
            .get(&token.identifier_id(self.context))
            .cloned()
        {
            match md {
                | MacroDefinition::ObjectLike { tokenizer } => {
                    self.warn_deprecated_macro(token);
                    let tokenizer = self.without_leading_whitespace(tokenizer);
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
                            released:       false,
                        },
                        tokenizer,
                    };
                    if self.tracing() {
                        self.trace_invocation(&frame);
                    }
                    self.push_tokenizer_frame(frame);
                    return None;
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
                    let Some((arguments, invocation_end)) =
                        self.capture_cross_frame_call(token, argument_names, variadic_name)
                    else {
                        return Some(token);
                    };
                    self.warn_deprecated_macro(token);
                    let tokenizer = self.without_leading_whitespace(tokenizer);
                    let (tokenizer, arguments) = if is_variadic {
                        self.prepare_variadic_body(token, tokenizer, arguments)
                    } else {
                        (tokenizer, arguments)
                    };
                    let frame = TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                            invocation_end,
                            invocation: self.invocation_location(token),
                            spelling: self.spelling_location(token),
                            name: token.identifier_id(self.context),
                            arguments,
                            is_variadic,
                            released: false,
                        },
                        tokenizer,
                    };
                    if self.tracing() {
                        self.trace_invocation(&frame);
                    }
                    self.push_tokenizer_frame(frame);
                    return None;
                },
                | MacroDefinition::BuiltIn => {
                    let result = self.expand_builtin(token);
                    if self.tracing() {
                        self.trace_builtin(token, result);
                    }
                    if let Some(result) = result {
                        return Some(result);
                    }
                    return None;
                },
            }
        }
        Some(token)
    }

    /// A replacement list read from its first token: the white space
    /// before it, after the macro's name or parameters, is not part of the
    /// list. A token the replacement begins with is then separated from the
    /// one before the invocation only as the invocation was, which `#`
    /// spells: with `#define X 1`, `-X` stringizes as `"-1"`.
    ///
    /// C99: §6.10.3 paragraph 7, p. 151; PDF p. 163; §6.10.3.2 paragraph 2,
    /// p. 153; PDF p. 165.
    fn without_leading_whitespace<'t>(
        &mut self,
        mut tokenizer: TokenSource<'t>,
    ) -> TokenSource<'t> {
        let blanks = tokenizer
            .spellings()
            .take_while(|&(kind, _)| kind == PreprocessorTokenType::Whitespace)
            .count();
        for _ in 0..blanks {
            _ = tokenizer.next_item(self.context);
        }
        tokenizer
    }

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
            if !is_blank(token.kind()) {
                break token;
            }
        };
        if opening.kind() != PreprocessorTokenType::OpeningParenthesis {
            return None;
        }
        // Argument tokens in order, as runs shared with the replays they
        // were read from or copied from the tokens read with provenance,
        // and where each argument's runs end.
        let mut pieces = ArenaVec::new_in(self.scratch);
        let mut copying = CopiedRun::new(self.scratch);
        let mut group_ends = ArenaVec::new_in(self.scratch);
        let mut group_sites = ArenaVec::new_in(self.scratch);
        let mut depth = 1usize;
        // The `(` of each group open within the call, when it is shared.
        let mut opens = ArenaVec::new_in(self.scratch);
        let closing = loop {
            let Some(token) = cursor.next_argument(self) else {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing function-like macro invocation",
                    ),
                    source_vectors: invocation.source_vectors,
                });
                return None;
            };
            let mut open = None;
            match token.kind() {
                | PreprocessorTokenType::OpeningParenthesis => depth += 1,
                | PreprocessorTokenType::ClosingParenthesis => {
                    depth -= 1;
                    if depth == 0 {
                        break token;
                    }
                    open = opens.pop().flatten();
                },
                | PreprocessorTokenType::Comma
                    if depth == 1 && (!variadic || group_ends.len() < names.len()) =>
                {
                    finish_copy(&mut copying, &mut pieces, self.scratch);
                    group_ends.push(pieces.len());
                    group_sites.push(token.read(self.context).source_vectors);
                    continue;
                },
                | _ => (),
            }
            // C99 §6.10.3.4p2: retain the paint of tokens taken from a
            // replacement, even when the call closes in the rest of source.
            let painted = matches!(
                token.kind(),
                PreprocessorTokenType::Identifier | PreprocessorTokenType::UniversalIdentifier
            ) && Self::macro_is_disabled_in(
                cursor.active_frames(&self.tokenizer_stack),
                token.identifier_id(self.context),
            );
            let mut token = match token {
                | CallToken::Shared(shared) if !painted => {
                    finish_copy(&mut copying, &mut pieces, self.scratch);
                    self.collect_shared(
                        &mut cursor,
                        &mut pieces,
                        &group_ends,
                        &mut opens,
                        shared,
                        open,
                    );
                    continue;
                },
                | token => token.read(self.context),
            };
            if token.kind == PreprocessorTokenType::OpeningParenthesis {
                opens.push(None);
            }
            if painted {
                token.kind = if token.kind == PreprocessorTokenType::UniversalIdentifier {
                    PreprocessorTokenType::UnavailableUniversalIdentifier
                } else {
                    PreprocessorTokenType::UnavailableIdentifier
                };
            }
            copying.push(self.context, &token);
        };
        // A normal argument cursor is terminated by a closing parenthesis.
        // Keep that sentinel so all existing raw #/## readers share bounds.
        // Read with its provenance, it is copied after the last argument's
        // tokens, so a call copies its tokens in one piece.
        let parameters = names.len() + usize::from(variadic);
        let (closing, closing_run) = match closing {
            | CallToken::Shared(shared) => {
                finish_copy(&mut copying, &mut pieces, self.scratch);
                (shared.read(self.context), SharedRun::new(shared))
            },
            | CallToken::Read(token) if parameters != 0 => {
                copying.push(self.context, &token);
                let (last, closing_run) = copying.finish().split_last();
                if !last.is_empty() {
                    pieces.push(last);
                }
                (token, closing_run)
            },
            | CallToken::Read(token) => {
                finish_copy(&mut copying, &mut pieces, self.scratch);
                (token, SharedRun::default())
            },
        };
        group_ends.push(pieces.len());
        let empty = group_ends.len() == 1 && pieces.iter().all(|run| run.kinds().all(is_blank));
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
            for (i, &end) in group_ends.iter().enumerate() {
                if pieces[start..end]
                    .iter()
                    .all(|run| run.kinds().all(is_blank))
                {
                    let site = group_sites.get(i).copied();
                    self.report_empty_macro_argument(site.unwrap_or(closing.source_vectors));
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
        let disabled_macros = self.disabled_macros_in(cursor.active_frames(&self.tokenizer_stack));
        let location = self
            .context
            .first_source_vector_or_default(closing.source_vectors);
        let mut arguments = ArenaVec::with_capacity_in(parameters, self.scratch);
        for (i, name) in names.iter().copied().chain(variadic_name).enumerate() {
            let group = match i {
                | _ if i >= group_ends.len() => &[][..],
                | 0 => &pieces[..group_ends[0]],
                | _ => &pieces[group_ends[i - 1]..group_ends[i]],
            };
            let tokenizer =
                TokenSource::replay_runs(self.scratch, group, closing_run, location.clone());
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
        let invocation_end = match cursor
            .index
            .map(|index| &cursor.frame(&self.tokenizer_stack, index).frame_type)
        {
            | Some(
                TokenizerFrameType::ObjectLikeMacroInvocation { invocation_end, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation_end, .. },
            ) => invocation_end.clone(),
            | _ => location.clone(),
        };
        cursor.commit(self, location);
        Some((arguments.leak(), invocation_end))
    }

    /// Adds `token`, shared from a replay, to the argument being collected:
    /// to the last run when it follows that run's last token, unless an
    /// argument ended between them. A `(` whose group an earlier call saw
    /// closed in the same replay skips the group, which is added whole; a
    /// `)` that closes `open` within one run records that group.
    ///
    /// Out of line, as only calls collected from a replay share tokens.
    ///
    /// C99: §6.10.3 paragraph 11, p. 152; PDF p. 164: within inner
    /// parentheses, nothing separates or ends arguments.
    #[inline(never)]
    fn collect_shared(
        &mut self,
        cursor: &mut MacroCallCursor<'x>,
        pieces: &mut ArenaVec<'x, SharedRun<'x>>,
        group_ends: &[usize],
        opens: &mut ArenaVec<'x, Option<SharedToken<'x>>>,
        token: SharedToken<'x>,
        open: Option<SharedToken<'x>>,
    ) {
        let group_start = group_ends.last().copied().unwrap_or(0);
        let extended =
            pieces.len() > group_start && pieces.last_mut().is_some_and(|run| run.extend(token));
        if !extended {
            pieces.push(SharedRun::new(token));
        }
        let Some(run) = pieces.last_mut() else {
            unreachable!("the token was just added to a run");
        };
        match token.kind() {
            | PreprocessorTokenType::OpeningParenthesis => {
                opens.push(Some(token));
                if let Some(&offset) = self.paren_ends.get(&token.address())
                    && let Some(group) = cursor.skip_group(&self.tokenizer_stack, token, offset)
                {
                    let appended = run.append(group);
                    debug_assert!(appended, "a skipped group follows its `(`");
                }
            },
            | PreprocessorTokenType::ClosingParenthesis => {
                if let Some(open) = open
                    && let Some(offset) = run.distance(open, token)
                {
                    _ = self.paren_ends.insert(open.address(), offset);
                }
            },
            | _ => (),
        }
    }

    /// Whether `name` is a macro being replaced where the current token is
    /// read, so that the name is not replaced again.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    pub(in crate::translation_phases::preprocessing) fn macro_is_disabled(
        &self,
        name: StringCacheId,
    ) -> bool {
        Self::macro_is_disabled_in(Frames::of(&self.tokenizer_stack), name)
    }

    /// Whether `name`, which `##` made, is a macro being replaced where the
    /// paste happened. The paste belongs to the replacement list that holds
    /// the `##`, not to the argument its operand came from, so the
    /// arguments being read above that list do not decide it: with
    /// `#define glue(a, b) a ## b`, the `glue` of `glue(gl, ue)` stays
    /// unreplaced, even before a `(`, as in GCC and Clang.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166; §6.10.3.4
    /// paragraph 2, p. 155; PDF p. 167.
    pub(in crate::translation_phases::preprocessing) fn pasted_macro_is_disabled(
        &self,
        name: StringCacheId,
    ) -> bool {
        let stack = &self.tokenizer_stack[..];
        let list = stack
            .iter()
            .rposition(|frame| {
                !matches!(
                    frame.frame_type,
                    TokenizerFrameType::FunctionLikeMacroArgument { .. }
                )
            })
            .map_or(0, |index| index + 1);
        Self::macro_is_disabled_in(Frames::of(&stack[..list]), name)
    }

    /// Whether `name` is disabled in the frames owning a collected token.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    fn macro_is_disabled_in(frames: Frames<'_, 'x>, name: StringCacheId) -> bool {
        for frame in frames.iter_rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    released: false,
                    operand,
                    ..
                } => {
                    if argument.disabled_macros.contains(&name) {
                        return true;
                    }
                    if !operand {
                        return false;
                    }
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation {
                    name: active,
                    released: false,
                    ..
                }
                | TokenizerFrameType::FunctionLikeMacroInvocation {
                    name: active,
                    released: false,
                    ..
                } if *active == name => return true,
                | _ => (),
            }
        }
        false
    }

    /// The disabled set of the frames supplying an invocation's arguments.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    fn disabled_macros_in(&self, frames: Frames<'_, 'x>) -> &'x [StringCacheId] {
        if let Some(frame) = frames.iter_rev().next() {
            match &frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return &[],
                | TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    released: false,
                    operand: false,
                    ..
                } => return argument.disabled_macros,
                | _ => (),
            }
        }
        let mut names = ArenaVec::new_in(self.scratch);
        for frame in frames.iter_rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    released: false,
                    operand,
                    ..
                } => {
                    names.extend_from_slice(argument.disabled_macros);
                    if !operand {
                        break;
                    }
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation {
                    name,
                    released: false,
                    ..
                }
                | TokenizerFrameType::FunctionLikeMacroInvocation {
                    name,
                    released: false,
                    ..
                } => names.push(*name),
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
/// The cursor reads no frame below `floor`, so it copies only the frames from
/// there up; the expander's frames below stay where they are. Prescanning an
/// argument fences reading at the argument's frame, so a call nested in an
/// argument copies one frame, however deep the nesting.
///
/// C99: §6.10.3.4 paragraph 1, p. 155; PDF p. 167: a replacement is rescanned
/// along with the rest of the source, so an invocation's `(` and arguments
/// may follow the replacement list that ends with the macro's name.
struct MacroCallCursor<'x> {
    /// Copies of the expander's frames from `base` up.
    frames:       ArenaVec<'x, TokenizerFrame<'x>>,
    base:         usize,
    /// The frame being read, as an index into the whole stack.
    index:        Option<usize>,
    floor:        usize,
    /// Consumed prefix stays in place until the queue empties.
    pending:      ArenaVec<'x, PreprocessorToken>,
    pending_head: usize,
    line_start:   bool,
}

/// A token of a macro call being collected: read with its provenance, or,
/// from a replay that substitutes nothing, still in that replay, so that the
/// call's arguments can share it.
#[derive(Clone, Copy)]
enum CallToken<'x> {
    Read(PreprocessorToken),
    Shared(SharedToken<'x>),
}

/// The frames of the stack as a macro call sees them: the expander's frames
/// below a cursor's copies, then the copies, innermost last.
#[derive(Clone, Copy)]
struct Frames<'s, 'x> {
    lower: &'s [TokenizerFrame<'x>],
    upper: &'s [TokenizerFrame<'x>],
}

impl<'x> MacroCallCursor<'x> {
    /// Execute written directives while collecting an invocation's arguments,
    /// as GCC and Clang do. Generated hashes remain ordinary argument tokens.
    /// C99: §6.10.3p11, p. 152; PDF p. 164 makes this undefined behavior;
    /// accepting it is a GNU extension.
    fn next_argument(
        &mut self,
        preprocessor: &mut Expander<'_, '_, '_, 'x>,
    ) -> Option<CallToken<'x>> {
        loop {
            let token = self.next(preprocessor)?;
            let source = self.index.is_some_and(|index| {
                matches!(
                    self.frame(&preprocessor.tokenizer_stack, index).frame_type,
                    TokenizerFrameType::SourceFile { .. }
                )
            });
            if source && self.line_start && token.kind() == PreprocessorTokenType::Hash {
                let hash = token.read(preprocessor.context);
                self.run_directive(preprocessor, hash);
                continue;
            }
            if source {
                match token.kind() {
                    | PreprocessorTokenType::Newline => self.line_start = true,
                    | PreprocessorTokenType::Whitespace => {},
                    | _ => self.line_start = false,
                }
            }
            return Some(token);
        }
    }

    /// Executes the directive that `hash` begins, in the source frame being
    /// read, and goes on reading after it. Out of line: it is rare, and the
    /// frame juggling would otherwise sit in every call's collection.
    #[cold]
    #[inline(never)]
    fn run_directive(
        &mut self,
        preprocessor: &mut Expander<'_, '_, '_, 'x>,
        hash: PreprocessorToken,
    ) {
        preprocessor.context.report_extension_since(
            "directive inside macro arguments",
            crate::configuration::FeatureOrigin::Gnu,
            hash.source_vectors,
        );
        let index = self.index.expect("source frame exists") - self.base;
        let saved_tokenizer = std::mem::replace(
            &mut preprocessor.tokenizer,
            self.frames[index].tokenizer.clone(),
        );
        let saved_frames = std::mem::replace(
            &mut preprocessor.tokenizer_stack,
            ArenaVec::new_in(preprocessor.scratch),
        );
        preprocessor
            .tokenizer_stack
            .extend(saved_frames[..self.base].iter().cloned());
        preprocessor
            .tokenizer_stack
            .extend(self.frames[..=index].iter().cloned());
        let saved_lines = (
            preprocessor.last_was_newline,
            preprocessor.current_is_newline,
        );
        preprocessor.resume_at_line_start();
        preprocessor.parse_directive(hash);
        // The directive may have changed any frame, so the cursor
        // now copies the whole stack.
        self.base = 0;
        self.frames.clear();
        self.frames
            .extend(preprocessor.tokenizer_stack.iter().cloned());
        if let Some(last) = self.frames.last_mut() {
            last.tokenizer = preprocessor.tokenizer.clone();
        }
        self.index = self.frames.len().checked_sub(1);
        preprocessor.tokenizer = saved_tokenizer;
        preprocessor.tokenizer_stack = saved_frames;
        (
            preprocessor.last_was_newline,
            preprocessor.current_is_newline,
        ) = saved_lines;
        self.line_start = true;
    }

    fn next(&mut self, preprocessor: &mut Expander<'_, '_, '_, 'x>) -> Option<CallToken<'x>> {
        loop {
            if let Some(token) = self.pending.get(self.pending_head).copied() {
                self.pending_head += 1;
                return Some(CallToken::Read(token));
            }
            self.pending.clear();
            self.pending_head = 0;
            let index = self.index.filter(|index| *index >= self.floor)?;
            let frame = &mut self.frames[index - self.base];
            let position = frame.tokenizer.position(preprocessor.context);
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
            // Replayed tokens that substitute nothing are shared, not read:
            // reading them would record their provenance once per call
            // that a nested argument is collected for.
            let shared = !replacement
                && !matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. })
                && frame.tokenizer.is_replay();
            let token = if shared {
                frame.tokenizer.next_shared().map(CallToken::Shared)
            } else {
                frame
                    .tokenizer
                    .next_item(preprocessor.context)
                    .map(CallToken::Read)
            };
            let ends = match (&mut frame.frame_type, token) {
                | (TokenizerFrameType::SourceFile { .. }, None) => return None,
                | (_, None) => true,
                | (
                    TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                    | TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                    Some(token),
                ) => token.kind() == PreprocessorTokenType::Newline,
                | (
                    TokenizerFrameType::FunctionLikeMacroArgument {
                        argument,
                        paren_depth: Some(depth),
                        ..
                    },
                    Some(token),
                ) => {
                    match preprocessor.update_macro_argument_paren_depth(
                        token.kind(),
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
            let token = token?;
            let CallToken::Read(mut token) = token else {
                return Some(token);
            };
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
            return Some(CallToken::Read(token));
        }
    }

    /// Moves the real frame stack to the cursor. Replacement frames that the
    /// call read to their end are released, as GCC and Clang pop them: their
    /// macros are no longer being replaced when the call's own replacement
    /// is rescanned (the frames stay for expansion notes). So `f(2)(9)` for
    /// `#define f(a) a*g` and `#define g(a) f(a)` is `2*9*g` (C11
    /// §6.10.3.4p4 leaves the choice open). Microsoft's
    /// `TraceLoggingProvider.h` argument counter needs this. Tokens read
    /// from those frames keep their paint. C99: §6.10.3.4p2, p. 155; PDF p.
    /// 167. C11: §6.10.3.4p4, p. 170; PDF p. 188.
    fn commit(
        mut self,
        preprocessor: &mut Expander<'_, '_, '_, 'x>,
        location: crate::translation_phases::SourceVector,
    ) {
        let read = self.index.map_or(0, |index| index + 1);
        for frame in &mut self.frames[read - self.base..] {
            if let TokenizerFrameType::ObjectLikeMacroInvocation { released, .. }
            | TokenizerFrameType::FunctionLikeMacroInvocation { released, .. }
            | TokenizerFrameType::FunctionLikeMacroArgument { released, .. } =
                &mut frame.frame_type
            {
                *released = true;
            }
        }
        if self.pending_head < self.pending.len() {
            let tokenizer = TokenSource::replay(
                preprocessor.context,
                preprocessor.scratch,
                &[&self.pending[self.pending_head..]],
                location,
            );
            self.frames.insert(
                self.index.map_or(0, |index| index + 1) - self.base,
                TokenizerFrame {
                    frame_type: TokenizerFrameType::Rescan { argument: false },
                    tokenizer,
                },
            );
        }
        preprocessor.tokenizer = self.frames.last().unwrap().tokenizer.clone();
        preprocessor.tokenizer_stack.truncate(self.base);
        preprocessor.tokenizer_stack.extend(self.frames);
    }

    fn new(preprocessor: &Expander<'_, '_, '_, 'x>) -> Self {
        let stack = &preprocessor.tokenizer_stack;
        let floor = preprocessor
            .operand_fence
            .max(preprocessor.expansion_fence)
            .saturating_sub(1);
        let base = floor.min(stack.len() - 1);
        let mut frames = ArenaVec::new_in(preprocessor.scratch);
        frames.extend(stack[base..].iter().cloned());
        frames.last_mut().unwrap().tokenizer = preprocessor.tokenizer.clone();
        Self {
            frames,
            base,
            index: stack.len().checked_sub(1),
            floor,
            pending: ArenaVec::new_in(preprocessor.scratch),
            pending_head: 0,
            line_start: false,
        }
    }

    /// Moves past the tokens of the group that `open`, just shared, opens,
    /// whose `)` an earlier call saw `offset` tokens later in the same
    /// replay, and returns them; that `)` is read next. Reading them would
    /// give these tokens only if no name is painted where they are read,
    /// so the group is read when one could be. The expander holds `stack`
    /// below the cursor's copies.
    ///
    /// C99: §6.10.3 paragraph 11, p. 152; PDF p. 164: within inner
    /// parentheses, nothing separates or ends arguments.
    fn skip_group(
        &mut self,
        stack: &[TokenizerFrame<'x>],
        open: SharedToken<'x>,
        offset: usize,
    ) -> Option<SharedRun<'x>> {
        if !disables_nothing(self.active_frames(stack)) {
            return None;
        }
        let index = self.index?;
        self.frames[index - self.base]
            .tokenizer
            .skip_shared(open, offset)
    }

    /// The frame at `index` of the whole stack, of which the expander holds
    /// `stack` below the cursor's copies.
    fn frame<'s>(
        &'s self,
        stack: &'s [TokenizerFrame<'x>],
        index: usize,
    ) -> &'s TokenizerFrame<'x> {
        match index.checked_sub(self.base) {
            | Some(copied) => &self.frames[copied],
            | None => &stack[index],
        }
    }

    /// The frames enclosing the token just collected, excluding replacement
    /// frames that lookahead has already exhausted. The expander holds
    /// `stack` below the cursor's copies.
    ///
    /// C99: §6.10.3.4 paragraphs 1-2, p. 155; PDF p. 167.
    fn active_frames<'s>(&'s self, stack: &'s [TokenizerFrame<'x>]) -> Frames<'s, 'x> {
        let active = self.index.map_or(0, |index| index + 1);
        Frames {
            lower: &stack[..active.min(self.base)],
            upper: &self.frames[..active.saturating_sub(self.base)],
        }
    }
}

impl CallToken<'_> {
    fn kind(self) -> PreprocessorTokenType {
        match self {
            | Self::Read(token) => token.kind,
            | Self::Shared(token) => token.kind(),
        }
    }

    /// The name of an identifier, as [`PreprocessorToken::identifier_id`].
    fn identifier_id(self, context: &Context<'_>) -> StringCacheId {
        match self {
            | Self::Read(token) => token.identifier_id(context),
            | Self::Shared(token) => PreprocessorToken {
                kind:           token.kind(),
                contents:       token.contents(),
                source_vectors: SourceVectors::default(),
            }
            .identifier_id(context),
        }
    }

    /// The token with its provenance, recording a shared token's.
    fn read(self, context: &mut Context<'_>) -> PreprocessorToken {
        match self {
            | Self::Read(token) => token,
            | Self::Shared(token) => token.read(context),
        }
    }
}

impl<'s, 'x> Frames<'s, 'x> {
    /// All of `stack`.
    fn of(stack: &'s [TokenizerFrame<'x>]) -> Self {
        Self {
            lower: stack,
            upper: &[],
        }
    }

    /// The frames from the innermost out.
    fn iter_rev(self) -> impl Iterator<Item = &'s TokenizerFrame<'x>> {
        self.upper.iter().rev().chain(self.lower.iter().rev())
    }
}

/// Whether no name is disabled in `frames`, so that no token read there is
/// painted, as [`Expander::macro_is_disabled`] decides.
///
/// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
fn disables_nothing(frames: Frames<'_, '_>) -> bool {
    for frame in frames.iter_rev() {
        match &frame.frame_type {
            | TokenizerFrameType::FunctionLikeMacroArgument {
                argument,
                released: false,
                ..
            } => return argument.disabled_macros.is_empty(),
            | TokenizerFrameType::ObjectLikeMacroInvocation {
                released: false, ..
            }
            | TokenizerFrameType::FunctionLikeMacroInvocation {
                released: false, ..
            } => return false,
            | _ => (),
        }
    }
    true
}

/// Ends the run that `copying` holds, if any, as the next of `pieces`.
fn finish_copy<'x>(
    copying: &mut CopiedRun<'x>,
    pieces: &mut ArenaVec<'x, SharedRun<'x>>,
    arena: &'x Bump,
) {
    if !copying.is_empty() {
        pieces.push(std::mem::replace(copying, CopiedRun::new(arena)).finish());
    }
}

/// Whether `kind` is white space, which separates tokens but is no
/// argument's content.
fn is_blank(kind: PreprocessorTokenType) -> bool {
    matches!(
        kind,
        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
    )
}
