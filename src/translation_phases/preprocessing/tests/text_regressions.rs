//! Phase 4 read for a text of its tokens, as `-E` prints it
//! (`Context::phase_four_text`).

use std::path::PathBuf;

use crate::{
    configuration::{
        CompilerConfiguration,
        CompilerIdentity,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSeverity,
        TranslationError,
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

/// What phase 4 leaves of `source` for a text under `identity`: each token
/// as spelled, with a space between; and each diagnostic, with its severity.
struct Text {
    tokens:      String,
    diagnostics: Vec<(ErrorSeverity, String)>,
}

fn text_of(source: &str, identity: CompilerIdentity) -> Text {
    text_in(
        source,
        CompilerConfiguration::default().with_compiler_identity(identity),
    )
}

/// [`text_of`] under `configuration`.
fn text_in(source: &str, configuration: CompilerConfiguration) -> Text {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    context.phase_four_text = true;
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    preprocessor.for_each_item(&mut context, |context, token| {
        let spelling = context.string_cache.at(token.contents);
        tokens.push(spelling.trim_end_matches('\0').to_owned());
    });
    let mut diagnostics = Vec::new();
    while let Some(error) = context.pop_pending_error() {
        diagnostics.push((error.severity(), diagnostic_name(&error)));
    }
    Text {
        tokens: tokens.join(" "),
        diagnostics,
    }
}

/// The kind of `error`, as its debug form names it.
fn diagnostic_name(error: &TranslationError<'_>) -> String {
    let debug = format!("{error:?}");
    let kind = debug
        .split_once("error_type: ")
        .map_or(debug.as_str(), |(_, rest)| rest);
    kind.split([',', ' ', '{', '}'])
        .next()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn a_hash_inside_a_line_is_a_punctuator_of_the_text() {
    // C99 §6.10p2: only a `#` that is the first token of a line begins a
    // directive; GCC and Clang print the others, `%:` and `##` as written.
    let text = text_of(
        "a # b\nc %: d ## e %:%: f\nx #if 1\ny\n",
        CompilerIdentity::Gcc,
    );
    assert_eq!(text.tokens, "a # b c %: d ## e %:%: f x # if 1 y");
    assert_eq!(text.diagnostics, []);
}

#[test]
fn a_hash_a_replacement_produces_begins_no_directive() {
    // C99 §6.10.3.4p3: a replacement that looks like a directive is not
    // processed as one, and an empty one before `#` is still the line's
    // first token.
    let text = text_of(
        "#define H #\n#define E\nH define X 1\nX\nE # define Y 2\nY\n  # define Z 3\nZ\n",
        CompilerIdentity::Gcc,
    );
    assert_eq!(text.tokens, "# define X 1 X # define Y 2 Y 3");
    assert_eq!(text.diagnostics, []);
}

#[test]
fn a_stray_hash_in_a_translation_unit_is_an_error() {
    // Outside a text the punctuator has no use, as GCC's "stray '#' in
    // program" error says, and the rest of the line is not a directive.
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, CompilerConfiguration::default());
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "int value; #define X 1\nX\n",
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    preprocessor.for_each_item(&mut context, |context, token| {
        let spelling = context.string_cache.at(token.contents);
        tokens.push(spelling.trim_end_matches('\0').to_owned());
    });
    assert_eq!(tokens.join(" "), "int value ; define X 1 X");
    let error = context
        .pop_pending_error()
        .expect("the stray `#` is reported");
    assert_eq!(error.severity(), ErrorSeverity::Error);
    assert_eq!(diagnostic_name(&error), "HashMustBeFirstCharacterOnLine");
    assert!(context.pop_pending_error().is_none());
}

#[test]
fn an_unclosed_quote_keeps_the_rest_of_its_line_with_a_warning() {
    // GCC and Clang warn "missing terminating ' character" and print the
    // line as written, without the quote bcc appended for recovery.
    let text = text_of(
        "don't stop\nsay \"hi\nx 'a\" y\n#define S \"abc\nS S\nit's",
        CompilerIdentity::Gcc,
    );
    assert_eq!(
        text.tokens,
        "don 't stop say \"hi x 'a\" y \"abc \"abc it 's"
    );
    let warning = (ErrorSeverity::Warning, "MissingTerminatingQuote".to_owned());
    // Once for each literal, though `S`'s is read at each invocation.
    let missing = |quote: u8| (warning.0, format!("{}({quote})", warning.1));
    assert_eq!(
        text.diagnostics,
        [
            missing(b'\''),
            missing(b'"'),
            missing(b'\''),
            missing(b'"'),
            missing(b'\''),
        ]
    );
}

#[test]
fn a_directive_rejects_an_unclosed_literal_it_reads() {
    // GCC and Clang reject the literal where a directive needs one.
    let text = text_of("#if 'a\n#endif\n#line 5 \"f\nx\n", CompilerIdentity::Clang);
    assert_eq!(text.tokens, "x");
    assert_eq!(
        text.diagnostics,
        [
            (
                ErrorSeverity::Warning,
                "MissingTerminatingQuote(39)".to_owned()
            ),
            (ErrorSeverity::Error, "NewlineInCharacter".to_owned()),
            (
                ErrorSeverity::Warning,
                "MissingTerminatingQuote(34)".to_owned()
            ),
            (ErrorSeverity::Error, "NewlineInString".to_owned()),
        ]
    );
}
