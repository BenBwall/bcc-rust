//! GCC vector extensions checked through the translation-unit seam.

use super::*;

#[test]
fn vector_layout_matches_clang() {
    with_source(
        include_str!("../../../../tests/fixtures/targets/vector.c"),
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let vector = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "v4i")
                .unwrap();
            assert_eq!(
                sema.types.layout(vector.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
        },
    );
}

#[test]
fn vector_operations_keep_known_types() {
    with_source(
        include_str!("../../../../tests/fixtures/targets/vector.c"),
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            for expression in sema.expressions {
                assert_ne!(
                    sema.types.nodes[expression.ty.index],
                    TypeKind::Unknown,
                    "{:?}",
                    expression.expression.kind
                );
            }
        },
    );
}

#[test]
fn vector_operand_conversions_use_input_elements() {
    with_source(
        "typedef int I __attribute__((vector_size(16))); typedef float F \
         __attribute__((vector_size(16))); void f(I a,F b) { a+b; b<1.0f; }",
        |context, sema| {
            assert_eq!(context.pending_error_count(), 0);
            let int = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "I")
                .unwrap()
                .ty;
            let float = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "F")
                .unwrap()
                .ty;
            let mut conversions = sema
                .conversions
                .iter()
                .filter(|c| c.kind == super::super::expressions::ConversionKind::Arithmetic);
            assert_eq!(conversions.next().unwrap().ty, int);
            assert_eq!(conversions.next().unwrap().ty, int);
            assert_eq!(conversions.next().unwrap().ty, float);
            assert_eq!(conversions.next().unwrap().ty, float);
            assert!(conversions.next().is_none());
        },
    );
}

#[test]
fn invalid_vector_constraints_do_not_taint_following_input() {
    with_source(
        "typedef int V __attribute__((vector_size(16))); typedef float F \
         __attribute__((vector_size(16))); void f(V v,F a,int i,long l) { v=v+l; a=a%a; v=!v; \
         v=__builtin_shufflevector(v,v,i,1,2,3); v=__builtin_convertvector(v,int); \
         __builtin_ia32_pshufd(v,i); } int following;\n",
        |context, sema| {
            assert_eq!(
                context.pending_error_count(),
                6,
                "{:?}",
                context.take_pending_errors()
            );
            let following = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "following")
                .unwrap();
            assert_eq!(
                sema.types.nodes[following.ty.index],
                TypeKind::Scalar(Scalar::Int)
            );
        },
    );
}

#[test]
fn overloaded_intrinsics_reject_invalid_element_types() {
    with_source(
        "typedef unsigned U __attribute__((vector_size(16))); struct S {int n;}; void f(U u,void \
         *p,struct S s,double _Complex c) {__builtin_elementwise_abs(u); \
         __builtin_nontemporal_load(p); __builtin_nondeterministic_value(s); \
         __builtin_elementwise_abs(c);}",
        |context, _| assert_eq!(context.pending_error_count(), 4),
    );
}

#[test]
fn vector_comparison_masks_have_natural_alignment() {
    with_source(
        "typedef int A __attribute__((vector_size(16),aligned(1))); void f(A a) {a<a;}",
        |context, sema| {
            assert_eq!(context.pending_error_count(), 0);
            let mask = sema
                .expressions
                .iter()
                .find(|e| {
                    matches!(
                        e.expression.kind,
                        ExpressionType::Binary {
                            operator: BinaryOperator::LessThan,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(
                sema.types.layout(mask.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
        },
    );
}
