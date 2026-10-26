//! The `_FloatN` and `_FloatNx` types of C23 Annex H behind
//! `CompilerConfiguration::with_interchange_float_types`: distinct types with
//! the representations of the standard types, their constants, conversions,
//! predefined characteristics and builtins.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
};

fn interchange(standard: CStandard, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, policy).with_interchange_float_types(true)
}

fn messages(context: &mut Context<'_>) -> Vec<String> {
    context
        .take_pending_errors()
        .iter()
        .map(|error| format!("{error:?}"))
        .collect()
}

/// C23 §H.4.3p1: within one precision an interchange type is chosen over
/// the standard type, which is chosen over the extended type.
#[test]
fn interchange_types_are_distinct_and_follow_annex_h_conversions() {
    let source = r#"
#define T(x) _Generic((x), float: 1, double: 2, long double: 3, _Float32: 4, \
    _Float64: 5, _Float32x: 6, _Float64x: 7, _Float128: 8, default: 0)
_Float32 f32; float f; double d; _Float64 f64; _Float32x f32x; _Float64x f64x;
long double ld; _Float128 f128;
_Static_assert(T(f32) == 4 && T(f64) == 5 && T(f32x) == 6 && T(f64x) == 7, "types");
_Static_assert(T(f32 + f) == 4 && T(f + f32) == 4, "interchange over standard");
_Static_assert(T(f64 + d) == 5 && T(f32x + d) == 2, "standard over extended");
_Static_assert(T(f64x + ld) == 3 && T(f32 + f32x) == 6, "precision first");
_Static_assert(T(f32x + f64) == 5 && T(f64x + f64) == 7 && T(f64x + f128) == 8, "rank");
_Static_assert(T(+f32) == 4 && T(f32 * 2) == 4, "no promotion");
_Static_assert(T(1.0f32) == 4 && T(1.0F64) == 5 && T(1.0f32x) == 6 && T(1.0F64x) == 7, "suffixes");
_Static_assert(T(1.0f128) == 8 && T(0x1p3f32) == 4, "binary128 and hexadecimal constants");
_Static_assert(__builtin_types_compatible_p(_Float128, __float128), "one binary128 type");
_Static_assert(!__builtin_types_compatible_p(_Float32, float), "distinct from float");
_Static_assert(!__builtin_types_compatible_p(_Float64, _Float32x), "distinct from each other");
_Static_assert(sizeof(_Float32) == 4 && _Alignof(_Float64) == 8 && sizeof(_Float64x) == 16, "layout");
_Static_assert(sizeof(_Complex _Float32) == 8 && sizeof(_Float64 _Complex) == 16, "complex");
_Static_assert((int)2.75f64 == 2 && (int)1.5f32 == 1 && (int)3.5f64x == 3, "integer casts fold");
static _Float32 initialized = 2.5f32 * 2;
static int folded = (int)(1.5f32 + 1.0f32);
"#;
    with_configuration(
        source,
        interchange(CStandard::C11, ExtensionPolicy::Allow),
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
            let f32 = unit
                .bindings
                .iter()
                .find(|binding| context.string_cache.at(binding.name.name) == "f32")
                .unwrap();
            assert_eq!(
                unit.types.nodes[f32.ty.index],
                TypeKind::Scalar(Scalar::Float32)
            );
        },
    );
}

#[test]
fn interchange_builtins_and_characteristics_follow_the_switch() {
    let source = r#"
_Float32 inf = __builtin_inff32();
_Float64 huge = __builtin_huge_valf64();
_Float32x nan = __builtin_nanf32x("");
_Float64x signaling = __builtin_nansf64x("");
#if !__has_builtin(__builtin_inff32) || __FLT32_MANT_DIG__ != 24 || __FLT64X_MANT_DIG__ != 64
#error characteristics
#endif
#if __FLT128_MAX_EXP__ != 16384 || __FLT32X_DIG__ != 15
#error characteristics
#endif
static _Float32 maximum = __FLT32_MAX__;
_Static_assert(_Generic(__FLT64_EPSILON__, _Float64: 1, default: 0), "suffixed values");
"#;
    with_configuration(
        source,
        interchange(CStandard::C17, ExtensionPolicy::Allow),
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
            for info in unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
            {
                assert!(
                    matches!(unit.types.nodes[info.ty.index], TypeKind::Scalar(s) if s.floating())
                );
                assert_eq!(info.constant, ConstantClass::Arithmetic);
            }
        },
    );
}

/// Under the default Clang identity glibc declares these names as typedefs,
/// so they stay identifiers, as do the builtins and macros.
#[test]
fn without_the_switch_the_names_stay_identifiers() {
    let source = r#"
typedef float _Float32;
typedef double _Float64;
typedef __float128 _Float128;
_Float32 x;
_Static_assert(__builtin_types_compatible_p(_Float32, float), "glibc typedef");
#if __has_builtin(__builtin_inff32) || defined __FLT32_MANT_DIG__
#error not a GCC identity
#endif
"#;
    with_source(source, |context, _| {
        assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
    });
    with_source("double x = 1.0f32;", |context, _| {
        let errors = messages(context);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("InvalidDecimalFloatLiteral")),
            "{errors:?}"
        );
    });
}

/// GCC: "ISO C does not support the '_Float32' type before C23".
#[test]
fn pedantic_modes_report_the_types_before_c23() {
    let source = "_Float32 x = 1.0f32; _Float64x y; _Float128 z = 1.0f128; double w = 0x1p128;";
    with_configuration(
        source,
        interchange(CStandard::C11, ExtensionPolicy::Warn),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 5, "{errors:?}");
            assert!(
                errors.iter().all(|error| error.contains("C23")),
                "{errors:?}"
            );
        },
    );
    with_configuration(
        source,
        interchange(CStandard::C23, ExtensionPolicy::Deny),
        |context, _| {
            assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
        },
    );
}

/// `_Float64x` needs a format wider than binary64 and `_Float128` binary128;
/// the MSVC target has neither as a type.
#[test]
fn msvc_target_lacks_the_wide_types() {
    for source in ["_Float64x x;", "_Float128 x;"] {
        with_configuration(
            source,
            interchange(CStandard::C17, ExtensionPolicy::Allow).with_target(Target::WindowsMsvc),
            |context, _| {
                let errors = messages(context);
                assert!(
                    errors
                        .iter()
                        .any(|error| error.contains("UnsupportedInterchangeFloat")),
                    "{source}: {errors:?}"
                );
            },
        );
    }
    with_configuration(
        "_Float32 a; _Float64 b; _Float32x c;\n#ifdef __FLT64X_MANT_DIG__\n#error no \
         _Float64x\n#endif",
        interchange(CStandard::C17, ExtensionPolicy::Allow).with_target(Target::WindowsMsvc),
        |context, _| {
            assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
        },
    );
}

/// C23 §H.4.3p1 compares sets of values: where `long double` is binary64
/// (the MSVC target), `_Float64` is chosen over it and it over `_Float32x`,
/// as GCC chooses with `-mlong-double-64`.
#[test]
fn conversions_follow_the_target_formats() {
    let source = r#"
#define T(x) _Generic((x), double: 2, long double: 3, _Float64: 5, _Float32x: 6, default: 0)
_Float64 f64; long double ld; _Float32x f32x; double d;
_Static_assert(T(f64 + ld) == 5 && T(ld + f64) == 5, "interchange over long double");
_Static_assert(T(f32x + ld) == 3 && T(d + ld) == 3 && T(f32x + d) == 2, "standard over extended");
"#;
    with_configuration(
        source,
        interchange(CStandard::C17, ExtensionPolicy::Allow).with_target(Target::WindowsMsvc),
        |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
    );
    // With the x87 `long double` it is the wider type.
    with_configuration(
        "_Static_assert(_Generic((_Float64)0 + (long double)0, long double: 1, default: 0), \
         \"x87\");",
        interchange(CStandard::C17, ExtensionPolicy::Allow),
        |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
    );
}

/// Only a lowercase `x` ends an extended suffix: GCC rejects `f32X`.
#[test]
fn extended_suffixes_take_a_lowercase_x() {
    for source in ["double a = 1.0f32X;", "double a = 1.0F64X;"] {
        with_configuration(
            source,
            interchange(CStandard::C23, ExtensionPolicy::Allow),
            |context, _| {
                let errors = messages(context);
                assert!(
                    errors
                        .iter()
                        .any(|error| error.contains("InvalidDecimalFloatLiteral")),
                    "{source}: {errors:?}"
                );
            },
        );
    }
}

/// The `nans` builtins give signaling NaNs of their own format with the
/// tag's payload, as GCC does.
#[test]
fn signaling_nan_builtins_keep_their_payload() {
    with_configuration(
        "_Float32 f = __builtin_nansf32(\"0x7\"); _Float64 g = __builtin_nansf64(\"\"); double h \
         = __builtin_nans(\"5\");",
        interchange(CStandard::C17, ExtensionPolicy::Allow),
        |context, unit| {
            assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
            let bits: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .map(|info| {
                    let ty = match unit.types.nodes[info.ty.index] {
                        | TypeKind::Scalar(Scalar::Float32) => crate::ir::Type::F32,
                        | _ => crate::ir::Type::F64,
                    };
                    crate::float_parsing::bits::constant_bits(&info.floating.unwrap().real, ty)
                        .unwrap()
                })
                .collect();
            assert_eq!(
                bits,
                [0x7F80_0007, 0x7FF4_0000_0000_0000, 0x7FF0_0000_0000_0005]
            );
        },
    );
}
