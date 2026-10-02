//! Conversion of preprocessing tokens into parser tokens.

use super::{
    Preprocessor,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    token::{
        CharacterTokenType,
        FloatTokenType,
        IntegerSuffix,
        IntegerTokenType,
        KeywordTokenType,
        OperatorTokenType,
        SignedIntegerLiteralType,
        StringTokenType,
        Token,
        TokenType,
        UnsignedIntegerLiteralType,
    },
};
use crate::{
    float_parsing::{
        ParseFloatError,
        string_to_double,
        string_to_float,
        string_to_long_double,
    },
    translation_phases::{
        Context,
        StrExt,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
};

impl Preprocessor {
    pub(super) fn concatenate_adjacent_strings(
        &mut self,
        context: &mut Context,
        first: Token,
    ) -> Token {
        let TokenType::String(first_kind) = first.kind else {
            return first;
        };
        let mut wide = matches!(first_kind, StringTokenType::WideString(_));
        let first_contents = match first_kind {
            | StringTokenType::String(contents) | StringTokenType::WideString(contents) => contents,
        };
        let mut contents = context.string_cache.at(first_contents).to_string();
        let mut sources = vec![first.source_vectors];

        loop {
            let existing_errors = context.take_pending_errors();
            let next = self.next_parser_token(context);
            let generated_errors = context.take_pending_errors();

            let Some(next) = next else {
                context.append_pending_errors(existing_errors);
                self.pending_parser_errors.extend(generated_errors);
                break;
            };
            let TokenType::String(next_kind) = next.kind else {
                context.append_pending_errors(existing_errors);
                self.pending_parser_token = Some(next);
                self.pending_parser_errors.extend(generated_errors);
                break;
            };
            context.append_pending_errors(existing_errors);
            context.append_pending_errors(generated_errors);
            let next_contents = match next_kind {
                | StringTokenType::String(contents) => contents,
                | StringTokenType::WideString(contents) => {
                    wide = true;
                    contents
                },
            };
            contents.push_str(context.string_cache.at(next_contents));
            sources.push(next.source_vectors);
        }

        let contents = context.string_cache.intern(&contents);
        Token {
            kind: if wide {
                TokenType::String(StringTokenType::WideString(contents))
            } else {
                TokenType::String(StringTokenType::String(contents))
            },
            contents,
            source_vectors: context.merge_vector_list(&sources),
        }
    }

    #[inline(always)]
    fn parse_integer_radix(
        &mut self,
        context: &mut Context,
        radix: u32,
        start_index: usize,
        invalid_integer_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
    ) -> Token {
        _ = self;
        let mut index = start_index;
        let (result, did_overflow) = {
            let contents = context.string_cache.at(token.contents);
            let mut result = 0u64;
            let mut did_overflow = false;
            while let Some(digit) = contents.char_at(index).and_then(|c| c.to_digit(radix)) {
                let (new_result, overflow) = result.overflowing_mul(u64::from(radix));
                if overflow {
                    did_overflow = true;
                }
                result = new_result;
                let (new_result, overflow) = result.overflowing_add(u64::from(digit));
                if overflow {
                    did_overflow = true;
                }
                result = new_result;
                index += 1;
            }
            (result, did_overflow)
        };
        if did_overflow {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::IntegerLiteralOverflow,
                source_vectors: token.source_vectors,
            });
        }
        let contents = context.string_cache.at(token.contents);
        let suffix_type = match (
            contents.char_at(index),
            contents.char_at(index + 1),
            contents.char_at(index + 2),
        ) {
            | (Some('u'), Some('l'), Some('l'))
            | (Some('U'), Some('L'), Some('L'))
            | (Some('l'), Some('l'), Some('u'))
            | (Some('L'), Some('L'), Some('U')) => {
                index += 3;
                Some(IntegerSuffix::UnsignedLongLong)
            },
            | (Some('u'), Some('l'), _)
            | (Some('U'), Some('L'), _)
            | (Some('l'), Some('u'), _)
            | (Some('L'), Some('U'), _) => {
                index += 2;
                Some(IntegerSuffix::UnsignedLong)
            },
            | (Some('u' | 'U'), _, _) => {
                index += 1;
                Some(IntegerSuffix::Unsigned)
            },
            | (Some('l'), Some('l'), _) | (Some('L'), Some('L'), _) => {
                index += 2;
                Some(IntegerSuffix::LongLong)
            },
            | (Some('l' | 'L'), _, _) => {
                index += 1;
                Some(IntegerSuffix::Long)
            },
            | _ => None,
        };
        if index != contents.len() - 1 {
            context.preprocessor_error(PreprocessorError {
                error_type:     invalid_integer_literal_error,
                source_vectors: token.source_vectors,
            });
        }
        match suffix_type {
            | Some(IntegerSuffix::UnsignedLongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::LongLong) if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::LongLong,
                        to:   UnsignedIntegerLiteralType::UnsignedLongLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::LongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::LongLong(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::UnsignedLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::Long) if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::Long,
                        to:   UnsignedIntegerLiteralType::UnsignedLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::Long) => Token {
                kind:           TokenType::Integer(IntegerTokenType::Long(result as _)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::Unsigned) if result > u64::from(u32::MAX) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedUnsignedPromotion {
                        from: UnsignedIntegerLiteralType::UnsignedInt,
                        to:   UnsignedIntegerLiteralType::UnsignedLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            #[expect(
                clippy::cast_possible_truncation,
                reason = "At this point we know that result definitely fits into a u32."
            )]
            | Some(IntegerSuffix::Unsigned) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedInt(result as u32)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | None if result > i64::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                        from: SignedIntegerLiteralType::Int,
                        to:   UnsignedIntegerLiteralType::UnsignedLongLong,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(result)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | None if result > i32::MAX as u64 => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ForcedSignedPromotion {
                        from: SignedIntegerLiteralType::Int,
                        to:   SignedIntegerLiteralType::Long,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Integer(IntegerTokenType::Long(result as i64)),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            #[expect(
                clippy::cast_possible_truncation,
                reason = "We know result fits into an i32 at this point."
            )]
            | None => Token {
                kind:           TokenType::Integer(IntegerTokenType::Int(result as i32)),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
        }
    }

    fn parse_hexadecimal_integer(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_integer_radix(
            context,
            16,
            2,
            PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            token,
        )
    }

    fn parse_binary_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            2,
            2,
            PreprocessorErrorType::InvalidBinaryIntegerLiteral,
            token,
        )
    }

    fn parse_octal_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            8,
            1,
            PreprocessorErrorType::InvalidOctalIntegerLiteral,
            token,
        )
    }

    fn parse_decimal_integer(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_integer_radix(
            context,
            10,
            0,
            PreprocessorErrorType::InvalidDecimalIntegerLiteral,
            token,
        )
    }

    #[inline(always)]
    fn parse_float(
        &mut self,
        context: &mut Context,
        invalid_float_literal_error: PreprocessorErrorType,
        token: PreprocessorToken,
    ) -> Token {
        _ = self;
        let contents = context.string_cache.at(token.contents);

        let res = match contents.char_at(contents.len() - 2) {
            | Some('f' | 'F') => string_to_float(contents).map(FloatTokenType::Float),
            | Some('l' | 'L') => string_to_long_double(contents).map(FloatTokenType::LongDouble),
            | _ => string_to_double(contents).map(FloatTokenType::Double),
        };
        match res {
            | Ok(kind) => Token {
                kind:           TokenType::Float(kind),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Err(ParseFloatError::Invalid(kind)) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     invalid_float_literal_error,
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Float(kind),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Err(ParseFloatError::OutOfRange(kind, error)) => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::FloatConstantOutOfRange {
                        type_name: kind.type_name(),
                        error,
                    },
                    source_vectors: token.source_vectors,
                });
                Token {
                    kind:           TokenType::Float(kind),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
        }
    }

    fn parse_hexadecimal_float(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidHexadecimalFloatLiteral,
            token,
        )
    }

    fn parse_decimal_float(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidDecimalFloatLiteral,
            token,
        )
    }

    fn eval_escape_sequences(&mut self, context: &mut Context, token: PreprocessorToken) -> String {
        _ = self;
        let mut ret = String::new();
        let mut index = 0;
        let string = context.string_cache.at(token.contents);
        if let Some('L') = string.char_at(0) {
            index += 1;
        }
        if let Some('"' | '\'') = string.char_at(index) {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            if c == '\\' {
                let Some(c) = string.char_at(index + 1) else {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnterminatedEscapeSequence,
                        source_vectors: token.source_vectors,
                    });
                    return ret;
                };
                index += 2;
                ret.push(match c {
                    | 'a' => '\x07',
                    | 'b' => '\x08',
                    | 'f' => '\x0C',
                    | 'n' => '\n',
                    | 'r' => '\r',
                    | 't' => '\t',
                    | 'v' => '\x0B',
                    | '\'' => '\'',
                    | '"' => '"',
                    | '?' => '?',
                    | '\\' => '\\',
                    | 'x' => {
                        let code_point = (|| {
                            let mut code_point = 0u32;
                            while let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) {
                                index += 1;
                                code_point = code_point.checked_mul(16)?;
                                code_point = code_point.checked_add(d)?;
                            }
                            Some(code_point)
                        })();
                        let Some(code_point) = code_point else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::HexEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:     PreprocessorErrorType::InvalidHexEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | '0'..='7' => {
                        #[expect(clippy::cast_possible_truncation, reason = "We are checking that d is range before casting to a u16.")]
                        let code_point = (|| {
                            let mut code_point = c as u16 - '0' as u16;
                            for _ in 0..2 {
                                let Some(d) = string.char_at(index).and_then(|c| c.to_digit(8))
                                else {
                                    break;
                                };
                                index += 1;
                                code_point = code_point.checked_mul(8)?;

                                code_point = code_point.checked_add(d as u16)?;
                            }
                            Some(code_point)
                        })();

                        let Some(code_point) = code_point else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::OctalEscapeSequenceTooLarge,
                                    source_vectors: token.source_vectors,
                                },
                            );

                            continue;
                        };
                        let Ok(c) = char::try_from(u32::from(code_point)) else {
                            Context::raw_preprocessor_error(
                                &mut context.pending_errors,
                                PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidOctalEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | 'u' => {
                        let mut code_point = 0u32;
                        for _ in 0..4 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) else {
                                Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort,
                                    source_vectors: token.source_vectors,
                                });
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | 'U' => {
                        let mut code_point = 0u32;
                        for _ in 0..8 {
                            let Some(d) = string.char_at(index).and_then(|c| c.to_digit(16)) else {
                                Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall,
                                    source_vectors: token.source_vectors,
                                });
                                break;
                            };
                            index += 1;
                            code_point *= 16;
                            code_point += d;
                        }
                        let Ok(c) = char::try_from(code_point) else {
                            Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue;
                        };
                        c
                    },
                    | _ => {
                        Context::raw_preprocessor_error(&mut context.pending_errors,PreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidEscapeSequence,
                                source_vectors: token.source_vectors,
                            },
                        );
                        continue;
                    },
                });
            } else {
                ret.push(c);
                index += c.len_utf8();
            }
        }
        if ret.ends_with('"') || ret.ends_with('\'') {
            _ = ret.pop();
        }
        ret
    }

    fn build_token(token: PreprocessorToken, kind: TokenType) -> Token {
        Token {
            kind,
            contents: token.contents,
            source_vectors: token.source_vectors,
        }
    }

    fn build_operator_token(token: PreprocessorToken, kind: OperatorTokenType) -> Token {
        Self::build_token(token, TokenType::Operator(kind))
    }

    pub(super) fn parse_number(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Token {
        let contents = context.string_cache.at(token.contents);
        let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
        let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
        let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
        if is_hex {
            if contents.contains(['.', 'p', 'P']) {
                self.parse_hexadecimal_float(context, token)
            } else {
                self.parse_hexadecimal_integer(context, token)
            }
        } else if is_binary {
            self.parse_binary_integer(context, token)
        } else if contents.contains(['.', 'e', 'E']) {
            self.parse_decimal_float(context, token)
        } else if is_octal {
            self.parse_octal_integer(context, token)
        } else {
            self.parse_decimal_integer(context, token)
        }
    }

    fn parse_string(&mut self, context: &mut Context, token: PreprocessorToken) -> StringTokenType {
        let contents = self.eval_escape_sequences(context, token);
        let cached_contents = context.string_cache.intern(&contents);
        if context.string_cache.at(token.contents).starts_with('L') {
            StringTokenType::WideString(cached_contents)
        } else {
            StringTokenType::String(cached_contents)
        }
    }

    pub(super) fn parse_character(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> CharacterTokenType {
        let contents = self.eval_escape_sequences(context, token);
        let wide = context.string_cache.at(token.contents).starts_with('L');
        if contents.chars().take(2).count() != 1 {
            if !wide && !contents.is_empty() {
                // C99 6.4.4.4 leaves this value implementation-defined. Pack
                // UTF-8 execution bytes most-significant first into an int,
                // keeping the final four bytes when the spelling is longer.
                let value = contents
                    .bytes()
                    .fold(0_i32, |value, byte| value.wrapping_shl(8) | i32::from(byte));
                return CharacterTokenType::MultiChar(value);
            }
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                source_vectors: token.source_vectors,
            });
        }
        let char = contents.chars().next().unwrap_or('\0');
        if wide {
            CharacterTokenType::WideChar(char)
        } else {
            CharacterTokenType::Char(char)
        }
    }

    pub(super) fn map_preprocessor_token(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Option<Token> {
        Some(match token.kind {
            | PreprocessorTokenType::Number => self.parse_number(context, token),
            | PreprocessorTokenType::Newline => return None,
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    unreachable!("Handled in next_preprocessor_token");
                }
                self.parse_directive(context, token);
                return None;
            },
            | PreprocessorTokenType::GeneratedString => Token {
                kind:           TokenType::String(StringTokenType::String(token.contents)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::WideGeneratedString => {
                // Discard the L prefix.
                let contents = &context.string_cache.at(token.contents)[1..].to_token_string();
                let contents = context.string_cache.intern(contents);
                Token {
                    kind: TokenType::String(StringTokenType::WideString(contents)),
                    contents,
                    source_vectors: token.source_vectors,
                }
            },
            | PreprocessorTokenType::String => Token {
                kind:           TokenType::String(self.parse_string(context, token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Character => Token {
                kind:           TokenType::Character(self.parse_character(context, token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined =>
                Self::build_token(
                    token,
                    match context.string_cache.at(token.contents) {
                        | "auto" => TokenType::Keyword(KeywordTokenType::Auto),
                        | "break" => TokenType::Keyword(KeywordTokenType::Break),
                        | "case" => TokenType::Keyword(KeywordTokenType::Case),
                        | "char" => TokenType::Keyword(KeywordTokenType::Char),
                        | "const" => TokenType::Keyword(KeywordTokenType::Const),
                        | "continue" => TokenType::Keyword(KeywordTokenType::Continue),
                        | "default" => TokenType::Keyword(KeywordTokenType::Default),
                        | "do" => TokenType::Keyword(KeywordTokenType::Do),
                        | "double" => TokenType::Keyword(KeywordTokenType::Double),
                        | "else" => TokenType::Keyword(KeywordTokenType::Else),
                        | "enum" => TokenType::Keyword(KeywordTokenType::Enum),
                        | "extern" => TokenType::Keyword(KeywordTokenType::Extern),
                        | "float" => TokenType::Keyword(KeywordTokenType::Float),
                        | "for" => TokenType::Keyword(KeywordTokenType::For),
                        | "goto" => TokenType::Keyword(KeywordTokenType::Goto),
                        | "if" => TokenType::Keyword(KeywordTokenType::If),
                        | "inline" => TokenType::Keyword(KeywordTokenType::Inline),
                        | "int" => TokenType::Keyword(KeywordTokenType::Int),
                        | "long" => TokenType::Keyword(KeywordTokenType::Long),
                        | "register" => TokenType::Keyword(KeywordTokenType::Register),
                        | "restrict" => TokenType::Keyword(KeywordTokenType::Restrict),
                        | "return" => TokenType::Keyword(KeywordTokenType::Return),
                        | "short" => TokenType::Keyword(KeywordTokenType::Short),
                        | "signed" => TokenType::Keyword(KeywordTokenType::Signed),
                        | "sizeof" => TokenType::Keyword(KeywordTokenType::Sizeof),
                        | "static" => TokenType::Keyword(KeywordTokenType::Static),
                        | "struct" => TokenType::Keyword(KeywordTokenType::Struct),
                        | "switch" => TokenType::Keyword(KeywordTokenType::Switch),
                        | "typedef" => TokenType::Keyword(KeywordTokenType::Typedef),
                        | "union" => TokenType::Keyword(KeywordTokenType::Union),
                        | "unsigned" => TokenType::Keyword(KeywordTokenType::Unsigned),
                        | "void" => TokenType::Keyword(KeywordTokenType::Void),
                        | "volatile" => TokenType::Keyword(KeywordTokenType::Volatile),
                        | "while" => TokenType::Keyword(KeywordTokenType::While),
                        | "_Bool" => TokenType::Keyword(KeywordTokenType::Bool),
                        | "_Complex" => TokenType::Keyword(KeywordTokenType::Complex),
                        | "_Imaginary" => TokenType::Keyword(KeywordTokenType::Imaginary),
                        | _ => TokenType::Identifier,
                    },
                ),
            | PreprocessorTokenType::Plus =>
                Self::build_operator_token(token, OperatorTokenType::Plus),
            | PreprocessorTokenType::Minus =>
                Self::build_operator_token(token, OperatorTokenType::Minus),
            | PreprocessorTokenType::Asterisk =>
                Self::build_operator_token(token, OperatorTokenType::Asterisk),
            | PreprocessorTokenType::ForwardSlash =>
                Self::build_operator_token(token, OperatorTokenType::ForwardSlash),
            | PreprocessorTokenType::Percent =>
                Self::build_operator_token(token, OperatorTokenType::Percent),
            | PreprocessorTokenType::LessThanLessThan =>
                Self::build_operator_token(token, OperatorTokenType::LessThanLessThan),
            | PreprocessorTokenType::GreaterThanGreaterThan =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanGreaterThan),
            | PreprocessorTokenType::LessThan =>
                Self::build_operator_token(token, OperatorTokenType::LessThan),
            | PreprocessorTokenType::LessThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::LessThanEquals),
            | PreprocessorTokenType::GreaterThan =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThan),
            | PreprocessorTokenType::GreaterThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanEquals),
            | PreprocessorTokenType::EqualsEquals =>
                Self::build_operator_token(token, OperatorTokenType::EqualsEquals),
            | PreprocessorTokenType::ExclamationMarkEquals =>
                Self::build_operator_token(token, OperatorTokenType::ExclamationMarkEquals),
            | PreprocessorTokenType::Ampersand =>
                Self::build_operator_token(token, OperatorTokenType::Ampersand),
            | PreprocessorTokenType::Caret =>
                Self::build_operator_token(token, OperatorTokenType::Caret),
            | PreprocessorTokenType::Pipe =>
                Self::build_operator_token(token, OperatorTokenType::Pipe),
            | PreprocessorTokenType::AmpersandAmpersand =>
                Self::build_operator_token(token, OperatorTokenType::AmpersandAmpersand),
            | PreprocessorTokenType::PipePipe =>
                Self::build_operator_token(token, OperatorTokenType::PipePipe),
            | PreprocessorTokenType::QuestionMark =>
                Self::build_operator_token(token, OperatorTokenType::QuestionMark),
            | PreprocessorTokenType::Colon =>
                Self::build_operator_token(token, OperatorTokenType::Colon),
            | PreprocessorTokenType::SemiColon =>
                Self::build_operator_token(token, OperatorTokenType::Semicolon),
            | PreprocessorTokenType::OpeningParenthesis =>
                Self::build_operator_token(token, OperatorTokenType::OpeningParenthesis),
            | PreprocessorTokenType::ClosingParenthesis =>
                Self::build_operator_token(token, OperatorTokenType::ClosingParenthesis),
            | PreprocessorTokenType::OpeningSquareBracket =>
                Self::build_operator_token(token, OperatorTokenType::OpeningSquareBracket),
            | PreprocessorTokenType::ClosingSquareBracket =>
                Self::build_operator_token(token, OperatorTokenType::ClosingSquareBracket),
            | PreprocessorTokenType::OpeningCurlyBrace =>
                Self::build_operator_token(token, OperatorTokenType::OpeningCurlyBrace),
            | PreprocessorTokenType::ClosingCurlyBrace =>
                Self::build_operator_token(token, OperatorTokenType::ClosingCurlyBrace),
            | PreprocessorTokenType::Period =>
                Self::build_operator_token(token, OperatorTokenType::Period),
            | PreprocessorTokenType::Arrow =>
                Self::build_operator_token(token, OperatorTokenType::Arrow),
            | PreprocessorTokenType::PlusPlus =>
                Self::build_operator_token(token, OperatorTokenType::PlusPlus),
            | PreprocessorTokenType::MinusMinus =>
                Self::build_operator_token(token, OperatorTokenType::MinusMinus),
            | PreprocessorTokenType::AsteriskEquals =>
                Self::build_operator_token(token, OperatorTokenType::AsteriskEquals),
            | PreprocessorTokenType::ForwardSlashEquals =>
                Self::build_operator_token(token, OperatorTokenType::ForwardSlashEquals),
            | PreprocessorTokenType::PercentEquals =>
                Self::build_operator_token(token, OperatorTokenType::PercentEquals),
            | PreprocessorTokenType::PlusEquals =>
                Self::build_operator_token(token, OperatorTokenType::PlusEquals),
            | PreprocessorTokenType::MinusEquals =>
                Self::build_operator_token(token, OperatorTokenType::MinusEquals),
            | PreprocessorTokenType::LessThanLessThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::LessThanLessThanEquals),
            | PreprocessorTokenType::GreaterThanGreaterThanEquals =>
                Self::build_operator_token(token, OperatorTokenType::GreaterThanGreaterThanEquals),
            | PreprocessorTokenType::AmpersandEquals =>
                Self::build_operator_token(token, OperatorTokenType::AmpersandEquals),
            | PreprocessorTokenType::CaretEquals =>
                Self::build_operator_token(token, OperatorTokenType::CaretEquals),
            | PreprocessorTokenType::PipeEquals =>
                Self::build_operator_token(token, OperatorTokenType::PipeEquals),
            | PreprocessorTokenType::Equals =>
                Self::build_operator_token(token, OperatorTokenType::Equals),
            | PreprocessorTokenType::Comma =>
                Self::build_operator_token(token, OperatorTokenType::Comma),
            | PreprocessorTokenType::Tilde =>
                Self::build_operator_token(token, OperatorTokenType::Tilde),
            | PreprocessorTokenType::ExclamationMark =>
                Self::build_operator_token(token, OperatorTokenType::ExclamationMark),
            | PreprocessorTokenType::Ellipsis =>
                Self::build_operator_token(token, OperatorTokenType::Ellipsis),
            | PreprocessorTokenType::HashHash => {
                let error_type = if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroArgument { .. }
                            | TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                            | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    PreprocessorErrorType::CannotUseHashHashAfterFunctionLikeMacroCall
                } else {
                    PreprocessorErrorType::HashHashUsedOutsideOfMacro
                };
                context.preprocessor_error(PreprocessorError {
                    error_type,
                    source_vectors: token.source_vectors,
                });
                return None;
            },

            | PreprocessorTokenType::Placeholder
            | PreprocessorTokenType::AngleBracketString
            | PreprocessorTokenType::IncludeString
            | PreprocessorTokenType::Whitespace => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedTokenAtPhase7(token.kind),
                    source_vectors: token.source_vectors,
                });
                return None;
            },
        })
    }
}
