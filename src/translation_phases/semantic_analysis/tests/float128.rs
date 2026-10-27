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

/// Binary128 constants fold with IEEE binary128 arithmetic, correctly
/// rounded, as libgcc computes them at run time; conversions to other types
/// round once.
#[test]
fn binary128_arithmetic_constants_fold_exactly() {
    with_source(
        "__float128 a=1.0q/3, b=0.1q+0.2q, c=(__float128)3, d=1?0.1q:0.2q, e=-(1.0q-1.0q), \
         f=(__float128)0x1p-16400L*0x1p-90q; double narrowed=(double)(1.0q/3); int \
         truncated=(int)-7.75q; _Static_assert((int)7.75q == 7, \"cast\"); \
         _Static_assert(sizeof(int[(int)2.5q]) == 8, \"bound\"); _Complex __float128 \
         z=1.5q-2.0qi+1;",
        |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let folded = |bits: u128| {
                unit.expressions.iter().any(|info| {
                    info.binary128
                        .is_some_and(|value| value.real.to_bits() == bits && value.imag.is_zero())
                })
            };
            for bits in [
                0x3FFD_5555_5555_5555_5555_5555_5555_5555,
                0x3FFD_3333_3333_3333_3333_3333_3333_3334,
                0x4000_8000_0000_0000_0000_0000_0000_0000,
                0x3FFB_9999_9999_9999_9999_9999_9999_999A,
                0x8000_0000_0000_0000_0000_0000_0000_0000,
                // 2^-16490, a subnormal.
                0x10,
            ] {
                assert!(folded(bits), "{bits:#x}");
            }
            assert!(
                unit.expressions
                    .iter()
                    .any(|info| info.binary128.is_some_and(|value| {
                        value.real.to_bits() == 0x3FFF_8000_0000_0000_0000_0000_0000_0000
                            && value.imag.to_bits() == 0xC000_0000_0000_0000_0000_0000_0000_0000
                    }))
            );
            assert!(unit.expressions.iter().any(|info| {
                info.floating
                    .and_then(|value| value.real.x87_parts())
                    .is_some_and(|parts| parts == (false, 0x3FFD, 0xAAAA_AAAA_AAAA_A800))
            }));
            assert!(
                unit.expressions
                    .iter()
                    .any(|info| { info.integer.is_some_and(|value| value.value == -7) })
            );
            assert!(unit.expressions.iter().all(|info| !info.unfolded_binary128));
        },
    );
}

/// The product and quotient of two complex binary128 values, which
/// libgcc's `__multc3` and `__divtc3` compute, stay unfolded; so does
/// everything computed from them.
#[test]
fn complex_binary128_products_stay_unfolded() {
    with_source(
        "_Complex __float128 p=(1.0q+1.0qi)*(2.0q+1.0qi); __float128 r=__real__ \
         ((1.0q+1.0qi)/(2.0q+1.0qi));",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let unfolded: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| info.unfolded_binary128)
                .collect();
            assert!(unfolded.len() >= 3);
            for info in unfolded {
                assert_eq!(info.constant, ConstantClass::Arithmetic);
                assert!(!info.ice);
                assert!(info.binary128.is_none() && info.floating.is_none());
            }
        },
    );
}

/// GCC's binary128 constant builtins fold: infinity, quiet and signaling
/// NaNs with the tag's payload, absolute value and sign copying.
#[test]
fn binary128_builtins_fold_to_their_bits() {
    with_source(
        "__float128 i=__builtin_inff128(), h=__builtin_huge_valf128(), \
         q=__builtin_nanf128(\"5\"), s=__builtin_nansf128(\"\"), a=__builtin_fabsf128(-1.0q), \
         c=__builtin_copysignf128(1.0q, -2.0q);",
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0);
            let calls: Vec<u128> = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .filter_map(|info| info.binary128.map(|value| value.real.to_bits()))
                .collect();
            assert_eq!(
                calls,
                [
                    0x7FFF_0000_0000_0000_0000_0000_0000_0000,
                    0x7FFF_0000_0000_0000_0000_0000_0000_0000,
                    0x7FFF_8000_0000_0000_0000_0000_0000_0005,
                    0x7FFF_4000_0000_0000_0000_0000_0000_0000,
                    0x3FFF_0000_0000_0000_0000_0000_0000_0000,
                    0xBFFF_0000_0000_0000_0000_0000_0000_0000,
                ]
            );
        },
    );
}
