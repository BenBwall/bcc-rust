//! Conversion of preprocessing tokens into parser tokens.
//!
//! C99: translation phases 5-7, §5.1.1.2 paragraph 1 items 5-7, p. 10; PDF
//! p. 22. Phase 5 decodes the escape sequences of character constants and
//! string literals (§6.4.4.4, pp. 59-61; PDF pp. 71-73); phase 6
//! concatenates adjacent string literals (§6.4.5 paragraph 4, p. 62; PDF
//! p. 74); phase 7 converts each preprocessing token into a token, which
//! must have the lexical form of one (§6.4 paragraph 2, p. 49; PDF p. 61).
//! Identifiers become keywords where they can (§6.4.2.1 paragraph 4, p. 51;
//! PDF p. 63), and pp-numbers become integer or floating constants (§6.4.8
//! paragraph 4, p. 65; PDF p. 77).
//!
//! Source characters map to the execution character set as UTF-8: a narrow
//! literal holds UTF-8 bytes and a wide one Unicode code points. The mapping
//! is implementation-defined (§6.4.4.4 paragraph 2, p. 60; PDF p. 72).

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
        ImaginaryIntegerKind,
        IntegerSuffix,
        IntegerTokenType,
        KeywordTokenType,
        LiteralEncoding,
        LiteralId,
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
    configuration::{
        CStandard,
        Feature,
    },
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

/// The integer representation an integer constant is typed against.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum IntegerRepresentation {
    /// Translation phase 7 under LP64: `int` has 32 bits, and `long` and
    /// `long long` have 64.
    ///
    /// C99: the widths are implementation-defined (§5.2.4.2.1 paragraph 1,
    /// pp. 21-22; PDF pp. 33-34).
    Lp64,
    /// An `#if` or `#elif` expression, where every signed type acts as
    /// `intmax_t` and every unsigned type as `uintmax_t`, here 64 bits. So
    /// `0xFFFFFFFF` is a signed, positive `int` there although it is
    /// `unsigned int` in phase 7.
    ///
    /// C99: §6.10.1 paragraph 4 and footnote 145, p. 148; PDF p. 160.
    IntMax,
}

/// A type an integer constant can have.
///
/// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum IntegerConstantType {
    Int,
    UnsignedInt,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
}

impl IntegerConstantType {
    /// The list of types an integer constant tries in order, taking the
    /// first that can represent its value. A decimal constant without `u`
    /// tries only signed types; an octal or hexadecimal constant also tries
    /// the unsigned counterpart after each signed type. Binary constants, an
    /// extension, use the octal and hexadecimal lists, as GCC and Clang do.
    ///
    /// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
    fn candidates(
        suffix: Option<IntegerSuffix>,
        is_decimal: bool,
        standard: CStandard,
    ) -> &'static [Self] {
        use IntegerConstantType::{
            Int,
            Long,
            LongLong,
            UnsignedInt,
            UnsignedLong,
            UnsignedLongLong,
        };
        // C89: §3.1.3.2; PDF p. 42. Decimal constants may have unsigned
        // long type, and long-long types are absent from the native lists.
        // Explicit ll suffixes keep the later-standard extension below.
        if standard < CStandard::C99 {
            match (suffix, is_decimal) {
                | (None, true) => return &[Int, Long, UnsignedLong],
                | (None, false) => return &[Int, UnsignedInt, Long, UnsignedLong],
                | (Some(IntegerSuffix::Unsigned), _) => return &[UnsignedInt, UnsignedLong],
                | (Some(IntegerSuffix::Long), _) => return &[Long, UnsignedLong],
                | (Some(IntegerSuffix::UnsignedLong), _) => return &[UnsignedLong],
                | _ => {},
            }
        }
        match (suffix, is_decimal) {
            | (None, true) => &[Int, Long, LongLong],
            | (None, false) => &[
                Int,
                UnsignedInt,
                Long,
                UnsignedLong,
                LongLong,
                UnsignedLongLong,
            ],
            | (Some(IntegerSuffix::Unsigned), _) => &[UnsignedInt, UnsignedLong, UnsignedLongLong],
            | (Some(IntegerSuffix::Long), true) => &[Long, LongLong],
            | (Some(IntegerSuffix::Long), false) =>
                &[Long, UnsignedLong, LongLong, UnsignedLongLong],
            | (Some(IntegerSuffix::UnsignedLong), _) => &[UnsignedLong, UnsignedLongLong],
            | (Some(IntegerSuffix::LongLong), true) => &[LongLong],
            | (Some(IntegerSuffix::LongLong), false) => &[LongLong, UnsignedLongLong],
            | (Some(IntegerSuffix::UnsignedLongLong), _) => &[UnsignedLongLong],
        }
    }

    /// The largest value this type holds under `representation`.
    fn max(self, representation: IntegerRepresentation) -> u64 {
        match (self, representation) {
            | (Self::Int, IntegerRepresentation::Lp64) => i32::MAX.unsigned_abs().into(),
            | (Self::UnsignedInt, IntegerRepresentation::Lp64) => u32::MAX.into(),
            | (Self::Int | Self::Long | Self::LongLong, _) => i64::MAX.unsigned_abs(),
            | (Self::UnsignedInt | Self::UnsignedLong | Self::UnsignedLongLong, _) => u64::MAX,
        }
    }

    /// A token holding `value`, which this type can represent. Under
    /// [`IntegerRepresentation::IntMax`] every type is 64 bits wide, so the
    /// token is `long long` or `unsigned long long` by signedness.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "`max` has bounded `value` by this type's range."
    )]
    fn token_type(self, value: u64, representation: IntegerRepresentation) -> IntegerTokenType {
        debug_assert!(
            value <= self.max(representation),
            "the type was chosen to hold the value"
        );
        match (self, representation) {
            | (Self::Int, IntegerRepresentation::Lp64) => IntegerTokenType::Int(value as i32),
            | (Self::UnsignedInt, IntegerRepresentation::Lp64) =>
                IntegerTokenType::UnsignedInt(value as u32),
            | (Self::Long, IntegerRepresentation::Lp64) =>
                IntegerTokenType::Long(Packed::new(value as i64)),
            | (Self::UnsignedLong, IntegerRepresentation::Lp64) =>
                IntegerTokenType::UnsignedLong(Packed::new(value)),
            | (Self::Int | Self::Long | Self::LongLong, _) =>
                IntegerTokenType::LongLong(Packed::new(value as i64)),
            | (Self::UnsignedInt | Self::UnsignedLong | Self::UnsignedLongLong, _) =>
                IntegerTokenType::UnsignedLongLong(Packed::new(value)),
        }
    }
}

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
struct LiteralBuilder<'pp> {
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

impl<'tu, 'pp> Expander<'_, 'tu, 'pp, '_> {
    /// Joins `first` with the string literals that follow it. The result is
    /// wide if any part is. Escapes were decoded per literal beforehand, so
    /// `"\x12" "3"` holds two characters.
    ///
    /// C99: §6.4.5 paragraph 4, p. 62; PDF p. 74, and paragraph 7, p. 63;
    /// PDF p. 75.
    pub(super) fn concatenate_adjacent_strings(&mut self, first: Token) -> Token {
        let TokenType::String(first_kind) = first.kind else {
            return first;
        };
        let mut wide = matches!(first_kind, StringTokenType::WideString(_));
        let mut encoding = if let StringTokenType::EncodedString(_, e) = first_kind {
            Some(e)
        } else {
            None
        };
        let first_contents = match first_kind {
            | StringTokenType::String(contents)
            | StringTokenType::WideString(contents)
            | StringTokenType::EncodedString(contents, _) => contents,
        };
        // Most literals stand alone: fill the reused builders only after
        // adjacency.
        let mut builder: Option<LiteralBuilder<'pp>> = None;

        loop {
            // Diagnostics reported while reading the next token stay after
            // the ones already pending.
            let existing_errors = self.context.pending_error_count();
            let next = self.next_parser_token();

            if self.context.source_segment_count() > self.source_segment_limit {
                self.pending_parser_token = next;
                self.pending_parser_errors
                    .extend(self.context.split_off_pending_errors(existing_errors));
                break;
            }

            let Some(next) = next else {
                self.pending_parser_errors
                    .extend(self.context.split_off_pending_errors(existing_errors));
                break;
            };
            let TokenType::String(next_kind) = next.kind else {
                self.pending_parser_token = Some(next);
                self.pending_parser_errors
                    .extend(self.context.split_off_pending_errors(existing_errors));
                break;
            };
            let next_encoding = if let StringTokenType::EncodedString(_, e) = next_kind {
                Some(e)
            } else {
                None
            };
            if (encoding.is_some() && next_encoding.is_some() && encoding != next_encoding)
                || (wide && next_encoding.is_some())
                || (encoding.is_some() && matches!(next_kind, StringTokenType::WideString(_)))
            {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::LanguageConstraint(
                        "incompatible string literal encoding prefixes",
                    ),
                    source_vectors: next.source_vectors,
                });
            }
            encoding = encoding.or(next_encoding);
            let next_contents = match next_kind {
                | StringTokenType::EncodedString(contents, _)
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
                    .extend_from_slice(self.context.literal_units(first_contents));
                builder.sources.push(first.source_vectors);
                builder
                    .spelling
                    .push_str(self.context.string_cache.at(first.contents));
                builder
            });
            builder
                .units
                .extend_from_slice(self.context.literal_units(next_contents));
            builder.sources.push(next.source_vectors);
            builder.spelling.push(' ');
            builder
                .spelling
                .push_str(self.context.string_cache.at(next.contents));
        }

        let Some(builder) = builder else {
            return first;
        };
        let contents = self.context.intern_literal(&builder.units);
        let token = Token {
            kind:           TokenType::String(string_token_type(contents, encoding, wide)),
            contents:       self.context.string_cache.intern(&*builder.spelling),
            source_vectors: self.context.merge_vector_list(&builder.sources),
        };
        self.state.literal_scratch.return_builder(builder);
        token
    }

    /// Converts an integer pp-number in `radix`, giving it the first type of
    /// the list for its radix and suffix that holds the value under
    /// `representation`.
    ///
    /// C99: §6.4.4.1 paragraphs 1-6, pp. 54-56; PDF pp. 66-68. A value beyond
    /// 64 bits is an error, and its wrapped value is typed without further
    /// diagnostics.
    ///
    /// A decimal constant without `u` that no type in its list can represent
    /// has no type under paragraph 6, p. 56; PDF p. 68, since there are no
    /// extended integer types here. It is given `unsigned long long`, as
    /// Clang does, with a warning, whichever of `l` or `ll` it carries.
    ///
    /// The widening warnings, for a constant that leaves `int` for `long` or
    /// `unsigned int` for `unsigned long`, flag valid code and are not
    /// required by C99.
    #[inline(always)]
    fn parse_integer_radix(
        &mut self,
        radix: u32,
        start_index: usize,
        invalid_integer_literal_error: PreprocessorErrorType<'tu>,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        let mut index = start_index;
        let (result, did_overflow) = {
            let contents = self.context.string_cache.at(token.contents);
            let mut result = 0u64;
            let mut did_overflow = false;
            loop {
                if contents.as_bytes().get(index) == Some(&b'\'')
                    && self.context.configuration.accepts(Feature::DigitSeparators)
                    && (index > start_index || (radix == 8 && start_index == 1 && index == 1))
                    && contents
                        .char_at(index - 1)
                        .and_then(|c| c.to_digit(radix))
                        .is_some()
                    && contents
                        .char_at(index + 1)
                        .and_then(|c| c.to_digit(radix))
                        .is_some()
                {
                    index += 1;
                }
                let Some(digit) = contents.char_at(index).and_then(|c| c.to_digit(radix)) else {
                    break;
                };
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
        let missing_digits = index == start_index && (radix != 8 || start_index == 2);
        if did_overflow {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::IntegerLiteralOverflow,
                source_vectors: token.source_vectors,
            });
        }
        let contents = self.context.string_cache.at(token.contents);
        let tail = &contents[index..contents.len() - 1];
        let bitint_unsigned = if self.context.configuration.accepts(Feature::BitIntSuffixes)
            && matches!(
                tail,
                "wb" | "WB" | "uwb" | "Uwb" | "uWB" | "UWB" | "wbu" | "wbU" | "WBu" | "WBU"
            )
            && !missing_digits
        {
            let unsigned = tail.contains(['u', 'U']);
            if representation == IntegerRepresentation::Lp64 {
                let width = (64 - result.leading_zeros()).max(1) + u32::from(!unsigned);
                return Self::build_token(
                    token,
                    TokenType::Integer(IntegerTokenType::BitInt(
                        Packed::new(result),
                        u8::try_from(width).expect("64-bit magnitudes require at most 65 bits"),
                        unsigned,
                    )),
                );
            }
            // C23: §6.10.2p13, p. 167; PDF p. 180: even bit-precise
            // integer types act as intmax_t/uintmax_t in #if. Apply the
            // ordinary magnitude checks instead of wrapping a signed 65-bit
            // constant into the frontend's signed 64-bit evaluator.
            index = contents.len() - 1;
            Some(unsigned)
        } else {
            None
        };
        if matches!(
            tail,
            "ll" | "LL" | "ull" | "Ull" | "uLL" | "ULL" | "llu" | "llU" | "LLu" | "LLU"
        ) {
            self.context.report_extension(
                Feature::LongLong,
                "long long integer constant",
                token.source_vectors,
            );
        }
        let contents = self.context.string_cache.at(token.contents);
        let suffix_type = if let Some(unsigned) = bitint_unsigned {
            unsigned.then_some(IntegerSuffix::Unsigned)
        } else {
            match (
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
            }
        };
        if missing_digits || index != contents.len() - 1 {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     invalid_integer_literal_error,
                source_vectors: token.source_vectors,
            });
        }
        let is_decimal = radix == 10;
        let candidates = IntegerConstantType::candidates(
            suffix_type,
            is_decimal,
            self.context.configuration.standard(),
        );
        let constant_type = match candidates
            .iter()
            .copied()
            .find(|candidate| result <= candidate.max(representation))
        {
            | Some(constant_type) => constant_type,
            // Only a decimal list without `u` can run out, and only once its
            // last type, a 64-bit signed one, cannot hold the value.
            | None => {
                debug_assert!(
                    is_decimal && result > i64::MAX.unsigned_abs(),
                    "only a signed decimal list runs out of types"
                );
                if !did_overflow {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedToUnsignedConversion {
                            to: UnsignedIntegerLiteralType::UnsignedLongLong,
                        },
                        source_vectors: token.source_vectors,
                    });
                }
                IntegerConstantType::UnsignedLongLong
            },
        };
        if !did_overflow {
            match (candidates[0], constant_type) {
                | (IntegerConstantType::Int, IntegerConstantType::Long) =>
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedSignedPromotion {
                            from: SignedIntegerLiteralType::Int,
                            to:   SignedIntegerLiteralType::Long,
                        },
                        source_vectors: token.source_vectors,
                    }),
                | (IntegerConstantType::UnsignedInt, IntegerConstantType::UnsignedLong) =>
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::ForcedUnsignedPromotion {
                            from: UnsignedIntegerLiteralType::UnsignedInt,
                            to:   UnsignedIntegerLiteralType::UnsignedLong,
                        },
                        source_vectors: token.source_vectors,
                    }),
                | _ => {},
            }
        }
        Token {
            kind:           TokenType::Integer(constant_type.token_type(result, representation)),
            source_vectors: token.source_vectors,
            contents:       token.contents,
        }
    }

    fn parse_hexadecimal_integer(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        self.parse_integer_radix(
            16,
            2,
            PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            token,
            representation,
        )
    }

    /// Converts an enabled `0b`/`0B` constant after the mode/policy check.
    /// Native in C23 §6.4.4.1p1; a GNU extension in earlier GNU modes.
    fn parse_binary_integer(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        self.parse_integer_radix(
            2,
            2,
            PreprocessorErrorType::InvalidBinaryIntegerLiteral,
            token,
            representation,
        )
    }

    fn parse_octal_integer(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        self.parse_integer_radix(
            8,
            1,
            PreprocessorErrorType::InvalidOctalIntegerLiteral,
            token,
            representation,
        )
    }

    fn parse_decimal_integer(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        self.parse_integer_radix(
            10,
            0,
            PreprocessorErrorType::InvalidDecimalIntegerLiteral,
            token,
            representation,
        )
    }

    /// Converts a floating pp-number to the type its suffix names.
    ///
    /// C99: §6.4.4.2 paragraphs 1-4, pp. 57-58; PDF pp. 69-70. A value its
    /// type cannot represent violates §6.4.4 paragraph 2, p. 54; PDF p. 66.
    #[inline(always)]
    fn parse_float(
        &mut self,
        invalid_float_literal_error: PreprocessorErrorType<'tu>,
        token: PreprocessorToken,
    ) -> Token {
        let contents = self.context.string_cache.at(token.contents);

        let mut normalized = ArenaString::new_in(self.scratch);
        if contents.contains('\'') && self.context.configuration.accepts(Feature::DigitSeparators) {
            let hex = contents.starts_with("0x") || contents.starts_with("0X");
            for (index, byte) in contents
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\'')
            {
                let radix = if hex && !contents[..index].contains(['p', 'P']) {
                    16
                } else {
                    10
                };
                if index == 0
                    || !contents
                        .char_at(index - 1)
                        .is_some_and(|c| c.is_digit(radix))
                    || !contents
                        .char_at(index + 1)
                        .is_some_and(|c| c.is_digit(radix))
                {
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     invalid_float_literal_error,
                        source_vectors: token.source_vectors,
                    });
                    return Self::build_token(
                        token,
                        TokenType::Float(FloatTokenType::Double(Packed::new(0.0))),
                    );
                }
                _ = byte;
            }
            for c in contents.chars().filter(|c| *c != '\'') {
                normalized.push(c);
            }
        }
        let contents = if normalized.is_empty() {
            contents
        } else {
            &normalized
        };
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
                self.context.preprocessor_error(PreprocessorError {
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
                self.context.preprocessor_error(PreprocessorError {
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

    fn parse_hexadecimal_float(&mut self, token: PreprocessorToken) -> Token {
        self.parse_float(PreprocessorErrorType::InvalidHexadecimalFloatLiteral, token)
    }

    fn parse_decimal_float(&mut self, token: PreprocessorToken) -> Token {
        self.parse_float(PreprocessorErrorType::InvalidDecimalFloatLiteral, token)
    }

    /// Appends the units `token` spells to `units`, which start empty, and
    /// returns whether an escape sequence was invalid.
    ///
    /// C99: phase 5, §5.1.1.2 paragraph 1 item 5, p. 10; PDF p. 22, and
    /// `escape-sequence`, §6.4.4.4 paragraphs 1 and 3-8, pp. 59-60; PDF
    /// pp. 71-72. An octal escape takes at most three digits and a hex escape
    /// all that follow (paragraph 7). A narrow octal or hex escape must fit
    /// in an 8-bit `unsigned char` (paragraph 9, p. 61; PDF p. 73), and a
    /// wide one in 32 bits. A universal character name needs exactly 4 or 8
    /// digits and must not name a surrogate or a character below U+00A0 other
    /// than `$`, `@`, and `` ` `` (§6.4.3 paragraphs 1-2, p. 53; PDF p. 65).
    /// Any other character after `\` requires a diagnostic (footnote 65,
    /// p. 60; PDF p. 72).
    fn eval_escape_sequences(
        context: &mut Context<'_>,
        token: PreprocessorToken,
        units: &mut ArenaVec<'_, LiteralUnit>,
    ) -> bool {
        let string = context.string_cache.at(token.contents);
        let prefix = string.find(['"', '\'']).unwrap_or(0);
        let wide = prefix != 0 && !string.starts_with("u8");
        // The largest value a numeric escape may give one code unit of the
        // literal's element type: C99 §6.4.4.4p9, p. 61; PDF p. 73, with
        // C11's 16-bit `u` literals.
        let max_escape = if string.starts_with('u') && !string.starts_with("u8") {
            65535
        } else if wide {
            u32::MAX
        } else {
            255
        };
        let mut index = prefix + 1;
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
                    | 'x' | 'o'
                        if string.as_bytes().get(index) == Some(&b'{')
                            && context.configuration.accepts(Feature::DelimitedEscapes) =>
                    {
                        index += 1;
                        let radix = if c == 'x' { 16 } else { 8 };
                        let mut value = Some(0u32);
                        let mut count = 0;
                        while index < end {
                            let Some(digit) = string.as_bytes()[index]
                                .is_ascii()
                                .then(|| char::from(string.as_bytes()[index]).to_digit(radix))
                                .flatten()
                            else {
                                break;
                            };
                            value = value
                                .and_then(|v| v.checked_mul(radix))
                                .and_then(|v| v.checked_add(digit));
                            count += 1;
                            index += 1;
                        }
                        if count == 0 || string.as_bytes().get(index) != Some(&b'}') {
                            error = Some(PreprocessorErrorType::LanguageConstraint(
                                "invalid delimited escape sequence",
                            ));
                            None
                        } else {
                            index += 1;
                            if let Some(value) = value.filter(|v| *v <= max_escape) {
                                Some(LiteralUnit::Numeric(value))
                            } else {
                                error = Some(PreprocessorErrorType::LanguageConstraint(
                                    "delimited escape sequence is too large",
                                ));
                                None
                            }
                        }
                    },
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
                        } else if let Some(value) = value.filter(|value| *value <= max_escape) {
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
                        } else if let Some(character) = char::from_u32(value).filter(|_| {
                            // C23 §6.4.3p2, p. 56; PDF p. 69 permits basic
                            // and control characters inside literals.
                            context.configuration.standard() >= CStandard::C23
                                || value >= 0xA0
                                || matches!(value, 0x24 | 0x40 | 0x60)
                        }) {
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

    /// Converts a pp-number into an integer or floating constant, which then
    /// acquires its type and value.
    ///
    /// C99: §6.4.8 paragraph 4, p. 65; PDF p. 77; the forms are those of
    /// §6.4.4.1 paragraph 1, pp. 54-55; PDF pp. 66-67, and §6.4.4.2
    /// paragraph 1, p. 57; PDF p. 69.
    ///
    /// Integer constants are typed under `representation`: phase 7 uses
    /// [`IntegerRepresentation::Lp64`] and `#if` evaluation
    /// [`IntegerRepresentation::IntMax`].
    pub(super) fn parse_number(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        let contents = self.context.string_cache.at(token.contents);
        let spelling = contents.trim_end_matches('\0');
        let imaginary = match spelling.as_bytes() {
            | [.., b'i' | b'I' | b'j' | b'J'] => Some(spelling.len() - 1),
            // Joined to a hexadecimal integer, an `f` suffix would read as a
            // digit; an integer takes no `f` suffix (C99 §6.4.4.1p1).
            | [.., b'i' | b'I' | b'j' | b'J', b'f' | b'F']
                if (spelling.starts_with("0x") || spelling.starts_with("0X"))
                    && !spelling.contains(['p', 'P']) =>
                None,
            | [.., b'i' | b'I' | b'j' | b'J', b'f' | b'F' | b'l' | b'L'] =>
                Some(spelling.len() - 2),
            | _ => None,
        };
        if self
            .context
            .configuration
            .accepts(Feature::ImaginaryConstants)
            && let Some(index) = imaginary
        {
            let mut real = ArenaString::new_in(self.scratch);
            real.push_str(&spelling[..index]);
            real.push_str(&spelling[index + 1..]);
            real.push('\0');
            let real_contents = self.context.string_cache.intern(&*real);
            self.context.report_extension(
                Feature::ImaginaryConstants,
                "imaginary constant",
                token.source_vectors,
            );
            let real_token = self.parse_plain_number(
                PreprocessorToken {
                    contents: real_contents,
                    ..token
                },
                representation,
            );
            let kind = match real_token.kind {
                | TokenType::Float(FloatTokenType::Float(v)) =>
                    TokenType::Float(FloatTokenType::ImaginaryFloat(v)),
                | TokenType::Float(FloatTokenType::Double(v)) =>
                    TokenType::Float(FloatTokenType::ImaginaryDouble(v)),
                | TokenType::Float(FloatTokenType::LongDouble(v)) =>
                    TokenType::Float(FloatTokenType::ImaginaryLongDouble(v)),
                | TokenType::Integer(v) => {
                    let component = match v {
                        | IntegerTokenType::Int(_) => ImaginaryIntegerKind::Int,
                        | IntegerTokenType::Long(_) => ImaginaryIntegerKind::Long,
                        | IntegerTokenType::LongLong(_) => ImaginaryIntegerKind::LongLong,
                        | IntegerTokenType::UnsignedInt(_) => ImaginaryIntegerKind::UnsignedInt,
                        | IntegerTokenType::UnsignedLong(_) => ImaginaryIntegerKind::UnsignedLong,
                        | _ => ImaginaryIntegerKind::UnsignedLongLong,
                    };
                    TokenType::Integer(IntegerTokenType::Imaginary(
                        Packed::new(
                            u64::try_from(i128::from(v))
                                .expect("integer constant magnitudes fit u64"),
                        ),
                        component,
                    ))
                },
                | kind => kind,
            };
            return Self::build_token(token, kind);
        }
        self.parse_plain_number(token, representation)
    }

    /// Converts the real component without recursively stripping imaginary
    /// suffixes. C99: §6.4.4.1-2, pp. 54-58; PDF pp. 66-70.
    fn parse_plain_number(
        &mut self,
        token: PreprocessorToken,
        representation: IntegerRepresentation,
    ) -> Token {
        let contents = self.context.string_cache.at(token.contents);
        let is_hex = contents.starts_with("0x") || contents.starts_with("0X");
        let is_binary = contents.starts_with("0b") || contents.starts_with("0B");
        let prefixed_octal = contents.starts_with("0o") || contents.starts_with("0O");
        if prefixed_octal && self.context.configuration.accepts(Feature::OctalPrefix) {
            return self.parse_integer_radix(
                8,
                2,
                PreprocessorErrorType::InvalidOctalIntegerLiteral,
                token,
                representation,
            );
        }
        let is_octal = contents.starts_with('0') && !is_hex && !is_binary;
        if is_hex {
            if contents.contains(['.', 'p', 'P']) {
                self.context.report_extension(
                    Feature::HexFloats,
                    "hexadecimal floating constant",
                    token.source_vectors,
                );
                self.parse_hexadecimal_float(token)
            } else {
                self.parse_hexadecimal_integer(token, representation)
            }
        } else if is_binary {
            if self.context.configuration.accepts(Feature::BinaryConstants) {
                self.context.report_extension(
                    Feature::BinaryConstants,
                    "binary integer constant",
                    token.source_vectors,
                );
            } else {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::InvalidBinaryIntegerLiteral,
                    source_vectors: token.source_vectors,
                });
            }
            self.parse_binary_integer(token, representation)
        } else if contents.contains(['.', 'e', 'E']) {
            self.parse_decimal_float(token)
        } else if is_octal {
            self.parse_octal_integer(token, representation)
        } else {
            self.parse_decimal_integer(token, representation)
        }
    }

    /// Decodes a character or wide string literal.
    ///
    /// C99: §6.4.5 paragraphs 1-3, p. 62; PDF p. 74.
    fn parse_string(&mut self, token: PreprocessorToken) -> StringTokenType {
        let mut units = self.state.literal_scratch.take_units();
        _ = Self::eval_escape_sequences(self.context, token, &mut units);
        let cached_contents = self.context.intern_literal(&units);
        self.state.literal_scratch.return_units(units);
        let spelling = self.context.string_cache.at(token.contents);
        string_token_type(
            cached_contents,
            literal_encoding(spelling),
            spelling.starts_with('L'),
        )
    }

    pub(super) fn parse_character(&mut self, token: PreprocessorToken) -> CharacterTokenType {
        let mut units = self.state.literal_scratch.take_units();
        let had_escape_error = Self::eval_escape_sequences(self.context, token, &mut units);
        let wide = self
            .context
            .string_cache
            .at(token.contents)
            .starts_with('L');
        _ = self.context.intern_literal(&units);
        let character = if let Some(encoding) =
            literal_encoding(self.context.string_cache.at(token.contents))
        {
            let value = units.first().map_or(0, |unit| match *unit {
                | LiteralUnit::Character(c) => u32::from(c),
                | LiteralUnit::Numeric(v) => v,
            });
            let max = match encoding {
                | LiteralEncoding::Utf8 if matches!(units.first(), Some(LiteralUnit::Numeric(_))) =>
                    255,
                | LiteralEncoding::Utf8 => 127,
                | LiteralEncoding::Utf16 => 65535,
                | LiteralEncoding::Utf32
                    if matches!(units.first(), Some(LiteralUnit::Numeric(_))) =>
                    u32::MAX,
                | LiteralEncoding::Utf32 => 0x0010_FFFF,
            };
            if (units.len() != 1 && (!units.is_empty() || !had_escape_error)) || value > max {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::LanguageConstraint(
                        "character constant must encode exactly one code unit",
                    ),
                    source_vectors: token.source_vectors,
                });
            }
            CharacterTokenType::EncodedChar(value, encoding)
        } else {
            Self::character_value(self.context, token, &units, wide, had_escape_error)
        };
        self.state.literal_scratch.return_units(units);
        character
    }

    /// The value of a character constant whose units are `units`.
    ///
    /// C99: §6.4.4.4 paragraphs 10-11, p. 61; PDF p. 73. A narrow constant
    /// whose character takes several UTF-8 bytes, or that has several
    /// characters, gets an implementation-defined multi-character value. A
    /// wide constant with more than one character, also
    /// implementation-defined, is rejected. An empty constant does not match
    /// `c-char-sequence` (paragraph 1, p. 59; PDF p. 71).
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

    /// Converts one preprocessing token into a token, or returns `None` for
    /// one that is not passed on: a new-line, a directive that `#` begins,
    /// or a token in no token's lexical form, which is diagnosed. A name
    /// whose macro was being replaced is an ordinary identifier again, since
    /// macros mean nothing after phase 4 (§6.10.3.5 paragraph 1, p. 155; PDF
    /// p. 167).
    ///
    /// C99: §6.4 paragraphs 2-3, p. 49; PDF p. 61.
    pub(super) fn map_preprocessor_token(&mut self, token: PreprocessorToken) -> Option<Token> {
        Some(match token.kind {
            | PreprocessorTokenType::Other => {
                let character = self
                    .context
                    .string_cache
                    .at(token.contents)
                    .chars()
                    .next()
                    .expect("Other tokens contain one character");
                let source = self
                    .context
                    .first_source_vector(token.source_vectors)
                    .clone();
                self.context.preprocessor_tokenizer_error(
                    PreprocessorTokenizerError::unknown_character(source, character),
                );
                return None;
            },
            | PreprocessorTokenType::Number =>
                self.parse_number(token, IntegerRepresentation::Lp64),
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
                self.parse_directive(token);
                return None;
            },
            | PreprocessorTokenType::GeneratedString
            | PreprocessorTokenType::WideGeneratedString => {
                let wide = token.kind == PreprocessorTokenType::WideGeneratedString;
                let spelling = self.context.string_cache.at(token.contents);
                let encoding = wide.then(|| literal_encoding(spelling)).flatten();
                let prefix_length = encoding.map_or(usize::from(wide), |e| e.prefix().len());
                let text = &spelling[prefix_length..];
                let mut units = self.state.literal_scratch.take_units();
                units.extend(text.chars().map(LiteralUnit::Character));
                let id = self.context.intern_literal(&units);
                self.state.literal_scratch.return_units(units);
                Token {
                    kind: TokenType::String(string_token_type(id, encoding, wide)),
                    ..Self::build_token(token, TokenType::Identifier)
                }
            },
            | PreprocessorTokenType::String => Token {
                kind:           TokenType::String(self.parse_string(token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Character => Token {
                kind:           TokenType::Character(self.parse_character(token)),
                contents:       token.contents,
                source_vectors: token.source_vectors,
            },
            | PreprocessorTokenType::Identifier
            | PreprocessorTokenType::UniversalIdentifier
            | PreprocessorTokenType::UnavailableIdentifier
            | PreprocessorTokenType::UnavailableUniversalIdentifier
            | PreprocessorTokenType::Defined => {
                let contents = token.identifier_id(self.context);
                let classification =
                    KeywordTokenType::classify(contents, self.context.configuration);
                if let Some(keyword) = classification
                    && let Some(origin) = keyword.origin
                {
                    self.context.report_extension_since(
                        keyword.spelling,
                        origin,
                        token.source_vectors,
                    );
                }
                Token {
                    kind: classification.map_or(TokenType::Identifier, |keyword| {
                        TokenType::Keyword(keyword.kind)
                    }),
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
            // `##` is a punctuator (C99 §6.4.6p1) that only replacement
            // lists give a meaning (§6.10.3.3).
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
                self.context.preprocessor_error(PreprocessorError {
                    error_type,
                    source_vectors: token.source_vectors,
                });
                return None;
            },

            | PreprocessorTokenType::Placeholder | PreprocessorTokenType::Whitespace => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedTokenAtPhase7(token.kind),
                    source_vectors: token.source_vectors,
                });
                return None;
            },
        })
    }
}

/// A string literal's token type: encoding-prefixed, wide (`L`), or plain.
fn string_token_type(
    contents: LiteralId,
    encoding: Option<LiteralEncoding>,
    wide: bool,
) -> StringTokenType {
    match encoding {
        | Some(encoding) => StringTokenType::EncodedString(contents, encoding),
        | None if wide => StringTokenType::WideString(contents),
        | None => StringTokenType::String(contents),
    }
}

/// C11 §6.4.5p1: the prefix determines the literal's encoding.
fn literal_encoding(spelling: &str) -> Option<LiteralEncoding> {
    if spelling.starts_with("u8") {
        Some(LiteralEncoding::Utf8)
    } else if spelling.starts_with('u') {
        Some(LiteralEncoding::Utf16)
    } else if spelling.starts_with('U') {
        Some(LiteralEncoding::Utf32)
    } else {
        None
    }
}
