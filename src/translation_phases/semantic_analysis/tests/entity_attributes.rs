//! GNU entity attributes: retained symbol facts, cleanup regions, and the
//! applicability and jump diagnostics Clang gives (bundled Clang 23 on
//! x86_64-unknown-linux-gnu and x86_64-w64-windows-gnu; GCC 15 for the
//! priority range).

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

fn gnu(target: Target) -> CompilerConfiguration {
    CompilerConfiguration::default()
        .with_target(target)
        .with_gnu_extensions(true)
}

/// The semantic diagnostic kinds `source` reports, in order.
fn kinds(source: &str, target: Target) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, gnu(target), |context, _| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                kinds.push(error.kind);
            }
        }
    });
    kinds
}

fn binding<'a>(context: &Context<'_>, s: &'a SemanticTranslationUnit<'_>, name: &str) -> usize {
    s.bindings
        .iter()
        .rposition(|b| context.string_cache.at(b.name.name) == name)
        .unwrap_or_else(|| panic!("no binding {name}"))
}

#[test]
fn entity_attributes_merge_over_declarations_and_resolve_aliases() {
    let source = r#"
int real(int x) { return x; }
int first(int) __attribute__((alias("second")));
int second(int) __attribute__((alias("real")));
extern int value __attribute__((weak));
__attribute__((section(".data.mine"), used)) int stored = 1;
static int keep __attribute__((used));
__attribute__((visibility("hidden"))) int hidden_object;
__attribute__((aligned(32))) void aligned_function(void) {}
void late(void);
void late(void) __attribute__((weak));
void late(void) {}
"#;
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_configuration(source, gnu(target), |context, s| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            let attributes = |name| s.entity_attributes(binding(context, s, name)).copied();
            assert!(attributes("value").unwrap().weak);
            assert!(attributes("late").unwrap().weak);
            let stored = attributes("stored").unwrap();
            assert_eq!(stored.section, Some(".data.mine"));
            assert!(stored.used && !stored.weak);
            assert!(attributes("keep").unwrap().used);
            assert_eq!(
                attributes("hidden_object").unwrap().visibility,
                Some(Visibility::Hidden)
            );
            assert_eq!(attributes("aligned_function").unwrap().code_alignment, 32);
            assert!(attributes("real").is_none());
            // Chains resolve to the final definition.
            assert_eq!(s.symbols.aliases.len(), 2);
            for alias in s.symbols.aliases {
                assert_eq!(
                    context.string_cache.at(s.bindings[alias.target].name.name),
                    "real"
                );
            }
            assert!(s.symbols.refusals.is_empty());
        });
    }
}

#[test]
fn constructors_and_destructors_register_definitions_in_order() {
    let source = r"
__attribute__((constructor(300))) static void a(void) {}
void b(void) __attribute__((destructor));
__attribute__((constructor, destructor(200))) void c(void) {}
void b(void) {}
__attribute__((constructor)) void declared_only(void);
__attribute__((constructor(1 + 200))) void d() {}
";
    with_configuration(source, gnu(Target::LinuxGnu), |context, s| {
        assert_eq!(
            context.error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let registered: Vec<_> = s
            .symbols
            .init_functions
            .iter()
            .map(|init| {
                (
                    context.string_cache.at(s.bindings[init.binding].name.name),
                    init.kind,
                    init.priority,
                )
            })
            .collect();
        assert_eq!(
            registered,
            [
                ("a", InitFunctionKind::Constructor, 300),
                ("c", InitFunctionKind::Constructor, 65_535),
                ("c", InitFunctionKind::Destructor, 200),
                ("b", InitFunctionKind::Destructor, 65_535),
                ("d", InitFunctionKind::Constructor, 201),
            ]
        );
    });
}

#[test]
fn cleanup_regions_follow_declarations_and_their_initializers() {
    let source = r"
void drop(int *p);
void release(void *p);
int value(void);
void f(void) {
    int a __attribute__((cleanup(drop))) = value();
    {
        long b __attribute__((cleanup(release)));
        int c __attribute__((cleanup(drop))) = a;
    }
}
";
    with_configuration(source, gnu(Target::LinuxGnu), |context, s| {
        assert_eq!(
            context.error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let calls: Vec<_> = s
            .control
            .regions
            .iter()
            .filter_map(|region| match (region.kind, region.cleanup) {
                | (
                    RegionKind::CleanupDeclaration(variable),
                    CleanupKind::Call {
                        variable: called,
                        function,
                    },
                ) => {
                    assert_eq!(variable, called);
                    Some((
                        context.string_cache.at(s.bindings[variable.0].name.name),
                        context.string_cache.at(s.bindings[function.0].name.name),
                    ))
                },
                | _ => None,
            })
            .collect();
        assert_eq!(calls, [("a", "drop"), ("b", "release"), ("c", "drop")]);
        // `c`'s region is inside `b`'s: its cleanup link reaches `b`'s.
        let c = s
            .control
            .regions
            .iter()
            .position(|region| {
                matches!(region.kind, RegionKind::CleanupDeclaration(v) if context.string_cache.at(s.bindings[v.0].name.name) == "c")
            })
            .unwrap();
        let parent = s.control.regions[c].parent.unwrap();
        let outer = s.control.regions[parent.0].nearest_cleanup.unwrap();
        assert!(matches!(
            s.control.regions[outer.0].kind,
            RegionKind::CleanupDeclaration(v) if context.string_cache.at(s.bindings[v.0].name.name) == "b"
        ));
        // The initializer `value()` executes before `a`'s region opens.
        let call = s
            .expressions
            .iter()
            .find(|info| matches!(info.expression.kind, ExpressionType::Call { .. }))
            .unwrap();
        let point = s.control.expression_point(call.expression).unwrap();
        assert!(s.control.regions[point.region.0].nearest_cleanup.is_none());
    });
}

#[test]
fn attribute_diagnostics_match_clang() {
    use SemanticErrorKind as K;
    let cases: &[(&str, &[SemanticErrorKind])] = &[
        ("__attribute__((frobnicate)) int x;", &[K::UnknownAttribute]),
        ("[[vendor::frobnicate]] int x;", &[K::UnknownAttribute]),
        ("static int x __attribute__((weak));", &[K::WeakInternal]),
        (
            "void f(void) { int x __attribute__((weak)); }",
            &[K::WeakInternal],
        ),
        (
            "typedef int T __attribute__((weak));",
            &[K::IgnoredAttribute],
        ),
        ("int x __attribute__((weak(1)));", &[K::AttributeArguments]),
        (
            "int t = 1; int a __attribute__((alias(\"t\")));",
            &[K::AliasDefinition],
        ),
        (
            "extern int a __attribute__((alias(\"missing\")));",
            &[K::AliasTarget],
        ),
        (
            "extern int d; extern int a __attribute__((alias(\"d\")));",
            &[K::AliasTarget],
        ),
        (
            "void f(void) {} extern int a __attribute__((alias(\"f\")));",
            &[K::AliasKind],
        ),
        (
            "extern int a __attribute__((alias(\"b\"))); extern int b \
             __attribute__((alias(\"a\")));",
            &[K::AliasCycle, K::AliasCycle],
        ),
        (
            "void f(void) {} void a(int) __attribute__((alias(\"f\")));",
            &[K::AliasTypeMismatch],
        ),
        (
            "extern int a __attribute__((alias(t)));",
            &[K::AttributeArguments],
        ),
        (
            "void f(void) { int x __attribute__((alias(\"f\"))); }",
            &[K::AttributeSubject],
        ),
        (
            "void f(void) { int x __attribute__((section(\"s\"))); }",
            &[K::AttributeSubject],
        ),
        (
            "void f(void) { int x __attribute__((used)); }",
            &[K::IgnoredAttribute],
        ),
        (
            "void f(void) __attribute__((section(\"a\"))); void f(void) \
             __attribute__((section(\"b\")));",
            &[K::SectionMismatch],
        ),
        (
            "int x __attribute__((visibility(\"bogus\")));",
            &[K::InvalidVisibility],
        ),
        (
            "__attribute__((constructor(50))) void f(void) {}",
            &[K::ReservedPriority],
        ),
        (
            "__attribute__((constructor(65536))) void f(void) {}",
            &[K::InvalidPriority],
        ),
        (
            "__attribute__((destructor(-1))) void f(void) {}",
            &[K::InvalidPriority],
        ),
        (
            "__attribute__((constructor(\"x\"))) void f(void) {}",
            &[K::AttributeRequiresConstant],
        ),
        (
            "__attribute__((constructor)) int x;",
            &[K::IgnoredAttribute],
        ),
        (
            "void d(int *); int x __attribute__((cleanup(d)));",
            &[K::IgnoredAttribute],
        ),
        (
            "void d(int *); void f(void) { static int x __attribute__((cleanup(d))); }",
            &[K::IgnoredAttribute],
        ),
        (
            "void d(int *); void f(int p __attribute__((cleanup(d)))) {}",
            &[K::IgnoredAttribute],
        ),
        (
            "void f(void) { int x __attribute__((cleanup(nope))); }",
            &[K::UndeclaredIdentifier],
        ),
        (
            "int o; void f(void) { int x __attribute__((cleanup(o))); }",
            &[K::CleanupNotFunction],
        ),
        (
            "void f(void) { int x __attribute__((cleanup(\"f\"))); }",
            &[K::CleanupNotFunction],
        ),
        (
            "void d(int *, int); void f(void) { int x __attribute__((cleanup(d))); }",
            &[K::CleanupParameterCount],
        ),
        (
            "void d(); void f(void) { int x __attribute__((cleanup(d))); }",
            &[K::CleanupParameterCount],
        ),
        (
            "void d(unsigned *); void f(void) { int x __attribute__((cleanup(d))); }",
            &[K::CleanupParameterType],
        ),
        (
            "void d(int *); void f(void) { int x __attribute__((cleanup(d, d))); }",
            &[K::AttributeArguments],
        ),
        (
            "void d(int *); void f(void) { goto in; { int x __attribute__((cleanup(d))) = 1; in:; \
             } }",
            &[K::JumpIntoCleanupScope],
        ),
        (
            "void d(int *); void f(int n) { switch (n) { int x __attribute__((cleanup(d))); case \
             1:; } }",
            &[K::SwitchIntoCleanupScope],
        ),
        (
            "void d(int *); void f(void) { void *t = &&out; { int x __attribute__((cleanup(d))) = \
             1; goto *t; } out:; }",
            &[K::IndirectJumpCleanupScope],
        ),
        (
            "void stdcall_f(void) __attribute__((stdcall));",
            &[K::IgnoredConvention],
        ),
    ];
    for &(source, expected) in cases {
        assert_eq!(kinds(source, Target::LinuxGnu), expected, "{source}");
    }
}

#[test]
fn valid_cleanup_signatures_and_jumps_are_accepted() {
    for source in [
        "void d(void *); void f(void) { volatile int x __attribute__((cleanup(d))); }",
        "void d(int *); void f(void) { const int x __attribute__((cleanup(d))) = 1; }",
        "void d(const int *); void f(void) { int x __attribute__((cleanup(d))); }",
        "int d(int *, ...); void f(void) { int x __attribute__((cleanup(d))); }",
        "void d(void *); void f(int n) { int v[n] __attribute__((cleanup(d))); }",
        "void d(int *); void f(int n) { again:; int x __attribute__((cleanup(d))) = n; if (n--) \
         goto again; }",
        "void d(int *); void f(void) { { int x __attribute__((cleanup(d))); goto out; } out:; }",
        "void d(int *); int f(void) { return ({ int x __attribute__((cleanup(d))) = 3; x; }); }",
        "void d(int *); void f(void) { void *t = &&out; goto *t; out:; { int x \
         __attribute__((cleanup(d))); } }",
    ] {
        assert_eq!(kinds(source, Target::LinuxGnu), [], "{source}");
    }
}

#[test]
fn unimplementable_attributes_and_conventions_are_retained_refusals() {
    let cases: &[(&str, Target, Option<UnsupportedAttribute>)] = &[
        (
            "int r(void); int f(void) __attribute__((ifunc(\"r\")));",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::Ifunc),
        ),
        (
            "__attribute__((naked)) void f(void) {}",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::Naked),
        ),
        (
            "__attribute__((returns_twice)) int f(void);",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::ReturnsTwice),
        ),
        (
            "void __attribute__((ms_abi)) f(void);",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::CallingConvention),
        ),
        (
            "void __attribute__((ms_abi)) f(void);",
            Target::WindowsGnu,
            None,
        ),
        (
            "void __attribute__((sysv_abi)) f(void);",
            Target::WindowsGnu,
            Some(UnsupportedAttribute::CallingConvention),
        ),
        (
            "typedef void (__attribute__((vectorcall)) *fp)(void);",
            Target::WindowsGnu,
            Some(UnsupportedAttribute::CallingConvention),
        ),
        (
            "struct __attribute__((gcc_struct)) S { int a : 3; };",
            Target::WindowsGnu,
            Some(UnsupportedAttribute::RecordLayout),
        ),
        (
            "struct __attribute__((ms_struct)) S { int a : 3; };",
            Target::WindowsGnu,
            None,
        ),
        (
            "__attribute__((constructor)) void f(int x) {}",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::InitFunctionParameters),
        ),
        (
            "int x __attribute__((section(\"a\\\"b\")));",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::SectionName),
        ),
    ];
    for &(source, target, expected) in cases {
        with_configuration(source, gnu(target), |context, s| {
            assert_eq!(
                context.error_count(),
                0,
                "{source}: {:?}",
                context.take_pending_errors()
            );
            assert_eq!(
                s.symbols.refusals.first().map(|refusal| refusal.capability),
                expected,
                "{source} on {target:?}"
            );
        });
    }
}

#[test]
fn system_and_unknown_free_headers_stay_quiet() {
    // Accepted hints and representation attributes produce no diagnostics.
    let source = r#"
__attribute__((noreturn, nothrow, leaf)) void quit(int);
__attribute__((format(printf, 1, 2), nonnull(1))) int say(const char *, ...);
__attribute__((malloc, alloc_size(1), warn_unused_result)) void *grab(unsigned long);
struct __attribute__((packed, aligned(4))) P { char c; int i; };
__attribute__((always_inline, gnu_inline)) extern inline int twice(int x) { return 2 * x; }
[[deprecated, maybe_unused]] static int old;
[[gnu::unused]] static int quiet;
"#;
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        assert_eq!(kinds(source, target), [], "{target:?}");
    }
}
