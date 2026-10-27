//! Documented GCC scalar intrinsic types and invalid operand diagnostics.
use super::*;

#[test]
fn builtin_value_constraints_do_not_duplicate_incompatible_type_errors() {
    with_source(
        include_str!("../../../../tests/fixtures/diagnostics/sema-builtin-values.c"),
        |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(unit.errors, 13, "{errors:?}");
            assert_eq!(
                errors
                    .iter()
                    .filter(|e| matches!(
                        e,
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::InvalidArgumentType,
                            ..
                        })
                    ))
                    .count(),
                3
            );
            assert!(
                unit.bindings
                    .iter()
                    .any(|b| context.string_cache.at(b.name.name) == "following")
            );
        },
    );
}

#[test]
fn selected_builtin_callees_preserve_intrinsic_constraints_and_shadows() {
    with_source(
        include_str!("../../../../tests/fixtures/semantic/builtin-selected-callees.c"),
        |context, unit| assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors()),
    );
    with_source(
        "void f(void *p,int \
         n){__builtin_choose_expr(1,__builtin_object_size,__builtin_object_size)(p,n);}",
        |context, unit| {
            assert_eq!(unit.errors, 1, "{:?}", context.take_pending_errors());
        },
    );
}

#[test]
fn intrinsic_only_builtin_designators_require_a_call() {
    for expression in [
        "&__builtin_clz",
        "__builtin_clz",
        "&(__builtin_choose_expr(1,__builtin_clz,__builtin_clz))",
        "_Generic(1,int:__builtin_clz,default:__builtin_ctz)",
    ] {
        with_source(
            &format!("void f(void){{{expression};}} int following;"),
            |context, unit| {
                assert_eq!(
                    unit.errors,
                    1,
                    "{expression}: {:?}",
                    context.take_pending_errors()
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
fn statement_expression_tails_produce_converted_values() {
    with_source(
        include_str!("../../../../tests/fixtures/semantic/statement-expression-values.c"),
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
            let results = unit
                .expressions
                .iter()
                .filter(|i| matches!(i.expression.kind, ExpressionType::StatementExpression(_)))
                .collect::<Vec<_>>();
            assert_eq!(results.len(), 11);
            for result in &results {
                assert_eq!(result.category, ValueCategory::Rvalue);
                assert_eq!(result.ty, result.ty.unqualified());
                assert_eq!(result.bit_field, None);
            }
            assert!(matches!(
                unit.types.nodes[results[0].ty.index],
                TypeKind::Pointer(_)
            ));
            assert!(matches!(
                unit.types.nodes[results[1].ty.index],
                TypeKind::Pointer(_)
            ));
            assert_eq!(
                unit.types.nodes[results[2].ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
            assert!(matches!(
                unit.types.nodes[results[3].ty.index],
                TypeKind::Tag(_)
            ));
            assert_eq!(
                unit.types.nodes[results[4].ty.index],
                TypeKind::Scalar(Scalar::UnsignedInt)
            );
            for result in &results[5..8] {
                assert_eq!(
                    unit.types.nodes[result.ty.index],
                    TypeKind::Scalar(Scalar::Void)
                );
            }
        },
    );
}

#[test]
fn scalar_builtin_dispatch_preserves_user_bindings() {
    with_source(
        &crate::test_support::builtin_binding_program(),
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
            assert!(
                unit.expressions
                    .iter()
                    .filter(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                    .all(|i| unit.types.nodes[i.ty.index] == TypeKind::Scalar(Scalar::Int))
            );
        },
    );
    for name in [
        "__builtin_clz",
        "__builtin_add_overflow",
        "__builtin_expect",
        "__builtin_strlen",
        "__sync_fetch_and_add",
        "__builtin_popcount",
        "__builtin_va_start",
        "__builtin_va_copy",
        "__builtin_va_end",
    ] {
        let source = format!(
            "int f(unsigned x){{return 7;}} int main(void){{int (*{name})(unsigned)=f;return \
             ({name})(1)!=7;}}"
        );
        with_source(&source, |context, s| {
            assert_eq!(s.errors, 0, "{name}: {:?}", context.take_pending_errors());
            let call = s
                .expressions
                .iter()
                .find(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                .unwrap();
            assert_eq!(s.types.nodes[call.ty.index], TypeKind::Scalar(Scalar::Int));
        });
    }
}

#[test]
fn gcc_scalar_builtin_types_follow_the_target() {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::WindowsGnu,
    ] {
        with_configuration(
            r"
unsigned short a;unsigned b;unsigned long long c;long l;void *p;__SIZE_TYPE__ size;
int main(void) {int r; a=__builtin_bswap16(1);b=__builtin_bswap32(1);c=__builtin_bswap64(1);l=__builtin_expect(1,1);p=__builtin_alloca(4);size=__builtin_object_size(p,0);r=__builtin_clz(1);return __builtin_add_overflow(1,2,&r);}
",
            crate::configuration::CompilerConfiguration::default().with_target(target),
            |context, s| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                let calls = s
                    .expressions
                    .iter()
                    .filter(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                    .map(|i| s.types.nodes[i.ty.index])
                    .collect::<Vec<_>>();
                assert_eq!(
                    &calls[..4],
                    &[
                        TypeKind::Scalar(Scalar::UnsignedShort),
                        TypeKind::Scalar(Scalar::UnsignedInt),
                        TypeKind::Scalar(if target == crate::target::Target::LinuxGnu {
                            Scalar::UnsignedLong
                        } else {
                            Scalar::UnsignedLongLong
                        }),
                        TypeKind::Scalar(Scalar::Long)
                    ]
                );
                assert!(matches!(calls[4], TypeKind::Pointer(_)));
                assert_eq!(calls[5], TypeKind::Scalar(s.types.target.size_t));
                assert_eq!(calls[6], TypeKind::Scalar(Scalar::Int));
                assert_eq!(calls[7], TypeKind::Scalar(Scalar::Bool));
            },
        );
    }
}

#[test]
fn object_size_modes_check_and_fold_the_converted_int() {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::LinuxMusl,
        crate::target::Target::WindowsGnu,
        crate::target::Target::WindowsMsvc,
    ] {
        for builtin in ["object_size", "dynamic_object_size"] {
            for (mode, expected) in [
                ("4294967296ULL", Some(-1)),
                ("4294967297ULL", Some(-1)),
                ("4294967298ULL", Some(0)),
                ("4294967299ULL", Some(0)),
                ("-4294967296LL", Some(-1)),
                ("((unsigned __int128)1 << 127) + 2", Some(0)),
                ("4294967300ULL", None),
                ("4294967295ULL", None),
                ("-1", None),
                ("4", None),
                ("n", None),
            ] {
                // An operand with side effects is not evaluated, so its
                // answer is folded now; a parameter's waits for inlining.
                for operand in ["(char *)p + n++", "p"] {
                    let source = format!(
                        "__SIZE_TYPE__ f(void *p,int n){{return \
                         __builtin_{builtin}({operand},{mode});}}"
                    );
                    with_configuration(
                        &source,
                        crate::configuration::CompilerConfiguration::default().with_target(target),
                        |context, unit| {
                            let errors = context.take_pending_errors();
                            assert_eq!(
                                unit.errors,
                                usize::from(expected.is_none()),
                                "{target:?} {source}: {errors:?}"
                            );
                            if let Some(expected) = expected {
                                let call = unit
                                    .expressions
                                    .iter()
                                    .find(|i| {
                                        matches!(i.expression.kind, ExpressionType::Call { .. })
                                    })
                                    .unwrap();
                                if operand == "p" {
                                    assert!(
                                        matches!(
                                            unit.function_extensions.object_size(call.expression),
                                            Some(ObjectSize::Deferred { mode, .. })
                                                if (mode >= 2) == (expected == 0)
                                        ),
                                        "{target:?} {source}"
                                    );
                                    assert!(call.integer.is_none(), "{target:?} {source}");
                                } else {
                                    assert_eq!(
                                        call.integer.and_then(Integer::to_u64),
                                        Some(if expected == 0 { 0 } else { u64::MAX }),
                                        "{target:?} {source}"
                                    );
                                }
                            } else {
                                assert!(
                                    matches!(
                                        errors.as_slice(),
                                        [TranslationError::Semantic(SemanticError {
                                            kind: SemanticErrorKind::InvalidObjectSizeMode,
                                            ..
                                        })]
                                    ),
                                    "{target:?} {source}: {errors:?}"
                                );
                            }
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn alignment_checks_the_converted_size_t() {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::LinuxMusl,
        crate::target::Target::WindowsGnu,
        crate::target::Target::WindowsMsvc,
    ] {
        for (alignment, accepted) in [
            ("((unsigned __int128)1 << 64) + 16", true),
            ("((unsigned __int128)1 << 127) + 16", true),
            ("-((__int128)1 << 64) + 16", true),
            ("(unsigned __int128)1 << 64", false),
            ("((unsigned __int128)1 << 64) + 3", false),
            ("-1", false),
            ("n", false),
        ] {
            let source = format!(
                "void *f(void *p,int n){{return __builtin_assume_aligned(p,{alignment},n);}}"
            );
            with_configuration(
                &source,
                crate::configuration::CompilerConfiguration::default().with_target(target),
                |context, unit| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        unit.errors,
                        usize::from(!accepted),
                        "{target:?} {source}: {errors:?}"
                    );
                    if !accepted {
                        assert!(
                            matches!(
                                errors.as_slice(),
                                [TranslationError::Semantic(SemanticError {
                                    kind: SemanticErrorKind::InvalidBuiltinAlignment,
                                    ..
                                })]
                            ),
                            "{target:?} {source}: {errors:?}"
                        );
                    }
                },
            );
        }
    }
}

#[test]
fn variadic_and_type_generic_builtin_constants_keep_their_width() {
    for (call, expected) in [
        (
            "__builtin_prefetch(p,4294967296ULL,3)",
            SemanticErrorKind::InvalidPrefetchReadWrite,
        ),
        (
            "__builtin_prefetch(p,0,4294967299ULL)",
            SemanticErrorKind::InvalidPrefetchLocality,
        ),
    ] {
        with_source(&format!("void f(void *p){{{call};}}"), |context, unit| {
            assert_eq!(unit.errors, 1, "{call}");
            let errors = context.take_pending_errors();
            assert!(
                matches!(
                    errors.as_slice(),
                    [TranslationError::Semantic(SemanticError {
                        kind,
                        ..
                    })] if *kind == expected
                ),
                "{call}: {errors:?}"
            );
        });
    }
    with_source(
        r#"
_Static_assert(__builtin_choose_expr(4294967296ULL,7,9)==7,"wide condition");
_Static_assert(__builtin_choose_expr((unsigned __int128)1<<127,7,9)==7,"unsigned condition");
_Static_assert(__builtin_constant_p(4294967296ULL),"wide ICE");
_Static_assert(!__builtin_add_overflow_p(4294967296ULL,1,0ULL),"wide destination");
_Static_assert(__builtin_add_overflow_p(4294967296ULL,1,0u),"narrow destination");
void f(void *p,int n) { __builtin_assume_aligned(p,16,n); __builtin_prefetch(p,0ULL,3ULL); }
"#,
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
        },
    );
}

#[test]
fn expect_probability_checks_the_converted_double_endpoints() {
    // Float-to-double promotion is exact: these float and double cases
    // round at source precision, whereas long double rounds at the call.
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::LinuxMusl,
        crate::target::Target::WindowsGnu,
        crate::target::Target::WindowsMsvc,
    ] {
        for (probability, accepted) in [
            ("(float)(1.0L + 0x1p-25L)", true),
            ("(float)-0x1p-150L", true),
            ("1.0f + 0x1p-23f", false),
            ("-0x1p-149f", false),
            ("(double)(1.0L + 0x1p-54L)", true),
            ("(double)-0x1p-1075L", true),
            ("1.0 + 0x1p-52", false),
            ("-0x1p-1074", false),
            ("1.0L + 0x1p-60L", true),
            ("-0x1p-1075L", true),
            ("1.0L + 0x1p-52L", false),
            ("-0x1p-1074L", false),
        ] {
            let source = format!(
                "long f(long x){{return __builtin_expect_with_probability(x,1,{probability});}}"
            );
            with_configuration(
                &source,
                crate::configuration::CompilerConfiguration::default().with_target(target),
                |context, unit| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        unit.errors,
                        usize::from(!accepted),
                        "{target:?} {probability}: {errors:?}"
                    );
                    if !accepted {
                        assert!(
                            matches!(
                                errors.as_slice(),
                                [TranslationError::Semantic(SemanticError {
                                    kind: SemanticErrorKind::InvalidBuiltinProbability,
                                    ..
                                })]
                            ),
                            "{target:?} {probability}: {errors:?}"
                        );
                    }
                },
            );
        }
    }
}

#[test]
fn gcc_scalar_builtin_counts_types_and_immediates_are_checked() {
    for (call, expected) in [
        ("__builtin_clz()", SemanticErrorKind::InvalidArgumentCount),
        (
            "__builtin_clzll(1,2)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_bswap16(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        ("__builtin_ctz(&r)", SemanticErrorKind::InvalidArgumentType),
        (
            "__builtin_popcount(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_parity(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        ("__builtin_ffs(&r)", SemanticErrorKind::InvalidArgumentType),
        (
            "__builtin_clrsb(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_expect(1)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_expect_with_probability(1,1,1.5)",
            SemanticErrorKind::InvalidBuiltinProbability,
        ),
        (
            "__builtin_assume_aligned(&r,3)",
            SemanticErrorKind::InvalidBuiltinAlignment,
        ),
        (
            "__builtin_unreachable(1)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        ("__builtin_trap(1)", SemanticErrorKind::InvalidArgumentCount),
        (
            "__builtin_prefetch(&r,2,3)",
            SemanticErrorKind::InvalidPrefetchReadWrite,
        ),
        (
            "__builtin_prefetch(&r,0,4)",
            SemanticErrorKind::InvalidPrefetchLocality,
        ),
        (
            "__builtin_prefetch(1.0)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_constant_p()",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_object_size(&r,4)",
            SemanticErrorKind::InvalidObjectSizeMode,
        ),
        (
            "__builtin_dynamic_object_size(&r,r)",
            SemanticErrorKind::InvalidObjectSizeMode,
        ),
        (
            "__builtin_alloca(&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_add_overflow(1,2)",
            SemanticErrorKind::InvalidArgumentCount,
        ),
        (
            "__builtin_sub_overflow(1,2,&b)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow(1.0,2,&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_add_overflow(1,2,&e)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow(1,2,&c)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_uadd_overflow(1,2,&r)",
            SemanticErrorKind::InvalidArgumentType,
        ),
        (
            "__builtin_mul_overflow_p(1,2,b)",
            SemanticErrorKind::InvalidArgumentType,
        ),
    ] {
        let source = format!("void f(void) {{int r;_Bool b;enum E {{A}} e;const int c=0;{call};}}");
        with_source(&source, |context, _| {
            let errors = context.take_pending_errors();
            assert!(errors.iter().any(|error|matches!(error,TranslationError::Semantic(SemanticError {kind,..}) if *kind==expected)),"{call}: {errors:?}");
        });
    }
}

#[test]
fn gcc_scalar_constant_queries_and_overflow_predicates_are_ices() {
    with_source(
        r#"
_Static_assert(__builtin_constant_p(1+2),"ICE");
_Static_assert(__builtin_constant_p(1.0),"floating constant");
_Static_assert(__builtin_add_overflow_p(127,1,(signed char)0),"narrow");
_Static_assert(!__builtin_add_overflow_p(-1,1u,0u),"mixed");
_Static_assert(__builtin_sub_overflow_p(0u,1u,0u),"negative");
_Static_assert(__builtin_mul_overflow_p((unsigned __int128)1<<127,2,(unsigned __int128)0),"wide");
int r; volatile int v;
_Static_assert(!__builtin_add_overflow_p(1,2,r),"ignored lvalue");
_Static_assert(!__builtin_add_overflow_p(1,2,(__INTPTR_TYPE__)&v),"ignored address");
int main(void){return 0;}
"#,
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

#[test]
fn gcc_scalar_builtin_policy_uses_the_shared_extension_gate() {
    with_configuration(
        "int f(unsigned x){return __builtin_clz(x);}",
        crate::configuration::CompilerConfiguration::new(
            CStandard::C99,
            crate::configuration::ExtensionPolicy::Deny,
        ),
        |context, _| {
            assert!(context.pending_error_count() > 0);
        },
    );
}

#[test]
fn user_alloca_declarations_keep_their_type_in_strict_and_gnu_modes() {
    for gnu in [false, true] {
        let config = crate::configuration::CompilerConfiguration::new(
            CStandard::C99,
            crate::configuration::ExtensionPolicy::Allow,
        )
        .with_gnu_extensions(gnu);
        for source in [
            "int alloca(int x){return x+1;} int main(void){return alloca(1)!=2;}",
            "int calls; void *alloca(__SIZE_TYPE__) __asm__(\"my_alloca\"); void \
             *alloca(__SIZE_TYPE__ n){++calls;return &calls;} int main(void){return \
             alloca(7)!=&calls||calls!=1;}",
        ] {
            with_configuration(source, config, |context, unit| {
                assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
                let callee = unit.expressions.iter().find(|i| matches!(i.expression.kind,ExpressionType::Identifier(name) if context.string_cache.at(name.name)=="alloca")).unwrap();
                assert!(!unit.bindings[callee.binding.unwrap()].origin.is_intrinsic());
            });
        }
    }
}

#[test]
fn builtin_spellings_respect_function_typedef_and_enumerator_bindings() {
    with_source(
        "int __builtin_trap(int); int f(void){return __builtin_trap(7);}",
        |context, unit| {
            assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
            let call = unit
                .expressions
                .iter()
                .find(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                .unwrap();
            assert_eq!(
                unit.types.nodes[call.ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
        },
    );
    with_source(
        "enum E {__builtin_clz=0}; int f(void){return __builtin_clz(1);}",
        |context, _| {
            assert!(context.take_pending_errors().iter().any(|e| matches!(
                e,
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::InvalidCall,
                    ..
                })
            )));
        },
    );
    with_source(
        "typedef int __builtin_clz; int f(void){return __builtin_clz(1);}",
        |_, unit| {
            let call = unit
                .expressions
                .iter()
                .find(|i| matches!(i.expression.kind, ExpressionType::Call { .. }))
                .unwrap();
            assert!(unit.types.unanalyzed(call.ty));
        },
    );
}
