use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    io::Error as IoError,
    mem::{
        replace,
        take,
    },
    ops::{
        ControlFlow,
        RangeBounds,
    },
    path::{
        Path,
        PathBuf,
    },
    rc::Rc,
};

use chrono::Local;

use super::{
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVector,
    SourceVectors,
    StrExt,
    TokenString,
    TranslationError,
    TranslationPhase,
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
        PreprocessorTokenizer,
    },
};
use crate::{
    configuration::{
        CStandard,
        ExtensionPolicy,
    },
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        closest_match,
        count_of,
        quote_spelling,
    },
    float_parsing::{
        FloatRangeError,
        LongDouble,
        ParseFloatError,
        string_to_double,
        string_to_float,
        string_to_long_double,
    },
    util::{
        HashMap,
        HashSet,
        last_entry::last_entry,
        read_to_string_lossy,
        shared::{
            SharedString,
            SharedVec,
        },
        string_cache::StringCacheId,
    },
};

const PREDEFINED_MACRO_NAMES: [&str; 5] =
    ["__LINE__", "__FILE__", "__DATE__", "__TIME__", "_Pragma"];

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TokenizerFrameType {
    SourceFile,
    ObjectLikeMacroInvocation {
        name: StringCacheId,
    },
    FunctionLikeMacroInvocation {
        name:        StringCacheId,
        arguments:   Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        is_variadic: bool,
    },
    FunctionLikeMacroArgument {
        argument:            Box<FunctionLikeMacroArgument>,
        paren_depth:         usize,
        has_generated_token: bool,
    },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        configuration::CompilerConfiguration,
        translation_phases::TranslationError,
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
                .filter_map(|(kind, spelling)| (*kind == TokenType::Identifier)
                    .then_some(spelling.as_str()))
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
                            error_type:
                                PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
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
                            error_type:
                                PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
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
            "#define A 1\n#define A  1\n#define F(a) a\n#define F(a) a\n#define E\n#define \
             E\nafter\n",
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
        let directory =
            std::env::temp_dir().join(format!("bcc-include-search-{}", std::process::id()));
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
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum MacroDefinition {
    ObjectLike {
        tokenizer: PreprocessorTokenizer,
    },
    FunctionLike {
        argument_names: Rc<[StringCacheId]>,
        tokenizer:      PreprocessorTokenizer,
        is_variadic:    bool,
    },
    BuiltIn,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TokenizerFrame {
    frame_type: TokenizerFrameType,
    tokenizer:  PreprocessorTokenizer,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum HashHash {
    Lhs(PreprocessorToken),
    Rhs(PreprocessorToken),
    Empty,
}

#[derive(Debug)]
pub(crate) struct Preprocessor {
    pub(crate) tokenizer:       PreprocessorTokenizer,
    pub(crate) tokenizer_stack: Vec<TokenizerFrame>,
    pub(crate) hash_hash_stack: Vec<HashHash>,
    once_set:                   HashSet<u32>,
    macro_definitions:          HashMap<StringCacheId, MacroDefinition>,
    current_is_newline:         bool,
    last_was_newline:           bool,
    /// Provenance of the `if`, `ifdef`, or `ifndef` name of each conditional
    /// directive still waiting for its `#endif`, outermost first. It is owned
    /// rather than an arena range because token iteration compacts the
    /// preprocessor arena while a conditional remains open.
    open_conditionals:          Vec<Box<[SourceVector]>>,

    generate_placeholders:      bool,
    quote_include_directories:  SharedVec<PathBuf>,
    system_include_directories: SharedVec<PathBuf>,
    expression_parser:          PreprocessorExpressionParser,
    pending_parser_token:       Option<Token>,
    pending_parser_errors:      Vec<TranslationError>,
    /// Fixed on first use so every `__DATE__` and `__TIME__` in one
    /// translation unit agrees (C99 §6.10.8p1).
    translation_timestamp:      Option<TranslationTimestamp>,
}

/// Compares one position of two macro definitions under C99 §6.10.3p2: the
/// tokens must be spelled identically, while any two whitespace separations
/// are equivalent and a line end matches the end of input.
fn same_replacement_token(
    context: &Context,
    old: Option<&PreprocessorToken>,
    new: Option<&PreprocessorToken>,
) -> bool {
    let ends = |token: Option<&PreprocessorToken>| {
        token.is_none_or(|token| token.kind == PreprocessorTokenType::Newline)
    };
    match (old, new) {
        | _ if ends(old) && ends(new) => true,
        | (Some(old), Some(new)) =>
            old.kind == new.kind
                && (old.kind == PreprocessorTokenType::Whitespace
                    || context.string_cache.at(old.contents)
                        == context.string_cache.at(new.contents)),
        | _ => false,
    }
}

/// How far [`Preprocessor::skip_over_dead_code`] skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipMode {
    /// A group whose condition was false: stop at the matching `#elif` whose
    /// condition holds, at `#else`, or at `#endif`.
    FalseGroup,
    /// The groups after a translated one: skip through the matching `#endif`.
    ToEndif,
}

/// The date and time of translation, spelled as C99 §6.10.8p1 requires.
#[derive(Debug)]
struct TranslationTimestamp {
    date: String,
    time: String,
}

impl TranslationTimestamp {
    /// Honors `SOURCE_DATE_EPOCH` (reproducible-builds.org, also used by GCC
    /// and Clang) so builds can pin the expansion; otherwise uses local time.
    fn now() -> Self {
        let pinned = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|seconds| seconds.trim().parse::<i64>().ok())
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map(|time| time.naive_utc());
        let time = pinned.unwrap_or_else(|| Local::now().naive_local());
        Self {
            date: time.format("%b %e %Y").to_string(),
            time: time.format("%H:%M:%S").to_string(),
        }
    }
}

/// Spells `value` as a narrow C string literal whose evaluated contents are
/// exactly `value`.
fn string_literal_spelling(value: &str) -> String {
    let mut spelling = String::with_capacity(value.len() + 2);
    spelling.push('"');
    for c in value.chars() {
        match c {
            | '\\' | '"' => {
                spelling.push('\\');
                spelling.push(c);
            },
            | '\n' => spelling.push_str("\\n"),
            | _ => spelling.push(c),
        }
    }
    spelling.push('"');
    spelling
}

impl GetPosition for Preprocessor {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.tokenizer.position(context)
    }
}

impl SetPosition for Preprocessor {
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.tokenizer.set_position(context, position);
    }
}

impl GetSourceFileIndex for Preprocessor {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.tokenizer.source_file_index()
    }
}

impl SetSourceFileIndex for Preprocessor {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.tokenizer
            .set_source_file_index(context, source_file_index);
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionParserState {
    Unary,
    Binary,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperator {
    // Unary
    UnaryPlus,
    UnaryMinus,
    BitwiseNot,
    LogicalNot,

    // Binary
    BinaryPlus,
    BinaryMinus,
    Multiply,
    Divide,
    Modulo,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    Equals,
    NotEquals,
    LeftShift,
    RightShift,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    QuestionMark,
    Conditional,
    Comma,

    // Grouping
    OpeningParenthesis,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PreprocessorExpressionAssociativity {
    Left,
    Right,
}

impl PreprocessorExpressionOperator {
    fn precedence(self) -> u32 {
        match self {
            // Based on https://en.cppreference.com/w/c/language/operator_precedence.
            | Self::UnaryPlus | Self::UnaryMinus | Self::BitwiseNot | Self::LogicalNot => 2,
            | Self::Multiply | Self::Divide | Self::Modulo => 3,
            | Self::BinaryPlus | Self::BinaryMinus => 4,
            | Self::LeftShift | Self::RightShift => 5,
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals => 6,
            | Self::Equals | Self::NotEquals => 7,
            | Self::BitwiseAnd => 8,
            | Self::BitwiseXor => 9,
            | Self::BitwiseOr => 10,
            | Self::LogicalAnd => 11,
            | Self::LogicalOr => 12,
            | Self::QuestionMark | Self::Conditional => 13,
            | Self::Comma => 14,
            // OpeningParenthesis is not a normal operator. We only pop it off the stack when we
            // encounter a closing parenthesis.
            | Self::OpeningParenthesis => u32::MAX,
        }
    }

    fn associativity(self) -> PreprocessorExpressionAssociativity {
        match self {
            | Self::QuestionMark
            | Self::Conditional
            | Self::UnaryPlus
            | Self::UnaryMinus
            | Self::BitwiseNot
            | Self::LogicalNot => PreprocessorExpressionAssociativity::Right,
            | Self::BinaryMinus
            | Self::BinaryPlus
            | Self::Multiply
            | Self::Divide
            | Self::Modulo
            | Self::LeftShift
            | Self::RightShift
            | Self::LessThan
            | Self::LessThanEquals
            | Self::GreaterThan
            | Self::GreaterThanEquals
            | Self::Equals
            | Self::NotEquals
            | Self::BitwiseAnd
            | Self::BitwiseOr
            | Self::BitwiseXor
            | Self::LogicalAnd
            | Self::LogicalOr
            | Self::Comma
            | Self::OpeningParenthesis => PreprocessorExpressionAssociativity::Left,
        }
    }

    fn has_precedence_over(self, other: Self) -> bool {
        match self.associativity() {
            | PreprocessorExpressionAssociativity::Left => self.precedence() <= other.precedence(),
            | PreprocessorExpressionAssociativity::Right => self.precedence() < other.precedence(),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct PreprocessorExpressionParser {
    operator_stack: Vec<PreprocessorExpressionOperator>,
    operand_stack:  PreprocessorExpressionOperandStack,
    state:          PreprocessorExpressionParserState,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PreprocessorExpressionOperand {
    Signed(i64),
    Unsigned(u64),
}

impl PreprocessorExpressionOperand {
    fn as_signed(self) -> i64 {
        match self {
            | Self::Signed(v) => v,
            | Self::Unsigned(v) => v as i64,
        }
    }

    fn as_unsigned(self) -> u64 {
        match self {
            | Self::Signed(v) => v as u64,
            | Self::Unsigned(v) => v,
        }
    }

    fn is_signed(self) -> bool {
        match self {
            | Self::Signed(_) => true,
            | Self::Unsigned(_) => false,
        }
    }

    fn is_unsigned(self) -> bool {
        match self {
            | Self::Signed(_) => false,
            | Self::Unsigned(_) => true,
        }
    }

    fn set_signed(self, value: i64) -> Self {
        match self {
            | Self::Signed(_) => Self::Signed(value),
            | Self::Unsigned(_) => Self::Unsigned(value as u64),
        }
    }

    #[expect(dead_code, reason = "This is currently unused")]
    fn set_unsigned(self, value: u64) -> Self {
        self.set_signed(value as i64)
    }

    fn map_signed(self, f: impl FnOnce(i64) -> i64) -> Self {
        match self {
            | Self::Signed(v) => Self::Signed(f(v)),
            | Self::Unsigned(v) => Self::Unsigned(f(v as i64) as u64),
        }
    }

    fn map_unsigned(self, f: impl FnOnce(u64) -> u64) -> Self {
        self.map_signed(|v| f(v as u64) as i64)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct EvaluatedPreprocessorExpressionOperand {
    value:                    PreprocessorExpressionOperand,
    contains_evaluated_comma: bool,
}

impl From<PreprocessorExpressionOperand> for EvaluatedPreprocessorExpressionOperand {
    fn from(value: PreprocessorExpressionOperand) -> Self {
        Self {
            value,
            contains_evaluated_comma: false,
        }
    }
}

impl EvaluatedPreprocessorExpressionOperand {
    fn with_comma_liveness(
        value: PreprocessorExpressionOperand,
        contains_evaluated_comma: bool,
    ) -> Self {
        Self {
            value,
            contains_evaluated_comma,
        }
    }

    fn as_signed(self) -> i64 {
        self.value.as_signed()
    }

    fn as_unsigned(self) -> u64 {
        self.value.as_unsigned()
    }

    fn is_signed(self) -> bool {
        self.value.is_signed()
    }

    fn is_unsigned(self) -> bool {
        self.value.is_unsigned()
    }

    fn set_signed(self, value: i64) -> Self {
        Self::with_comma_liveness(self.value.set_signed(value), self.contains_evaluated_comma)
    }

    fn map_unsigned(self, f: impl FnOnce(u64) -> u64) -> Self {
        Self::with_comma_liveness(self.value.map_unsigned(f), self.contains_evaluated_comma)
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
struct PreprocessorExpressionOperandStack {
    values:                  Vec<EvaluatedPreprocessorExpressionOperand>,
    pending_evaluated_comma: bool,
}

impl PreprocessorExpressionOperandStack {
    fn clear(&mut self) {
        self.values.clear();
        self.pending_evaluated_comma = false;
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn push(&mut self, operand: impl Into<EvaluatedPreprocessorExpressionOperand>) {
        let mut operand = operand.into();
        operand.contains_evaluated_comma |= self.pending_evaluated_comma;
        self.pending_evaluated_comma = false;
        self.values.push(operand);
    }

    fn pop(&mut self) -> Option<EvaluatedPreprocessorExpressionOperand> {
        let operand = self.values.pop()?;
        self.pending_evaluated_comma |= operand.contains_evaluated_comma;
        Some(operand)
    }

    fn pop_isolated(&mut self) -> Option<EvaluatedPreprocessorExpressionOperand> {
        self.values.pop()
    }
}

impl PreprocessorExpressionParser {
    fn new() -> Self {
        Self {
            operator_stack: Vec::new(),
            operand_stack:  PreprocessorExpressionOperandStack::default(),
            state:          PreprocessorExpressionParserState::Unary,
        }
    }

    fn reset(&mut self) {
        self.operator_stack.clear();
        self.operand_stack.clear();
        self.state = PreprocessorExpressionParserState::Unary;
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum SignedIntegerLiteralType {
    Int,
    Long,
    LongLong,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[expect(
    clippy::enum_variant_names,
    reason = "We are repeating the word 'unsigned' a lot here, but I think it's clearer this way."
)]
pub(crate) enum UnsignedIntegerLiteralType {
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum IntegerSuffix {
    Unsigned,
    Long,
    LongLong,
    UnsignedLong,
    UnsignedLongLong,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind:           TokenType,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) contents:       StringCacheId,
}

impl GetPosition for Token {
    #[inline(always)]
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for Token {
    #[inline(always)]
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum IntegerTokenType {
    Int(i32),
    Long(i64),
    LongLong(i64),
    UnsignedInt(u32),
    UnsignedLong(u64),
    UnsignedLongLong(u64),
}

impl From<IntegerTokenType> for i128 {
    fn from(v: IntegerTokenType) -> Self {
        match v {
            | IntegerTokenType::UnsignedLong(v) | IntegerTokenType::UnsignedLongLong(v) =>
                i128::from(v),
            | IntegerTokenType::Long(v) | IntegerTokenType::LongLong(v) => i128::from(v),
            | IntegerTokenType::UnsignedInt(v) => i128::from(v),
            | IntegerTokenType::Int(v) => i128::from(v),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum FloatTokenType {
    Float(f32),
    Double(f64),
    LongDouble(LongDouble),
}

impl FloatTokenType {
    /// The C type named by this constant's suffix.
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            | Self::Float(_) => "float",
            | Self::Double(_) => "double",
            | Self::LongDouble(_) => "long double",
        }
    }
}

impl Display for FloatTokenType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Float(v) => write!(f, "{v}"),
            | Self::Double(v) => write!(f, "{v}"),
            | Self::LongDouble(v) => write!(f, "{v}"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum KeywordTokenType {
    Auto,
    Break,
    Case,
    Char,
    Const,
    Continue,
    Default,
    Do,
    Double,
    Else,
    Enum,
    Extern,
    Float,
    For,
    Goto,
    If,
    Inline,
    Int,
    Long,
    Register,
    Restrict,
    Return,
    Short,
    Signed,
    Sizeof,
    Static,
    Struct,
    Switch,
    Typedef,
    Union,
    Unsigned,
    Void,
    Volatile,
    While,
    Bool,
    Complex,
    Imaginary,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum OperatorTokenType {
    Plus,
    Minus,
    Asterisk,
    ForwardSlash,
    Percent,
    LessThanLessThan,
    GreaterThanGreaterThan,
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    EqualsEquals,
    ExclamationMarkEquals,
    Ampersand,
    Caret,
    Pipe,
    AmpersandAmpersand,
    PipePipe,
    QuestionMark,
    Colon,
    Semicolon,
    OpeningParenthesis,
    ClosingParenthesis,
    OpeningSquareBracket,
    ClosingSquareBracket,
    OpeningCurlyBrace,
    ClosingCurlyBrace,
    Period,
    Arrow,
    PlusPlus,
    MinusMinus,
    Comma,
    Tilde,
    ExclamationMark,
    Equals,
    PlusEquals,
    MinusEquals,
    AsteriskEquals,
    ForwardSlashEquals,
    PercentEquals,
    LessThanLessThanEquals,
    GreaterThanGreaterThanEquals,
    AmpersandEquals,
    CaretEquals,
    PipeEquals,
    Ellipsis,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum StringTokenType {
    String(StringCacheId),
    WideString(StringCacheId),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum CharacterTokenType {
    Char(char),
    WideChar(char),
    /// Packed integer value of an ordinary multi-character constant.
    MultiChar(i32),
}

impl From<CharacterTokenType> for i64 {
    fn from(v: CharacterTokenType) -> Self {
        match v {
            | CharacterTokenType::Char(c) | CharacterTokenType::WideChar(c) =>
                i64::from(u32::from(c)),
            | CharacterTokenType::MultiChar(value) => i64::from(value),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum TokenType {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Identifier,
    Keyword(KeywordTokenType),
    Operator(OperatorTokenType),
    String(StringTokenType),
    Character(CharacterTokenType),
}

impl KeywordTokenType {
    /// The keyword as written in C source.
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            | Self::Auto => "auto",
            | Self::Break => "break",
            | Self::Case => "case",
            | Self::Char => "char",
            | Self::Const => "const",
            | Self::Continue => "continue",
            | Self::Default => "default",
            | Self::Do => "do",
            | Self::Double => "double",
            | Self::Else => "else",
            | Self::Enum => "enum",
            | Self::Extern => "extern",
            | Self::Float => "float",
            | Self::For => "for",
            | Self::Goto => "goto",
            | Self::If => "if",
            | Self::Inline => "inline",
            | Self::Int => "int",
            | Self::Long => "long",
            | Self::Register => "register",
            | Self::Restrict => "restrict",
            | Self::Return => "return",
            | Self::Short => "short",
            | Self::Signed => "signed",
            | Self::Sizeof => "sizeof",
            | Self::Static => "static",
            | Self::Struct => "struct",
            | Self::Switch => "switch",
            | Self::Typedef => "typedef",
            | Self::Union => "union",
            | Self::Unsigned => "unsigned",
            | Self::Void => "void",
            | Self::Volatile => "volatile",
            | Self::While => "while",
            | Self::Bool => "_Bool",
            | Self::Complex => "_Complex",
            | Self::Imaginary => "_Imaginary",
        }
    }
}

impl OperatorTokenType {
    /// The punctuator as written in C source (never a digraph).
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            | Self::Plus => "+",
            | Self::Minus => "-",
            | Self::Asterisk => "*",
            | Self::ForwardSlash => "/",
            | Self::Percent => "%",
            | Self::LessThanLessThan => "<<",
            | Self::GreaterThanGreaterThan => ">>",
            | Self::LessThan => "<",
            | Self::LessThanEquals => "<=",
            | Self::GreaterThan => ">",
            | Self::GreaterThanEquals => ">=",
            | Self::EqualsEquals => "==",
            | Self::ExclamationMarkEquals => "!=",
            | Self::Ampersand => "&",
            | Self::Caret => "^",
            | Self::Pipe => "|",
            | Self::AmpersandAmpersand => "&&",
            | Self::PipePipe => "||",
            | Self::QuestionMark => "?",
            | Self::Colon => ":",
            | Self::Semicolon => ";",
            | Self::OpeningParenthesis => "(",
            | Self::ClosingParenthesis => ")",
            | Self::OpeningSquareBracket => "[",
            | Self::ClosingSquareBracket => "]",
            | Self::OpeningCurlyBrace => "{",
            | Self::ClosingCurlyBrace => "}",
            | Self::Period => ".",
            | Self::Arrow => "->",
            | Self::PlusPlus => "++",
            | Self::MinusMinus => "--",
            | Self::Comma => ",",
            | Self::Tilde => "~",
            | Self::ExclamationMark => "!",
            | Self::Equals => "=",
            | Self::PlusEquals => "+=",
            | Self::MinusEquals => "-=",
            | Self::AsteriskEquals => "*=",
            | Self::ForwardSlashEquals => "/=",
            | Self::PercentEquals => "%=",
            | Self::LessThanLessThanEquals => "<<=",
            | Self::GreaterThanGreaterThanEquals => ">>=",
            | Self::AmpersandEquals => "&=",
            | Self::CaretEquals => "^=",
            | Self::PipeEquals => "|=",
            | Self::Ellipsis => "...",
        }
    }
}

impl TokenType {
    /// Describes a found token for a message, such as "keyword `int`",
    /// "`;`", or "identifier `count`". `spelling` is the token's source text
    /// when known.
    pub(crate) fn found(self, spelling: Option<&str>) -> String {
        let with_spelling = |kind: &str| match spelling.filter(|spelling| !spelling.is_empty()) {
            | Some(spelling) => format!("{kind} {}", quote_spelling(spelling)),
            | None => kind.to_owned(),
        };
        match self {
            | Self::Keyword(keyword) => format!("keyword `{}`", keyword.spelling()),
            | Self::Operator(operator) => format!("`{}`", operator.spelling()),
            | Self::Identifier => with_spelling("identifier"),
            | Self::Integer(_) => with_spelling("integer constant"),
            | Self::Float(_) => with_spelling("floating constant"),
            | Self::Character(_) => with_spelling("character constant"),
            | Self::String(_) => with_spelling("string literal"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreprocessorError {
    pub(crate) error_type:     PreprocessorErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for PreprocessorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for PreprocessorError {
    fn to_diagnostic(&self, context: &Context, source: SourceVectors) -> Diagnostic {
        self.error_type
            .explain(context.source_spelling(source))
            .at(self.severity(), source)
    }
}

impl std::error::Error for PreprocessorError {}

impl GetPosition for PreprocessorError {
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for PreprocessorError {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

impl GetSeverity for PreprocessorError {
    fn severity(&self) -> ErrorSeverity {
        match self.error_type {
            | PreprocessorErrorType::UnexpectedEndOfInput(_)
            | PreprocessorErrorType::InvalidHexadecimalFloatLiteral
            | PreprocessorErrorType::InvalidDecimalFloatLiteral
            | PreprocessorErrorType::InvalidHexadecimalIntegerLiteral
            | PreprocessorErrorType::InvalidBinaryIntegerLiteral
            | PreprocessorErrorType::InvalidOctalIntegerLiteral
            | PreprocessorErrorType::InvalidDecimalIntegerLiteral
            | PreprocessorErrorType::IntegerLiteralOverflow
            | PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression
            | PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                ..
            }
            | PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(
                ..,
            )
            | PreprocessorErrorType::MissingIdentifierInDefinedDirective(..)
            | PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(..)
            | PreprocessorErrorType::NoConditionInIfDirective
            | PreprocessorErrorType::NoConditionInElifDirective
            | PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives
            | PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives
            | PreprocessorErrorType::ElifDirectiveWithoutIfDirective
            | PreprocessorErrorType::ElseDirectiveWithoutIfDirective
            | PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(..)
            | PreprocessorErrorType::ExpectedIdentifierInDefineDirective(..)
            | PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(..)
            | PreprocessorErrorType::HeaderNotFound { .. }
            | PreprocessorErrorType::HeaderFileInaccessible(..)
            | PreprocessorErrorType::HashHashUsedOutsideOfMacro
            | PreprocessorErrorType::CannotUseHashHashAfterFunctionLikeMacroCall
            | PreprocessorErrorType::InvalidEscapeSequence
            | PreprocessorErrorType::UnterminatedEscapeSequence
            | PreprocessorErrorType::InvalidHexEscapeSequence
            | PreprocessorErrorType::HexEscapeSequenceTooLarge
            | PreprocessorErrorType::InvalidOctalEscapeSequence
            | PreprocessorErrorType::OctalEscapeSequenceTooLarge
            | PreprocessorErrorType::InvalidSmallUnicodeEscapeSequence
            | PreprocessorErrorType::SmallUnicodeEscapeSequenceTooShort
            | PreprocessorErrorType::InvalidLargeUnicodeEscapeSequence
            | PreprocessorErrorType::LargeUnicodeEscapeSequenceTooSmall
            | PreprocessorErrorType::MultiCharacterLiteralsUnsupported
            | PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(..)
            | PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(..)
            | PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(..)
            | PreprocessorErrorType::VariadicMacroMustBeLastParameter(..)
            | PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(..)
            | PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(..)
            | PreprocessorErrorType::ExpectedIdentifierInUndefDirective(..)
            | PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(..)
            | PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(..)
            | PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(..)
            | PreprocessorErrorType::MissingRightHandSideOfHashHashOperator
            | PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator
            | PreprocessorErrorType::TokenMergingError(..)
            | PreprocessorErrorType::MissingNumberInLineDirective(..)
            | PreprocessorErrorType::MissingNewlineAfterLineDirective(..)
            | PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(..)
            | PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(..)
            | PreprocessorErrorType::MissingStringLiteralInPragmaOperator(..)
            | PreprocessorErrorType::UnknownPragmaSTDCArgument(..)
            | PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument
            | PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch
            | PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(..)
            | PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression
            | PreprocessorErrorType::BinaryPlusOverflow
            | PreprocessorErrorType::BinaryMinusOverflow
            | PreprocessorErrorType::DivideOverflow
            | PreprocessorErrorType::DivideByZero
            | PreprocessorErrorType::ModuloOverflow
            | PreprocessorErrorType::ModuloByZero
            | PreprocessorErrorType::MultiplyOverflow
            | PreprocessorErrorType::UnaryMinusOverflow
            | PreprocessorErrorType::LeftShiftOverflow
            | PreprocessorErrorType::RightShiftOverflow
            | PreprocessorErrorType::BitwiseNotWithoutOperand
            | PreprocessorErrorType::LogicalNotWithoutOperand
            | PreprocessorErrorType::UnaryMinusWithoutOperand
            | PreprocessorErrorType::UnaryPlusWithoutOperand
            | PreprocessorErrorType::BinaryPlusWithoutRhs
            | PreprocessorErrorType::BinaryMinusWithoutRhs
            | PreprocessorErrorType::MultiplyWithoutRhs
            | PreprocessorErrorType::DivideWithoutRhs
            | PreprocessorErrorType::ModuloWithoutRhs
            | PreprocessorErrorType::BitwiseOrWithoutRhs
            | PreprocessorErrorType::BitwiseAndWithoutRhs
            | PreprocessorErrorType::BitwiseXorWithoutRhs
            | PreprocessorErrorType::LogicalAndWithoutRhs
            | PreprocessorErrorType::LogicalOrWithoutRhs
            | PreprocessorErrorType::LessThanWithoutRhs
            | PreprocessorErrorType::LessThanEqualsWithoutRhs
            | PreprocessorErrorType::GreaterThanWithoutRhs
            | PreprocessorErrorType::GreaterThanEqualsWithoutRhs
            | PreprocessorErrorType::EqualsWithoutRhs
            | PreprocessorErrorType::NotEqualsWithoutRhs
            | PreprocessorErrorType::LeftShiftWithoutRhs
            | PreprocessorErrorType::RightShiftWithoutRhs
            | PreprocessorErrorType::TernaryOperatorWithoutRhs
            | PreprocessorErrorType::TernaryOperatorWithoutMhs
            | PreprocessorErrorType::ColonWithoutMatchingQuestionMark
            | PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression
            | PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(_)
            | PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression
            | PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression
            | PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(_)
            | PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(..)
            | PreprocessorErrorType::UnexpectedTokenAtPhase7(..)
            | PreprocessorErrorType::ErrorDirective(..)
             => ErrorSeverity::Error,
            | PreprocessorErrorType::CommaOperatorInPreprocessorExpression(policy) =>
                match policy {
                    | ExtensionPolicy::Allow => unreachable!(
                        "allowed extensions must not produce a comma diagnostic"
                    ),
                    | ExtensionPolicy::Warn => ErrorSeverity::Warning,
                    | ExtensionPolicy::Deny => ErrorSeverity::Error,
                },
            | PreprocessorErrorType::RedefinitionOfBuiltInMacro(..)
            | PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(..)
            | PreprocessorErrorType::FloatConstantOutOfRange { .. }
            | PreprocessorErrorType::ForcedSignedToUnsignedConversion { .. }
            | PreprocessorErrorType::ForcedUnsignedPromotion { .. }
            | PreprocessorErrorType::ForcedSignedPromotion { .. }
            | PreprocessorErrorType::HashMustBeFirstCharacterOnLine
            | PreprocessorErrorType::UnknownDirective
            | PreprocessorErrorType::HashMustBeFollowedByIdentifier
            | PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence
            | PreprocessorErrorType::LineDirectiveNumberTooLarge(..)
            | PreprocessorErrorType::UnknownPragmaDirective
            | PreprocessorErrorType::ExtraTokensAfterPragmaOnce(..)
            | PreprocessorErrorType::ExtraTokensAfterPragmaOperator
            | PreprocessorErrorType::ExtraTokensAfterIncludeDirective
            | PreprocessorErrorType::ExtraTokensAfterIfdefDirective
            | PreprocessorErrorType::ExtraTokensAfterIfndefDirective
            | PreprocessorErrorType::PragmaOnceInNonHeader => ErrorSeverity::Warning,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PreprocessorErrorType {
    InvalidHexadecimalFloatLiteral,
    InvalidDecimalFloatLiteral,
    FloatConstantOutOfRange {
        type_name: &'static str,
        error:     FloatRangeError,
    },
    InvalidHexadecimalIntegerLiteral,
    InvalidBinaryIntegerLiteral,
    InvalidOctalIntegerLiteral,
    InvalidDecimalIntegerLiteral,
    IntegerLiteralOverflow,
    ForcedSignedToUnsignedConversion {
        from: SignedIntegerLiteralType,
        to:   UnsignedIntegerLiteralType,
    },
    ForcedUnsignedPromotion {
        from: UnsignedIntegerLiteralType,
        to:   UnsignedIntegerLiteralType,
    },
    ForcedSignedPromotion {
        from: SignedIntegerLiteralType,
        to:   SignedIntegerLiteralType,
    },
    HashMustBeFirstCharacterOnLine,
    HashMustBeFollowedByIdentifier,
    UnknownDirective,
    EmptyParenthesesInPreprocessorExpression,
    UnaryPlusWithoutOperand,
    UnaryMinusWithoutOperand,
    BitwiseNotWithoutOperand,
    LogicalNotWithoutOperand,
    BinaryPlusWithoutRhs,
    BinaryMinusWithoutRhs,
    MultiplyWithoutRhs,
    DivideWithoutRhs,
    ModuloWithoutRhs,
    LessThanWithoutRhs,
    LessThanEqualsWithoutRhs,
    GreaterThanWithoutRhs,
    GreaterThanEqualsWithoutRhs,
    EqualsWithoutRhs,
    NotEqualsWithoutRhs,
    LeftShiftWithoutRhs,
    RightShiftWithoutRhs,
    BitwiseAndWithoutRhs,
    BitwiseXorWithoutRhs,
    BitwiseOrWithoutRhs,
    LogicalAndWithoutRhs,
    LogicalOrWithoutRhs,
    TernaryOperatorWithoutMhs,
    TernaryOperatorWithoutRhs,
    ColonWithoutMatchingQuestionMark,
    CommaOperatorInPreprocessorExpression(ExtensionPolicy),
    BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(PreprocessorExpressionOperator),
    DivideByZero,
    ModuloByZero,
    UnaryMinusOverflow,
    BinaryPlusOverflow,
    BinaryMinusOverflow,
    MultiplyOverflow,
    DivideOverflow,
    ModuloOverflow,
    LeftShiftOverflow,
    RightShiftOverflow,
    UnterminatedOpeningParenthesisInPreprocessorExpression,
    TildeInsteadOfBinaryOperatorInPreprocessorExpression,
    ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
    FunctionCallOperatorNotSupportedInPreprocessorExpression,
    DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
    AddressOfOperatorNotSupportedInPreprocessorExpression,
    DereferenceOperatorNotSupportedInPreprocessorExpression,
    ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(PreprocessorExpressionOperator),
    NumberInsteadOfBinaryOperatorInPreprocessorExpression,
    IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
    CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
    UnexpectedTokenInPreprocessorExpression(PreprocessorTokenType),
    UnexpectedTokenAtPhase7(PreprocessorTokenType),
    FloatInsteadOfIntegerInPreprocessorExpression,
    ExpectedBinaryOperatorInPreprocessorExpression,
    MissingOpeningParenthesisOrIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingIdentifierInDefinedDirective(PreprocessorTokenType),
    MissingClosingParenthesisInDefinedDirective(PreprocessorTokenType),
    NoConditionInIfDirective,
    NoConditionInElifDirective,
    MoreIfDirectivesThanEndifDirectives,
    MoreEndifDirectivesThanIfDirectives,
    ElifDirectiveWithoutIfDirective,
    ElseDirectiveWithoutIfDirective,
    ExpectedIdentifierInIfdefDirective(PreprocessorTokenType),
    ExpectedIdentifierInIfndefDirective(PreprocessorTokenType),
    ExpectedIdentifierInDefineDirective(PreprocessorTokenType),
    RedefinitionOfBuiltInMacro(String),
    UndefinedIdentifierInPreprocessorExpression(String),
    ExpectedIncludeStringOrAngleBracketString(PreprocessorTokenType),
    UnexpectedEndOfInput(&'static str),
    WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
        expected: usize,
        found:    usize,
    },
    HeaderNotFound {
        name:             String,
        is_system_header: bool,
        searched:         Vec<PathBuf>,
    },
    HeaderFileInaccessible(IoError),
    HashHashUsedOutsideOfMacro,
    CannotUseHashHashAfterFunctionLikeMacroCall,
    InvalidEscapeSequence,
    UnterminatedEscapeSequence,
    InvalidHexEscapeSequence,
    HexEscapeSequenceTooLarge,
    InvalidOctalEscapeSequence,
    OctalEscapeSequenceTooLarge,
    InvalidSmallUnicodeEscapeSequence,
    SmallUnicodeEscapeSequenceTooShort,
    InvalidLargeUnicodeEscapeSequence,
    LargeUnicodeEscapeSequenceTooSmall,
    MultiCharacterLiteralsUnsupported,
    RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(String),
    RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(String),
    ExpectedIdentifierInMacroDefinition(PreprocessorTokenType),
    VariadicMacroMustBeLastParameter(String),
    ExpectedCommaOrClosingParenthesisInMacroDefinition(PreprocessorTokenType),
    MacroRedefinedWithDifferentDefinition(String),
    ExpectedIdentifierInUndefDirective(PreprocessorTokenType),
    ExpectedNewlineAfterUndefDirective(PreprocessorTokenType),
    HashOperatorMustBeFollowedByAMacroArgument(PreprocessorTokenType),
    IdentifierNotMacroArgumentAfterHashOperator(String),
    MissingRightHandSideOfHashHashOperator,
    MissingLeftHandSideOfHashHashOperator,
    TokenMergingError(String, String),
    MissingNumberInLineDirective(PreprocessorTokenType),
    MissingNewlineAfterLineDirective(PreprocessorTokenType),
    LineDirectiveIsNotASimpleDigitSequence,
    LineDirectiveNumberTooLarge(i128),
    MissingOpeningParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingClosingParenthesisInPragmaOperator(PreprocessorTokenType),
    MissingStringLiteralInPragmaOperator(PreprocessorTokenType),
    UnknownPragmaDirective,
    UnknownPragmaSTDCArgument(String),
    ExtraTokensAfterPragmaOnce(PreprocessorTokenType),
    ExtraTokensAfterPragmaOperator,
    ExtraTokensAfterIncludeDirective,
    ExtraTokensAfterIfdefDirective,
    ExtraTokensAfterIfndefDirective,
    STDCPragmaDirectiveWithoutArgument,
    STDCPragmaDirectiveWithoutOnOffSwitch,
    MissingOnOffSwitchInSTDCPragma(String),
    PragmaOnceInNonHeader,
    ErrorDirective(String),
}

impl SignedIntegerLiteralType {
    fn spelling(self) -> &'static str {
        match self {
            | Self::Int => "int",
            | Self::Long => "long",
            | Self::LongLong => "long long",
        }
    }
}

impl UnsignedIntegerLiteralType {
    fn spelling(self) -> &'static str {
        match self {
            | Self::UnsignedInt => "unsigned int",
            | Self::UnsignedLong => "unsigned long",
            | Self::UnsignedLongLong => "unsigned long long",
        }
    }
}

impl PreprocessorExpressionOperator {
    /// The operator as written in C source.
    fn spelling(self) -> &'static str {
        match self {
            | Self::UnaryPlus | Self::BinaryPlus => "+",
            | Self::UnaryMinus | Self::BinaryMinus => "-",
            | Self::BitwiseNot => "~",
            | Self::LogicalNot => "!",
            | Self::Multiply => "*",
            | Self::Divide => "/",
            | Self::Modulo => "%",
            | Self::LessThan => "<",
            | Self::LessThanEquals => "<=",
            | Self::GreaterThan => ">",
            | Self::GreaterThanEquals => ">=",
            | Self::Equals => "==",
            | Self::NotEquals => "!=",
            | Self::LeftShift => "<<",
            | Self::RightShift => ">>",
            | Self::BitwiseAnd => "&",
            | Self::BitwiseXor => "^",
            | Self::BitwiseOr => "|",
            | Self::LogicalAnd => "&&",
            | Self::LogicalOr => "||",
            | Self::QuestionMark => "?",
            | Self::Conditional => ":",
            | Self::Comma => ",",
            | Self::OpeningParenthesis => "(",
        }
    }
}

/// The directives of C99 §6.10, for suggestions.
const DIRECTIVE_NAMES: [&str; 12] = [
    "if", "ifdef", "ifndef", "elif", "else", "endif", "include", "define", "undef", "line",
    "error", "pragma",
];

const DIRECTIVE_LIST_NOTE: &str = "C99 §6.10: the directives are `#if`, `#ifdef`, `#ifndef`, \
                                   `#elif`, `#else`, `#endif`, `#include`, `#define`, `#undef`, \
                                   `#line`, `#error`, and `#pragma`";

const IF_EXPRESSION_NOTE: &str =
    "C99 §6.10.1p1: the condition must be an integer constant expression";

const ESCAPE_LIST_NOTE: &str = "C99 §6.4.4.4: the escapes are `\\'`, `\\\"`, `\\?`, `\\\\`, \
                                `\\a`, `\\b`, `\\f`, `\\n`, `\\r`, `\\t`, `\\v`, octal `\\ooo`, \
                                hexadecimal `\\xhh`, and universal `\\uXXXX` or `\\UXXXXXXXX`";

impl PreprocessorErrorType {
    /// Describes the error; `spelling` is the source text it points at, used
    /// to name what was actually written.
    #[expect(
        clippy::too_many_lines,
        reason = "One exhaustive table keeps every preprocessor message reviewable in one place."
    )]
    pub(crate) fn explain(&self, spelling: Option<&str>) -> Explanation {
        let quoted = || spelling.map(quote_spelling);
        let titled = |title: &str| match quoted() {
            | Some(quoted) => format!("{title} {quoted}"),
            | None => title.to_owned(),
        };
        let missing_operand = |operator: &str, side: &str| {
            Explanation::new(format!("expected an expression {side} `{operator}`"))
                .label(format!("`{operator}` needs an operand here"))
        };
        let overflow = |operation: &str| {
            Explanation::new(format!("{operation} overflows in `#if` expression"))
                .label("the result is out of range")
                .note(
                    "C99 §6.10.1p3: `#if` arithmetic uses `intmax_t` and `uintmax_t`, and §6.6p4 \
                     requires constant expressions to stay in range",
                )
        };
        let instead_of_operator = |found: &str| {
            Explanation::new(format!("expected an operator, found {found}"))
                .label("expected a binary operator before this")
                .note("operands in an `#if` expression must be joined by operators")
        };
        match self {
            | Self::InvalidHexadecimalFloatLiteral =>
                Explanation::new(titled("invalid hexadecimal floating constant"))
                    .label("not a valid hexadecimal floating constant")
                    .note(
                        "C99 §6.4.4.2: a hexadecimal floating constant needs hexadecimal digits \
                         and a binary exponent, as in `0x1.8p3`",
                    ),
            | Self::InvalidDecimalFloatLiteral =>
                Explanation::new(titled("invalid floating constant"))
                    .label("not a valid floating constant")
                    .note(
                        "C99 §6.4.4.2: a floating constant is digits with a `.` or an exponent \
                         and an optional `f`, `F`, `l`, or `L` suffix, as in `1.5`, `2e10`, or \
                         `.5f`",
                    ),
            | Self::FloatConstantOutOfRange { type_name, error } => match error {
                | FloatRangeError::Overflow =>
                    Explanation::new(format!("floating constant is too large for `{type_name}`"))
                        .label("this value becomes infinity")
                        .note(
                            "C99 §6.4.4p2: the value of a constant must be representable in its \
                             type",
                        ),
                | FloatRangeError::Underflow =>
                    Explanation::new(format!("floating constant is too small for `{type_name}`"))
                        .label("this nonzero value becomes zero")
                        .note(
                            "C99 §6.4.4p2: the value of a constant must be representable in its \
                             type",
                        ),
            },
            | Self::InvalidHexadecimalIntegerLiteral =>
                Explanation::new(titled("invalid hexadecimal integer constant"))
                    .label("not a valid hexadecimal constant")
                    .note(
                        "C99 §6.4.4.1: hexadecimal digits are `0`-`9`, `a`-`f`, and `A`-`F`, \
                         optionally followed by an integer suffix such as `u`, `l`, or `ull`",
                    ),
            | Self::InvalidBinaryIntegerLiteral =>
                Explanation::new(titled("invalid binary integer constant"))
                    .label("not a valid binary constant")
                    .note("binary digits are `0` and `1`"),
            | Self::InvalidOctalIntegerLiteral =>
                Explanation::new(titled("invalid octal integer constant"))
                    .label("not a valid octal constant")
                    .note(
                        "C99 §6.4.4.1: a constant starting with `0` is octal, and octal digits \
                         are `0`-`7`",
                    ),
            | Self::InvalidDecimalIntegerLiteral =>
                Explanation::new(titled("invalid integer constant"))
                    .label("not a valid integer constant")
                    .note(
                        "C99 §6.4.4.1: an integer constant is digits followed by an optional \
                         suffix such as `u`, `l`, `ll`, or `ull`",
                    ),
            | Self::IntegerLiteralOverflow => Explanation::new("integer constant is too large")
                .label("does not fit in `unsigned long long`")
                .note("the largest integer type, `unsigned long long`, has 64 bits"),
            | Self::ForcedSignedToUnsignedConversion { from, to } => match from {
                | SignedIntegerLiteralType::Int =>
                    Explanation::new("integer constant is so large that it is unsigned")
                        .label(format!("this constant has type `{}`", to.spelling()))
                        .note("C99 §6.4.4.1p5: no signed type can represent this decimal constant")
                        .help("add a `u` suffix to make the unsigned type explicit"),
                | SignedIntegerLiteralType::Long | SignedIntegerLiteralType::LongLong =>
                    Explanation::new(format!(
                        "integer constant is too large for `{}`",
                        from.spelling()
                    ))
                    .label(format!("this constant has type `{}`", to.spelling()))
                    .help("add a `u` suffix to make the unsigned type explicit"),
            },
            | Self::ForcedUnsignedPromotion { from, to } => Explanation::new(format!(
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format!("this constant has type `{}`", to.spelling())),
            | Self::ForcedSignedPromotion { from, to } => Explanation::new(format!(
                "integer constant does not fit in `{}`",
                from.spelling()
            ))
            .label(format!("this constant has type `{}`", to.spelling())),
            | Self::HashMustBeFirstCharacterOnLine => Explanation::new("stray `#` in program")
                .label("a directive must start a line")
                .note(
                    "C99 §6.10p2: `#` begins a directive only as the first token on a line; \
                     elsewhere it is valid only inside a function-like macro definition",
                ),
            | Self::HashMustBeFollowedByIdentifier => {
                let found = spelling.map_or_else(|| "a token".to_owned(), quote_spelling);
                Explanation::new(format!(
                    "expected a directive name after `#`, found {found}"
                ))
                .label("expected a directive name")
                .note(DIRECTIVE_LIST_NOTE)
            },
            | Self::UnknownDirective => {
                let name = spelling.unwrap_or_default();
                let explanation =
                    Explanation::new(format!("unknown preprocessing directive `#{name}`"))
                        .label("not a C99 directive")
                        .note(DIRECTIVE_LIST_NOTE);
                match closest_match(name, &DIRECTIVE_NAMES) {
                    | Some(suggestion) =>
                        explanation.help(format!("did you mean `#{suggestion}`?")),
                    | None => explanation,
                }
            },
            | Self::EmptyParenthesesInPreprocessorExpression =>
                Explanation::new("expected an expression inside `()`")
                    .label("empty parentheses")
                    .note(IF_EXPRESSION_NOTE),
            | Self::UnaryPlusWithoutOperand | Self::BinaryPlusWithoutRhs =>
                missing_operand("+", "after"),
            | Self::UnaryMinusWithoutOperand | Self::BinaryMinusWithoutRhs =>
                missing_operand("-", "after"),
            | Self::BitwiseNotWithoutOperand => missing_operand("~", "after"),
            | Self::LogicalNotWithoutOperand => missing_operand("!", "after"),
            | Self::MultiplyWithoutRhs => missing_operand("*", "after"),
            | Self::DivideWithoutRhs => missing_operand("/", "after"),
            | Self::ModuloWithoutRhs => missing_operand("%", "after"),
            | Self::LessThanWithoutRhs => missing_operand("<", "after"),
            | Self::LessThanEqualsWithoutRhs => missing_operand("<=", "after"),
            | Self::GreaterThanWithoutRhs => missing_operand(">", "after"),
            | Self::GreaterThanEqualsWithoutRhs => missing_operand(">=", "after"),
            | Self::EqualsWithoutRhs => missing_operand("==", "after"),
            | Self::NotEqualsWithoutRhs => missing_operand("!=", "after"),
            | Self::LeftShiftWithoutRhs => missing_operand("<<", "after"),
            | Self::RightShiftWithoutRhs => missing_operand(">>", "after"),
            | Self::BitwiseAndWithoutRhs => missing_operand("&", "after"),
            | Self::BitwiseXorWithoutRhs => missing_operand("^", "after"),
            | Self::BitwiseOrWithoutRhs => missing_operand("|", "after"),
            | Self::LogicalAndWithoutRhs => missing_operand("&&", "after"),
            | Self::LogicalOrWithoutRhs => missing_operand("||", "after"),
            | Self::TernaryOperatorWithoutMhs => missing_operand("?", "after"),
            | Self::TernaryOperatorWithoutRhs => missing_operand(":", "after"),
            | Self::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "after"),
            | Self::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(operator) =>
                missing_operand(operator.spelling(), "before"),
            | Self::ColonWithoutMatchingQuestionMark =>
                Explanation::new("`:` without a matching `?`")
                    .label("no `?` precedes this `:` in the same parentheses"),
            | Self::CommaOperatorInPreprocessorExpression(_) =>
                Explanation::new("comma operator in `#if` expression")
                    .label("evaluated comma operator")
                    .note(
                        "C99 §6.6p3: constant expressions shall not contain comma operators, \
                         except within an operand that is not evaluated",
                    ),
            | Self::DivideByZero => Explanation::new("division by zero in `#if` expression")
                .label("the divisor is zero")
                .note("C99 §6.5.5p5: the result of `/` by zero is undefined"),
            | Self::ModuloByZero => Explanation::new("remainder by zero in `#if` expression")
                .label("the divisor is zero")
                .note("C99 §6.5.5p5: the result of `%` by zero is undefined"),
            | Self::UnaryMinusOverflow => overflow("negation"),
            | Self::BinaryPlusOverflow => overflow("addition"),
            | Self::BinaryMinusOverflow => overflow("subtraction"),
            | Self::MultiplyOverflow => overflow("multiplication"),
            | Self::DivideOverflow => overflow("division"),
            | Self::ModuloOverflow => overflow("remainder"),
            | Self::LeftShiftOverflow => overflow("left shift"),
            | Self::RightShiftOverflow => overflow("right shift"),
            | Self::UnterminatedOpeningParenthesisInPreprocessorExpression =>
                Explanation::new("unclosed `(` in `#if` expression").label("expected `)`"),
            | Self::TildeInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`~`"),
            | Self::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`!`"),
            | Self::NumberInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Number.found(spelling)),
            | Self::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Identifier.found(spelling)),
            | Self::CharacterInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator(&PreprocessorTokenType::Character.found(spelling)),
            | Self::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression =>
                instead_of_operator("`defined`"),
            | Self::ExpectedBinaryOperatorInPreprocessorExpression =>
                Explanation::new("expected an operator between the operands of `#if`")
                    .label("the expression ends with an operand still unjoined")
                    .note(IF_EXPRESSION_NOTE),
            | Self::AddressOfOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("`&` cannot take an address in `#if` expression")
                    .label("addresses do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::DereferenceOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("`*` cannot dereference in `#if` expression")
                    .label("pointers do not exist during preprocessing")
                    .note(IF_EXPRESSION_NOTE),
            | Self::FunctionCallOperatorNotSupportedInPreprocessorExpression =>
                Explanation::new("function call in `#if` expression")
                    .label("functions cannot be called during preprocessing")
                    .note(
                        "C99 §6.10.1p3: identifiers that are not macros evaluate to `0`, so this \
                         looks like a call of an undefined function-like macro",
                    )
                    .help("define the function-like macro before this directive"),
            | Self::FloatInsteadOfIntegerInPreprocessorExpression =>
                Explanation::new("floating constant in `#if` expression")
                    .label("not an integer")
                    .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenInPreprocessorExpression(kind) => Explanation::new(format!(
                "unexpected {} in `#if` expression",
                kind.found(spelling)
            ))
            .label("not valid in an integer constant expression")
            .note(IF_EXPRESSION_NOTE),
            | Self::UnexpectedTokenAtPhase7(kind) =>
                Explanation::new(format!("stray {} in program", kind.found(spelling)))
                    .label("only meaningful inside a preprocessing directive"),
            | Self::MissingOpeningParenthesisOrIdentifierInDefinedDirective(kind) =>
                Explanation::new(format!(
                    "expected a macro name after `defined`, found {}",
                    kind.found(spelling)
                ))
                .label("expected a macro name")
                .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingIdentifierInDefinedDirective(kind) => Explanation::new(format!(
                "expected a macro name inside `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.1p1: write `defined NAME` or `defined(NAME)`"),
            | Self::MissingClosingParenthesisInDefinedDirective(kind) => Explanation::new(format!(
                "expected `)` to close `defined(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::NoConditionInIfDirective => Explanation::new("`#if` with no condition")
                .label("expected an expression")
                .help("write the condition to test, as in `#if VERSION >= 2`"),
            | Self::NoConditionInElifDirective => Explanation::new("`#elif` with no condition")
                .label("expected an expression")
                .help("write the condition to test, or use `#else`"),
            | Self::MoreIfDirectivesThanEndifDirectives =>
                Explanation::new(format!("unterminated `#{}`", spelling.unwrap_or("if")))
                    .label("this conditional has no matching `#endif`")
                    .help("add `#endif` where the conditional section should end"),
            | Self::MoreEndifDirectivesThanIfDirectives =>
                Explanation::new("`#endif` without `#if`")
                    .label("no conditional directive is open here"),
            | Self::ElifDirectiveWithoutIfDirective => Explanation::new("`#elif` without `#if`")
                .label("no conditional directive is open here"),
            | Self::ElseDirectiveWithoutIfDirective => Explanation::new("`#else` without `#if`")
                .label("no conditional directive is open here"),
            | Self::ExpectedIdentifierInIfdefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#ifdef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedIdentifierInIfndefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#ifndef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedIdentifierInDefineDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#define`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name")
            .note("C99 §6.10.3: a macro name is an identifier"),
            | Self::RedefinitionOfBuiltInMacro(name) =>
                Explanation::new(format!("cannot redefine predefined macro `{name}`"))
                    .label("predefined by the implementation")
                    .note("C99 §6.10.8p4: predefined macro names shall not be redefined"),
            | Self::UndefinedIdentifierInPreprocessorExpression(name) =>
                Explanation::new(format!("`{name}` is not defined; it evaluates to 0"))
                    .label("not a macro")
                    .note(
                        "C99 §6.10.1p3: identifiers that are not macro names are replaced with \
                         `0` in `#if`",
                    )
                    .help(format!(
                        "use `defined({name})` to test whether it is defined"
                    )),
            | Self::ExpectedIncludeStringOrAngleBracketString(kind) => Explanation::new(format!(
                "expected a header name after `#include`, found {}",
                kind.found(spelling)
            ))
            .label("expected `\"file.h\"` or `<file.h>`")
            .note(
                "C99 §6.10.2: `#include` takes `\"name\"`, `<name>`, or macros that expand to one \
                 of them",
            ),
            | Self::UnexpectedEndOfInput(activity) => Explanation::new(format!(
                "unexpected end of file while {}",
                activity.trim_end_matches('.')
            ))
            .label("the file ends here"),
            | Self::WrongNumberOfArgumentsInFunctionLikeMacroInvocation { expected, found } =>
                Explanation::new(format!(
                    "this macro takes {} but {} {} supplied",
                    count_of(*expected, "argument"),
                    count_of(*found, "argument"),
                    if *found == 1 { "was" } else { "were" }
                ))
                .label(format!("expected {}", count_of(*expected, "argument")))
                .note(
                    "C99 §6.10.3p4: an invocation must supply one argument per parameter, plus \
                     any for `...`",
                ),
            | Self::HeaderNotFound {
                name,
                is_system_header,
                searched,
            } => {
                let mut explanation = Explanation::new(format!("cannot find header `{name}`"))
                    .label("not found in any search directory");
                if searched.is_empty() {
                    explanation =
                        explanation.note("the path is absolute; no directory was searched");
                } else {
                    let list: Vec<String> = searched
                        .iter()
                        .map(|directory| {
                            if directory.as_os_str().is_empty() {
                                "  . (the working directory)".to_owned()
                            } else {
                                format!("  {}", directory.display())
                            }
                        })
                        .collect();
                    explanation = explanation
                        .note(format!("searched these directories:\n{}", list.join("\n")));
                }
                explanation.help(if *is_system_header {
                    "add the directory containing it with `--isystem <dir>`"
                } else {
                    "add the directory containing it with `--iquote <dir>` or `--isystem <dir>`"
                })
            },
            | Self::HeaderFileInaccessible(error) =>
                Explanation::new(format!("cannot read included file: {error}"))
                    .label("included here"),
            | Self::HashHashUsedOutsideOfMacro =>
                Explanation::new("`##` outside a macro definition")
                    .label("token pasting only happens in replacement lists")
                    .note("C99 §6.10.3.3: `##` is an operator of macro replacement lists"),
            | Self::CannotUseHashHashAfterFunctionLikeMacroCall =>
                Explanation::new("`##` cannot follow a function-like macro invocation")
                    .label("pasting onto an invocation is not supported"),
            | Self::InvalidEscapeSequence => Explanation::new("unknown escape sequence")
                .label("contains an escape C99 does not define")
                .note(ESCAPE_LIST_NOTE),
            | Self::UnterminatedEscapeSequence => Explanation::new("incomplete escape sequence")
                .label("`\\` ends the literal")
                .help("write `\\\\` for a backslash character"),
            | Self::InvalidHexEscapeSequence =>
                Explanation::new("hexadecimal escape sequence is not a valid character")
                    .label("contains an invalid `\\x` escape"),
            | Self::HexEscapeSequenceTooLarge =>
                Explanation::new("hexadecimal escape sequence is out of range")
                    .label("contains an oversized `\\x` escape"),
            | Self::InvalidOctalEscapeSequence =>
                Explanation::new("octal escape sequence is not a valid character")
                    .label("contains an invalid octal escape"),
            | Self::OctalEscapeSequenceTooLarge =>
                Explanation::new("octal escape sequence is out of range")
                    .label("contains an oversized octal escape"),
            | Self::InvalidSmallUnicodeEscapeSequence =>
                Explanation::new("`\\u` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note("C99 §6.4.3: a universal character name must be a valid code point"),
            | Self::SmallUnicodeEscapeSequenceTooShort =>
                Explanation::new("`\\u` escape needs exactly four hexadecimal digits")
                    .label("contains a short `\\u` escape"),
            | Self::InvalidLargeUnicodeEscapeSequence =>
                Explanation::new("`\\U` escape does not name a valid character")
                    .label("contains an invalid universal character name")
                    .note("C99 §6.4.3: a universal character name must be a valid code point"),
            | Self::LargeUnicodeEscapeSequenceTooSmall =>
                Explanation::new("`\\U` escape needs exactly eight hexadecimal digits")
                    .label("contains a short `\\U` escape"),
            | Self::MultiCharacterLiteralsUnsupported => {
                if spelling.is_some_and(|spelling| spelling.ends_with("''")) {
                    Explanation::new("empty character constant")
                        .label("contains no character")
                        .note("C99 §6.4.4.4: a character constant contains at least one character")
                } else {
                    Explanation::new("wide character constant with more than one character")
                        .label("its value is implementation-defined")
                        .note(
                            "C99 §6.4.4.4p11: bcc does not assign a value to multi-character wide \
                             constants",
                        )
                }
            },
            | Self::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(name) => Explanation::new(
                format!("function-like macro `{name}` redefined as an object-like macro"),
            )
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format!("add `#undef {name}` before this definition")),
            | Self::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(name) => Explanation::new(
                format!("object-like macro `{name}` redefined as a function-like macro"),
            )
            .label("redefined here")
            .note("C99 §6.10.3p2: a macro may only be redefined identically")
            .help(format!("add `#undef {name}` before this definition")),
            | Self::ExpectedIdentifierInMacroDefinition(kind) => Explanation::new(format!(
                "expected a parameter name, found {}",
                kind.found(spelling)
            ))
            .label("expected an identifier or `...`"),
            | Self::VariadicMacroMustBeLastParameter(name) =>
                Explanation::new(format!("`...` must be the last parameter of `{name}`"))
                    .label("parameter after `...`")
                    .note("C99 §6.10.3p12: `...` ends the parameter list"),
            | Self::ExpectedCommaOrClosingParenthesisInMacroDefinition(kind) =>
                Explanation::new(format!(
                    "expected `,` or `)` in macro parameter list, found {}",
                    kind.found(spelling)
                ))
                .label("expected `,` or `)`"),
            | Self::MacroRedefinedWithDifferentDefinition(name) =>
                Explanation::new(format!("macro `{name}` redefined differently"))
                    .label("this definition differs from the previous one")
                    .note(
                        "C99 §6.10.3p2: a redefinition must have the same parameters and an \
                         identical replacement list",
                    )
                    .help(format!("add `#undef {name}` before this definition")),
            | Self::ExpectedIdentifierInUndefDirective(kind) => Explanation::new(format!(
                "expected a macro name after `#undef`, found {}",
                kind.found(spelling)
            ))
            .label("expected a macro name"),
            | Self::ExpectedNewlineAfterUndefDirective(kind) => Explanation::new(format!(
                "unexpected {} after the macro name in `#undef`",
                kind.found(spelling)
            ))
            .label("`#undef` takes one name"),
            | Self::HashOperatorMustBeFollowedByAMacroArgument(kind) => Explanation::new(format!(
                "expected a macro parameter after `#`, found {}",
                kind.found(spelling)
            ))
            .label("expected a parameter name")
            .note("C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a parameter"),
            | Self::IdentifierNotMacroArgumentAfterHashOperator(name) =>
                Explanation::new(format!("`{name}` is not a parameter of this macro"))
                    .label("`#` can only stringify a parameter")
                    .note(
                        "C99 §6.10.3.2p1: in a function-like macro, `#` must be followed by a \
                         parameter",
                    ),
            | Self::MissingRightHandSideOfHashHashOperator =>
                Explanation::new("`##` cannot end a replacement list")
                    .label("nothing follows this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::MissingLeftHandSideOfHashHashOperator =>
                Explanation::new("`##` cannot start a replacement list")
                    .label("nothing precedes this `##`")
                    .note("C99 §6.10.3.3p1: `##` needs a token on each side"),
            | Self::TokenMergingError(lhs, rhs) => Explanation::new(format!(
                "pasting `{lhs}` and `{rhs}` does not give a valid token"
            ))
            .label("invalid token paste")
            .note("C99 §6.10.3.3p3: the result of `##` must be a single valid preprocessing token"),
            | Self::MissingNumberInLineDirective(kind) => Explanation::new(format!(
                "expected a line number after `#line`, found {}",
                kind.found(spelling)
            ))
            .label("expected a line number"),
            | Self::MissingNewlineAfterLineDirective(kind) =>
                Explanation::new(format!("unexpected {} after `#line`", kind.found(spelling)))
                    .label("`#line` takes a number and an optional file name"),
            | Self::LineDirectiveIsNotASimpleDigitSequence =>
                Explanation::new("`#line` needs a plain decimal line number")
                    .label("not a digit sequence")
                    .note("C99 §6.10.4p3: the line number is a digit sequence, not any constant"),
            | Self::LineDirectiveNumberTooLarge(number) =>
                Explanation::new(format!("line number {number} is out of range"))
                    .label("too large")
                    .note("C99 §6.10.4p3: the line number must be at most 2147483647"),
            | Self::MissingOpeningParenthesisInPragmaOperator(kind) => Explanation::new(format!(
                "expected `(` after `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected `(`")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::MissingClosingParenthesisInPragmaOperator(kind) => Explanation::new(format!(
                "expected `)` to close `_Pragma(`, found {}",
                kind.found(spelling)
            ))
            .label("expected `)`"),
            | Self::MissingStringLiteralInPragmaOperator(kind) => Explanation::new(format!(
                "expected a string literal in `_Pragma`, found {}",
                kind.found(spelling)
            ))
            .label("expected a string literal")
            .note("C99 §6.10.9: write `_Pragma(\"...\")`"),
            | Self::UnknownPragmaDirective => Explanation::new("unknown pragma ignored")
                .label("not recognized")
                .note("bcc recognizes `#pragma once` and the `#pragma STDC` pragmas"),
            | Self::UnknownPragmaSTDCArgument(argument) =>
                Explanation::new(format!("unknown `STDC` pragma `{argument}`"))
                    .label("not a standard pragma")
                    .note(
                        "C99 §6.10.6: the standard pragmas are `FP_CONTRACT`, `FENV_ACCESS`, and \
                         `CX_LIMITED_RANGE`",
                    ),
            | Self::ExtraTokensAfterPragmaOnce(kind) => Explanation::new(format!(
                "unexpected {} after `#pragma once`",
                kind.found(spelling)
            ))
            .label("`#pragma once` takes no arguments"),
            | Self::ExtraTokensAfterPragmaOperator =>
                Explanation::new("extra tokens after the pragma in `_Pragma`")
                    .label("not part of the pragma"),
            | Self::ExtraTokensAfterIncludeDirective =>
                Explanation::new("extra tokens at end of `#include` directive").label("ignored"),
            | Self::ExtraTokensAfterIfdefDirective =>
                Explanation::new("extra tokens at end of `#ifdef` directive").label("ignored"),
            | Self::ExtraTokensAfterIfndefDirective =>
                Explanation::new("extra tokens at end of `#ifndef` directive").label("ignored"),
            | Self::STDCPragmaDirectiveWithoutArgument =>
                Explanation::new("expected a pragma name after `#pragma STDC`")
                    .label("expected `FP_CONTRACT`, `FENV_ACCESS`, or `CX_LIMITED_RANGE`"),
            | Self::STDCPragmaDirectiveWithoutOnOffSwitch =>
                Explanation::new("expected `ON`, `OFF`, or `DEFAULT` in `#pragma STDC`")
                    .label("the pragma ends here")
                    .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::MissingOnOffSwitchInSTDCPragma(argument) => Explanation::new(format!(
                "expected `ON`, `OFF`, or `DEFAULT` after `{argument}`"
            ))
            .label("expected an on-off switch")
            .note("C99 §6.10.6p2: each standard pragma takes an on-off switch"),
            | Self::PragmaOnceInNonHeader => Explanation::new("`#pragma once` in main file")
                .label("only affects files that are included"),
            | Self::ErrorDirective(message) => {
                let message = message.trim();
                Explanation::new(if message.is_empty() {
                    "#error".to_owned()
                } else {
                    format!("#error {message}")
                })
                .label("`#error` directive")
            },
        }
    }
}

impl Display for PreprocessorErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(&self.explain(None).message)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct FunctionLikeMacroArgument {
    name:                StringCacheId,
    tokenizer:           PreprocessorTokenizer,
    enclosing_arguments: Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>>,
    disabled_macros:     Rc<[StringCacheId]>,
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "Integer literal values are range-checked before narrowing."
)]
#[expect(
    clippy::while_let_loop,
    reason = "The macro-parameter loop has multiple semantic exit conditions."
)]
impl Preprocessor {
    pub(crate) fn new(
        context: &mut Context,
        source_name: Box<Path>,
        source: SharedString,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
    ) -> Self {
        let macro_definitions = PREDEFINED_MACRO_NAMES
            .into_iter()
            .map(|s| -> (StringCacheId, MacroDefinition) {
                (context.string_cache.intern(s), MacroDefinition::BuiltIn)
            })
            .collect();
        let source_file_index = context.intern_source_file(source_name);
        context.record_source_text(source_file_index, source.clone());
        let tokenizer = PreprocessorTokenizer::new(source_file_index, source);
        Self {
            tokenizer_stack: vec![TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                tokenizer:  tokenizer.clone(),
            }],
            hash_hash_stack: Vec::new(),
            once_set: HashSet::default(),
            tokenizer,
            macro_definitions,
            last_was_newline: true,
            current_is_newline: true,
            open_conditionals: Vec::new(),
            generate_placeholders: false,
            quote_include_directories,
            system_include_directories,
            expression_parser: PreprocessorExpressionParser::new(),
            pending_parser_token: None,
            pending_parser_errors: Vec::new(),
            translation_timestamp: None,
        }
    }

    fn next_parser_token(&mut self, context: &mut Context) -> Option<Token> {
        context.append_pending_errors(take(&mut self.pending_parser_errors));
        if let Some(token) = self.pending_parser_token.take() {
            return Some(token);
        }
        loop {
            let Some(token) = self.next_preprocessor_token::<true>(context) else {
                for vectors in take(&mut self.open_conditionals) {
                    let source_vectors = context.push_source_vectors(&vectors);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(context, token) {
                return Some(result);
            }
        }
    }

    /// Produces the next iterator item while keeping buffered provenance alive.
    ///
    /// Adjacent-string concatenation may already have mapped a later token or
    /// EOF diagnostic. Source-vector compaction therefore belongs to the
    /// producer that owns that buffered work, not to each iterator consumer.
    pub(crate) fn next_iterator_item(&mut self, context: &mut Context) -> Option<Token> {
        if self.next_iterator_item_compacts() {
            context.source_vectors.0.clear();
        }
        self.next_item(context)
    }

    /// Whether the next [`Self::next_iterator_item`] call discards the
    /// preprocessor provenance arena. Consumers retaining provenance across
    /// calls, such as a deferred diagnostic, must resolve it first.
    pub(crate) fn next_iterator_item_compacts(&self) -> bool {
        self.pending_parser_token.is_none() && self.pending_parser_errors.is_empty()
    }

    fn concatenate_adjacent_strings(&mut self, context: &mut Context, first: Token) -> Token {
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

    fn skip_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.tokenizer.next_item(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    fn skip_and_expand_until_newline(&mut self, context: &mut Context) {
        loop {
            if matches!(
                self.next_preprocessor_token::<true>(context),
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                }) | None
            ) {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            }
        }
    }

    fn push_tokenizer_frame(&mut self, _context: &mut Context, frame: TokenizerFrame) {
        self.tokenizer_stack.last_mut().unwrap().tokenizer = take(&mut self.tokenizer);
        self.tokenizer = frame.tokenizer.clone();
        self.tokenizer_stack.push(frame);
    }

    fn pop_tokenizer_frame(&mut self, _context: &mut Context) {
        let f = self.tokenizer_stack.pop();
        drop(f);
        if let Some(last) = self.tokenizer_stack.last() {
            self.tokenizer = last.tokenizer.clone();
        }
    }

    fn expect_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_token_with_rewind::<SHOULD_IGNORE_WHITESPACE>(
            context,
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            true,
        )
    }

    fn expect_token_without_rewind<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        self.expect_token_with_rewind::<SHOULD_IGNORE_WHITESPACE>(
            context,
            is_correct_token,
            on_wrong_token_type,
            eof_message,
            false,
        )
    }

    fn expect_token_with_rewind<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
        rewind_on_error: bool,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.position(context);
            match self.next_preprocessor_token::<SHOULD_IGNORE_WHITESPACE>(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            if rewind_on_error {
                                self.set_position(context, start);
                            }
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    if rewind_on_error {
                        self.set_position(context, start);
                    }
                    let source_vectors =
                        context.create_source_vectors(start, self.source_file_index(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    fn expect_token_from_previous_phase<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
        mut is_correct_token: impl FnMut(&mut Self, &mut Context, PreprocessorToken) -> bool,
        mut on_wrong_token_type: impl FnMut(
            &mut Self,
            &mut Context,
            PreprocessorToken,
        ) -> ControlFlow<PreprocessorError>,
        eof_message: &'static str,
    ) -> Option<PreprocessorToken> {
        loop {
            let start = self.position(context);
            match self.tokenizer.next_item(context) {
                | Some(token) => {
                    if SHOULD_IGNORE_WHITESPACE && token.kind == PreprocessorTokenType::Whitespace {
                        continue;
                    }
                    if is_correct_token(self, context, token) {
                        return Some(token);
                    }
                    match on_wrong_token_type(self, context, token) {
                        | ControlFlow::Continue(()) => continue,
                        | ControlFlow::Break(e) => {
                            self.set_position(context, start);
                            context.preprocessor_error(e);
                            return None;
                        },
                    }
                },
                | None => {
                    self.set_position(context, start);
                    let source_vectors =
                        context.create_source_vectors(start, self.source_file_index(), 0);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(eof_message),
                        source_vectors,
                    });
                    return None;
                },
            }
        }
    }

    fn merge_token_contents(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        self.merge_token_contents_with_ranges(context, lhs, rhs, .., .., result_token_type)
    }

    fn merge_token_contents_with_ranges(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
        lhs_range: impl RangeBounds<usize>,
        rhs_range: impl RangeBounds<usize>,
        result_token_type: PreprocessorTokenType,
    ) -> PreprocessorToken {
        _ = self;
        let mut new_contents = TokenString::new();
        new_contents.push_str(
            &context.string_cache.at(lhs.contents)[(
                lhs_range.start_bound().cloned(),
                lhs_range.end_bound().cloned(),
            )],
        );
        new_contents.push_str(
            &context.string_cache.at(rhs.contents)[(
                rhs_range.start_bound().cloned(),
                rhs_range.end_bound().cloned(),
            )],
        );
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        PreprocessorToken {
            kind: result_token_type,
            contents: context.string_cache.intern(&new_contents),
            source_vectors,
        }
    }

    fn create_merge_error(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> PreprocessorToken {
        _ = self;
        let lhs_contents = context.string_cache.at(lhs.contents).to_string();
        let rhs_contents = context.string_cache.at(rhs.contents).to_string();
        let source_vectors = context.merge_vectors(lhs.source_vectors, rhs.source_vectors);
        context.preprocessor_error(PreprocessorError {
            error_type: PreprocessorErrorType::TokenMergingError(lhs_contents, rhs_contents),
            source_vectors,
        });
        lhs
    }

    fn merge_tokens(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        match (lhs.kind, rhs.kind) {
            | (PreprocessorTokenType::Placeholder, PreprocessorTokenType::Placeholder) => None,
            | (PreprocessorTokenType::Placeholder, _) => Some(rhs),
            | (_, PreprocessorTokenType::Placeholder) => Some(lhs),
            | (
                PreprocessorTokenType::Identifier | PreprocessorTokenType::Defined,
                PreprocessorTokenType::Identifier
                | PreprocessorTokenType::Defined
                | PreprocessorTokenType::Number,
            ) => {
                let rhs_contents = context.string_cache.at(rhs.contents);
                if rhs_contents.contains('.') {
                    return Some(self.create_merge_error(context, lhs, rhs));
                }
                let rhs_range = if rhs.kind == PreprocessorTokenType::Number {
                    0..rhs_contents.len() - 1
                } else {
                    0..rhs_contents.len()
                };
                let new = self.merge_token_contents_with_ranges(
                    context,
                    lhs,
                    rhs,
                    ..,
                    rhs_range,
                    PreprocessorTokenType::Identifier,
                );
                let kind = if context.string_cache.at(new.contents) == "defined" {
                    PreprocessorTokenType::Defined
                } else {
                    PreprocessorTokenType::Identifier
                };
                Some(PreprocessorToken {
                    kind,
                    contents: new.contents,
                    source_vectors: new.source_vectors,
                })
            },
            | (
                PreprocessorTokenType::Identifier,
                PreprocessorTokenType::String
                | PreprocessorTokenType::Character
                | PreprocessorTokenType::GeneratedString,
            ) => {
                // C99 §6.10.3.3p3: `L ## "x"` forms the wide literal `L"x"`,
                // so the right-hand side must still be a narrow literal.
                if context.string_cache.at(lhs.contents) == "L"
                    && !context.string_cache.at(rhs.contents).starts_with('L')
                {
                    Some(self.merge_token_contents(
                        context,
                        lhs,
                        rhs,
                        match rhs.kind {
                            | PreprocessorTokenType::GeneratedString =>
                                PreprocessorTokenType::WideGeneratedString,
                            | _ => rhs.kind,
                        },
                    ))
                } else {
                    Some(self.create_merge_error(context, lhs, rhs))
                }
            },
            | (
                PreprocessorTokenType::Period | PreprocessorTokenType::Number,
                PreprocessorTokenType::Number,
            )
            | (PreprocessorTokenType::Number, PreprocessorTokenType::Period) => {
                let lhs_contents = context.string_cache.at(lhs.contents);
                let lhs_range = if lhs.kind == PreprocessorTokenType::Number {
                    0..lhs_contents.len() - 1
                } else {
                    0..lhs_contents.len()
                };
                // We don't remove the trailing null byte from the right-hand
                // side because we're generating a new number
                // token, and number tokens should always have a
                // trailing null byte.
                Some(self.merge_token_contents_with_ranges(
                    context,
                    lhs,
                    rhs,
                    lhs_range,
                    ..,
                    PreprocessorTokenType::Number,
                ))
            },
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Plus) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusPlus)),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Minus) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusMinus),
            ),
            | (PreprocessorTokenType::Plus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PlusEquals),
            ),
            | (PreprocessorTokenType::Minus, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::MinusEquals),
            ),
            | (PreprocessorTokenType::Asterisk, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::AsteriskEquals),
            ),
            | (PreprocessorTokenType::ForwardSlash, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ForwardSlashEquals,
                )),
            | (PreprocessorTokenType::Percent, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PercentEquals),
            ),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::LessThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThan,
                )),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::GreaterThan) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThan,
                )),
            | (PreprocessorTokenType::LessThan, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::LessThanEquals),
            ),
            | (PreprocessorTokenType::GreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanEquals,
                )),
            | (PreprocessorTokenType::LessThanLessThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::LessThanLessThanEquals,
                )),
            | (PreprocessorTokenType::GreaterThanGreaterThan, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::GreaterThanGreaterThanEquals,
                )),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandEquals,
                )),
            | (PreprocessorTokenType::Caret, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::CaretEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipeEquals),
            ),
            | (PreprocessorTokenType::ExclamationMark, PreprocessorTokenType::Equals) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::ExclamationMarkEquals,
                )),
            | (PreprocessorTokenType::Equals, PreprocessorTokenType::Equals) => Some(
                self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::EqualsEquals),
            ),
            | (PreprocessorTokenType::Pipe, PreprocessorTokenType::Pipe) =>
                Some(self.merge_token_contents(context, lhs, rhs, PreprocessorTokenType::PipePipe)),
            | (PreprocessorTokenType::Ampersand, PreprocessorTokenType::Ampersand) =>
                Some(self.merge_token_contents(
                    context,
                    lhs,
                    rhs,
                    PreprocessorTokenType::AmpersandAmpersand,
                )),

            | _ => Some(self.create_merge_error(context, lhs, rhs)),
        }
    }

    fn expand_macros<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            if self.tokenizer_stack.is_empty() {
                std::hint::cold_path();
                break 'base None;
            }
            match self.tokenizer.next_item(context) {
                | Some(token)
                    if token.kind == PreprocessorTokenType::Whitespace
                        && SHOULD_IGNORE_WHITESPACE =>
                {
                    continue 'base;
                },
                | Some(mut token) => {
                    match self.tokenizer_stack.last_mut().unwrap() {
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                                | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                            ..
                        } =>
                            if token.kind == PreprocessorTokenType::Newline {
                                self.pop_tokenizer_frame(context);
                                continue 'base;
                            },
                        | TokenizerFrame {
                            frame_type:
                                TokenizerFrameType::FunctionLikeMacroArgument {
                                    paren_depth,
                                    argument,
                                    has_generated_token,
                                },
                            ..
                        } => {
                            let paren_depth = *paren_depth;
                            let argument = argument.clone();
                            let has_generated_token = *has_generated_token;
                            if let Some(paren_depth) = self.update_macro_argument_paren_depth(
                                context,
                                token,
                                argument.name,
                                paren_depth,
                            ) {
                                let TokenizerFrame {
                                    frame_type:
                                        TokenizerFrameType::FunctionLikeMacroArgument {
                                            paren_depth: p,
                                            has_generated_token,
                                            ..
                                        },
                                    ..
                                } = self.tokenizer_stack.last_mut().unwrap()
                                else {
                                    unreachable!();
                                };
                                *p = paren_depth;
                                *has_generated_token = true;
                            } else {
                                self.pop_tokenizer_frame(context);
                                if self.generate_placeholders && !has_generated_token {
                                    break 'base Some(PreprocessorToken {
                                        kind:           PreprocessorTokenType::Placeholder,
                                        contents:       context.string_cache.intern(""),
                                        source_vectors: SourceVectors::default(),
                                    });
                                }
                                continue 'base;
                            }
                            if token.kind == PreprocessorTokenType::Newline {
                                if SHOULD_IGNORE_WHITESPACE {
                                    continue 'base;
                                }
                                token.kind = PreprocessorTokenType::Whitespace;
                                token.contents = context.string_cache.intern(" ");
                            }
                        },
                        | TokenizerFrame {
                            frame_type: TokenizerFrameType::SourceFile,
                            ..
                        } => (),
                    }
                    break Some(token);
                },
                | None => {
                    self.pop_tokenizer_frame(context);
                    continue 'base;
                },
            }
        }
    }

    fn update_macro_argument_paren_depth(
        &self,
        context: &Context,
        token: PreprocessorToken,
        argument_name: StringCacheId,
        paren_depth: usize,
    ) -> Option<usize> {
        _ = self;
        if (token.kind == PreprocessorTokenType::Comma
            && context.string_cache.at(argument_name) != "__VA_ARGS__")
            || (token.kind == PreprocessorTokenType::ClosingParenthesis && paren_depth == 1)
        {
            return None;
        }
        if token.kind == PreprocessorTokenType::OpeningParenthesis {
            return Some(paren_depth + 1);
        }
        if token.kind == PreprocessorTokenType::ClosingParenthesis {
            return Some(paren_depth - 1);
        }
        Some(paren_depth)
    }

    fn current_is_header(&self, _context: &Context) -> bool {
        // The first in the tokenizer stack is the original source file.
        for frame in self.tokenizer_stack.iter().skip(1).rev() {
            match frame.frame_type {
                | TokenizerFrameType::SourceFile => return true,
                | _ => (),
            }
        }
        false
    }

    #[expect(
        dead_code,
        clippy::type_complexity,
        reason = "Macro-frame inspection is retained for pending expansion paths."
    )]
    fn current_function_like_macro(
        &self,
        _context: &Context,
    ) -> Option<(
        u32,
        PreprocessorTokenizer,
        Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>,
        bool,
    )> {
        match self.tokenizer_stack.last() {
            | Some(TokenizerFrame {
                frame_type:
                    TokenizerFrameType::FunctionLikeMacroInvocation {
                        arguments,
                        is_variadic,
                        ..
                    },
                tokenizer,
            }) => Some((
                tokenizer.source_file_index(),
                tokenizer.clone(),
                arguments.clone(),
                *is_variadic,
            )),
            | _ => None,
        }
    }

    #[expect(
        dead_code,
        reason = "Macro-frame inspection is retained for pending expansion paths."
    )]
    fn current_is_function_like_macro(&self, _context: &Context) -> bool {
        matches!(
            self.tokenizer_stack.last(),
            Some(TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                ..
            })
        )
    }

    fn handle_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        let token = self.expand_macros::<SHOULD_IGNORE_WHITESPACE>(context)?;

        match token.kind {
            | PreprocessorTokenType::Hash => {
                if matches!(
                    self.tokenizer_stack.last(),
                    Some(TokenizerFrame {
                        frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { .. },
                        ..
                    })
                ) {
                    Some(self.parse_hash_operator(context, token))
                } else {
                    Some(token)
                }
            },
            | _ => Some(token),
        }
    }

    fn handle_hash_hash_operator<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        'base: loop {
            let lhs = self.handle_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)?;
            let replacement_list = match self.tokenizer_stack.last().map(|frame| &frame.frame_type)
            {
                | Some(
                    TokenizerFrameType::FunctionLikeMacroInvocation { .. }
                    | TokenizerFrameType::ObjectLikeMacroInvocation { .. },
                ) => true,
                | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                    argument.enclosing_arguments.is_some(),
                | _ => false,
            };
            let hash_hash = if replacement_list {
                let save = self.position(context);
                context.set_ignore_tokenizer_errors(true);
                let hash_hash = Self::next_ignore_whitespace(&mut self.tokenizer, context);
                context.set_ignore_tokenizer_errors(false);
                self.set_position(context, save);
                if hash_hash.is_some_and(|v| v.kind == PreprocessorTokenType::HashHash) {
                    Self::next_ignore_whitespace(&mut self.tokenizer, context)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(h) = hash_hash {
                let Some(rhs) = self.handle_hash_hash_operator::<true>(context) else {
                    let source_vectors = context.create_source_vectors(
                        self.tokenizer.position(context),
                        self.tokenizer.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                            "parsing hash-hash operator. Hash hash operator must be followed by a \
                             preprocessor token on the same line.",
                        ),
                        source_vectors,
                    });
                    return Some(lhs);
                };
                if let Some(r) = self.parse_hash_hash_operator(context, lhs, h, rhs) {
                    return Some(r);
                }
                continue 'base;
            }
            return Some(lhs);
        }
    }

    fn next_preprocessor_token<const SHOULD_IGNORE_WHITESPACE: bool>(
        &mut self,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        self.last_was_newline = self.current_is_newline;
        let ret = 'base: loop {
            self.generate_placeholders = true;
            let Some(mut token) =
                self.handle_hash_hash_operator::<SHOULD_IGNORE_WHITESPACE>(context)
            else {
                break 'base None;
            };
            self.generate_placeholders = false;
            if !self.hash_hash_stack.is_empty() {
                'merge: loop {
                    let next_is_end = self.macro_argument_is_at_end(context);
                    let argument_continues = matches!(
                        self.tokenizer_stack.last(),
                        Some(TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroArgument { .. },
                            ..
                        })
                    ) && !next_is_end;
                    let new = match self.hash_hash_stack.last() {
                        | None | Some(HashHash::Empty) => None,
                        | Some(HashHash::Lhs(lhs)) => {
                            let lhs = *lhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, lhs, token)
                        },
                        | Some(HashHash::Rhs(rhs)) => {
                            // Paste the last token of the left argument.
                            if argument_continues {
                                break 'merge;
                            }
                            let rhs = *rhs;
                            _ = self.hash_hash_stack.pop();
                            self.merge_tokens(context, token, rhs)
                        },
                    };
                    if (next_is_end || token.kind == PreprocessorTokenType::Placeholder)
                        && let Some(x @ HashHash::Empty) = self.hash_hash_stack.last_mut()
                    {
                        *x = HashHash::Lhs(if let Some(new) = new { new } else { token });
                        continue 'base;
                    }
                    if let Some(new) = new {
                        token = new;
                    } else {
                        break 'merge;
                    }
                }
            }
            if token.kind == PreprocessorTokenType::Placeholder {
                continue 'base;
            }
            if token.kind != PreprocessorTokenType::Identifier {
                break 'base Some(token);
            }

            if self.macro_is_disabled(token.contents) {
                break 'base Some(token);
            }
            if let Some(md) = self.macro_definitions.get(&token.contents).cloned() {
                match md {
                    | MacroDefinition::ObjectLike { tokenizer } => {
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::ObjectLikeMacroInvocation {
                                name: token.contents,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::FunctionLike {
                        argument_names,
                        tokenizer,
                        is_variadic,
                    } => {
                        let position = self.position(context);
                        loop {
                            match self.tokenizer.next_item(context) {
                                | Some(brace) if brace.kind == PreprocessorTokenType::Whitespace =>
                                    continue,
                                | Some(brace)
                                    if brace.kind == PreprocessorTokenType::OpeningParenthesis =>
                                    break,
                                // C99 §6.10.3p10: without a following `(` the
                                // name is not an invocation and stays as is.
                                | Some(_) | None => {
                                    self.set_position(context, position);
                                    break 'base Some(token);
                                },
                            }
                        }
                        let mut i = 0;
                        let enclosing_arguments = self.get_arguments(context);
                        let disabled_macros = self.disabled_macros();
                        let mut arguments = HashMap::default();
                        let mut paren_depth = 1isize;
                        macro_rules! at {
                            () => {
                                argument_names
                                    .get(i)
                                    .copied()
                                    .unwrap_or(context.string_cache.intern("<undefined>"))
                            };
                        }
                        'outer: loop {
                            if is_variadic && i >= argument_names.len() {
                                break;
                            }
                            let tokenizer = self.tokenizer.clone();
                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            drop(arguments.insert(
                                                at!(),
                                                FunctionLikeMacroArgument {
                                                    name: at!(),
                                                    tokenizer,
                                                    enclosing_arguments:
                                                        enclosing_arguments.clone(),
                                                    disabled_macros: disabled_macros.clone(),
                                                },
                                            ));
                                            break 'outer;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token) if token.kind == PreprocessorTokenType::Comma => {
                                        drop(arguments.insert(
                                            at!(),
                                            FunctionLikeMacroArgument {
                                                name: at!(),
                                                tokenizer,
                                                enclosing_arguments: enclosing_arguments.clone(),
                                                disabled_macros: disabled_macros.clone(),
                                            },
                                        ));
                                        i += 1;
                                        continue 'outer;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.set_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }

                        if arguments.len() != argument_names.len() && !is_variadic {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::WrongNumberOfArgumentsInFunctionLikeMacroInvocation {
                                        expected:   argument_names.len(),
                                        found:      arguments.len(),
                                    },
                                    source_vectors: token.source_vectors,
                                },
                            );
                        }
                        if is_variadic {
                            drop(arguments.insert(
                                context.string_cache.intern("__VA_ARGS__"),
                                FunctionLikeMacroArgument {
                                    name:                context.string_cache.intern("__VA_ARGS__"),
                                    tokenizer:           self.tokenizer.clone(),
                                    enclosing_arguments: enclosing_arguments.clone(),
                                    disabled_macros:     disabled_macros.clone(),
                                },
                            ));
                            let mut paren_depth = 1isize;

                            loop {
                                match self.tokenizer.next_item(context) {
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::ClosingParenthesis =>
                                    {
                                        if paren_depth == 1 {
                                            break;
                                        }
                                        paren_depth -= 1;
                                    },
                                    | Some(token)
                                        if token.kind
                                            == PreprocessorTokenType::OpeningParenthesis =>
                                    {
                                        paren_depth += 1;
                                        continue;
                                    },
                                    | Some(_) => {
                                        continue;
                                    },
                                    | None => {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnexpectedEndOfInput(
                                                    "parsing function-like macro invocation",
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        self.set_position(context, position);
                                        break 'base Some(token);
                                    },
                                }
                            }
                        }
                        let frame = TokenizerFrame {
                            frame_type: TokenizerFrameType::FunctionLikeMacroInvocation {
                                name: token.contents,
                                arguments: Rc::new(arguments),
                                is_variadic,
                            },
                            tokenizer,
                        };
                        self.push_tokenizer_frame(context, frame);
                        continue;
                    },
                    | MacroDefinition::BuiltIn => match context.string_cache.at(token.contents) {
                        // C99 §6.10.8p1: each built-in expands to an ordinary
                        // token spelled as C source, located at the invocation.
                        | "__FILE__" => {
                            let spelling = string_literal_spelling(
                                &context.source_files[self.source_file_index()].to_string_lossy(),
                            );
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | "__LINE__" => {
                            // Number spellings carry the trailing NUL that
                            // numeric conversion expects.
                            let spelling = format!("{}\0", self.line(context));
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::Number,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | name @ ("__DATE__" | "__TIME__") => {
                            let is_date = name == "__DATE__";
                            let timestamp = self
                                .translation_timestamp
                                .get_or_insert_with(TranslationTimestamp::now);
                            let spelling = string_literal_spelling(if is_date {
                                &timestamp.date
                            } else {
                                &timestamp.time
                            });
                            break 'base Some(PreprocessorToken {
                                kind:           PreprocessorTokenType::String,
                                contents:       context.string_cache.intern(&spelling),
                                source_vectors: token.source_vectors,
                            });
                        },
                        | "_Pragma" => {
                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::OpeningParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            let Some(string_token) = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::String,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingStringLiteralInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            ) else {
                                continue 'base;
                            };

                            let input =
                                self.prepare_pragma_operator_string(context, string_token.contents);

                            let tokenizer = take(&mut self.tokenizer);
                            // Each operator gets its own identity: diagnostics
                            // rendered later must quote this payload, not the
                            // most recent one.
                            let pragma_string = context.add_synthetic_source_file(
                                PathBuf::from("<pragma string>").into_boxed_path(),
                                input.clone(),
                            );
                            self.tokenizer = PreprocessorTokenizer::new(pragma_string, input);
                            self.parse_pragma_directive(context, string_token);
                            if self.tokenizer.next_item(context).is_some() {
                                let source_vectors = context.create_source_vectors(
                                    self.position(context),
                                    self.source_file_index(),
                                    0,
                                );
                                context.preprocessor_error(PreprocessorError {
                                    error_type:
                                        PreprocessorErrorType::ExtraTokensAfterPragmaOperator,
                                    source_vectors,
                                });
                            }
                            self.tokenizer = tokenizer;

                            _ = self.expect_token::<true>(
                                context,
                                |_, _, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                                |_, _, token|
                                    ControlFlow::Break(PreprocessorError {
                                            error_type:     PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(token.kind),
                                            source_vectors: token.source_vectors,
                                        },
                                    ),
                                "parsing pragma operator",
                            );

                            continue 'base;
                        },
                        | s => unreachable!(
                            "Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"
                        ),
                    },
                }
            }
            if let Some(frame) = self.handle_macro_argument(context, token) {
                self.push_tokenizer_frame(context, frame);
                continue;
            }
            break 'base Some(token);
        };
        self.generate_placeholders = false;
        self.current_is_newline = ret.is_none_or(|t| t.kind == PreprocessorTokenType::Newline);
        ret
    }

    fn next_ignore_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(t) if t.kind == PreprocessorTokenType::Whitespace => continue,
                | Some(t) => return Some(t),
                | None => return None,
            }
        }
    }

    fn next_treat_newlines_as_whitespace(
        tokenizer: &mut PreprocessorTokenizer,
        context: &mut Context,
        last_was_whitespace: &mut bool,
    ) -> Option<PreprocessorToken> {
        loop {
            match tokenizer.next_item(context) {
                | Some(mut t) => {
                    match t.kind {
                        | PreprocessorTokenType::Whitespace => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                        },
                        | PreprocessorTokenType::Newline => {
                            if *last_was_whitespace {
                                continue;
                            }
                            *last_was_whitespace = true;
                            t.contents = context.string_cache.intern(" ");
                            t.kind = PreprocessorTokenType::Whitespace;
                        },
                        | _ => *last_was_whitespace = false,
                    }
                    return Some(t);
                },
                | None => return None,
            }
        }
    }

    fn prepare_pragma_operator_string(
        &self,
        context: &Context,
        string: StringCacheId,
    ) -> SharedString {
        _ = self;
        let string = context.string_cache.at(string);
        let mut ret = String::new();
        // Skip the leading quote.
        let mut index = 1;
        if string.char_at(0) == Some('L') {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            match c {
                | '\\' => match string.char_at(index + 1) {
                    | Some('"') => {
                        ret.push('"');
                        index += 2;
                    },
                    | Some('\\') => {
                        ret.push('\\');
                        index += 2;
                    },
                    | _ => (),
                },
                | _ => {
                    ret.push(c);
                    index += c.len_utf8();
                },
            }
        }
        // Pop the trailing quote.
        if ret.char_at(ret.len() - 1) == Some('"') {
            _ = ret.pop();
        }
        ret.push('\n');
        SharedString::from(ret)
    }

    fn get_arguments(
        &self,
        _context: &Context,
    ) -> Option<Rc<HashMap<StringCacheId, FunctionLikeMacroArgument>>> {
        match self.tokenizer_stack.last().map(|frame| &frame.frame_type) {
            // Argument tokens belong to the invocation's caller. Looking them
            // up in the callee's map can make a same-named parameter expand itself.
            | Some(TokenizerFrameType::FunctionLikeMacroArgument { argument, .. }) =>
                argument.enclosing_arguments.clone(),
            | Some(TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. }) =>
                Some(arguments.clone()),
            | _ => None,
        }
    }

    fn macro_argument_is_at_end(&mut self, context: &mut Context) -> bool {
        let Some(TokenizerFrame {
            frame_type:
                TokenizerFrameType::FunctionLikeMacroArgument {
                    argument,
                    paren_depth,
                    ..
                },
            ..
        }) = self.tokenizer_stack.last()
        else {
            return false;
        };
        let name = argument.name;
        let depth = *paren_depth;
        let position = self.position(context);
        let next_is_end = loop {
            match self.tokenizer.next_item(context) {
                | Some(token)
                    if matches!(
                        token.kind,
                        PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                    ) =>
                    continue,
                | Some(token) =>
                    break self
                        .update_macro_argument_paren_depth(context, token, name, depth)
                        .is_none(),
                | None => break true,
            }
        };
        self.set_position(context, position);
        next_is_end
    }

    fn macro_is_disabled(&self, name: StringCacheId) -> bool {
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    return argument.disabled_macros.contains(&name);
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name: active }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name: active, .. }
                    if *active == name =>
                    return true,
                | _ => (),
            }
        }
        false
    }

    fn disabled_macros(&self) -> Rc<[StringCacheId]> {
        let mut names = Vec::new();
        for frame in self.tokenizer_stack.iter().rev() {
            match &frame.frame_type {
                | TokenizerFrameType::FunctionLikeMacroArgument { argument, .. } => {
                    names.extend_from_slice(&argument.disabled_macros);
                    break;
                },
                | TokenizerFrameType::ObjectLikeMacroInvocation { name }
                | TokenizerFrameType::FunctionLikeMacroInvocation { name, .. } => names.push(*name),
                | _ => (),
            }
        }
        Rc::from(names)
    }

    fn handle_macro_argument(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> Option<TokenizerFrame> {
        if let Some(arguments) = self.get_arguments(context)
            && let Some(arg) = arguments.get(&token.contents)
        {
            let frame = TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroArgument {
                    argument:            Box::new(arg.clone()),
                    paren_depth:         1,
                    has_generated_token: false,
                },
                tokenizer:  arg.tokenizer.clone(),
            };
            return Some(frame);
        }

        None
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

    fn parse_number(&mut self, context: &mut Context, token: PreprocessorToken) -> Token {
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

    fn parse_character(
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

    fn map_preprocessor_token(
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

    fn parse_hash_hash_operator(
        &mut self,
        context: &mut Context,
        lhs: PreprocessorToken,
        _hash_hash: PreprocessorToken,
        rhs: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        let lhs_frame = self.handle_macro_argument(context, lhs);
        let rhs_frame = self.handle_macro_argument(context, rhs);
        self.hash_hash_stack.push(HashHash::Empty);
        let rhs_is_macro_argument = if let Some(frame) = rhs_frame {
            self.push_tokenizer_frame(context, frame);
            true
        } else {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Rhs(rhs);
            false
        };
        if let Some(frame) = lhs_frame {
            self.push_tokenizer_frame(context, frame);
        } else if rhs_is_macro_argument {
            *self.hash_hash_stack.last_mut().unwrap() = HashHash::Lhs(lhs);
        } else {
            _ = self.hash_hash_stack.pop();
            return self.merge_tokens(context, lhs, rhs);
        }
        None
    }

    fn parse_hash_operator(
        &mut self,
        context: &mut Context,
        token: PreprocessorToken,
    ) -> PreprocessorToken {
        let position = self.position(context);
        let Some(argument_name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::HashOperatorMustBeFollowedByAMacroArgument(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing '#' operator in function-like macro invocation.",
        ) else {
            self.set_position(context, position);
            return PreprocessorToken {
                kind:           PreprocessorTokenType::GeneratedString,
                contents:       context.string_cache.intern(""),
                source_vectors: token.source_vectors,
            };
        };
        let (mut token_tokenizer, argument_id) = match self.tokenizer_stack.last().unwrap() {
            | TokenizerFrame {
                frame_type: TokenizerFrameType::FunctionLikeMacroInvocation { arguments, .. },
                ..
            } => match arguments.get(&argument_name.contents) {
                | None => {
                    self.set_position(context, position);
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::IdentifierNotMacroArgumentAfterHashOperator(
                                context.string_cache.at(argument_name.contents).to_owned(),
                            ),
                        source_vectors: argument_name.source_vectors,
                    });
                    return PreprocessorToken {
                        kind:           PreprocessorTokenType::GeneratedString,
                        contents:       context.string_cache.intern(""),
                        source_vectors: token.source_vectors,
                    };
                },
                | Some(v) => (v.tokenizer.clone(), v.name),
            },
            | _ => unreachable!(),
        };
        let mut last_was_whitespace = true;
        let mut synthetic_contents = String::new();
        let mut paren_depth = 1;
        'base: loop {
            let Some(token) = Self::next_treat_newlines_as_whitespace(
                &mut token_tokenizer,
                context,
                &mut last_was_whitespace,
            ) else {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing '#' operator in function-like macro invocation",
                    ),
                    source_vectors: argument_name.source_vectors,
                });
                break 'base;
            };
            match self.update_macro_argument_paren_depth(context, token, argument_id, paren_depth) {
                | Some(depth) => paren_depth = depth,
                | None => break,
            }
            let mut s = context.string_cache.at(token.contents);
            if token.kind == PreprocessorTokenType::Number {
                // Remove trailing null byte.
                s = &s[..s.len() - 1];
            }
            synthetic_contents.push_str(s);
        }
        PreprocessorToken {
            kind:           PreprocessorTokenType::GeneratedString,
            contents:       context.string_cache.intern(&synthetic_contents),
            source_vectors: token.source_vectors,
        }
    }

    fn parse_directive(&mut self, context: &mut Context, token: PreprocessorToken) {
        if !self.last_was_newline {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                source_vectors: token.source_vectors,
            });
        }
        let Some(directive) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
            return;
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => return,
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined | PreprocessorTokenType::Identifier => (),
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
                return;
            },
        }
        match context.string_cache.at(directive.contents) {
            | "if" => self.parse_if_directive(context, directive),
            | "ifdef" => self.parse_ifdef_directive(context, directive),
            | "ifndef" => self.parse_ifndef_directive(context, directive),
            | "elif" => self.parse_elif_directive(context, directive),
            | "else" => self.parse_else_directive(context, directive),
            | "endif" => self.parse_endif_directive(context, directive),
            | "include" => self.parse_include_directive(context, directive),
            | "define" => self.parse_define_directive(context, directive),
            | "undef" => self.parse_undef_directive(context, directive),
            | "line" => self.parse_line_directive(context, directive),
            | "error" => self.parse_error_directive(context, directive),
            | "pragma" => self.parse_pragma_directive(context, directive),
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
            },
        }
    }

    fn map_operator(
        &mut self,
        _context: &Context,
        operator: PreprocessorToken,
    ) -> PreprocessorExpressionOperator {
        let state = replace(
            &mut self.expression_parser.state,
            PreprocessorExpressionParserState::Unary,
        );
        match operator.kind {
            | PreprocessorTokenType::Plus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryPlus
                } else {
                    PreprocessorExpressionOperator::BinaryPlus
                },
            | PreprocessorTokenType::Minus =>
                if state == PreprocessorExpressionParserState::Unary {
                    PreprocessorExpressionOperator::UnaryMinus
                } else {
                    PreprocessorExpressionOperator::BinaryMinus
                },
            | PreprocessorTokenType::Asterisk => PreprocessorExpressionOperator::Multiply,
            | PreprocessorTokenType::ForwardSlash => PreprocessorExpressionOperator::Divide,
            | PreprocessorTokenType::Percent => PreprocessorExpressionOperator::Modulo,
            | PreprocessorTokenType::LessThanLessThan => PreprocessorExpressionOperator::LeftShift,
            | PreprocessorTokenType::GreaterThanGreaterThan =>
                PreprocessorExpressionOperator::RightShift,
            | PreprocessorTokenType::LessThan => PreprocessorExpressionOperator::LessThan,
            | PreprocessorTokenType::LessThanEquals =>
                PreprocessorExpressionOperator::LessThanEquals,
            | PreprocessorTokenType::GreaterThan => PreprocessorExpressionOperator::GreaterThan,
            | PreprocessorTokenType::GreaterThanEquals =>
                PreprocessorExpressionOperator::GreaterThanEquals,
            | PreprocessorTokenType::EqualsEquals => PreprocessorExpressionOperator::Equals,
            | PreprocessorTokenType::ExclamationMarkEquals =>
                PreprocessorExpressionOperator::NotEquals,
            | PreprocessorTokenType::Ampersand => PreprocessorExpressionOperator::BitwiseAnd,
            | PreprocessorTokenType::Caret => PreprocessorExpressionOperator::BitwiseXor,
            | PreprocessorTokenType::Pipe => PreprocessorExpressionOperator::BitwiseOr,
            | PreprocessorTokenType::AmpersandAmpersand =>
                PreprocessorExpressionOperator::LogicalAnd,
            | PreprocessorTokenType::PipePipe => PreprocessorExpressionOperator::LogicalOr,
            | PreprocessorTokenType::QuestionMark => PreprocessorExpressionOperator::QuestionMark,
            | PreprocessorTokenType::Comma => PreprocessorExpressionOperator::Comma,
            | PreprocessorTokenType::Tilde => PreprocessorExpressionOperator::BitwiseNot,
            | PreprocessorTokenType::ExclamationMark => PreprocessorExpressionOperator::LogicalNot,
            | _ => unreachable!(),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "This function is long because it contains the logic for evaluating an operator \
                  in a constant expression. I don't think splitting it up would anything clearer."
    )]
    fn handle_expression_operator(
        &mut self,
        context: &mut Context,
        op: PreprocessorExpressionOperator,
    ) {
        match op {
            | PreprocessorExpressionOperator::UnaryPlus => {
                if self.expression_parser.operand_stack.is_empty() {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryPlusWithoutOperand,
                        source_vectors,
                    });
                }
            },
            | PreprocessorExpressionOperator::UnaryMinus => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryMinusWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                let (new, did_overflow) = operand.as_signed().overflowing_neg();
                if operand.is_signed() && did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::UnaryMinusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser
                    .operand_stack
                    .push(operand.set_signed(new));
            },
            | PreprocessorExpressionOperator::BitwiseNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(operand.map_unsigned(|v| !v));
            },
            | PreprocessorExpressionOperator::LogicalNot => {
                let Some(operand) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalNotWithoutOperand,
                        source_vectors,
                    });
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        operand.as_signed() == 0,
                    )));
            },
            | PreprocessorExpressionOperator::BinaryPlus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BinaryPlus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryPlusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_add(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryPlusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::BinaryMinus => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BinaryMinus operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryMinusWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_sub(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BinaryMinusOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Multiply => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Multiply operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MultiplyWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = lhs.as_signed().overflowing_mul(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MultiplyOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Divide => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Divide operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideByZero,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = lhs.as_signed().overflowing_div(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::DivideOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::Modulo => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Modulo operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                if rhs.as_signed() == 0 {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloByZero,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(if is_unsigned {
                        PreprocessorExpressionOperand::Unsigned(0)
                    } else {
                        PreprocessorExpressionOperand::Signed(0)
                    });
                    return;
                }
                let (new, did_overflow) = lhs.as_signed().overflowing_rem(rhs.as_signed());
                if did_overflow && !is_unsigned {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::ModuloOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LeftShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LeftShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LeftShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = lhs.as_signed().overflowing_shl(rhs.as_unsigned() as u32);
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LeftShiftOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::RightShift => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: RightShift operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::RightShiftWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let (new, did_overflow) = {
                    let did_overflow = u32::try_from(rhs.as_unsigned()).is_err();
                    let (new, overflow) = lhs.as_signed().overflowing_shr(rhs.as_unsigned() as u32);
                    (new, did_overflow || overflow)
                };
                if did_overflow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::RightShiftOverflow,
                        source_vectors,
                    });
                }
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(new as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(new)
                });
            },
            | PreprocessorExpressionOperator::LessThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LessThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() < rhs.as_unsigned()
                } else {
                    lhs.as_signed() < rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::LessThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: LessThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LessThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() <= rhs.as_unsigned()
                } else {
                    lhs.as_signed() <= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThan => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: GreaterThan operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() > rhs.as_unsigned()
                } else {
                    lhs.as_signed() > rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::GreaterThanEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: GreaterThanEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::GreaterThanEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if is_unsigned {
                    lhs.as_unsigned() >= rhs.as_unsigned()
                } else {
                    lhs.as_signed() >= rhs.as_signed()
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(result)));
            },
            | PreprocessorExpressionOperator::Equals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: Equals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::EqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() == rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::NotEquals => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: NotEquals operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::NotEqualsWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser
                    .operand_stack
                    .push(PreprocessorExpressionOperand::Signed(i64::from(
                        lhs.as_signed() != rhs.as_signed(),
                    )));
            },
            | PreprocessorExpressionOperator::BitwiseAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() & rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseXor => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseXor operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseXorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() ^ rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::BitwiseOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop()
                    .expect("Compiler bug: BitwiseOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::BitwiseOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let is_unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = lhs.as_signed() | rhs.as_signed();
                self.expression_parser.operand_stack.push(if is_unsigned {
                    PreprocessorExpressionOperand::Unsigned(result as u64)
                } else {
                    PreprocessorExpressionOperand::Signed(result)
                });
            },
            | PreprocessorExpressionOperator::LogicalAnd => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop_isolated()
                    .expect("Compiler bug: LogicalAnd operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalAndWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let lhs_is_true = lhs.as_signed() != 0;
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                        PreprocessorExpressionOperand::Signed(i64::from(
                            lhs_is_true && rhs.as_signed() != 0,
                        )),
                        lhs.contains_evaluated_comma
                            || (lhs_is_true && rhs.contains_evaluated_comma),
                    ),
                );
            },
            | PreprocessorExpressionOperator::LogicalOr => {
                let rhs = self
                    .expression_parser
                    .operand_stack
                    .pop_isolated()
                    .expect("Compiler bug: LogicalOr operator without lhs");
                let Some(lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::LogicalOrWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                let lhs_is_true = lhs.as_signed() != 0;
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                        PreprocessorExpressionOperand::Signed(i64::from(
                            lhs_is_true || rhs.as_signed() != 0,
                        )),
                        lhs.contains_evaluated_comma
                            || (!lhs_is_true && rhs.contains_evaluated_comma),
                    ),
                );
            },
            | PreprocessorExpressionOperator::Comma => {
                let Some(rhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    return;
                };
                let Some(_lhs) = self.expression_parser.operand_stack.pop_isolated() else {
                    self.expression_parser.operand_stack.push(rhs);
                    return;
                };
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(rhs.value, true),
                );
            },
            | PreprocessorExpressionOperator::QuestionMark => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                    source_vectors,
                });
            },
            | PreprocessorExpressionOperator::Conditional => {
                let Some(final_operand) = self.expression_parser.operand_stack.pop_isolated()
                else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let Some(middle_operand) = self.expression_parser.operand_stack.pop_isolated()
                else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let Some(condition) = self.expression_parser.operand_stack.pop_isolated() else {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::TernaryOperatorWithoutRhs,
                        source_vectors,
                    });
                    self.expression_parser
                        .operand_stack
                        .push(PreprocessorExpressionOperand::Signed(0));
                    return;
                };
                let selected_operand = if condition.as_signed() != 0 {
                    middle_operand
                } else {
                    final_operand
                };
                self.expression_parser.operand_stack.push(
                    EvaluatedPreprocessorExpressionOperand::with_comma_liveness(
                        selected_operand.value,
                        condition.contains_evaluated_comma
                            || selected_operand.contains_evaluated_comma,
                    ),
                );
            },
            | PreprocessorExpressionOperator::OpeningParenthesis => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnterminatedOpeningParenthesisInPreprocessorExpression,
                    source_vectors,
                });
            },
        }
    }

    fn parse_defined_operator(&mut self, context: &mut Context) {
        let Some(ident_or_opening_paren) = self.expect_token_from_previous_phase::<true>(context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::Identifier | PreprocessorTokenType::OpeningParenthesis),
            |_, _, t|
                ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::MissingOpeningParenthesisOrIdentifierInDefinedDirective(t.kind),
                        source_vectors: t.source_vectors,
                    },
                )
            ,
            "parsing defined operator",
        ) else {
            // The malformed operator still stands for one operand, so the
            // expression continues in the binary state without cascading.
            self.skip_token_unless_line_end(context);
            self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };
        if ident_or_opening_paren.kind == PreprocessorTokenType::Identifier {
            let is_defined = self
                .macro_definitions
                .contains_key(&ident_or_opening_paren.contents);
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        }
        assert_eq!(
            ident_or_opening_paren.kind,
            PreprocessorTokenType::OpeningParenthesis,
            "Compiler bug: ident_or_opening_paren should be an opening parenthesis or identifier."
        );
        let Some(ident) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::Identifier),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingIdentifierInDefinedDirective(
                        t.kind,
                    ),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        ) else {
            self.skip_token_unless_line_end(context);
            self.expression_parser
                .operand_stack
                .push(PreprocessorExpressionOperand::Signed(0));
            self.expression_parser.state = PreprocessorExpressionParserState::Binary;
            return;
        };

        _ = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| matches!(t.kind, PreprocessorTokenType::ClosingParenthesis),
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::MissingClosingParenthesisInDefinedDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing defined operator",
        );
        let is_defined = self.macro_definitions.contains_key(&ident.contents);
        self.expression_parser
            .operand_stack
            .push(PreprocessorExpressionOperand::Signed(i64::from(is_defined)));
        self.expression_parser.state = PreprocessorExpressionParserState::Binary;
    }

    /// Consumes the next token of a directive after it was diagnosed, unless
    /// it ends the line.
    fn skip_token_unless_line_end(&mut self, context: &mut Context) {
        let position = self.position(context);
        match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
            | Some(token) if token.kind != PreprocessorTokenType::Newline => {},
            | _ => self.set_position(context, position),
        }
    }

    /// Pops to the innermost pending binary operator, if any. A missing one
    /// means the dangling operand was already diagnosed (for example a unary
    /// operator with no operand).
    fn last_binary_operator(&mut self) -> Option<PreprocessorExpressionOperator> {
        loop {
            match self.expression_parser.operator_stack.pop()? {
                | PreprocessorExpressionOperator::OpeningParenthesis
                | PreprocessorExpressionOperator::UnaryMinus
                | PreprocessorExpressionOperator::UnaryPlus
                | PreprocessorExpressionOperator::BitwiseNot
                | PreprocessorExpressionOperator::LogicalNot => continue,
                | op => return Some(op),
            }
        }
    }

    fn eval_preprocessor_expression(
        &mut self,
        context: &mut Context,
        on_no_expression_error: PreprocessorErrorType,
    ) -> bool {
        const UNARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Unary;
        const BINARY: PreprocessorExpressionParserState = PreprocessorExpressionParserState::Binary;
        self.expression_parser.reset();
        'main: loop {
            match self.next_preprocessor_token::<true>(context) {
                | None => {
                    let source_vectors = context.create_source_vectors(self.position(context), self.source_file_index(), 0);
                    context.preprocessor_error(
                        PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                            source_vectors,
                        });
                    break 'main;
                },
                | Some(token) => match (token.kind, self.expression_parser.state) {
                    | (PreprocessorTokenType::Newline, _) =>
                        break 'main,
                    (PreprocessorTokenType::Plus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryPlus),
                    (PreprocessorTokenType::Minus, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::UnaryMinus),
                    (PreprocessorTokenType::Tilde, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::BitwiseNot),
                    (PreprocessorTokenType::Tilde, BINARY) =>
                        context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TildeInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                ,
                    (PreprocessorTokenType::ExclamationMark, UNARY) =>
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::LogicalNot),
                    (PreprocessorTokenType::ExclamationMark, BINARY) =>
                        context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExclamationMarkInsteadOfBinaryOperatorInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    (PreprocessorTokenType::OpeningParenthesis, UNARY) => {
                        self.expression_parser.operator_stack.push(PreprocessorExpressionOperator::OpeningParenthesis);
                    }
                    (PreprocessorTokenType::OpeningParenthesis, BINARY) => {
                        context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::FunctionCallOperatorNotSupportedInPreprocessorExpression,
                                source_vectors: token.source_vectors,
                            },
                        );
                        let mut paren_depth = 1;
                        // Step over function call.
                        while paren_depth > 0 {
                            match self.next_preprocessor_token::<true>(context) {
                                | None => {
                                    let source_vectors = context.create_source_vectors(self.position(context), self.source_file_index(), 0);
                                    context.preprocessor_error(PreprocessorError {
                                            error_type: PreprocessorErrorType::UnexpectedEndOfInput("parsing preprocessor expression"),
                                            source_vectors,
                                        },
                                    );
                                    break 'main;
                                },
                                | Some(token) => match token.kind {
                                    | PreprocessorTokenType::OpeningParenthesis => paren_depth += 1,
                                    | PreprocessorTokenType::ClosingParenthesis => paren_depth -= 1,
                                    | _ => (),
                                },
                            }
                        }
                    }
                    (PreprocessorTokenType::ClosingParenthesis, _) => {
                        self.expression_parser.state = BINARY;
                        if !self.expression_parser.operator_stack.last().is_some_and(|op| *op != PreprocessorExpressionOperator::OpeningParenthesis) {
                            context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::EmptyParenthesesInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                },
                            );
                            continue 'main;
                        }
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op == PreprocessorExpressionOperator::OpeningParenthesis {
                                break;
                            }
                            self.handle_expression_operator(context, op);
                        }
                    }
                    (PreprocessorTokenType::Defined, UNARY) =>
                        self.parse_defined_operator(context),
                    (PreprocessorTokenType::Defined, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DefinedOperatorInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Asterisk, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::DereferenceOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Ampersand, UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::AddressOfOperatorNotSupportedInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Colon, state) => {
                        if state == UNARY {
                            context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::TernaryOperatorWithoutMhs,
                                source_vectors: token.source_vectors,
                            });
                        }
                        let mut matched_question_mark = false;
                        while let Some(mut entry) =
                            last_entry(&mut self.expression_parser.operator_stack)
                        {
                            match *entry {
                                | PreprocessorExpressionOperator::QuestionMark => {
                                    _ = entry.insert(PreprocessorExpressionOperator::Conditional);
                                    matched_question_mark = true;
                                    break;
                                },
                                | PreprocessorExpressionOperator::OpeningParenthesis => break,
                                | _ => {
                                    let operator = entry.remove();
                                    self.handle_expression_operator(context, operator);
                                },
                            }
                        }
                        if matched_question_mark {
                            self.expression_parser.state = UNARY;
                        } else {
                            context.preprocessor_error(PreprocessorError {
                                error_type: PreprocessorErrorType::ColonWithoutMatchingQuestionMark,
                                source_vectors: token.source_vectors,
                            });
                        }
                    },
                    (
                        | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                        PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                        PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals |
                        PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                        PreprocessorTokenType::Comma,
                        UNARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::BinaryOperatorInsteadOfUnaryExpressionInPreprocessorExpression(self.map_operator(context, token)),
                            source_vectors: token.source_vectors,
                        }),
                    | (PreprocessorTokenType::Plus | PreprocessorTokenType::Minus | PreprocessorTokenType::Asterisk
                    | PreprocessorTokenType::ForwardSlash | PreprocessorTokenType::Percent | PreprocessorTokenType::LessThanLessThan |
                    PreprocessorTokenType::GreaterThanGreaterThan | PreprocessorTokenType::LessThan | PreprocessorTokenType::LessThanEquals | PreprocessorTokenType::GreaterThan |
                    PreprocessorTokenType::GreaterThanEquals | PreprocessorTokenType::EqualsEquals | PreprocessorTokenType::ExclamationMarkEquals | PreprocessorTokenType::Ampersand |
                    PreprocessorTokenType::Caret | PreprocessorTokenType::Pipe | PreprocessorTokenType::AmpersandAmpersand | PreprocessorTokenType::PipePipe | PreprocessorTokenType::QuestionMark |
                    PreprocessorTokenType::Comma,
                    BINARY) => {
                        let token_op = self.map_operator(context, token);
                        while let Some(op) = self.expression_parser.operator_stack.pop() {
                            if op == PreprocessorExpressionOperator::QuestionMark
                                && token_op == PreprocessorExpressionOperator::Comma
                            {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            } else if op.has_precedence_over(token_op) {
                                self.handle_expression_operator(context, op);
                            } else {
                                self.expression_parser.operator_stack.push(op);
                                break;
                            }
                        }
                        self.expression_parser.operator_stack.push(token_op);
                        self.expression_parser.state = UNARY;
                    },
                    (PreprocessorTokenType::Number, UNARY) => {
                        match self.parse_number(context, token,).kind {
                            | TokenType::Float(_) => {
                                // C99 §6.10.1p1 admits only integer constant
                                // expressions. Recover with a zero operand
                                // so evaluation continues without cascading.
                                context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::FloatInsteadOfIntegerInPreprocessorExpression,
                                    source_vectors: token.source_vectors,
                                });
                                self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                            },
                            | TokenType::Integer(v) => match v {
                                | IntegerTokenType::Int(i) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(i64::from(i))),
                                | IntegerTokenType::Long(l) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(l)),
                                | IntegerTokenType::LongLong(ll) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(ll)),
                                | IntegerTokenType::UnsignedInt(ui) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(u64::from(ui))),
                                | IntegerTokenType::UnsignedLong(ul) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ul)),
                                | IntegerTokenType::UnsignedLongLong(ull) => self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Unsigned(ull)),
                            },
                            _ => unreachable!("Compiler bug: parse_number should return a number token."),
                        }
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Number, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::NumberInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Identifier, UNARY) => {
                        context.preprocessor_error(PreprocessorError {
                                    error_type: PreprocessorErrorType::UndefinedIdentifierInPreprocessorExpression(context.string_cache.at(token.contents).to_owned()),
                                    source_vectors: token.source_vectors,
                                },
                        );
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(0));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Identifier, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::IdentifierInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    (PreprocessorTokenType::Character, UNARY) => {
                        let value = i64::from(self.parse_character(context, token));
                        self.expression_parser.operand_stack.push(PreprocessorExpressionOperand::Signed(value));
                        self.expression_parser.state = BINARY;
                    },
                    (PreprocessorTokenType::Character, BINARY) => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::CharacterInsteadOfBinaryOperatorInPreprocessorExpression,
                            source_vectors: token.source_vectors,
                        },
                    ),
                    _ => context.preprocessor_error(PreprocessorError {
                            error_type: PreprocessorErrorType::UnexpectedTokenInPreprocessorExpression(token.kind),
                            source_vectors: token.source_vectors,
                        },
                    ),
                },
            }
        }
        if self.expression_parser.state == UNARY
            && let Some(operator) = self.last_binary_operator()
        {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_index(), 0);
            context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedRightHandSideOfBinaryOperatorInPreprocessorExpression(
                        operator,
                    ),
                    source_vectors
                }
            );
        }
        while let Some(op) = self.expression_parser.operator_stack.pop() {
            self.handle_expression_operator(context, op);
        }
        match self.expression_parser.operand_stack.len() {
            | 1 => {
                let operand = self.expression_parser.operand_stack.pop().unwrap();
                let extension_policy = match context.configuration.standard() {
                    | CStandard::C99 => context.configuration.extension_policy(),
                };
                if operand.contains_evaluated_comma && extension_policy != ExtensionPolicy::Allow {
                    let source_vectors = context.create_source_vectors(
                        self.position(context),
                        self.source_file_index(),
                        0,
                    );
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                            extension_policy,
                        ),
                        source_vectors,
                    });
                }
                operand.as_signed() != 0
            },
            | 0 => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type: on_no_expression_error,
                    source_vectors,
                });
                true
            },
            | _ => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::ExpectedBinaryOperatorInPreprocessorExpression,
                    source_vectors,
                });
                true
            },
        }
    }

    /// Skips the lines of a conditional group that is not being translated
    /// (C99 §6.10.1p6). Only conditional directives are recognized inside
    /// skipped lines; every other line, including malformed directives, is
    /// ignored.
    ///
    /// `at_line_start` says whether the directive that started the skip has
    /// already consumed its terminating newline.
    fn skip_over_dead_code(
        &mut self,
        context: &mut Context,
        mut at_line_start: bool,
        mode: SkipMode,
    ) {
        let depth = self.open_conditionals.len();
        context.set_ignore_tokenizer_errors(true);
        'lines: while depth > 0 && self.open_conditionals.len() >= depth {
            if !at_line_start {
                loop {
                    match self.tokenizer.next_item(context) {
                        | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                        | Some(_) => {},
                        | None => break 'lines,
                    }
                }
            }
            at_line_start = false;
            let Some(first) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                break 'lines;
            };
            match first.kind {
                | PreprocessorTokenType::Newline => {
                    at_line_start = true;
                    continue 'lines;
                },
                | PreprocessorTokenType::Hash => {},
                | _ => continue 'lines,
            }
            let Some(name) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                break 'lines;
            };
            match name.kind {
                | PreprocessorTokenType::Newline => {
                    at_line_start = true;
                    continue 'lines;
                },
                | PreprocessorTokenType::Identifier => {},
                | _ => continue 'lines,
            }
            let innermost = self.open_conditionals.len() == depth;
            match context.string_cache.at(name.contents) {
                | "if" | "ifdef" | "ifndef" => self
                    .open_conditionals
                    .push(context.get_source_vectors(name.source_vectors).into()),
                | "endif" => {
                    drop(self.open_conditionals.pop());
                },
                | "elif" if innermost && mode == SkipMode::FalseGroup => {
                    context.set_ignore_tokenizer_errors(false);
                    let taken = self.eval_preprocessor_expression(
                        context,
                        PreprocessorErrorType::NoConditionInElifDirective,
                    );
                    if taken {
                        self.last_was_newline = true;
                        self.current_is_newline = true;
                        return;
                    }
                    context.set_ignore_tokenizer_errors(true);
                    at_line_start = true;
                },
                | "else" if innermost && mode == SkipMode::FalseGroup => break 'lines,
                | _ => {},
            }
        }
        context.set_ignore_tokenizer_errors(false);
        if !at_line_start {
            self.skip_until_newline(context);
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_if_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.open_conditionals
            .push(context.get_source_vectors(directive.source_vectors).into());
        if self
            .eval_preprocessor_expression(context, PreprocessorErrorType::NoConditionInIfDirective)
        {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context, true, SkipMode::FalseGroup);
        }
    }

    /// `#elif` and `#else` reached while translating a group end that group:
    /// the rest of the conditional is skipped through its `#endif`.
    fn parse_elif_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.skip_remaining_groups(
            context,
            directive,
            PreprocessorErrorType::ElifDirectiveWithoutIfDirective,
        );
    }

    fn parse_else_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.skip_remaining_groups(
            context,
            directive,
            PreprocessorErrorType::ElseDirectiveWithoutIfDirective,
        );
    }

    fn skip_remaining_groups(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
        unmatched_error: PreprocessorErrorType,
    ) {
        if self.open_conditionals.is_empty() {
            context.preprocessor_error(PreprocessorError {
                error_type:     unmatched_error,
                source_vectors: directive.source_vectors,
            });
            self.skip_until_newline(context);
            return;
        }
        self.skip_over_dead_code(context, false, SkipMode::ToEndif);
    }

    fn parse_endif_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        if self.open_conditionals.pop().is_none() {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MoreEndifDirectivesThanIfDirectives,
                source_vectors: directive.source_vectors,
            });
        }
        self.skip_until_newline(context);
    }

    fn parse_ifdef_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.parse_macro_test_directive(context, directive, true);
    }

    fn parse_ifndef_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        self.parse_macro_test_directive(context, directive, false);
    }

    /// Handles `#ifdef` (`wants_defined`) and `#ifndef`. A missing macro name
    /// is diagnosed and the group is skipped, as GCC and Clang do.
    fn parse_macro_test_directive(
        &mut self,
        context: &mut Context,
        directive: PreprocessorToken,
        wants_defined: bool,
    ) {
        self.open_conditionals
            .push(context.get_source_vectors(directive.source_vectors).into());
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     if wants_defined {
                        PreprocessorErrorType::ExpectedIdentifierInIfdefDirective(token.kind)
                    } else {
                        PreprocessorErrorType::ExpectedIdentifierInIfndefDirective(token.kind)
                    },
                    source_vectors: token.source_vectors,
                })
            },
            if wants_defined {
                "parsing ifdef directive"
            } else {
                "parsing ifndef directive"
            },
        ) else {
            self.skip_over_dead_code(context, false, SkipMode::FalseGroup);
            return;
        };
        if self
            .expect_token_from_previous_phase::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     if wants_defined {
                            PreprocessorErrorType::ExtraTokensAfterIfdefDirective
                        } else {
                            PreprocessorErrorType::ExtraTokensAfterIfndefDirective
                        },
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing conditional directive",
            )
            .is_none()
        {
            self.skip_until_newline(context);
        }
        if self.macro_definitions.contains_key(&name.contents) == wants_defined {
            self.last_was_newline = true;
            self.current_is_newline = true;
        } else {
            self.skip_over_dead_code(context, true, SkipMode::FalseGroup);
        }
    }

    /// Resolves an include name the way GCC and Clang do (C99 §6.10.2p2-3
    /// leave the places implementation-defined):
    ///
    /// 1. `"name"` first looks beside the file containing the directive (for
    ///    `--input`, whose name has no directory, that is the working
    ///    directory), then in each `--iquote` directory.
    /// 2. Both forms then search each `--isystem` directory, `CPATH`, and
    ///    `C_INCLUDE_PATH`.
    ///
    /// The process working directory is never searched implicitly, so the
    /// result depends on the source tree rather than where the compiler runs.
    ///
    /// `including_file` is the file containing the directive, captured before
    /// a macro-expanded operand can switch to its definition's tokenizer.
    fn find_header_from_path(
        &mut self,
        context: &mut Context,
        including_file: u32,
        include_token: PreprocessorToken,
        path: &Path,
        is_system_header: bool,
    ) -> Option<u32> {
        let mut searched = Vec::new();
        let header = if path.is_absolute() {
            path.is_file().then(|| path.to_owned())
        } else {
            let mut candidates = Vec::new();
            if !is_system_header {
                let including_file = context.get_source_file(including_file);
                candidates.push(
                    including_file
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                );
                candidates.extend(self.quote_include_directories.iter().cloned());
            }
            candidates.extend(self.system_include_directories.iter().cloned());
            let mut found = None;
            for directory in candidates {
                let candidate = directory.join(path);
                if candidate.is_file() {
                    found = Some(candidate);
                    break;
                }
                searched.push(directory);
            }
            found
        };
        let Some(header) = header else {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderNotFound {
                    name: path.to_string_lossy().into_owned(),
                    is_system_header,
                    searched,
                },
                source_vectors: include_token.source_vectors,
            });
            return None;
        };
        let source_file_index = context.intern_source_file(header.into_boxed_path());
        if self.once_set.contains(&source_file_index) {
            None
        } else {
            Some(source_file_index)
        }
    }

    fn parse_include_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        let including_file = self.source_file_index();
        context.set_is_tokenizing_include_string(true);
        let include_string =
            self.expect_token_without_rewind::<true>(
                context,
                |_, context, token| match token.kind {
                    | PreprocessorTokenType::AngleBracketString
                    | PreprocessorTokenType::IncludeString => true,
                    | _ if context.string_cache.at(token.contents).starts_with('<') => true,
                    | _ => false,
                },
                |_, _, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing include directive",
            );
        context.set_is_tokenizing_include_string(false);
        let Some(include_string) = include_string else {
            if !self.current_is_newline {
                self.skip_and_expand_until_newline(context);
            }
            return;
        };
        let header_source_index = match include_string.kind {
            | PreprocessorTokenType::IncludeString => {
                let contents = context
                    .string_cache
                    .at(include_string.contents)
                    .strip_circumfix('"', '"')
                    .expect("Include strings must be enclosed in double quotes.")
                    .to_token_string();
                let path = Path::new(contents.as_str());
                if self
                    .expect_token_from_previous_phase::<true>(
                        context,
                        |_, _, t| t.kind == PreprocessorTokenType::Newline,
                        |_, _, t| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                                source_vectors: t.source_vectors,
                            })
                        },
                        "parsing include directive.",
                    )
                    .is_none()
                {
                    self.skip_until_newline(context);
                }

                self.find_header_from_path(context, including_file, include_string, path, false)
            },
            | PreprocessorTokenType::AngleBracketString => {
                let contents = context
                    .string_cache
                    .at(include_string.contents)
                    .strip_circumfix('<', '>')
                    .expect("Angle-bracket strings must be enclosed in angle brackets.")
                    .to_token_string();
                let path = Path::new(contents.as_str());
                if self
                    .expect_token_from_previous_phase::<true>(
                        context,
                        |_, _, t| t.kind == PreprocessorTokenType::Newline,
                        |_, _, t| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                                source_vectors: t.source_vectors,
                            })
                        },
                        "parsing include directive.",
                    )
                    .is_none()
                {
                    self.skip_until_newline(context);
                }
                self.find_header_from_path(context, including_file, include_string, path, true)
            },
            | _ => {
                let mut contents = TokenString::new();
                contents.push_str(&context.string_cache.at(include_string.contents)[1..]);
                let start_index = Context::duplicate_source_vectors(
                    &mut context.source_vectors.0,
                    include_string.source_vectors,
                );
                loop {
                    match self.next_preprocessor_token::<false>(context) {
                        | Some(token) => {
                            if token.kind == PreprocessorTokenType::Newline {
                                break;
                            }
                            let token_contents = context.string_cache.at(token.contents);
                            _ = Context::duplicate_source_vectors(
                                &mut context.source_vectors.0,
                                token.source_vectors,
                            );
                            if let Some(idx) = token_contents.find('>') {
                                contents.push_str(&token_contents[..idx]);
                                break;
                            }
                            contents.push_str(context.string_cache.at(token.contents));
                        },
                        | None => {
                            context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                    "parsing include directive",
                                ),
                                source_vectors: directive.source_vectors,
                            });
                            break;
                        },
                    }
                }
                let path = Path::new(contents.as_str());
                let synthetic_token = PreprocessorToken {
                    source_vectors: SourceVectors::new(
                        start_index,
                        context.source_vectors.0.len() as u32,
                    ),
                    contents:       context.string_cache.intern(&contents),
                    kind:           PreprocessorTokenType::AngleBracketString,
                };
                self.find_header_from_path(context, including_file, synthetic_token, path, true)
            },
        };
        let Some(header_source_index) = header_source_index else {
            return;
        };
        let header_path = context.get_source_file(header_source_index);
        let Ok(header_string) = read_to_string_lossy(header_path).map_err(|e| {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderFileInaccessible(e),
                source_vectors: directive.source_vectors,
            });
        }) else {
            return;
        };
        let header_string = SharedString::from(header_string);
        context.record_source_text(header_source_index, header_string.clone());
        self.push_tokenizer_frame(
            context,
            TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile,
                tokenizer:  PreprocessorTokenizer::new(header_source_index, header_string),
            },
        );
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_define_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing define directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        let old_definition = self.macro_definitions.get(&name.contents).cloned();
        let tokenizer = self.tokenizer.clone();
        let old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                | MacroDefinition::BuiltIn => {
                    // C99 §6.10.8p4: predefined macro names cannot be
                    // redefined, so the built-in definition stays in effect.
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                    self.skip_until_newline(context);
                    return;
                },
            },
        };
        // A function-like definition requires '(' immediately after its name.
        // The probe may consume the directive's newline when the replacement
        // list is empty; remember that so the next line is not skipped too.
        let mut line_ended = false;
        let opening_paren = match self.tokenizer.next_item(context) {
            | Some(
                token @ PreprocessorToken {
                    kind: PreprocessorTokenType::OpeningParenthesis,
                    ..
                },
            ) => Some(token),
            | Some(token) => {
                line_ended = token.kind == PreprocessorTokenType::Newline;
                None
            },
            | None => {
                line_ended = true;
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing macro definition",
                    ),
                    source_vectors: name.source_vectors,
                });
                None
            },
        };
        if opening_paren.is_some() {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                | MacroDefinition::FunctionLike { .. } => false,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            let mut argument_names = Vec::new();
            let mut is_variadic = false;
            loop {
                let Some(name_or_ellipsis) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| {
                        t.kind == PreprocessorTokenType::Identifier
                            || t.kind == PreprocessorTokenType::Ellipsis
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
                    },
                    |_, _, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(
                                    token.kind,
                                ),
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing macro definition",
                ) else {
                    break;
                };
                if is_variadic {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind == PreprocessorTokenType::Identifier {
                    argument_names.push(name_or_ellipsis.contents);
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis,
                    |_, _, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            let tokenizer = self.tokenizer.clone();
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::FunctionLike {
                    tokenizer,
                    argument_names: argument_names.into(),
                    is_variadic,
                },
            ));
        } else {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => false,
                | MacroDefinition::FunctionLike { .. } => true,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            drop(self.macro_definitions.insert(
                name.contents,
                MacroDefinition::ObjectLike {
                    tokenizer: tokenizer.clone(),
                },
            ));
        }
        let mut last = Option::<PreprocessorToken>::None;
        if let Some(mut old_tokenizer) = old_tokenizer {
            // Compare replacement lists body to body, and parameter lists
            // separately (C99 §6.10.3p2).
            let (mut new_tokenizer, parameters_match) =
                match (self.macro_definitions.get(&name.contents), &old_definition) {
                    | (
                        Some(MacroDefinition::FunctionLike {
                            tokenizer,
                            argument_names,
                            is_variadic,
                        }),
                        Some(MacroDefinition::FunctionLike {
                            argument_names: old_argument_names,
                            is_variadic: old_is_variadic,
                            ..
                        }),
                    ) => (
                        tokenizer.clone(),
                        argument_names == old_argument_names && is_variadic == old_is_variadic,
                    ),
                    | (
                        Some(
                            MacroDefinition::FunctionLike { tokenizer, .. }
                            | MacroDefinition::ObjectLike { tokenizer },
                        ),
                        _,
                    ) => (tokenizer.clone(), true),
                    | _ => (tokenizer, true),
                };
            let mut error_has_been_generated = false;
            if !parameters_match {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                        context.string_cache.at(name.contents).to_owned(),
                    ),
                    source_vectors: name.source_vectors,
                });
                error_has_been_generated = true;
            }
            loop {
                let old_next = old_tokenizer.next_item(context);
                let new_next = new_tokenizer.next_item(context);
                if let Some(t) = new_next.as_ref()
                    && t.kind == PreprocessorTokenType::HashHash
                    && last.is_none()
                {
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                        source_vectors: t.source_vectors,
                    });
                }
                if !same_replacement_token(context, old_next.as_ref(), new_next.as_ref())
                    && !error_has_been_generated
                {
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                context.string_cache.at(name.contents).to_owned(),
                            ),
                        source_vectors: name.source_vectors,
                    });
                    error_has_been_generated = true;
                }
                if !new_next
                    .as_ref()
                    .is_some_and(|t| t.kind != PreprocessorTokenType::Newline)
                {
                    if last
                        .as_ref()
                        .is_some_and(|t| t.kind == PreprocessorTokenType::HashHash)
                    {
                        context.preprocessor_error(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                            source_vectors: last.unwrap().source_vectors,
                        });
                    }
                    break;
                }
                last = new_next;
            }
        }
        if !line_ended {
            loop {
                match self.tokenizer.next_item(context) {
                    | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(_) => continue,
                }
            }
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_undef_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Identifier,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        drop(self.macro_definitions.remove(&name.contents));
        _ = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Newline,
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        );
    }

    fn parse_line_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(token) = self.expect_token::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Number,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        ) else {
            self.skip_and_expand_until_newline(context);
            return;
        };
        let digits = context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0');
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                source_vectors: token.source_vectors,
            });
            self.skip_and_expand_until_newline(context);
            return;
        }
        let (value, did_overflow) = {
            let mut value = 0i128;
            let mut did_overflow = false;
            for b in digits.bytes() {
                value *= 10;
                if value > i128::from(i32::MAX) {
                    did_overflow = true;
                }
                value += i128::from(b - b'0');
                if value > i128::from(i32::MAX) {
                    did_overflow = true;
                }
            }
            (value, did_overflow)
        };
        if did_overflow {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(value),
                source_vectors: token.source_vectors,
            });
        } else {
            self.set_line(context, value as u32);
        }
        let name = self.expect_token::<true>(
            context,
            |_, _, t| {
                matches!(
                    t.kind,
                    PreprocessorTokenType::String | PreprocessorTokenType::Newline
                )
            },
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        );
        if let Some(
            t @ PreprocessorToken {
                kind: PreprocessorTokenType::String,
                ..
            },
        ) = name
        {
            let path_box = PathBuf::from(context.string_cache.at(t.contents)).into_boxed_path();
            let source_file_index = context.intern_source_file(path_box);
            self.set_source_file_index(context, source_file_index);
            _ = self.expect_token::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, t| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(
                            t.kind,
                        ),
                        source_vectors: t.source_vectors,
                    })
                },
                "parsing line directive",
            );
        }
    }

    fn parse_error_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        let mut contents = String::new();
        // A directive ending at end of file is complete; the missing final
        // newline is diagnosed on its own.
        while let Some(token) = self.tokenizer.next_item(context) {
            if token.kind == PreprocessorTokenType::Newline {
                break;
            }
            contents.push_str(
                context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0'),
            );
        }
        context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::ErrorDirective(contents),
            source_vectors: directive.source_vectors,
        });
    }

    fn parse_pragma_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        'base: loop {
            let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing pragma directive",
                    ),
                    source_vectors,
                });
                break 'base;
            };
            match token.kind {
                | PreprocessorTokenType::Whitespace => continue 'base,
                | PreprocessorTokenType::Newline => break 'base,
                | PreprocessorTokenType::Identifier => {
                    match context.string_cache.at(token.contents) {
                        | "once" => {
                            if !self.current_is_header(context) {
                                context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::PragmaOnceInNonHeader,
                                    source_vectors: token.source_vectors,
                                });
                            }
                            _ = self.once_set.insert(self.source_file_index());
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline =>
                                    break 'base,
                                | Some(t) => {
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                t.kind,
                                            ),
                                        source_vectors: token.source_vectors,
                                    });
                                    break 'base;
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_index(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | "STDC" => {
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if token.kind != PreprocessorTokenType::Identifier
                                        || !matches!(
                                            s,
                                            "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE"
                                        )
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                    s.to_owned(),
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_index(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }

                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if token.kind != PreprocessorTokenType::Identifier
                                        || !matches!(s, "ON" | "OFF" | "DEFAULT")
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(s.to_owned()),
                                                    source_vectors: token.source_vectors,
                                                },
                                            );
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = context.create_source_vectors(
                                        self.position(context),
                                        self.source_file_index(),
                                        0,
                                    );
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | _ => break 'base,
                    }
                },
                | _ => {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                        source_vectors: token.source_vectors,
                    });
                    break 'base;
                },
            }
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
}

impl TranslationPhase for Preprocessor {
    type Item = Token;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        let token = self.next_parser_token(context)?;
        Some(self.concatenate_adjacent_strings(context, token))
    }
}
