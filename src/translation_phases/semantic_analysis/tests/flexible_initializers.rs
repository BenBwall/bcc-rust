//! Initializing a flexible array member (GNU extension, #59): only a static
//! object's own structure may, under the extension policy as in Clang's
//! `-Wgnu-flexible-array-initializer`; elsewhere it is an error as in GCC
//! and Clang.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

/// `(errors, extension diagnostics)` of `source` under `policy`.
fn diagnostics(source: &str, policy: ExtensionPolicy) -> (Vec<SemanticErrorKind>, usize) {
    let mut result = (Vec::new(), 0);
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C11, policy),
        |context, _| {
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(e) => result.0.push(e.kind),
                    | TranslationError::Extension(_) => result.1 += 1,
                    | other => panic!("{source}: unexpected {other:?}"),
                }
            }
        },
    );
    result
}

const STATIC: &str = r#"struct F { int n; int v[]; };
    static struct F f = { 3, { 1, 2, 3 } };
    struct S { short len; char text[]; } const msg = { 5, "hello" };
    struct F elided = { 1, 2, 3 };
    struct F designated = { .v = { [4] = 1 } };
    void g(void) { static struct F local = { 1, { 2 } }; (void)local; }
    _Static_assert(sizeof f == sizeof(struct F), "sizeof is the structure's");"#;

#[test]
fn static_objects_initialize_their_flexible_array_member_as_an_extension() {
    // Clang: accepted silently by default, warned with -pedantic, an error
    // with -pedantic-errors; one diagnostic per initialized member.
    assert_eq!(diagnostics(STATIC, ExtensionPolicy::Allow), (vec![], 0));
    assert_eq!(diagnostics(STATIC, ExtensionPolicy::Warn), (vec![], 5));
    assert_eq!(diagnostics(STATIC, ExtensionPolicy::Deny), (vec![], 5));
    with_configuration(
        STATIC,
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow),
        |context, s| {
            assert!(context.take_pending_errors().is_empty());
            let size = |name: &str| {
                let binding = s
                    .bindings
                    .iter()
                    .position(|b| context.string_cache.at(b.name.name) == name)
                    .unwrap();
                s.flexible_object_size(binding)
            };
            assert_eq!(size("f"), Some(16));
            assert_eq!(size("msg"), Some(8));
            assert_eq!(size("elided"), Some(12));
            assert_eq!(size("designated"), Some(24));
            assert_eq!(size("local"), Some(8));
        },
    );
}

#[test]
fn other_flexible_array_member_initializers_are_errors() {
    for source in [
        // Automatic storage.
        "struct F { int n; int v[]; }; void g(void) { struct F a = { 1, { 2 } }; (void)a; }",
        // A structure nested in another object, braced or by brace elision.
        "struct F { int n; int v[]; }; struct G { struct F f; int after; } g = { { 1, { 2 } } };",
        "struct F { int n; int v[]; }; struct G { struct F f; int after; } g = { 1, 3 };",
        "struct F { int n; int v[]; }; struct F a[2] = { 1, 2 };",
        // A designator into an element of the member (Clang).
        "struct F { int n; int v[]; }; static struct F f = { .v[1] = 2 };",
        // Compound literals.
        "struct F { int n; int v[]; }; struct F *p = &(struct F){ 1, { 2 } };",
    ] {
        assert_eq!(
            diagnostics(source, ExtensionPolicy::Allow).0,
            [SemanticErrorKind::FlexibleArrayInitializer],
            "{source}"
        );
    }
    // Initializers that leave the member empty stay valid.
    assert_eq!(
        diagnostics(
            "struct F { int n; int v[]; }; void g(void) { struct F a = { 1 }; (void)a; } struct G \
             { struct F f; int after; } outer = { { 1 }, 2 };",
            ExtensionPolicy::Allow
        ),
        (vec![], 0)
    );
}

/// The storage sizes semantic analysis records for objects whose
/// initializer gives their flexible array member elements.
fn flexible_sizes(source: &str) -> Vec<u64> {
    let mut sizes = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow),
        |context, s| {
            assert!(context.take_pending_errors().is_empty(), "{source}");
            sizes = (0..s.bindings.len())
                .filter_map(|binding| s.flexible_object_size(binding))
                .collect();
        },
    );
    sizes
}

#[test]
fn empty_lists_initialize_any_flexible_array_member_as_an_extension() {
    // Clang accepts an empty list for every flexible array member, with one
    // extension diagnostic each; GCC rejects the automatic ones.
    const EMPTY: &str = "struct F { int n; int v[]; };
        void g(void) { struct F a = { 1, {} }; struct F b = { .v = {}, .n = 2 }; (void)a; (void)b; \
                         }
        struct G { struct F f; int after; } outer = { { 1, {} }, 2 };
        struct F *p = &(struct F){ 1, {} };
        static struct F s = { 1, {} };";
    assert_eq!(diagnostics(EMPTY, ExtensionPolicy::Allow), (vec![], 0));
    // Five flexible array initializations, five C23 empty initializers and
    // a structure with a flexible array member inside another.
    assert_eq!(diagnostics(EMPTY, ExtensionPolicy::Warn), (vec![], 11));
    // Only the static object's own member records an extent.
    assert_eq!(flexible_sizes(EMPTY), [4]);
}

#[test]
fn brace_elision_into_flexible_array_elements_counts_each_element() {
    // Clang: the elided pair gives one element, and the braced list four.
    assert_eq!(
        flexible_sizes(
            "struct Q { int n; struct { short a, b; } v[]; };
             static struct Q one = { 1, 2 };
             static struct Q four = { 1, { { 4, 5 }, 6, 7, [3].b = 8 } };"
        ),
        [8, 20]
    );
}

#[test]
fn a_repeated_flexible_array_member_initializer_keeps_the_larger_extent() {
    // §6.7.8p19: the later list overrides the earlier one, but the storage
    // keeps room for every listed element (Clang allocates only the last).
    assert_eq!(
        flexible_sizes(
            "struct F { int n; int v[]; };
             static struct F twice = { .v = { 1, 2, 3 }, .n = 1, .v = { 4 } };"
        ),
        [16]
    );
}
