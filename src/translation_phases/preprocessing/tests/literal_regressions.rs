//! C99 literal syntax, decoded values and diagnostic recovery across
//! strategies.

use std::path::PathBuf;

use super::Preprocessor;
use crate::{
    translation_phases::{
        Context,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        packed::Packed,
        shared::SharedVec,
    },
};

#[derive(Debug)]
struct Observation {
    integers:   Vec<IntegerTokenType>,
    strings:    Vec<(bool, String)>,
    characters: Vec<CharacterTokenType>,
    spellings:  Vec<String>,
    errors:     Vec<(String, Vec<SourceVector>)>,
}

fn record_token(token: Token, context: &Context, observation: &mut Observation) {
    observation
        .spellings
        .push(context.string_cache.at(token.contents).to_owned());
    match token.kind {
        | TokenType::Integer(value) => observation.integers.push(value),
        | TokenType::Character(value) => observation.characters.push(value),
        | TokenType::String(StringTokenType::String(contents)) => observation.strings.push((
            false,
            context
                .literal_text(contents, false)
                .as_deref()
                .expect("UTF-8 test literal")
                .to_owned(),
        )),
        | TokenType::String(StringTokenType::WideString(contents)) => observation.strings.push((
            true,
            context
                .literal_text(contents, true)
                .as_deref()
                .expect("UTF-8 test literal")
                .to_owned(),
        )),
        | _ => {},
    }
}

fn record_errors(context: &mut Context, observation: &mut Observation) {
    while let Some(error) = context.pop_pending_error() {
        let sources = error.source_vectors(context);
        let kind = match error {
            | TranslationError::Preprocessing(error) => format!("{:?}", error.error_type),
            | other => format!("{other:?}"),
        };
        observation
            .errors
            .push((kind, context.get_source_vectors(sources).to_vec()));
    }
}

fn observe(source: &str) -> Observation {
    let mut context = Context::new();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<literal regressions>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut observation = Observation {
        integers:   Vec::new(),
        strings:    Vec::new(),
        characters: Vec::new(),
        spellings:  Vec::new(),
        errors:     Vec::new(),
    };
    let tokens = preprocessor.preprocess_all(&mut context);
    record_errors(&mut context, &mut observation);
    for token in tokens {
        record_token(token, &context, &mut observation);
    }
    observation
}

#[test]
fn integer_unsigned_and_long_suffix_cases_are_independent() {
    for prefix in ["1", "01", "0x1", "0X1"] {
        for (suffixes, expected) in [
            (
                ["ul", "uL", "Ul", "UL", "lu", "lU", "Lu", "LU"],
                IntegerTokenType::UnsignedLong(Packed::new(1)),
            ),
            (
                ["ull", "uLL", "Ull", "ULL", "llu", "llU", "LLu", "LLU"],
                IntegerTokenType::UnsignedLongLong(Packed::new(1)),
            ),
        ] {
            for suffix in suffixes {
                let source = format!("{prefix}{suffix}; after\n");
                let actual = observe(&source);
                assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
                assert_eq!(actual.integers, [expected], "{source:?}");
                assert_eq!(actual.spellings.last().unwrap(), "after");
            }
        }
    }
}

#[test]
fn mixed_case_long_long_suffix_is_still_invalid() {
    for suffix in ["lL", "Ll", "ulL", "uLl", "UlL", "ULl", "lLu", "LlU"] {
        let source = format!("1{suffix}; after\n");
        let actual = observe(&source);
        assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
        assert_eq!(actual.errors[0].0, "InvalidDecimalIntegerLiteral");
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
}

#[test]
fn hexadecimal_integer_prefix_requires_a_digit_before_the_suffix() {
    for spelling in ["0x", "0X", "0xu", "0xL", "0xLL", "0xul", "0xULL", "0XUL"] {
        let source = format!("{spelling}; after\n");
        let actual = observe(&source);
        assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
        assert_eq!(actual.errors[0].0, "InvalidHexadecimalIntegerLiteral");
        assert_eq!(
            actual.errors[0].1[0].length,
            u32::try_from(spelling.len()).unwrap()
        );
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
    // The leading octal zero counts as a digit even though conversion starts at
    // index 1.
    for spelling in [
        "0", "0u", "0L", "0LL", "0UL", "0ULL", "0x0", "0x0UL", "0X0ULL",
    ] {
        let source = format!("{spelling}; after\n");
        let actual = observe(&source);
        assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
        assert_eq!(i128::from(actual.integers[0]), 0);
    }
}

#[test]
fn hexadecimal_escape_requires_at_least_one_digit() {
    for prefix in ["", "L"] {
        for (contents, recovery) in [("\\x", ""), ("\\xg", "g"), ("a\\xz", "az")] {
            let literal = format!("{prefix}\"{contents}\"");
            let source = format!("{literal}; after\n");
            let actual = observe(&source);
            assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
            assert_eq!(actual.errors[0].0, "InvalidHexEscapeSequence");
            assert_eq!(
                actual.errors[0].1[0].length,
                u32::try_from(literal.len()).unwrap()
            );
            assert_eq!(actual.strings, [(prefix == "L", recovery.to_owned())]);
            assert_eq!(actual.spellings.last().unwrap(), "after");
        }
        for (escape, expected) in [("\\x0", "\0"), ("\\x41", "A"), ("\\x41g", "Ag")] {
            let source = format!("{prefix}\"{escape}\"; after\n");
            let actual = observe(&source);
            assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
            assert_eq!(actual.strings, [(prefix == "L", expected.to_owned())]);
        }
    }
}

#[test]
fn universal_character_names_reject_forbidden_low_code_points() {
    for (escape, error) in [
        ("\\u0000", "InvalidSmallUnicodeEscapeSequence"),
        ("\\u0041", "InvalidSmallUnicodeEscapeSequence"),
        ("\\u007F", "InvalidSmallUnicodeEscapeSequence"),
        ("\\u009F", "InvalidSmallUnicodeEscapeSequence"),
        ("\\U00000000", "InvalidLargeUnicodeEscapeSequence"),
        ("\\U00000061", "InvalidLargeUnicodeEscapeSequence"),
        ("\\U0000009F", "InvalidLargeUnicodeEscapeSequence"),
    ] {
        for prefix in ["", "L"] {
            // Extra valid character avoids an empty-character recovery cascade.
            let literal = format!("{prefix}\"a{escape}z\"");
            let source = format!("{literal}; after\n");
            let actual = observe(&source);
            assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
            assert_eq!(actual.errors[0].0, error);
            assert_eq!(
                actual.errors[0].1[0].length,
                u32::try_from(literal.len()).unwrap()
            );
            assert_eq!(actual.strings, [(prefix == "L", "az".to_owned())]);
            assert_eq!(actual.spellings.last().unwrap(), "after");
        }
    }
}

#[test]
fn universal_character_name_exceptions_and_nonbasic_characters_are_valid() {
    for (escape, expected) in [
        ("\\u0024", "$"),
        ("\\u0040", "@"),
        ("\\u0060", "`"),
        ("\\u00A0", "\u{a0}"),
        ("\\u00E9", "é"),
        ("\\U00000024", "$"),
        ("\\U00000040", "@"),
        ("\\U00000060", "`"),
        ("\\U000000A0", "\u{a0}"),
        ("\\U0001F980", "🦀"),
    ] {
        for prefix in ["", "L"] {
            let source = format!("{prefix}\"{escape}\"; after\n");
            let actual = observe(&source);
            assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
            assert_eq!(actual.strings, [(prefix == "L", expected.to_owned())]);
        }
    }
}

#[test]
fn incomplete_universal_character_names_have_one_primary_diagnostic() {
    for (escape, error) in [
        ("\\u1", "SmallUnicodeEscapeSequenceTooShort"),
        ("\\U1", "LargeUnicodeEscapeSequenceTooSmall"),
    ] {
        let source = format!("\"a{escape}z\"; after\n");
        let actual = observe(&source);
        assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
        assert_eq!(actual.errors[0].0, error);
        assert_eq!(actual.strings, [(false, "az".to_owned())]);
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
}

#[test]
fn invalid_character_escapes_preserve_another_valid_character() {
    for (escape, error) in [
        ("\\x", "InvalidHexEscapeSequence"),
        ("\\u0041", "InvalidSmallUnicodeEscapeSequence"),
        ("\\U00000061", "InvalidLargeUnicodeEscapeSequence"),
    ] {
        for prefix in ["", "L"] {
            // Retain 'a' so rejection does not produce a secondary
            // empty-character error.
            let source = format!("{prefix}'a{escape}'; after\n");
            let actual = observe(&source);
            assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
            assert_eq!(actual.errors[0].0, error);
            assert_eq!(
                actual.characters,
                [if prefix == "L" {
                    CharacterTokenType::WideChar(u32::from('a'))
                } else {
                    CharacterTokenType::Char('a')
                }]
            );
            assert_eq!(actual.spellings.last().unwrap(), "after");
        }
    }
}

#[test]
fn valid_character_escape_controls_cover_narrow_and_wide_literals() {
    for (escape, expected) in [
        ("\\x0", '\0'),
        ("\\x1f", '\u{1f}'),
        ("\\x41", 'A'),
        ("\\u0024", '$'),
        ("\\u0040", '@'),
        ("\\u0060", '`'),
        ("\\U00000024", '$'),
        ("\\U00000040", '@'),
        ("\\U00000060", '`'),
    ] {
        for prefix in ["", "L"] {
            let source = format!("{prefix}'{escape}'; after\n");
            let actual = observe(&source);
            assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
            assert_eq!(
                actual.characters,
                [if prefix == "L" {
                    CharacterTokenType::WideChar(u32::from(expected))
                } else {
                    CharacterTokenType::Char(expected)
                }]
            );
        }
    }
}

#[test]
fn failed_character_escapes_have_one_primary_diagnostic() {
    for (escape, error) in [
        ("\\x", "InvalidHexEscapeSequence"),
        ("\\u001f", "InvalidSmallUnicodeEscapeSequence"),
        ("\\u0000", "InvalidSmallUnicodeEscapeSequence"),
        ("\\u1", "SmallUnicodeEscapeSequenceTooShort"),
        ("\\U1", "LargeUnicodeEscapeSequenceTooSmall"),
        ("\\uD800", "InvalidSmallUnicodeEscapeSequence"),
        ("\\U00110000", "InvalidLargeUnicodeEscapeSequence"),
        ("\\q", "InvalidEscapeSequence"),
    ] {
        for prefix in ["", "L"] {
            let literal = format!("{prefix}'{escape}'");
            let source = format!("{literal}; after\n");
            let actual = observe(&source);
            assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
            assert_eq!(actual.errors[0].0, error);
            assert_eq!(
                actual.errors[0].1[0].length,
                u32::try_from(literal.len()).unwrap()
            );
            assert_eq!(
                actual.characters,
                [if prefix == "L" {
                    CharacterTokenType::WideChar(u32::from('\0'))
                } else {
                    CharacterTokenType::Char('\0')
                }]
            );
            assert_eq!(actual.spellings.last().unwrap(), "after");
        }
    }
}

#[test]
fn truly_empty_character_constants_still_diagnose() {
    for prefix in ["", "L"] {
        let source = format!("{prefix}''; after\n");
        let actual = observe(&source);
        assert_eq!(actual.errors.len(), 1, "{source:?}: {actual:#?}");
        assert_eq!(actual.errors[0].0, "MultiCharacterLiteralsUnsupported");
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
}
