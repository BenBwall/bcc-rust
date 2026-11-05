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
    kinds_as(source, CompilerIdentity::Gcc)
}

/// The semantic error kinds of `source` in C23 under `identity`.
fn kinds_as(source: &str, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(identity)
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

/// A register array converted to a pointer has its address taken, which
/// GCC reports ("address of register compound literal requested"); GCC
/// still subscripts one.
#[test]
fn register_literal_arrays_do_not_decay() {
    use SemanticErrorKind as K;
    for (source, expected) in [
        (
            "void g(int *); void f(void) { g((register int[2]){ 1, 2 }); }",
            &[K::RegisterArrayAddress][..],
        ),
        (
            "void f(void) { int *p = (register int[2]){ 1, 2 }; }",
            &[K::RegisterArrayAddress],
        ),
        ("int f(void) { return (register int[2]){ 1, 2 }[1]; }", &[]),
        (
            "int *f(void) { return &(register int[2]){ 1, 2 }[1]; }",
            &[K::InvalidAddressOperand],
        ),
    ] {
        assert_eq!(kinds(source), expected, "{source}");
    }
}

/// The same holds for a register array object, which Clang does not even
/// subscript ("address of register variable requested").
#[test]
fn register_array_objects_do_not_decay() {
    use SemanticErrorKind as K;
    let call = "void g(int *); void f(void) { register int a[2] = { 1, 2 }; g(a); }";
    let subscript = "int f(void) { register int a[2] = { 1, 2 }; return a[1]; }";
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            kinds_as(call, identity),
            [K::RegisterArrayAddress],
            "{identity:?}"
        );
    }
    assert_eq!(kinds_as(subscript, CompilerIdentity::Gcc), []);
    assert_eq!(
        kinds_as(subscript, CompilerIdentity::Clang),
        [K::RegisterArrayAddress]
    );
    // One error for the element's address: GCC's for `&`, Clang's for the
    // conversion.
    let address = "void f(void) { register int a[2]; (void)&a[1]; }";
    assert_eq!(
        kinds_as(address, CompilerIdentity::Gcc),
        [K::InvalidAddressOperand]
    );
    assert_eq!(
        kinds_as(address, CompilerIdentity::Clang),
        [K::RegisterArrayAddress]
    );
    // Clang checks only a register array named directly: an array member
    // of a register structure decays there. GCC subscripts it but rejects
    // any other conversion.
    let member = "struct S { int a[2]; }; void g(int *); int f(void) { register struct S s = {{1, \
                  2}}; g(s.a); return s.a[1]; }";
    assert_eq!(kinds_as(member, CompilerIdentity::Clang), []);
    assert_eq!(
        kinds_as(member, CompilerIdentity::Gcc),
        [K::RegisterArrayAddress]
    );
    // Its element is still register storage, so Clang refuses its address
    // once, as it does through a parenthesized name.
    let element = "void f(void) { register struct S { int a[2]; } s; int *p = &s.a[1]; }";
    assert_eq!(
        kinds_as(element, CompilerIdentity::Clang),
        [K::InvalidAddressOperand]
    );
    let element = "void f(void) { register int a[2]; int *p = &(a)[1]; }";
    assert_eq!(
        kinds_as(element, CompilerIdentity::Clang),
        [K::InvalidAddressOperand]
    );
    let parenthesized = "void g(int *); void f(void) { register int a[2]; g((a)); }";
    assert_eq!(kinds_as(parenthesized, CompilerIdentity::Clang), []);
    assert_eq!(
        kinds_as(parenthesized, CompilerIdentity::Gcc),
        [K::RegisterArrayAddress]
    );
}

/// Clang 23 parses no storage-class specifier in a compound literal; its
/// identity reports the specifiers once and types a plain literal.
#[test]
fn clang_rejects_literal_storage_classes() {
    for source in [
        "int *p = &(register int){ 0 };",
        "void f(void) { int *q = &(thread_local int){ 0 }; }",
        "int f(void) { return (register int[2]){ 1, 2 }[0]; }",
        "int *f(void) { return &(static int){ 1 }; }",
    ] {
        assert_eq!(
            kinds_as(source, CompilerIdentity::Clang),
            [SemanticErrorKind::LiteralStorageClass],
            "{source}"
        );
    }
}
