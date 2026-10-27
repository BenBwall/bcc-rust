//! `_Float16`, IEEE binary16: a type for the GCC and Clang identities, its
//! `f16` constants, conversions, characteristics and diagnostics, and the
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

/// `_Float16` ranks below `float` (C23 §H.4.3p1); an integer operand
/// converts to it, and no default argument promotion widens it.
#[test]
fn half_is_a_distinct_type_below_float() {
    let source = r#"
#define T(x) _Generic((x), _Float16: 1, float: 2, double: 3, int: 4, default: 0)
_Float16 h; float f; double d;
_Static_assert(T(h) == 1 && T(1.5f16) == 1 && T(0x1p-3F16) == 1, "type");
_Static_assert(T(h + 1) == 1 && T(h * h) == 1 && T(-h) == 1 && T(h + f) == 2, "conversions");
_Static_assert(T(h + d) == 3 && T(h < h) == 4 && T(h ? h : 1) == 1, "results");
_Static_assert(sizeof(_Float16) == 2 && _Alignof(_Float16) == 2, "layout");
_Static_assert(!__builtin_types_compatible_p(_Float16, short), "distinct");
_Static_assert(__FLT16_MANT_DIG__ == 11 && __FLT16_MAX_EXP__ == 16, "characteristics");
_Static_assert(T(__FLT16_MAX__) == 1 && (int)__FLT16_MAX__ == 65504, "maximum");
static _Float16 initialized = 2.5f16 * 2;
"#;
    for id in [CompilerIdentity::Clang, CompilerIdentity::Gcc] {
        with_configuration(
            source,
            identity(id, ExtensionPolicy::Allow),
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context));
                let h = unit
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == "h")
                    .unwrap();
                assert_eq!(
                    unit.types.nodes[h.ty.index],
                    TypeKind::Scalar(Scalar::Float16)
                );
            },
        );
    }
}

/// GCC evaluates every `_Float16` constant and operation in `float` and
/// rounds only a conversion to `_Float16`. Clang's constant evaluator reads
/// a constant in binary16 and rounds each operation; its function code
/// keeps a chain's excess, so lowering evaluates those operators. `0.1f16`
/// is 0.0999755859375 in binary16 and 0.100000001 in `float`. Each
/// assertion holds for the compiler it names (GCC 15, the pinned Clang).
#[test]
fn constants_fold_in_each_identitys_evaluation_format() {
    let source = r#"
#ifdef __clang__
#define V(gcc, clang) clang
#else
#define V(gcc, clang) gcc
#endif
_Static_assert((int)(0.1f16 * 3 * 1000) == V(300, 299), "a chain");
_Static_assert((int)(0.1f16 * 10000) == 1000, "one operation");
_Static_assert((int)((_Float16)(1 / 3.0) * 3000) == V(999, 1000), "a cast rounds");
_Static_assert((int)((_Float16)1.00048828125000001 * 4096) == 4096, "double rounds once");
_Static_assert((int)((1 ? 0.1f16 * 3 : 1.0f16) * 1000) == V(300, 299), "?:");
_Static_assert(((float)(0.1f16 * 0.1f16) != (float)(_Float16)(0.1f16 * 0.1f16)) == V(1, 0), "conversions");
_Static_assert((60000.0f16 * 2 / 4 < 40000) == V(1, 0), "range");
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
            // Clang's function code computes the chains in `float`.
            let runtime = unit.expressions.iter().any(|info| info.runtime_query);
            assert_eq!(runtime, id == CompilerIdentity::Clang);
        });
    }
}

/// MSVC and TCC have no `_Float16`: the name is an identifier and `f16` no
/// suffix, and the characteristics macros are not predefined.
#[test]
fn only_the_gcc_and_clang_identities_provide_the_type() {
    for id in [CompilerIdentity::Msvc, CompilerIdentity::Tcc] {
        with_configuration(
            "typedef short _Float16;\n#ifdef __FLT16_MANT_DIG__\n#error no type\n#endif",
            identity(id, ExtensionPolicy::Allow),
            |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
        );
        with_configuration(
            "double x = 1.0f16;",
            identity(id, ExtensionPolicy::Allow),
            |context, _| {
                let errors = messages(context);
                assert!(
                    errors
                        .iter()
                        .any(|error| error.contains("InvalidDecimalFloatLiteral")),
                    "{errors:?}"
                );
            },
        );
    }
}

/// GCC: "ISO C does not support the '_Float16' type before C23" and
/// "non-standard suffix on floating constant"; Clang reports neither.
/// An out-of-range constant is reported in both, and `_Complex _Float16`
/// is refused.
#[test]
fn diagnostics_follow_the_identity() {
    let source = "_Float16 x = 1.5f16;";
    with_configuration(
        source,
        identity(CompilerIdentity::Gcc, ExtensionPolicy::Warn),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 2, "{errors:?}");
            assert!(
                errors.iter().all(|error| error.contains("C23")),
                "{errors:?}"
            );
        },
    );
    with_configuration(
        source,
        identity(CompilerIdentity::Clang, ExtensionPolicy::Deny),
        |context, _| assert_eq!(context.pending_error_count(), 0, "{:?}", messages(context)),
    );
    with_source(
        "_Float16 big = 65520.0f16, small = 1e-9f16;",
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
    with_source("_Complex _Float16 z;", |context, _| {
        let errors = messages(context);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("UnsupportedComplexHalf")),
            "{errors:?}"
        );
    });
}
