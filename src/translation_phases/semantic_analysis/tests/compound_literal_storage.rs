//! C23 storage-class specifiers in compound literals: the storage duration
//! they give a literal and the constraints of the declaration it stands
//! for. C23 §6.5.3.6p4-7, §6.7.2p3, §6.9p2; C99 §6.7.4p3.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

/// The semantic error kinds of `source` in C23 under GCC's identity.
fn kinds(source: &str) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(CompilerIdentity::Gcc)
            .with_target(crate::target::Target::LinuxGnu),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    result.push(error.kind);
                }
            }
        },
    );
    result
}

#[test]
fn literal_storage_follows_its_specifiers() {
    for source in [
        "int *f(void) { return &(static int){ 1 }; }",
        "int *f(void) { return (static thread_local int[2]){ 1, 2 }; }",
        "int *p = &(static int){ 1 }; int *q = (static int[2]){ 1, 2 };",
        "typedef __SIZE_TYPE__ size_t; size_t s = sizeof (thread_local int){ 1 };",
        "int f(void) { return (register int){ 3 } + (constexpr static int){ 4 }; }",
        "static inline int *f(void) { return &(static int){ 1 }; }",
        "inline const int *f(void) { return &(static const int){ 1 }; }",
        "inline int *f(void) { return &(static int){ 1 }; } extern int *f(void);",
        // A static literal's address is an address constant.
        "void f(void) { static int *p = &(static int){ 1 }; }",
    ] {
        assert_eq!(kinds(source), [], "{source}");
    }
}

#[test]
fn literal_storage_breaks_declaration_constraints() {
    use SemanticErrorKind as K;
    for (source, kind) in [
        ("int *p = &(register int){ 0 };", K::InvalidStorage),
        (
            "void f(void) { int *q = &(thread_local int){ 0 }; }",
            K::ThreadLocalBlockScope,
        ),
        (
            "int v; void f(void) { int *p = &(static int){ v }; }",
            K::NonConstantInitializer,
        ),
        (
            "void f(void) { &(register int){ 0 }; }",
            K::InvalidAddressOperand,
        ),
        (
            "struct s { int a, b; }; void f(void) { &((register struct s){ 1, 2 }.b); }",
            K::InvalidAddressOperand,
        ),
        // An automatic literal's address is no address constant.
        (
            "void f(void) { static int *p = &(int){ 1 }; }",
            K::NonConstantInitializer,
        ),
        // A thread-local literal's address is no address constant either.
        (
            "int *p = &(thread_local int){ 1 };",
            K::NonConstantInitializer,
        ),
        (
            "inline void f(void) { (static int){ 1 }; }",
            K::InlineStaticObject,
        ),
    ] {
        assert_eq!(kinds(source), [kind], "{source}");
    }
}
