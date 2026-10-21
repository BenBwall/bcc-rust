//! Pragma directive execution, destringizing, and implementation-defined
//! operands.
//!
//! C99: #pragma, §6.10.6, p. 159; PDF p. 171; _Pragma destringizing,
//! §6.10.9 paragraph 1, p. 161; PDF p. 173.

use std::fmt::Write as _;

use super::super::{
    Expander,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    runtime::MacroDeprecation,
    token::{
        StringTokenType,
        TokenType,
    },
};
use crate::{
    translation_phases::{
        Context,
        SourceVectors,
        interface::StrExt,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::{
        bump::ArenaString,
        string_cache::StringCacheId,
    },
};

impl Expander<'_, '_, '_, '_> {
    /// Executes a `#pragma` directive, or the pragma of a `_Pragma` operator,
    /// returning whether its new-line was consumed. Most operands are not
    /// macro-replaced; pack operands are replaced for GCC/Clang compatibility.
    ///
    /// C99: §6.10.6 paragraphs 1-2, p. 159; PDF p. 171. `STDC` pragmas must
    /// name `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE` and an
    /// `on-off-switch`; they are checked but have no effect yet. `#pragma
    /// once`, `#pragma pack`, `#pragma GCC system_header` and `#pragma GCC
    /// warning`/`error` are bcc's implementation-defined pragmas. `#pragma
    /// weak` keeps only the name it makes weak, for the emission model of
    /// semantic analysis. Other pragmas
    /// are ignored (paragraph 1), and one that does not begin with an
    /// identifier draws a warning first. `from` is the physical
    /// invocation's byte offset, including when `_Pragma` supplies tokens
    /// from a synthetic string (C99 §6.10.9p1, p. 161; PDF p. 173).
    pub(in crate::translation_phases::preprocessing) fn parse_pragma_directive(
        &mut self,
        from: u32,
    ) -> bool {
        let mut consumed_newline = false;
        let mut completed_stdc = false;
        'base: loop {
            let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
            else {
                let source_vectors = self.current_location();
                self.context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing pragma directive",
                    ),
                    source_vectors,
                });
                break 'base;
            };
            match token.kind {
                | PreprocessorTokenType::Newline => {
                    consumed_newline = true;
                    break 'base;
                },
                | PreprocessorTokenType::Identifier
                | PreprocessorTokenType::UniversalIdentifier => {
                    match self.context.string_cache.at(token.contents) {
                        | "pack" => {
                            self.retain_pack_pragma(token.source_vectors);
                            consumed_newline = true;
                            break 'base;
                        },
                        | "once" => {
                            if !self.current_is_header() {
                                self.context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::PragmaOnceInNonHeader,
                                    source_vectors: token.source_vectors,
                                });
                            }
                            _ = self
                                .state
                                .once_set
                                .insert(self.physical_source_file_index());
                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    break 'base;
                                },
                                | Some(extra) => {
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                extra.kind,
                                            ),
                                        source_vectors: extra.source_vectors,
                                    });
                                    break 'base;
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | "weak" => {
                            // `#pragma weak name` and `#pragma weak name =
                            // target` make `name` weak. bcc emits it as
                            // before, but semantic analysis keeps the name:
                            // Clang's evaluator never folds a weak entity.
                            let Some(name) =
                                Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
                            else {
                                break 'base;
                            };
                            if name.kind == PreprocessorTokenType::Newline {
                                consumed_newline = true;
                                break 'base;
                            }
                            if name.kind.is_identifier() {
                                let id = name.identifier_id(self.context);
                                self.context.weak_pragmas.push(id);
                            }
                            let line_state = (self.last_was_newline, self.current_is_newline);
                            self.skip_until_newline();
                            (self.last_was_newline, self.current_is_newline) = line_state;
                            consumed_newline = true;
                            break 'base;
                        },
                        | "clang" => {
                            if !self.pragma_deprecated_macro() {
                                let line_state = (self.last_was_newline, self.current_is_newline);
                                self.skip_until_newline();
                                (self.last_was_newline, self.current_is_newline) = line_state;
                            }
                            consumed_newline = true;
                            break 'base;
                        },
                        | "GCC" => {
                            let Some(operand) =
                                Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
                            else {
                                break 'base;
                            };
                            if operand.kind == PreprocessorTokenType::Newline {
                                consumed_newline = true;
                                break 'base;
                            }
                            if operand.kind.is_identifier() {
                                match self.context.string_cache.at(operand.contents) {
                                    | "system_header" => {
                                        self.pragma_system_header(operand.source_vectors, from);
                                    },
                                    | kind @ ("warning" | "error") => {
                                        self.pragma_gcc_message(operand, kind == "error");
                                        consumed_newline = true;
                                        break 'base;
                                    },
                                    | _ => {},
                                }
                            }
                            // Other GCC pragmas, and any operands, are ignored.
                            let line_state = (self.last_was_newline, self.current_is_newline);
                            self.skip_until_newline();
                            (self.last_was_newline, self.current_is_newline) = line_state;
                            consumed_newline = true;
                            break 'base;
                        },
                        | "STDC" => {
                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    self.context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = self.context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(
                                            s,
                                            "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE"
                                        )
                                    {
                                        if self.context.group_severity(
                                            crate::configuration::DiagnosticGroup::UnknownStdcPragma,
                                            token.source_vectors,
                                        ).is_some() {
                                            self.context.preprocessor_error(PreprocessorError {
                                                error_type:
                                                    PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                        self.context.diagnostic_text(s),
                                                    ),
                                                source_vectors: token.source_vectors,
                                            });
                                        }
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }

                            match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    self.context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = self.context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(s, "ON" | "OFF" | "DEFAULT")
                                    {
                                        self.context.preprocessor_error(PreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(self.context.diagnostic_text(s)),
                                                    source_vectors: token.source_vectors,
                                                },
                                            );
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location();
                                    self.context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                            completed_stdc = true;
                        },
                        | _ => {
                            if !completed_stdc {
                                // An unknown pragma is ignored as a whole.
                                // Preserve the caller's line state for _Pragma,
                                // whose payload uses a temporary tokenizer.
                                let line_state = (self.last_was_newline, self.current_is_newline);
                                self.skip_until_newline();
                                (self.last_was_newline, self.current_is_newline) = line_state;
                                consumed_newline = true;
                            }
                            break 'base;
                        },
                    }
                },
                | _ => {
                    if self
                        .context
                        .group_severity(
                            crate::configuration::DiagnosticGroup::UnknownPragmas,
                            token.source_vectors,
                        )
                        .is_some()
                    {
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                            source_vectors: token.source_vectors,
                        });
                    }
                    break 'base;
                },
            }
        }
        consumed_newline
    }

    /// GCC/Clang pack operands are macro-replaced, unlike ordinary pragmas.
    /// Pass their tokens and output boundary to phase 7 for interpretation.
    /// C99: §6.10.6p1 and footnote 152, p. 159; PDF p. 171 permit this choice.
    /// Clang: <https://clang.llvm.org/doxygen/ParsePragma_8cpp_source.html>.
    fn retain_pack_pragma(&mut self, source: SourceVectors) {
        use crate::translation_phases::pragma_pack::{
            PackPragma,
            PackToken,
        };
        let arena = self.context.tu_arena();
        let mut operands = crate::util::bump::ArenaVec::new_in(arena);
        self.expand_pragma_line(source, |_, token| {
            operands.push(PackToken {
                spelling:   token.contents,
                identifier: token.kind.is_identifier(),
            });
        });
        let source = self.context.retain_token_source(source);
        self.context.pack_pragmas.push(PackPragma {
            position: self.context.preprocessed_tokens,
            operands: operands.leak(),
            source,
        });
    }

    /// `#pragma GCC warning` and `#pragma GCC error` report their operand,
    /// one or more ordinary string literals after macro replacement,
    /// optionally parenthesized, as a warning or an error at the pragma's
    /// kind, as GCC and Clang do. Any other operand is an error, as in Clang.
    /// C99: implementation-defined pragma behavior, §6.10.6p1, p. 159; PDF
    /// p. 171; string literal concatenation, §6.4.5p4, p. 62; PDF p. 74.
    /// GCC: <https://gcc.gnu.org/onlinedocs/gcc/Diagnostic-Pragmas.html>.
    fn pragma_gcc_message(&mut self, kind: PreprocessorToken, error: bool) {
        let mut tokens = crate::util::bump::ArenaVec::new_in(self.scratch);
        self.expand_pragma_line(kind.source_vectors, |_, token| tokens.push(token));
        let mut rest = tokens.as_slice();
        let parenthesized = rest
            .first()
            .is_some_and(|token| token.kind == PreprocessorTokenType::OpeningParenthesis);
        if parenthesized {
            rest = &rest[1..];
        }
        let count = rest
            .iter()
            .take_while(|token| self.is_deprecation_message_literal(**token))
            .count();
        let (literals, mut rest) = rest.split_at(count);
        let mut closed = !parenthesized;
        if parenthesized
            && rest
                .first()
                .is_some_and(|token| token.kind == PreprocessorTokenType::ClosingParenthesis)
        {
            rest = &rest[1..];
            closed = true;
        }
        if literals.is_empty() || !rest.is_empty() || !closed {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LanguageConstraint(if error {
                    "`#pragma GCC error` requires a string literal"
                } else {
                    "`#pragma GCC warning` requires a string literal"
                }),
                source_vectors: kind.source_vectors,
            });
            return;
        }
        let mut text = ArenaString::new_in(self.scratch);
        for &literal in literals {
            self.append_deprecation_message(literal, &mut text);
        }
        let message = self.context.diagnostic_text(&text);
        self.context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::PragmaMessage { error, message },
            source_vectors: kind.source_vectors,
        });
    }

    /// Macro-replaces the rest of a pragma line and passes each resulting
    /// token to `each`, as GCC and Clang do for the pragmas whose operands
    /// they replace.
    /// C99: §6.10.6p1 and footnote 152, p. 159; PDF p. 171 permit this choice.
    fn expand_pragma_line(
        &mut self,
        source: SourceVectors,
        mut each: impl FnMut(&mut Self, PreprocessorToken),
    ) {
        let mut raw = crate::util::bump::ArenaVec::new_in(self.scratch);
        let line_state = (self.last_was_newline, self.current_is_newline);
        while let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
            if token.kind == PreprocessorTokenType::Newline {
                break;
            }
            raw.push(token);
        }
        // A pragma operator temporarily replaces the tokenizer while its
        // caller's macro frames remain active. Expand only the captured line:
        // its parentheses/newline must never end or resume those frames.
        let location = self.context.first_source_vector_or_default(source);
        let tokenizer = crate::translation_phases::preprocessor_tokenizer::TokenSource::replay(
            self.context,
            self.scratch,
            &[&raw],
            location,
        );
        let hash_hash_stack = std::mem::replace(
            &mut self.hash_hash_stack,
            crate::util::bump::ArenaVec::new_in(self.scratch),
        );
        let generate_placeholders = self.generate_placeholders;
        self.push_tokenizer_frame(super::super::runtime::TokenizerFrame {
            frame_type: super::super::runtime::TokenizerFrameType::Rescan { argument: false },
            tokenizer,
        });
        let depth = self.tokenizer_stack.len();
        let operand_fence = std::mem::replace(&mut self.operand_fence, 0);
        let expansion_fence = std::mem::replace(&mut self.expansion_fence, depth);
        let verbatim_fence = std::mem::replace(&mut self.verbatim_fence, 0);
        while let Some(token) = self.next_preprocessor_token::<true>() {
            each(self, token);
        }
        self.operand_fence = operand_fence;
        self.expansion_fence = expansion_fence;
        self.verbatim_fence = verbatim_fence;
        self.generate_placeholders = generate_placeholders;
        self.hash_hash_stack = hash_hash_stack;
        (self.last_was_newline, self.current_is_newline) = line_state;
    }

    /// The source text a `_Pragma` string literal stands for (C99 §6.10.9p1),
    /// with a final newline, in the translation-unit arena, where diagnostics
    /// can still quote it.
    ///
    /// C99: §6.10.9 paragraph 1, p. 161; PDF p. 173: destringizing drops an
    /// `L` prefix and the quotes, and turns `\"` into `"` and `\\` into `\`.
    pub(in crate::translation_phases::preprocessing) fn prepare_pragma_operator_string<'c>(
        context: &Context<'c>,
        string: StringCacheId,
    ) -> &'c str {
        let string = context.string_cache.at(string);
        // Nothing else is allocated in the arena while the text is written.
        let mut text = context.tu_arena().tail_vec::<u8>();
        // A quote is written only once another character follows it, so the
        // literal's closing quote is never written.
        let mut quote_pending = false;
        let mut write = |c: char| {
            if std::mem::take(&mut quote_pending) {
                text.push(b'"');
            }
            if c == '"' {
                quote_pending = true;
            } else {
                for &byte in c.encode_utf8(&mut [0; 4]).as_bytes() {
                    text.push(byte);
                }
            }
        };
        // Skip the leading quote.
        let mut index = 1;
        if string.char_at(0) == Some('L') {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            match c {
                | '\\' => match string.char_at(index + 1) {
                    | Some('"') => {
                        write('"');
                        index += 2;
                    },
                    | Some('\\') => {
                        write('\\');
                        index += 2;
                    },
                    | _ => {
                        // Only escaped quotes and backslashes are removed by
                        // C99 §6.10.9p1. Preserve other escapes and progress.
                        write('\\');
                        index += 1;
                    },
                },
                | _ => {
                    write(c);
                    index += c.len_utf8();
                },
            }
        }
        // A final pending quote is the trailing quote, which is dropped.
        text.push(b'\n');
        let text = text.into_slice();
        // SAFETY: only complete UTF-8 encodings of characters were written.
        unsafe { std::str::from_utf8_unchecked(text) }
    }

    /// `#pragma GCC system_header`: the rest of the current header is a
    /// system header, as in GCC and Clang. The primary source file never is.
    /// C99: an implementation-defined pragma, §6.10.6 paragraph 1, p. 159;
    /// PDF p. 171.
    fn pragma_system_header(&mut self, source_vectors: SourceVectors, from: u32) {
        if !self.current_is_header() {
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::SystemHeaderPragmaInMainFile,
                source_vectors,
            });
            return;
        }
        let file = self.physical_source_file_index();
        self.context.mark_system_header(file, from);
    }

    /// Clang's implementation-defined macro deprecation pragma.
    /// C99: §6.10.6p1, p. 159; PDF p. 171.
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#deprecating-macros>
    fn pragma_deprecated_macro(&mut self) -> bool {
        let Some(kind) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context) else {
            return false;
        };
        if !kind.kind.is_identifier() || self.context.string_cache.at(kind.contents) != "deprecated"
        {
            return kind.kind == PreprocessorTokenType::Newline;
        }
        if let Err(consumed_newline) = self.deprecated_pragma_token(
            |_, token| token.kind == PreprocessorTokenType::OpeningParenthesis,
            "expected ( in #pragma clang deprecated",
        ) {
            return consumed_newline;
        }
        let name = match self.deprecated_pragma_token(
            |_, token| token.kind.is_identifier(),
            "expected macro name in #pragma clang deprecated",
        ) {
            | Ok(name) => name,
            | Err(consumed_newline) => return consumed_newline,
        };
        let name_id = name.identifier_id(self.context);
        if !self.state.macro_definitions.contains_key(&name_id) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LanguageConstraint(
                    self.context.diagnostic_format(format_args!(
                        "no macro named `{}`",
                        self.context.string_cache.at(name.contents),
                    )),
                ),
                source_vectors: name.source_vectors,
            });
            return false;
        }
        let mut closing = match self.deprecated_pragma_token(
            |_, token| {
                matches!(
                    token.kind,
                    PreprocessorTokenType::Comma | PreprocessorTokenType::ClosingParenthesis
                )
            },
            "expected ) in #pragma clang deprecated",
        ) {
            | Ok(token) => token,
            | Err(consumed_newline) => return consumed_newline,
        };
        let mut message = None;
        if closing.kind == PreprocessorTokenType::Comma {
            let mut text = ArenaString::new_in(self.scratch);
            let mut literal = match self.deprecated_pragma_token(
                Self::is_deprecation_message_literal,
                "expected string literal in #pragma clang deprecated",
            ) {
                | Ok(token) => token,
                | Err(consumed_newline) => return consumed_newline,
            };
            loop {
                self.append_deprecation_message(literal, &mut text);
                let next = match self.deprecated_pragma_token(
                    |this, token| {
                        this.is_deprecation_message_literal(token)
                            || token.kind == PreprocessorTokenType::ClosingParenthesis
                    },
                    "expected ) in #pragma clang deprecated",
                ) {
                    | Ok(token) => token,
                    | Err(consumed_newline) => return consumed_newline,
                };
                if next.kind == PreprocessorTokenType::ClosingParenthesis {
                    closing = next;
                    break;
                }
                literal = next;
            }
            message = Some(&*self.state.arena.alloc_str(&text));
        }
        _ = self.state.deprecated_macros.insert(
            name_id,
            MacroDeprecation {
                message,
                location: self
                    .context
                    .first_source_vector(closing.source_vectors)
                    .clone(),
            },
        );
        false
    }

    /// A pragma operand, with recovery that preserves the following line.
    /// C99: implementation-defined pragma grammar, §6.10.6p1, p. 159;
    /// PDF p. 171. Unknown Clang pragmas are still ignored by the caller.
    fn deprecated_pragma_token(
        &mut self,
        accepts: impl Fn(&Self, PreprocessorToken) -> bool,
        message: &'static str,
    ) -> Result<PreprocessorToken, bool> {
        let token = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
        if let Some(token) = token
            && accepts(self, token)
        {
            return Ok(token);
        }
        let source_vectors =
            token.map_or_else(|| self.current_location(), |token| token.source_vectors);
        self.context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::LanguageConstraint(message),
            source_vectors,
        });
        Err(token.is_none_or(|token| token.kind == PreprocessorTokenType::Newline))
    }

    /// Clang permits only ordinary, unexpanded string literals in messages.
    /// C99: §6.4.5p1, p. 62; PDF p. 74, as an implementation-defined
    /// pragma operand (§6.10.6p1, p. 159; PDF p. 171).
    fn is_deprecation_message_literal(&self, token: PreprocessorToken) -> bool {
        token.kind == PreprocessorTokenType::String
            && self
                .context
                .string_cache
                .at(token.contents)
                .starts_with('"')
    }

    /// Decode and concatenate message literals, escaping non-printing bytes
    /// as Clang does so diagnostics never carry a raw NUL. This pragma uses
    /// ordinary phase-5 decoding and phase-6 concatenation rules.
    /// C99: §6.4.5p4, p. 62; PDF p. 74; implementation-defined pragma
    /// behavior §6.10.6p1, p. 159; PDF p. 171.
    fn append_deprecation_message(&mut self, token: PreprocessorToken, text: &mut ArenaString<'_>) {
        let Some(token) = self.map_preprocessor_token(token) else {
            return;
        };
        let TokenType::String(StringTokenType::String(contents)) = token.kind else {
            return;
        };
        let append = |text: &mut ArenaString<'_>, character: char| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                let _ = write!(text, "<U+{:04X}>", u32::from(character));
            } else {
                text.push(character);
            }
        };
        if let Some(decoded) = self.context.literal_text_in(self.scratch, contents, false) {
            for character in decoded.chars() {
                append(text, character);
            }
        } else {
            // Non-UTF-8 execution bytes are still valid message contents.
            for unit in self.context.literal_units(contents) {
                match *unit {
                    | super::LiteralUnit::Character(character) => append(text, character),
                    | super::LiteralUnit::Numeric(value) if value < 128 => {
                        if let Some(character) = char::from_u32(value) {
                            append(text, character);
                        }
                    },
                    | super::LiteralUnit::Numeric(value) => {
                        let _ = write!(text, "<{value:02X}>");
                    },
                }
            }
        }
    }
}
