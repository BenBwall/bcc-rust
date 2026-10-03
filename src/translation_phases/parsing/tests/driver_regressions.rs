//! Regression tests for verified driver bugs found by the overnight bug hunt.

use std::path::PathBuf;

use crate::{
    pipeline::PreprocessingStrategy,
    translation_phases::{
        Context,
        parsing::inspection::InspectionOptions,
    },
    util::shared::SharedVec,
};

/// Parses `source` under `strategy` and renders its located syntax tree.
fn located_tree(source: &str, strategy: PreprocessingStrategy) -> String {
    let mut context = Context::new();
    let preprocessor = strategy.preprocessor(
        &mut context,
        PathBuf::from("<strategy-test>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = strategy
        .parser(preprocessor, &mut context)
        .parse_translation_unit(&mut context);
    unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions {
            show_locations: true,
        },
    )
}

#[test]
fn recovered_nodes_are_located_identically_under_every_strategy() {
    for source in [
        "void f(void) {\n  if () ;\n}\nint after;\n",
        "void f(void) {\n  )\n  x;\n}\nint after;\n",
        "void f(void){ return ) ; }\nint z;\n",
        "void f(void) {\n  goto ;\n}\n",
        "void f(int x) {\n  switch (x) { case : ; }\n}\n",
        "int a;\nvoid f(void){\n  if () ;\n  while () ;\n}\nint z;\nint y;\n",
    ] {
        let streaming = located_tree(source, PreprocessingStrategy::Streaming);
        for strategy in [
            PreprocessingStrategy::BatchLexing,
            PreprocessingStrategy::Batch,
        ] {
            assert_eq!(
                located_tree(source, strategy),
                streaming,
                "{strategy:?} disagrees with streaming for {source:?}"
            );
        }
    }
}

#[test]
fn missing_and_error_nodes_are_located_at_the_offending_token() {
    let tree = located_tree(
        "void f(void) {\n  if () ;\n}\n",
        PreprocessingStrategy::Batch,
    );
    assert!(tree.contains("condition: missing @2:7"), "{tree}");

    let tree = located_tree(
        "void f(void) {\n  )\n  x;\n}\n",
        PreprocessingStrategy::Streaming,
    );
    // A stray `)` at statement start is skipped as a recovered null
    // statement located at the `)` itself.
    assert!(tree.contains("null recovered @2:3"), "{tree}");
}

/// Parses `source` with the streaming strategy and returns how many source
/// segments the context holds afterwards.
fn source_segments_after_parsing(source: &str) -> usize {
    let mut context = Context::new();
    let strategy = PreprocessingStrategy::Streaming;
    let preprocessor = strategy.preprocessor(
        &mut context,
        PathBuf::from("<segment-test>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let _unit = strategy
        .parser(preprocessor, &mut context)
        .parse_translation_unit(&mut context);
    context.source_segment_count()
}

#[test]
fn an_error_under_deep_nesting_keeps_provenance_linear() {
    // Every enclosing cast used to copy the growing provenance of the
    // error operand, so 6000 casts exceeded the 40 million segment budget.
    let depth = 2_000;
    let source = format!(
        "typedef int T; int a;\nvoid g(void){{ a = {}; }}\nint y = ;\n",
        "(T)".repeat(depth)
    );
    let segments = source_segments_after_parsing(&source);
    assert!(
        segments < 50 * depth,
        "{segments} segments for depth {depth}"
    );

    let source = format!(
        "int f(int);\nvoid g(void){{ {}x +{}; }}\n",
        "f(".repeat(depth),
        ")".repeat(depth)
    );
    let segments = source_segments_after_parsing(&source);
    assert!(
        segments < 50 * depth,
        "{segments} segments for depth {depth}"
    );
}

#[test]
fn recovered_function_bodies_use_the_current_token_in_every_strategy() {
    let source = "int b long; ;\nint after;\n";
    let streaming = located_tree(source, PreprocessingStrategy::Streaming);
    for strategy in [
        PreprocessingStrategy::BatchLexing,
        PreprocessingStrategy::Batch,
    ] {
        assert_eq!(located_tree(source, strategy), streaming);
    }
}

#[test]
fn inspection_preserves_declaration_and_pointer_specifiers() {
    let tree = located_tree(
        "static inline const int f(register volatile int x) { const int * restrict p; return \
         (const int)x; }\n",
        PreprocessingStrategy::Streaming,
    );
    for expected in [
        "storage=static",
        "function-specifiers=inline",
        "qualifiers=const",
        "storage=register",
        "qualifiers=volatile",
        "qualifiers=restrict",
    ] {
        assert!(tree.contains(expected), "missing {expected}: {tree}");
    }
}

#[test]
fn inspection_lists_do_while_body_before_condition() {
    let tree = located_tree(
        "void f(void) { do { body(); } while (condition); }\n",
        PreprocessingStrategy::Streaming,
    );
    assert!(
        tree.find("identifier body").unwrap() < tree.find("identifier condition").unwrap(),
        "{tree}"
    );
}

#[test]
fn deeply_nested_inspection_has_bounded_indentation_and_explicit_depth() {
    let source = format!("int f(void) {{ return {}1; }}\n", "- ".repeat(1000));
    let tree = located_tree(&source, PreprocessingStrategy::Streaming);
    assert!(tree.len() < 200_000, "{} output bytes", tree.len());
    assert!(
        tree.contains("[depth="),
        "deep indentation must retain its actual depth"
    );
}

#[test]
fn abstract_empty_function_declarators_preserve_unspecified_parameters() {
    let tree = located_tree(
        "int f(); int (*p)(); int g(void) { return sizeof(int ()); }\n",
        PreprocessingStrategy::Streaming,
    );
    assert_eq!(
        tree.matches("function identifier-list").count(),
        3,
        "{tree}"
    );
}

#[test]
fn parenthesized_declarator_locations_include_the_opening_delimiter() {
    let tree = located_tree("int (value);\n", PreprocessingStrategy::Streaming);
    assert!(tree.contains("parenthesized-declarator @1:5"), "{tree}");
}

#[test]
fn missing_semicolon_help_uses_macro_invocations_after_full_batch_preprocessing() {
    for (definition, call) in [
        ("#define DECL int a", "DECL"),
        ("#define DECL(x) int x", "DECL(a)"),
    ] {
        for strategy in [
            PreprocessingStrategy::Streaming,
            PreprocessingStrategy::BatchLexing,
            PreprocessingStrategy::Batch,
        ] {
            let source = format!(
                "{definition}\nstruct S {{\n{call}\nint b;\n}};\nstruct T {{\n{call}\nint \
                 b;\n}};\n"
            );
            let mut context = Context::new();
            let preprocessor = strategy.preprocessor(
                &mut context,
                PathBuf::from("<macro-help>").into_boxed_path(),
                source.into(),
                SharedVec::default(),
                SharedVec::default(),
            );
            let _unit = strategy
                .parser(preprocessor, &mut context)
                .parse_translation_unit(&mut context);
            let insertions: Vec<_> = context
                .take_pending_errors()
                .into_iter()
                .filter_map(|error| match error {
                    | crate::translation_phases::TranslationError::Parsing(error) =>
                        error.insertion_point,
                    | _ => None,
                })
                .map(|source| {
                    let vector = &context.get_source_vectors(source)[0];
                    (vector.line, vector.column)
                })
                .collect();
            assert_eq!(
                insertions,
                [
                    (3, u32::try_from(call.len()).unwrap() + 1),
                    (7, u32::try_from(call.len()).unwrap() + 1)
                ],
                "{strategy:?}: {call}"
            );
        }
    }
}
