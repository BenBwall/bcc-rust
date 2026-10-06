//! `#include` operands read from ordinary preprocessing tokens: header names
//! taken from the source text, the C99 §6.4.7p3 sequences, missing closing
//! delimiters, and the backslash extension.

use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use super::{
    Context,
    Preprocessor,
    PreprocessorError,
    PreprocessorErrorType,
    SharedVec,
    TokenType,
    TranslationError,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        ErrorSeverity,
        GetSeverity,
        GetSourceVectors,
        SourceVector,
    },
};

/// A temporary directory holding a main file's headers.
struct Headers(PathBuf);

impl Headers {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "bcc-header-names-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        drop(std::fs::remove_dir_all(&directory));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    /// Writes a header that defines `name` as an identifier, at `path`
    /// relative to the directory.
    fn write(&self, path: &Path, name: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{name}\n")).unwrap();
    }
}

impl Drop for Headers {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// The header `"dir\file.h"` names: a file in `dir` where backslash separates
/// directories, otherwise a file whose name contains the backslash.
fn backslash_header() -> PathBuf {
    if cfg!(windows) {
        Path::new("dir").join("file.h")
    } else {
        PathBuf::from(r"dir\file.h")
    }
}

struct Outcome<'a, 'tu> {
    identifiers: Vec<String>,
    errors:      Vec<TranslationError<'tu>>,
    context:     &'a mut Context<'tu>,
}

impl Outcome<'_, '_> {
    fn preprocessor_errors(&self) -> Vec<&PreprocessorErrorType<'_>> {
        self.errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Preprocessing(PreprocessorError { error_type, .. }) =>
                    Some(error_type),
                | _ => None,
            })
            .collect()
    }

    /// Where the first error matching `predicate` points.
    fn location(&mut self, predicate: impl Fn(&PreprocessorErrorType<'_>) -> bool) -> SourceVector {
        let error = self
            .errors
            .iter()
            .find(|error| {
                matches!(error, TranslationError::Preprocessing(PreprocessorError { error_type, .. }) if predicate(error_type))
            })
            .expect("the error was reported");
        let source = error.source_vectors(self.context);
        self.context.get_source_vectors(source)[0].clone()
    }
}

fn with_preprocess<R>(
    headers: &Headers,
    source: &str,
    inspect: impl FnOnce(Outcome<'_, '_>) -> R,
) -> R {
    with_preprocess_in(headers, source, ExtensionPolicy::Allow, inspect)
}

fn with_preprocess_in<R>(
    headers: &Headers,
    source: &str,
    policy: ExtensionPolicy,
    inspect: impl FnOnce(Outcome<'_, '_>) -> R,
) -> R {
    with_preprocess_directories(headers, source, policy, SharedVec::default(), inspect)
}

fn with_preprocess_directories<R>(
    headers: &Headers,
    source: &str,
    policy: ExtensionPolicy,
    system_directories: SharedVec<PathBuf>,
    inspect: impl FnOnce(Outcome<'_, '_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context =
        Context::with_configuration(&tu, CompilerConfiguration::new(CStandard::C99, policy));
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        headers.0.join("main.c").into_boxed_path(),
        source,
        SharedVec::default(),
        system_directories,
    );
    let mut identifiers = Vec::new();
    preprocessor.for_each_item(&mut context, |context, token| {
        if token.kind == TokenType::Identifier {
            identifiers.push(context.string_cache.at(token.contents).to_owned());
        }
    });
    let errors = context.take_pending_errors();
    inspect(Outcome {
        identifiers,
        errors,
        context: &mut context,
    })
}

#[test]
fn written_header_names_keep_their_source_text() {
    let headers = Headers::new();
    headers.write(Path::new("a b.h"), "spaced");
    headers.write(Path::new("plain.h"), "plain");
    for (source, expected_header) in [
        ("#include \"a b.h\"\nafter\n", "spaced"),
        ("#include <a b.h>\nafter\n", "spaced"),
        ("#  include<a b.h>\nafter\n", "spaced"),
        ("#include \"plain.h\"\nafter\n", "plain"),
        ("#include <pl\\\nain.h>\nafter\n", "plain"),
        ("??=include \"plain.h\"\nafter\n", "plain"),
    ] {
        let directory = SharedVec::from(vec![headers.0.clone()]);
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let preprocess_arena = crate::util::bump::Bump::new();
        let mut preprocessor = Preprocessor::new(
            &preprocess_arena,
            &mut context,
            headers.0.join("main.c").into_boxed_path(),
            source,
            SharedVec::default(),
            directory,
        );
        let mut identifiers = Vec::new();
        preprocessor.for_each_item(&mut context, |context, token| {
            identifiers.push(context.string_cache.at(token.contents).to_owned());
        });
        let errors = context.take_pending_errors();
        assert!(errors.is_empty(), "{source:?}: {errors:#?}");
        assert_eq!(identifiers.len(), 2, "{source:?}: {identifiers:?}");
        assert_eq!(identifiers[0], expected_header, "{source:?}");
        assert_eq!(identifiers[1], "after", "{source:?}");
    }
}

#[test]
fn characters_glued_to_angle_header_closing_are_extra_tokens() {
    let headers = Headers::new();
    headers.write(Path::new("plain.h"), "plain");
    let preprocess_header = |source: &str, inspect: &mut dyn FnMut(Outcome<'_, '_>)| {
        with_preprocess_directories(
            &headers,
            source,
            ExtensionPolicy::Allow,
            SharedVec::from(vec![headers.0.clone()]),
            inspect,
        );
    };
    for tail in [">", "=", "> extra"] {
        let source = format!("#include <plain.h>{tail}\nafter\n");
        preprocess_header(&source, &mut |mut outcome| {
            assert_eq!(outcome.identifiers, ["plain", "after"], "{source:?}");
            assert!(
                matches!(
                    outcome.preprocessor_errors().as_slice(),
                    [PreprocessorErrorType::ExtraTokensAfterIncludeDirective]
                ),
                "{source:?}: {:#?}",
                outcome.errors
            );
            let location = outcome.location(|error| {
                matches!(
                    error,
                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective
                )
            });
            assert_eq!(location.line, 1, "{source:?}");
            assert_eq!(location.column, 19, "{source:?}");
            assert_eq!(location.length, 1, "{source:?}");
        });
    }

    std::fs::write(headers.0.join("middle.h"), "#include <plain.h>>").unwrap();
    preprocess_header("#include \"middle.h\"\nafter\n", &mut |outcome| {
        assert_eq!(outcome.identifiers, ["plain", "after"]);
        assert_eq!(
            outcome
                .preprocessor_errors()
                .iter()
                .filter(|error| matches!(
                    error,
                    PreprocessorErrorType::ExtraTokensAfterIncludeDirective
                ))
                .count(),
            1,
            "{:#?}",
            outcome.errors
        );
    });
}

#[test]
fn macro_operands_combine_token_spellings() {
    let headers = Headers::new();
    headers.write(Path::new("plain.h"), "plain");
    with_preprocess(
        &headers,
        "#define Q \"plain.h\"\n#include Q\n#define NAME plain\n#define S(x) #x\n#include \
         S(plain.h)\n",
        |outcome| {
            assert!(outcome.errors.is_empty(), "{:#?}", outcome.errors);
            assert_eq!(outcome.identifiers, ["plain", "plain"]);
        },
    );
}

#[test]
fn skipped_include_lines_are_ordinary_tokens() {
    let headers = Headers::new();
    for source in [
        "#if 0\n#include <it's.h>\n#include \"unterminated\n#include <a//b.h>\n#endif\nafter\n",
        "#ifdef NOPE\n#include <a\\b.h>\n#else\nafter\n#endif\n",
    ] {
        with_preprocess(&headers, source, |outcome| {
            assert!(
                outcome.errors.is_empty(),
                "{source:?}: {:#?}",
                outcome.errors
            );
            assert_eq!(outcome.identifiers, ["after"], "{source:?}");
        });
    }
}

#[test]
fn undefined_sequences_are_reported_once_at_their_position() {
    let headers = Headers::new();
    for (source, sequence, column) in [
        ("#include <it's.h>\n", "'", 13),
        ("#include \"it's.h\"\n", "'", 13),
        ("#include <a\\b.h>\n", "\\", 12),
        ("#include <a\"b.h>\n", "\"", 12),
        ("#include <a//b.h>\n", "//", 12),
        ("#include \"a//b.h\"\n", "//", 12),
        ("#include <a/*b.h>\n", "/*", 12),
        ("#include \"a/*b.h\"\n", "/*", 12),
        ("#include <x??/y.h>\n", "\\", 12),
    ] {
        with_preprocess(&headers, source, |mut outcome| {
            let invalid: Vec<_> = outcome
                .preprocessor_errors()
                .into_iter()
                .filter_map(|error| match error {
                    | PreprocessorErrorType::InvalidCharacterInHeaderName(found) => Some(*found),
                    | _ => None,
                })
                .collect();
            assert_eq!(invalid, [sequence], "{source:?}: {:#?}", outcome.errors);
            let location = outcome.location(|error| {
                matches!(
                    error,
                    PreprocessorErrorType::InvalidCharacterInHeaderName(_)
                )
            });
            assert_eq!(location.column, column, "{source:?}");
            assert_eq!(
                location.length as usize,
                if source.contains("??/") {
                    3
                } else {
                    sequence.len()
                },
                "{source:?}"
            );
            // Lookup still happens, with the name as written.
            assert!(
                outcome
                    .preprocessor_errors()
                    .iter()
                    .any(|error| matches!(error, PreprocessorErrorType::HeaderNotFound { .. })),
                "{source:?}"
            );
        });
    }
}

#[test]
fn expanded_header_errors_keep_the_lookup_failure() {
    let headers = Headers::new();
    for source in [
        "#define H <it's.h>\n#include H\nafter\n",
        "#define H \"it's.h\"\n#include H\nafter\n",
    ] {
        with_preprocess(&headers, source, |mut outcome| {
            assert_eq!(outcome.identifiers, ["after"]);
            assert!(outcome.preprocessor_errors().iter().any(|error| matches!(
                error,
                PreprocessorErrorType::InvalidCharacterInHeaderName("'")
            )));
            assert!(
                outcome
                    .preprocessor_errors()
                    .iter()
                    .any(|error| matches!(error, PreprocessorErrorType::HeaderNotFound { .. }))
            );
            let invalid = outcome.location(|error| {
                matches!(
                    error,
                    PreprocessorErrorType::InvalidCharacterInHeaderName(_)
                )
            });
            assert_eq!(invalid.line, 1, "{source:?}");
            assert_eq!(invalid.column, 14, "{source:?}");
            assert_eq!(invalid.length, 1, "{source:?}");
        });
    }
}

#[test]
fn missing_closing_delimiters_are_reported_before_lookup() {
    let headers = Headers::new();
    for (source, delimiter, column) in [
        ("#include <stdio.h\nafter\n", '>', 18),
        ("#include <stdio.h", '>', 18),
        ("#include \"stdio.h\nafter\n", '"', 18),
        ("#include \"stdio.h", '"', 18),
    ] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_preprocess_in(&headers, source, policy, |mut outcome| {
                let errors = outcome.preprocessor_errors();
                let missing = errors
                    .iter()
                    .position(|error| {
                        matches!(error, PreprocessorErrorType::UnterminatedHeaderName(found) if *found == delimiter)
                    })
                    .unwrap_or_else(|| panic!("{source:?}: {:#?}", outcome.errors));
                if let Some(lookup) = errors
                    .iter()
                    .position(|error| matches!(error, PreprocessorErrorType::HeaderNotFound { .. }))
                {
                    assert!(missing < lookup, "{source:?}");
                }
                let location = outcome.location(|error| {
                    matches!(error, PreprocessorErrorType::UnterminatedHeaderName(_))
                });
                assert_eq!(
                    (location.column, location.length),
                    (column, 0),
                    "{source:?}"
                );
                if source.ends_with("after\n") {
                    assert_eq!(outcome.identifiers, ["after"], "{source:?}");
                }
            });
        }
    }
}

#[test]
fn a_backslash_before_the_first_quote_uses_the_quoted_header_extension() {
    let headers = Headers::new();
    for source in ["#include \"dir\\\"", "#include \"dir\\\"\nafter\n"] {
        for policy in [ExtensionPolicy::Allow, ExtensionPolicy::Warn] {
            with_preprocess_in(&headers, source, policy, |outcome| {
                assert!(outcome.errors.iter().any(|error| matches!(
                    error,
                    TranslationError::Preprocessing(PreprocessorError {
                        error_type: PreprocessorErrorType::HeaderNotFound { name, .. },
                        ..
                    }) if *name == "dir\\"
                )));
                assert!(!outcome.errors.iter().any(|error| {
                    matches!(error, TranslationError::PreprocessorTokenizining(_))
                }));
                assert!(!outcome.preprocessor_errors().iter().any(|error| {
                    matches!(error, PreprocessorErrorType::UnterminatedHeaderName(_))
                }));
                assert_eq!(
                    outcome.preprocessor_errors().iter().any(|error| matches!(
                        error,
                        PreprocessorErrorType::BackslashInQuotedHeaderName(_)
                    )),
                    policy == ExtensionPolicy::Warn
                );
                if source.ends_with("after\n") {
                    assert_eq!(outcome.identifiers, ["after"]);
                }
            });
        }
        with_preprocess_in(&headers, source, ExtensionPolicy::Deny, |outcome| {
            assert!(
                outcome.errors.iter().any(|error| {
                    matches!(error, TranslationError::PreprocessorTokenizining(_))
                })
            );
            assert!(outcome.preprocessor_errors().iter().any(|error| {
                matches!(error, PreprocessorErrorType::UnterminatedHeaderName('"'))
            }));
            assert!(
                !outcome
                    .preprocessor_errors()
                    .iter()
                    .any(|error| { matches!(error, PreprocessorErrorType::HeaderNotFound { .. }) })
            );
        });
    }
}

#[test]
fn text_after_an_escaped_closing_quote_is_extra_tokens() {
    let headers = Headers::new();
    let extra_tokens = |error: &PreprocessorErrorType<'_>| {
        matches!(
            error,
            PreprocessorErrorType::ExtraTokensAfterIncludeDirective
        )
    };
    let not_found = |error: &PreprocessorErrorType<'_>| matches!(error, PreprocessorErrorType::HeaderNotFound { name, .. } if *name == "a\\");
    // The lexer reads one closed string, or one unterminated string.
    for source in [
        "#include \"a\\\" b\"\nafter\n",
        "#include \"a\\\" b\nafter\n",
        "#include \"a\\\" b",
    ] {
        for policy in [ExtensionPolicy::Allow, ExtensionPolicy::Warn] {
            with_preprocess_in(&headers, source, policy, |mut outcome| {
                let errors = outcome.preprocessor_errors();
                assert!(errors.iter().any(|error| not_found(error)), "{source:?}");
                assert_eq!(
                    errors.iter().filter(|error| extra_tokens(error)).count(),
                    1,
                    "{source:?}: {:#?}",
                    outcome.errors
                );
                assert!(!outcome.errors.iter().any(|error| {
                    matches!(error, TranslationError::PreprocessorTokenizining(_))
                }));
                // The warning points at `b`; the header name is `"a\"`.
                let extra = outcome.location(extra_tokens);
                assert_eq!((extra.line, extra.column, extra.length), (1, 15, 1));
                let header = outcome.location(not_found);
                assert_eq!((header.line, header.column, header.length), (1, 10, 4));
                let warning = outcome
                    .errors
                    .iter()
                    .find(|error| {
                        matches!(error, TranslationError::Preprocessing(PreprocessorError { error_type, .. }) if extra_tokens(error_type))
                    })
                    .unwrap();
                assert_eq!(warning.severity(), ErrorSeverity::Warning);
                if source.ends_with("after\n") {
                    assert_eq!(outcome.identifiers, ["after"], "{source:?}");
                }
            });
        }
        with_preprocess_in(&headers, source, ExtensionPolicy::Deny, |outcome| {
            let errors = outcome.preprocessor_errors();
            assert!(
                !errors.iter().any(|error| extra_tokens(error)),
                "{source:?}"
            );
            assert!(!errors.iter().any(|error| not_found(error)), "{source:?}");
            // The escaped quote does not close the name: a closed string
            // names `a\" b` and rejects its backslash; an unterminated one
            // keeps the lexer's error and the missing-quote error.
            if source.starts_with("#include \"a\\\" b\"") {
                assert!(errors.iter().any(|error| matches!(
                    error,
                    PreprocessorErrorType::BackslashInQuotedHeaderName(ExtensionPolicy::Deny)
                )));
            } else {
                assert!(outcome.errors.iter().any(|error| {
                    matches!(error, TranslationError::PreprocessorTokenizining(_))
                }));
                assert!(errors.iter().any(|error| {
                    matches!(error, PreprocessorErrorType::UnterminatedHeaderName('"'))
                }));
            }
        });
    }
    // Only whitespace after the closing quote is not an extra token.
    with_preprocess_in(
        &headers,
        "#include \"a\\\"  \nafter\n",
        ExtensionPolicy::Allow,
        |outcome| {
            let errors = outcome.preprocessor_errors();
            assert!(errors.iter().any(|error| not_found(error)));
            assert!(!errors.iter().any(|error| extra_tokens(error)));
            assert_eq!(outcome.identifiers, ["after"]);
        },
    );
}

#[test]
fn quoted_backslashes_follow_the_extension_policy() {
    let headers = Headers::new();
    headers.write(&backslash_header(), "found");
    let source = "#include \"dir\\file.h\"\nafter\n";

    with_preprocess_in(&headers, source, ExtensionPolicy::Allow, |outcome| {
        assert!(outcome.errors.is_empty(), "{:#?}", outcome.errors);
        assert_eq!(outcome.identifiers, ["found", "after"]);
    });

    with_preprocess_in(&headers, source, ExtensionPolicy::Warn, |mut outcome| {
        assert_eq!(outcome.identifiers, ["found", "after"]);
        assert!(matches!(
            outcome.errors.as_slice(),
            [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::BackslashInQuotedHeaderName(
                    ExtensionPolicy::Warn
                ),
                ..
            })]
        ));
        assert_eq!(outcome.errors[0].severity(), ErrorSeverity::Warning);
        let location = outcome.location(|error| {
            matches!(error, PreprocessorErrorType::BackslashInQuotedHeaderName(_))
        });
        assert_eq!((location.column, location.length), (14, 1));
    });

    with_preprocess_in(&headers, source, ExtensionPolicy::Deny, |outcome| {
        // The header is not looked up, so nothing from it appears.
        assert_eq!(outcome.identifiers, ["after"]);
        assert!(matches!(
            outcome.errors.as_slice(),
            [TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::BackslashInQuotedHeaderName(
                    ExtensionPolicy::Deny
                ),
                ..
            })]
        ));
        assert_eq!(outcome.errors[0].severity(), ErrorSeverity::Error);
    });
}

#[test]
fn angle_backslashes_are_errors_under_every_policy() {
    let headers = Headers::new();
    for policy in [
        ExtensionPolicy::Allow,
        ExtensionPolicy::Warn,
        ExtensionPolicy::Deny,
    ] {
        with_preprocess_in(&headers, "#include <dir\\file.h>\n", policy, |outcome| {
            assert!(
                outcome.preprocessor_errors().iter().any(|error| matches!(
                    error,
                    PreprocessorErrorType::InvalidCharacterInHeaderName("\\")
                )),
                "{policy:?}: {:#?}",
                outcome.errors
            );
            assert!(
                !outcome.preprocessor_errors().iter().any(|error| matches!(
                    error,
                    PreprocessorErrorType::BackslashInQuotedHeaderName(_)
                )),
                "{policy:?}"
            );
        });
    }
}

#[test]
fn renamed_files_read_header_names_from_the_physical_file() {
    let headers = Headers::new();
    headers.write(Path::new("plain.h"), "plain");
    with_preprocess(
        &headers,
        "#line 7 \"renamed.c\"\n#include \"plain.h\"\n#include <it's.h>\n",
        |outcome| {
            assert_eq!(outcome.identifiers, ["plain"]);
            assert!(outcome.preprocessor_errors().iter().any(|error| matches!(
                error,
                PreprocessorErrorType::InvalidCharacterInHeaderName("'")
            )));
        },
    );
}
