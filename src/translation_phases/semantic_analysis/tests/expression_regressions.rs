//! Phase-7 regressions for builtin expression types, constants and recovery.
//! C99: §7.17p3, p. 254; PDF p. 266; §7.15.1.1p2, pp. 249-250;
//! PDF pp. 261-262; §6.7.5.2p1, p. 116; PDF p. 128.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn offsetof_resolves_anonymous_members_and_unnamed_bit_fields() {
    for (source, expected) in [
        (
            "struct S { int pad; union { int a; long b; }; int tail; }; int \
             arr[__builtin_offsetof(struct S, tail) == 16 ? 1 : -1];",
            16,
        ),
        (
            "#include <stddef.h>\nstruct S { int pad; struct { int a; int b; }; }; int \
             arr[offsetof(struct S, b) == 8 ? 1 : -1];",
            8,
        ),
        (
            "struct S { int :1; int x; }; int arr[__builtin_offsetof(struct S, x) == 4 ? 1 : -1];",
            4,
        ),
    ] {
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
            |context, unit| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{source}: {:?}",
                    context.take_pending_errors()
                );
                let info = unit
                    .expressions
                    .iter()
                    .find(|info| matches!(info.expression.kind, ExpressionType::Builtin(_)))
                    .unwrap();
                assert_eq!(info.integer.unwrap().value, expected, "{source}");
                assert!(info.ice);
                assert_eq!(
                    unit.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::UnsignedLong)
                );
                assert!(matches!(
                    unit.types.nodes[unit.bindings.last().unwrap().ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(1))
                ));
            },
        );
    }
}

#[test]
fn failed_va_arg_operands_suppress_dependent_diagnostics() {
    for (body, expected) in [
        (
            "int *p = __builtin_va_arg(missing, int);",
            SemanticErrorKind::UndeclaredIdentifier,
        ),
        (
            "__builtin_va_arg(missing, int).x;",
            SemanticErrorKind::UndeclaredIdentifier,
        ),
        (
            "int *p = __builtin_va_arg(123, int);",
            SemanticErrorKind::InvalidVaList,
        ),
        (
            "__builtin_va_arg(123, int).x;",
            SemanticErrorKind::InvalidVaList,
        ),
    ] {
        let source = format!(
            "void f(__builtin_va_list ap) {{ {body} int ok = __builtin_va_arg(ap, int); }}"
        );
        with_source(&source, |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError { kind, .. }) if kind == expected
            ));
            let mut builtins = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Builtin(_)));
            assert_eq!(
                unit.types.nodes[builtins.next().unwrap().ty.index],
                TypeKind::Unknown
            );
            assert_eq!(
                unit.types.nodes[builtins.next().unwrap().ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
            assert!(builtins.next().is_none());
        });
    }
}

#[test]
fn constant_p_has_int_type_and_checks_array_bounds() {
    for configuration in [
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Deny).with_gnu_extensions(true),
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Deny),
    ] {
        with_configuration(
            "int n; int bad[__builtin_constant_p(1) ? -1 : 1]; int good[__builtin_constant_p(n) ? \
             -1 : 1]; int size[sizeof(__builtin_constant_p(1)) == sizeof(int) ? 1 : -1]; int \
             signedness[(1 ? __builtin_constant_p(0) : 0UL) - 2 < 0 ? -1 : 1];",
            configuration,
            |context, unit| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{configuration:?}: {errors:?}");
                assert!(matches!(
                    errors[0],
                    TranslationError::Semantic(SemanticError {
                        kind: SemanticErrorKind::InvalidArrayBound,
                        ..
                    })
                ));
                let mut calls = unit
                    .expressions
                    .iter()
                    .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }));
                for expected in [1, 0, 1, 1] {
                    let info = calls.next().unwrap();
                    assert_eq!(
                        unit.types.nodes[info.ty.index],
                        TypeKind::Scalar(Scalar::Int)
                    );
                    assert_eq!(info.integer.unwrap().value, expected);
                    assert!(info.ice);
                }
                assert!(calls.next().is_none());
                for name in ["good", "size", "signedness"] {
                    let binding = unit
                        .bindings
                        .iter()
                        .find(|binding| context.string_cache.at(binding.name.name) == name)
                        .unwrap();
                    assert!(matches!(
                        unit.types.nodes[binding.ty.index],
                        TypeKind::Array(_, ArrayBound::Constant(1))
                    ));
                }
            },
        );
    }
}
