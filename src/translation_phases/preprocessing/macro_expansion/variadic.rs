//! Optional variadic replacements and dialect comma elision.
//!
//! C99: GNU comma-paste and traditional MSVC comma elision extend
//! §6.10.3.3 paragraph 2, p. 154; PDF p. 166. Optional replacements implement
//! C23 §6.10.5.1 paragraphs 2-7, pp. 179-180; PDF pp. 192-193.

use std::{
    cell::OnceCell,
    fmt::Write,
    mem::replace,
};

use super::{
    super::{
        Expander,
        driver::{
            TokenizerFrame,
            TokenizerFrameType,
        },
        errors::{
            PreprocessorError,
            PreprocessorErrorType,
        },
    },
    FunctionLikeMacroArgument,
    MacroArguments,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

impl<'x> Expander<'_, '_, '_, 'x> {
    /// Expands the inner replacement as a unit before outer #/##: parameters
    /// are substituted and `#` and `##` applied, but nothing is rescanned, so
    /// no name in the result is macro-replaced here.
    /// C23: §6.10.5.1 paragraph 7, p. 180; PDF p. 193.
    fn expand_optional_replacement(
        &mut self,
        invocation: PreprocessorToken,
        arguments: MacroArguments<'x>,
        argument: &'x FunctionLikeMacroArgument<'x>,
        selected: &[PreprocessorToken],
        empty: &mut Option<bool>,
    ) -> ArenaVec<'x, PreprocessorToken> {
        let mut prepared = ArenaVec::new_in(self.scratch);
        let mut index = 0;
        while index < selected.len() {
            if let Some(consumed) = self.prepare_variadic_comma(
                invocation,
                &selected[index..],
                argument,
                empty,
                &mut prepared,
            ) {
                index += consumed;
            } else {
                prepared.push(selected[index]);
                index += 1;
            }
        }
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
            .first_source_vector_or_default(invocation.source_vectors);
        let tokenizer = TokenSource::replay(
            self.context,
            self.scratch,
            &[&prepared, &[newline]],
            location.clone(),
        );
        self.push_tokenizer_frame(TokenizerFrame {
            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                name: invocation.identifier_id(self.context),
                invocation: location.clone(),
                spelling: location.clone(),
                invocation_end: location,
                arguments,
                is_variadic: true,
            },
            tokenizer,
        });
        let depth = self.tokenizer_stack.len();
        let saved_operand = replace(&mut self.operand_fence, depth);
        let saved_expansion = replace(&mut self.expansion_fence, depth);
        let saved_verbatim = replace(&mut self.verbatim_fence, depth);
        let mut result = ArenaVec::new_in(self.scratch);
        while let Some(token) = self.next_preprocessor_token::<false>() {
            result.push(token);
        }
        self.operand_fence = saved_operand;
        self.expansion_fence = saved_expansion;
        self.verbatim_fence = saved_verbatim;
        self.hash_hash_stack = saved_hashes;
        self.generate_placeholders = placeholder_mode;
        self.state.retain_placeholders = retain_placeholders;
        (self.last_was_newline, self.current_is_newline) = newlines;
        result
    }

    /// Prepares GNU comma-paste and traditional MSVC comma elision before
    /// ordinary substitution, including within a selected optional body.
    /// These are extensions to C99 §6.10.3.3 paragraph 2, p. 154; PDF p. 166.
    /// Returns the number of consumed tokens when a comma form was handled.
    fn prepare_variadic_comma(
        &mut self,
        invocation: PreprocessorToken,
        tokens: &[PreprocessorToken],
        argument: &'x FunctionLikeMacroArgument<'x>,
        empty: &mut Option<bool>,
        output: &mut ArenaVec<'x, PreprocessorToken>,
    ) -> Option<usize> {
        use PreprocessorTokenType as T;
        let token = *tokens.first()?;
        if token.kind != T::Comma {
            return None;
        }
        let mut next = 1;
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
        if !tokens.get(next).is_some_and(|t| {
            t.kind.is_identifier() && t.identifier_id(self.context) == argument.name
        }) {
            return None;
        }
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
            return Some(next + 1);
        }
        if self.context.configuration.accepts(Feature::MsVaArgs)
            && self.variadic_argument_is_empty(empty, invocation, argument)
        {
            self.context.report_extension(
                Feature::MsVaArgs,
                "empty __VA_ARGS__ comma elision",
                token.source_vectors,
            );
            return Some(next + 1);
        }
        None
    }

    fn va_opt_error(
        &mut self,
        error_type: PreprocessorErrorType<'static>,
        token: PreprocessorToken,
    ) {
        self.context.preprocessor_error(PreprocessorError {
            error_type,
            source_vectors: token.source_vectors,
        });
    }

    /// Validates the optional variadic replacement at definition time.
    /// C23: §6.10.5.1p3, p. 179; PDF p. 192.
    pub(in crate::translation_phases::preprocessing) fn validate_variadic_body(
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
            // Outside a variadic macro, the replacement-list reader already
            // reported the name, which then remains an identifier.
            if !variadic {
                continue;
            }
            if !self.context.configuration.accepts(Feature::VaOpt) {
                self.va_opt_error(PreprocessorErrorType::VaOptUnavailable, token);
                valid = false;
                continue;
            }
            self.context
                .report_extension(Feature::VaOpt, "__VA_OPT__", token.source_vectors);
            let open = Self::next_ignore_whitespace(&mut body, self.context);
            if open.is_none_or(|t| t.kind != T::OpeningParenthesis) {
                self.va_opt_error(
                    PreprocessorErrorType::MissingOpeningParenthesisAfterVaOpt,
                    token,
                );
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
                    self.va_opt_error(PreprocessorErrorType::NestedVaOpt, inner);
                    valid = false;
                }
                _ = first.get_or_insert(inner.kind);
                last = Some(inner.kind);
            }
            if depth != 0 {
                self.va_opt_error(PreprocessorErrorType::UnterminatedVaOpt, token);
                valid = false;
            }
            if first == Some(T::HashHash) || last == Some(T::HashHash) {
                self.va_opt_error(PreprocessorErrorType::HashHashAtVaOptBoundary, token);
                valid = false;
            }
        }
        self.context.set_ignore_tokenizer_errors(ignored);
        valid
    }

    /// Selects optional tokens and dialect comma elision before ordinary
    /// substitution. C23: §6.10.5.1p2-7, pp. 179-180; PDF pp. 192-193.
    /// GNU comma-paste and traditional MSVC comma elision are extensions.
    ///
    /// Each nonempty optional replacement that is not a `#` operand becomes
    /// one more argument, read through a name no source identifier can
    /// spell, so its result is rescanned with the rest of the replacement
    /// list but never substituted again (§6.10.5.1 paragraph 4).
    pub(in crate::translation_phases::preprocessing) fn prepare_variadic_body(
        &mut self,
        invocation: PreprocessorToken,
        mut body: TokenSource<'x>,
        arguments: MacroArguments<'x>,
    ) -> (TokenSource<'x>, MacroArguments<'x>) {
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
            return (original, arguments);
        }
        let Some(argument) = arguments.iter().find(|argument| argument.variadic) else {
            return (original, arguments);
        };
        // Whether `__VA_ARGS__` would be replaced by no tokens. It is decided
        // only where a construct needs it, since deciding prescans the
        // argument, which may then be used only as a `#` operand.
        let mut empty = None;
        let mut output = ArenaVec::new_in(self.scratch);
        let mut results = ArenaVec::new_in(self.scratch);
        let location = self
            .context
            .first_source_vector_or_default(invocation.source_vectors);
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
                let selected = if self.variadic_argument_is_empty(&mut empty, invocation, argument)
                {
                    ArenaVec::new_in(self.scratch)
                } else {
                    self.expand_optional_replacement(
                        invocation,
                        arguments,
                        argument,
                        &tokens[start..index],
                        &mut empty,
                    )
                };
                let hash = output
                    .iter()
                    .rposition(|t: &PreprocessorToken| t.kind != T::Whitespace)
                    .filter(|&i| output[i].kind == T::Hash);
                if let Some(hash) = hash {
                    output.truncate(hash);
                    // C23 §6.10.5.2p3: discard placemarkers and boundary
                    // whitespace before stringizing the optional argument.
                    let boundary = |token: &PreprocessorToken| {
                        matches!(token.kind, T::Whitespace | T::Newline | T::Placeholder)
                    };
                    let end = selected
                        .iter()
                        .rposition(|token| !boundary(token))
                        .map_or(0, |i| i + 1);
                    let start = selected[..end]
                        .iter()
                        .position(|token| !boundary(token))
                        .unwrap_or(end);
                    let contents =
                        Self::stringify(self.context, self.scratch, &selected[start..end]);
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
                    let mut name = ArenaString::new_in(self.scratch);
                    _ = write!(name, "__VA_OPT__ {}", results.len());
                    let name = self.context.string_cache.intern(&*name);
                    let tokenizer = TokenSource::replay(
                        self.context,
                        self.scratch,
                        &[&selected],
                        location.clone(),
                    );
                    let expanded = self.scratch.alloc(OnceCell::new());
                    drop(expanded.set(tokenizer.clone()));
                    results.push(FunctionLikeMacroArgument {
                        omitted: false,
                        variadic: false,
                        substituted: true,
                        name,
                        tokenizer,
                        enclosing_arguments: None,
                        disabled_macros: argument.disabled_macros,
                        expanded,
                    });
                    output.push(PreprocessorToken {
                        kind:           T::Identifier,
                        contents:       name,
                        source_vectors: token.source_vectors,
                    });
                }
                index += usize::from(index < tokens.len());
                continue;
            }
            if let Some(consumed) = self.prepare_variadic_comma(
                invocation,
                &tokens[index..],
                argument,
                &mut empty,
                &mut output,
            ) {
                index += consumed;
                continue;
            }
            output.push(token);
            index += 1;
        }
        let arguments = if results.is_empty() {
            arguments
        } else {
            let mut all = ArenaVec::with_capacity_in(arguments.len() + results.len(), self.scratch);
            all.extend_from_slice(arguments);
            all.extend(results);
            all.leak()
        };
        (
            TokenSource::replay(self.context, self.scratch, &[&output], location),
            arguments,
        )
    }

    /// Whether the variadic argument, completely macro-replaced, has no
    /// tokens, deciding it once per invocation.
    /// C23: §6.10.5.1 paragraph 7, p. 180; PDF p. 193.
    fn variadic_argument_is_empty(
        &mut self,
        empty: &mut Option<bool>,
        invocation: PreprocessorToken,
        argument: &'x FunctionLikeMacroArgument<'x>,
    ) -> bool {
        if let Some(empty) = *empty {
            return empty;
        }
        let mut expanded = self.expanded_argument(invocation, argument);
        let result = !std::iter::from_fn(|| expanded.next_item(self.context)).any(|t| {
            !matches!(
                t.kind,
                PreprocessorTokenType::Whitespace
                    | PreprocessorTokenType::Newline
                    | PreprocessorTokenType::Placeholder
            )
        });
        *empty = Some(result);
        result
    }
}
