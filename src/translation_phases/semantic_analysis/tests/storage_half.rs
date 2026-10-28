//! Clang's `__fp16`: binary16 storage for the Clang identities whose values
//! promote to `float`, refused as a parameter or result type; GCC, MSVC and
//! TCC keep the name an identifier.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

fn identity(identity: CompilerIdentity) -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow)
        .with_compiler_identity(identity)
        .with_gnu_extensions(true)
}

fn messages(context: &mut Context<'_>) -> Vec<String> {
    context
        .take_pending_errors()
        .iter()
        .map(|error| format!("{error:?}"))
        .collect()
}

/// Every operation promotes an `__fp16` operand to `float`, as an integer
/// promotes to `int`; assignment keeps the type and rounds once. Each
/// assertion holds for the pinned Clang.
#[test]
fn values_promote_to_float() {
    let source = r#"
#define T(x) _Generic((x), __fp16: 1, _Float16: 2, float: 3, double: 4, int: 5, default: 0)
__fp16 a; _Float16 h;
_Static_assert(T(a) == 1 && T(a + a) == 3 && T(-a) == 3 && T(a + h) == 3 && T(a * 2) == 3, "promotions");
_Static_assert(T(a = 1) == 1 && T(a ? a : a) == 3 && T(a + 1.0) == 4, "results");
_Static_assert(sizeof(__fp16) == 2 && _Alignof(__fp16) == 2, "layout");
_Static_assert(!__builtin_types_compatible_p(__fp16, _Float16), "distinct");
_Static_assert((float)(__fp16)0.1 == 0x1.998p-4f && (__fp16)65520.0 > 1e9, "rounding");
static __fp16 folded = (__fp16)0.1 * 3;
__fp16 *pointer(__fp16 *p);
"#;
    for id in [CompilerIdentity::Clang, CompilerIdentity::ClangCl] {
        with_configuration(source, identity(id), |context, _| {
            assert_eq!(
                context.pending_error_count(),
                0,
                "{id:?}: {:?}",
                messages(context)
            );
        });
    }
}

/// Clang: "parameters cannot have __fp16 type; did you forget * ?",
/// "function return value cannot have __fp16 type; did you forget * ?" and
/// "'_Complex half' is invalid".
#[test]
fn storage_only_uses_are_refused() {
    with_configuration(
        "__fp16 f(void);\nvoid (*g)(__fp16);\n__fp16 k();\n_Complex __fp16 z;",
        identity(CompilerIdentity::Clang),
        |context, _| {
            let errors = messages(context);
            assert_eq!(errors.len(), 4, "{errors:?}");
            for kind in ["Fp16Return", "Fp16Parameter", "ComplexFp16"] {
                assert!(
                    errors.iter().any(|error| error.contains(kind)),
                    "{errors:?}"
                );
            }
        },
    );
}

/// Only Clang provides the type on x86-64.
#[test]
fn other_identities_keep_the_identifier() {
    for id in [
        CompilerIdentity::Gcc,
        CompilerIdentity::Msvc,
        CompilerIdentity::Tcc,
    ] {
        with_configuration(
            "typedef short __fp16; __fp16 x;",
            identity(id),
            |context, _| {
                assert_eq!(
                    context.pending_error_count(),
                    0,
                    "{id:?}: {:?}",
                    messages(context)
                );
            },
        );
    }
}
