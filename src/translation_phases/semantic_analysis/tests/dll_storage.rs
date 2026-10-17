//! Windows DLL storage: Clang's applicability and redeclaration diagnostics
//! for `dllimport` and `dllexport` (bundled Clang 23 on
//! x86_64-pc-windows-msvc and x86_64-w64-windows-gnu), and the storage and
//! inline linkage facts each entity retains for code generation.
//! MSVC extension: Microsoft Learn, "dllexport, dllimport".

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

const WINDOWS: [Target; 2] = [Target::WindowsMsvc, Target::WindowsGnu];

fn microsoft(target: Target) -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_target(target)
        .with_gnu_extensions(true)
        .with_msvc_extensions(true)
}

/// The semantic diagnostic kinds `source` reports, in order.
fn kinds(source: &str, target: Target) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, microsoft(target), |context, _| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                kinds.push(error.kind);
            }
        }
    });
    kinds
}

/// The retained storage and inline linkage of `name`'s entity.
fn facts(source: &str, target: Target, name: &str) -> (Option<DllStorage>, Option<InlineOdr>) {
    let mut facts = (None, None);
    with_configuration(source, microsoft(target), |context, s| {
        let binding = s
            .bindings
            .iter()
            .rposition(|b| context.string_cache.at(b.name.name) == name)
            .unwrap_or_else(|| panic!("no binding {name}"));
        let attributes = s.entity_attributes(binding).copied().unwrap_or_default();
        facts = (attributes.dll, attributes.inline_odr);
    });
    facts
}

/// Clang's diagnostics, the same on both Windows targets.
#[test]
fn dll_attributes_follow_clang_applicability_and_redeclaration_rules() {
    use SemanticErrorKind as K;
    let cases: &[(&str, &[K])] = &[
        (
            "__declspec(dllimport) int f(void); int g(void) { return f(); }",
            &[],
        ),
        ("__declspec(dllexport) int f(void) { return 1; }", &[]),
        (
            "__declspec(dllimport) int x; int g(void) { return x; }",
            &[],
        ),
        (
            "__declspec(dllimport) int x = 1;",
            &[K::DllImportDataDefinition],
        ),
        (
            "__declspec(dllimport) void g(void) {}",
            &[K::DllImportFunctionDefinition],
        ),
        (
            "__declspec(dllimport) int f(void); __declspec(dllimport) int f(void) { return 1; }",
            &[K::DllImportFunctionDefinition],
        ),
        (
            "__declspec(dllimport) static int f(void);",
            &[K::DllInternal],
        ),
        (
            "static __declspec(dllexport) int f(void) { return 1; }",
            &[K::DllInternal],
        ),
        (
            "int main(void) { __declspec(dllimport) static int x; return x; }",
            &[K::DllInternal],
        ),
        (
            "__declspec(dllimport) __thread int x;",
            &[K::DllThreadLocal],
        ),
        (
            "__declspec(dllexport) _Thread_local int x;",
            &[K::DllThreadLocal],
        ),
        // One declaration's `dllexport` prevails over its `dllimport`, and
        // so does a redeclaration's.
        (
            "__declspec(dllexport) __declspec(dllimport) int f(void);",
            &[K::IgnoredAttribute],
        ),
        (
            "__declspec(dllimport) __declspec(dllexport) int f(void);",
            &[K::IgnoredAttribute],
        ),
        (
            "__declspec(dllimport) int f(void); __declspec(dllexport) int f(void);",
            &[K::IgnoredAttribute],
        ),
        (
            "__declspec(dllexport) int f(void); __declspec(dllimport) int f(void);",
            &[K::IgnoredAttribute],
        ),
        // Adding storage warns, or is an error once used, except an import
        // of a function, which its thunk still serves.
        (
            "int f(void); __declspec(dllimport) int f(void);",
            &[K::DllAttributeOnRedeclaration],
        ),
        (
            "int f(void); int g(void) { return f(); } __declspec(dllimport) int f(void);",
            &[K::DllAttributeOnRedeclaration],
        ),
        (
            "int f(void); int g(void) { return f(); } __declspec(dllexport) int f(void);",
            &[K::DllAttributeAfterUse],
        ),
        (
            "int x; int g(void) { return x; } __declspec(dllimport) extern int x;",
            &[K::DllAttributeAfterUse],
        ),
        (
            "int x; __declspec(dllexport) int x;",
            &[K::DllAttributeOnRedeclaration],
        ),
        // Only an evaluated reference uses the entity: a constant `sizeof`
        // operand does not, a VM one does.
        (
            "extern int x; int n = sizeof(x); __declspec(dllimport) extern int x; int g(void) { \
             return x; }",
            &[K::DllAttributeOnRedeclaration],
        ),
        (
            "int f(void); int n = sizeof(&f); __declspec(dllexport) int f(void) { return 1; }",
            &[K::DllAttributeOnRedeclaration],
        ),
        (
            "int f(void); int g(void) { return __alignof__(f()); } __declspec(dllexport) int \
             f(void) { return 1; }",
            &[K::DllAttributeOnRedeclaration],
        ),
        (
            "extern int x; int g(int n) { return sizeof(int[n][x]); } __declspec(dllexport) int x;",
            &[K::DllAttributeAfterUse],
        ),
        // A redeclaration without `dllimport` drops it, except at block
        // scope.
        (
            "__declspec(dllimport) int f(void); int f(void);",
            &[K::DllImportDropped],
        ),
        (
            "__declspec(dllimport) extern int x; extern int x;",
            &[K::DllImportDropped],
        ),
        (
            "__declspec(dllimport) int f(void); void g(void) { int f(void); f(); }",
            &[],
        ),
        // An attribute first written after the definition is ignored.
        (
            "int f(void) { return 1; } __declspec(dllexport) int f(void);",
            &[K::AttributeAfterDefinition],
        ),
        (
            "__declspec(dllexport) __attribute__((visibility(\"hidden\"))) int f(void) { return \
             1; }",
            &[K::DllExportHidden],
        ),
        // Anything but a function or object ignores it; a parameter keeps
        // it silently.
        (
            "__declspec(dllexport) typedef int T;",
            &[K::IgnoredAttribute],
        ),
        ("void h(__declspec(dllimport) int a);", &[]),
        // An imported object's address is not an address constant; an
        // imported function's import thunk is.
        (
            "__declspec(dllimport) int x; int *p = &x;",
            &[K::NonConstantInitializer],
        ),
        (
            "void g(void) { __declspec(dllimport) extern int x; static int *p = &x; }",
            &[K::NonConstantInitializer],
        ),
        (
            "__declspec(dllimport) int g(void); int (*p)(void) = &g;",
            &[],
        ),
        (
            "__declspec(dllimport) extern int x; int *f(void) { return &x; }",
            &[],
        ),
    ];
    for target in WINDOWS {
        for &(source, expected) in cases {
            assert_eq!(kinds(source, target), expected, "{target:?}: {source}");
        }
    }
}

/// The ABIs differ for inline functions and for a definition after an
/// imported declaration.
#[test]
fn dll_import_of_inline_functions_and_definitions_differ_by_abi() {
    use SemanticErrorKind as K;
    for (source, msvc, mingw) in [
        (
            "__declspec(dllimport) inline int f(void) { return 1; }",
            &[][..],
            &[K::DllImportOnInline][..],
        ),
        (
            "__declspec(dllimport) int f(void); inline int f(void) { return 1; }",
            &[],
            &[K::DllImportRedeclaredInline],
        ),
        (
            "__declspec(dllimport) int f(void); int f(void) { return 1; }",
            &[K::DllExportAdded],
            &[K::DllImportDropped],
        ),
        (
            "__declspec(dllimport) extern int x; int x = 2;",
            &[K::DllExportAdded],
            &[K::DllImportDropped],
        ),
        (
            "__declspec(dllimport) int x; int x;",
            &[K::DllExportAdded],
            &[K::DllImportDropped],
        ),
        (
            "__declspec(dllimport) extern int x; extern int x;",
            &[K::DllImportDropped],
            &[K::DllImportDropped],
        ),
    ] {
        assert_eq!(kinds(source, Target::WindowsMsvc), msvc, "MSVC: {source}");
        assert_eq!(kinds(source, Target::WindowsGnu), mingw, "MinGW: {source}");
    }
}

/// `__declspec` takes names exactly, as Clang does: the wrapped GNU
/// spelling is unsupported there on every target, while the GNU attribute
/// and MinGW's `__declspec` macro (without Microsoft extensions) unwrap it.
#[test]
fn wrapped_declspec_names_are_unsupported_while_gnu_spellings_apply() {
    for target in WINDOWS {
        for source in [
            "__declspec(__dllimport__) int f(void);",
            "__declspec(__dllexport__) int f(void) { return 1; }",
            "__declspec(__selectany__) int x = 1;",
            "__declspec(weak) int x;",
        ] {
            assert_eq!(
                kinds(source, target),
                [SemanticErrorKind::UnsupportedDeclspec],
                "{target:?}: {source}"
            );
        }
        assert_eq!(
            facts(
                "__declspec(__dllexport__) int f(void) { return 1; }",
                target,
                "f"
            )
            .0,
            None
        );
        assert_eq!(
            facts(
                "__attribute__((__dllexport__)) int f(void) { return 1; }",
                target,
                "f"
            )
            .0,
            Some(DllStorage::Export)
        );
    }
    let mingw = CompilerConfiguration::default()
        .with_target(Target::WindowsGnu)
        .with_gnu_extensions(true);
    with_configuration(
        "__declspec(__dllimport__) int f(void); int g(void) { return f(); }",
        mingw,
        |context, s| {
            assert_eq!(context.error_count(), 0);
            let f = s
                .bindings
                .iter()
                .position(|b| context.string_cache.at(b.name.name) == "f")
                .unwrap();
            assert_eq!(
                s.entity_attributes(f).and_then(|a| a.dll),
                Some(DllStorage::Import)
            );
        },
    );
    assert_eq!(
        kinds("__declspec(__dllimport__) int f(void);", Target::LinuxGnu),
        [SemanticErrorKind::UnsupportedDeclspec]
    );
}

/// Code generation exports what the declarations end exporting and imports
/// what the unit's last reference named imported, as Clang fixes a
/// declaration at its references.
#[test]
fn retained_storage_follows_the_last_reference_and_final_declarations() {
    for target in WINDOWS {
        for (source, expected) in [
            (
                "__declspec(dllimport) int f(void); int g(void) { return f(); }",
                Some(DllStorage::Import),
            ),
            (
                "__declspec(dllimport) int f(void); int f(void); int g(void) { return f(); }",
                None,
            ),
            (
                "__declspec(dllimport) int f(void); int g(void) { return f(); } int f(void);",
                Some(DllStorage::Import),
            ),
            (
                "int f(void); int g(void) { return f(); } __declspec(dllimport) int f(void);",
                None,
            ),
            (
                "__declspec(dllexport) int f(void); int f(void); int f(void) { return 1; }",
                Some(DllStorage::Export),
            ),
            (
                "__declspec(dllexport) int x; extern int x;",
                Some(DllStorage::Export),
            ),
            (
                "int main(void) { __declspec(dllimport) int f; return f; }",
                Some(DllStorage::Import),
            ),
            // Only evaluated references in emitted code fix the storage:
            // not one in an unreferenced static body, nor a `sizeof`.
            (
                "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                 static int unused(void) { return x; }",
                Some(DllStorage::Import),
            ),
            (
                "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; int \
                 n = sizeof(x);",
                Some(DllStorage::Import),
            ),
            (
                "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                 static int called(void) { return x; } int h(void) { return called(); }",
                None,
            ),
            (
                "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; \
                 __attribute__((used)) static int kept(void) { return x; }",
                None,
            ),
        ] {
            assert_eq!(
                facts(
                    source,
                    target,
                    if source.contains(" x") { "x" } else { "f" }
                )
                .0,
                expected,
                "{target:?}: {source}"
            );
        }
        // A `dllimport` object without a storage class is `extern`, at file
        // or block scope, so it is not a tentative definition.
        with_configuration(
            "__declspec(dllimport) int x; int main(void) { __declspec(dllimport) int y; return x \
             + y; }",
            microsoft(target),
            |context, s| {
                assert_eq!(context.error_count(), 0);
                for name in ["x", "y"] {
                    let binding = s
                        .bindings
                        .iter()
                        .position(|b| context.string_cache.at(b.name.name) == name)
                        .unwrap();
                    assert_eq!(s.bindings[binding].linkage, Linkage::External, "{name}");
                    assert!(
                        s.definitions.iter().all(|d| d.binding != binding),
                        "{name} is not defined"
                    );
                }
            },
        );
    }
}

/// Clang's `basicGVALinkageForFunction`: a Microsoft inline definition is
/// `linkonce_odr`, or `weak_odr` once an explicit `extern` declaration (at
/// any scope, before or after the body) or `dllexport` requests it; on
/// MinGW only `dllexport` gives an inline body that linkage. A body that is
/// not inline itself, a GNU inline body and an imported one have none.
#[test]
fn inline_definitions_retain_their_comdat_linkage() {
    use InlineOdr::{
        Discardable,
        Strong,
    };
    for (source, msvc, mingw) in [
        (
            "inline int f(void) { return 7; } int g(void) { return f(); }",
            Some(Discardable),
            None,
        ),
        (
            "inline int f(void) { return 7; } int f(void);",
            Some(Discardable),
            None,
        ),
        (
            "extern int f(void); inline int f(void) { return 7; }",
            Some(Strong),
            None,
        ),
        (
            "inline int f(void) { return 7; } extern int f(void);",
            Some(Strong),
            None,
        ),
        (
            "extern inline int f(void) { return 7; }",
            Some(Strong),
            None,
        ),
        (
            "inline int f(void); int f(void) { return 7; } extern int f(void);",
            Some(Strong),
            None,
        ),
        (
            "static void dead(void) { extern int f(void); } inline int f(void) { return 7; }",
            Some(Strong),
            None,
        ),
        ("int f(void) { return 7; } inline int f(void);", None, None),
        (
            "__declspec(dllexport) inline int f(void) { return 7; }",
            Some(Strong),
            Some(Strong),
        ),
        (
            "__declspec(dllexport) int f(void); inline int f(void) { return 7; }",
            Some(Strong),
            Some(Strong),
        ),
        (
            "inline __attribute__((gnu_inline)) int f(void) { return 7; }",
            None,
            None,
        ),
        (
            "__declspec(dllimport) inline int f(void) { return 7; } int g(void) { return f(); }",
            None,
            None,
        ),
    ] {
        assert_eq!(
            facts(source, Target::WindowsMsvc, "f").1,
            msvc,
            "MSVC: {source}"
        );
        assert_eq!(
            facts(source, Target::WindowsGnu, "f").1,
            mingw,
            "MinGW: {source}"
        );
    }
}

/// A MinGW exported inline body is an external `weak_odr` definition, not
/// a C99 inline-only one: it may reference internal functions and define
/// modifiable static objects, whether `dllexport` is on the body or an
/// earlier declaration. Without the export the C99 constraints remain.
/// C99: §6.7.4p3, p. 112; PDF p. 124.
#[test]
fn exported_mingw_inline_bodies_are_external_definitions() {
    use SemanticErrorKind as K;
    for (source, expected) in [
        (
            "static int helper(void) { return 7; } __declspec(dllexport) inline int f(void) { \
             return helper(); }",
            &[][..],
        ),
        (
            "__declspec(dllexport) inline int f(void); static int helper(void) { return 7; } \
             inline int f(void) { return helper(); }",
            &[],
        ),
        (
            "__declspec(dllexport) inline int f(void) { static int n; return ++n; }",
            &[],
        ),
        (
            "static int helper(void) { return 7; } inline int f(void) { return helper(); }",
            &[K::InlineInternalReference],
        ),
        (
            "inline int f(void) { static int n; return ++n; }",
            &[K::InlineStaticObject],
        ),
    ] {
        assert_eq!(kinds(source, Target::WindowsGnu), expected, "{source}");
        assert_eq!(kinds(source, Target::WindowsMsvc), [], "{source}");
    }
    assert_eq!(
        facts(
            "__declspec(dllexport) inline int f(void); inline int f(void) { static int n; return \
             ++n; }",
            Target::WindowsGnu,
            "f",
        ),
        (Some(DllStorage::Export), Some(InlineOdr::Strong))
    );
}

/// An imported inline body defines nothing, so an alias may not target it,
/// with `extern` on Microsoft (where the body would otherwise be strong) as
/// without it. MinGW ignores the `dllimport`, so `extern` makes the body an
/// external definition, and without it only C99 rules decide.
#[test]
fn aliases_may_not_target_imported_inline_bodies() {
    use SemanticErrorKind as K;
    for (prefix, mingw) in [
        ("", &[K::DllImportOnInline, K::AliasTarget][..]),
        ("extern ", &[K::DllImportOnInline]),
    ] {
        let source = format!(
            "__declspec(dllimport) {prefix}inline int f(void) {{ return 7; }} int a(void) \
             __attribute__((alias(\"f\")));"
        );
        assert_eq!(
            kinds(&source, Target::WindowsMsvc),
            [K::AliasTarget],
            "{source}"
        );
        assert_eq!(kinds(&source, Target::WindowsGnu), mingw, "{source}");
    }
}
