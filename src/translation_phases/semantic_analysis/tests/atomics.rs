//! C11 atomic type, conversion and intrinsic regression tests.

use super::*;

#[test]
fn atomic_types_have_layout_and_distinct_pointer_identity() {
    with_source(
        "_Atomic int a=1; _Atomic(int) b=2; int c; _Static_assert(sizeof(a)==4,\"size\"); \
         _Static_assert(_Alignof(_Atomic(int))==4,\"align\"); _Static_assert(_Generic(&a, \
         _Atomic(int)*:1, default:0),\"identity\"); _Static_assert(_Generic(a, int:1, \
         default:0),\"conversion\"); void f(void) { a++; --b; a+=2; c=a; a=c; }",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            assert_eq!(unit.bindings[0].ty, unit.bindings[1].ty);
            assert_ne!(unit.bindings[0].ty, unit.bindings[2].ty);
            assert_eq!(
                unit.types.layout(unit.bindings[0].ty),
                Some(Layout { size: 4, align: 4 })
            );
        },
    );
}

#[test]
fn generic_atomic_result_checks_are_evaluated() {
    with_source(
        "_Atomic int a; _Static_assert(_Generic(a,int:0,default:1),\"must fail\");",
        |context, _| {
            assert!(context.take_pending_errors().iter().any(|error| matches!(
                error,
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::FailedAssertion,
                    ..
                })
            )));
        },
    );
}

#[test]
fn atomic_casts_do_not_fold_in_strict_or_gnu_constant_requirements() {
    for gnu in [false, true] {
        with_configuration(
            "_Atomic int initialized=(_Atomic(int))1; \
             _Static_assert((_Atomic(int))1+2==3,\"non-ICE\"); \
             _Static_assert(_Generic(1,int:(_Atomic(int))1,default:0),\"non-ICE\"); \
             _Static_assert(_Generic(1,int:1,default:(_Atomic(int))1),\"unselected\");",
            crate::configuration::CompilerConfiguration::new(
                CStandard::C11,
                crate::configuration::ExtensionPolicy::Allow,
            )
            .with_gnu_extensions(gnu),
            |context, _| {
                let errors = context.take_pending_errors();
                assert_eq!(
                    errors
                        .iter()
                        .filter(|error| matches!(
                            error,
                            TranslationError::Semantic(SemanticError {
                                kind: SemanticErrorKind::InvalidConstant,
                                ..
                            })
                        ))
                        .count(),
                    2,
                    "{errors:?}"
                );
                assert_eq!(errors.len(), 2, "{errors:?}");
            },
        );
    }
}

#[test]
fn atomic_builtin_results_and_constraints() {
    with_source(
        "_Atomic int a; int n; void f(void) { __c11_atomic_init(&a,1); n=__c11_atomic_load(&a,0); \
         __atomic_store_n(&n,1,0); n=__sync_fetch_and_add(&n,1); }",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let calls = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .map(|info| unit.types.nodes[info.ty.index])
                .collect::<Vec<_>>();
            assert_eq!(
                calls,
                [
                    TypeKind::Scalar(Scalar::Void),
                    TypeKind::Scalar(Scalar::Int),
                    TypeKind::Scalar(Scalar::Void),
                    TypeKind::Scalar(Scalar::Int)
                ]
            );
        },
    );
}

#[test]
fn atomic_resource_probe_checks_every_target_and_revision() {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::LinuxMusl,
        crate::target::Target::WindowsGnu,
        crate::target::Target::WindowsMsvc,
    ] {
        for standard in [CStandard::C11, CStandard::C17, CStandard::C23] {
            with_configuration(
                include_str!("../../../../tests/fixtures/targets/atomic.c"),
                crate::configuration::CompilerConfiguration::new(
                    standard,
                    crate::configuration::ExtensionPolicy::Allow,
                )
                .with_target(target),
                |context, unit| {
                    assert_eq!(
                        context.pending_error_count(),
                        0,
                        "{} {standard:?}: {:?}",
                        target.triple(),
                        context.take_pending_errors()
                    );
                    for info in unit.expressions {
                        if let ExpressionType::Call {
                            function_expression,
                            ..
                        } = info.expression.kind
                        {
                            assert_ne!(unit.types.nodes[info.ty.index], TypeKind::Unknown);
                            if let ExpressionType::Identifier(name) = function_expression.kind
                                && matches!(
                                    context.string_cache.at(name.name),
                                    "__c11_atomic_init"
                                        | "__c11_atomic_store"
                                        | "__c11_atomic_thread_fence"
                                        | "__c11_atomic_signal_fence"
                                        | "__atomic_store_n"
                                        | "__atomic_load"
                                        | "__atomic_store"
                                        | "__atomic_exchange"
                                        | "__atomic_clear"
                                        | "__atomic_thread_fence"
                                        | "__atomic_signal_fence"
                                        | "__sync_lock_release"
                                        | "__sync_synchronize"
                                )
                            {
                                assert_eq!(
                                    unit.types.nodes[info.ty.index],
                                    TypeKind::Scalar(Scalar::Void)
                                );
                            }
                        }
                    }
                },
            );
        }
    }
}

#[test]
fn atomic_constraint_diagnostics_preserve_following_input() {
    for (source, kind, count) in [
        (
            include_str!("../../../../tests/fixtures/diagnostics/sema-atomic-type.c"),
            SemanticErrorKind::InvalidAtomicType,
            8,
        ),
        (
            include_str!("../../../../tests/fixtures/diagnostics/sema-atomic-builtin.c"),
            SemanticErrorKind::InvalidAtomicOperand,
            8,
        ),
        (
            include_str!("../../../../tests/fixtures/diagnostics/sema-atomic-order.c"),
            SemanticErrorKind::InvalidAtomicOrder,
            6,
        ),
    ] {
        with_configuration(
            source,
            crate::configuration::CompilerConfiguration::new(
                CStandard::C11,
                crate::configuration::ExtensionPolicy::Allow,
            ),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert_eq!(
                    errors
                        .iter()
                        .filter(
                            |e| matches!(e,TranslationError::Semantic(error) if error.kind==kind)
                        )
                        .count(),
                    count,
                    "{errors:?}"
                );
                assert!(
                    unit.bindings
                        .iter()
                        .any(|b| context.string_cache.at(b.name.name) == "following")
                );
            },
        );
    }
}

#[test]
fn atomic_availability_uses_existing_extension_policy() {
    use crate::configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
    };
    for standard in [
        CStandard::C89,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
    ] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_configuration(
                "_Atomic int a; _Atomic(int) b;",
                CompilerConfiguration::new(standard, policy),
                |context, _| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        errors
                            .iter()
                            .filter(|e| matches!(e, TranslationError::Extension(_)))
                            .count(),
                        if standard < CStandard::C11 && policy != ExtensionPolicy::Allow {
                            2
                        } else {
                            0
                        }
                    );
                    assert!(
                        !errors
                            .iter()
                            .any(|e| matches!(e, TranslationError::Semantic(_)))
                    );
                },
            );
        }
    }
}

#[test]
fn atomic_var_init_deprecation_is_limited_to_c17() {
    for (standard, warnings) in [(CStandard::C11, 0), (CStandard::C17, 1)] {
        with_configuration(
            "#include <stdatomic.h>\natomic_int a=ATOMIC_VAR_INIT(1);",
            crate::configuration::CompilerConfiguration::new(
                standard,
                crate::configuration::ExtensionPolicy::Allow,
            ),
            |context, _| {
                assert_eq!(
                    context.pending_error_count(),
                    warnings,
                    "{:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
    with_configuration(
        "#define _CLANG_DISABLE_CRT_DEPRECATION_WARNINGS\n#include <stdatomic.h>\natomic_int \
         a=ATOMIC_VAR_INIT(1);",
        crate::configuration::CompilerConfiguration::new(
            CStandard::C17,
            crate::configuration::ExtensionPolicy::Allow,
        ),
        |context, _| assert_eq!(context.pending_error_count(), 0),
    );
}
