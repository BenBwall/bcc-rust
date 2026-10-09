//! Regressions for declaration visibility, compatibility and recovery.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn hidden_function_prototypes_do_not_constrain_later_calls() {
    for source in [
        "void f(void){extern int h(int);} void g(void){extern int h(); h(1,2);}",
        "int h(); void f(void){extern int h(int);} void g(void){extern int h(); h(1,2);}",
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
fn hidden_array_bounds_do_not_complete_later_declarations() {
    for source in [
        "void f(void){extern int a[3];} void g(void){extern int a[]; sizeof a;}",
        "extern int a[]; void f(void){extern int a[3];} void g(void){extern int a[]; sizeof a;}",
    ] {
        with_source(source, |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::InvalidSizeof,
                    ..
                })
            ));
        });
    }
}

#[test]
fn visible_linked_types_still_form_composites() {
    with_source(
        "extern int a[3]; void f(void){int a; {extern int a[];}} void g(void){extern int a[]; int \
         size[sizeof a == 12 ? 1 : -1];} int h(int); void k(void){extern int h(); h(1,2);}",
        |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::InvalidArgumentCount,
                    ..
                })
            ));
        },
    );
}

#[test]
fn incompatible_redeclarations_preserve_the_accepted_function_binding() {
    for standard in [CStandard::C99, CStandard::C11, CStandard::C23] {
        with_configuration(
            "int f(int *); int f(double); void g(int *p) { f(p); } int f(int *);",
            CompilerConfiguration::new(standard, ExtensionPolicy::Deny),
            |context, s| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{standard:?}: {errors:?}");
                assert!(matches!(
                    errors[0],
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::IncompatibleDeclaration,
                        ..
                    })
                ));
                let TranslationError::Semantic(error) = errors[0] else {
                    unreachable!();
                };
                let declarations = s
                    .bindings
                    .iter()
                    .filter(|b| context.string_cache.at(b.name.name) == "f")
                    .collect::<Vec<_>>();
                assert_eq!(declarations.len(), 3);
                assert_ne!(declarations[0].ty, declarations[1].ty);
                assert_eq!(declarations[0].ty, declarations[2].ty);
                assert_eq!(error.previous, Some(declarations[0].name.source_vectors));
                let reference = s
                    .expressions
                    .iter()
                    .find(|info| {
                        matches!(info.expression.kind, ExpressionType::Identifier(name)
                        if context.string_cache.at(name.name) == "f")
                    })
                    .unwrap();
                assert_eq!(reference.ty, declarations[0].ty);
            },
        );
    }
}

#[test]
fn incompatible_kinds_and_hidden_redeclarations_preserve_accepted_bindings() {
    for source in [
        "int f(int *); extern int f; void g(int *p) { f(p); } int f(int *);",
        "void f(void){extern int h(int *);} void g(void){extern int h(double);} void \
         k(void){extern int h(int *); h(0);}",
    ] {
        with_source(source, |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::IncompatibleDeclaration,
                    ..
                })
            ));
        });
    }
}

#[test]
fn repeated_typedefs_reject_variably_modified_types() {
    for (standard, gnu) in [
        (CStandard::C11, false),
        (CStandard::C17, false),
        (CStandard::C23, false),
        (CStandard::C99, true),
        (CStandard::C11, true),
    ] {
        for source in [
            "void f(int n){typedef int T[n]; typedef int T[n];}",
            "void f(int n){typedef int (*T)[n]; typedef int (*T)[n];}",
            "void f(int n){typedef int A[n]; typedef A T; typedef A T;}",
        ] {
            with_configuration(
                source,
                CompilerConfiguration::new(standard, ExtensionPolicy::Deny)
                    .with_gnu_extensions(gnu),
                |context, _| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        errors.len(),
                        1,
                        "{standard:?}, gnu={gnu}: {source}: {errors:?}"
                    );
                    assert!(matches!(
                        errors[0],
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::DuplicateDeclaration,
                            ..
                        })
                    ));
                },
            );
        }
    }
}

#[test]
fn repeated_non_vm_typedefs_and_nested_vm_typedefs_remain_valid() {
    for (standard, gnu) in [
        (CStandard::C11, false),
        (CStandard::C17, false),
        (CStandard::C23, false),
        (CStandard::C99, true),
    ] {
        with_configuration(
            "typedef int I; typedef int I; typedef int A[3]; typedef int A[3]; typedef int \
             (*P)[3]; typedef int (*P)[3]; void f(int n){typedef int T[n]; {typedef int T[n];}}",
            CompilerConfiguration::new(
                standard,
                if gnu {
                    ExtensionPolicy::Allow
                } else {
                    ExtensionPolicy::Deny
                },
            )
            .with_gnu_extensions(gnu),
            |context, _| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{standard:?}, gnu={gnu}: {:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
}

#[test]
fn hidden_linked_declarations_still_check_compatibility() {
    with_source(
        "void f(void){extern int a[3];} void g(void){extern double a[];}",
        |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::IncompatibleDeclaration,
                    ..
                })
            ));
        },
    );
}
