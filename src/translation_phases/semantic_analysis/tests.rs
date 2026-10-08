//! Tests for the declaration semantic phase, C99 §6.2 and §6.7.

use std::path::Path;

use super::*;

fn with_source(source: &str, run: impl FnOnce(&mut Context<'_>, &SemanticTranslationUnit<'_>)) {
    with_configuration(
        source,
        crate::configuration::CompilerConfiguration::default(),
        run,
    );
}

fn with_configuration(
    source: &str,
    configuration: crate::configuration::CompilerConfiguration,
    run: impl FnOnce(&mut Context<'_>, &SemanticTranslationUnit<'_>),
) {
    let tu = Bump::new();
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::with_configuration(&tu, configuration);
    let unit = crate::pipeline::parse_translation_unit(
        &mut context,
        Path::new("<input>"),
        source,
        &[],
        &[],
    );
    let semantic = analyze(&mut context, &unit);
    run(&mut context, &semantic);
}

#[test]
fn canonical_types_and_declarator_precedence() {
    with_source(
        "int a; signed int b; int *p[3]; int (*q)[3]; int f(void);",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(s.bindings[0].ty, s.bindings[1].ty);
            assert!(matches!(
                s.types.nodes[s.bindings[2].ty.index],
                TypeKind::Array(_, ArrayBound::Constant(3))
            ));
            assert!(matches!(
                s.types.nodes[s.bindings[3].ty.index],
                TypeKind::Pointer(_)
            ));
            assert_eq!(
                s.types.layout(s.bindings[2].ty),
                Some(Layout {
                    size:  24,
                    align: 8,
                })
            );
        },
    );
}

#[test]
fn enum_constants_array_sizes_and_layout() {
    with_source(
        "enum E { A=2, B=A*3, C=0 ? 1/0 : B+1 }; struct S {char c; int i; short s;}; int a[C];",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let tag = s
                .types
                .tags
                .iter()
                .find(|t| t.kind == TagKind::Struct)
                .unwrap();
            assert_eq!(
                tag.layout.get(),
                Some(Layout {
                    size:  12,
                    align: 4,
                })
            );
            assert_eq!(
                tag.members
                    .get()
                    .iter()
                    .map(|m| m.offset)
                    .collect::<Vec<_>>(),
                [0, 4, 8]
            );
            assert_eq!(
                s.types.layout(s.bindings.last().unwrap().ty),
                Some(Layout {
                    size:  28,
                    align: 4,
                })
            );
        },
    );
}

#[test]
fn linkage_shadowing_and_composites() {
    with_source(
        "static int x; extern int x; extern int a[]; int a[4]; void f(int p) {int x; {int x;} \
         extern int a[4];}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(s.bindings[1].linkage, Linkage::Internal);
            assert_eq!(
                s.types.layout(s.bindings[3].ty),
                Some(Layout {
                    size:  16,
                    align: 4,
                })
            );
            assert_eq!(
                s.bindings
                    .iter()
                    .filter(|b| b.kind == BindingKind::Parameter
                        && s.scopes[b.scope].kind == ScopeKind::Function)
                    .count(),
                1
            );
        },
    );
}

#[test]
fn constraints_are_structured_and_source_backed() {
    with_source(
        "int x; double x; void f(void) {int y; int y;} enum E {A=2147483647,B};",
        |context, _| {
            let kinds = context
                .take_pending_errors()
                .iter()
                .filter_map(|e| match e {
                    | TranslationError::Semantic(e) => Some(e.kind),
                    | _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                kinds,
                [
                    SemanticErrorKind::IncompatibleDeclaration,
                    SemanticErrorKind::DuplicateDeclaration,
                    SemanticErrorKind::EnumeratorRange
                ]
            );
        },
    );
}

#[test]
fn deeply_nested_pointer_and_blocks_do_not_recurse() {
    let pointers = format!("int {}p;", "*".repeat(100_000));
    with_source(&pointers, |context, s| {
        assert_eq!(context.pending_error_count(), 0);
        assert_eq!(
            s.types.layout(s.bindings[0].ty),
            Some(Layout { size: 8, align: 8 })
        );
    });
    let blocks = format!(
        "void f(void) {{{}int x;{}}}",
        "{".repeat(10_000),
        "}".repeat(10_000)
    );
    with_source(&blocks, |context, s| {
        assert_eq!(context.pending_error_count(), 0);
        assert!(s.scopes.len() > 10_000);
    });
}

#[test]
fn interning_compatibility_and_layout_are_iterative() {
    let tu = Bump::new();
    let scratch = Bump::new();
    let mut types = TypeInterner::new(&tu, &scratch);
    let int = types.scalar(Scalar::Int);
    let long = types.scalar(Scalar::Long);
    let a = types.intern(TypeKind::Array(int, ArrayBound::Incomplete));
    let b = types.intern(TypeKind::Array(int, ArrayBound::Constant(3)));
    assert_eq!(types.composite(a, b), Some(b));
    assert_eq!(types.composite(int, long), None);
    let mut x = a;
    let mut y = b;
    for _ in 0..100_000 {
        x = types.intern(TypeKind::Pointer(x));
        y = types.intern(TypeKind::Pointer(y));
    }
    assert_eq!(types.composite(x, y), Some(y));
}

#[test]
fn integer_operations_apply_width_signedness_and_short_circuit_rules() {
    assert_eq!(Integer::int(i128::from(i32::MAX)).increment(), None);
    assert_eq!(
        Integer {
            value:  i128::from(u32::MAX),
            bits:   32,
            signed: false,
        }
        .binary(BinaryOperator::Addition, Integer::int(1))
        .unwrap()
        .value,
        0
    );
    assert_eq!(
        Integer::int(1).binary(BinaryOperator::LeftShift, Integer::int(32)),
        None
    );
    assert_eq!(Integer::int(-1).cast(32, false).value, i128::from(u32::MAX));
}

#[test]
fn integer_division_truncates_toward_zero_across_the_64_bit_range() {
    let divide = |op, l, r| Integer::int(l).binary(op, Integer::int(r)).unwrap().value;
    for (l, r, quotient, remainder) in [
        (7, 2, 3, 1),
        (-7, 2, -3, -1),
        (7, -2, -3, 1),
        (-7, -2, 3, -1),
    ] {
        assert_eq!(divide(BinaryOperator::Division, l, r), quotient);
        assert_eq!(divide(BinaryOperator::Modulo, l, r), remainder);
    }
    let unsigned_max = Integer {
        value:  i128::from(u64::MAX),
        bits:   64,
        signed: false,
    };
    let three = Integer {
        value:  3,
        bits:   64,
        signed: false,
    };
    assert_eq!(
        unsigned_max
            .binary(BinaryOperator::Division, three)
            .unwrap()
            .value,
        i128::from(u64::MAX / 3)
    );
    let long_min = Integer {
        value:  i128::from(i64::MIN),
        bits:   64,
        signed: true,
    };
    assert_eq!(
        long_min
            .binary(BinaryOperator::Modulo, Integer::int(3))
            .unwrap()
            .value,
        i128::from(i64::MIN % 3)
    );
    assert_eq!(
        long_min.binary(BinaryOperator::Division, Integer::int(-1)),
        None
    );
}

#[test]
fn casts_conditional_conversions_and_array_qualification() {
    with_source(
        "enum E {A=(int)1.75, B=(int)2.5L, C=1?-1:0U}; typedef int A[2]; const A a;",
        |context, s| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.iter().filter(|e|matches!(e,TranslationError::Semantic(e) if e.kind==SemanticErrorKind::EnumeratorRange)).count(),1);
            assert_eq!(s.bindings[0].value.unwrap().value, 1);
            assert_eq!(s.bindings[1].value.unwrap().value, 2);
            let ty = s.bindings.last().unwrap().ty;
            let TypeKind::Array(element, _) = s.types.nodes[ty.index] else {
                panic!("qualified array typedef");
            };
            assert!(element.qualifiers.contains(TypeQualifiers::CONST));
        },
    );
}

#[test]
fn sysv_aggregate_layout_matches_linux_clang_probes() {
    with_source(
        "struct Basic {char c; int i; short s;}; struct Nested {char c; struct Basic s; double \
         d;}; union Choice {char c[9]; long l; double d;}; struct Bits {char c; unsigned a:3; \
         unsigned b:5; unsigned :0; short s;}; struct Mixed {unsigned char a:4; unsigned b:4; \
         char c;}; struct Flexible {char c; long n; int tail[];};",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(
                s.types
                    .tags
                    .iter()
                    .map(|t| t.layout.get().unwrap())
                    .collect::<Vec<_>>(),
                [
                    Layout {
                        size:  12,
                        align: 4,
                    },
                    Layout {
                        size:  24,
                        align: 8,
                    },
                    Layout {
                        size:  16,
                        align: 8,
                    },
                    Layout { size: 8, align: 4 },
                    Layout { size: 4, align: 4 },
                    Layout {
                        size:  16,
                        align: 8,
                    }
                ]
            );
            assert_eq!(s.types.tags[1].members.get()[2].offset, 16);
            assert_eq!(s.types.tags[3].members.get()[4].offset, 4);
            assert_eq!(s.types.tags[4].members.get()[2].offset, 1);
            assert_eq!(s.types.tags[5].members.get()[2].offset, 16);
        },
    );
}

#[test]
fn parameter_tags_and_enumerators_reach_definition_bodies() {
    with_source(
        "void f(enum E {A=3} p) {int x[A]; struct Local {int a;}; struct Local l;}",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let array = s
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "x")
                .unwrap();
            assert_eq!(
                s.types.layout(array.ty),
                Some(Layout {
                    size:  12,
                    align: 4,
                })
            );
        },
    );
}

#[test]
fn every_source_prefix_recovers_and_terminates() {
    let source = "typedef int T; struct S {T a; unsigned b:3;}; enum E {A=1,B=A+2}; int f(T \
                  p[static 3]) {for(int i=0;i<3;i++){T v[B];} return p[0];}";
    for end in 0..=source.len() {
        with_source(&source[..end], |_, _| {});
    }
}

#[test]
fn old_style_definitions_merge_promoted_parameters() {
    with_source(
        "int f(int, double); int f(a,b) short a; float b; {return a;} int f();",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let f = s.bindings.last().unwrap();
            let TypeKind::Function {
                parameters,
                prototype,
                ..
            } = s.types.nodes[f.ty.index]
            else {
                panic!("function type")
            };
            assert!(prototype);
            assert_eq!(parameters.len(), 2);
            assert!(matches!(
                s.types.nodes[parameters[1].index],
                TypeKind::Scalar(Scalar::Double)
            ));
        },
    );
    with_source(
        "int f(double); int f(a) int a; {return a;}",
        |context, _| {
            assert_eq!(context.pending_error_count(), 1);
        },
    );
}

#[test]
fn outermost_parameter_arrays_and_flexible_member_constraints() {
    with_source(
        "void f(int a[static const 3][4]); extern void x; enum E {A=(int)(2.5)};",
        |context, _| {
            assert_eq!(context.pending_error_count(), 0);
        },
    );
    with_source(
        "void x; void f(int a[3][static 4]); struct S {int a; int b[];}; struct T {struct S s;}; \
         struct S a[2];",
        |context, _| {
            assert_eq!(context.pending_error_count(), 4);
        },
    );
}

#[test]
fn missing_typedef_binding_is_defensive_and_structured() {
    let tu = Bump::new();
    let scratch = Bump::new();
    let mut context = Context::new(&tu);
    let name = Identifier::new(
        context.string_cache.intern("missing"),
        SourceVectors::empty(),
    );
    {
        let mut analyzer = Analyzer::new(&mut context, &scratch);
        analyzer.resolve_spec(
            TypeSpecifiers::TypedefName(name),
            TypeQualifiers::empty(),
            SourceVectors::empty(),
            false,
        );
        while let Some(work) = analyzer.work.pop() {
            analyzer.step(work);
        }
        assert_eq!(analyzer.take_type(), analyzer.types.unknown());
    }
    let errors = context.take_pending_errors();
    assert!(matches!(
        errors[0],
        TranslationError::Semantic(SemanticError {
            kind: SemanticErrorKind::UnknownTypedef,
            ..
        })
    ));
}

#[test]
fn unmodeled_extensions_suppress_dependent_constraints() {
    with_source(
        "int a[3uwb]; _Atomic(int) atomic; int b[sizeof(_Atomic(int))];",
        |context, _| {
            assert!(
                !context
                    .take_pending_errors()
                    .iter()
                    .any(|e| matches!(e, TranslationError::Semantic(_)))
            );
        },
    );
}

#[test]
fn unsigned_constant_multiplication_wraps_at_target_width() {
    with_source(
        "enum E {A=18446744073709551615ULL*18446744073709551615ULL}; int a[A];",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(
                s.types.layout(s.bindings.last().unwrap().ty),
                Some(Layout { size: 4, align: 4 })
            );
        },
    );
}

#[test]
fn aggregate_attributes_do_not_fabricate_compatibility_or_layout() {
    with_source(
        "enum __attribute__((packed)) E {A}; extern enum E x; extern unsigned char x;",
        |context, s| {
            assert!(
                !context
                    .take_pending_errors()
                    .iter()
                    .any(|e| matches!(e, TranslationError::Semantic(_)))
            );
            assert!(s.types.tags[0].layout.get().is_none());
        },
    );
}

#[test]
fn deeply_nested_conditional_constants_use_cached_models() {
    let source = format!("enum E {{A={}1}}; int a[A];", "0?0:".repeat(25_000));
    with_source(&source, |context, _| {
        assert_eq!(context.pending_error_count(), 0);
    });
}

#[test]
fn unselected_type_names_declare_tags_and_enumerators() {
    with_source(
        "enum E {A=0?(enum X {Q=3})1:2, B=Q, C=(1?(enum X)-1:0UL)<0}; int a[B];",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            assert_eq!(
                s.bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == "C")
                    .unwrap()
                    .value
                    .unwrap()
                    .value,
                0
            );
            assert_eq!(
                s.types.layout(s.bindings.last().unwrap().ty),
                Some(Layout {
                    size:  12,
                    align: 4,
                })
            );
        },
    );
}

#[test]
fn opaque_type_operands_preserve_nested_declaration_names() {
    with_source(
        "_Atomic(enum E {A=3}) atom; int a[A]; enum F : unsigned long {B=3}; int b[B];",
        |context, s| {
            assert!(
                !context
                    .take_pending_errors()
                    .iter()
                    .any(|e| matches!(e, TranslationError::Semantic(_)))
            );
            let a = s
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "a")
                .unwrap();
            assert_eq!(
                s.types.layout(a.ty),
                Some(Layout {
                    size:  12,
                    align: 4,
                })
            );
            assert!(s.types.tags[1].layout.get().is_none());
        },
    );
}

#[test]
fn enum_composites_retain_nominal_identity() {
    with_source(
        "enum E {A}; enum F {B}; extern enum E e; extern int e; extern enum F e;",
        |context, _| {
            let errors = context.take_pending_errors();
            assert_eq!(
                errors
                    .iter()
                    .filter(|e| matches!(e, TranslationError::Semantic(_)))
                    .count(),
                1
            );
        },
    );
}

#[test]
fn function_typedef_qualifiers_warn_without_qualifying_function() {
    with_source(
        "typedef int Function(void); const Function f; Function *const p;",
        |context, s| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1);
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::QualifiedFunction,
                    ..
                })
            ));
            assert_eq!(s.bindings[0].ty, s.bindings[1].ty);
            assert!(s.bindings[2].ty.qualifiers.contains(TypeQualifiers::CONST));
        },
    );
}

#[test]
fn gnu_void_and_function_size_constants_remain_accepted() {
    with_configuration(
        "int a[sizeof(void)]; int b[sizeof(int(void))];",
        crate::configuration::CompilerConfiguration::default().with_gnu_extensions(true),
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            for binding in s.bindings {
                assert_eq!(
                    s.types.layout(binding.ty),
                    Some(Layout { size: 4, align: 4 })
                );
            }
        },
    );
}
