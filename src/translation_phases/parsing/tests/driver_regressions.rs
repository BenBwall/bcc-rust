//! Regression tests for verified driver bugs found by the overnight bug hunt.

use std::{
    fmt::Write,
    path::PathBuf,
};

use crate::{
    translation_phases::{
        Context,
        parsing::{
            Parser,
            inspection::InspectionOptions,
        },
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

/// Parses `source` and renders its located syntax tree.
fn located_tree(source: &str) -> String {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let parse_arena = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<located-tree-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();
    unit.inspect(
        context.tu_arena(),
        &context,
        InspectionOptions {
            show_locations: true,
        },
    )
    .to_owned()
}

#[test]
fn recovered_nodes_keep_their_locations() {
    let mut snapshot = String::new();
    for source in [
        "void f(void) {\n  if () ;\n}\nint after;\n",
        "void f(void) {\n  )\n  x;\n}\nint after;\n",
        "void f(void){ return ) ; }\nint z;\n",
        "void f(void) {\n  goto ;\n}\n",
        "void f(int x) {\n  switch (x) { case : ; }\n}\n",
        "int a;\nvoid f(void){\n  if () ;\n  while () ;\n}\nint z;\nint y;\n",
        "int b long; ;\nint after;\n",
    ] {
        _ = writeln!(snapshot, "=== {source:?}");
        snapshot.push_str(&located_tree(source));
    }
    // Recorded while the streaming and batch pipelines still had to agree.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inspection/recovered_locations.snap");
    if std::env::var_os("BLESS").is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &snapshot).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("{}: {error}; run with BLESS=1 to create it", path.display())
    });
    pretty_assertions::assert_eq!(expected, snapshot);
}

#[test]
fn missing_and_error_nodes_are_located_at_the_offending_token() {
    let tree = located_tree("void f(void) {\n  if () ;\n}\n");
    assert!(tree.contains("condition: missing @2:7"), "{tree}");

    let tree = located_tree("void f(void) {\n  )\n  x;\n}\n");
    // A stray `)` at statement start is skipped as a recovered null
    // statement located at the `)` itself.
    assert!(tree.contains("null recovered @2:3"), "{tree}");
}

/// Parses `source` and returns how many source segments the context holds
/// afterwards.
fn source_segments_after_parsing(source: &str) -> usize {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let parse_arena = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<segment-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let _unit = Parser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();
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
fn inspection_preserves_declaration_and_pointer_specifiers() {
    let tree = located_tree(
        "static inline const int f(register volatile int x) { const int * restrict p; return \
         (const int)x; }\n",
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
    let tree = located_tree("void f(void) { do { body(); } while (condition); }\n");
    assert!(
        tree.find("identifier body").unwrap() < tree.find("identifier condition").unwrap(),
        "{tree}"
    );
}

#[test]
fn deeply_nested_inspection_has_bounded_indentation_and_explicit_depth() {
    let source = format!("int f(void) {{ return {}1; }}\n", "- ".repeat(1000));
    let tree = located_tree(&source);
    assert!(tree.len() < 200_000, "{} output bytes", tree.len());
    assert!(
        tree.contains("[depth="),
        "deep indentation must retain its actual depth"
    );
}

#[test]
fn abstract_empty_function_declarators_preserve_unspecified_parameters() {
    let tree = located_tree("int f(); int (*p)(); int g(void) { return sizeof(int ()); }\n");
    assert_eq!(
        tree.matches("function identifier-list").count(),
        3,
        "{tree}"
    );
}

#[test]
fn parenthesized_declarator_locations_include_the_opening_delimiter() {
    let tree = located_tree("int (value);\n");
    assert!(tree.contains("parenthesized-declarator @1:5"), "{tree}");
}

#[test]
fn missing_semicolon_help_uses_macro_invocations_after_full_batch_preprocessing() {
    for (definition, call) in [
        ("#define DECL int a", "DECL"),
        ("#define DECL(x) int x", "DECL(a)"),
    ] {
        let source = format!(
            "{definition}\nstruct S {{\n{call}\nint b;\n}};\nstruct T {{\n{call}\nint b;\n}};\n"
        );
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let preprocess_arena = crate::util::bump::Bump::new();
        let parse_arena = crate::util::bump::Bump::new();
        let preprocessor = Preprocessor::new(
            &preprocess_arena,
            &mut context,
            PathBuf::from("<macro-help>").into_boxed_path(),
            &source,
            SharedVec::default(),
            SharedVec::default(),
        );
        let _unit = Parser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();
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
            "{call}"
        );
    }
}
