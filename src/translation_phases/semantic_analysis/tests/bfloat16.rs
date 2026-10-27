//! `__bf16`, bfloat16: a type for the GCC and Clang identities that ranks
//! below `_Float16`, GCC's `bf16` constants and `__BFLT16_*` macros, and the
//! evaluation in `float` each identity folds constants with.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

fn identity(identity: CompilerIdentity, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C11, policy).with_compiler_identity(identity)
}

fn messages(context: &mut Context<'_>) -> Vec<String> {
    context
        .take_pending_errors()
        .iter()
        .map(|error| format!("{error:?}"))
        .collect()
}

/// `__bf16` ranks below `float` and `_Float16`, as GCC and Clang convert
/// it; an integer operand converts to it, and no default argument
/// promotion widens it.
#[test]
fn bfloat16_is_a_distinct_type_below_float16() {
    let source = r#"
#define T(x) _Generic((x), __bf16: 1, _Float16: 2, float: 3, double: 4, int: 5, default: 0)
__bf16 b; _Float16 h; float f; double d;
_Static_assert(T(b) == 1 && T(b + 1) == 1 && T(b * b) == 1 && T(-b) == 1, "type");
_Static_assert(T(b + h) == 2 && T(h - b) == 2 && T(b + f) == 3 && T(b + d) == 4, "conversions");
_Static_assert(T(b < b) == 5 && T(b ? b : 1) == 1 && T(1 ? b : h) == 2, "results");
_Static_assert(sizeof(__bf16) == 2 && _Alignof(__bf16) == 2, "layout");
_Static_assert(!__builtin_types_compatible_p(__bf16, _Float16), "distinct");
static __bf16 initialized = (__bf16)2.5f * 2;
"#;
    for id in [CompilerIdentity::Clang, CompilerIdentity::Gcc] {
        with_configuration(
            source,
            identity(id, ExtensionPolicy::Allow),
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
                let b = unit
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == "b")
                    .unwrap();
                assert_eq!(
                    unit.types.nodes[b.ty.index],
                    TypeKind::Scalar(Scalar::BFloat16)
                );
            },
        );
    }
}

/// GCC keeps every `__bf16` operation's `float` value and converts integer
/// operands to `float`; Clang's constant evaluator rounds each operation
/// and converts an integer, and a `__bf16` operand of a `_Float16`
/// operation, to the operation's type first. Each assertion holds for the
/// compiler it names (GCC 15, the pinned Clang).
#[test]
fn constants_fold_in_each_identitys_evaluation_format() {
    let source = r#"
#ifdef __clang__
#define V(gcc, clang) clang
#else
#define V(gcc, clang) gcc
#endif
#define P ((__bf16)1.0078125f * (__bf16)1.0078125f)
_Static_assert((int)((P - (__bf16)1.015625f) * 65536) == V(4, 0), "a chain");
_Static_assert((int)((__bf16)0.0f + 257) == V(257, 256), "an integer operand");
_Static_assert((int)((P + (_Float16)1.0009765625f) * 1024) == V(2065, 2064), "with _Float16");
_Static_assert((unsigned long long)(__bf16)0x8080000000000001ull == 0x8100000000000000ull, "once");
_Static_assert((int)((__bf16)(1 / 3.0) * 768) == 256, "a cast rounds");
_Static_assert((float)(__bf16)0x1.0100000000001p0Q == 0x1.02p0f, "binary128 rounds once");
_Static_assert((float)(_Float16)0x1.0020000000001p0Q == 0x1.004p0f, "for _Float16 too");
_Static_assert((float)(__bf16)0x1.0100000000001p0L == 0x1.02p0f, "x87 rounds once");
_Static_assert(((float)P != (float)(__bf16)P) == V(1, 0), "conversions");
"#;
    for id in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        // GNU modes fold arithmetic constant expressions in assertions.
        let configuration = identity(id, ExtensionPolicy::Allow).with_gnu_extensions(true);
        with_configuration(source, configuration, |context, unit| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{id:?}: {:?}",
                messages(context)
            );
            let runtime = unit.expressions.iter().any(|info| info.runtime_query);
            assert_eq!(runtime, id == CompilerIdentity::Clang);
        });
    }
}

/// GCC's `bf16` and `BF16` suffixes read a constant rounded once to
/// bfloat16, which GCC evaluates in `float` like the operations, and its
/// `__BFLT16_*` macros describe the format; Clang has neither.
#[test]
fn gcc_provides_constants_and_characteristics() {
    let source = r#"
_Static_assert(_Generic(1.0bf16, __bf16: 1, default: 0) && _Generic(0x1p0BF16, __bf16: 1, default: 0), "type");
_Static_assert((int)(1.0078125bf16 * 1.0078125bf16 * 65536) == 66564, "excess");
_Static_assert((float)(__bf16)0.1bf16 == 0x1.9ap-4f, "rounded");
_Static_assert(__BFLT16_MANT_DIG__ == 8 && __BFLT16_MAX_EXP__ == 128 && !__BFLT16_IS_IEC_60559__, "characteristics");
_Static_assert(_Generic(__BFLT16_MAX__, __bf16: 1, default: 0) && __BFLT16_MAX__ == 0x1.fep127f, "maximum");
"#;
    with_configuration(
        source,
        identity(CompilerIdentity::Gcc, ExtensionPolicy::Allow).with_gnu_extensions(true),
        |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
    );
    with_configuration(
        "#ifdef __BFLT16_MANT_DIG__\n#error no macros\n#endif\ndouble x = 1.0bf16;",
        identity(CompilerIdentity::Clang, ExtensionPolicy::Allow),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(
                errors[0].contains("InvalidDecimalFloatLiteral"),
                "{errors:?}"
            );
        },
    );
}

/// MSVC and TCC have no `__bf16`; GCC's `-pedantic` reports the suffix as
/// non-standard, out-of-range constants are reported, and neither compiler
/// has `_Complex __bf16`.
#[test]
fn diagnostics_follow_the_identity() {
    for id in [CompilerIdentity::Msvc, CompilerIdentity::Tcc] {
        with_configuration(
            "typedef short __bf16;",
            identity(id, ExtensionPolicy::Allow),
            |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
        );
    }
    with_configuration(
        "__bf16 x = 1.5bf16;",
        identity(CompilerIdentity::Gcc, ExtensionPolicy::Warn),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 1, "{errors:?}");
        },
    );
    with_configuration(
        "__bf16 x = 1.5f;",
        identity(CompilerIdentity::Gcc, ExtensionPolicy::Deny),
        |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
    );
    with_configuration(
        "__bf16 big = 1e39bf16, small = 1e-50bf16;",
        identity(CompilerIdentity::Gcc, ExtensionPolicy::Allow),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 2, "{errors:?}");
            assert!(
                errors
                    .iter()
                    .all(|error| error.contains("FloatConstantOutOfRange")),
                "{errors:?}"
            );
        },
    );
    for id in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        with_configuration(
            "_Complex __bf16 z;",
            identity(id, ExtensionPolicy::Allow),
            |context, _| {
                let errors = messages(context);
                assert!(
                    errors.iter().any(|error| error.contains("ComplexBFloat16")),
                    "{errors:?}"
                );
            },
        );
    }
}

/// An integer constant converts to every floating type with one rounding
/// from its exact value, however wide: each value's last bit decides a
/// tie that rounding to the x87 format's 64 bits first would lose. Each
/// assertion holds in GCC 15 and the pinned Clang.
#[test]
fn integer_constants_round_once() {
    let source = r#"
#define T(top, tie) (((unsigned __int128)1 << top) + ((unsigned __int128)1 << tie) + 1)
_Static_assert((float)T(100, 76) == 0x1.000002p100f, "float");
_Static_assert((double)T(100, 47) == 0x1.0000000000001p100, "double");
_Static_assert((float)(__bf16)T(100, 92) == 0x1.02p100f, "bf16");
_Static_assert((float)(_Float16)T(15, 4) == 0x1.004p15f, "f16");
_Static_assert((long double)T(100, 36) == 0x1.0000000000000002p100L, "x87");
_Static_assert((float)-(__int128)T(100, 76) == -0x1.000002p100f, "signed");
_Static_assert((double)(__int128)T(100, 47) + 0.0 == 0x1.0000000000001p100, "arithmetic");
_Static_assert((float)(__bf16)((__int128)T(100, 92) + 0) == 0x1.02p100f, "cast of a sum");
"#;
    for id in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        let configuration = identity(id, ExtensionPolicy::Allow).with_gnu_extensions(true);
        with_configuration(source, configuration, |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{id:?}: {:?}",
                messages(context)
            );
        });
    }
}
