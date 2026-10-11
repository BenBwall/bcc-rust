//! Stage-2 rule and non-recursive traversal regressions.

use super::{
    super::expressions::{
        ConversionKind,
        ValueCategory,
    },
    *,
};

fn accepts(source: &str) {
    with_source(source, |context, _| {
        let errors = context.take_pending_errors();
        assert!(errors.is_empty(), "{source}: {errors:?}");
    });
}

fn rejects(source: &str, expected: SemanticErrorKind) {
    with_source(source, |context, _| {
        let errors = context.take_pending_errors();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TranslationError::Semantic(e) if e.kind == expected)),
            "{source}: {errors:?}"
        );
    });
}

#[test]
fn expression_categories_and_contextual_conversions_are_retained() {
    with_source(
        "int f(int); void g(void) { const int c=1; int a[3]; int x=0; int (*p)(int)=f; x=a[1]; \
         x=p(c); sizeof a; &a; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            assert!(
                unit.expressions
                    .iter()
                    .any(|e| e.category == ValueCategory::FunctionDesignator)
            );
            assert!(
                unit.expressions
                    .iter()
                    .any(|e| e.category == ValueCategory::Lvalue)
            );
            assert!(
                unit.expressions
                    .iter()
                    .any(|e| e.category == ValueCategory::ModifiableLvalue)
            );
            for kind in [
                ConversionKind::Lvalue,
                ConversionKind::ArrayDecay,
                ConversionKind::FunctionDecay,
                ConversionKind::Assignment,
            ] {
                assert!(unit.conversions.iter().any(|c| c.kind == kind), "{kind:?}");
            }
            assert!(
                unit.expressions
                    .iter()
                    .any(|e| e.integer.is_some_and(|v| v.value == 12) && e.ice)
            );
        },
    );
}

/// C99 §6.5.3.3p5: `!E` compares E with 0. The operand keeps its own type;
/// only its lvalue conversion is recorded, as for `&&` and `||`.
#[test]
fn logical_not_records_no_arithmetic_conversion_of_its_operand() {
    with_source(
        "int f(double d, int *p, char c) { return !d + !p + !c + !0.5; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let mut negations = 0;
            for info in unit.expressions {
                let ExpressionType::Unary {
                    operator: UnaryOperator::LogicalNot,
                    operand_expression,
                } = info.expression.kind
                else {
                    continue;
                };
                negations += 1;
                assert_eq!(
                    unit.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::Int)
                );
                let operand = unit.expression_info(operand_expression).unwrap();
                let kinds = unit
                    .expression_conversions(operand_expression)
                    .iter()
                    .map(|c| (c.kind, c.ty))
                    .collect::<Vec<_>>();
                if matches!(operand_expression.kind, ExpressionType::Identifier(_)) {
                    assert_eq!(kinds, [(ConversionKind::Lvalue, operand.ty.unqualified())]);
                } else {
                    assert!(kinds.is_empty(), "{kinds:?}");
                    assert_eq!(info.integer.map(|v| v.value), Some(0), "!0.5 is 0");
                }
            }
            assert_eq!(negations, 4);
        },
    );
}

#[test]
fn every_core_operator_has_positive_coverage() {
    accepts(
        "struct S { int x; unsigned b:3; }; int f(int x, ...); void g(int n) { int a[3]={1,2,3}; \
         int *p=a; int *q=p+1; struct S s={1,2}; int x=1; x=p[0]; x=0[p]; x=s.x; x=(&s)->b; ++x; \
         x--; ++p; p--; x=+x; x=-x; x=~x; x=!p; x=*p; p=&x; x=x*2/2%2; x=x+2-1; n=q-p; x=x<<1>>1; \
         x=x<2; x=x<=2; x=x>2; x=x>=2; x=p==0; x=p!=q; x=p<q; x=x&2^3|4; x=x&&p||n; x=n?x:2L; \
         p=n?p:0; x*=2; x/=2; x%=2; x+=2; p+=1; p-=1; x-=1; x<<=1; x>>=1; x&=2; x^=2; x|=2; \
         x=(n,x); x=f(x,(float)n); x=(int)1.5; (void)x; x=((struct S){.x=4}).x; sizeof a; \
         sizeof(int[n]); if(p) x++; while(x) --x; for(;p;x++) break; switch(x) {case sizeof s: \
         break;} return; }",
    );
}

#[test]
fn arithmetic_conversion_rank_and_complex_precision() {
    with_source(
        "void f(void) { unsigned long u=1; long long s=2; u+s; (short)1+(unsigned short)2; \
         1.0f+2; 1.0L+2.0; (_Complex float)1+2.0; }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let types: Vec<_> = unit
                .expressions
                .iter()
                .filter(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Addition,
                            ..
                        }
                    )
                })
                .map(|e| unit.types.nodes[e.ty.index])
                .collect();
            assert_eq!(
                types,
                [
                    TypeKind::Scalar(Scalar::UnsignedLongLong),
                    TypeKind::Scalar(Scalar::Int),
                    TypeKind::Scalar(Scalar::Float),
                    TypeKind::Scalar(Scalar::LongDouble),
                    TypeKind::Scalar(Scalar::ComplexDouble)
                ]
            );
        },
    );
}

#[test]
fn prototype_variadic_and_unprototyped_calls() {
    with_source(
        "int f(int, const int *); int v(int,...); int old(); void g(void) { int x=0; \
         f((short)1,&x); v(1,(float)2,(char)3); old((float)2,(char)3); }",
        |context, unit| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(
                unit.conversions
                    .iter()
                    .filter(|conversion| conversion.kind == ConversionKind::DefaultArgument)
                    .count(),
                4
            );
            for (name, index, expected) in [
                ("v", 1, Scalar::Double),
                ("v", 2, Scalar::Int),
                ("old", 0, Scalar::Double),
                ("old", 1, Scalar::Int),
            ] {
                let arguments = unit
                    .expressions
                    .iter()
                    .find_map(|info| {
                        if let ExpressionType::Call {
                            function_expression,
                            arguments,
                        } = info.expression.kind
                            && let ExpressionType::Identifier(identifier) = function_expression.kind
                            && context.string_cache.at(identifier.name) == name
                        {
                            Some(arguments)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                let argument = arguments.as_slice()[index];
                let conversion = unit
                    .conversions
                    .iter()
                    .find(|conversion| {
                        std::ptr::eq(conversion.expression, argument)
                            && conversion.kind == ConversionKind::DefaultArgument
                    })
                    .unwrap_or_else(|| {
                        panic!("{name} argument {index}: missing default promotion")
                    });
                assert_eq!(
                    unit.types.nodes[conversion.ty.index],
                    TypeKind::Scalar(expected),
                    "{name} argument {index}"
                );
            }
        },
    );
    rejects(
        "int f(int); void g(void) {f();}",
        SemanticErrorKind::InvalidArgumentCount,
    );
    rejects(
        "int f(int); void g(void) {f(1,2);}",
        SemanticErrorKind::InvalidArgumentCount,
    );
    rejects(
        "int f(int *); void g(void) {f(1);}",
        SemanticErrorKind::InvalidArgumentType,
    );
    rejects(
        "void f(void) {int x=0; x();}",
        SemanticErrorKind::InvalidCall,
    );
}

#[test]
fn pointer_assignment_qualifiers_and_null_constants() {
    accepts(
        "void f(void) {int x; int *p=&x; const int *q=p; void *v=p; p=v; p=0; q=(void *)0; p=2-2; \
         _Bool b=p; const void *c=q; p=1?p:0; }",
    );
    rejects(
        "void f(void) {const int *q; int *p; p=q;}",
        SemanticErrorKind::InvalidAssignment,
    );
    rejects(
        "void f(void) {int **p; const int **q; q=p;}",
        SemanticErrorKind::InvalidAssignment,
    );
    rejects(
        "void f(void) {int *p; p=1;}",
        SemanticErrorKind::InvalidAssignment,
    );
    rejects(
        "void f(void) {int *p; p-=p;}",
        SemanticErrorKind::InvalidAdditiveOperands,
    );
}

#[test]
fn unary_and_lvalue_constraints() {
    rejects(
        "void f(void) { int *p; +p; }",
        SemanticErrorKind::InvalidUnaryOperand,
    );
    rejects(
        "void f(void) { double x; ~x; }",
        SemanticErrorKind::InvalidUnaryOperand,
    );
    rejects(
        "void f(void) {int x; *x;}",
        SemanticErrorKind::InvalidUnaryOperand,
    );
    rejects(
        "void f(void) {const int x=0; x++;}",
        SemanticErrorKind::ExpectedModifiableLvalue,
    );
    rejects(
        "struct S {const int x;}; void f(void) {struct S s={1}; s=s;}",
        SemanticErrorKind::ExpectedModifiableLvalue,
    );
    rejects(
        "void f(void) {register int x; &x;}",
        SemanticErrorKind::InvalidAddressOperand,
    );
    rejects(
        "struct S {int x:2;}; void f(void) {struct S s={1}; &s.x;}",
        SemanticErrorKind::InvalidAddressOperand,
    );
    rejects(
        "void f(void) { &(1+2); }",
        SemanticErrorKind::InvalidAddressOperand,
    );
}

#[test]
fn binary_and_conditional_constraints() {
    for (expression, kind) in [
        ("p*p", SemanticErrorKind::InvalidArithmeticOperands),
        ("x%2", SemanticErrorKind::InvalidArithmeticOperands),
        ("p+p", SemanticErrorKind::InvalidAdditiveOperands),
        ("x<<1", SemanticErrorKind::InvalidIntegerOperands),
        ("s&&1", SemanticErrorKind::InvalidLogicalOperands),
        ("p==1", SemanticErrorKind::InvalidComparisonOperands),
        ("s?1:2", SemanticErrorKind::InvalidConditionalOperands),
        ("1?p:x", SemanticErrorKind::InvalidConditionalOperands),
        ("p[x]", SemanticErrorKind::InvalidSubscript),
        ("s.missing", SemanticErrorKind::InvalidMemberAccess),
        ("p.x", SemanticErrorKind::InvalidMemberAccess),
        ("(struct S)1", SemanticErrorKind::InvalidCast),
    ] {
        rejects(
            &format!(
                "struct S {{int x;}}; void f(void) {{ int *p; double x; struct S s; {expression}; \
                 }}"
            ),
            kind,
        );
    }
}

#[test]
fn sizeof_expression_ice_vla_and_constraint_rules() {
    accepts(
        "int a[4]; enum E {N=sizeof a, M=sizeof(a[0]), K=sizeof(1+2)}; int b[N]; void f(int n) \
         {int v[n]; sizeof v;}",
    );
    rejects(
        "int f(void); int x=sizeof f;",
        SemanticErrorKind::InvalidSizeof,
    );
    rejects(
        "struct S; int x=sizeof(struct S);",
        SemanticErrorKind::InvalidSizeof,
    );
    rejects(
        "struct S {int x:2;}; void f(void) {struct S s; sizeof s.x;}",
        SemanticErrorKind::InvalidSizeof,
    );
    rejects(
        "void f(int n) {int a[n]; enum E {N=sizeof a};}",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn initializer_current_object_designators_and_extent_inference() {
    with_source(
        "struct S {int a[2]; int b;}; struct S s={1,2,3}; struct S t={.a[1]=2,3}; int \
         a[][2]={1,2,3,4}; int b[]={[4]=1,2,[1]=3}; union U {int x; double y;}; union U u={.y=2}; \
         char c[]=\"abc\"; char d[3]={\"abc\"}; int x={1};",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            for (name, size) in [("a", 16), ("b", 24), ("c", 4), ("d", 3)] {
                let binding = unit
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                assert_eq!(unit.types.layout(binding.ty).unwrap().size, size, "{name}");
            }
        },
    );
    rejects("int a[2]={1,2,3};", SemanticErrorKind::ExcessInitializer);
    rejects("int x={1,2};", SemanticErrorKind::ExcessInitializer);
    rejects("int a[2]={[2]=1};", SemanticErrorKind::InvalidDesignator);
    rejects(
        "struct S {int x;}; struct S s={.y=1};",
        SemanticErrorKind::InvalidDesignator,
    );
    rejects("int *p=1;", SemanticErrorKind::InvalidInitializer);
    rejects(
        "void f(int n) {int a[n]={1};}",
        SemanticErrorKind::InvalidInitializer,
    );
}

#[test]
fn static_storage_arithmetic_and_address_constants() {
    accepts(
        "int a[4]; struct S {int x; int y[3];}; struct S s; int f(void); int *p=&a[2]; int \
         *q=a+3; int *r=&s.y[1]; int (*fp)(void)=f; int *n=(void *)0; double d=1.0+2.0; int \
         x=(int)3.5; int z=0?1/0:3;",
    );
    rejects("int x; int y=x;", SemanticErrorKind::NonConstantInitializer);
    rejects(
        "int f(void); int x=f();",
        SemanticErrorKind::NonConstantInitializer,
    );
    rejects(
        "void f(void) {int x; static int *p=&x;}",
        SemanticErrorKind::NonConstantInitializer,
    );
    rejects(
        "int a[3]; int n; int *p=&a[n];",
        SemanticErrorKind::NonConstantInitializer,
    );
    accepts("int x; _Bool b=&x; int *a[1]; void f(void *p, void *q) {p<q;}");
    rejects("int n=1/0;", SemanticErrorKind::ConstantOverflow);
    // Annex F: a floating division by zero is an infinite constant.
    accepts("double n=1.0/0.0;");
    rejects("int i=(int)(1.0/0.0);", SemanticErrorKind::ConstantOverflow);
}

#[test]
fn floating_arithmetic_constants_preserve_precision_and_casts() {
    with_source(
        "double a=1.25*2+0.5; long double b=0x1.0000000000000002p0L-1.0L; int c=(int)(1.5+2.5); \
         int d=1.0?4:5; double e=(_Complex double)3*(_Complex double)2;",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            assert!(
                unit.expressions
                    .iter()
                    .any(|e| e.integer.is_some_and(|v| v.value == 4) && !e.ice)
            );
            assert!(
                unit.expressions
                    .iter()
                    .filter_map(|e| e.floating)
                    .any(|v| format!("{}", v.real) == "0x1p-63")
            );
        },
    );
    rejects(
        "enum E {N=(int)(1.5+2.5)};",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn const_record_arrays_and_bitfield_promotions() {
    rejects(
        "struct S {int a[2];}; void f(const struct S *p) {p->a[0]=1;}",
        SemanticErrorKind::ExpectedModifiableLvalue,
    );
    with_source(
        "struct S {unsigned b:3; unsigned all:32;}; void f(struct S s) {+s.b; +s.all;}",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let types: Vec<_> = unit
                .expressions
                .iter()
                .filter(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Unary {
                            operator: UnaryOperator::Plus,
                            ..
                        }
                    )
                })
                .map(|e| unit.types.nodes[e.ty.index])
                .collect();
            assert_eq!(
                types,
                [
                    TypeKind::Scalar(Scalar::Int),
                    TypeKind::Scalar(Scalar::UnsignedInt)
                ]
            );
        },
    );
}

#[test]
fn implicit_function_declarations_follow_the_language_mode() {
    use crate::configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
        LanguageMode,
    };
    for spelling in ["c89", "gnu89", "gnu99"] {
        let mode = LanguageMode::parse(spelling).unwrap();
        let configuration = CompilerConfiguration::new(mode.standard, ExtensionPolicy::Allow)
            .with_gnu_extensions(mode.gnu);
        with_configuration(
            "int f(void) {return undeclared(1);}",
            configuration,
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0, "{spelling}");
                assert!(unit.bindings.iter().any(|b| b.kind == BindingKind::Function
                    && context.string_cache.at(b.name.name) == "undeclared"));
            },
        );
    }
    rejects(
        "int f(void) {return undeclared(1);}",
        SemanticErrorKind::UndeclaredIdentifier,
    );
    let configuration =
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny).with_gnu_extensions(true);
    with_configuration(
        "int f(void) {return undeclared(1);}",
        configuration,
        |context, _| {
            assert!(
                context
                    .take_pending_errors()
                    .iter()
                    .any(|e| matches!(e, TranslationError::Extension(_)))
            );
        },
    );
}

#[test]
fn scalar_statement_sites_and_assertions() {
    accepts(
        "_Static_assert(sizeof(int)==4,\"int\"); void f(void) {switch(1) {case 2+3:;} if(1){} \
         while(0){} for(;1;)break;}",
    );
    rejects(
        "_Static_assert(0,\"failed\");",
        SemanticErrorKind::FailedAssertion,
    );
    rejects(
        "struct S {int x;}; void f(void) {struct S s; if(s){} }",
        SemanticErrorKind::InvalidCondition,
    );
    rejects(
        "void f(void) {switch(1.0) {} }",
        SemanticErrorKind::InvalidSwitchExpression,
    );
    rejects(
        "void f(int x) {switch(x) {case x:;} }",
        SemanticErrorKind::InvalidConstant,
    );
}

#[test]
fn undeclared_operand_suppresses_dependent_diagnostics() {
    with_source(
        "void f(void) { missing + 1; } int valid;",
        |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(
                matches!(errors[0], TranslationError::Semantic(e) if e.kind == SemanticErrorKind::UndeclaredIdentifier)
            );
            assert_eq!(unit.bindings.len(), 3);
        },
    );
}

#[test]
fn one_hundred_thousand_expression_nodes_do_not_recurse() {
    for expression in [
        format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000)),
        format!("{}1", "!".repeat(100_000)),
        format!("1{}", "+1".repeat(100_000)),
    ] {
        accepts(&format!("void f(void) {{ {expression}; }}"));
    }
}

#[test]
fn predefined_function_identifier_has_static_const_array_type() {
    accepts("int __func__; void named_function(void) {sizeof __func__; const char *s=__func__;}");
    with_source(
        "void named_function(void) {sizeof __func__;}",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            assert!(unit.expressions.iter().any(|e| matches!(
                e.expression.kind,
                ExpressionType::SizeofExpr(_)
            ) && e.integer.is_some_and(|v| v.value == 15)));
        },
    );
}

#[test]
fn mixed_floating_operands_round_before_arithmetic() {
    with_source(
        "int comparison=16777217==16777216.0f; float addition=16777217+1.0f;",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let comparison = unit
                .expressions
                .iter()
                .find(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Equal,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(comparison.integer.unwrap().value, 1);
            let addition = unit
                .expressions
                .iter()
                .find(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::Addition,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(format!("{}", addition.floating.unwrap().real), "0x1p+24");
        },
    );
}

#[test]
fn sizeof_matches_shared_linux_clang_assertions() {
    accepts(include_str!(
        "../../../../tests/fixtures/semantic/expression-sizeof-probe.c"
    ));
}
