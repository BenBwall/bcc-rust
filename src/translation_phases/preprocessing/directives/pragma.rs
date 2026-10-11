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
    /// whose operands are not macro-replaced. Returns whether the directive's
    /// new-line was consumed.
    ///
    /// C99: §6.10.6 paragraphs 1-2, p. 159; PDF p. 171. `STDC` pragmas must
    /// name `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE` and an
    /// `on-off-switch`; they are checked but have no effect yet. `#pragma
    /// once` and `#pragma GCC system_header` are bcc's implementation-defined
    /// pragmas. Other pragmas are ignored (paragraph 1), and one that does
    /// not begin with an identifier draws a warning first.
    /// `from` is the physical invocation's byte offset, including when
    /// `_Pragma` supplies tokens from a synthetic string (C99 §6.10.9p1,
    /// p. 161; PDF p. 173).
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
                            if operand.kind.is_identifier()
                                && self.context.string_cache.at(operand.contents) == "system_header"
                            {
                                self.pragma_system_header(operand.source_vectors, from);
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
                                        self.context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                    self.context.diagnostic_text(s),
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
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
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                        source_vectors: token.source_vectors,
                    });
                    break 'base;
                },
            }
        }
        consumed_newline
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
