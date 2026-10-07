//! Macro definitions, argument collection, and `#`/`##` operators.
//!
//! C99: macro replacement §6.10.3, pp. 151-153; PDF pp. 163-165; argument
//! substitution §6.10.3.1, p. 153; PDF p. 165; the `#` operator §6.10.3.2,
//! p. 153; PDF p. 165; the `##` operator §6.10.3.3, p. 154; PDF p. 166;
//! rescanning §6.10.3.4, p. 155; PDF p. 167.
//!
//! The order of evaluation of `#` and `##` is unspecified (§6.10.3.2
//! paragraph 2, p. 153; PDF p. 165, and §6.10.3.3 paragraph 3, p. 154;
//! PDF p. 166). Here `#` stringifies its operand before an adjacent `##`
//! pastes the result.

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

use super::{
    Expander,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
        spell_string_literal,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
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

/// A macro name's current definition. It lasts until `#undef` names it or
/// preprocessing ends.
///
/// C99: §6.10.3.5 paragraph 1, p. 155; PDF p. 167.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition<'pp> {
    /// `# define identifier replacement-list new-line`; the tokenizer reads
    /// the replacement list.
    ///
    /// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
    ObjectLike { tokenizer: TokenSource<'pp> },
    /// `# define identifier lparen identifier-list(opt) ) replacement-list
    /// new-line` and its `...` forms.
    ///
    /// C99: §6.10.3 paragraph 10, p. 152; PDF p. 164.
    FunctionLike {
        argument_names: &'pp [StringCacheId],
        tokenizer:      TokenSource<'pp>,
        is_variadic:    bool,
    },
    /// A predefined macro or the `_Pragma` operator, expanded by the driver.
    ///
    /// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172, and §6.10.9 paragraph
    /// 1, p. 161; PDF p. 173.
    BuiltIn,
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
    pub(super) omitted:             bool,
    pub(super) name:                StringCacheId,
    pub(super) tokenizer:           TokenSource<'x>,
    pub(super) enclosing_arguments: Option<MacroArguments<'x>>,
    pub(super) disabled_macros:     &'x [StringCacheId],
    /// Shared once-only argument prescan; raw #/## operands bypass it.
    pub(super) expanded:            &'x OnceCell<TokenSource<'x>>,
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
                        tokens = if let Some(argument) = arguments.and_then(|arguments| {
                            find_argument(arguments, token.identifier_id(preprocessor.context))
                        }) {
                            preprocessor.read_argument(argument, false)
                        } else {
                            let mut tokens = ArenaVec::new_in(preprocessor.scratch);
                            tokens.push(token);
                            tokens
                        };
                    }
                    let mut right = if let Some(argument) = arguments.and_then(|arguments| {
                        find_argument(arguments, rhs.identifier_id(preprocessor.context))
                    }) {
                        preprocessor.read_argument(argument, false)
                    } else {
                        let mut right = ArenaVec::new_in(preprocessor.scratch);
                        right.push(rhs);
                        right
                    };
                    if !tokens.is_empty() && !right.is_empty() {
                        let lhs = tokens.pop().unwrap();
                        let rhs = right.remove(0);
                        if let Some(merged) = preprocessor.merge_tokens(lhs, rhs) {
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
                    frame_type: TokenizerFrameType::Rescan,
                    tokenizer,
                },
            );
        }
        preprocessor.tokenizer = self.frames.last().unwrap().tokenizer.clone();
        preprocessor.tokenizer_stack.clear();
        preprocessor.tokenizer_stack.extend(self.frames);
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
impl<'x> Expander<'_, '_, '_, 'x> {
    /// Expands the inner replacement as a unit before outer #/##; it is not
    /// rescanned. C23: §6.10.5.1p7, p. 180; PDF p. 193.
    fn expand_optional_replacement(
        &mut self,
        invocation: PreprocessorToken,
        arguments: MacroArguments<'x>,
        selected: &[PreprocessorToken],
    ) -> ArenaVec<'x, PreprocessorToken> {
        let saved_hashes = replace(&mut self.hash_hash_stack, ArenaVec::new_in(self.scratch));
        let newlines = (self.last_was_newline, self.current_is_newline);
        let placeholder_mode = self.generate_placeholders;
        let retain_placeholders = replace(&mut self.state.retain_placeholders, true);
        let newline = PreprocessorToken {
            kind:           PreprocessorTokenType::Newline,
            contents:       self.context.string_cache.intern("\n"),
            source_vectors: invocation.source_vectors,
        };
        let location = self
            .context
            .get_source_vectors(invocation.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default();
        let tokenizer = TokenSource::replay(
            self.context,
            self.scratch,
            &[selected, &[newline]],
            location.clone(),
        );
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                name: invocation.identifier_id(self.context),
                invocation: location.clone(),
                invocation_end: location,
                arguments,
                is_variadic: true,
            },
            tokenizer,
        });
        let depth = self.tokenizer_stack.len();
        let saved_operand = replace(&mut self.operand_fence, depth);
        let saved_expansion = replace(&mut self.expansion_fence, depth);
        let mut result = ArenaVec::new_in(self.scratch);
        while let Some(token) = self.next_preprocessor_token::<false>() {
            result.push(token);
        }
        self.operand_fence = saved_operand;
        self.expansion_fence = saved_expansion;
        self.hash_hash_stack = saved_hashes;
        self.generate_placeholders = placeholder_mode;
        self.state.retain_placeholders = retain_placeholders;
        (self.last_was_newline, self.current_is_newline) = newlines;
        result
    }

    /// Validates the optional variadic replacement at definition time.
    /// C23: §6.10.5.1p3, p. 179; PDF p. 192.
    pub(super) fn validate_variadic_body(
        &mut self,
        mut body: TokenSource<'_>,
        variadic: bool,
    ) -> bool {
        use PreprocessorTokenType as T;
        let ignored = self.context.ignore_tokenizer_errors();
        self.context.set_ignore_tokenizer_errors(true);
        let mut valid = true;
        while let Some(token) = body.next_item(self.context) {
            if token.kind == T::Newline {
                break;
            }
            if !token.kind.is_identifier()
                || self.context.string_cache.at(token.contents) != "__VA_OPT__"
            {
                continue;
            }
            if !variadic || !self.context.configuration.accepts(Feature::VaOpt) {
                self.language_error(
                    "__VA_OPT__ requires an enabled variadic macro replacement",
                    token.source_vectors,
                );
                valid = false;
                continue;
            }
            self.context
                .report_extension(Feature::VaOpt, "__VA_OPT__", token.source_vectors);
            let open = Self::next_ignore_whitespace(&mut body, self.context);
            if open.is_none_or(|t| t.kind != T::OpeningParenthesis) {
                self.language_error("expected '(' after __VA_OPT__", token.source_vectors);
                valid = false;
                continue;
            }
            let mut depth = 1usize;
            let mut first = None;
            let mut last = None;
            while let Some(inner) = body.next_item(self.context) {
                if inner.kind == T::Newline {
                    break;
                }
                if inner.kind == T::OpeningParenthesis {
                    depth += 1;
                }
                if inner.kind == T::ClosingParenthesis {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                if inner.kind == T::Whitespace {
                    continue;
                }
                if inner.kind.is_identifier()
                    && self.context.string_cache.at(inner.contents) == "__VA_OPT__"
                {
                    self.language_error("__VA_OPT__ cannot be nested", inner.source_vectors);
                    valid = false;
                }
                _ = first.get_or_insert(inner.kind);
                last = Some(inner.kind);
            }
            if depth != 0 {
                self.language_error("unterminated __VA_OPT__ replacement", token.source_vectors);
                valid = false;
            }
            if first == Some(T::HashHash) || last == Some(T::HashHash) {
                self.language_error(
                    "## cannot begin or end a __VA_OPT__ replacement",
                    token.source_vectors,
                );
                valid = false;
            }
        }
        self.context.set_ignore_tokenizer_errors(ignored);
        valid
    }

    /// Selects optional tokens and dialect comma elision before ordinary
    /// substitution. C23: §6.10.5.1p2-7, pp. 179-180; PDF pp. 192-193.
    /// GNU comma-paste and traditional MSVC comma elision are extensions.
    pub(super) fn prepare_variadic_body(
        &mut self,
        invocation: PreprocessorToken,
        mut body: TokenSource<'x>,
        arguments: MacroArguments<'x>,
    ) -> TokenSource<'x> {
        use PreprocessorTokenType as T;
        let original = body.clone();
        let mut tokens = ArenaVec::new_in(self.scratch);
        let mut optional = false;
        let mut comma = false;
        while let Some(token) = body.next_item(self.context) {
            optional |= token.kind.is_identifier()
                && self.context.string_cache.at(token.contents) == "__VA_OPT__";
            comma |= token.kind == T::Comma;
            tokens.push(token);
            if token.kind == T::Newline {
                break;
            }
        }
        if !optional && !comma {
            return original;
        }
        let va_name = self.context.string_cache.intern("__VA_ARGS__");
        let Some(argument) = find_argument(arguments, va_name) else {
            return original;
        };
        let empty = if optional || self.context.configuration.accepts(Feature::MsVaArgs) {
            let mut expanded = self.expanded_argument(invocation, argument);
            !std::iter::from_fn(|| expanded.next_item(self.context))
                .any(|t| !matches!(t.kind, T::Whitespace | T::Newline | T::Placeholder))
        } else {
            false
        };
        let mut output = ArenaVec::new_in(self.scratch);
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index];
            if token.kind.is_identifier()
                && self.context.string_cache.at(token.contents) == "__VA_OPT__"
            {
                index += 1;
                while tokens.get(index).is_some_and(|t| t.kind == T::Whitespace) {
                    index += 1;
                }
                if tokens
                    .get(index)
                    .is_none_or(|t| t.kind != T::OpeningParenthesis)
                {
                    continue;
                }
                index += 1;
                let start = index;
                let mut depth = 1usize;
                while index < tokens.len() {
                    if tokens[index].kind == T::OpeningParenthesis {
                        depth += 1;
                    }
                    if tokens[index].kind == T::ClosingParenthesis {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    index += 1;
                }
                let selected = if empty {
                    ArenaVec::new_in(self.scratch)
                } else {
                    self.expand_optional_replacement(invocation, arguments, &tokens[start..index])
                };
                let hash = output
                    .iter()
                    .rposition(|t: &PreprocessorToken| t.kind != T::Whitespace)
                    .filter(|&i| output[i].kind == T::Hash);
                if let Some(hash) = hash {
                    output.truncate(hash);
                    let contents = Self::stringify(self.context, self.scratch, &selected);
                    output.push(PreprocessorToken {
                        kind: T::String,
                        contents,
                        source_vectors: token.source_vectors,
                    });
                } else if selected.is_empty() {
                    output.push(PreprocessorToken {
                        kind:           T::Placeholder,
                        contents:       self.context.string_cache.intern(""),
                        source_vectors: token.source_vectors,
                    });
                } else {
                    output.extend_from_slice(&selected);
                }
                index += usize::from(index < tokens.len());
                continue;
            }
            if token.kind == T::Comma {
                let mut next = index + 1;
                while tokens.get(next).is_some_and(|t| t.kind == T::Whitespace) {
                    next += 1;
                }
                let paste = tokens.get(next).is_some_and(|t| t.kind == T::HashHash);
                if paste {
                    next += 1;
                    while tokens.get(next).is_some_and(|t| t.kind == T::Whitespace) {
                        next += 1;
                    }
                }
                if tokens.get(next).is_some_and(|t| {
                    t.kind.is_identifier() && t.identifier_id(self.context) == va_name
                }) {
                    if paste {
                        self.context.report_extension(
                            Feature::GnuVaArgs,
                            ", ## __VA_ARGS__",
                            token.source_vectors,
                        );
                        if !argument.omitted {
                            output.push(token);
                            output.push(tokens[next]);
                        }
                        index = next + 1;
                        continue;
                    }
                    if empty && self.context.configuration.accepts(Feature::MsVaArgs) {
                        self.context.report_extension(
                            Feature::MsVaArgs,
                            "empty __VA_ARGS__ comma elision",
                            token.source_vectors,
                        );
                        index = next + 1;
                        continue;
                    }
                }
            }
            output.push(token);
            index += 1;
        }
        TokenSource::replay(
            self.context,
            self.scratch,
            &[&output],
            self.context
                .get_source_vectors(invocation.source_vectors)
                .first()
                .cloned()
                .unwrap_or_default(),
        )
    }

    /// Capture an invocation whose opening, arguments, or closing delimiter
    /// can come from different replacement/argument/source frames.
    ///
    /// C99: §6.10.3.4 paragraph 1, p. 155; PDF p. 167; argument separation
    /// and count, §6.10.3 paragraphs 4 and 11-12, pp. 151-153; PDF
    /// pp. 163-165.
    pub(super) fn capture_cross_frame_call(
        &mut self,
        invocation: PreprocessorToken,
        names: &[StringCacheId],
        variadic: bool,
    ) -> Option<(MacroArguments<'x>, crate::translation_phases::SourceVector)> {
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
            let Some(token) = cursor.next(self) else {
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
            let policy = self.context.configuration.extension_policy();
            if policy != crate::configuration::ExtensionPolicy::Allow
                && self.context.configuration.standard() < crate::configuration::CStandard::C23
                && !self.context.configuration.accepts(Feature::MsVaArgs)
            {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingVariadicArgument(policy),
                    source_vectors: invocation.source_vectors,
                });
            }
        }
        let disabled_macros = self.disabled_macros();
        let location = self
            .context
            .get_source_vectors(closing.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default();
        let parameters = names.len() + usize::from(variadic);
        let mut arguments = ArenaVec::with_capacity_in(parameters, self.scratch);
        for (i, name) in names
            .iter()
            .copied()
            .chain(variadic.then(|| self.context.string_cache.intern("__VA_ARGS__")))
            .enumerate()
        {
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

    pub(super) fn get_arguments(&self) -> Option<MacroArguments<'x>> {
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

    pub(super) fn macro_argument_is_at_end(&mut self) -> bool {
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
                        self.update_macro_argument_paren_depth(token, name, depth)
                            .is_none()
                    }),
                | None => break true,
            }
        };
        self.set_position(position);
        next_is_end
    }

    /// Whether `name` is a macro being replaced where the current token is
    /// read, so that the name is not replaced again.
    ///
    /// C99: §6.10.3.4 paragraph 2, p. 155; PDF p. 167.
    pub(super) fn macro_is_disabled(&self, name: StringCacheId) -> bool {
        for frame in self.tokenizer_stack.iter().rev() {
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
    pub(super) fn disabled_macros(&self) -> &'x [StringCacheId] {
        if let Some(frame) = self.tokenizer_stack.last() {
            match &frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return &[],
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } =>
                    return argument.disabled_macros,
                | _ => (),
            }
        }
        let mut names = ArenaVec::new_in(self.scratch);
        for frame in self.tokenizer_stack.iter().rev() {
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

    /// Substitutes the macro-replaced argument for a parameter that is not a
    /// `#` or `##` operand.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
    pub(super) fn handle_macro_argument(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame<'x>> {
        let arguments = self.get_arguments()?;
        let argument = find_argument(arguments, token.identifier_id(self.context))?;
        // Prescan is isolated from the replacement list. Rescanning the result
        // then uses the callee's disabled-name set, not the caller's.
        Some(TokenizerFrame {
            frame_type: TokenizerFrameType::Rescan,
            tokenizer:  self.expanded_argument(token, argument),
        })
    }

    /// Share prescan results with speculative invocation lookahead too:
    /// repeating replacement would also repeat its diagnostics and
    /// `_Pragma` effects.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165: the argument is
    /// replaced as if it formed the rest of the file, with no other tokens
    /// available.
    fn expanded_argument(
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
            .get_source_vectors(token.source_vectors)
            .first()
            .cloned()
            .unwrap_or_default();
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
        self.operand_fence != 0 && self.tokenizer_stack.len() == self.operand_fence
    }

    /// Returns the frame that reads `token`'s argument as a `##` operand.
    ///
    /// An argument written in a replacement list that has parameters of its
    /// own is replayed after those parameters are replaced, so the operand is
    /// what that enclosing replacement produced.
    fn operand_frame(&mut self, token: PreprocessorToken) -> Option<TokenizerFrame<'x>> {
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
                .get_source_vectors(token.source_vectors)
                .first()
                .cloned()
                .unwrap_or_default();
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
    fn replace_operand_argument(
        &mut self,
        argument: &'x FunctionLikeMacroArgument<'x>,
    ) -> ArenaVec<'x, PreprocessorToken> {
        self.read_argument(argument, false)
    }

    fn read_argument(
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
                paren_depth: Some(1),
                has_generated_token: false,
            },
            tokenizer:  argument.tokenizer.clone(),
        });
        let depth = self.tokenizer_stack.len();
        let fence = replace(&mut self.operand_fence, if expand { 0 } else { depth });
        let expansion_fence = replace(&mut self.expansion_fence, depth);
        let mut tokens = ArenaVec::new_in(self.scratch);
        while let Some(token) = self.next_preprocessor_token::<false>() {
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
    fn stringify(
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

    /// Pastes `lhs ## rhs`. A parameter operand stands for its argument as
    /// written, the last token of a left argument and the first of a right
    /// one taking part; an empty argument is a placemarker.
    ///
    /// C99: §6.10.3.3 paragraphs 2-3, p. 154; PDF p. 166.
    fn parse_hash_hash_operator(
        &mut self,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
        rhs_is_pasted: bool,
    ) -> Option<PreprocessorToken> {
        let lhs_frame = self.operand_frame(lhs);
        let rhs_frame = if rhs_is_pasted {
            None
        } else {
            self.operand_frame(rhs)
        };
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = rhs_frame {
            self.push_tokenizer_frame(frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs);
            false
        };
        if let Some(frame) = lhs_frame {
            self.push_tokenizer_frame(frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs);
        } else {
            _ = self.hash_hash_stack.pop();
            return self.merge_tokens(lhs, rhs);
        }
        None
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
        let argument_id = argument.name;
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
            match self.update_macro_argument_paren_depth(token, argument_id, paren_depth) {
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

    fn merge_token_contents(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        self.merge_token_contents_with_ranges(lhs, rhs, .., .., result_token_type)
    }

    fn merge_token_contents_with_ranges(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        lhs_range: impl RangeBounds<usize>,
        rhs_range: impl RangeBounds<usize>,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        let mut new_contents = ArenaString::new_in(self.scratch);
        new_contents.push_str(
            &self.context.string_cache.at(lhs.contents)[(
                lhs_range.start_bound().cloned(),
                lhs_range.end_bound().cloned(),
            )],
        );
        new_contents.push_str(
            &self.context.string_cache.at(rhs.contents)[(
                rhs_range.start_bound().cloned(),
                rhs_range.end_bound().cloned(),
            )],
        );
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: result_token_type,
            contents: self.context.string_cache.intern(new_contents.as_str()),
            source_vectors,
        }
    }

    fn create_merge_error(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        // Number spellings carry a trailing NUL that is not source text.
        let spell = |token: PreprocessorToken| {
            let contents = self.context.string_cache.at(token.contents);
            self.context.diagnostic_text(match token.kind {
                | PreprocessorTokenType::Number => contents.strip_suffix('\0').unwrap_or(contents),
                | _ => contents,
            })
        };
        let lhs_contents = spell(lhs);
        let rhs_contents = spell(rhs);
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        self.context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    /// Appends `rhs` to the pp-number `lhs`, keeping the trailing NUL that
    /// number spellings carry.
    fn extend_number(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        let lhs_contents = self.context.string_cache.at(lhs.contents);
        let mut contents = ArenaString::new_in(self.scratch);
        contents.push_str(lhs_contents.strip_suffix('\0').unwrap_or(lhs_contents));
        contents.push_str(self.context.string_cache.at(rhs.contents));
        contents.push_str("\0");
        let source_vectors = self
            .context
            .merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: PreprocessorTokenType::Number,
            contents: self.context.string_cache.intern(contents.as_str()),
            source_vectors,
        }
    }

    /// Concatenates two preprocessing tokens for `##`, or `None` when both
    /// are placemarkers. A result that is not one valid preprocessing token
    /// is undefined behavior; it is diagnosed, keeping the left operand.
    /// Forming a universal character name by concatenation is undefined too;
    /// an identifier that pasting leaves with a `\` is read as a universal
    /// identifier.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166, and §5.1.1.2
    /// paragraph 1 item 4, p. 10; PDF p. 22.
    pub(super) fn merge_tokens(
        &mut self,
        mut lhs: PreprocessorToken,
        mut rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        for token in [&mut lhs, &mut rhs] {
            if token.kind.is_identifier() {
                token.kind = PreprocessorTokenType::Identifier;
            }
        }
        let mut token = self.merge_tokens_impl(lhs, rhs)?;
        if !self.context.configuration.is_native(Feature::Digraphs)
            && let spelling @ ("<:" | ":>" | "<%" | "%>" | "%:" | "%:%:") =
                self.context.string_cache.at(token.contents)
        {
            let spelling = match spelling {
                | "<:" => "<:",
                | ":>" => ":>",
                | "<%" => "<%",
                | "%>" => "%>",
                | "%:" => "%:",
                | _ => "%:%:",
            };
            self.context
                .report_extension(Feature::Digraphs, spelling, token.source_vectors);
        }
        if token.kind == PreprocessorTokenType::Identifier
            && self.context.string_cache.at(token.contents).contains('\\')
        {
            (token.kind, token.contents) =
                crate::translation_phases::preprocessor_tokenizer::ucn::identifier(
                    self.context,
                    self.scratch,
                    token.contents,
                );
        }
        Some(token)
    }

    fn merge_tokens_impl(
        &mut self,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        if !self.context.configuration.accepts(Feature::Digraphs)
            && matches!(
                (lhs.kind, rhs.kind),
                (
                    PreprocessorTokenType::LessThan,
                    PreprocessorTokenType::Colon | PreprocessorTokenType::Percent
                ) | (
                    PreprocessorTokenType::Colon | PreprocessorTokenType::Percent,
                    PreprocessorTokenType::GreaterThan
                ) | (PreprocessorTokenType::Percent, PreprocessorTokenType::Colon)
            )
        {
            return Some(self.create_merge_error(lhs, rhs));
        }
        match (lhs.kind, rhs.kind) {
            // C99 §6.10.3.3p3: two placemarkers give one placemarker, and a
            // placemarker with another token gives that token.
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(rhs),
            | (_, PreprocessorTokenType::Placeholder) => Some(lhs),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                let rhs_contents = self.context.string_cache.at(rhs.contents);
                // A pp-number with `.` or an exponent sign continues no
                // identifier.
                if rhs_contents.contains(['.', '+', '-', '\'']) {
                    return Some(self.create_merge_error(lhs, rhs));
                }
                let rhs_range = if rhs.kind == PreprocessorTokenType::Number {
                    0..rhs_contents.len() - 1
                } else {
                    0..rhs_contents.len()
                };
                let new = self.merge_token_contents_with_ranges(
                    lhs,
                    rhs,
                    ..,
                    rhs_range,
                    PreprocessorTokenType::Identifier,
                );
                let kind = if self.context.string_cache.at(new.contents) == "defined" {
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
                let prefix = self.context.string_cache.at(lhs.contents);
                let enabled = prefix == "L"
                    || (matches!(prefix, "u" | "U" | "u8")
                        && self
                            .context
                            .configuration
                            .accepts(Feature::UnicodeLiteralPrefixes)
                        && (prefix != "u8"
                            || rhs.kind != PreprocessorTokenType::Character
                            || self
                                .context
                                .configuration
                                .accepts(Feature::Utf8CharacterConstants)));
                if enabled
                    && (rhs.kind == PreprocessorTokenType::GeneratedString
                        || self
                            .context
                            .string_cache
                            .at(rhs.contents)
                            .starts_with(['"', '\'']))
                {
                    Some(self.merge_token_contents(
                        lhs,
                        rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    ))
                } else {
                    Some(self.create_merge_error(lhs, rhs))
                }
            },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            ) => {
                let lhs_contents = self.context.string_cache.at(lhs.contents);
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
            ) => Some(self.extend_number(lhs, rhs)),
            | (
                PreprocessorTokenType::Number,
                PreprocessorTokenType::Plus | PreprocessorTokenType::Minus,
            ) if self
                .context
                .string_cache
                .at(lhs.contents)
                .trim_end_matches('\0')
                .ends_with(['e', 'E', 'p', 'P']) =>
                Some(self.extend_number(lhs, rhs)),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::MinusMinus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::Arrow)),
            // Digraphs retain their source spelling when pasted (C99 6.4.6).
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Colon) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::OpeningSquareBracket),
            ),
            | (PreprocessorTokenType::Colon, PreprocessorTokenType::GreaterThan) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ClosingSquareBracket),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Percent) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::OpeningCurlyBrace)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ClosingCurlyBrace)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Colon) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::Hash)),
            | (PreprocessorTokenType::Hash, PreprocessorTokenType::Hash) => {
                // Mixing the spellings, such as `#` and `%:`, produces two
                // preprocessing tokens rather than one valid paste.
                let lhs_contents = self.context.string_cache.at(lhs.contents);
                let rhs_contents = self.context.string_cache.at(rhs.contents);
                if (lhs_contents == "#" && rhs_contents == "#")
                    || (lhs_contents == "%:" && rhs_contents == "%:")
                {
                    Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::HashHash))
                } else {
                    Some(self.create_merge_error(lhs, rhs))
                }
            },
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PlusEquals)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::MinusEquals)),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AsteriskEquals)),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ForwardSlashEquals)),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PercentEquals)),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanLessThan)),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::GreaterThanGreaterThan),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanEquals)),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::GreaterThanEquals)),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThanEquals)
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::LessThanLessThanEquals),
            ),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThanEquals)
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AmpersandEquals)),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::CaretEquals)),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PipeEquals)),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(lhs, rhs, PreprocessorTokenType::ExclamationMarkEquals),
            ),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::EqualsEquals)),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::PipePipe)),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) =>
                Some(self.merge_token_contents(lhs, rhs, PreprocessorTokenType::AmpersandAmpersand)),

            | _ => Some(self.create_merge_error(lhs, rhs)),
        }
    }

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
                            let argument_name = argument.name;
                            let has_generated_token = *has_generated_token;
                            let next_paren_depth = match paren_depth {
                                | Some(depth) => self
                                    .update_macro_argument_paren_depth(token, argument_name, depth)
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

    /// The placemarker that stands for an empty `##` operand; it exists only
    /// within phase 4.
    ///
    /// C99: §6.10.3.3 paragraph 2 and footnote 151, p. 154; PDF p. 166.
    fn placeholder(context: &mut Context<'_>) -> PreprocessorToken {
        PreprocessorToken {
            kind:           PreprocessorTokenType::Placeholder,
            contents:       context.string_cache.intern(""),
            source_vectors: SourceVectors::default(),
        }
    }

    fn update_macro_argument_paren_depth(
        &self,
        token: PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        // C99 §6.10.3p11: only a comma outside inner parentheses ends a
        // named argument.
        if paren_depth == 1
            && ((token.kind == PreprocessorTokenType::Comma
                && self.context.string_cache.at(argument_name) != "__VA_ARGS__")
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

    pub(super) fn current_is_header(&self) -> bool {
        // The first in the tokenizer stack is the original source file.
        for frame in self.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile { .. } => return true,
                | _ => (),
            }
        }
        false
    }

    /// Reads the next token, applying `#` when it is one. `#` is an operator
    /// only in the replacement list of a function-like macro.
    ///
    /// C99: §6.10.3.2 paragraph 1, p. 153; PDF p. 165.
    fn handle_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
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

    /// Reads the next token, applying any `##` that follows it. The flag
    /// tells whether `##` formed the token, which then names no parameter.
    ///
    /// C99: §6.10.3.3 paragraph 3, p. 154; PDF p. 166: `##` is an operator in
    /// the replacement list of either form of macro, but not one that comes
    /// from an argument.
    pub(super) fn handle_hash_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
    ) -> Option<(PreprocessorToken, bool)> {
        'base: loop {
            let lhs = self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>()?;
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
                let save = self.position();
                self.context.set_ignore_tokenizer_errors(true);
                let hash_hash = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
                self.context.set_ignore_tokenizer_errors(false);
                self.set_position(save);
                if hash_hash.is_some_and(|v| v.kind == PreprocessorTokenType::HashHash) {
                    Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(h) = hash_hash {
                let Some((rhs, rhs_is_pasted)) = self.handle_hash_hash_operator::<true>() else {
                    let source_vectors = self.current_location();
                    self.context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing hash-hash operator. Hash hash operator must be followed by a \
                             preprocessor token on the same line.",
                        ),
                        source_vectors,
                    });
                    return Some((lhs, false));
                };
                if let Some(r) = self.parse_hash_hash_operator(lhs, h, rhs, rhs_is_pasted) {
                    return Some((r, true));
                }
                continue 'base;
            }
            return Some((lhs, false));
        }
    }
}
