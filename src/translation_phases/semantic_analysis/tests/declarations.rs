//! Declaration, tag and layout regressions from the Stage-1 review.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

fn gnu17() -> CompilerConfiguration {
    CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow).with_gnu_extensions(true)
}

fn pedantic(standard: CStandard, gnu: bool) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, ExtensionPolicy::Deny).with_gnu_extensions(gnu)
}

/// Semantic error kinds in discovery order.
fn kinds(source: &str, configuration: CompilerConfiguration) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Semantic(e) => kinds.push(e.kind),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    kinds
}

/// Extension diagnostics, in order, as their rendered messages.
fn extensions(source: &str, configuration: CompilerConfiguration) -> Vec<String> {
    let mut messages = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            match error {
                | TranslationError::Extension(e) => messages.push(e.to_string()),
                | other => panic!("{source}: unexpected {other:?}"),
            }
        }
    });
    messages
}

fn tag_layout(source: &str, configuration: CompilerConfiguration, name: &str) -> Option<Layout> {
    let mut layout = None;
    with_configuration(source, configuration, |context, s| {
        assert_eq!(context.pending_error_count(), 0, "{source}");
        layout = s
            .types
            .tags
            .iter()
            .find(|t| t.name.is_some_and(|n| context.string_cache.at(n) == name))
            .and_then(|t| t.layout.get());
    });
    layout
}

#[test]
fn nested_tag_redefinition_is_diagnosed_once_and_terminates() {
    assert_eq!(
        kinds(
            "struct S { struct S { int x; } y; int z; }; struct S s; void f(void) { s = s; } int \
             n = sizeof(s);",
            gnu17()
        ),
        [SemanticErrorKind::TagRedefinition]
    );
    assert_eq!(
        kinds("enum E { A = sizeof(enum E { B }) };", gnu17()),
        [SemanticErrorKind::TagRedefinition]
    );
}

#[test]
fn rejected_tag_bodies_still_declare_their_contents() {
    assert_eq!(
        kinds(
            "struct S { int x; }; struct S { enum { Q = 2 } q; }; int a[Q]; enum E { A }; enum E \
             { B = 3 }; int b[B]; union S { int u; enum { R } r; } v; int c[R + 1];",
            gnu17()
        ),
        [
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagRedefinition,
            SemanticErrorKind::TagKindMismatch,
        ]
    );
}

#[test]
fn self_containing_records_terminate() {
    assert_eq!(
        kinds(
            "struct S { int a; struct S s; }; struct S x; void f(void) { x = x; x.a = 1; struct S \
             y = {1}; }",
            gnu17()
        ),
        [SemanticErrorKind::InvalidMember]
    );
}

#[test]
fn declaration_attributes_never_erase_a_shared_tag_layout() {
    assert_eq!(
        kinds(
            "struct A { int a; }; struct B { double b; }; __attribute__((unused)) static struct A \
             ua; struct A g(void); struct B g(void); _Alignas(8) struct A aligned; extern struct \
             A obj; extern int obj;",
            gnu17()
        ),
        [
            SemanticErrorKind::IncompatibleDeclaration,
            SemanticErrorKind::IncompatibleDeclaration,
        ]
    );
    assert_eq!(
        tag_layout(
            "struct A { int a; }; __attribute__((unused)) static struct A ua; _Alignas(8) struct \
             A b;",
            gnu17(),
            "A"
        ),
        Some(Layout { size: 4, align: 4 })
    );
}

#[test]
fn only_layout_attributes_make_declarator_types_unanalyzed() {
    // Function, pointer and member attributes that do not change layout keep
    // the declared type, so later conflicts are still found.
    assert_eq!(
        kinds(
            "extern int printf(const char *, ...) __attribute__((__format__(__printf__, 1, 2))); \
             extern long printf(const char *, ...); int *__attribute__((unused)) p; long *p;",
            gnu17()
        ),
        [
            SemanticErrorKind::IncompatibleDeclaration,
            SemanticErrorKind::IncompatibleDeclaration,
        ]
    );
    // x86-64 ignores calling conventions; pointer-size modifiers change layout.
    assert_eq!(
        kinds(
            "void __cdecl c(void); int c(void); int *__ptr32 r; long r;",
            gnu17().with_msvc_extensions(true)
        ),
        [SemanticErrorKind::IncompatibleDeclaration]
    );
    // Alignment, packing, modes and vectors are not modeled.
    assert_eq!(
        kinds(
            "int x __attribute__((aligned(16))); extern long x; int \
             *__attribute__((__aligned__(8))) q; long q; int m __attribute__((mode(DI))); long m;",
            gnu17()
        ),
        []
    );
    assert_eq!(
        tag_layout(
            "struct S { int a __attribute__((unused)); char b; };",
            gnu17(),
            "S"
        ),
        Some(Layout { size: 8, align: 4 })
    );
    assert_eq!(
        tag_layout(
            "struct S { int a; char b __attribute__((aligned(8))); };",
            gnu17(),
            "S"
        ),
        None
    );
}

#[test]
fn unanalyzed_redeclarations_inherit_the_previous_kind() {
    assert_eq!(
        kinds(
            "int f(void); extern __typeof__(f) f; __typeof__(f) f; int g(void) { return 0; } \
             extern __typeof__(g) g __asm__(\"g_alias\"); void h(void) { f(); g(); }",
            gnu17()
        ),
        []
    );
    // GNU typeof stays an extension under the pedantic policy.
    assert_eq!(
        extensions(
            "int f(void); extern __typeof__(f) f;",
            pedantic(CStandard::C99, false)
        ),
        ["'__typeof__' is a GNU extension"]
    );
}

#[test]
fn enums_use_the_gcc_x86_64_compatible_integer_type() {
    for configuration in [CompilerConfiguration::default(), gnu17()] {
        assert_eq!(
            kinds(
                "enum E {A, B}; unsigned int f(void); enum E f(void); int x[(enum E)-1 > 0 ? 1 : \
                 -1]; enum N {M = -1}; int g(void); enum N g(void); int y[(enum N)-1 < 0 ? 1 : \
                 -1]; int z[A - 1 < 0 ? 1 : -1]; void h(enum E e) { int w[e - 1 > 0 ? 1 : 1]; }",
                configuration
            ),
            []
        );
        assert_eq!(
            kinds(
                "enum E {A}; int f(void); enum E f(void); enum N {M = -1}; unsigned g(void); enum \
                 N g(void);",
                configuration
            ),
            [
                SemanticErrorKind::IncompatibleDeclaration,
                SemanticErrorKind::IncompatibleDeclaration,
            ]
        );
    }
    assert_eq!(
        tag_layout("enum L {P = 0x100000000L};", gnu17(), "L"),
        Some(Layout { size: 8, align: 8 })
    );
    assert_eq!(
        tag_layout("enum L {P = -1, Q = 0x7fffffff};", gnu17(), "L"),
        Some(Layout { size: 4, align: 4 })
    );
}

#[test]
fn enumerators_beyond_int_are_a_policy_extension() {
    let source = "enum { HI = 0x80000000, ALL = 0xFFFFFFFF, NEXT }; int t[HI > 0 ? 1 : -1]; int \
                  u[NEXT == 0x100000000L ? 1 : -1];";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C99, false)),
        [
            "'enumerator value outside the range of int' is a C23 extension",
            "'enumerator value outside the range of int' is a C23 extension",
            "'enumerator value outside the range of int' is a C23 extension",
        ]
    );
    assert_eq!(
        extensions(source, pedantic(CStandard::C23, false)),
        Vec::<String>::new()
    );
    assert_eq!(
        kinds("enum { N = -1, U = 0xFFFFFFFFFFFFFFFF };", gnu17()),
        [SemanticErrorKind::EnumeratorRange]
    );
}

#[test]
fn exceptional_array_bounds_are_variable_length_arrays() {
    let source = "void f(void) { int b[1/0]; int c[(1 << 31) / 4 + 1]; (void)b; }";
    assert_eq!(
        kinds(source, gnu17()),
        [SemanticErrorKind::InvalidArrayBound]
    );
    with_configuration(source, gnu17(), |_, s| {
        let b = s
            .bindings
            .iter()
            .find(|b| b.duration == Duration::Automatic)
            .unwrap();
        assert!(matches!(
            s.types.nodes[b.ty.index],
            TypeKind::Array(_, ArrayBound::Variable)
        ));
    });
    assert_eq!(
        kinds(
            "int g[1/0]; enum { E = 1/0 }; void h(int x) { switch (x) { case 1/0: break; } } \
             struct S { int w : 1/0; };",
            gnu17()
        ),
        [
            SemanticErrorKind::FileScopeVariableType,
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
        ]
    );
}

#[test]
fn shifting_into_the_sign_bit_is_a_gnu_extension() {
    let source = "enum { F = 1 << 31 }; int x[(1 << 31) < 0 ? 1 : -1]; static int s = 1 << 31; \
                  int y[F < 0 ? 1 : -1];";
    assert_eq!(kinds(source, gnu17()), []);
    assert_eq!(kinds(source, CompilerConfiguration::default()), []);
    assert_eq!(
        extensions(source, pedantic(CStandard::C99, false)),
        ["'left shift into the sign bit' is a GNU extension"; 3]
    );
    assert_eq!(
        kinds("enum { G = 3 << 31, H = 1 << 32 };", gnu17()),
        [
            SemanticErrorKind::ConstantOverflow,
            SemanticErrorKind::ConstantOverflow,
        ]
    );
}
