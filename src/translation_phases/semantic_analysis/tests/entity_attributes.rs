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

fn binding(context: &Context<'_>, s: &SemanticTranslationUnit<'_>, name: &str) -> usize {
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
__attribute__((visibility("hidden"))) static int internal_object;
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
            // Internal linkage has no symbol visibility; Clang is silent.
            assert!(attributes("internal_object").is_none());
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
            assert_eq!(s.symbols.refusals, []);
        });
    }
}

#[test]
fn returns_twice_is_one_retained_fact_for_attributes_and_cataloged_names() {
    let source = r"
typedef long jmp_buf[25];
int _setjmp(jmp_buf);
int __sigsetjmp(jmp_buf, int);
void longjmp(jmp_buf, int);
__attribute__((returns_twice)) int save(void *);
int plain(void *);
static int vfork(void) { return 0; }
";
    with_configuration(source, gnu(Target::LinuxGnu), |context, s| {
        assert_eq!(
            context.error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let returns_twice = |name| {
            s.entity_attributes(binding(context, s, name))
                .is_some_and(|attributes| attributes.returns_twice)
        };
        for name in ["_setjmp", "__sigsetjmp", "save"] {
            assert!(returns_twice(name), "{name}");
        }
        // `longjmp` restores rather than returns twice; an internal
        // function is not the library's.
        for name in ["longjmp", "plain", "vfork"] {
            assert!(!returns_twice(name), "{name}");
        }
    });
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
        // GCC-only, unknown to bundled Clang, which ignores either order.
        (
            "struct __attribute__((scalar_storage_order(\"big-endian\"))) S { int a; };",
            &[K::UnknownAttribute],
        ),
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
        // Clang's jump checker: a computed goto may neither enter nor leave
        // a cleanup scope, whichever address-taken label it reaches.
        (
            "void d(int *); void f(void *t) { goto *t; { int x __attribute__((cleanup(d))) = 1; \
             in:; } void *p = &&in; }",
            &[K::IndirectJumpCleanupScope],
        ),
        (
            "void d(int *); void f(void) { { void *t = &&l; l:; int x __attribute__((cleanup(d))) \
             = 1; goto *t; } }",
            &[K::IndirectJumpCleanupScope],
        ),
        (
            "void d(int *); void f(void) { { int x __attribute__((cleanup(d))) = 1; void *t = \
             &&m; { int y __attribute__((cleanup(d))) = 2; goto *t; } m:; } }",
            &[K::IndirectJumpCleanupScope],
        ),
        (
            "void stdcall_f(void) __attribute__((stdcall));",
            &[K::IgnoredConvention],
        ),
        // Clang gives GCC's attributes a `gnu::` scope and its own a
        // `clang::` scope; the other scope is unknown and has no effect.
        (
            "[[clang::constructor]] void f(void) {}",
            &[K::UnknownAttribute],
        ),
        ("[[clang::weak]] extern int w;", &[K::UnknownAttribute]),
        ("[[gnu::overloadable]] void f(int);", &[K::UnknownAttribute]),
        // Names only some targets know.
        (
            "__attribute__((dllimport)) extern int i;",
            &[K::UnknownAttribute],
        ),
        // Clang's attributes beyond bcc's own rules are known.
        (
            "__attribute__((guarded_var)) int g; __attribute__((noderef)) int *p;",
            &[],
        ),
        // An attribute a redeclaration adds after the definition is
        // ignored; one already written before the definition is not.
        (
            "int f(void) { return 0; } int f(void) __attribute__((weak, cold));",
            &[K::AttributeAfterDefinition, K::AttributeAfterDefinition],
        ),
        (
            "__attribute__((used)) int f(void); int f(void) { return 0; } int f(void) \
             __attribute__((used, noreturn, cdecl));",
            &[],
        ),
        (
            "int o = 1; extern int o __attribute__((section(\"s\")));",
            &[K::AttributeAfterDefinition],
        ),
        // A tentative definition completes at the end of the unit.
        (
            "int t; int t __attribute__((section(\"s\"))); int t = 2;",
            &[],
        ),
        // Argument errors rather than parse errors, as Clang reports them.
        ("__attribute__((constructor())) void f(void) {}", &[]),
        (
            "__attribute__((constructor(1, 2))) void f(void) {}",
            &[K::AttributeArguments],
        ),
        (
            "__attribute__((destructor(200, x))) void f(void) {}",
            &[K::AttributeArguments],
        ),
        // The undeclared identifier is the only error.
        (
            "__attribute__((constructor(missing))) void f(void) {}",
            &[K::UndeclaredIdentifier],
        ),
        (
            "void f(void) __attribute__((section(L\"x\")));",
            &[K::UnevaluatedStringPrefix],
        ),
        (
            "typedef int T __attribute__((section(\"s\")));",
            &[K::AttributeSubject],
        ),
        (
            "void f(void) {} typedef int T __attribute__((alias(\"f\")));",
            &[K::AttributeSubject],
        ),
        (
            "struct S { int m __attribute__((section(\"s\"))); };",
            &[K::AttributeSubject],
        ),
        (
            "void f(void) {} struct S { int m __attribute__((alias(\"f\"))); };",
            &[K::AttributeSubject],
        ),
        (
            "void f(int p __attribute__((section(\"s\"))));",
            &[K::AttributeSubject],
        ),
        // Microsoft `selectany` is for data with external linkage.
        (
            "__attribute__((selectany)) int f(void) { return 1; }",
            &[K::SelectAnySubject],
        ),
        (
            "static __attribute__((selectany)) int s = 1;",
            &[K::SelectAnySubject],
        ),
        (
            "void f(void) { __attribute__((selectany)) int l = 1; (void)l; }",
            &[K::SelectAnySubject],
        ),
        ("typedef int T __attribute__((selectany));", &[]),
        // An `extern` `selectany` declaration is a definition.
        (
            "extern int d __attribute__((selectany)); int d = 5;",
            &[K::DuplicateDefinition],
        ),
        (
            "extern int a[] __attribute__((selectany));",
            &[K::IncompleteTentativeDefinition],
        ),
        (
            "extern int e; extern int e __attribute__((selectany)); int f;",
            &[],
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
        // Every target inside the goto's own cleanup scope, or a cleanup
        // scope elsewhere in the function, as Clang accepts.
        "void d(int *); void f(int i) { { int x __attribute__((cleanup(d))) = 1; void *t[] = \
         {&&a, &&b}; goto *t[i]; a:; b:; } }",
        "void d(int *); void f(int i) { void *t[] = {&&a, &&b}; { int x \
         __attribute__((cleanup(d))) = 1; } goto *t[i]; a:; b:; }",
        // Jumps that stay inside one cleanup scope.
        "void d(int *); void f(int n) { int x __attribute__((cleanup(d))) = n; if (n) goto out; \
         n++; out:; }",
        "void d(int *); void f(int n) { { int x __attribute__((cleanup(d))) = n; back: if (n--) \
         goto back; } }",
        // A goto in the variable's own initializer is in its scope already,
        // as in Clang's jump checker; so is one in an enclosing initializer.
        "void d(int *); void f(int n) { int x __attribute__((cleanup(d))) = ({ if (n) goto in; 1; \
         }); in:; }",
        "void d(int *); void f(int n) { int x __attribute__((cleanup(d))) = ({ int y \
         __attribute__((cleanup(d))) = ({ if (n) goto in; 2; }); y; }); in:; }",
    ] {
        assert_eq!(kinds(source, Target::LinuxGnu), [], "{source}");
    }
    // The own-initializer exception does not cover another scope it enters.
    assert_eq!(
        kinds(
            "void d(int *); void f(int n) { int x __attribute__((cleanup(d))) = ({ if (n) goto \
             in; 1; }); { int y __attribute__((cleanup(d))) = 2; in:; } }",
            Target::LinuxGnu
        ),
        [SemanticErrorKind::JumpIntoCleanupScope]
    );
}

/// Scoped and target-specific spellings take effect exactly where Clang
/// knows them; an attribute written after the definition does not.
#[test]
fn entity_attributes_follow_clang_spellings_and_definition_order() {
    let source = r#"
[[gnu::weak]] extern int scoped;
__attribute__((selectany)) const int chosen = 7;
[[clang::weak]] extern int foreign;
int late(void) { return 0; }
int late(void) __attribute__((weak, section(".late")));
void prefixed(void) __attribute__((section(L"wide")));
"#;
    with_configuration(source, gnu(Target::LinuxGnu), |context, s| {
        let attributes = |name| s.entity_attributes(binding(context, s, name)).copied();
        assert!(attributes("chosen").unwrap().select_any);
        assert!(attributes("scoped").unwrap().weak);
        assert!(attributes("foreign").is_none());
        assert!(attributes("late").is_none());
        assert_eq!(attributes("prefixed").unwrap().section, Some("wide"));
    });
    for (target, known) in [(Target::LinuxGnu, false), (Target::WindowsGnu, true)] {
        assert_eq!(
            kinds("__attribute__((dllexport)) int exported;", target).is_empty(),
            known,
            "{target:?}"
        );
    }
    // Clang knows `ifunc` only for glibc's Linux target.
    assert_eq!(
        kinds(
            "int r(void); int f(void) __attribute__((ifunc(\"r\")));",
            Target::LinuxMusl
        ),
        [SemanticErrorKind::UnknownAttribute]
    );
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
        // An entity fact that lowering projects onto the IR function.
        (
            "__attribute__((returns_twice)) int f(void);",
            Target::LinuxGnu,
            None,
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
        // A thread-local alias of a thread-local object is Clang's
        // `thread_local alias`; mixing storage durations names the wrong
        // storage, which Clang emits silently.
        (
            "__thread int x = 1; extern __thread int y __attribute__((alias(\"x\")));",
            Target::LinuxGnu,
            None,
        ),
        (
            "__thread int x = 1; extern int y __attribute__((alias(\"x\")));",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::AliasThreadLocal),
        ),
        (
            "int x = 1; extern _Thread_local int y __attribute__((alias(\"x\")));",
            Target::WindowsGnu,
            Some(UnsupportedAttribute::AliasThreadLocal),
        ),
        // Clang attributes whose ABI, addressing or linkage bcc does not
        // model are refused, not accepted as hints.
        (
            "int use(void *p __attribute__((swift_async_context))) { return p != 0; }",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::SwiftParameter),
        ),
        (
            "__attribute__((internal_linkage)) int value = 7;",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::InternalLinkage),
        ),
        (
            "int __attribute__((address_space(256))) *gs;",
            Target::LinuxGnu,
            Some(UnsupportedAttribute::AddressSpace),
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
    let source = r"
__attribute__((noreturn, nothrow, leaf)) void quit(int);
__attribute__((format(printf, 1, 2), nonnull(1))) int say(const char *, ...);
__attribute__((malloc, alloc_size(1), warn_unused_result)) void *grab(unsigned long);
struct __attribute__((packed, aligned(4))) P { char c; int i; };
__attribute__((always_inline, gnu_inline)) extern inline int twice(int x) { return 2 * x; }
[[deprecated, maybe_unused]] static int old;
[[gnu::unused]] static int quiet;
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        assert_eq!(kinds(source, target), [], "{target:?}");
    }
}
