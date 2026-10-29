//! The type-generic bit builtins: constant folding at every width, which
//! builtins and operands each identity accepts, and the diagnostics of
//! misuse under each identity's rules.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

fn c23(identity: CompilerIdentity) -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
        .with_compiler_identity(identity)
}

/// The semantic error kinds of `source` under `identity`.
fn errors(source: &str, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, c23(identity), |context, unit| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                kinds.push(error.kind);
            }
        }
        assert!(
            unit.bindings
                .iter()
                .any(|b| context.string_cache.at(b.name.name) == "following"),
            "{source}"
        );
    });
    kinds
}

#[test]
fn bit_builtins_fold_in_constant_expressions_under_each_identity() {
    let constants = include_str!("../../../../tests/fixtures/semantic/stdbit-constants.c");
    for (identity, definition) in [
        (CompilerIdentity::Gcc, "BCC_TEST_GCC"),
        (CompilerIdentity::Clang, "BCC_TEST_CLANG"),
    ] {
        let source = format!("#define {definition}\n{constants}");
        assert_eq!(errors(&source, identity), [], "{identity:?}");
    }
}

#[test]
fn gcc_only_bit_builtins_exist_for_the_gcc_identity() {
    for (identity, expected) in [
        (CompilerIdentity::Gcc, 1),
        (CompilerIdentity::Clang, 0),
        (CompilerIdentity::ClangCl, 0),
        (CompilerIdentity::Tcc, 0),
        (CompilerIdentity::Msvc, 0),
    ] {
        let source = format!(
            "_Static_assert(__has_builtin(__builtin_parityg) == {expected}); \
             _Static_assert(__has_builtin(__builtin_ffsg) == {expected}); \
             _Static_assert(__has_builtin(__builtin_clrsbg) == {expected}); \
             _Static_assert(__has_builtin(__builtin_popcountg)); \
             _Static_assert(__has_builtin(__builtin_stdc_rotate_right)); int following;"
        );
        assert_eq!(errors(&source, identity), [], "{identity:?}");
    }
}

/// Clang counts `bool` at one bit and an enumeration at its underlying
/// type's width, and refuses them only for the `__builtin_stdc_*` forms
/// other than the rotations; GCC refuses both everywhere.
#[test]
fn bool_and_enumerated_operands_follow_each_identity() {
    let source = "enum e { A }; int f(bool b, enum e x) { return __builtin_popcountg(b) + \
                  __builtin_clzg(x) + __builtin_stdc_rotate_left(b, 1) + \
                  __builtin_stdc_rotate_right(x, 1); } int following;";
    assert_eq!(errors(source, CompilerIdentity::Clang), []);
    assert_eq!(
        errors(source, CompilerIdentity::Gcc),
        [SemanticErrorKind::BitBuiltinBoolOrEnum; 4]
    );
    let stdc = "int f(bool b) { return __builtin_stdc_leading_zeros(b); } int following;";
    assert_eq!(
        errors(stdc, CompilerIdentity::Clang),
        [SemanticErrorKind::BitBuiltinBoolOrEnum]
    );
}

/// The misuse fixtures report one error of the expected kind for each
/// misuse, under their identity, and keep the following declaration.
#[test]
fn misuse_reports_each_identity_s_constraint() {
    use SemanticErrorKind as K;
    let gcc = include_str!("../../../../tests/fixtures/diagnostics/sema-stdbit-gcc.c");
    let mut expected = vec![K::BitBuiltinSigned];
    expected.extend([K::BitBuiltinBoolOrEnum; 2]);
    expected.extend([K::BitBuiltinNotInteger; 3]);
    expected.extend([K::BitBuiltinUnsigned; 2]);
    expected.push(K::BitBuiltinBoolOrEnum);
    expected.extend([K::BitBuiltinSigned; 2]);
    expected.extend([K::BitBuiltinFallback; 5]);
    expected.extend([K::BitBuiltinRotateCount; 2]);
    expected.extend([K::InvalidArgumentCount; 4]);
    assert_eq!(errors(gcc, CompilerIdentity::Gcc), expected);
    let clang = include_str!("../../../../tests/fixtures/diagnostics/sema-stdbit-clang.c");
    let mut expected = vec![K::BitBuiltinSigned; 2];
    expected.extend([K::BitBuiltinBoolOrEnum; 2]);
    expected.push(K::BitBuiltinNotInteger);
    expected.extend([K::BitBuiltinFallback; 3]);
    expected.push(K::BitBuiltinRotateCount);
    expected.push(K::InvalidArgumentCount);
    assert_eq!(errors(clang, CompilerIdentity::Clang), expected);
}

/// A bit-field operand computes at the field's width under GCC's rules
/// and at its declared type's under Clang's; the result of a
/// value-returning form keeps the declared type.
#[test]
fn bit_field_operands_follow_each_identity() {
    let source = "struct s { unsigned a : 3; } v = { 1 }; int f(void) { return \
                  __builtin_clzg(v.a); } unsigned g(void) { return __builtin_stdc_bit_ceil(v.a); \
                  } int following;";
    for (identity, width) in [(CompilerIdentity::Gcc, 3), (CompilerIdentity::Clang, 32)] {
        with_configuration(source, c23(identity), |context, unit| {
            assert_eq!(context.pending_error_count(), 0, "{identity:?}");
            let calls: Vec<_> = unit
                .expressions
                .iter()
                .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
                .collect();
            assert_eq!(calls.len(), 2, "{identity:?}");
            for call in calls {
                let operation = call.operation_type.expect("an operation type");
                let bits = match unit.types.nodes[operation.index] {
                    | TypeKind::BitInt {
                        width,
                        signed: false,
                    } => width,
                    | TypeKind::Scalar(Scalar::UnsignedInt) => 32,
                    | other => panic!("{identity:?}: {other:?}"),
                };
                assert_eq!(bits, width, "{identity:?}");
            }
        });
    }
}
