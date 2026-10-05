use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

use super::{
    Context,
    IntegerTokenType,
    Preprocessor,
    PreprocessorError,
    PreprocessorErrorType,
    SharedVec,
    StringTokenType,
    Token,
    TokenType,
    TranslationError,
};
#[derive(Debug)]
struct TemporaryHeaders(PathBuf);

impl TemporaryHeaders {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "bcc-directive-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn write(&self, name: &str, source: &str) {
        std::fs::write(self.0.join(name), source).unwrap();
    }
}

impl Drop for TemporaryHeaders {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn with_directive_tokens_at_path<R>(
    source: &str,
    path: &Path,
    inspect: impl FnOnce(&[Token], &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        path.to_path_buf().into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let tokens = preprocessor.preprocess_all(&mut context);
    inspect(&tokens, &mut context)
}

fn with_directive_tokens_with_system_directory<R>(
    source: &str,
    path: &Path,
    system_directory: &Path,
    inspect: impl FnOnce(&[Token], &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        path.to_path_buf().into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::from(vec![system_directory.to_path_buf()]),
    );
    let tokens = preprocessor.preprocess_all(&mut context);
    inspect(&tokens, &mut context)
}

fn with_directive_tokens<R>(
    source: &str,
    inspect: impl FnOnce(&[Token], &mut Context<'_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<directive-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut tokens = Vec::new();
    preprocessor.for_each_item(&mut context, |_, token| {
        tokens.push(token);
    });
    inspect(&tokens, &mut context)
}

#[test]
fn pragma_destringizing_preserves_non_special_escapes() {
    for (literal, expected) in [
        (r#""unknown \n""#, "unknown \\n\n"),
        (r#""unknown \t""#, "unknown \\t\n"),
        (r#""unknown \"quote\" \\""#, "unknown \"quote\" \\\n"),
        (r#"L"unknown café""#, "unknown café\n"),
        (r#""""#, "\n"),
    ] {
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let preprocess_arena = crate::util::bump::Bump::new();
        let mut preprocessor = Preprocessor::new(
            &preprocess_arena,
            &mut context,
            PathBuf::from("<pragma-test>").into_boxed_path(),
            "",
            SharedVec::default(),
            SharedVec::default(),
        );
        let literal = context.string_cache.intern(literal);
        let prepared = preprocessor.run(&mut context, |preprocessor| {
            std::ops::ControlFlow::Break(
                crate::translation_phases::preprocessing::Expander::prepare_pragma_operator_string(
                    preprocessor.context,
                    literal,
                )
                .to_string(),
            )
        });
        assert_eq!(prepared, expected);
    }
}

#[test]
fn enormous_line_number_diagnoses_without_panicking_and_retains_its_digits() {
    let digits = "9".repeat(1000);
    let source = format!("#line {digits}\nafter\n");
    with_directive_tokens(&source, |tokens, context| {
        assert_eq!(tokens.len(), 1);
        assert_eq!(context.string_cache.at(tokens[0].contents), "after");
        let errors = context.take_pending_errors();
        assert!(matches!(
            errors.as_slice(),
            [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::LineDirectiveNumberTooLarge(..),
                ..
            })]
        ));
        assert_eq!(
            errors[0].to_string(),
            format!("line number {digits} is out of range")
        );
    });
}

#[test]
fn missing_line_number_preserves_the_following_line() {
    for (source, directive_line) in [
        ("#line\nint kept;\n", 1),
        ("#define EMPTY\n#line EMPTY\nint kept;\n", 2),
        ("#line wrong discarded\nint kept;\n", 1),
    ] {
        with_directive_tokens_at_path(source, Path::new("main.c"), |tokens, context| {
            assert_eq!(
                tokens
                    .iter()
                    .map(|token| context.string_cache.at(token.contents))
                    .collect::<Vec<_>>(),
                ["int", "kept", ";"],
                "{source:?}"
            );
            let kept = context.first_source_vector(tokens[0].source_vectors);
            assert_eq!(kept.line, directive_line + 1, "{source:?}");
            let errors = context.take_pending_errors();
            let [TranslationError::Preprocessing(error)] = errors.as_slice() else {
                panic!("{source:?}: {errors:#?}");
            };
            assert!(matches!(
                error.error_type,
                PreprocessorErrorType::MissingNumberInLineDirective(_)
            ));
            let location = context.first_source_vector(error.source_vectors);
            assert_eq!(location.line, directive_line, "{source:?}");
        });
    }
}

#[test]
fn pragma_expectations_preserve_tokens_across_source_boundaries() {
    let headers = TemporaryHeaders::new();
    headers.write(
        "macros.h",
        "#define BAD 123\n#define TEXT \"STDC FP_CONTRACT ON\"\n#define OPEN (\n\
         #define CLOSE )\n#define P _Pragma\n#define EMPTY\n",
    );
    for (operand, recovered, expected_errors, error_in_header) in [
        ("_Pragma(BAD)\n", vec!["123", ")"], 1, true),
        ("_Pragma BAD\n", vec!["123"], 2, true),
        ("_Pragma(TEXT BAD\n", vec!["123"], 1, true),
        ("_Pragma(EMPTY)\n", vec![")"], 1, false),
        ("_Pragma(\n", vec![], 1, false),
        ("_Pragma(TEXT\n", vec![], 1, false),
        ("_Pragma(TEXT ", vec![], 1, false),
        ("_Pragma ", vec![], 2, false),
        ("_Pragma(\"STDC FP_CONTRACT ON\")\n", vec![], 0, false),
        ("P OPEN TEXT CLOSE\n", vec![], 0, false),
    ] {
        let source = format!("#include \"macros.h\"\n{operand}int after;\n");
        with_directive_tokens_at_path(&source, &headers.0.join("main.c"), |tokens, context| {
            let mut expected = recovered;
            expected.extend(["int", "after", ";"]);
            assert_eq!(
                tokens
                    .iter()
                    .map(|token| {
                        context.string_cache.at(token.contents).trim_end_matches('\0')
                    })
                    .collect::<Vec<_>>(),
                expected,
                "{source:?}"
            );
            let after = context.first_source_vector(tokens[tokens.len() - 2].source_vectors);
            assert_eq!(
                context.get_source_file(after.source_file_index),
                headers.0.join("main.c").as_path(),
                "{source:?}"
            );
            assert_eq!(after.line, if operand.ends_with('\n') { 3 } else { 2 });
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), expected_errors, "{source:?}: {errors:#?}");
            for error in errors {
                let TranslationError::Preprocessing(error) = error else {
                    panic!("{source:?}: {error:#?}");
                };
                assert!(matches!(
                    error.error_type,
                    PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(_)
                        | PreprocessorErrorType::MissingStringLiteralInPragmaOperator(_)
                        | PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(_)
                ));
                let location = context.first_source_vector(error.source_vectors);
                let expected_file = headers.0.join(if error_in_header {
                    "macros.h"
                } else {
                    "main.c"
                });
                assert_eq!(
                    context.get_source_file(location.source_file_index),
                    expected_file.as_path()
                );
                assert_eq!(location.line, if error_in_header { 1 } else { 2 });
            }
        });
    }
}

#[test]
fn unfinished_pragma_expectations_terminate_at_end_of_input() {
    for source in ["_Pragma", "_Pragma(", "_Pragma(\"STDC FP_CONTRACT ON\""] {
        with_directive_tokens(source, |tokens, context| {
            assert!(tokens.is_empty(), "{source:?}: {tokens:#?}");
            assert!(!context.take_pending_errors().is_empty(), "{source:?}");
        });
    }
}

#[test]
fn line_directive_applies_to_the_following_line_and_strips_file_delimiters() {
    for source in [
        "#line 10 \"logical.c\"\n__FILE__; __LINE__\n",
        "#define NUMBER 10\n#define FILE \"logical.c\"\n#line NUMBER FILE\n__FILE__; __LINE__\n",
    ] {
        with_directive_tokens(source, |tokens, context| {
            assert_eq!(tokens.len(), 3, "{tokens:#?}");
            let TokenType::String(StringTokenType::String(contents)) = tokens[0].kind else {
                panic!("expected __FILE__ string: {tokens:#?}");
            };
            assert_eq!(
                context
                    .literal_text_in(context.tu_arena(), contents, false)
                    .expect("UTF-8 test literal"),
                "logical.c"
            );
            assert_eq!(
                tokens[2].kind,
                TokenType::Integer(IntegerTokenType::Int(10))
            );
            let vector = context
                .get_source_vectors(tokens[0].source_vectors)
                .first()
                .unwrap();
            assert_eq!(vector.line, 10);
            assert_eq!(
                context.get_source_file(vector.source_file_index),
                PathBuf::from("logical.c").as_path()
            );
            assert!(context.take_pending_errors().is_empty());
        });
    }
}

#[test]
fn direct_pragmas_consume_the_entire_directive_line() {
    for source in [
        "#pragma unknown token token2\nafter\n",
        "#pragma STDC unknown token token2\nafter\n",
        "#pragma STDC FP_CONTRACT bad token2\nafter\n",
        "#pragma STDC FP_CONTRACT ON\nafter\n",
        "_Pragma(\"unknown token token2\")\nafter\n",
    ] {
        with_directive_tokens(source, |tokens, context| {
            assert_eq!(
                tokens.len(),
                1,
                "pragma leaked tokens: {source:?}: {tokens:#?}"
            );
            assert_eq!(context.string_cache.at(tokens[0].contents), "after");
            let source = context
                .get_source_vectors(tokens[0].source_vectors)
                .first()
                .unwrap();
            assert_eq!(source.line, 2);
            assert_eq!(source.column, 1);
        });
    }
}

#[test]
fn repeated_line_directives_replace_the_presumed_line() {
    with_directive_tokens(
        "#line 30\n__LINE__\n#line 70\n__LINE__\n",
        |tokens, context| {
            assert_eq!(tokens.len(), 2);
            assert_eq!(
                tokens[0].kind,
                TokenType::Integer(IntegerTokenType::Int(30))
            );
            assert_eq!(
                tokens[1].kind,
                TokenType::Integer(IntegerTokenType::Int(70))
            );
            assert!(context.take_pending_errors().is_empty());
        },
    );
}

#[test]
fn line_filename_escapes_follow_string_literal_rules() {
    with_directive_tokens(
        "#line 10 \"dir\\\\file.c\"\n__FILE__\n",
        |tokens, context| {
            let TokenType::String(StringTokenType::String(contents)) = tokens[0].kind else {
                panic!("expected file name: {tokens:#?}");
            };
            assert_eq!(
                context
                    .literal_text_in(context.tu_arena(), contents, false)
                    .expect("UTF-8 test literal"),
                "dir\\file.c"
            );
            assert!(context.take_pending_errors().is_empty());
        },
    );
}

#[test]
fn line_filename_may_be_generated_by_stringification() {
    with_directive_tokens_at_path(
        "#define S(x) #x\n#line 20 S(logical.c)\n__FILE__; __LINE__\n",
        Path::new("<directive-test>"),
        |tokens, context| {
            assert_eq!(tokens.len(), 3);
            let TokenType::String(StringTokenType::String(contents)) = tokens[0].kind else {
                panic!("expected __FILE__ string: {tokens:#?}");
            };
            assert_eq!(
                context
                    .literal_text_in(context.tu_arena(), contents, false)
                    .expect("UTF-8 test literal"),
                "logical.c"
            );
            assert_eq!(
                tokens[2].kind,
                TokenType::Integer(IntegerTokenType::Int(20))
            );
            assert!(context.take_pending_errors().is_empty());
        },
    );
}

#[test]
fn malformed_undef_tail_is_diagnosed_and_not_emitted_as_code() {
    with_directive_tokens_at_path(
        "#define X 1\n#undef X bad tail\nX after\n",
        Path::new("<directive-test>"),
        |tokens, context| {
            let names: Vec<_> = tokens
                .iter()
                .map(|token| context.string_cache.at(token.contents))
                .collect();
            assert_eq!(names, ["X", "after"]);
            let errors = context.take_pending_errors();
            let [TranslationError::Preprocessing(error)] = errors.as_slice() else {
                panic!("expected one undef diagnostic: {errors:#?}");
            };
            assert!(matches!(
                error.error_type,
                PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(..)
            ));
            let vector = &context.get_source_vectors(error.source_vectors)[0];
            assert_eq!((vector.line, vector.column), (2, 10));
        },
    );
}

#[test]
fn malformed_line_tails_do_not_escape_or_remap_the_directive_line() {
    for source in [
        "#line 30 bad tail\nafter\n",
        "#line 30 \"fake.c\" bad tail\nafter\n",
    ] {
        with_directive_tokens_at_path(source, Path::new("<directive-test>"), |tokens, context| {
            assert_eq!(tokens.len(), 1);
            assert_eq!(context.string_cache.at(tokens[0].contents), "after");
            let errors = context.take_pending_errors();
            let [TranslationError::Preprocessing(error)] = errors.as_slice() else {
                panic!("expected one line directive diagnostic: {errors:#?}");
            };
            assert!(matches!(
                error.error_type,
                PreprocessorErrorType::MissingNewlineAfterLineDirective(..)
            ));
            let vector = &context.get_source_vectors(error.source_vectors)[0];
            assert_eq!(vector.line, 1);
            assert_eq!(
                context.get_source_file(vector.source_file_index),
                Path::new("<directive-test>")
            );
            let after = &context.get_source_vectors(tokens[0].source_vectors)[0];
            assert_eq!(after.line, 2);
        });
    }
}

#[test]
fn recursive_include_reports_limit_once_and_preserves_surviving_input() {
    let headers = TemporaryHeaders::new();
    headers.write("loop.h", "#include \"loop.h\"\nheader_after\n");
    with_directive_tokens_at_path(
        "#include \"loop.h\"\ncaller_after\n",
        &headers.0.join("main.c"),
        |tokens, context| {
            assert_eq!(
                tokens.len(),
                201,
                "expected 200 included files and the caller"
            );
            assert!(
                tokens[..200]
                    .iter()
                    .all(|token| context.string_cache.at(token.contents) == "header_after")
            );
            assert_eq!(
                context.string_cache.at(tokens[200].contents),
                "caller_after"
            );
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:#?}");
            assert!(
                errors[0].to_string().contains("include nesting"),
                "{}",
                errors[0]
            );
        },
    );
}

#[test]
fn include_nesting_supports_at_least_fifteen_header_levels() {
    let headers = TemporaryHeaders::new();
    for level in 0..15 {
        let source = if level == 14 {
            format!("level_{level}\n")
        } else {
            format!("#include \"level{}.h\"\nlevel_{level}\n", level + 1)
        };
        headers.write(&format!("level{level}.h"), &source);
    }
    with_directive_tokens_at_path(
        "#include \"level0.h\"\ncaller_after\n",
        &headers.0.join("main.c"),
        |tokens, context| {
            let actual: Vec<_> = tokens
                .iter()
                .map(|token| context.string_cache.at(token.contents).to_owned())
                .collect();
            let mut expected: Vec<_> = (0..15)
                .rev()
                .map(|level| format!("level_{level}"))
                .collect();
            expected.push("caller_after".to_owned());
            assert_eq!(actual, expected);
            assert!(context.take_pending_errors().is_empty());
        },
    );
}

#[test]
fn repeated_nonrecursive_includes_do_not_count_toward_nesting_limit() {
    let headers = TemporaryHeaders::new();
    headers.write("plain.h", "included\n");
    let source = format!("{}caller_after\n", "#include \"plain.h\"\n".repeat(250));
    with_directive_tokens_at_path(&source, &headers.0.join("main.c"), |tokens, context| {
        assert_eq!(tokens.len(), 251);
        assert!(
            tokens[..250]
                .iter()
                .all(|token| context.string_cache.at(token.contents) == "included")
        );
        assert_eq!(
            context.string_cache.at(tokens[250].contents),
            "caller_after"
        );
        assert!(context.take_pending_errors().is_empty());
    });
}

#[test]
fn conditional_arms_reject_duplicate_else_and_late_elif() {
    for (source, expected) in [
        (
            "#if 0\n#else\nyes\n#else\nwrong\n#endif\nafter\n",
            "`#else` after `#else`",
        ),
        (
            "#if 1\nyes\n#else\n#else\nwrong\n#endif\nafter\n",
            "`#else` after `#else`",
        ),
        (
            "#if 0\n#else\nyes\n#elif 1\nwrong\n#endif\nafter\n",
            "`#elif` after `#else`",
        ),
    ] {
        super::preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, ["yes", "after"]);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
            assert_eq!(errors[0].to_string(), expected);
        });
    }
}

#[test]
fn conditional_tails_diagnose_without_leaking_tokens() {
    for source in [
        "#if 0\n#else extra\nyes\n#endif\nafter\n",
        "#if 1\nyes\n#endif extra\nafter\n",
    ] {
        super::preprocess(source, |identifiers, errors| {
            assert_eq!(identifiers, ["yes", "after"]);
            assert_eq!(errors.len(), 1, "{source}: {errors:#?}");
        });
    }
}

#[test]
fn null_and_undef_directives_preserve_start_of_line() {
    let source = "#\n#define A 1\n#undef A\n#ifdef A\nwrong\n#else\nyes\n#endif\nafter\n";
    super::preprocess(source, |identifiers, errors| {
        assert_eq!(identifiers, ["yes", "after"]);
        assert!(errors.is_empty(), "{errors:#?}");
    });
}

#[test]
fn line_remapping_preserves_physical_include_lookup_and_pragma_once() {
    let headers = TemporaryHeaders::new();
    headers.write(
        "header.h",
        "#line 30 \"virtual/header.c\"\n#pragma once\nfirst\n",
    );
    let source =
        "#line 10 \"virtual/main.c\"\n#include \"header.h\"\n#include \"header.h\"\nafter\n";
    with_directive_tokens_at_path(source, &headers.0.join("main.c"), |tokens, context| {
        let names = tokens
            .iter()
            .filter(|token| token.kind == TokenType::Identifier)
            .map(|token| context.string_cache.at(token.contents))
            .collect::<Vec<_>>();
        assert_eq!(names, ["first", "after"], "");
        assert!(context.take_pending_errors().is_empty());
    });
}

#[test]
fn stringified_include_operands_resolve_headers_and_consume_their_tail() {
    let headers = TemporaryHeaders::new();
    std::fs::write(headers.0.join("generated.h"), "inside\n").unwrap();
    let source = "#define S(x) #x\n#include S(generated.h)\nafter\n";
    with_directive_tokens_at_path(source, &headers.0.join("main.c"), |tokens, context| {
        assert!(context.take_pending_errors().is_empty());
        assert_eq!(
            tokens
                .iter()
                .map(|token| context.string_cache.at(token.contents))
                .collect::<Vec<_>>(),
            ["inside", "after"]
        );
    });
}

#[test]
fn macro_include_tails_are_checked_in_the_callers_source_file() {
    let headers = TemporaryHeaders::new();
    headers.write("a.h", "int inside;\n");
    for (definitions, operand) in [
        ("#define H \"a.h\"\n", "H"),
        ("#define H <a.h>\n", "H"),
        ("#define H \"a.h\"\n#define ALIAS H\n", "ALIAS"),
        ("#define S(x) #x\n", "S(a.h)"),
    ] {
        for tail in [" int injected;", " EMPTY", ""] {
            let source = format!(
                "#define EMPTY\n{definitions}#include {operand}{tail}\nint after;\n"
            );
            with_directive_tokens_with_system_directory(
                &source,
                &headers.0.join("main.c"),
                &headers.0,
                |tokens, context| {
                    assert_eq!(
                        tokens
                            .iter()
                            .map(|token| context.string_cache.at(token.contents))
                            .collect::<Vec<_>>(),
                        ["int", "inside", ";", "int", "after", ";"],
                        "{source:?}"
                    );
                    let errors = context.take_pending_errors();
                    if tail != " int injected;" {
                        assert!(errors.is_empty(), "{source:?}: {errors:#?}");
                        return;
                    }
                    let [TranslationError::Preprocessing(error)] = errors.as_slice() else {
                        panic!("{source:?}: {errors:#?}");
                    };
                    assert!(matches!(
                        error.error_type,
                        PreprocessorErrorType::ExtraTokensAfterIncludeDirective
                    ));
                    let location = context.first_source_vector(error.source_vectors);
                    assert_eq!(location.line as usize, definitions.lines().count() + 2);
                    assert_eq!(location.column as usize, operand.len() + 11);
                    assert_eq!(
                        context.get_source_file(location.source_file_index),
                        headers.0.join("main.c").as_path()
                    );
                },
            );
        }
    }
}
#[test]
fn unfinished_include_does_not_consume_the_following_line() {
    with_directive_tokens_at_path(
        "#include <bcc_missing_header\nafter\n",
        Path::new("test.c"),
        |tokens, context| {
            assert_eq!(tokens.len(), 1, "");
            assert_eq!(context.string_cache.at(tokens[0].contents), "after");
            assert!(
                !context
                    .take_pending_errors()
                    .iter()
                    .any(|error| error.to_string().contains("extra tokens"))
            );
        },
    );
}

#[test]
fn terminal_angle_include_stays_in_its_source_file() {
    let headers = TemporaryHeaders::new();
    headers.write("a.h", "from_header\n");
    for (nested, found) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = if found { "a.h" } else { "missing.h" };
        let terminal = format!("#include <{name}>");
        let source = if nested {
            headers.write("middle.h", &terminal);
            "#include \"middle.h\"\nparent_after\n".to_owned()
        } else {
            terminal
        };
        with_directive_tokens_with_system_directory(
            &source,
            &headers.0.join("main.c"),
            &headers.0,
            |tokens, context| {
                let names: Vec<_> = tokens
                    .iter()
                    .map(|token| context.string_cache.at(token.contents))
                    .collect();
                let expected = match (nested, found) {
                    | (false, true) => vec!["from_header"],
                    | (false, false) => vec![],
                    | (true, true) => vec!["from_header", "parent_after"],
                    | (true, false) => vec!["parent_after"],
                };
                assert_eq!(names, expected, "nested={nested}, found={found}");
                let errors = context.take_pending_errors();
                assert_eq!(
                    errors
                        .iter()
                        .filter(|error| error.to_string().contains("no newline at end of file"))
                        .count(),
                    1,
                    "nested={nested}, found={found}: {errors:#?}"
                );
                assert_eq!(
                    errors
                        .iter()
                        .filter(|error| error.to_string().contains("cannot find header"))
                        .count(),
                    usize::from(!found),
                    "nested={nested}, found={found}: {errors:#?}"
                );
                assert!(
                    errors.iter().all(|error| {
                        !error.to_string().contains("unexpected end of file")
                            && !error.to_string().contains("extra tokens")
                    }),
                    "nested={nested}, found={found}: {errors:#?}"
                );
            },
        );
    }
}

#[test]
fn terminal_include_variants_stay_in_their_source_file() {
    let headers = TemporaryHeaders::new();
    headers.write("a.h", "from_header\n");
    for found in [true, false] {
        let name = if found { "a.h" } else { "missing.h" };
        for (operand, prefix) in [
            (format!("<{name}> "), String::new()),
            (format!("\"{name}\" "), String::new()),
            ("H".to_owned(), format!("#define H <{name}>\n")),
        ] {
            for nested in [false, true] {
                let terminal = format!("{prefix}#include {operand}");
                let source = if nested {
                    headers.write("middle.h", &terminal);
                    "#include \"middle.h\"\nparent_after\n".to_owned()
                } else {
                    terminal
                };
                with_directive_tokens_with_system_directory(
                    &source,
                    &headers.0.join("main.c"),
                    &headers.0,
                    |tokens, context| {
                        let names: Vec<_> = tokens
                            .iter()
                            .map(|token| context.string_cache.at(token.contents))
                            .collect();
                        let mut expected = if found { vec!["from_header"] } else { vec![] };
                        if nested {
                            expected.push("parent_after");
                        }
                        assert_eq!(names, expected, "{operand:?}, nested={nested}");
                        let errors = context.take_pending_errors();
                        assert_eq!(
                            errors
                                .iter()
                                .filter(|error| error
                                    .to_string()
                                    .contains("no newline at end of file"))
                                .count(),
                            1,
                            "{operand:?}, nested={nested}: {errors:#?}"
                        );
                        assert_eq!(
                            errors
                                .iter()
                                .filter(|error| error.to_string().contains("cannot find header"))
                                .count(),
                            usize::from(!found),
                            "{operand:?}, nested={nested}: {errors:#?}"
                        );
                        assert!(
                            errors.iter().all(|error| {
                                !error.to_string().contains("unexpected end of file")
                                    && !error.to_string().contains("extra tokens")
                            }),
                            "{operand:?}, nested={nested}: {errors:#?}"
                        );
                    },
                );
            }
        }
    }
}
