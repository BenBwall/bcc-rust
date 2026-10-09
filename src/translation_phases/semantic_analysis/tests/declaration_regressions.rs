//! Regressions for declaration visibility, compatibility and recovery.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

#[test]
fn sizeof_arrays_with_inner_variable_dimensions_is_not_an_ice() {
    with_configuration(
        "void f(int n){int a[sizeof(int[3][n])]; int b[sizeof(int[n][3])];}",
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        |context, s| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{errors:?}");
            for name in ["a", "b"] {
                let binding = s
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                assert!(matches!(
                    s.types.nodes[binding.ty.index],
                    TypeKind::Array(_, ArrayBound::Variable)
                ));
            }
        },
    );
}

#[test]
fn every_array_bound_expression_requires_integer_type() {
    for source in [
        "void f(double n){int a[n * 2]; int after[3];}",
        "void f(double n){int a[-n]; int after[3];}",
        "void f(int *p){int a[p + 1]; int after[3];}",
        "int a[1.5 + 1]; int after[3];",
        "double g(void); void f(void){int a[g()]; int after[3];}",
        "void f(int n){int a[n ? 1.5 : 2]; int after[3];}",
        "void f(double n){int a[(1, n)]; int after[3];}",
        "void f(double *p){int a[p[0]]; int after[3];}",
        "struct S {double n;}; void f(struct S s){int a[s.n]; int after[3];}",
        "void f(double n){int a[n]; int after[3];}",
        "void f(int n){int a[(double)n]; int after[3];}",
        "int a[1.5]; int after[3];",
        "void f(void){int a[\"size\"]; int after[3];}",
    ] {
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, s| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                assert!(
                    matches!(
                        errors[0],
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::InvalidArrayBound,
                            ..
                        })
                    ),
                    "{source}: {errors:?}"
                );
                let after = s
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == "after")
                    .unwrap();
                assert!(matches!(
                    s.types.nodes[after.ty.index],
                    TypeKind::Array(_, ArrayBound::Constant(3))
                ));
            },
        );
    }
}

#[test]
fn integer_and_unknown_array_bound_types_preserve_existing_behavior() {
    with_configuration(
        "void f(int n, double d, int *p){int a[n * 2]; int b[-n]; int c[p - p + 1]; int \
         e[(int)d];}",
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        |context, _| {
            let errors = context.take_pending_errors();
            assert!(errors.is_empty(), "{errors:?}");
        },
    );
    with_source(
        "void f(void){int a[missing + 1]; int after[3];}",
        |context, s| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(SemanticError {
                    kind: SemanticErrorKind::UndeclaredIdentifier,
                    ..
                })
            ));
            assert!(
                s.bindings
                    .iter()
                    .any(|b| context.string_cache.at(b.name.name) == "after")
            );
        },
    );
}

#[test]
fn inline_is_invalid_on_parameter_declarations() {
    for source in [
        "int f(inline int x); int after;",
        "int f(inline int); int after;",
        "int f(inline int x){return x;} int after;",
        "int f(inline int callback(void)); int after;",
        "int f(inline int a[3]); int after;",
    ] {
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, s| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                assert!(
                    matches!(
                        errors[0],
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::InvalidInline,
                            ..
                        })
                    ),
                    "{source}: {errors:?}"
                );
                assert!(
                    s.bindings
                        .iter()
                        .any(|b| context.string_cache.at(b.name.name) == "after")
                );
            },
        );
    }
    with_source("inline int f(int x){return x;}", |context, _| {
        let errors = context.take_pending_errors();
        assert!(errors.is_empty(), "{errors:?}");
    });
}

#[test]
fn restrict_on_array_typedefs_is_validated_on_the_element() {
    for source in [
        "typedef int A[3]; restrict A a; int after;",
        "typedef int A[2][3]; restrict A a; int after;",
        "typedef int F(void); typedef F *A[3]; restrict A a; int after;",
    ] {
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
            |context, s| {
                let errors = context.take_pending_errors();
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                assert!(
                    matches!(
                        errors[0],
                        TranslationError::Semantic(SemanticError {
                            kind: SemanticErrorKind::InvalidRestrict,
                            ..
                        })
                    ),
                    "{source}: {errors:?}"
                );
                assert!(
                    s.bindings
                        .iter()
                        .any(|b| context.string_cache.at(b.name.name) == "after")
                );
            },
        );
    }
    with_source("typedef int *P[2][3]; restrict P p;", |context, s| {
        let errors = context.take_pending_errors();
        assert!(errors.is_empty(), "{errors:?}");
        let mut ty = s.bindings.last().unwrap().ty;
        for bound in [2, 3] {
            let TypeKind::Array(element, ArrayBound::Constant(n)) = s.types.nodes[ty.index] else {
                panic!("array typedef shape");
            };
            assert_eq!(n, bound);
            ty = element;
        }
        assert!(matches!(s.types.nodes[ty.index], TypeKind::Pointer(_)));
        assert!(ty.qualifiers.contains(TypeQualifiers::RESTRICT));
    });
}

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

#[test]
fn bit_field_cursor_overflow_rejects_layout_and_preserves_following_binding() {
    for source in [
        "struct S { char a[9223372036854775807]; unsigned b:1; }; int after;",
        "struct S { char a[9223372036854775807L]; unsigned :0; }; int after;",
        "struct S { char a[2305843009213693951L]; unsigned b:7; unsigned c:1; }; int after;",
        "struct S { char a[2305843009213693951L]; unsigned b:8; unsigned c:1; }; int after;",
    ] {
        with_source(source, |context, unit| {
            // The unsuffixed review input also warns about promotion to long.
            let pending = context.take_pending_errors();
            let errors = pending
                .iter()
                .filter(|e| matches!(e, TranslationError::Semantic(_)))
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                TranslationError::Semantic(e) if e.kind == SemanticErrorKind::ObjectTooLarge
            ));
            let tag = unit
                .types
                .tags
                .iter()
                .find(|t| t.kind == TagKind::Struct)
                .unwrap();
            assert!(tag.complete.get());
            assert_eq!(tag.layout.get(), None);
            assert!(
                unit.bindings
                    .iter()
                    .any(|b| context.string_cache.at(b.name.name) == "after")
            );
        });
    }
}

#[test]
fn unnamed_invalid_bit_fields_report_once_with_source_and_recover() {
    for source in [
        "struct S { void :3; }; int after;",
        "struct S { struct Missing :3; }; int after;",
        "struct S { void :missing; }; int after;",
    ] {
        with_source(source, |context, unit| {
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source}: {errors:?}");
            let TranslationError::Semantic(error) = errors[0] else {
                panic!("{errors:?}");
            };
            assert_ne!(context.get_source_vectors(error.source_vectors), []);
            let tag = unit
                .types
                .tags
                .iter()
                .find(|t| t.kind == TagKind::Struct && t.complete.get())
                .unwrap();
            assert_eq!(tag.layout.get(), None);
            assert!(
                unit.bindings
                    .iter()
                    .any(|b| context.string_cache.at(b.name.name) == "after")
            );
        });
    }
}

#[test]
fn anonymous_member_paths_use_linear_retained_storage() {
    let mut previous = None;
    for depth in [2000, 4000, 8000] {
        let source = format!(
            "struct Outer {{ {} int leaf; {} }}; struct Outer value = {{.leaf = 7}}; int f(void) \
             {{ return value.leaf; }}",
            "struct {".repeat(depth),
            "};".repeat(depth),
        );
        let tu = Bump::new();
        let source = tu.alloc_str(&format!("{source}\n"));
        let mut context = Context::with_configuration(
            &tu,
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Deny),
        );
        let parsed = crate::pipeline::parse_translation_unit(
            &mut context,
            Path::new("<input>"),
            source,
            &[],
            &[],
        );
        assert_eq!(context.pending_error_count(), 0);
        let before = tu.used();
        let unit = analyze(&mut context, &parsed);
        let retained = tu.used() - before;
        assert_eq!(
            context.pending_error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let outer = unit
            .types
            .tags
            .iter()
            .find(|t| {
                t.name
                    .is_some_and(|n| context.string_cache.at(n) == "Outer")
            })
            .unwrap();
        assert_eq!(outer.layout.get(), Some(Layout { size: 4, align: 4 }));
        assert_eq!(outer.fields.get()[0].path.len(), depth + 1);
        assert_eq!(outer.fields.get()[0].offset, 0);
        if let Some(previous) = previous {
            assert!(
                retained <= 3 * previous,
                "depth={depth}: {previous} -> {retained} retained bytes"
            );
        }
        previous = Some(retained);
    }
}

#[test]
fn anonymous_member_designators_preserve_index_order_and_offsets() {
    with_configuration(
        "struct Outer { int first; struct { char pad; union { int leaf; char alternate; }; }; int \
         last; }; struct Outer value = {.leaf = 7, .last = 9}; _Static_assert(sizeof(struct \
         Outer) == 16, \"layout\"); int f(void) { return value.leaf; }",
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow),
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let outer = unit
                .types
                .tags
                .iter()
                .find(|t| {
                    t.name
                        .is_some_and(|n| context.string_cache.at(n) == "Outer")
                })
                .unwrap();
            let leaf = outer
                .fields
                .get()
                .iter()
                .find(|f| context.string_cache.at(f.name.name) == "leaf")
                .unwrap();
            assert_eq!(leaf.offset, 8);
            assert_eq!(leaf.path.iter().collect::<Vec<_>>(), [1, 1, 0]);
        },
    );
}
