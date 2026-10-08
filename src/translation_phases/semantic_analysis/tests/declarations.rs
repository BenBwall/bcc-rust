//! Declaration, tag and layout regressions from the Stage-1 review.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

fn gnu17() -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow).with_gnu_extensions(true)
}

fn pedantic(standard: CStandard, gnu: bool) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, ExtensionPolicy::Deny).with_gnu_extensions(gnu)
}

/// Semantic error kinds in discovery order.
fn kinds(source: &str, configuration: CompilerConfiguration) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Semantic(e) => kinds.push(e.kind),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    kinds
}

/// Extension diagnostics, in order, as their rendered messages.
fn extensions(source: &str, configuration: CompilerConfiguration) -> Vec<String> {
    let mut messages = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Extension(e) => messages.push(e.to_string()),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    messages
}

fn tag_layout(source: &str, configuration: CompilerConfiguration, name: &str) -> Option<Layout> {
    let mut layout = None;
    with_configuration(source, configuration, |context, s| {
        assert_eq!(context.pending_error_count(), 0, "{source}");
        layout = s
            .types
            .tags
            .iter()
            .find(|t| t.name.is_some_and(|n| context.string_cache.at(n) == name))
            .and_then(|t| t.layout.get());
    });
    layout
}

#[test]
fn nested_tag_redefinition_is_diagnosed_once_and_terminates() {
    assert_eq!(
        kinds(
            "struct S { struct S { int x; } y; int z; }; struct S s; void f(void) { s = s; } int \
             n = sizeof(s);",
            gnu17()
        ),
        [SemanticErrorKind::TagRedefinition]
    );
    assert_eq!(
        kinds("enum E { A = sizeof(enum E { B }) };", gnu17()),
        [SemanticErrorKind::TagRedefinition]
    );
}

#[test]
fn rejected_tag_bodies_still_declare_their_contents() {
    assert_eq!(
        kinds(
            "struct S { int x; }; struct S { enum { Q = 2 } q; }; int a[Q]; enum E { A }; enum E \
             { B = 3 }; int b[B]; union S { int u; enum { R } r; } v; int c[R + 1];",
            gnu17()
        ),
        [
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagKindMismatch,
        ]
    );
}

#[test]
fn self_containing_records_terminate() {
    assert_eq!(
        kinds(
            "struct S { int a; struct S s; }; struct S x; void f(void) { x = x; x.a = 1; struct S \
             y = {1}; }",
            gnu17()
        ),
        [SemanticErrorKind::InvalidMember]
    );
}
