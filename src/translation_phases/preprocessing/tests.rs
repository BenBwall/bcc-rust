use std::path::PathBuf;

use super::{
    Preprocessor,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    token::{
        CharacterTokenType,
        IntegerTokenType,
        StringTokenType,
        Token,
        TokenType,
    },
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSeverity,
        SourceVectors,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::shared::SharedVec,
};

fn preprocess_with_configuration(
    source: &str,
    configuration: CompilerConfiguration,
) -> (Vec<String>, Vec<TranslationError>) {
    let mut context = Context::with_configuration(configuration);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut identifiers = Vec::new();
    while let Some(token) = preprocessor.next_item(&mut context) {
        if token.kind == TokenType::Identifier {
            identifiers.push(context.string_cache.at(token.contents).to_owned());
        }
    }
    let mut errors = Vec::new();
    while let Some(error) = context.pop_pending_error() {
        errors.push(error);
    }
    (identifiers, errors)
}

fn preprocess(source: &str) -> (Vec<String>, Vec<TranslationError>) {
    preprocess_with_configuration(source, CompilerConfiguration::default())
}

#[test]
fn adjacent_string_lookahead_restores_diagnostics_to_the_phase_context() {
    let (_, errors) = preprocess("\"a\" 0xg\n");

    assert!(errors.iter().any(|error| matches!(
        error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            ..
        })
    )));
}

#[test]
fn adjacent_string_diagnostics_remain_in_source_order() {
    let (_, errors) = preprocess("\"\\q\" \"\\u1\"\n");

    assert!(matches!(
        errors.as_slice(),
        [
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidEscapeSequence,
                ..
            }),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort,
                ..
            })
        ]
    ));
}

#[test]
fn malformed_include_restores_include_tokenization_mode() {
    let mut context = Context::new();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "#include 123 extra\nint x = a < b > c;\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    while let Some(token) = preprocessor.next_item(&mut context) {
        tokens.push((
            token.kind,
            context.string_cache.at(token.contents).to_owned(),
        ));
    }
    let errors = context.take_pending_errors();

    assert_eq!(
        tokens
            .iter()
            .filter_map(
                |(kind, spelling)| (*kind == TokenType::Identifier).then_some(spelling.as_str())
            )
            .collect::<Vec<_>>(),
        ["x", "a", "b", "c"]
    );
    assert!(tokens.iter().all(|(_, spelling)| spelling != "123"));
    assert!(tokens.iter().all(|(_, spelling)| spelling != "extra"));
    assert!(errors.iter().any(|error| matches!(
        error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
            ..
        })
    )));
    assert!(errors.iter().all(|error| !matches!(
        error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::UnexpectedTokenAtPhase7(_),
            ..
        })
    )));
}

#[test]
fn empty_include_preserves_the_following_logical_line() {
    for source in [
        "#include\nint sentinel;\n",
        "#include /* comment */\nint sentinel;\n",
        "#include\n#define TYPE int\nTYPE sentinel;\n",
    ] {
        let (identifiers, errors) = preprocess(source);

        assert_eq!(identifiers, ["sentinel"], "{source:?}");
        assert!(errors.iter().any(|error| matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
                ..
            })
        )));
    }
}

#[test]
fn malformed_macro_expanded_include_unwinds_before_the_following_line() {
    for source in [
        "#define BAD 123\n#include BAD extra\nint sentinel;\n",
        "#define BAD() 123\n#include BAD() extra\nint sentinel;\n",
        "#define VALUE 123\n#define BAD VALUE\n#include BAD extra\nint sentinel;\n",
    ] {
        let (identifiers, errors) = preprocess(source);

        assert_eq!(identifiers, ["sentinel"], "{source:?}");
        assert!(errors.iter().any(|error| matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
                ..
            })
        )));
    }
}

#[test]
fn malformed_macro_include_does_not_repeat_expansion_diagnostics() {
    let source = "#define BAD(x) x\n#include BAD(123,456) extra\nint sentinel;\n";
    let (identifiers, errors) = preprocess(source);

    assert_eq!(identifiers, ["sentinel"]);
    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                            ..
                        },
                    ..
                })
            ))
            .count(),
        1,
        "{errors:#?}"
    );
    assert!(errors.iter().any(|error| matches!(
        error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
            ..
        })
    )));
}

#[test]
fn phase_07_mapping_diagnoses_every_internal_only_token_kind() {
    let mut context = Context::new();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<phase-7-totality-test>").into_boxed_path(),
        String::new().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let contents = context.string_cache.intern("internal-only");

    for kind in [
        PreprocessorTokenType::Placeholder,
        PreprocessorTokenType::AngleBracketString,
        PreprocessorTokenType::IncludeString,
        PreprocessorTokenType::Whitespace,
    ] {
        let token = PreprocessorToken {
            kind,
            source_vectors: SourceVectors::default(),
            contents,
        };
        assert_eq!(
            preprocessor.map_preprocessor_token(&mut context, token),
            None
        );
        assert!(matches!(
            context.pop_pending_error(),
            Some(TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::UnexpectedTokenAtPhase7(actual),
                ..
            })) if actual == kind
        ));
    }
}

fn strict_c99() -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny)
}

fn selected_identifier(expression: &str, identifier: &str) {
    let source = format!("#if {expression}\n{identifier}\n#endif\n");
    let (identifiers, errors) = preprocess(&source);

    assert_eq!(identifiers, [identifier]);
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
}

#[test]
fn middle_nested_conditional_pairs_with_the_nearest_question_mark() {
    selected_identifier("(1 ? 0 ? 2 : 3 : 4) == 3", "MIDDLE_RESULT_3");
}

#[test]
fn right_nested_conditional_remains_right_associative() {
    selected_identifier("(0 ? 1 : 1 ? 2 : 3) == 2", "RIGHT_RESULT_2");
}

#[test]
fn parentheses_are_a_conditional_pairing_boundary() {
    selected_identifier("((1 ? 0 : 1) ? 2 : 3) == 3", "PAREN_RESULT_3");
}

#[test]
fn middle_expression_operators_reduce_before_colon() {
    selected_identifier("(1 ? 1 + 2 : 4) == 3", "MIDDLE_ARITHMETIC_RESULT_3");
}

#[test]
fn unmatched_colon_reports_a_diagnostic_without_panicking() {
    let (_, errors) = preprocess("#if 1 : 2\nUNREACHABLE\n#endif\n");

    assert!(
        errors.iter().any(|error| matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ColonWithoutMatchingQuestionMark,
                ..
            })
        )),
        "diagnostics: {errors:#?}"
    );
}

#[test]
fn trailing_unmatched_colon_reports_a_diagnostic_without_panicking() {
    let (_, errors) = preprocess("#if 1 :\nRECOVERED\n#endif\n");

    assert!(
        errors.iter().any(|error| matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ColonWithoutMatchingQuestionMark,
                ..
            })
        )),
        "diagnostics: {errors:#?}"
    );
}

#[test]
fn default_extension_mode_evaluates_comma_to_rhs_without_a_diagnostic() {
    let (identifiers, errors) = preprocess("#if (1, 0)\nUNREACHABLE\n#endif\n");

    assert_eq!(identifiers, Vec::<String>::new());
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
}

#[test]
fn strict_c99_diagnoses_an_evaluated_comma_after_reducing_to_rhs() {
    let (identifiers, errors) =
        preprocess_with_configuration("#if (0, 2)\nCOMMA_RESULT_2\n#endif\n", strict_c99());

    assert_eq!(identifiers, ["COMMA_RESULT_2"]);
    assert!(
        errors
            .iter()
            .any(|error| error.severity() == ErrorSeverity::Error
                && matches!(
                    error,
                    TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                            ExtensionPolicy::Deny
                        ),
                        ..
                    })
                )),
        "diagnostics: {errors:#?}"
    );
}

#[test]
fn warning_policy_reports_an_evaluated_comma_as_a_warning() {
    let configuration = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Warn);
    let (identifiers, errors) =
        preprocess_with_configuration("#if (0, 2)\nCOMMA_RESULT_2\n#endif\n", configuration);

    assert_eq!(identifiers, ["COMMA_RESULT_2"]);
    assert!(
        errors
            .iter()
            .any(|error| error.severity() == ErrorSeverity::Warning
                && matches!(
                    error,
                    TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                            ExtensionPolicy::Warn
                        ),
                        ..
                    })
                )),
        "diagnostics: {errors:#?}"
    );
}

#[test]
fn strict_c99_does_not_diagnose_comma_in_short_circuited_rhs() {
    let (identifiers, errors) = preprocess_with_configuration(
        "#if 1 || (1, 2)\nSHORT_CIRCUIT_RESULT_1\n#endif\n",
        strict_c99(),
    );

    assert_eq!(identifiers, ["SHORT_CIRCUIT_RESULT_1"]);
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
}

#[test]
fn comma_in_unevaluated_conditional_middle_preserves_the_question_marker() {
    let (identifiers, errors) =
        preprocess_with_configuration("#if 0 ? 2, 3 : 0\nUNREACHABLE\n#endif\n", strict_c99());

    assert_eq!(identifiers, Vec::<String>::new());
    assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
}

/// Preprocesses `source` as the file `path`, returning every token.
fn tokens_of(source: &str, path: &str) -> (Vec<Token>, Context) {
    let mut context = Context::new();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from(path).into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    while let Some(token) = preprocessor.next_item(&mut context) {
        tokens.push(token);
    }
    (tokens, context)
}

fn string_value(context: &Context, token: Token) -> (bool, String) {
    match token.kind {
        | TokenType::String(StringTokenType::String(contents)) =>
            (false, context.string_cache.at(contents).to_owned()),
        | TokenType::String(StringTokenType::WideString(contents)) =>
            (true, context.string_cache.at(contents).to_owned()),
        | other => panic!("expected a string literal, found {other:?}"),
    }
}

#[test]
fn wide_literal_spellings_keep_their_opening_quote() {
    // C99 §6.10.3.2p2: `#` produces each argument token's spelling.
    let (tokens, context) = tokens_of("#define S(x) #x\nS(L'b') S(L\"w\")\n", "<test>");
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        string_value(&context, tokens[0]),
        (false, "L'b'L\"w\"".to_owned())
    );

    let (tokens, mut context) = tokens_of("L'\"'\n", "<test>");
    assert!(matches!(
        tokens[0].kind,
        TokenType::Character(CharacterTokenType::WideChar('"'))
    ));
    assert!(context.take_pending_errors().is_empty());

    // C99 §6.10.3.3p3: `L ## "ab"` pastes into the wide literal `L"ab"`.
    let (tokens, mut context) = tokens_of("#define W(x) L ## x\nW(\"ab\")\n", "<test>");
    assert_eq!(string_value(&context, tokens[0]), (true, "ab".to_owned()));
    assert!(context.take_pending_errors().is_empty());
}

#[test]
fn wide_string_contents_may_start_with_a_quote() {
    let (tokens, mut context) = tokens_of("L\"'x\"\n", "<test>");
    assert_eq!(string_value(&context, tokens[0]), (true, "'x".to_owned()));
    assert!(context.take_pending_errors().is_empty());
}

#[test]
fn file_and_line_are_spelled_as_c_tokens_at_their_use() {
    let path = r"C:\dir\Lab.c";
    let (tokens, mut context) = tokens_of("\n__FILE__ __LINE__\n", path);

    assert_eq!(string_value(&context, tokens[0]), (false, path.to_owned()));
    assert!(matches!(
        tokens[1].kind,
        TokenType::Integer(IntegerTokenType::Int(2))
    ));
    let line_vectors = context.get_source_vectors(tokens[1].source_vectors);
    assert_eq!((line_vectors[0].line, line_vectors[0].column), (2, 10));
    assert!(context.take_pending_errors().is_empty());
}

#[test]
fn conditional_groups_select_exactly_one_group() {
    for (source, expected) in [
        ("#ifndef NOPE\nyes\n#endif\n", &["yes"][..]),
        ("#define D\n#ifndef D\nno\n#endif\nafter\n", &["after"]),
        ("#define D\n#ifdef D\nyes\n#endif\n", &["yes"]),
        ("#if 1\nyes\n#else\nno\n#endif\n", &["yes"]),
        ("#if 1\nyes\n#elif 1\nno\n#endif\n", &["yes"]),
        ("#if 0\n#endif\nafter\n", &["after"]),
        ("#if 0\n#else\nyes\n#endif\n", &["yes"]),
        ("#if 0\nno\n#elif 1\nyes\n#else\nno2\n#endif\n", &["yes"]),
        (
            "#if 0\n#if 1\nno\n#else\nno2\n#endif\n#elif 0\nno3\n#else\nyes\n#endif\n",
            &["yes"],
        ),
    ] {
        let (identifiers, errors) = preprocess(source);
        assert_eq!(identifiers, expected, "{source:?}");
        assert!(errors.is_empty(), "{source:?}: {errors:#?}");
    }
}

#[test]
fn malformed_conditions_still_find_their_endif() {
    for source in ["#if 1.5\n#endif\nafter\n", "#if defined 1\n#endif\nafter\n"] {
        let (identifiers, errors) = preprocess(source);
        assert_eq!(identifiers, ["after"], "{source:?}");
        assert_eq!(errors.len(), 1, "{source:?}: {errors:#?}");
    }
    let (_, errors) = preprocess("#if defined +\n#endif\n");
    assert_eq!(errors.len(), 1, "{errors:#?}");
}

#[test]
fn unterminated_conditionals_point_at_their_directive() {
    let (tokens, mut context) = tokens_of("#if 1\nx\n", "<test>");
    assert_eq!(tokens.len(), 1);
    let errors = context.take_pending_errors();
    let [
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
            source_vectors,
        }),
    ] = errors.as_slice()
    else {
        panic!("expected one unterminated-conditional error: {errors:#?}");
    };
    assert_eq!(context.source_spelling(*source_vectors), Some("if"));
}

#[test]
fn identical_redefinitions_and_empty_definitions_are_accepted() {
    let (identifiers, errors) = preprocess(
        "#define A 1\n#define A  1\n#define F(a) a\n#define F(a) a\n#define E\n#define E\nafter\n",
    );
    assert_eq!(identifiers, ["after"]);
    assert!(errors.is_empty(), "{errors:#?}");

    for source in [
        "#define A 1\n#define A 2\n",
        "#define F(a) a\n#define F(b) b\n",
    ] {
        let (identifiers, errors) = preprocess(&format!("{source}after\n"));
        assert_eq!(identifiers, ["after"], "{source:?}");
        assert!(matches!(
            errors.as_slice(),
            [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(_),
                ..
            })]
        ));
    }
}

#[test]
fn function_like_macro_names_without_parentheses_are_not_invocations() {
    let (identifiers, errors) = preprocess("#define f(x) x\nf ;\n");
    assert_eq!(identifiers, ["f"]);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn quoted_includes_search_beside_the_including_file_not_the_working_directory() {
    let directory = std::env::temp_dir().join(format!("bcc-include-search-{}", std::process::id()));
    let nested = directory.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("sibling.h"), "from_sibling\n").unwrap();
    let main = nested.join("main.c");

    let (tokens, mut context) = tokens_of(
        "#include \"sibling.h\"\n#include <sibling.h>\n",
        main.to_str().unwrap(),
    );
    let identifiers: Vec<&str> = tokens
        .iter()
        .filter(|token| token.kind == TokenType::Identifier)
        .map(|token| context.string_cache.at(token.contents))
        .collect();
    assert_eq!(identifiers, ["from_sibling"]);
    let errors = context.take_pending_errors();
    assert!(
        matches!(
            errors.as_slice(),
            [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::HeaderNotFound {
                    is_system_header: true,
                    ..
                },
                ..
            })]
        ),
        "{errors:#?}"
    );
    drop(std::fs::remove_dir_all(&directory));
}
