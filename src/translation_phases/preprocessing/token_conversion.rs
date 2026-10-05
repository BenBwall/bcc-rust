//! Conversion of preprocessing tokens into parser tokens.

use super::{
    Expander,
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
        LiteralUnit,
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
        SourceVectors,
        StrExt,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            PreprocessorTokenizerError,
        },
    },
    util::{
        bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
        packed::Packed,
    },
};

/// Storage that string-literal conversion reuses, kept with the
/// preprocessor's long-lived state so that converting a literal leaves
/// nothing behind.
pub(super) struct LiteralScratch<'pp> {
    arena:   &'pp Bump,
    /// The units of the literal being decoded.
    units:   ArenaVec<'pp, LiteralUnit>,
    /// A spare builder for adjacent-literal concatenation.
    builder: Option<LiteralBuilder<'pp>>,
}

/// One concatenation of adjacent string literals in progress.
pub(super) struct LiteralBuilder<'pp> {
    units:    ArenaVec<'pp, LiteralUnit>,
    sources:  ArenaVec<'pp, SourceVectors>,
    spelling: ArenaString<'pp>,
}

impl<'pp> LiteralScratch<'pp> {
    pub(super) fn new(arena: &'pp Bump) -> Self {
        Self {
            arena,
            units: ArenaVec::new_in(arena),
            builder: None,
        }
    }

    /// The empty unit storage, which a nested conversion would not share.
    fn take_units(&mut self) -> ArenaVec<'pp, LiteralUnit> {
        std::mem::replace(&mut self.units, ArenaVec::new_in(self.arena))
    }

    fn return_units(&mut self, mut units: ArenaVec<'pp, LiteralUnit>) {
        units.clear();
        self.units = units;
    }

    fn take_builder(&mut self) -> LiteralBuilder<'pp> {
        self.builder.take().unwrap_or_else(|| LiteralBuilder {
            units:    ArenaVec::new_in(self.arena),
            sources:  ArenaVec::new_in(self.arena),
            spelling: ArenaString::new_in(self.arena),
        })
    }

    fn return_builder(&mut self, mut builder: LiteralBuilder<'pp>) {
        builder.units.clear();
        builder.sources.clear();
        builder.spelling.clear();
        self.builder = Some(builder);
    }
}

impl<'tu, 'pp> Expander<'tu, 'pp, '_> {
    pub(super) fn concatenate_adjacent_strings(
        &mut self,
        context: &mut Context<'tu>,
        first: Token,
    ) -> Token {
        let TokenType::String(first_kind) = first.kind else {
            return first;
        };
        let mut wide = matches!(first_kind, StringTokenType::WideString(_));
        let first_contents = match first_kind {
            | StringTokenType::String(contents) | StringTokenType::WideString(contents) => contents,
        };
        // Most literals stand alone: fill the reused builders only after
        // adjacency.
        let mut builder: Option<LiteralBuilder<'pp>> = None;

        loop {
            // Diagnostics reported while reading the next token stay after
            // the ones already pending.
            let existing_errors = context.pending_error_count();
            let next = self.next_parser_token(context);

            if context.source_segment_count() > self.source_segment_limit {
                self.pending_parser_token = next;
                self.pending_parser_errors
                    .extend(context.split_off_pending_errors(existing_errors));
                break;
            }

            let Some(next) = next else {
                self.pending_parser_errors
                    .extend(context.split_off_pending_errors(existing_errors));
                break;
            };
            let TokenType::String(next_kind) = next.kind else {
                self.pending_parser_token = Some(next);
                self.pending_parser_errors
                    .extend(context.split_off_pending_errors(existing_errors));
                break;
            };
            let next_contents = match next_kind {
                | StringTokenType::String(contents) => contents,
                | StringTokenType::WideString(contents) => {
                    wide = true;
                    contents
                },
            };
            let builder = builder.get_or_insert_with(|| {
                let mut builder = self.state.literal_scratch.take_builder();
                builder
                    .units
                    .extend_from_slice(context.literal_units(first_contents));
                builder.sources.push(first.source_vectors);
                builder
                    .spelling
                    .push_str(context.string_cache.at(first.contents));
                builder
            });
            builder
                .units
                .extend_from_slice(context.literal_units(next_contents));
            builder.sources.push(next.source_vectors);
            builder.spelling.push(' ');
            builder
                .spelling
                .push_str(context.string_cache.at(next.contents));
        }

        let Some(builder) = builder else {
            return first;
        };
        let contents = context.intern_literal(&builder.units);
        let token = Token {
            kind:           if wide {
                TokenType::String(StringTokenType::WideString(contents))
            } else {
                TokenType::String(StringTokenType::String(contents))
            },
            contents:       context.string_cache.intern(&*builder.spelling),
            source_vectors: context.merge_vector_list(&builder.sources),
        };
        self.state.literal_scratch.return_builder(builder);
        token
    }

    #[inline(always)]
    fn parse_integer_radix<'ctx>(
        &mut self,
        context: &mut Context<'ctx>,
        radix: u32,
        start_index: usize,
        invalid_integer_literal_error: PreprocessorErrorType<'ctx>,
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
        let missing_digits = radix == 16 && index == start_index;
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
            | (Some('u' | 'U'), Some('l'), Some('l'))
            | (Some('u' | 'U'), Some('L'), Some('L'))
            | (Some('l'), Some('l'), Some('u' | 'U'))
            | (Some('L'), Some('L'), Some('u' | 'U')) => {
                index += 3;
                Some(IntegerSuffix::UnsignedLongLong)
            },
            | (Some('u' | 'U'), Some('l' | 'L'), _) | (Some('l' | 'L'), Some('u' | 'U'), _) => {
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
        if missing_digits || index != contents.len() - 1 {
            context.preprocessor_error(PreprocessorError {
                error_type:     invalid_integer_literal_error,
                source_vectors: token.source_vectors,
            });
        }
        match suffix_type {
            | Some(IntegerSuffix::UnsignedLongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(
                    Packed::new(result),
                )),
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
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(
                        Packed::new(result),
                    )),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::LongLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::LongLong(Packed::new(
                    result as _,
                ))),
                source_vectors: token.source_vectors,
                contents:       token.contents,
            },
            | Some(IntegerSuffix::UnsignedLong) => Token {
                kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(Packed::new(
                    result,
                ))),
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
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(
                        Packed::new(result),
                    )),
                    source_vectors: token.source_vectors,
                    contents:       token.contents,
                }
            },
            | Some(IntegerSuffix::Long) => Token {
                kind:           TokenType::Integer(IntegerTokenType::Long(Packed::new(
                    result as _,
                ))),
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
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLong(
                        Packed::new(result),
                    )),
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
                    kind:           TokenType::Integer(IntegerTokenType::UnsignedLongLong(
                        Packed::new(result),
                    )),
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
                    kind:           TokenType::Integer(IntegerTokenType::Long(Packed::new(
                        result as i64,
                    ))),
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
        context: &mut Context<'_>,
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

    fn parse_binary_integer(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_integer_radix(
            context,
            2,
            2,
            PreprocessorErrorType::InvalidBinaryIntegerLiteral,
            token,
        )
    }

    fn parse_octal_integer(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_integer_radix(
            context,
            8,
            1,
            PreprocessorErrorType::InvalidOctalIntegerLiteral,
            token,
        )
    }

    fn parse_decimal_integer(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_integer_radix(
            context,
            10,
            0,
            PreprocessorErrorType::InvalidDecimalIntegerLiteral,
            token,
        )
    }

    #[inline(always)]
    fn parse_float<'ctx>(
        &mut self,
        context: &mut Context<'ctx>,
        invalid_float_literal_error: PreprocessorErrorType<'ctx>,
        token: PreprocessorToken,
    ) -> Token {
        _ = self;
        let contents = context.string_cache.at(token.contents);

        let res = match contents.char_at(contents.len() - 2) {
            | Some('f' | 'F') => string_to_float(contents).map(FloatTokenType::Float),
            | Some('l' | 'L') => string_to_long_double(contents).map(FloatTokenType::LongDouble),
            | _ =>
                string_to_double(contents).map(|value| FloatTokenType::Double(Packed::new(value))),
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
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidHexadecimalFloatLiteral,
            token,
        )
    }

    fn parse_decimal_float(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Token {
        self.parse_float(
            context,
            PreprocessorErrorType::InvalidDecimalFloatLiteral,
            token,
        )
    }

    /// Appends the units `token` spells to `units`, which start empty, and
    /// returns whether an escape sequence was invalid.
    fn eval_escape_sequences(
        context: &mut Context<'_>,
        token: PreprocessorToken,
        units: &mut ArenaVec<'_, LiteralUnit>,
    ) -> bool {
        let string = context.string_cache.at(token.contents);
        let wide = string.starts_with('L');
        let mut index = usize::from(wide) + 1;
        let quote = string.as_bytes().get(index - 1).copied();
        let end = if string.len() > index && string.as_bytes().last().copied() == quote {
            string.len() - 1
        } else {
            string.len()
        };
        let mut failed = false;
        while index < end {
            let c = string[index..].chars().next().unwrap();
            index += c.len_utf8();
            if c != '\\' {
                units.push(LiteralUnit::Character(c));
                continue;
            }
            let mut error = None;
            let unit = if index == end {
                error = Some(PreprocessorErrorType::UnterminatedEscapeSequence);
                None
            } else {
                let c = string[index..].chars().next().unwrap();
                index += c.len_utf8();
                match c {
                    | 'a' => Some(LiteralUnit::Character('\x07')),
                    | 'b' => Some(LiteralUnit::Character('\x08')),
                    | 'f' => Some(LiteralUnit::Character('\x0c')),
                    | 'n' => Some(LiteralUnit::Character('\n')),
                    | 'r' => Some(LiteralUnit::Character('\r')),
                    | 't' => Some(LiteralUnit::Character('\t')),
                    | 'v' => Some(LiteralUnit::Character('\x0b')),
                    | '\'' | '"' | '?' | '\\' => Some(LiteralUnit::Character(c)),
                    | 'x' | '0'..='7' => {
                        let hex = c == 'x';
                        let radix = if hex { 16 } else { 8 };
                        let mut count = usize::from(!hex);
                        let mut value = Some(if hex { 0u32 } else { c.to_digit(8).unwrap() });
                        while index < end && (hex || count < 3) {
                            let Some(digit) = string[index..]
                                .chars()
                                .next()
                                .and_then(|c| c.to_digit(radix))
                            else {
                                break;
                            };
                            index += 1;
                            count += 1;
                            value = value
                                .and_then(|value| value.checked_mul(radix))
                                .and_then(|value| value.checked_add(digit));
                        }
                        if count == 0 {
                            error = Some(PreprocessorErrorType::InvalidHexEscapeSequence);
                            None
                        } else if let Some(value) = value.filter(|value| wide || *value <= 255) {
                            Some(LiteralUnit::Numeric(value))
                        } else {
                            error = Some(if hex {
                                PreprocessorErrorType::HexEscapeSequenceTooLarge
                            } else {
                                PreprocessorErrorType::OctalEscapeSequenceTooLarge
                            });
                            None
                        }
                    },
                    | 'u' | 'U' => {
                        let required = if c == 'u' { 4 } else { 8 };
                        let mut count = 0;
                        let mut value = 0u32;
                        while count < required && index < end {
                            let Some(digit) =
                                string[index..].chars().next().and_then(|c| c.to_digit(16))
                            else {
                                break;
                            };
                            value = value * 16 + digit;
                            index += 1;
                            count += 1;
                        }
                        if count != required {
                            error = Some(if c == 'u' {
                                PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort
                            } else {
                                PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall
                            });
                            None
                        } else if let Some(character) = char::from_u32(value)
                            .filter(|_| value >= 0xA0 || matches!(value, 0x24 | 0x40 | 0x60))
                        {
                            Some(LiteralUnit::Character(character))
                        } else {
                            error = Some(if c == 'u' {
                                PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence
                            } else {
                                PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence
                            });
                            None
                        }
                    },
                    | _ => {
                        error = Some(PreprocessorErrorType::InvalidEscapeSequence);
                        None
                    },
                }
            };
            if let Some(unit) = unit {
                units.push(unit);
            }
            if let Some(error_type) = error {
                failed = true;
                Context::raw_preprocessor_error(
                    &mut context.pending_errors,
                    PreprocessorError {
                        error_type,
                        source_vectors: token.source_vectors,
                    },
                );
            }
        }
        failed
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
        context: &mut Context<'_>,
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

    fn parse_string(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> StringTokenType {
        let mut units = self.state.literal_scratch.take_units();
        _ = Self::eval_escape_sequences(context, token, &mut units);
        let cached_contents = context.intern_literal(&units);
        self.state.literal_scratch.return_units(units);
        if context.string_cache.at(token.contents).starts_with('L') {
            StringTokenType::WideString(cached_contents)
        } else {
            StringTokenType::String(cached_contents)
        }
    }

    pub(super) fn parse_character(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> CharacterTokenType {
        let mut units = self.state.literal_scratch.take_units();
        let had_escape_error = Self::eval_escape_sequences(context, token, &mut units);
        let wide = context.string_cache.at(token.contents).starts_with('L');
        _ = context.intern_literal(&units);
        let character = Self::character_value(context, token, &units, wide, had_escape_error);
        self.state.literal_scratch.return_units(units);
        character
    }

    /// The value of a character constant whose units are `units`.
    fn character_value(
        context: &mut Context<'_>,
        token: PreprocessorToken,
        units: &[LiteralUnit],
        wide: bool,
        had_escape_error: bool,
    ) -> CharacterTokenType {
        if wide {
            if units.len() != 1 && (!units.is_empty() || !had_escape_error) {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                    source_vectors: token.source_vectors,
                });
            }
            CharacterTokenType::WideChar(units.first().map_or(0, |unit| match *unit {
                | LiteralUnit::Character(c) => u32::from(c),
                | LiteralUnit::Numeric(code) => code,
            }))
        } else {
            let bytes = units.iter().flat_map(|unit| {
                let mut encoded = [0; 4];
                let length = match *unit {
                    | LiteralUnit::Character(c) => c.encode_utf8(&mut encoded).len(),
                    | LiteralUnit::Numeric(code) => {
                        encoded[0] =
                            u8::try_from(code).expect("narrow escape checked during decoding");
                        1
                    },
                };
                encoded.into_iter().take(length)
            });
            let mut first_two = bytes.clone();
            match (first_two.next(), first_two.next()) {
                | (Some(byte), None) => CharacterTokenType::Char(char::from(byte)),
                | (None, _) => {
                    if !had_escape_error {
                        context.preprocessor_error(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::MultiCharacterLiteralsUnsupported,
                            source_vectors: token.source_vectors,
                        });
                    }
                    CharacterTokenType::Char('\0')
                },
                | _ => CharacterTokenType::MultiChar(
                    bytes.fold(0_i32, |value, byte| value.wrapping_shl(8) | i32::from(byte)),
                ),
            }
        }
    }

    pub(super) fn map_preprocessor_token(
        &mut self,
        context: &mut Context<'_>,
        token: PreprocessorToken,
    ) -> Option<Token> {
        Some(match token.kind {
            | PreprocessorTokenType::Other => {
                let character = context
                    .string_cache
                    .at(token.contents)
                    .chars()
                    .next()
                    .expect("Other tokens contain one character");
                let source = context.first_source_vector(token.source_vectors).clone();
                context.preprocessor_tokenizer_error(
                    PreprocessorTokenizerError::unknown_character(source, character),
                );
                return None;
            },
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
            | PreprocessorTokenType::GeneratedString
            | PreprocessorTokenType::WideGeneratedString => {
                let wide = token.kind == PreprocessorTokenType::WideGeneratedString;
                let text = &context.string_cache.at(token.contents)[usize::from(wide)..];
                let mut units = self.state.literal_scratch.take_units();
                units.extend(text.chars().map(LiteralUnit::Character));
                let id = context.intern_literal(&units);
                self.state.literal_scratch.return_units(units);
                Token {
                    kind: TokenType::String(if wide {
                        StringTokenType::WideString(id)
                    } else {
                        StringTokenType::String(id)
                    }),
                    ..Self::build_token(token, TokenType::Identifier)
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
            | PreprocessorTokenType::Identifier
            | PreprocessorTokenType::UniversalIdentifier
            | PreprocessorTokenType::UnavailableIdentifier
            | PreprocessorTokenType::UnavailableUniversalIdentifier
            | PreprocessorTokenType::Defined => {
                let contents = token.identifier_id(context);
                Token {
                    kind: KeywordTokenType::from_cache_id(contents)
                        .map_or(TokenType::Identifier, TokenType::Keyword),
                    contents,
                    source_vectors: token.source_vectors,
                }
            },
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

            | PreprocessorTokenType::Placeholder | PreprocessorTokenType::Whitespace => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedTokenAtPhase7(token.kind),
                    source_vectors: token.source_vectors,
                });
                return None;
            },
        })
    }
}
