//! Regressions for function definition constraints, C99 §6.7.5.3 and §6.9.1.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn empty_old_style_definition_matches_prior_prototype_count() {
    for source in [
        "int f(int); int f() { return 0; }",
        "int f(int); int (f()) { return 0; }",
    ] {
        with_source(source, |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(errors.as_slice(), [TranslationError::Semantic(e)]
                    if e.kind == SemanticErrorKind::IncompatibleDeclaration
                        && e.previous.is_some()),
                "{source}: {errors:?}"
            );
        });
    }
    for source in [
        "int f(void); int f() { return 0; }",
        "int f(); int f() { return 0; }",
        "int f() { return 0; }",
        "int f(int); int f(a) short a; { return a; }",
    ] {
        with_source(source, |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{source}: {:?}",
                context.take_pending_errors()
            );
        });
    }
}

#[test]
fn nested_function_definitions_reject_static_and_extern_storage() {
    let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_gnu_extensions(true);
    for source in [
        "void f(void) { static int g(void) { return 0; } g(); }",
        "void f(void) { extern int g(void) { return 0; } g(); }",
    ] {
        with_configuration(source, configuration, |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(errors.as_slice(), [TranslationError::Semantic(e)]
                    if e.kind == SemanticErrorKind::NestedFunctionStorage),
                "{source}: {errors:?}"
            );
        });
    }
    for source in [
        "void f(void) { auto int g(void) { return 0; } g(); }",
        "void f(void) { int g(void) { return 0; } g(); }",
        "static int g(void) { return 0; } void f(void) { g(); }",
        "extern int f(void) { return 0; }",
    ] {
        with_configuration(source, configuration, |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{source}: {:?}",
                context.take_pending_errors()
            );
        });
    }
}

#[test]
fn unnamed_c23_definition_parameters_require_complete_adjusted_types() {
    let configuration = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow);
    for source in [
        "struct S; void f(struct S) {}",
        "union U; void f(union U) {}",
        "typedef struct S S; void f(S) {}",
    ] {
        with_configuration(source, configuration, |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(errors.as_slice(), [TranslationError::Semantic(e)]
                    if e.kind == SemanticErrorKind::IncompleteDefinitionParameter
                        && e.name.is_none()),
                "{source}: {errors:?}"
            );
        });
    }
    for source in [
        "struct S { int n; }; void f(struct S) {}",
        "struct S; void f(struct S *) {}",
        "void f(int [3]) {}",
        "void f(int (void)) {}",
        "void f(int) {}",
        "void f(void) {}",
    ] {
        with_configuration(source, configuration, |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{source}: {:?}",
                context.take_pending_errors()
            );
        });
    }
}
