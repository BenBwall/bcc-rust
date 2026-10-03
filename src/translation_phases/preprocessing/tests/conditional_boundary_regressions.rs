//! Source-file conditional boundary regressions only; no else-order policy.
//! Scratch draft, not registered or compiled.

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

use crate::{
    pipeline::PreprocessingStrategy,
    translation_phases::{
        Context,
        SourceVector,
        TranslationError,
    },
    util::shared::SharedVec,
};

const STRATEGIES: [PreprocessingStrategy; 3] = [
    PreprocessingStrategy::Streaming,
    PreprocessingStrategy::BatchLexing,
    PreprocessingStrategy::Batch,
];

#[derive(Debug)]
struct Headers(PathBuf);

impl Headers {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "bcc-conditional-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).unwrap();
    }
}

impl Drop for Headers {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

#[derive(Debug)]
struct ErrorRecord {
    kind:    String,
    vectors: Vec<SourceVector>,
    files:   Vec<PathBuf>,
}

#[derive(Debug, Default)]
struct Observation {
    spellings: Vec<String>,
    errors:    Vec<ErrorRecord>,
}

fn drain_errors(context: &mut Context, observation: &mut Observation) {
    for error in context.take_pending_errors() {
        let TranslationError::Preprocessing(error) = error else {
            panic!("unexpected non-preprocessing diagnostic: {error:#?}");
        };
        let vectors = context.get_source_vectors(error.source_vectors).to_vec();
        let files = vectors
            .iter()
            .map(|vector| {
                context
                    .get_source_file(vector.source_file_index)
                    .to_path_buf()
            })
            .collect();
        observation.errors.push(ErrorRecord {
            kind: format!("{:?}", error.error_type),
            vectors,
            files,
        });
    }
}

fn observe(source: &str, path: &Path, strategy: PreprocessingStrategy) -> Observation {
    let mut context = Context::new();
    let mut preprocessor = strategy.preprocessor(
        &mut context,
        path.to_path_buf().into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut observation = Observation::default();
    if strategy == PreprocessingStrategy::Batch {
        for token in preprocessor.preprocess_all(&mut context) {
            observation
                .spellings
                .push(context.string_cache.at(token.contents).to_owned());
        }
        drain_errors(&mut context, &mut observation);
    } else {
        loop {
            let token = preprocessor.next_iterator_item(&mut context);
            drain_errors(&mut context, &mut observation);
            let Some(token) = token else { break };
            observation
                .spellings
                .push(context.string_cache.at(token.contents).to_owned());
        }
    }
    observation
}

fn assert_error(error: &ErrorRecord, kind: &str, path: &Path, line: u32) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.files[0], path);
    assert_eq!((error.vectors[0].line, error.vectors[0].column), (line, 2));
}

#[test]
fn header_terminators_cannot_close_or_skip_the_callers_group() {
    let headers = Headers::new();
    let main = headers.0.join("main.c");
    let header = headers.0.join("bad.h");
    let source = "#if 1\n#include \"bad.h\"\ncaller_inside\n#endif\ncaller_after\n";
    for (directive, error_kind) in [
        ("#endif", "MoreEndifDirectivesThanIfDirectives"),
        ("#else", "ElseDirectiveWithoutIfDirective"),
        ("#elif 0", "ElifDirectiveWithoutIfDirective"),
    ] {
        headers.write("bad.h", &format!("{directive}\nheader_after\n"));
        for strategy in STRATEGIES {
            let actual = observe(source, &main, strategy);
            assert_eq!(
                actual.spellings,
                ["header_after", "caller_inside", "caller_after"],
                "{directive}: {strategy:?}: {actual:#?}"
            );
            assert_eq!(actual.errors.len(), 1, "{actual:#?}");
            assert_error(&actual.errors[0], error_kind, &header, 1);
        }
    }
}

#[test]
fn unclosed_header_groups_diagnose_locally_and_restore_the_caller() {
    let headers = Headers::new();
    let main = headers.0.join("main.c");
    let header = headers.0.join("open.h");
    let source =
        "#if 1\n#include \"open.h\"\ncaller_inside\n#else\ncaller_dead\n#endif\ncaller_after\n";
    for (contents, expected) in [
        (
            "#if 1\nheader_after\n",
            &["header_after", "caller_inside", "caller_after"][..],
        ),
        (
            "#if 0\nheader_dead\n",
            &["caller_inside", "caller_after"][..],
        ),
    ] {
        headers.write("open.h", contents);
        for strategy in STRATEGIES {
            let actual = observe(source, &main, strategy);
            assert_eq!(actual.spellings, expected, "{strategy:?}: {actual:#?}");
            assert_eq!(actual.errors.len(), 1, "{actual:#?}");
            assert_error(
                &actual.errors[0],
                "MoreIfDirectivesThanEndifDirectives",
                &header,
                1,
            );
        }
    }
}

#[test]
fn header_eof_diagnoses_every_local_opening_once_in_source_order() {
    let headers = Headers::new();
    let header = headers.0.join("open.h");
    headers.write("open.h", "#if 1\nouter_header\n#if 1\ninner_header\n");
    for strategy in STRATEGIES {
        let actual = observe(
            "#include \"open.h\"\ncaller_after\n",
            &headers.0.join("main.c"),
            strategy,
        );
        assert_eq!(
            actual.spellings,
            ["outer_header", "inner_header", "caller_after"]
        );
        assert_eq!(actual.errors.len(), 2, "{actual:#?}");
        assert_error(
            &actual.errors[0],
            "MoreIfDirectivesThanEndifDirectives",
            &header,
            1,
        );
        assert_error(
            &actual.errors[1],
            "MoreIfDirectivesThanEndifDirectives",
            &header,
            3,
        );
    }
}

#[test]
fn valid_nested_header_conditionals_preserve_the_callers_selection() {
    let headers = Headers::new();
    headers.write(
        "outer.h",
        "#if 1\n#include \"inner.h\"\nouter_header\n#endif\n",
    );
    headers.write("inner.h", "#if 0\ndead\n#else\ninner_header\n#endif\n");
    for strategy in STRATEGIES {
        let actual = observe(
            "#if 1\n#include \
             \"outer.h\"\ncaller_inside\n#else\ncaller_dead\n#endif\ncaller_after\n",
            &headers.0.join("main.c"),
            strategy,
        );
        assert_eq!(
            actual.spellings,
            [
                "inner_header",
                "outer_header",
                "caller_inside",
                "caller_after"
            ]
        );
        assert!(actual.errors.is_empty(), "{actual:#?}");
    }
}

#[test]
fn unclosed_false_header_without_a_caller_group_preserves_the_remainder() {
    let headers = Headers::new();
    let header = headers.0.join("open.h");
    headers.write("open.h", "#if 0\nheader_dead\n");
    for strategy in STRATEGIES {
        let actual = observe(
            "#include \"open.h\"\ncaller_after\n",
            &headers.0.join("main.c"),
            strategy,
        );
        assert_eq!(actual.spellings, ["caller_after"]);
        assert_eq!(actual.errors.len(), 1, "{actual:#?}");
        assert_error(
            &actual.errors[0],
            "MoreIfDirectivesThanEndifDirectives",
            &header,
            1,
        );
    }
}

#[test]
fn macro_frame_pops_do_not_end_a_source_file_conditional() {
    let headers = Headers::new();
    headers.write(
        "macros.h",
        "#define OBJECT object_token\n#define ID(x) x\n#if 1\nOBJECT ID(argument_token)\n#endif\n",
    );
    for strategy in STRATEGIES {
        let actual = observe(
            "#if 1\n#include \"macros.h\"\ncaller_inside\n#endif\ncaller_after\n",
            &headers.0.join("main.c"),
            strategy,
        );
        assert_eq!(
            actual.spellings,
            [
                "object_token",
                "argument_token",
                "caller_inside",
                "caller_after"
            ]
        );
        assert!(actual.errors.is_empty(), "{actual:#?}");
    }
}

#[test]
fn presumed_header_filename_does_not_change_its_conditional_boundary() {
    let headers = Headers::new();
    headers.write("bad.h", "#line 1 \"mapped.h\"\n#endif\nheader_after\n");
    for strategy in STRATEGIES {
        let actual = observe(
            "#if 1\n#include \"bad.h\"\ncaller_inside\n#endif\ncaller_after\n",
            &headers.0.join("main.c"),
            strategy,
        );
        assert_eq!(
            actual.spellings,
            ["header_after", "caller_inside", "caller_after"]
        );
        assert_eq!(actual.errors.len(), 1, "{actual:#?}");
        assert_error(
            &actual.errors[0],
            "MoreEndifDirectivesThanIfDirectives",
            Path::new("mapped.h"),
            1,
        );
    }
}

#[test]
fn skipped_nested_malformed_operands_remain_ignored() {
    let headers = Headers::new();
    let source = "#if 0\n#unknown ##\n# 123\n#ifdef\n#if + garbage\n#endif ignored\n#endif \
                  also_ignored\n#else\nchosen\n#endif\nafter\n";
    for strategy in STRATEGIES {
        let actual = observe(source, &headers.0.join("main.c"), strategy);
        assert_eq!(actual.spellings, ["chosen", "after"]);
        assert!(actual.errors.is_empty(), "{actual:#?}");
    }
}
