//! Phase-7 regressions for builtin expression types, constants and recovery.
//! C99: §7.17p3, p. 254; PDF p. 266; §7.15.1.1p2, pp. 249-250;
//! PDF pp. 261-262; §6.7.5.2p1, p. 116; PDF p. 128.

use super::*;
use crate::configuration::{
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn typeof_expression_pointer_member_suppresses_unanalyzed_target() {
    with_configuration(
        "struct S { int x; } s; void f(void) { __typeof__(s) *p = &s; p->x = 1; }",
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true),
        assert_clean,
    );
}

#[test]
fn typeof_type_pointer_member_suppresses_unanalyzed_target() {
    with_configuration(
        "struct S { int x; }; void f(void) { __typeof__(struct S) *p; p->x; }",
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true),
        assert_clean,
    );
}

#[test]
fn typeof_function_pointer_call_suppresses_unanalyzed_target() {
    with_configuration(
        "int t(void); __typeof__(t) *q; void f(void) { q(); }",
        CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
            .with_gnu_extensions(true),
        assert_clean,
    );
}

fn assert_clean(context: &mut Context<'_>, _: &SemanticTranslationUnit<'_>) {
    assert_eq!(
        context.pending_error_count(),
        0,
        "{:?}",
        context.take_pending_errors()
    );
}

#[test]
fn unevaluated_commas_preserve_null_pointer_ice() {
    for expression in [
        "1 ? 0 : (1,0)",
        "0 ? (1,0) : 0",
        "0 && (1,0)",
        "!(1 || (1,0))",
    ] {
        with_configuration(
            &format!("int *p = {expression};"),
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, unit| {
                assert_clean(context, unit);
                let info = unit.expressions.last().unwrap();
                assert!(info.ice, "{expression}");
                assert_eq!(info.integer.unwrap().value, 0, "{expression}");
            },
        );
    }
}

#[test]
fn evaluated_commas_are_not_null_pointer_ice() {
    for expression in ["1 ? (1,0) : 0", "0 ? 0 : (1,0)", "1 && (1,0)", "0 || (1,0)"] {
        with_configuration(
            &format!("int *p = {expression};"),
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, _| {
                let errors = context.take_pending_errors();
                assert!(
                    errors.iter().any(|error| matches!(
                        error,
                        TranslationError::Semantic(error)
                            if error.kind == SemanticErrorKind::InvalidInitializer
                    )),
                    "{expression}: {errors:?}"
                );
            },
        );
    }
}

#[test]
fn conditional_ice_requires_integer_result_type() {
    for expression in ["1 ? 0 : 1.0", "0 ? 1.0 : 0"] {
        with_configuration(
            &format!("int *p = {expression};"),
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert!(
                    errors.iter().any(|error| matches!(
                        error,
                        TranslationError::Semantic(error)
                            if error.kind == SemanticErrorKind::InvalidInitializer
                    )),
                    "{expression}: {errors:?}"
                );
                assert!(!unit.expressions.last().unwrap().ice, "{expression}");
            },
        );
    }
}

#[test]
fn vla_alignof_is_an_element_alignment_ice() {
    for (operator, configuration) in [
        (
            "_Alignof",
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny),
        ),
        (
            "__alignof__",
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
        ),
    ] {
        let source = format!(
            "void f(int n) {{ int a[n]; enum {{ N = {operator}(int[n]), M = {operator}(int[2][n]) \
             }}; int *p = {operator}(int[n]) == 4 ? 0 : 1; sizeof(int[n]); sizeof(a); }}"
        );
        with_configuration(&source, configuration, |context, unit| {
            assert_clean(context, unit);
            for name in ["N", "M"] {
                let binding = unit
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == name)
                    .unwrap();
                assert_eq!(binding.value.unwrap().value, 4);
            }
            let mut alignments = 0;
            for info in unit.expressions {
                if matches!(info.expression.kind, ExpressionType::AlignofType(_)) {
                    alignments += 1;
                    assert_eq!(info.integer.unwrap().value, 4);
                    assert!(info.ice);
                } else if matches!(
                    info.expression.kind,
                    ExpressionType::SizeofType(_) | ExpressionType::SizeofExpr(_)
                ) {
                    assert!(info.integer.is_none());
                    assert!(!info.ice);
                }
            }
            assert_eq!(alignments, 3);
        });
    }
}

#[test]
fn nested_subscript_type_classification_scales_linearly() {
    for n in [128, 512] {
        let source = format!(
            "int a{}; int f(void) {{ return a{}; }}\n",
            "[1]".repeat(n),
            "[0]".repeat(n)
        );
        let tu = Bump::new();
        let scratch = Bump::new();
        let mut context = Context::new(&tu);
        let unit = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<input>"),
            tu.alloc_str(&source),
            crate::headers::HeaderSearch::default(),
        );
        let mut analyzer = Analyzer::new(&mut context, &scratch);
        for &root in unit.external_declarations().iter().rev() {
            analyzer.work.push(Work::Root(root));
        }
        while let Some(work) = analyzer.work.pop() {
            analyzer.step(work);
        }
        analyzer.finish_translation_unit();
        assert_eq!(analyzer.context.pending_error_count(), 0);
        let subscripts = analyzer.expressions.iter().filter(|info| {
            matches!(
                info.expression.kind,
                ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    ..
                }
            )
        });
        assert_eq!(subscripts.count(), n);
        let last = analyzer.expressions.last().unwrap();
        assert_eq!(
            analyzer.types.nodes[last.ty.index],
            TypeKind::Scalar(Scalar::Int)
        );
        assert_eq!(
            last.category,
            super::super::expressions::ValueCategory::ModifiableLvalue
        );
        let steps = analyzer.types.steps.get();
        assert!(
            steps <= 16 * n,
            "{n} dimensions examined {steps} type nodes"
        );
    }
}

#[test]
fn static_const_bool_folding_uses_truth_conversion() {
    for (initializer, expected) in [("2", 1), ("-2", 1), ("0", 0)] {
        let source =
            format!("static const _Bool b = {initializer}; int a[b == {expected} ? 1 : -1];");
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert!(errors.is_empty(), "{source}: {errors:?}");
                assert_eq!(unit.bindings[0].value.unwrap().value, expected);
                assert!(matches!(
                    unit.types.nodes[unit.bindings[1].ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(1))
                ));
            },
        );
    }
}

#[test]
fn static_const_integer_folding_unwraps_scalar_braces() {
    for (initializer, scalar, expected) in
        [("{1}", "int", 1), ("{{1}}", "int", 1), ("{2}", "_Bool", 1)]
    {
        let source =
            format!("static const {scalar} b = {initializer}; int a[b == {expected} ? 1 : -1];");
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert!(errors.is_empty(), "{source}: {errors:?}");
                assert_eq!(unit.bindings[0].value.unwrap().value, expected);
                assert!(matches!(
                    unit.types.nodes[unit.bindings[1].ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(1))
                ));
            },
        );
    }
}

#[test]
fn unselected_comma_expression_uses_its_right_operand_integer_model() {
    for comma in ["(1UL,1)", "(1UL,(char)1)"] {
        let source = format!("enum {{N=(1 ? -1 : {comma}) < 0}}; int a[N ? 1 : -1];");
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, unit| {
                let errors = context.take_pending_errors();
                assert!(errors.is_empty(), "{source}: {errors:?}");
                assert_eq!(unit.bindings[0].value.unwrap().value, 1);
                assert!(matches!(
                    unit.types.nodes[unit.bindings[1].ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(1))
                ));
            },
        );
    }
}

#[test]
fn initializer_completed_arrays_enforce_object_size_limit_and_recover() {
    for source in [
        "int a[] = {[0x2000000000000000UL] = 1}; int b[sizeof a]; int after;",
        "int a[] = {[0x4000000000000000UL] = 1}; int b[sizeof a]; int after;",
        "char a[] = {[0x7fffffffffffffffUL] = 1}; int b[sizeof a]; int after;",
    ] {
        with_source(source, |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::ObjectTooLarge,
                    ..
                })
            ));
            assert_eq!(
                unit.types.nodes[unit.bindings[0].ty.index],
                TypeKind::Unknown
            );
            let after = unit
                .bindings
                .iter()
                .find(|binding| context.string_cache.at(binding.name.name) == "after")
                .unwrap();
            assert_eq!(
                unit.types.nodes[after.ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
        });
    }
    for (source, size) in [
        (
            "char a[] = {[0x7ffffffffffffffeUL] = 1};",
            0x7FFF_FFFF_FFFF_FFFF,
        ),
        (
            "int a[] = {[0x1ffffffffffffffeUL] = 1};",
            0x7FFF_FFFF_FFFF_FFFC,
        ),
        ("int a[] = {[3] = 1};", 16),
    ] {
        with_source(source, |context, unit| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{source}: {errors:?}");
            assert_eq!(unit.types.layout(unit.bindings[0].ty).unwrap().size, size);
        });
    }
}

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
