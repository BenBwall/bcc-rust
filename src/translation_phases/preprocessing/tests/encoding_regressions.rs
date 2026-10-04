//! Execution encoding and universal-character-name regressions.
//! UTF-8 source characters, 8-bit narrow codes, and 32-bit wide codes.

use std::path::PathBuf;

use super::Preprocessor;
use crate::{
    translation_phases::{
        Context,
        preprocessing::{
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::shared::SharedVec,
};

#[derive(Debug, Default)]
struct Observation {
    narrow:     Vec<Vec<u8>>,
    characters: Vec<i64>,
    spellings:  Vec<String>,
    errors:     Vec<String>,
}

fn record(token: Token, context: &Context<'_>, result: &mut Observation) {
    result.spellings.push(
        context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0')
            .to_owned(),
    );
    match token.kind {
        | TokenType::String(StringTokenType::String(id)) =>
            result.narrow.push(context.literal_bytes(id)),
        | TokenType::Character(value) => result.characters.push(i64::from(value)),
        | _ => {},
    }
}

fn observe(source: &str) -> Observation {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<encoding followup>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut result = Observation::default();
    for token in preprocessor.preprocess_all(&mut context) {
        record(token, &context, &mut result);
    }
    result.errors = context
        .take_pending_errors()
        .into_iter()
        .map(|error| format!("{error:?}"))
        .collect();
    result
}

#[test]
fn numeric_escapes_form_the_same_valid_utf8_bytes_as_source_characters() {
    // Must update the accessor in record() when literal values acquire a byte
    // arena.
    let source = "\"\\xc3\\xa9\"; \"é\"; after\n";
    let actual = observe(source);
    assert!(actual.errors.is_empty(), "{actual:#?}");
    assert_eq!(actual.narrow, [vec![0xC3, 0xA9], vec![0xC3, 0xA9]]);
    assert_eq!(actual.spellings.last().unwrap(), "after");
}

#[test]
fn wide_numeric_surrogate_code_unit_is_distinct_from_invalid_ucn() {
    let valid_numeric = "L'\\xd800'; after\n";
    let actual = observe(valid_numeric);
    assert!(actual.errors.is_empty(), "{actual:#?}");
    assert_eq!(actual.characters, [0xD800]);
    assert_eq!(actual.spellings.last().unwrap(), "after");
    let invalid_ucn = "L'\\uD800'; after\n";
    let actual = observe(invalid_ucn);
    assert!(
        actual
            .errors
            .iter()
            .any(|error| error.contains("InvalidSmallUnicodeEscapeSequence"))
    );
    assert_eq!(actual.spellings.last().unwrap(), "after");
}

#[test]
fn four_and_eight_digit_ucns_name_the_same_macro_identifier() {
    for (defined, invoked) in [
        ("\\u00E9", "\\U000000E9"),
        ("\\U000000E9", "\\u00E9"),
        ("prefix\\U000000E9tail", "prefix\\u00E9tail"),
    ] {
        let source = format!("#define {defined} 7\n{invoked} after\n");
        let actual = observe(&source);
        assert!(actual.errors.is_empty(), "{source:?}: {actual:#?}");
        assert_eq!(actual.spellings, ["7", "after"]);
    }
}

#[test]
fn arbitrary_narrow_bytes_and_numeric_range_recovery() {
    let actual = observe("\"\\xff\\0\"; after\n");
    assert!(actual.errors.is_empty(), "{actual:#?}");
    assert_eq!(actual.narrow, [vec![255, 0]]);
    for literal in ["\"\\x100z\"", "\"\\400z\"", "\"\\x100000000z\""] {
        let source = format!("{literal}; after\n");
        let actual = observe(&source);
        assert_eq!(actual.narrow, [b"z".to_vec()]);
        assert_eq!(actual.errors.len(), 1, "{actual:#?}");
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
}

#[test]
fn identifier_ucns_work_in_parameters_conditionals_pastes_and_source_unicode() {
    let source = "#define F(\\u00E9) \\U000000E9\n#define \\u00E9 9\n#if \
                  defined(\\U000000E9)\nF(3) é\n#endif\n#define CAT(a,b) a##b\nCAT(pre,\\u00E9) \
                  after\n";
    let actual = observe(source);
    assert!(actual.errors.is_empty(), "{actual:#?}");
    assert_eq!(actual.spellings, ["3", "9", "preé", "after"]);
}

#[test]
fn invalid_identifier_ucns_preserve_following_source() {
    for escaped in [
        "\\u0041",
        "\\u0660",
        "\\uD800",
        "\\U00110000",
        "\\u123",
        "\\u00A0",
    ] {
        let source = format!("{escaped}; after\n");
        let actual = observe(&source);
        assert!(!actual.errors.is_empty(), "{escaped}: {actual:#?}");
        assert_eq!(actual.spellings.last().unwrap(), "after");
    }
}
