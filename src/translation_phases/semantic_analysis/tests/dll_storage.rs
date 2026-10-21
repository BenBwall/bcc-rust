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

/// A reference fixes an entity's import only where Clang's code generation
/// emits it at `-O0`: not in the arm a constant condition skips, after a
/// jump before the next label, in a switch body before its first case, nor
/// in a body the unit does not emit. A constant-false loop still emits its
/// body, and a label resumes emission.
#[test]
fn skipped_code_and_unemitted_bodies_keep_the_import() {
    const IMPORTED: &str =
        "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; ";
    for (suffix, msvc, mingw) in [
        ("int h(void) { if (0) return x; return 0; }", true, true),
        (
            "int h(void) { if (1) return 0; else return x; }",
            true,
            true,
        ),
        ("int h(void) { if (1) return 0; return x; }", true, true),
        ("int h(void) { return 0 ? x : 0; }", true, true),
        ("int h(void) { return 1 ? 0 : x; }", true, true),
        ("int h(void) { return 0 && x; }", true, true),
        ("int h(void) { return 1 || x; }", true, true),
        ("int h(void) { return 0; return x; }", true, true),
        (
            "int h(int a) { if (a) return 0; else return 1; return x; }",
            true,
            true,
        ),
        (
            "int h(void) { for (;;) { break; return x; } return 0; }",
            true,
            true,
        ),
        (
            "int h(int a) { switch (a) { return x; case 1: return 0; } return 0; }",
            true,
            true,
        ),
        (
            "int h(int a) { return 0; switch (a) { case 1: return x; } }",
            true,
            true,
        ),
        ("inline int h(void) { return x; }", true, true),
        ("static inline int h(void) { return x; }", true, true),
        (
            "inline int h(void) { return x; } static int k(void) { return h(); }",
            true,
            true,
        ),
        (
            "static int h(void) { return x; } int k(void) { if (0) return h(); return 0; }",
            true,
            true,
        ),
        // Emitted references drop it.
        (
            "int h(void) { while (0) return x; return 0; }",
            false,
            false,
        ),
        ("int h(void) { do return x; while (0); }", false, false),
        ("int h(void) { return 1 && x; }", false, false),
        ("int h(void) { return 0; L: return x; }", false, false),
        ("int h(int a) { if (a) return 0; return x; }", false, false),
        (
            "int h(int a) { switch (a) { case 1: return x; } return 0; }",
            false,
            false,
        ),
        (
            "int h(void) { if (0) { L: return x; } goto L; }",
            false,
            false,
        ),
        (
            "int k(void) { return ({ return 0; 1; }) + x; }",
            false,
            false,
        ),
        (
            "static int h(void) { return x; } int k(void) { return h(); }",
            false,
            false,
        ),
        (
            "inline int h(void) { return x; } extern int h(void);",
            false,
            false,
        ),
        // A referenced inline body is emitted on Microsoft only, where it is
        // a discardable external definition; MinGW calls the external one.
        (
            "inline int h(void) { return x; } int k(void) { return h(); }",
            false,
            true,
        ),
    ] {
        let source = format!("{IMPORTED}{suffix}");
        for (target, imported) in [(Target::WindowsMsvc, msvc), (Target::WindowsGnu, mingw)] {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {suffix}"
            );
        }
    }
}

/// Clang's generated-reference state depends on emitted control edges,
/// including labels and breaks, rather than semantic reachability. These
/// expectations are pinned against Clang 23 on both Windows ABIs at -O0.
#[test]
fn labels_loop_exits_and_floating_conditions_fix_generated_import_state() {
    const IMPORTED: &str =
        "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; ";
    for (suffix, imported) in [
        // A label prevents elision of the whole constant-dead arm.
        ("int h(void) { if (0) { x; L: return 0; } goto L; }", false),
        (
            "int h(void) { if (0) { x; if (1) { L: return 0; } } goto L; }",
            false,
        ),
        (
            "int h(int n) { switch (n) { if (0) { x; case 1: return 0; } } return 0; }",
            false,
        ),
        // A nested switch's own case labels do not keep an outer dead arm.
        (
            "int h(int n) { if (0) { x; switch (n) { case 1: return 0; } } return 0; }",
            true,
        ),
        ("int h(void) { for (;;) {} return x; }", true),
        ("int h(void) { while (1) {} return x; }", true),
        ("int h(void) { while (1.0) {} return x; }", true),
        (
            "int h(void) { for (;;) { sizeof(({ break; 0; })); } return x; }",
            true,
        ),
        ("int h(void) { for (;;) { if (0) break; } return x; }", true),
        (
            "int h(int n) { for (;;) { if (n) break; } return x; }",
            false,
        ),
        (
            "int h(void) { while (1) { while (0) { break; } } return x; }",
            true,
        ),
        (
            "int h(int n) { while (1) { switch (n) { case 1: break; } } return x; }",
            true,
        ),
        (
            "int h(void) { for (;;) { return 0; L: break; } return x; }",
            false,
        ),
        (
            "int h(void) { while (1) { goto L; } x; L: return 0; }",
            true,
        ),
        ("int h(void) { for (;;) {} L: return x; }", false),
        // Clang generates condition exits for these forms at -O0, even
        // when the condition is constant and an optimizer removes them.
        ("int h(void) { for (;1;) {} return x; }", false),
        ("int h(void) { do {} while (1); return x; }", false),
        ("int h(void) { while (0) return x; return 0; }", false),
        // Bare floating conditions generate both arms. Integer results
        // computed from floating constants can elide their skipped arms.
        ("int h(void) { if (0.0) return x; return 0; }", false),
        ("int h(void) { if (0.5) return 0; return x; }", false),
        ("int h(void) { return 0.0 && x; }", false),
        ("int h(void) { return -0.0 ? x : 0; }", false),
        ("int h(void) { if ((int)0.0) return x; return 0; }", true),
        ("int h(void) { if (0.0 == 0.0) return 0; return x; }", true),
    ] {
        let source = format!("{IMPORTED}{suffix}");
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {suffix}"
            );
        }
    }
}

/// A folded switch emits only its selected statements, and an expression
/// statement ending in a call that does not return skips what follows, so
/// references there keep the import; references Clang emits drop it.
/// These expectations are pinned against Clang 23 on both Windows ABIs at
/// -O0 (#113). C99: §6.8.4.2p5, p. 134; PDF p. 146.
#[test]
fn folded_switches_and_calls_that_do_not_return_fix_generated_import_state() {
    const IMPORTED: &str = "__declspec(dllimport) extern int x; int g(void) { return x; } extern \
                            int x; __attribute__((noreturn)) void die(void); void exit(int); ";
    for (suffix, imported) in [
        (
            "int h(void) { switch (0) { case 1: return x; default: break; } return 0; }",
            true,
        ),
        (
            "int h(void) { switch (5) { case 1: return x; } return 0; }",
            true,
        ),
        (
            "int h(void) { switch (1) { case 1: return 0; case 2: return x; } return 0; }",
            true,
        ),
        (
            "int h(void) { switch (1) { case 1: return 0; } return x; }",
            true,
        ),
        (
            "int h(void) { switch (1) { case 1: break; } return x; }",
            false,
        ),
        (
            "int h(void) { switch (2) { case 1: break; case 2: return x; } return 0; }",
            false,
        ),
        (
            "int h(void) { switch (5) { case 1: break; default: return x; } return 0; }",
            false,
        ),
        (
            "int h(int a) { switch (1) { case 0: x; if (a) { case 1: ; } } return 0; }",
            false,
        ),
        (
            "int h(void) { switch ((0, 1)) { case 0: return x; case 1: break; } return 0; }",
            true,
        ),
        (
            "int h(void) { goto L; switch (0) { case 1: L: return x; default: break; } return 0; }",
            false,
        ),
        (
            "int h(void) { switch (5) { default: return 0; case 1: return x; } }",
            true,
        ),
        ("int h(void) { die(); return x; }", true),
        ("int h(void) { exit(0); return x; }", true),
        ("int h(void) { die(), x; return 0; }", false),
        ("int h(int a) { a ? die() : (void)0; return x; }", false),
        ("int h(void) { 1 ? die() : (void)0; return x; }", true),
        ("int h(void) { die(); L: return x; }", false),
        ("int h(void) { int y = (die(), 1); return x + y; }", false),
        ("int h(void) { ({ die(); }); return x; }", true),
        ("int h(void) { return ({ die(); x; }); }", false),
        (
            "int h(void) { void (*p)(void) = die; p(); return x; }",
            false,
        ),
        (
            "int h(void) { void (*p)(void) __attribute__((noreturn)) = die; p(); return x; }",
            true,
        ),
    ] {
        let source = format!("{IMPORTED}{suffix}");
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {suffix}"
            );
        }
    }
}

/// The forms the #113 review found counted differently from Clang, pinned
/// against Clang 23 on both Windows ABIs at -O0: labels and breaks in
/// statement expressions keep a switch unfolded, standard `[[noreturn]]`
/// stays with its declaration, a hand-declared `longjmp` is a builtin only
/// after `jmp_buf`, a complex product joins paths, unevaluated operands
/// change nothing, function types carry `noreturn` through any callee, and
/// Clang's evaluator folds conditions semantic analysis does not.
#[test]
fn review_forms_fix_generated_import_state() {
    const IMPORTED: &str = "__declspec(dllimport) extern int x; int g(void) { return x; } extern \
                            int x; __attribute__((noreturn)) void die(void); ";
    const NP: &str = "typedef void (*NP)(void) __attribute__((noreturn)); ";
    for (prefix, suffix, imported) in [
        (
            "",
            "int h(void) { switch (1) { case 1: ({ break; 0; }); case 2: x; } return 0; }",
            false,
        ),
        (
            "",
            "int h(void) { switch (1) { case 1: break; case 2: ({ label: 0; }); x; } return 0; }",
            false,
        ),
        (
            "",
            "int h(void) { switch (1) { case 1: break; case 2: { int v = ({ lab: 0; }); x; } } \
             return 0; }",
            false,
        ),
        (
            "",
            "[[noreturn]] void d2(void); __typeof__(d2) *p; int h(void) { p(); x; return 0; }",
            false,
        ),
        (
            "",
            "struct __jmp_buf_tag; void longjmp(struct __jmp_buf_tag *, int); struct \
             __jmp_buf_tag *jp; int h(void) { longjmp(jp, 1); x; return 0; }",
            false,
        ),
        (
            "",
            "typedef long jmp_buf[8]; void longjmp(long *, int); int h(void) { longjmp(0, 1); x; \
             return 0; }",
            true,
        ),
        (
            "",
            "int h(void) { _Complex double z = 1; die(), (z * z); x; return 0; }",
            false,
        ),
        (
            "",
            "int c; int h(void) { die(), sizeof(c ? 1 : 0); x; return 0; }",
            true,
        ),
        (NP, "NP a[1]; int h(void) { a[0](); x; return 0; }", true),
        (
            NP,
            "NP getnp(void); int h(void) { getnp()(); x; return 0; }",
            true,
        ),
        (
            NP,
            "NP p1, p2; int c; int h(void) { (c ? p1 : p2)(); x; return 0; }",
            true,
        ),
        (
            NP,
            "NP p1; void (*p2)(void); int c; int h(void) { (c ? p1 : p2)(); x; return 0; }",
            false,
        ),
        (
            "",
            "struct U { struct { void (*f)(void) __attribute__((noreturn)); }; } u; int h(void) { \
             u.f(); x; return 0; }",
            true,
        ),
        (
            "",
            "int h(void) { switch ((0.0, 1)) { case 1: break; default: x; } return 0; }",
            true,
        ),
        (
            "",
            "int h(void) { switch (({ 1; })) { case 1: break; default: x; } return 0; }",
            true,
        ),
        (
            "",
            "int c; int h(void) { switch (&c != 0) { case 1: break; default: x; } return 0; }",
            true,
        ),
        (
            "",
            "int h(void) { switch (1) { case 1: break; int v; case 2: x; } return 0; }",
            true,
        ),
        (
            "",
            "int c; int h(void) { if (__builtin_constant_p(c)) x; return 0; }",
            false,
        ),
        (
            "",
            "int h(void) { if (__builtin_constant_p(1) - 1) x; return 0; }",
            true,
        ),
        (
            "",
            "int h(void) { const int k = 0; if (k) x; return 0; }",
            true,
        ),
    ] {
        let source = format!("{IMPORTED}{prefix}{suffix}");
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {suffix}"
            );
        }
    }
}

/// The forms the second #113 review found counted differently from Clang,
/// pinned against Clang 23 on both Windows ABIs at -O0: a `default` before
/// a matching case whose collection fails skips nothing, a pointer
/// condition does not fold, the join rules read the expression's own type,
/// a `void` operand is no constant, a decrement of an atomic `_Bool` loops,
/// a run-time memory order switches unless a library call copies the
/// object, and a volatile read through a pointer is a side effect. From
/// the #104 review: string literal elements, `const` aggregates and
/// `__builtin_strlen` fold (`strlen` only with the builtin's type, which
/// `unsigned long` is not on LLP64). A variably modified declaration after
/// a jump resumes nothing, which keeps the import where Clang never reaches
/// the declaration, as under a union read bcc does not fold. A statement
/// expression in an operand a folded condition skips skips nothing after
/// its expression statement.
#[test]
fn second_review_forms_fix_generated_import_state() {
    const IMPORTED: &str = "__declspec(dllimport) extern int x; int g(void) { return x; } extern \
                            int x; __attribute__((noreturn)) void die(void); int c; int \
                            tick(void); ";
    for (suffix, imported) in [
        (
            "int h(void) { switch (2) { case 1: x; break; default: tick(); break; case 2: if (c) \
             break; tick(); break; } return 0; }",
            false,
        ),
        (
            "int h(void) { switch (3) { case 1: x; break; default: tick(); break; case 2: tick(); \
             break; } return 0; }",
            true,
        ),
        (
            "int h(void) { switch (2) { case 1: x; break; default: break; case 2: break; } return \
             0; }",
            true,
        ),
        ("int h(void) { if ((void *)0) x; return 0; }", false),
        (
            "int h(void) { int *const p = 0; if (p) x; return 0; }",
            false,
        ),
        (
            "int h(void) { die(), ((void *)0 && c); x; return 0; }",
            false,
        ),
        ("int h(void) { if ((void *)0 != 0) x; return 0; }", true),
        (
            "_Complex double z; int h(void) { die(), (1 ? z : z); x; return 0; }",
            false,
        ),
        (
            "struct s { int a; } s1, s2; int h(void) { die(), (1 ? s1 : s2); x; return 0; }",
            false,
        ),
        (
            "int h(void) { die(), (c ? (void)0 : (void)0); x; return 0; }",
            false,
        ),
        (
            "_Atomic _Bool b; int h(void) { die(), b--; x; return 0; }",
            false,
        ),
        (
            "_Atomic _Bool b; int h(void) { die(), b++; x; return 0; }",
            true,
        ),
        (
            "int e; int h(void) { die(), __atomic_load_n(&e, c); x; return 0; }",
            false,
        ),
        (
            "int e; int h(void) { const int o = 5; die(), __atomic_load_n(&e, o); x; return 0; }",
            true,
        ),
        (
            "struct b { char a[24]; } b1, b2; int h(void) { die(), __atomic_load(&b1, &b2, c); x; \
             return 0; }",
            true,
        ),
        (
            "int h(void) { die(), __atomic_thread_fence(c); x; return 0; }",
            false,
        ),
        (
            "volatile int *vp; int h(void) { if (__builtin_constant_p(*vp)) x; return 0; }",
            true,
        ),
        (
            "struct v { volatile int m; } *vq; int h(void) { if (__builtin_constant_p(vq->m)) x; \
             return 0; }",
            true,
        ),
        (
            "volatile int vx; int h(void) { if (__builtin_constant_p(&vx)) x; return 0; }",
            false,
        ),
        ("int h(void) { if (\"ab\"[1] - 'b') x; return 0; }", true),
        (
            "int h(void) { static const int a[2] = {0, 1}; if (a[0]) x; return 0; }",
            true,
        ),
        (
            "struct p { int x, y; }; int h(void) { const struct p q = {0, 1}; if (q.x) x; return \
             0; }",
            true,
        ),
        (
            "int h(void) { if (__builtin_strlen(\"abc\") == 0) x; return 0; }",
            true,
        ),
        (
            "unsigned long strlen(const char *); int h(void) { if (strlen(\"abc\") == 0) x; \
             return 0; }",
            false,
        ),
        ("int h(int n) { return 0; if (c) { int v[n]; x; } }", true),
        ("int h(int n) { die(); int v[n]; x; return 0; }", false),
        ("int h(void) { (void)(0 && ({ 5; })); x; return 0; }", false),
        ("int h(void) { (void)(1 || ({ 5; })); x; return 0; }", false),
        (
            "int h(void) { (void)(0 ? ({ 5; }) : 0); x; return 0; }",
            false,
        ),
        (
            "int h(void) { (void)(1 ? 0 : ({ 5; })); x; return 0; }",
            false,
        ),
        (
            "int h(void) { (void)(0 ? ({ return 99; 0; }) : 0); x; return 0; }",
            false,
        ),
        ("int h(void) { ({ return 0; 1; }); x; return 0; }", true),
        (
            "static const union { int i; float f; } u = {1}; int h(int n) { if (u.i) {} else { \
             return 0; int a[n]; x; } return 0; }",
            true,
        ),
    ] {
        let source = format!("{IMPORTED}{suffix}");
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {suffix}"
            );
        }
    }
    // `long double` is `double` on MSVC, which `atomicrmw` adds to; MinGW's
    // x87 `long double` loops.
    let source =
        format!("{IMPORTED}_Atomic long double ld; int h(void) {{ die(), ld++; x; return 0; }}");
    assert_eq!(
        facts(&source, Target::WindowsMsvc, "x").0,
        Some(DllStorage::Import)
    );
    assert_eq!(facts(&source, Target::WindowsGnu, "x").0, None);
}

/// Generated-reference filtering does not change semantic uses: an internal
/// function still needs a definition even after an infinite loop.
/// C99: §6.9p3, p. 140; PDF p. 152.
#[test]
fn generated_loop_exits_leave_semantic_reference_constraints_unchanged() {
    for target in WINDOWS {
        let source = "static int absent(void); int h(void) { for (;;) {} return absent(); }";
        with_configuration(source, microsoft(target), |context, sema| {
            let absent = sema
                .bindings
                .iter()
                .position(|binding| context.string_cache.at(binding.name.name) == "absent")
                .unwrap();
            assert!(
                sema.references.iter().any(|usage| usage.binding == absent),
                "{target:?}: semantic reference was filtered"
            );
            assert!(context.take_pending_errors().iter().any(|error| {
                matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::UndefinedInternal)
            }), "{target:?}: semantic definition constraint was filtered");
        });
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

#[test]
fn aliases_and_initializers_root_generated_dll_references() {
    let prefix =
        "__declspec(dllimport) extern int x; int before(void){return x;} extern int x; int sink;";
    for target in WINDOWS {
        for (body, imported) in [
            ("static int h(void){return x;}", true),
            (
                "static int h(void){return x;} int a(void) __attribute__((alias(\"h\")));",
                false,
            ),
            (
                "static int h(void){return x;} int owner(void){int a(void) \
                 __attribute__((alias(\"h\")));return 0;}",
                true,
            ),
            (
                "__attribute__((constructor)) static void h(void){sink=x;}",
                false,
            ),
            (
                "__attribute__((destructor)) static void h(void){sink=x;}",
                false,
            ),
            (
                "static int helper(void){return x;} static int h(void){return helper();} int \
                 a(void) __attribute__((alias(\"h\")));",
                false,
            ),
        ] {
            let source = format!("{prefix} {body}");
            assert!(
                kinds(&source, target)
                    .iter()
                    .all(|&kind| kind == SemanticErrorKind::DllImportDropped),
                "{target:?} {source}"
            );
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?} {source}"
            );
        }
    }
    for attribute in ["constructor", "destructor"] {
        let source =
            format!("{prefix} __attribute__(({attribute})) inline void h(void){{sink=x;}}");
        assert_eq!(facts(&source, Target::WindowsMsvc, "x").0, None, "{source}");
    }
}

/// The forms PR #117's first review found, against Clang 23 on both
/// Windows ABIs at -O0: a weak `const` object's value is never read, and a
/// `noreturn` type named by a typedef belongs below the declarator's
/// function derivations, so these references are emitted and drop the
/// import. Lowering still emits the code after a call that does not
/// return and a folded switch's unselected statements, which Clang skips;
/// when every reference is there, as in a static function only such code
/// calls, they keep the import they were written with, or the object they
/// name would not link.
#[test]
fn first_pr_review_forms_fix_generated_import_state() {
    const IMPORTED: &str =
        "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; ";
    for (source, imported) in [
        (
            "const int wk __attribute__((weak)) = 1; int h(void) { switch (wk) { case 1: break; \
             default: return x; } return 0; }",
            false,
        ),
        (
            "const int wk __attribute__((weak)) = 0; int h(void) { if (wk) return x; return 0; }",
            false,
        ),
        (
            "typedef void (*NP)(void) __attribute__((noreturn)); NP (*gp)(void); int h(void) { \
             gp(); return x; }",
            false,
        ),
        (
            "typedef void (*NP)(void) __attribute__((noreturn)); int h(NP p) { p(); return x; }",
            true,
        ),
    ] {
        let source = format!("{IMPORTED}{source}");
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {source}"
            );
        }
    }
    for source in [
        "__declspec(dllimport) extern int x; __attribute__((noreturn)) void die(void); int \
         h(void) { die(); return x; } extern int x;",
        "__declspec(dllimport) extern int x; int h(void) { switch (0) { case 1: return x; \
         default: break; } return 0; } extern int x;",
        "__declspec(dllimport) extern int x; int h(void) { if (\"a\"[0] == 98) return x; return \
         0; } extern int x;",
        "__declspec(dllimport) extern int x; __attribute__((noreturn)) void die(void); static int \
         s(void) { return x; } int h(void) { die(); return s(); } extern int x;",
    ] {
        for target in WINDOWS {
            assert_eq!(
                facts(source, target, "x").0,
                Some(DllStorage::Import),
                "{target:?}: {source}"
            );
        }
    }
}

/// The forms PR #117's second review found, against Clang 23 on both
/// Windows ABIs at -O0. The address of a `dllimport` object is no constant
/// result, a weak entity by `#pragma weak`, `weak_import` or a later
/// declaration does not fold in a body Clang emits after the unit, and the
/// Microsoft ABI emits a call's arguments from the last, so a join in an
/// earlier one follows a call that does not return in a later one: the
/// reference is emitted and drops the import. Where Clang references
/// nothing but lowering still emits code, any reference in a body lowering
/// emits that names the import keeps it, however later references were
/// written, and one in a body nothing emits decides nothing.
#[test]
fn second_pr_review_forms_fix_generated_import_state() {
    const IMPORTED: &str =
        "__declspec(dllimport) extern int x; int g(void) { return x; } extern int x; ";
    for (source, msvc, mingw) in [
        (
            "__declspec(dllimport) extern int di; int h(void) { int *const dp = &di; if (!dp) \
             return x; return 0; }",
            false,
            false,
        ),
        (
            "__declspec(dllimport) extern int d1, d2; void sinkp(int *); \
             __attribute__((noreturn)) void die(void); int c; int h(void) { die(), sinkp(c ? &d1 \
             : &d2); return x; }",
            false,
            false,
        ),
        (
            "int sink2(int, int); int tick(void); int c; __attribute__((noreturn)) void \
             die(void); int h(void) { sink2(c && tick(), (die(), 0)); return x; }",
            false,
            true,
        ),
        (
            "int sink3(int, int, int); int tick(void); int c; __attribute__((noreturn)) void \
             die(void); int h(void) { sink3(c && tick(), 0, (die(), 0)); return x; }",
            false,
            true,
        ),
        (
            "void wl(void); static int helper(void) { if (!wl) return x; return 0; } void \
             wl(void) __attribute__((weak)); int probe(void) { return helper(); }",
            false,
            false,
        ),
        (
            "void wf(void);\n#pragma weak wf\nint h(void) { if (!wf) return x; return 0; }",
            false,
            false,
        ),
        (
            "void wi(void) __attribute__((weak_import)); int h(void) { if (!wi) return x; return \
             0; }",
            false,
            false,
        ),
    ] {
        let source = format!("{IMPORTED}{source}");
        for (target, imported) in [(Target::WindowsMsvc, msvc), (Target::WindowsGnu, mingw)] {
            assert_eq!(
                facts(&source, target, "x").0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {source}"
            );
        }
    }
    const SKIPPED: &str = "__declspec(dllimport) extern int x; __attribute__((noreturn)) void \
                           die(void); int h(void) { die(); return x; } extern int x; ";
    for (source, name, imported) in [
        (
            format!("{SKIPPED}int k(void) {{ if (0) return x; return 0; }}"),
            "x",
            true,
        ),
        (
            format!("{SKIPPED}inline int k(void) {{ return x; }}"),
            "x",
            true,
        ),
        (
            format!("{SKIPPED}static int unused(void) {{ return x; }}"),
            "x",
            true,
        ),
        (
            format!("{SKIPPED}int k(void) {{ die(); return x; }}"),
            "x",
            true,
        ),
        (
            "__declspec(dllimport) extern int x; int h(void) { switch (2) { case 1: return x; \
             case 2: break; } return 0; } extern int x; int k(void) { if (0) return x; return 0; }"
                .to_owned(),
            "x",
            true,
        ),
        (
            "void fn(void); __attribute__((noreturn)) void die(void); int h(void) { die(); fn(); \
             return 0; } __declspec(dllimport) void fn(void); static void u(void) { fn(); }"
                .to_owned(),
            "fn",
            false,
        ),
    ] {
        for target in WINDOWS {
            assert_eq!(
                facts(&source, target, name).0,
                imported.then_some(DllStorage::Import),
                "{target:?}: {source}"
            );
        }
    }
}
