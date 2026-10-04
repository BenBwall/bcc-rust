//! Macro definitions, argument collection, and `#`/`##` operators.

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
    rc::Rc,
};

use super::{
    Preprocessor,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
        string_literal_spelling,
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
        SetPosition,
        SourceVectors,
        TokenString,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        HashMap,
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition<'pp> {
    ObjectLike {
        tokenizer: TokenSource,
    },
    FunctionLike {
        argument_names: &'pp [StringCacheId],
        tokenizer:      TokenSource,
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
    pub(super) tokenizer:           TokenSource,
    pub(super) enclosing_arguments: Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>>,
    pub(super) disabled_macros:     Rc<[StringCacheId]>,
    /// Shared once-only argument prescan; raw #/## operands bypass it.
    pub(super) expanded:            Rc<OnceCell<TokenSource>>,
}

/// Transactional logical cursor over replacement lists and their continuations.
/// Exhausted frames remain present until normal rescan unwinds them, preserving
/// disabled macro names. A failed lookahead changes no real source cursor.
struct MacroCallCursor<'pp> {
    frames:       ArenaVec<'pp, TokenizerFrame>,
    index:        Option<usize>,
    floor:        usize,
    /// Consumed prefix stays in place until the queue empties.
    pending:      ArenaVec<'pp, PreprocessorToken>,
    pending_head: usize,
}

impl<'pp> MacroCallCursor<'pp> {
    fn new(preprocessor: &Preprocessor<'_, 'pp>) -> Self {
        let mut frames = ArenaVec::new_in(preprocessor.state.arena);
        frames.extend(preprocessor.state.tokenizer_stack.iter().cloned());
        frames.last_mut().unwrap().tokenizer = preprocessor.tokenizer.clone();
        Self {
            index: frames.len().checked_sub(1),
            frames,
            floor: preprocessor
                .operand_fence
                .max(preprocessor.expansion_fence)
                .saturating_sub(1),
            pending: ArenaVec::new_in(preprocessor.state.arena),
            pending_head: 0,
        }
    }

    fn next(
        &mut self,
        preprocessor: &mut Preprocessor<'_, '_>,
        context: &mut Context<'_>,
    ) -> Option<PreprocessorToken> {
        loop {
            if let Some(token) = self.pending.get(self.pending_head).copied() {
                self.pending_head += 1;
                return Some(token);
            }
            self.pending.clear();
            self.pending_head = 0;
            let index = self.index.filter(|index| *index >= self.floor)?;
            let frame = &mut self.frames[index];
            let position = frame.tokenizer.position(context);
            let token = frame.tokenizer.next_item(context);
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
                        context,
                        token,
                        argument.name,
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
                frame.tokenizer.set_position(context, position);
                self.index = index.checked_sub(1);
                continue;
            }
            let mut token = token?;
            let arguments = match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. } =>
                    Some(arguments.clone()),
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } =>
                    argument.enclosing_arguments.clone(),
                | _ => None,
            };
            let replacement = matches!(
                frame.frame_type,
                TokenizerFrameType::ObjectLikeMacroInvocation { .. }
                    | TokenizerFrameType::FunctionLikeMacroInvocation { .. }
            ) || arguments.is_some();
            if replacement && token.kind == PreprocessorTokenType::Hash {
                let position = frame.tokenizer.position(context);
                let operand = Preprocessor::next_ignore_whitespace(&mut frame.tokenizer, context);
                if let Some(operand) = operand
                    && let Some(argument) = arguments
                        .as_ref()
                        .and_then(|arguments| arguments.get(&operand.identifier_id(context)))
                {
                    let raw = preprocessor.read_argument(context, argument, false);
                    token.contents = Preprocessor::stringify(context, &raw);
                    token.kind = PreprocessorTokenType::String;
                    token.source_vectors =
                        context.merge_vectors(token.source_vectors, operand.source_vectors);
                } else {
                    frame.tokenizer.set_position(context, position);
                }
            }
            let mut tokens = Vec::new();
            let mut pasted = false;
            if replacement {
                loop {
                    let position = frame.tokenizer.position(context);
                    let next = Preprocessor::next_ignore_whitespace(&mut frame.tokenizer, context);
                    if !next.is_some_and(|token| token.kind == PreprocessorTokenType::HashHash) {
                        frame.tokenizer.set_position(context, position);
                        break;
                    }
                    let Some(rhs) =
                        Preprocessor::next_ignore_whitespace(&mut frame.tokenizer, context)
                            .filter(|token| token.kind != PreprocessorTokenType::Newline)
                    else {
                        frame.tokenizer.set_position(context, position);
                        break;
                    };
                    if !pasted {
                        tokens = if let Some(argument) = arguments
                            .as_ref()
                            .and_then(|arguments| arguments.get(&token.identifier_id(context)))
                        {
                            preprocessor.read_argument(context, argument, false)
                        } else {
                            vec![token]
                        };
                    }
                    let mut right = if let Some(argument) = arguments
                        .as_ref()
                        .and_then(|arguments| arguments.get(&rhs.identifier_id(context)))
                    {
                        preprocessor.read_argument(context, argument, false)
                    } else {
                        vec![rhs]
                    };
                    if !tokens.is_empty() && !right.is_empty() {
                        let lhs = tokens.pop().unwrap();
                        let rhs = right.remove(0);
                        if let Some(merged) = preprocessor.merge_tokens(context, lhs, rhs) {
                            tokens.push(merged);
                        }
                    }
                    tokens.extend(right);
                    pasted = true;
                }
            }
            if pasted {
                self.pending.extend(tokens);
                continue;
            }
            if token.kind.is_identifier()
                && let Some(argument) = arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get(&token.identifier_id(context)))
                    .cloned()
            {
                let mut expanded = preprocessor.expanded_argument(context, token, &argument);
                while let Some(token) = expanded.next_item(context) {
                    self.pending.push(token);
                }
                continue;
            }
            return Some(token);
        }
    }

    fn commit(
        mut self,
        preprocessor: &mut Preprocessor<'_, '_>,
        context: &Context<'_>,
        location: crate::translation_phases::SourceVector,
    ) {
        if self.pending_head < self.pending.len() {
            let tokenizer =
                TokenSource::replay(context, &self.pending[self.pending_head..], location);
            self.frames.insert(
                self.index.unwrap() + 1,
                TokenizerFrame {
                    frame_type: TokenizerFrameType::Rescan,
                    tokenizer,
                },
            );
        }
        preprocessor.tokenizer = self.frames.last().unwrap().tokenizer.clone();
        preprocessor.state.tokenizer_stack.clear();
        preprocessor.state.tokenizer_stack.extend(self.frames);
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl Preprocessor<'_, '_> {
    /// Capture an invocation whose opening, arguments, or closing delimiter
    /// can come from different replacement/argument/source frames.
    pub(super) fn capture_cross_frame_call(
        &mut self,
        context: &mut Context<'_>,
        invocation: PreprocessorToken,
        names: &[StringCacheId],
        variadic: bool,
    ) -> Option<(
        HashMap<StringCacheId, FunctionLikeMacroArgument>,
        crate::translation_phases::SourceVector,
    )> {
        let mut cursor = MacroCallCursor::new(self);
        let opening = loop {
            let token = cursor.next(self, context)?;
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
        let mut groups = ArenaVec::new_in(self.state.arena);
        groups.push(ArenaVec::new_in(self.state.arena));
        let mut depth = 1usize;
        let closing = loop {
            let Some(token) = cursor.next(self, context) else {
                context.preprocessor_error(PreprocessorError {
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
                    if depth == 1 && (!variadic || groups.len() <= names.len()) =>
                {
                    groups.push(ArenaVec::new_in(self.state.arena));
                    continue;
                },
                | _ => (),
            }
            groups.last_mut().unwrap().push(token);
        };
        let empty = groups.len() == 1
            && groups[0].iter().all(|token| {
                matches!(
                    token.kind,
                    PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                )
            });
        let count = if names.is_empty() && !variadic && empty {
            0
        } else {
            groups.len()
        };
        if (!variadic && count != names.len()) || (variadic && count < names.len()) {
            context.preprocessor_error(PreprocessorError {
                error_type:
                    PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                        expected: names.len(),
                        found:    count,
                    },
                source_vectors: invocation.source_vectors,
            });
        } else if variadic && count == names.len() {
            let policy = context.configuration.extension_policy();
            if policy != crate::configuration::ExtensionPolicy::Allow {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingVariadicArgument(policy),
                    source_vectors: invocation.source_vectors,
                });
            }
        }
        let disabled_macros = self.disabled_macros();
        let location = context
            .get_source_vectors(closing.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default();
        let mut arguments = HashMap::default();
        for (i, name) in names
            .iter()
            .copied()
            .chain(variadic.then(|| context.string_cache.intern("__VA_ARGS__")))
            .enumerate()
        {
            // A normal argument cursor is terminated by a closing parenthesis.
            // Keep that sentinel so all existing raw #/## readers share bounds.
            let tokenizer = if let Some(tokens) = groups.get_mut(i) {
                tokens.push(closing);
                TokenSource::replay(context, tokens, location.clone())
            } else {
                TokenSource::replay(context, &[closing], location.clone())
            };
            drop(arguments.insert(
                name,
                FunctionLikeMacroArgument {
                    expanded: Rc::default(),
                    name,
                    tokenizer,
                    enclosing_arguments: None,
                    disabled_macros: disabled_macros.clone(),
                },
            ));
        }
        let invocation_end = match cursor.index.map(|index| &cursor.frames[index].frame_type) {
            | Some(
                TokenizerFrameType::ObjectLikeMacroInvocation { invocation_end, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { invocation_end, .. },
            ) => invocation_end.clone(),
            | _ => location.clone(),
        };
        cursor.commit(self, context, location);
        Some((arguments, invocation_end))
    }

    pub(super) fn get_arguments(
        &self,
        _context: &Context<'_>,
    ) -> Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>> {
        match self
            .state
            .tokenizer_stack
            .last()
            .map(|frame| &frame.frame_type)
        {
            // Argument tokens belong to the invocation's caller. Looking them
            // up in the callee's map can make a same-named parameter expand itself.
            | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                argument.enclosing_arguments.clone(),
            | Some(TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. }) =>
                Some(arguments.clone()),
            | _ => None,
        }
    }

    pub(super) fn macro_argument_is_at_end(&mut self, context: &mut Context<'_>) -> bool {
        let Some(TokenizerFrame {
            frame_type:
                TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    paren_depth,
                    ..
                },
            ..
        }) = self.state.tokenizer_stack.last()
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
                    break depth.is_some_and(|depth| {
                        self.update_macro_argument_paren_depth(context, token, name, depth)
                            .is_none()
                    }),
                | None => break true,
            }
        };
        self.set_position(context, position);
        next_is_end
    }

    pub(super) fn macro_is_disabled(&self, name: StringCacheId) -> bool {
        for frame in self.state.tokenizer_stack.iter().rev() {
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

    pub(super) fn disabled_macros(&self) -> Rc<[StringCacheId]> {
        if let Some(frame) = self.state.tokenizer_stack.last() {
            match &frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } =>
                    return self.empty_disabled_macros.clone(),
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } =>
                    return argument.disabled_macros.clone(),
                | _ => (),
            }
        }
        let mut names = Vec::new();
        for frame in self.state.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    names.extend_from_slice(&argument.disabled_macros);
                    break;
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name, .. }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name, .. } => names.push(*name),
                | _ => (),
            }
        }
        Rc::from(names)
    }

    pub(super) fn handle_macro_argument(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        let arguments = self.get_arguments(context)?;
        let argument = arguments.get(&token.identifier_id(context))?.clone();
        // Prescan is isolated from the replacement list. Rescanning the result
        // then uses the callee's disabled-name set, not the caller's.
        Some(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan,
            tokenizer:  self.expanded_argument(context, token, &argument),
        })
    }

    /// Share prescan results with speculative invocation lookahead too:
    /// repeating replacement would also repeat its diagnostics and
    /// `_Pragma` effects.
    fn expanded_argument(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
        argument: &FunctionLikeMacroArgument,
    ) -> TokenSource {
        if let Some(tokenizer) = argument.expanded.get() {
            return tokenizer.clone();
        }
        let tokens = self.read_argument(context, argument, true);
        let location = context
            .get_source_vectors(token.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default();
        let tokenizer = TokenSource::replay(context, &tokens, location);
        drop(argument.expanded.set(tokenizer.clone()));
        tokenizer
    }

    fn raw_macro_argument_frame(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        if let Some(arguments) = self.get_arguments(context)
            && let Some(arg) = arguments.get(&token.identifier_id(context))
        {
            let frame = TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                    argument:            Box::new(arg.clone()),
                    paren_depth:         Some(1),
                    has_generated_token: false,
                },
                tokenizer:  arg.tokenizer.clone(),
            };
            return Some(frame);
        }

        None
    }

    /// Whether the token just read belongs to the `#` or `##` operand being
    /// replaced rather than to an argument substituted into it.
    pub(super) fn is_reading_operand(&self) -> bool {
        self.operand_fence != 0 && self.state.tokenizer_stack.len() == self.operand_fence
    }

    /// Returns the frame that reads `token`'s argument as a `##` operand.
    ///
    /// An argument written in a replacement list that has parameters of its
    /// own is replayed after those parameters are replaced, so the operand is
    /// what that enclosing replacement produced.
    fn operand_frame(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        let mut frame = self.raw_macro_argument_frame(context, token)?;
        let TokenizerFrameType::FunctionLikeMacroArgument {
            argument,
            paren_depth,
            ..
        } = &mut frame.frame_type
        else {
            unreachable!("macro arguments are read by argument frames");
        };
        if argument.enclosing_arguments.is_some() {
            let tokens = self.replace_operand_argument(context, argument);
            let empty_location = context
                .get_source_vectors(token.source_vectors)
                .first()
                .cloned()
                .unwrap_or_default();
            let tokenizer = TokenSource::replay(context, &tokens, empty_location);
            argument.tokenizer = tokenizer.clone();
            argument.enclosing_arguments = None;
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
    fn replace_operand_argument(
        &mut self,
        context: &mut Context<'_>,
        argument: &FunctionLikeMacroArgument,
    ) -> Vec<PreprocessorToken> {
        self.read_argument(context, argument, false)
    }

    fn read_argument(
        &mut self,
        context: &mut Context<'_>,
        argument: &FunctionLikeMacroArgument,
        expand: bool,
    ) -> Vec<PreprocessorToken> {
        let hash_hash_stack = replace(
            &mut self.hash_hash_stack,
            ArenaVec::new_in(self.state.arena),
        );
        let generate_placeholders = self.generate_placeholders;
        let newlines = (self.last_was_newline, self.current_is_newline);
        self.push_tokenizer_frame(
            context,
            TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                    argument:            Box::new(argument.clone()),
                    paren_depth:         Some(1),
                    has_generated_token: false,
                },
                tokenizer:  argument.tokenizer.clone(),
            },
        );
        let depth = self.state.tokenizer_stack.len();
        let fence = replace(&mut self.operand_fence, if expand { 0 } else { depth });
        let expansion_fence = replace(&mut self.expansion_fence, depth);
        let mut tokens = Vec::new();
        while let Some(token) = self.next_preprocessor_token::<false>(context) {
            tokens.push(token);
        }
        self.operand_fence = fence;
        self.expansion_fence = expansion_fence;
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

    /// Spells replaced operand tokens as `#` does (C99 §6.10.3.2p2), with
    /// each run of whitespace as one space.
    fn stringify(context: &mut Context<'_>, tokens: &[PreprocessorToken]) -> StringCacheId {
        let mut spelling = String::from("\"");
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
        context.string_cache.intern(&spelling)
    }

    /// C99 6.10.3.2p2: escape quotes/backslashes only inside literal tokens.
    /// A backslash that was an Other token participates in phase-5 escapes.
    fn append_stringified_token(
        context: &Context<'_>,
        spelling: &mut String,
        token: PreprocessorToken,
    ) {
        let contents = context.string_cache.at(token.contents);
        let generated;
        let contents = match token.kind {
            | PreprocessorTokenType::Number => contents.strip_suffix('\0').unwrap_or(contents),
            | PreprocessorTokenType::GeneratedString => {
                generated = string_literal_spelling(contents);
                &generated
            },
            | PreprocessorTokenType::WideGeneratedString => {
                generated = format!("L{}", string_literal_spelling(&contents[1..]));
                &generated
            },
            | _ => contents,
        };
        if matches!(
            token.kind,
            PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString
                | PreprocessorTokenType::WideGeneratedString
        ) {
            for c in contents.chars() {
                if matches!(c, '\\' | '"') {
                    spelling.push('\\');
                }
                spelling.push(c);
            }
        } else {
            spelling.push_str(contents);
        }
    }

    fn parse_hash_hash_operator(
        &mut self,
        context: &mut Context<'_>,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
        rhs_is_pasted: bool,
    ) -> Option<PreprocessorToken> {
        let lhs_frame = self.operand_frame(context, lhs);
        let rhs_frame = if rhs_is_pasted {
            None
        } else {
            self.operand_frame(context, rhs)
        };
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
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> PreprocessorToken {
        let position = self.position(context);
        let Some(argument_name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind.is_identifier(),
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
                kind:           PreprocessorTokenType::String,
                contents:       context.string_cache.intern("\"\""),
                source_vectors: token.source_vectors,
            };
        };
        let argument = match self.state.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match arguments.get(&argument_name.identifier_id(context)) {
                | None => {
                    self.set_position(context, position);
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                context.diagnostic_text(
                                    context.string_cache.at(argument_name.contents),
                                ),
                            ),
                        source_vectors: argument_name.source_vectors,
                    });
                    return PreprocessorToken {
                        kind:           PreprocessorTokenType::String,
                        contents:       context.string_cache.intern("\"\""),
                        source_vectors: token.source_vectors,
                    };
                },
                | Some(v) => v.clone(),
            },
            | _ => unreachable!(),
        };
        if argument.enclosing_arguments.is_some() {
            let tokens = self.replace_operand_argument(context, &argument);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::String,
                contents:       Self::stringify(context, &tokens),
                source_vectors: token.source_vectors,
            };
        }
        let argument_id = argument.name;
        let mut token_tokenizer = argument.tokenizer;
        let mut last_was_whitespace = true;
        let mut synthetic_contents = String::from("\"");
        let mut pending_space = false;
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
            // C99 6.10.3.2p2: whitespace separates argument tokens, but
            // trailing whitespace is not part of the resulting string.
            if token.kind == PreprocessorTokenType::Whitespace {
                pending_space = true;
                continue;
            }
            if take(&mut pending_space) {
                synthetic_contents.push(' ');
            }
            Self::append_stringified_token(context, &mut synthetic_contents, token);
        }
        synthetic_contents.push('"');
        PreprocessorToken {
            kind:           PreprocessorTokenType::String,
            contents:       context.string_cache.intern(&synthetic_contents),
            source_vectors: token.source_vectors,
        }
    }

    fn merge_token_contents(
        &mut self,
        context: &mut Context<'_>,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        self.merge_token_contents_with_ranges(context, lhs, rhs, .., .., result_token_type)
    }

    fn merge_token_contents_with_ranges(
        &mut self,
        context: &mut Context<'_>,
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
        context: &mut Context<'_>,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        _ = self;
        // Number spellings carry a trailing NUL that is not source text.
        let spell = |token: PreprocessorToken| {
            let contents = context.string_cache.at(token.contents);
            context.diagnostic_text(match token.kind {
                | PreprocessorTokenType::Number => contents.strip_suffix('\0').unwrap_or(contents),
                | _ => contents,
            })
        };
        let lhs_contents = spell(lhs);
        let rhs_contents = spell(rhs);
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    /// Appends `rhs` to the pp-number `lhs`, keeping the trailing NUL that
    /// number spellings carry.
    fn extend_number(
        &mut self,
        context: &mut Context<'_>,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        _ = self;
        let lhs_contents = context.string_cache.at(lhs.contents);
        let mut contents = TokenString::new();
        contents.push_str(lhs_contents.strip_suffix('\0').unwrap_or(lhs_contents));
        contents.push_str(context.string_cache.at(rhs.contents));
        contents.push_str("\0");
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: PreprocessorTokenType::Number,
            contents: context.string_cache.intern(&contents),
            source_vectors,
        }
    }

    pub(super) fn merge_tokens(
        &mut self,
        context: &mut Context<'_>,
        mut lhs: PreprocessorToken,
        mut rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        for token in [&mut lhs, &mut rhs] {
            if token.kind.is_identifier() {
                token.kind = PreprocessorTokenType::Identifier;
            }
        }
        let mut token = self.merge_tokens_impl(context, lhs, rhs)?;
        if token.kind == PreprocessorTokenType::Identifier
            && context.string_cache.at(token.contents).contains('\\')
        {
            (token.kind, token.contents) =
                crate::translation_phases::preprocessor_tokenizer::ucn::identifier(
                    context,
                    token.contents,
                );
        }
        Some(token)
    }

    fn merge_tokens_impl(
        &mut self,
        context: &mut Context<'_>,
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
                // A pp-number with `.` or an exponent sign continues no
                // identifier.
                if rhs_contents.contains(['.', '+', '-']) {
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
                    && (rhs.kind == PreprocessorTokenType::GeneratedString
                        || !context.string_cache.at(rhs.contents).starts_with('L'))
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
            ) => {
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
            // C99 §6.4.8: a pp-number continues with identifier characters,
            // periods, and a sign after `e`, `E`, `p`, or `P`.
            | (
                PreprocessorTokenType::Number,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Period,
            ) => Some(self.extend_number(context, lhs, rhs)),
            | (
                PreprocessorTokenType::Number,
                PreprocessorTokenType::Plus | PreprocessorTokenType::Minus,
            ) if context
                .string_cache
                .at(lhs.contents)
                .trim_end_matches('\0')
                .ends_with(['e', 'E', 'p', 'P']) =>
                Some(self.extend_number(context, lhs, rhs)),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusMinus),
            ),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::Arrow)),
            // Digraphs retain their source spelling when pasted (C99 6.4.6).
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Colon) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::OpeningSquareBracket,
                )),
            | (PreprocessorTokenType::Colon, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ClosingSquareBracket,
                )),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Percent) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::OpeningCurlyBrace,
                )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ClosingCurlyBrace,
                )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Colon) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::Hash)),
            | (PreprocessorTokenType::Hash, PreprocessorTokenType::Hash) => {
                // Mixing the spellings, such as `#` and `%:`, produces two
                // preprocessing tokens rather than one valid paste.
                let lhs_contents = context.string_cache.at(lhs.contents);
                let rhs_contents = context.string_cache.at(rhs.contents);
                if (lhs_contents == "#" && rhs_contents == "#")
                    || (lhs_contents == "%:" && rhs_contents == "%:")
                {
                    Some(self.merge_token_contents(
                        context,
                        lhs,
                        rhs,
                        PreprocessorTokenType::HashHash,
                    ))
                } else {
                    Some(self.create_merge_error(context, lhs, rhs))
                }
            },
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
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThanEquals)
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThanEquals,
                )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThanEquals)
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
        context: &mut Context<'_>,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            if self.state.tokenizer_stack.is_empty() {
                std::hint::cold_path();
                break 'base None;
            }
            if self.state.tokenizer_stack.len() < self.operand_fence.max(self.expansion_fence) {
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
                    match self.state.tokenizer_stack.last_mut().unwrap() {
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
                            let argument_name = argument.name;
                            let has_generated_token = *has_generated_token;
                            let next_paren_depth = match paren_depth {
                                | Some(depth) => self
                                    .update_macro_argument_paren_depth(
                                        context,
                                        token,
                                        argument_name,
                                        depth,
                                    )
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
                                } = self.state.tokenizer_stack.last_mut().unwrap()
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
                                self.pop_tokenizer_frame(context);
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(Self::placeholder(context));
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
                            frame_type:
                                TokenizerFrameType::SourceFile { .. } | TokenizerFrameType::Rescan,
                            ..
                        } => (),
                    }
                    break Some(token);
                },
                | None => {
                    // A replayed operand ends with its tokens, and an empty
                    // one is a placeholder like any other (C99 §6.10.3.3p2).
                    let is_empty_replay = matches!(
                        self.state.tokenizer_stack.last(),
                        Some(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                                paren_depth: None,
                                has_generated_token: false,
                                ..
                            },
                            ..
                        })
                    );
                    self.pop_tokenizer_frame(context);
                    if self.generate_placeholders && is_empty_replay {
                        break 'base Some(Self::placeholder(context));
                    }
                    continue 'base;
                },
            }
        }
    }

    fn placeholder(context: &mut Context<'_>) -> PreprocessorToken {
        PreprocessorToken {
            kind:           PreprocessorTokenType::Placeholder,
            contents:       context.string_cache.intern(""),
            source_vectors: SourceVectors::default(),
        }
    }

    fn update_macro_argument_paren_depth(
        &self,
        context: &Context<'_>,
        token: PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        // C99 §6.10.3p11: only a comma outside inner parentheses ends a
        // named argument.
        if paren_depth == 1
            && ((token.kind == PreprocessorTokenType::Comma
                && context.string_cache.at(argument_name) != "__VA_ARGS__")
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

    pub(super) fn current_is_header(&self, _context: &Context<'_>) -> bool {
        // The first in the tokenizer stack is the original source file.
        for frame in self.state.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return true,
                | _ => (),
            }
        }
        false
    }

    fn handle_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context<'_>,
    ) -> Option<PreprocessorToken> {
        let token = self.expand_macros::<SHOULD_IGNORE_WHITESPACE>(context)?;

        match token.kind {
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.state.tokenizer_stack.last(),
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

    /// Reads the next token, applying any `##` that follows it. The flag
    /// tells whether `##` formed the token, which then names no parameter.
    pub(super) fn handle_hash_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context<'_>,
    ) -> Option<(PreprocessorToken, bool)> {
        'base: loop {
            let lhs = self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)?;
            let replacement_list = match self
                .state
                .tokenizer_stack
                .last()
                .map(|frame| &frame.frame_type)
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
                let Some((rhs, rhs_is_pasted)) = self.handle_hash_hash_operator::<true>(context)
                else {
                    let source_vectors = self.current_location(context);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing hash-hash operator. Hash hash operator must be followed by a \
                             preprocessor token on the same line.",
                        ),
                        source_vectors,
                    });
                    return Some((lhs, false));
                };
                if let Some(r) = self.parse_hash_hash_operator(context, lhs, h, rhs, rhs_is_pasted)
                {
                    return Some((r, true));
                }
                continue 'base;
            }
            return Some((lhs, false));
        }
    }
}
