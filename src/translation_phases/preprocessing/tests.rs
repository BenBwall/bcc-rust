mod conditional_boundary_regressions;
mod directive_regressions;
mod encoding_regressions;
mod expression_regressions;
mod header_name_regressions;
mod keyword_regressions;
mod literal_regressions;
mod macro_regressions;
mod observables;
mod other_token_regressions;
mod predefined_regressions;

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
        GetSourceVectors,
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

fn preprocess_with_configuration<R>(
    source: &str,
    configuration: CompilerConfiguration,
    inspect: impl FnOnce(Vec<String>, &[TranslationError<'_>]) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
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
    inspect(identifiers, &errors)
}

fn preprocess<R>(
    source: &str,
    inspect: impl FnOnce(Vec<String>, &[TranslationError<'_>]) -> R,
) -> R {
    preprocess_with_configuration(source, CompilerConfiguration::default(), inspect)
}

#[test]
fn adjacent_string_lookahead_restores_diagnostics_to_the_phase_context() {
    preprocess("\"a\" 0xg\n", |_, errors| {
        assert!(errors.iter().any(|error| matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
                ..
            })
        )));
    });
}

#[test]
fn adjacent_string_diagnostics_remain_in_source_order() {
    preprocess("\"\\q\" \"\\u1\"\n", |_, errors| {
        assert!(matches!(
            errors,
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
    });
}

#[test]
fn malformed_include_restores_include_tokenization_mode() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "#include 123 extra\nint x = a < b > c;\n",
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
        preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, ["sentinel"], "{source:?}");
            assert!(errors.iter().any(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
                    ..
                })
            )));
        });
    }
}

#[test]
fn malformed_macro_expanded_include_unwinds_before_the_following_line() {
    for source in [
        "#define BAD 123\n#include BAD extra\nint sentinel;\n",
        "#define BAD() 123\n#include BAD() extra\nint sentinel;\n",
        "#define VALUE 123\n#define BAD VALUE\n#include BAD extra\nint sentinel;\n",
    ] {
        preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, ["sentinel"], "{source:?}");
            assert!(errors.iter().any(|error| matches!(
                error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(_),
                    ..
                })
            )));
        });
    }
}

#[test]
fn malformed_macro_include_does_not_repeat_expansion_diagnostics() {
    let source = "#define BAD(x) x\n#include BAD(123,456) extra\nint sentinel;\n";
    preprocess(source, |identifiers, errors| {
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
    });
}

#[test]
fn phase_07_mapping_diagnoses_every_internal_only_token_kind() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<phase-7-totality-test>").into_boxed_path(),
        "",
        SharedVec::default(),
        SharedVec::default(),
    );
    let contents = context.string_cache.intern("internal-only");

    for kind in [
        PreprocessorTokenType::Placeholder,
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
    preprocess(&source, |identifiers, errors| {
        assert_eq!(identifiers, [identifier]);
        assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
    });
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
    preprocess("#if 1 : 2\nUNREACHABLE\n#endif\n", |_, errors| {
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
    });
}

#[test]
fn trailing_unmatched_colon_reports_a_diagnostic_without_panicking() {
    preprocess("#if 1 :\nRECOVERED\n#endif\n", |_, errors| {
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
    });
}

#[test]
fn default_extension_mode_evaluates_comma_to_rhs_without_a_diagnostic() {
    preprocess(
        "#if (1, 0)\nUNREACHABLE\n#endif\n",
        |identifiers, errors| {
            assert_eq!(identifiers, Vec::<String>::new());
            assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
        },
    );
}

#[test]
fn strict_c99_diagnoses_an_evaluated_comma_after_reducing_to_rhs() {
    preprocess_with_configuration(
        "#if (0, 2)\nCOMMA_RESULT_2\n#endif\n",
        strict_c99(),
        |identifiers, errors| {
            assert_eq!(identifiers, ["COMMA_RESULT_2"]);
            assert!(
                errors
                    .iter()
                    .any(|error| error.severity() == ErrorSeverity::Error
                        && matches!(
                            error,
                            TranslationError::Preprocessing(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                                        ExtensionPolicy::Deny
                                    ),
                                ..
                            })
                        )),
                "diagnostics: {errors:#?}"
            );
        },
    );
}

#[test]
fn warning_policy_reports_an_evaluated_comma_as_a_warning() {
    let configuration = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Warn);
    preprocess_with_configuration(
        "#if (0, 2)\nCOMMA_RESULT_2\n#endif\n",
        configuration,
        |identifiers, errors| {
            assert_eq!(identifiers, ["COMMA_RESULT_2"]);
            assert!(
                errors
                    .iter()
                    .any(|error| error.severity() == ErrorSeverity::Warning
                        && matches!(
                            error,
                            TranslationError::Preprocessing(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                                        ExtensionPolicy::Warn
                                    ),
                                ..
                            })
                        )),
                "diagnostics: {errors:#?}"
            );
        },
    );
}

#[test]
fn strict_c99_does_not_diagnose_comma_in_short_circuited_rhs() {
    preprocess_with_configuration(
        "#if 1 || (1, 2)\nSHORT_CIRCUIT_RESULT_1\n#endif\n",
        strict_c99(),
        |identifiers, errors| {
            assert_eq!(identifiers, ["SHORT_CIRCUIT_RESULT_1"]);
            assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
        },
    );
}

#[test]
fn comma_in_unevaluated_conditional_middle_preserves_the_question_marker() {
    preprocess_with_configuration(
        "#if 0 ? 2, 3 : 0\nUNREACHABLE\n#endif\n",
        strict_c99(),
        |identifiers, errors| {
            assert_eq!(identifiers, Vec::<String>::new());
            assert!(errors.is_empty(), "unexpected diagnostics: {errors:#?}");
        },
    );
}

/// Preprocesses `source` as the file `path`, keeping tokens and their context
/// alive for the inspection callback.
fn with_tokens_of<R>(
    source: &str,
    path: &str,
    inspect: impl FnOnce(&[Token], &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let mut preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from(path).into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    while let Some(token) = preprocessor.next_item(&mut context) {
        tokens.push(token);
    }
    inspect(&tokens, &mut context)
}

fn string_value(context: &Context<'_>, token: Token) -> (bool, String) {
    match token.kind {
        | TokenType::String(StringTokenType::String(contents)) => (
            false,
            context
                .literal_text(contents, false)
                .as_deref()
                .expect("UTF-8 test literal")
                .to_owned(),
        ),
        | TokenType::String(StringTokenType::WideString(contents)) => (
            true,
            context
                .literal_text(contents, true)
                .as_deref()
                .expect("UTF-8 test literal")
                .to_owned(),
        ),
        | other => panic!("expected a string literal, found {other:?}"),
    }
}

#[test]
fn wide_literal_spellings_keep_their_opening_quote() {
    // C99 §6.10.3.2p2: `#` produces each argument token's spelling.
    with_tokens_of(
        "#define S(x) #x\nS(L'b') S(L\"w\")\n",
        "<test>",
        |tokens, context| {
            assert_eq!(tokens.len(), 1);
            assert_eq!(
                string_value(context, tokens[0]),
                (false, "L'b'L\"w\"".to_owned())
            );
        },
    );

    with_tokens_of("L'\"'\n", "<test>", |tokens, context| {
        assert!(matches!(
            tokens[0].kind,
            TokenType::Character(CharacterTokenType::WideChar(34))
        ));
        assert!(context.take_pending_errors().is_empty());
    });

    // C99 §6.10.3.3p3: `L ## "ab"` pastes into the wide literal `L"ab"`.
    with_tokens_of(
        "#define W(x) L ## x\nW(\"ab\")\n",
        "<test>",
        |tokens, context| {
            assert_eq!(string_value(context, tokens[0]), (true, "ab".to_owned()));
            assert!(context.take_pending_errors().is_empty());
        },
    );
}

#[test]
fn wide_string_contents_may_start_with_a_quote() {
    with_tokens_of("L\"'x\"\n", "<test>", |tokens, context| {
        assert_eq!(string_value(context, tokens[0]), (true, "'x".to_owned()));
        assert!(context.take_pending_errors().is_empty());
    });
}

#[test]
fn file_and_line_are_spelled_as_c_tokens_at_their_use() {
    let path = r"C:\dir\Lab.c";
    with_tokens_of("\n__FILE__ __LINE__\n", path, |tokens, context| {
        assert_eq!(string_value(context, tokens[0]), (false, path.to_owned()));
        assert!(matches!(
            tokens[1].kind,
            TokenType::Integer(IntegerTokenType::Int(2))
        ));
        let line_vectors = context.get_source_vectors(tokens[1].source_vectors);
        assert_eq!((line_vectors[0].line, line_vectors[0].column), (2, 10));
        assert!(context.take_pending_errors().is_empty());
    });
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
        preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, expected, "{source:?}");
            assert!(errors.is_empty(), "{source:?}: {errors:#?}");
        });
    }
}

#[test]
fn malformed_conditions_still_find_their_endif() {
    for source in ["#if 1.5\n#endif\nafter\n", "#if defined 1\n#endif\nafter\n"] {
        preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, ["after"], "{source:?}");
            assert_eq!(errors.len(), 1, "{source:?}: {errors:#?}");
        });
    }
    preprocess("#if defined +\n#endif\n", |_, errors| {
        assert_eq!(errors.len(), 1, "{errors:#?}");
    });
}

#[test]
fn unterminated_conditionals_point_at_their_directive() {
    with_tokens_of("#if 1\nx\n", "<test>", |tokens, context| {
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
    });
}

#[test]
fn identical_redefinitions_and_empty_definitions_are_accepted() {
    preprocess(
        "#define A 1\n#define A  1\n#define F(a) a\n#define F(a) a\n#define E\n#define E\nafter\n",
        |identifiers, errors| {
            assert_eq!(identifiers, ["after"]);
            assert!(errors.is_empty(), "{errors:#?}");
        },
    );

    for source in [
        "#define A 1\n#define A 2\n",
        "#define F(a) a\n#define F(b) b\n",
    ] {
        preprocess(&format!("{source}after\n"), |identifiers, errors| {
            assert_eq!(identifiers, ["after"], "{source:?}");
            assert!(matches!(
                errors,
                [TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(_),
                    ..
                })]
            ));
        });
    }
}

#[test]
fn function_like_macro_names_without_parentheses_are_not_invocations() {
    preprocess("#define f(x) x\nf ;\n", |identifiers, errors| {
        assert_eq!(identifiers, ["f"]);
        assert!(errors.is_empty(), "{errors:#?}");
    });
}

#[test]
fn quoted_includes_search_beside_the_including_file_not_the_working_directory() {
    let directory = std::env::temp_dir().join(format!("bcc-include-search-{}", std::process::id()));
    let nested = directory.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("sibling.h"), "from_sibling\n").unwrap();
    let main = nested.join("main.c");

    with_tokens_of(
        "#include \"sibling.h\"\n#include <sibling.h>\n",
        main.to_str().unwrap(),
        |tokens, context| {
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
        },
    );
    drop(std::fs::remove_dir_all(&directory));
}

/// Preprocesses `source`, spelling each token as written in C source with a
/// space between tokens.
fn expansion_of<R>(source: &str, inspect: impl FnOnce(String, &[TranslationError<'_>]) -> R) -> R {
    with_tokens_of(source, "<test>", |tokens, context| {
        let spellings: Vec<String> = tokens
            .iter()
            .map(|&token| match token.kind {
                | TokenType::String(StringTokenType::String(contents)) => format!(
                    "{:?}",
                    context
                        .literal_text(contents, false)
                        .as_deref()
                        .expect("UTF-8 test literal")
                ),
                | _ => context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0')
                    .to_owned(),
            })
            .collect();
        let errors = context.take_pending_errors();
        inspect(spellings.join(" "), &errors)
    })
}

#[track_caller]
fn assert_expansion(source: &str, expected: &str) {
    expansion_of(source, |expansion, errors| {
        assert_eq!(expansion, expected, "{source}");
        assert!(errors.is_empty(), "{source}: {errors:#?}");
    });
}

#[test]
fn commas_inside_nested_parentheses_do_not_separate_macro_arguments() {
    // C99 §6.10.3p11: commas between matching inner parentheses do not
    // separate arguments.
    for (source, expected) in [
        ("#define F(x) x\nF((a, b))\n", "( a , b )"),
        ("#define F(x) [x]\nF(g(a, b))\n", "[ g ( a , b ) ]"),
        (
            "#define STR(x) #x\n#define NAME(p, i) p ## i\nSTR(NAME(a, 0))\n",
            "\"NAME(a, 0)\"",
        ),
        (
            "#define MIN(a, b) ((a) < (b) ? (a) : (b))\n#define MAX(a, b) ((a) > (b) ? (a) : \
             (b))\n#define CLAMP(x, lo, hi) MIN(MAX(x, lo), hi)\nCLAMP(v, 0, 9)\n",
            "( ( ( ( v ) > ( 0 ) ? ( v ) : ( 0 ) ) ) < ( 9 ) ? ( ( ( v ) > ( 0 ) ? ( v ) : ( 0 ) \
             ) ) : ( 9 ) )",
        ),
        (
            "#define V(x, ...) [x] __VA_ARGS__\nV(f(a, b), c, (d, e))\n",
            "[ f ( a , b ) ] c , ( d , e )",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn enclosing_parameters_are_replaced_before_a_nested_macro_takes_its_operand() {
    // C99 §6.10.3.1p1: each parameter is replaced by its fully macro-replaced
    // argument before the nested invocation is rescanned.
    for (source, expected) in [
        (
            "#define CAT(a, b) a ## b\n#define RECORD(i) CAT(record_, i)\nRECORD(7)\n",
            "record_7",
        ),
        (
            "#define CAT(a, b) a ## b\n#define SUFFIX(i) CAT(i, _x)\nSUFFIX(y)\n",
            "y_x",
        ),
        (
            "#define CAT(a, b) a ## b\n#define SEVEN 7\n#define RECORD(i) CAT(record_, \
             i)\nRECORD(SEVEN) CAT(record_, SEVEN)\n",
            "record_7 record_SEVEN",
        ),
        (
            "#define CAT(a, b) a ## b\n#define PAIR(i, j) CAT(i, j) CAT(j, i)\nPAIR(x, y)\n",
            "xy yx",
        ),
        // An empty enclosing argument is a placemarker operand.
        (
            "#define CAT(a, b) a ## b
#define E(i) CAT(i, z) CAT(i, i) w
E()
",
            "z w",
        ),
        (
            "#define STR(x) #x\n#define XSTR(x) STR(x)\n#define FOO bar\nXSTR(foo); XSTR(FOO); \
             STR(FOO)\n",
            "\"foo\" ; \"bar\" ; \"FOO\"",
        ),
        // C99 §6.10.3.5 EXAMPLE 4's `xstr(INCFILE(2).h)` combines both rules.
        (
            "#define str(s) # s\n#define xstr(s) str(s)\n#define INCFILE(n) vers ## \
             n\nxstr(INCFILE(2).h)\n",
            "\"vers2.h\"",
        ),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn numbers_paste_into_preprocessing_numbers() {
    // C99 §6.4.8: a pp-number continues with identifier characters, and with
    // a sign after an exponent letter.
    assert_expansion(
        "#define CAT(a, b) a ## b\nCAT(1, e5) CAT(0x, 1fu)\n",
        "1e5 0x1fu",
    );
    assert_expansion(
        "#define CAT(a, b) a ## b\n#define S(x) #x\n#define XS(x) S(x)\nXS(CAT(1e, +)) ; \
         XS(CAT(0x1p, -))\n",
        "\"1e+\" ; \"0x1p-\"",
    );

    // Neither a sign without an exponent nor an identifier followed by an
    // exponent sign forms one token, and diagnostics spell numbers as source.
    expansion_of(
        "#define CAT(a, b) a ## b\nCAT(1, +) CAT(x, 1e+5)\n",
        |_, errors| {
            let pastes: Vec<_> = errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::TokenMergingError(lhs, rhs),
                        ..
                    }) => Some((*lhs, *rhs)),
                    | _ => None,
                })
                .collect();
            assert_eq!(pastes, [("1", "+"), ("x", "1e+5")], "{errors:#?}");
        },
    );
}

#[test]
fn replayed_operands_report_their_source_locations() {
    let source = "#define CAT(a, b) a ## b\n#define O(i) CAT(i, 1 / 0 + 1)\n#if O(2)\n#endif\n";
    with_tokens_of(source, "<test>", |_, context| {
        let errors = context.take_pending_errors();
        let [error] = errors.as_slice() else {
            panic!("expected one diagnostic: {errors:#?}");
        };
        assert!(matches!(
            error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::DivideByZero,
                ..
            })
        ));
        let source_vectors = error.source_vectors(context);
        let location = &context.get_source_vectors(source_vectors)[0];
        assert_eq!(
            (
                context.get_source_file(location.source_file_index).to_str(),
                location.line,
            ),
            (Some("<test>"), 2)
        );
    });

    assert_expansion(
        "#define CAT(a, b) a ## b\n#define F(i) CAT(i, x __FILE__)\nF(y)\n",
        "yx \"<test>\"",
    );
}

#[test]
fn parameters_hide_macros_of_the_same_name() {
    // C99 §6.10.3.1: parameters are replaced before the replacement list is
    // rescanned, and `##` results are not parameters.
    for (source, expected) in [
        (
            "#define i 3\n#define TWICE(i) (i + i)\nTWICE(x)\n",
            "( x + x )",
        ),
        ("#define f(f) [f]\nf(f(1))\n", "[ [ 1 ] ]"),
        ("#define a 9\n#define G(a) a(a)\nG(a)\n", "9 ( 9 )"),
        ("#define F(xy) x ## y\nF(1)\n", "xy"),
    ] {
        assert_expansion(source, expected);
    }
}

#[test]
fn variadic_macros_may_omit_the_variable_arguments() {
    for (source, expected) in [
        (
            "#define V(x, ...) [x __VA_ARGS__]\nV(a) after\n",
            "[ a ] after",
        ),
        (
            "#define V(x, ...) [x __VA_ARGS__]\nV(a,) after\n",
            "[ a ] after",
        ),
        (
            "#define CAT(a, ...) a ## __VA_ARGS__\nCAT(p) CAT(p, q)\n",
            "p pq",
        ),
    ] {
        assert_expansion(source, expected);
    }

    preprocess(
        "#define W(x, y, ...) x y __VA_ARGS__\nW(a) after\n",
        |_, errors| {
            assert!(
                matches!(
                errors,
                [TranslationError::Preprocessing(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                            expected: 2,
                            found:    1,
                        },
                    ..
                })]
            ),
                "{errors:#?}"
            );
        },
    );

    // C99 §6.10.3p4 requires an argument for `...`.
    let source = "#define V(x, ...) x __VA_ARGS__\nV(a) after\n";
    for (policy, severity) in [
        (ExtensionPolicy::Warn, ErrorSeverity::Warning),
        (ExtensionPolicy::Deny, ErrorSeverity::Error),
    ] {
        let configuration = CompilerConfiguration::new(CStandard::C99, policy);
        preprocess_with_configuration(source, configuration, |identifiers, errors| {
            assert_eq!(identifiers, ["a", "after"]);
            assert!(
                matches!(
                    errors,
                    [error @ TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::MissingVariadicArgument(found),
                        ..
                    })] if *found == policy && error.severity() == severity
                ),
                "{errors:#?}"
            );
        });
    }
}

#[test]
fn whitespace_is_never_a_paste_operand() {
    // C99 §6.10.3.3p2: `##` pastes the preprocessing tokens beside it, and an
    // argument of only whitespace is a placemarker.
    for (source, expected) in [
        (
            "#define CAT(a, b) a ## b\nCAT( , x) CAT(y, ) CAT( p , q )\n",
            "x y pq",
        ),
        (
            "#define CAT(a, b) a ## b\n#define S(x) #x\n#define XS(x) S(x)\nXS(CAT( a , b )) ; \
             XS(CAT( , c ))\n",
            "\"ab\" ; \"c\"",
        ),
    ] {
        assert_expansion(source, expected);
    }
}
