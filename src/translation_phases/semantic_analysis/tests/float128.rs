//! Binary128 and type-generic math behavior through the semantic pipeline.

use super::*;

#[test]
fn binary128_math_helpers_have_real_types_and_constant_eligibility() {
    with_source(
        "__float128 huge=(__float128)__builtin_huge_val(); __float128 \
         nan_value=(__float128)__builtin_nan(\"\"); __float128 \
         absolute=__builtin_fabsf128(-1.0q); __float128 sign=__builtin_copysignf128(1.0q,-2.0q);",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let calls: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .collect();
            assert_eq!(calls.len(), 4);
            for info in calls {
                assert_ne!(unit.types.nodes[info.ty.index], TypeKind::Unknown);
                assert_eq!(info.constant, ConstantClass::Arithmetic);
                assert!(!info.ice);
            }
        },
    );
    with_source(
        "__float128 f(char *tag, __float128 value) { return __builtin_nanf128(tag) + \
         __builtin_fabsf128(value); }",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            for info in unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
            {
                assert_eq!(
                    unit.types.nodes[info.ty.index],
                    TypeKind::Scalar(Scalar::Float128)
                );
                assert_eq!(info.constant, ConstantClass::None);
            }
        },
    );
}

#[test]
fn binary128_layout_rank_and_static_literals() {
    with_source(
        "__float128 value = 1.0000000000000000000000000000000002Q; _Complex __float128 \
         complex_value; __typeof__(value + 1.L) common; _Static_assert(sizeof(value) == 16, \
         \"size\"); _Static_assert(_Alignof(__float128) == 16, \"align\");",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let value = unit
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "value")
                .unwrap();
            let common = unit
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "common")
                .unwrap();
            assert_eq!(
                unit.types.layout(value.ty),
                Some(Layout {
                    size:  16,
                    align: 16,
                })
            );
            assert_eq!(value.ty, common.ty);
            assert_ne!(unit.types.nodes[value.ty.index], TypeKind::Unknown);
        },
    );
}

#[test]
fn generic_math_selection_classification_and_components() {
    with_source(
        "__float128 q; _Complex __float128 z; _Static_assert(__builtin_classify_type(q) == 8, \
         \"real\"); _Static_assert(__builtin_classify_type(z) == 9, \"complex\"); \
         _Static_assert(__builtin_types_compatible_p(const double, double), \"qualifiers\"); \
         _Static_assert(!__builtin_types_compatible_p(__float128, long double), \"distinct\"); \
         _Static_assert(_Generic(q+1.L, __float128: 1, default: 0), \"rank\"); \
         _Static_assert(_Generic(__real__ z, __float128: 1, default: 0), \"component\"); \
         _Static_assert(__imag__ 7 == 0, \"imaginary integer\"); int v; void f(void) { \
         __builtin_choose_expr(1,v,(void)0)=3; }",
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
fn binary128_arithmetic_constants_are_retained_without_approximate_folding() {
    with_source(
        "__float128 a=0.1q, b=1.0q+2.0q, c=(__float128)3, d=1?0.1q:0.2q; double \
         conditional=0.0q?1.0:2.0;",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let constants: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| info.unfolded_binary128)
                .collect();
            assert!(constants.len() >= 8);
            for info in constants {
                assert_eq!(info.constant, ConstantClass::Arithmetic);
                assert!(!info.ice);
                assert!(info.integer.is_none() && info.floating.is_none());
            }
        },
    );
}
