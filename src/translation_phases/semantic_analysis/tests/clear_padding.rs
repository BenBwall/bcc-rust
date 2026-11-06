//! `__builtin_clear_padding`: which operands each identity accepts, and the
//! type retained for lowering. The wording is in
//! `tests/integration/personality_cli.rs`. GNU extension: GCC manual, "Other
//! Builtins".

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

const DECLARATIONS: &str = "struct S { char a; int b; }; struct I; struct F { int a; char f[]; };
struct G { struct F a; char b; }; union U { char a; struct F b; }; typedef void fn(void);\n";

fn accepted(body: &str, identity: CompilerIdentity) -> bool {
    let source = format!("{DECLARATIONS}void f(int n, void *p) {{ {body} }}\nint following;\n");
    let configuration = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
        .with_compiler_identity(identity);
    let mut accepted = true;
    with_configuration(&source, configuration, |context, unit| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                assert_eq!(
                    error.kind,
                    SemanticErrorKind::ClearPaddingOperand,
                    "{body}: only the call is refused"
                );
                accepted = false;
            }
        }
        assert!(
            unit.bindings
                .iter()
                .any(|b| context.string_cache.at(b.name.name) == "following"),
            "{body}"
        );
    });
    accepted
}

#[test]
fn each_identity_accepts_the_operands_of_its_compiler() {
    use CompilerIdentity::{
        Clang,
        Gcc,
    };
    // The operand's declaration, then whether GCC and Clang accept it.
    for (operand, gcc, clang) in [
        ("struct S *q = p;", true, true),
        ("int *q = p;", true, true),
        ("struct S (*q)[2] = p;", true, true),
        ("struct S (*q)[n] = p;", true, true),
        ("int *restrict q = p;", true, true),
        ("struct S *const q = p;", true, true),
        ("volatile struct S *q = p;", true, true),
        ("struct { char a; const int b; } *q = p;", true, true),
        ("void **q = p;", true, true),
        ("int q = 0;", false, false),
        ("struct S r = {0}; struct S q = r;", false, false),
        ("void *q = p;", false, false),
        ("const void *q = p;", false, false),
        ("struct I *q = p;", false, false),
        ("int (*q)[] = p;", false, false),
        ("const struct S *q = p;", false, false),
        ("void *const *q = p;", false, false),
        ("struct F *q = p;", false, false),
        ("struct G *q = p;", false, false),
        ("union U *q = p;", false, false),
        ("fn *q = 0;", true, false),
        ("const char (*q)[2] = p;", true, false),
        ("_Atomic struct S *q = p;", false, true),
        ("_Atomic int *q = p;", false, true),
        ("_Atomic(int *) q = 0;", true, false),
        ("struct F (*q)[2] = p;", false, true),
        ("_Atomic int (*q)[2] = p;", true, false),
        ("char q[4];", true, false),
    ] {
        let body = format!("{operand} __builtin_clear_padding(q);");
        assert_eq!(accepted(&body, Gcc), gcc, "GCC: {operand}");
        assert_eq!(accepted(&body, Clang), clang, "Clang: {operand}");
    }
}

#[test]
fn a_call_has_one_argument_and_no_value() {
    use CompilerIdentity::{
        Clang,
        Gcc,
    };
    for identity in [Gcc, Clang] {
        assert!(!accepted("__builtin_clear_padding();", identity));
        assert!(!accepted("__builtin_clear_padding(p, p);", identity));
        assert!(accepted(
            "int *q = p; (void)__builtin_clear_padding(q);",
            identity
        ));
    }
}

/// The kind of the type retained for the one call in `body`.
fn retained(body: &str, identity: CompilerIdentity) -> String {
    let source = format!("{DECLARATIONS}void f(int n, void *p) {{ {body} }}\nint following;\n");
    let configuration = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
        .with_compiler_identity(identity);
    let mut retained = String::new();
    with_configuration(&source, configuration, |context, unit| {
        assert_eq!(context.pending_error_count(), 0, "{body}");
        let mut calls = unit
            .expressions
            .iter()
            .filter(|info| matches!(info.expression.kind, ExpressionType::Call { .. }));
        let ty = calls
            .next()
            .and_then(|call| call.operation_type)
            .expect("a retained type");
        retained = match unit.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Void) => "nothing",
            | TypeKind::Array(..) => "array",
            | TypeKind::Tag(_) => "record",
            | TypeKind::Scalar(_) => "scalar",
            | TypeKind::Nullptr => "nullptr",
            | other => panic!("{body}: {other:?}"),
        }
        .to_owned();
    });
    retained
}

#[test]
fn the_retained_type_is_the_one_lowering_clears() {
    use CompilerIdentity::{
        Clang,
        Gcc,
    };
    for (operand, gcc, clang) in [
        ("struct S *q = p;", "record", "record"),
        ("int *q = p;", "scalar", "scalar"),
        ("struct S (*q)[2] = p;", "array", "array"),
        ("struct S (*q)[n] = p;", "array", "nothing"),
        ("struct S (*q)[n][2] = p;", "array", "nothing"),
        ("struct S (*q)[2][n] = p;", "array", "nothing"),
        ("typeof(nullptr) *q = p;", "nothing", "nullptr"),
        ("typeof(nullptr) (*q)[2] = p;", "nothing", "array"),
    ] {
        let body = format!("{operand} __builtin_clear_padding(q);");
        assert_eq!(retained(&body, Gcc), gcc, "GCC: {operand}");
        assert_eq!(retained(&body, Clang), clang, "Clang: {operand}");
    }
    assert_eq!(
        retained("fn *q = 0; __builtin_clear_padding(q);", Gcc),
        "nothing"
    );
}
