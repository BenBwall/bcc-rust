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

fn assert_null_conversion_warning(context: &mut Context<'_>, unit: &SemanticTranslationUnit<'_>) {
    let diagnostics = context.take_pending_errors();
    assert!(
        matches!(diagnostics.as_slice(), [TranslationError::Semantic(error)]
        if error.kind == SemanticErrorKind::PointerConstraint(crate::configuration::DiagnosticGroup::NonLiteralNullConversion)),
        "{diagnostics:?}"
    );
    assert_eq!(unit.errors, 0);
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
                assert_null_conversion_warning(context, unit);
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
                            if error.kind == SemanticErrorKind::PointerConstraint(crate::configuration::DiagnosticGroup::IntConversion)
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
            assert_null_conversion_warning(context, unit);
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
        assert_eq!(last.category, ValueCategory::ModifiableLvalue);
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
                // The builtin's reserved name is no extension diagnostic
                // even under Deny, as in GCC and Clang.
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{configuration:?}: {errors:?}");
                let constraint = errors
                    .iter()
                    .find(|e| matches!(e, TranslationError::Semantic(_)))
                    .unwrap();
                assert!(matches!(
                    constraint,
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

/// GCC gives an unparenthesized `_Alignof` operand `__alignof__`'s meaning:
/// the alignment of the expression's object, storage alignment included.
/// GNU extension: GCC manual, "Determining the Alignment of Functions,
/// Types or Variables".
#[test]
fn unparenthesized_alignof_operands_take_gnu_alignment() {
    with_source(
        "struct S { char c; double d; } s; double d[3]; _Alignas(32) int big; char \
         c;\n_Static_assert(_Alignof d[0] == 8 && _Alignof (d)[1] == 8, \
         \"element\");\n_Static_assert(_Alignof big == 32 && __alignof__ big == 32, \
         \"storage\");\n_Static_assert(__alignof__ s.d == 8 && _Alignof c + 1 == 2, \"member and \
         precedence\");\n_Static_assert(__alignof__(big) == 32 && _Alignof(int) == 4, \
         \"parenthesized\");",
        |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
        },
    );
}

/// GNU's cast to a union type applies only when the cast's type is a union
/// type, through a typedef or qualified too, and the operand has a member's
/// type. A cast to a pointer to a union, as Lua's `cast(union GCUnion *, o)`,
/// is an ordinary pointer conversion that no policy reports, and an operand
/// no member matches is refused without the extension report, as GCC 14 and
/// Clang 23 do. An `__extension__` operand or declaration silences the
/// report in both. GNU extension to C99 §6.5.4 paragraph 2, p. 81; PDF p. 93.
/// <https://gcc.gnu.org/onlinedocs/gcc/Cast-to-Union.html>
#[test]
fn only_a_cast_to_a_union_type_is_a_gnu_union_cast() {
    let declarations =
        "union U { int i; double d; }; typedef union U T; typedef const union U C;\n";
    for (body, unions) in [
        ("void *p; union U *q = (union U *)p; (void)q;", 0),
        ("union U *p; union U **q = (union U **)&p; (void)q;", 0),
        ("void *p; (void)(T *)p; (void)(const union U *)p;", 0),
        ("union U a = (union U)1; T b = (T)2.0; (void)a; (void)b;", 2),
        (
            "union U a = (const union U)1; union U b = (C)1; (void)a; (void)b;",
            2,
        ),
        // `__extension__` silences the cast, and a same-type cast, on its
        // operand or on a whole declaration, as in GCC and Clang.
        (
            "union U a = __extension__ (union U)1; union U b = __extension__ (union U)a; (void)b;",
            0,
        ),
        ("union U a = __extension__ ({ (union U)1; }); (void)a;", 0),
        (
            "__extension__ union U a = (union U)1; union U b = (union U)2; (void)a; (void)b;",
            1,
        ),
    ] {
        let source = format!("{declarations}void f(void) {{ {body} }}");
        with_configuration(
            &source,
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny),
            |context, _| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), unions, "{body}: {errors:?}");
                assert!(
                    errors.iter().all(|error| matches!(
                        error,
                        TranslationError::Extension(extension)
                            if extension.spelling() == "cast to union"
                    )),
                    "{body}: {errors:?}"
                );
            },
        );
    }
    // A marked function definition silences its body; the next one is
    // reported again.
    let source = format!(
        "{declarations}__extension__ int g(int i) {{ return ((union U)i).i; }}\nint h(int i) {{ \
         return ((union U)i).i; }}"
    );
    with_configuration(
        &source,
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny),
        |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(
                    errors.as_slice(),
                    [TranslationError::Extension(extension)]
                        if extension.spelling() == "cast to union"
                ),
                "{errors:?}"
            );
        },
    );
    let source = format!("{declarations}void f(short s) {{ union U u = (union U)s; (void)u; }}");
    with_configuration(
        &source,
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny),
        |context, _| {
            let errors = context.take_pending_errors();
            assert!(
                matches!(
                    errors.as_slice(),
                    [TranslationError::Semantic(error)]
                        if error.kind == SemanticErrorKind::InvalidCast
                ),
                "{errors:?}"
            );
        },
    );
}

/// A declaration or function definition marked `__extension__` silences
/// GCC's `-pedantic` reports of folded enumerators, case labels and bit-field
/// widths, as GCC 14 does, but leaves the report of a folded array bound, and
/// of a value folded from an address, as it is unmarked. GCC makes such an
/// array variable length, and whether it then accepts it depends on the
/// array's storage and uses, and `-Wall` adds its `-Waddress` to an address
/// compared with a null pointer; this report models neither, so only an
/// `__extension__` operand enclosing the value silences it.
/// C99: §6.6 paragraph 6, p. 95; PDF p. 107; §6.7.5.2 paragraph 4, p. 116;
/// PDF p. 128. GNU extension: GCC manual, "Alternate Keywords".
#[test]
fn extension_declarations_leave_gcc_folded_array_bound_reports() {
    use crate::configuration::CompilerIdentity;
    for (source, reported) in [
        ("__extension__ static int a[(int)(1.0 + 1)];", 1),
        ("__extension__ int a[(int)(1.0 + 1)];", 1),
        ("__extension__ struct T { int a[(int)(1.0 + 1)]; };", 1),
        ("__extension__ typedef int T[(int)(1.0 + 1)];", 1),
        (
            "__extension__ static int a[sizeof(int[(int)(1.0 + 1)])];",
            1,
        ),
        ("__extension__ static int (*p)[(int)(1.0 + 1)];", 1),
        (
            "__extension__ static int *p = (int[(int)(1.0 + 1)]){1, 2};",
            1,
        ),
        (
            "void f(void) { __extension__ static int a[(int)(1.0 + 1)]; (void)a; }",
            1,
        ),
        (
            "void f(void) { __extension__ typedef int T[(int)(1.0 + 1)]; static T t; (void)t; }",
            1,
        ),
        (
            "__extension__ void f(void) { static int a[(int)(1.0 + 1)]; (void)a; }",
            1,
        ),
        (
            "void f(void) { __extension__ int *p = (int[(int)(1.0 + 1)]){1, 2}; (void)p; }",
            1,
        ),
        (
            "void f(void) { __extension__ static int n = sizeof(int[(int)(1.0 + 1)]); (void)n; }",
            1,
        ),
        // GCC accepts these variable length arrays silently, but the
        // report stays as it is unmarked.
        (
            "void f(void) { __extension__ int a[(int)(1.0 + 1)]; (void)a; }",
            1,
        ),
        (
            "void f(void) { __extension__ int n = sizeof(int[(int)(1.0 + 1)]); (void)n; }",
            1,
        ),
        (
            "__extension__ void f(void) { struct T { int a[(int)(1.0 + 1)]; } t; (void)t; }",
            1,
        ),
        (
            "__extension__ void f(int a[static (int)(1.0 + 1)]) { (void)a; }",
            1,
        ),
        // An `__extension__` operand around the bound never silenced it, but
        // one around its type name does.
        ("static int a[__extension__ ((int)(1.0 + 1))];", 1),
        (
            "static int n = __extension__ sizeof(int[(int)(1.0 + 1)]);",
            0,
        ),
        ("static int a[(int)(1.0 + 1)];", 1),
        // The other folded constants are silenced in a marked declaration.
        ("__extension__ enum { A = (int)(1.0 + 1) };", 0),
        ("__extension__ struct B { int b : (int)(1.5 * 2); };", 0),
        (
            "__extension__ int g(int i) { switch (i) { case (int)(1.0 + 1): return 1; } return 0; \
             }",
            0,
        ),
        ("enum { A = (int)(1.0 + 1) };", 1),
        // A value folded from an address is reported as it is unmarked.
        ("int x; __extension__ enum { A = &x != 0 };", 1),
        (
            "int x; __extension__ int g(int i) { switch (i) { case &x != 0: return 1; } return 0; \
             }",
            1,
        ),
    ] {
        let configuration = CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny)
            .with_gnu_extensions(true)
            .with_compiler_extensions(true)
            .with_compiler_identity(CompilerIdentity::Gcc);
        with_configuration(source, configuration, |context, _| {
            let errors = context.take_pending_errors();
            let folded = errors
                .iter()
                .filter(|error| {
                    // `-pedantic-errors` makes GCC's pedantic report an error.
                    matches!(
                        error,
                        TranslationError::Classified(classified)
                            if matches!(
                                &*classified.error,
                                TranslationError::Semantic(error)
                                    if matches!(error.kind, SemanticErrorKind::PedanticConstant(_))
                            )
                    )
                })
                .count();
            assert_eq!(
                (folded, errors.len()),
                (reported, reported),
                "{source}: {errors:?}"
            );
        });
    }
}

/// GCC accepts a cast of a structure or union to its own type, qualifiers
/// aside, as its value, an rvalue ("ISO C forbids casting nonscalar to the
/// same type" under -Wpedantic); any other aggregate cast stays invalid.
/// GNU extension to C99 §6.5.4 paragraph 2, p. 81; PDF p. 93.
#[test]
fn aggregates_cast_to_their_own_type_are_rvalues() {
    let accepted = "struct S { int a; } s; union U { int i; } u; typedef struct S T;\nconst \
                    struct S cs;\nint f(void) { struct S t = (struct S)cs; T v = (T)s; union U w \
                    = (union U)u; return t.a + v.a + w.i + ((const struct S)s).a + \
                    (int)sizeof((struct S)s); }";
    for (policy, extensions) in [
        (ExtensionPolicy::Allow, 0),
        (ExtensionPolicy::Warn, 5),
        (ExtensionPolicy::Deny, 5),
    ] {
        with_configuration(
            accepted,
            CompilerConfiguration::new(CStandard::C11, policy),
            |context, _| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), extensions, "{policy:?}: {errors:?}");
                // All five are same-type casts, `(union U)u` included.
                assert!(
                    errors.iter().all(|error| matches!(
                        error,
                        TranslationError::Extension(extension)
                            if extension.spelling()
                                == "cast of a structure or union to its own type"
                    )),
                    "{errors:?}"
                );
            },
        );
    }
    for source in [
        "struct S { int a; } s; struct R { int a; } r; void f(void) { r = (struct R)s; }",
        "struct S { int a; } s, t; void f(void) { (struct S)s = t; }",
        "struct S { int a; } s; void f(void) { struct S *p = &(struct S)s; (void)p; }",
        "struct S { int a; } s; struct S t = (struct S)s;",
    ] {
        with_source(source, |context, _| {
            assert!(context.pending_error_count() > 0, "{source}");
        });
    }
}
