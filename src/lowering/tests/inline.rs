//! External emission follows C99 §6.7.4p6-7 and GCC's GNU89 inline rules.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

fn has_body(source: &str, config: CompilerConfiguration) -> bool {
    let text = lower_with(source, config).unwrap_or_else(|error| panic!("{source}: {error:?}"));
    let arena = Bump::new();
    let module = parse_module(&arena, &text).unwrap();
    let Some(crate::ir::Symbol::Function(id)) = module.symbol("f") else {
        return false;
    };
    module.function(id).body.is_some()
}

#[test]
fn inline_emission_uses_the_effective_mode_and_all_c99_declarations() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for gnu89 in [false, true] {
            let config = CompilerConfiguration::default()
                .with_target(target)
                .with_gnu89_inline(gnu89);
            for (source, defined) in [
                ("inline int f(void) { return 7; }", gnu89),
                ("extern inline int f(void) { return 7; }", !gnu89),
                ("static inline int f(void) { return 7; }", false),
                ("int f(void); inline int f(void) { return 7; }", true),
                ("inline int f(void) { return 7; } int f(void);", true),
                (
                    "inline int f(void) { return 7; } extern inline int f(void);",
                    true,
                ),
                (
                    "extern int f(void); extern inline int f(void) { return 7; }",
                    !gnu89,
                ),
                (
                    "extern inline int f(void) { return 7; } int f(void);",
                    !gnu89,
                ),
                (
                    "inline int f(void) { return 7; } int g(void) { int f(void); return f(); }",
                    gnu89,
                ),
            ] {
                assert_eq!(has_body(source, config), defined, "gnu89={gnu89}: {source}");
            }
        }
    }
}

/// The Microsoft ABI may discard an unused C99-mode inline body while
/// retaining its callable definition and dependencies when reached.
#[test]
fn msvc_c99_inline_bodies_follow_reachability_and_retention_attributes() {
    let config = CompilerConfiguration::default().with_target(Target::WindowsMsvc);
    for (source, defined) in [
        ("inline int f(void) { return 7; }", false),
        (
            "inline int f(void) { return 7; } int main(void) { return f(); }",
            true,
        ),
        ("inline int f(void) { return 7; } int (*p)(void) = f;", true),
        (
            "__attribute__((used)) inline int f(void) { return 7; }",
            true,
        ),
        ("__attribute__((constructor)) inline void f(void) {}", true),
        ("__attribute__((destructor)) inline void f(void) {}", true),
        (
            "extern inline int f(void) { return 7; } int other(void) \
             __attribute__((alias(\"f\")));",
            true,
        ),
        ("int f(void); inline int f(void) { return 7; }", false),
        ("inline int f(void) { return 7; } int f(void);", true),
        (
            "extern inline int f(void); inline int f(void) { return 7; }",
            true,
        ),
        (
            "inline int f(void) { return 7; } extern inline int f(void);",
            true,
        ),
        (
            "inline int f(void) { static int x = 7; return x; } int main(void) { return f(); }",
            true,
        ),
    ] {
        assert_eq!(has_body(source, config), defined, "{source}");
    }
    for called in [false, true] {
        let source = format!(
            "static int helper(void) {{ return 7; }} inline int f(void) {{ return helper(); }} \
             static int dead(void) {{ return f(); }} int main(void) {{ return {}; }}",
            if called { "f()" } else { "0" }
        );
        let text = lower_with(&source, config).unwrap();
        let arena = Bump::new();
        let module = parse_module(&arena, &text).unwrap();
        for name in ["f", "helper"] {
            assert_eq!(
                module.symbol(name).is_some(),
                called,
                "called={called} {name}: {text}"
            );
        }
        assert!(module.symbol("dead").is_none(), "{text}");
    }
}

/// Microsoft inline retention applies in either language mode; explicit
/// `gnu_inline` keeps GNU emission, including mode overrides before C99.
#[test]
fn msvc_discardable_inline_keeps_gnu_modes_and_attributes() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for standard in [CStandard::C89, CStandard::C99, CStandard::C17] {
            for gnu89 in [false, true] {
                let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_target(target)
                    .with_gnu_extensions(true)
                    .with_gnu89_inline(gnu89);
                for (source, defined) in [
                    (
                        "inline int f(void) { return 7; }",
                        target != Target::WindowsMsvc && gnu89,
                    ),
                    (
                        "extern inline int f(void) { return 7; }",
                        target == Target::WindowsMsvc || !gnu89,
                    ),
                    (
                        "inline __attribute__((gnu_inline)) int f(void) { return 7; }",
                        true,
                    ),
                    (
                        "extern inline __attribute__((gnu_inline)) int f(void) { return 7; }",
                        false,
                    ),
                    (
                        "inline __attribute__((gnu_inline)) int f(void); inline int f(void) { \
                         return 7; }",
                        true,
                    ),
                    (
                        "extern inline __attribute__((gnu_inline)) int f(void); extern inline int \
                         f(void) { return 7; }",
                        false,
                    ),
                ] {
                    assert_eq!(
                        has_body(source, config),
                        defined,
                        "{target:?} {standard:?} gnu89={gnu89}: {source}"
                    );
                }
            }
        }
    }
}

/// Clang emits Microsoft inline bodies in source order. Ordinary C99
/// redeclarations can materialize a deferred body only on the first
/// transition from inline-only file-scope declarations. The first explicit
/// extern also forces it in Microsoft compatibility mode. Block declarations
/// update inherited inline/extern facts without materializing a prior body.
/// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19;
/// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
#[test]
fn msvc_inline_retention_follows_declaration_order_and_scope() {
    for standard in [CStandard::C89, CStandard::C99, CStandard::C17] {
        for compatibility in [false, true] {
            let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                .with_target(Target::WindowsMsvc)
                .with_gnu_extensions(true)
                .with_msvc_extensions(compatibility);
            let gnu89 = config.gnu89_inline();
            for (source, defined) in [
                ("inline int f(void) { return 7; }", false),
                ("int f(void); inline int f(void) { return 7; }", false),
                ("inline int f(void) { return 7; } int f(void);", !gnu89),
                (
                    "int f(void); inline int f(void) { return 7; } int f(void);",
                    false,
                ),
                ("extern int f(void); inline int f(void) { return 7; }", true),
                (
                    "inline int f(void) { return 7; } extern int f(void);",
                    compatibility || !gnu89,
                ),
                ("extern inline int f(void) { return 7; }", true),
                ("inline int f(void); int f(void) { return 7; }", false),
                ("int f(void) { return 7; } inline int f(void);", true),
                (
                    "inline int f(void); int f(void); int f(void) { return 7; }",
                    false,
                ),
                (
                    "inline int f(void); int f(void) { return 7; } int f(void);",
                    false,
                ),
                (
                    "inline int f(void); int f(void) { return 7; } extern int f(void);",
                    compatibility,
                ),
                (
                    "static void dead(void) { int f(void); } inline int f(void) { return 7; }",
                    false,
                ),
                (
                    "static void dead(void) { extern int f(void); } inline int f(void) { return \
                     7; }",
                    true,
                ),
                (
                    "static void dead(void) { inline int f(void); } int f(void) { return 7; }",
                    false,
                ),
                (
                    "inline int f(void) { return 7; } static void dead(void) { int f(void); }",
                    false,
                ),
                (
                    "inline int f(void) { return 7; } static void dead(void) { extern int \
                     f(void); }",
                    false,
                ),
                (
                    "inline int f(void) { return 7; } static void dead(void) { extern int \
                     f(void); } extern int f(void);",
                    !gnu89,
                ),
                (
                    "int f(void); inline int f(void) { return 7; } static void dead(void) { \
                     extern int f(void); } extern int f(void);",
                    false,
                ),
                (
                    "inline int f(void) { return 7; } static void dead(void) { extern int \
                     f(void); } int (*p)(void) = f;",
                    true,
                ),
            ] {
                assert_eq!(
                    has_body(source, config),
                    defined,
                    "{standard:?} compatibility={compatibility}: {source}"
                );
            }
        }
    }
}

/// An applicable Microsoft export annotation retains the inline body and
/// its dependencies. An annotation first added after the body is ignored;
/// the other accepted attributes below do not independently require emission.
/// This tests retention, not DLL export directives or COMDAT linkage.
/// Microsoft ABI extension under C99: §4p6, p. 7; PDF p. 19.
#[test]
fn msvc_inline_export_retention_requires_an_applicable_annotation() {
    for standard in [CStandard::C89, CStandard::C99, CStandard::C17] {
        let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
            .with_target(Target::WindowsMsvc)
            .with_gnu_extensions(true)
            .with_msvc_extensions(true);
        for (body, retained) in [
            ("inline int f(void) { return helper(); }", false),
            (
                "__declspec(dllexport) inline int f(void) { return helper(); }",
                true,
            ),
            (
                "__attribute__((dllexport)) inline int f(void) { return helper(); }",
                true,
            ),
            (
                "__declspec(dllexport) inline int f(void); inline int f(void) { return helper(); }",
                true,
            ),
            (
                "inline int f(void) { return helper(); } __declspec(dllexport) inline int f(void);",
                false,
            ),
            (
                "__attribute__((dllexport)) inline int f(void); inline int f(void) { return \
                 helper(); }",
                true,
            ),
            (
                "inline int f(void) { return helper(); } __attribute__((dllexport)) inline int \
                 f(void);",
                false,
            ),
            (
                "__attribute__((weak)) inline int f(void) { return helper(); }",
                false,
            ),
            (
                "__attribute__((retain)) inline int f(void) { return helper(); }",
                false,
            ),
            (
                "__attribute__((externally_visible)) inline int f(void) { return helper(); }",
                false,
            ),
        ] {
            let source = format!(
                "static int helper(void) {{ return 7; }} {body} int main(void) {{ return 0; }}"
            );
            let text = lower_with(&source, config).unwrap();
            let arena = Bump::new();
            let module = parse_module(&arena, &text).unwrap();
            for name in ["f", "helper"] {
                let has_body = match module.symbol(name) {
                    | Some(crate::ir::Symbol::Function(id)) => module.function(id).body.is_some(),
                    | _ => false,
                };
                assert_eq!(has_body, retained, "{standard:?} {name}: {source}\n{text}");
            }
        }
    }
}

#[test]
fn gnu_inline_attribute_and_replacement_bodies_reach_lowering() {
    for source in [
        "extern inline __attribute__((gnu_inline)) int f(void) { return 1; } int f(void) { return \
         7; }",
        "inline __attribute__((gnu_inline)) int f(void); extern inline int f(void) { return 1; } \
         int f(void) { return 7; }",
        "extern inline __attribute__((gnu_inline)) int f(void) { return 1; } extern inline \
         __attribute__((gnu_inline)) int f(void) { return 2; } int f(void) { return 7; }",
    ] {
        let text = lowered(source);
        assert_eq!(text.matches("function @f(").count(), 1, "{text}");
        assert!(text.contains('7'), "{text}");
    }
    for (source, defined) in [
        (
            "inline __attribute__((gnu_inline)) int f(void) { return 7; }",
            true,
        ),
        (
            "extern inline __attribute__((__gnu_inline__)) int f(void) { return 7; }",
            false,
        ),
        (
            "extern inline __attribute__((gnu_inline)) int f(void) { return 7; } int f(void);",
            false,
        ),
        (
            "static inline __attribute__((gnu_inline)) int f(void) { return 7; }",
            false,
        ),
    ] {
        assert_eq!(
            has_body(source, CompilerConfiguration::default()),
            defined,
            "{source}"
        );
    }
}

/// C23 `[[gnu::gnu_inline]]` is the GNU attribute: an `extern inline` body
/// supplies no external definition (as bundled Clang emits only a
/// declaration of `f`), while `[[clang::gnu_inline]]` is unknown.
#[test]
fn scoped_gnu_inline_selects_gnu_omission() {
    let config = CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow);
    for (source, defined) in [
        (
            "[[gnu::gnu_inline]] extern inline int f(void) { return 7; } int (*p)(void) = f;",
            false,
        ),
        (
            "[[__gnu__::__gnu_inline__]] extern inline int f(void) { return 7; }",
            false,
        ),
        (
            "extern inline int f(void) { return 7; } int (*p)(void) = f;",
            true,
        ),
        (
            "[[clang::gnu_inline]] extern inline int f(void) { return 7; }",
            true,
        ),
    ] {
        assert_eq!(has_body(source, config), defined, "{source}");
    }
}

#[test]
fn pre_c99_defaults_to_gnu89_including_the_c95_amendment() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C17,
    ] {
        let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow);
        assert_eq!(config.gnu89_inline(), standard < CStandard::C99);
        assert_eq!(
            has_body("__inline__ int f(void) { return 7; }", config),
            standard < CStandard::C99
        );
    }
}

/// A private `always_inline` body is emitted, so a capability refused in
/// it, here an x86 segment address space, is refused rather than lowered
/// as an ordinary load; a body with inline assembly is not emitted, so its
/// refusals do not apply.
#[test]
fn private_inline_bodies_keep_their_refusals() {
    let errors = lower(
        "extern inline __attribute__((gnu_inline, always_inline)) int readgs(void) { return *(int \
         __attribute__((address_space(256))) *)0; } int main(void) { return readgs(); }",
    )
    .expect_err("the address space is refused");
    assert!(
        errors.iter().any(|error| error.contains("address_space")),
        "{errors:?}"
    );
    let assembly = lower(
        "extern inline __attribute__((gnu_inline, always_inline)) int readgs(void) { \
         __asm__(\"\"); return *(int __attribute__((address_space(256))) *)0; } int main(void) { \
         return readgs(); }",
    );
    assert!(
        assembly
            .as_ref()
            .is_ok_and(|text| text.contains("call @readgs(")),
        "{assembly:?}"
    );
}

/// A private `always_inline` body stands in for its own entity only: the
/// glibc `__REDIRECT` alias that labels another declaration with the same
/// symbol calls the external `read`, so the private body does not call
/// itself.
/// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125. GNU extension: GCC manual,
/// "Asm Labels".
#[test]
fn private_inline_bodies_do_not_capture_redirected_symbols() {
    let text = lower(
        "extern int __read_alias(int, void *, unsigned long) __asm__(\"read\"); extern __inline \
         __attribute__((always_inline, gnu_inline)) int read(int fd, void *b, unsigned long n) { \
         return __read_alias(fd, b, n); } int main(void) { char c; return read(0, &c, 1); }",
    )
    .unwrap_or_else(|error| panic!("{error:?}"));
    let private = text
        .split("function @read.always_inline(")
        .nth(1)
        .unwrap_or_else(|| panic!("the private body is defined\n{text}"));
    let private = private.split("\nfunction ").next().unwrap_or(private);
    assert!(private.contains("call @read("), "{text}");
    assert!(!private.contains("call @read.always_inline("), "{text}");
    let main = text
        .split("function @main(")
        .nth(1)
        .expect("main is defined");
    let main = main.split("\nfunction ").next().unwrap_or(main);
    assert!(main.contains("call @read.always_inline("), "{text}");
    // A wrapper of its own symbol is no `available_externally` body of it
    // (#89): as that body, its call to `read` would call itself. Clang emits
    // no body for it (its `isTriviallyRecursive`).
    let header = private.lines().next().unwrap_or_default();
    assert!(!header.contains("inline_copy_of"), "{text}");
    let arena = Bump::new();
    let module = parse_module(&arena, &text).unwrap();
    let Some(crate::ir::Symbol::Function(copy)) = module.symbol("read.always_inline") else {
        panic!("{text}");
    };
    assert_eq!(module.function(copy).inline_copy_of, None, "{text}");
}

/// GNU extern-inline bodies supply direct-call implementations while values
/// keep the external symbol, including Clang's size-annotated fortify overload.
/// C99: §6.3.2.1p4, p. 46; PDF p. 58; §6.5.3.2p1, p. 78; PDF p. 90.
#[test]
fn fortify_wrapper_values_keep_external_symbols() {
    for clang_fortify in [false, true] {
        let (attributes, size1, size0) = if clang_fortify {
            (
                ", overloadable",
                "__attribute__((pass_object_size(1)))",
                "__attribute__((pass_object_size(0)))",
            )
        } else {
            ("", "", "")
        };
        let source = format!(
            r#"
extern char *getcwd(char *, unsigned long);
extern char *__getcwd_alias(char *, unsigned long) __asm__("getcwd");
extern char *__getcwd_chk(char *, unsigned long, unsigned long);
extern long read(int, void *, unsigned long);
extern long __read_alias(int, void *, unsigned long) __asm__("read");
extern long __read_chk(int, void *, unsigned long, unsigned long);
extern inline __attribute__((always_inline, gnu_inline{attributes}))
char *getcwd(char *const buf {size1}, unsigned long size) {{
    unsigned long bos = __builtin_object_size(buf, 1);
    return bos == (unsigned long)-1 ? __getcwd_alias(buf, size) : __getcwd_chk(buf, size, bos);
}}
extern inline __attribute__((always_inline, gnu_inline{attributes}))
long read(int fd, void *const buf {size0}, unsigned long size) {{
    unsigned long bos = __builtin_object_size(buf, 0);
    return bos == (unsigned long)-1 ? __read_alias(fd, buf, size) : __read_chk(fd, buf, size, bos);
}}
char *(*cwd_value)(char *, unsigned long) = getcwd;
char *(*cwd_address)(char *, unsigned long) = &getcwd;
long (*read_value)(int, void *, unsigned long) = read;
long (*read_address)(int, void *, unsigned long) = &read;
int values(void) {{
    static char *(*cwd_static)(char *, unsigned long) = getcwd;
    static long (*read_static)(int, void *, unsigned long) = &read;
    char *(*cwd_local)(char *, unsigned long) = getcwd;
    long (*read_local)(int, void *, unsigned long) = &read;
    return cwd_local == cwd_static && read_local == read_static;
}}
int calls(char *buf, unsigned long size) {{
    return getcwd(buf, size) != 0 && read(0, buf, size) >= 0;
}}
"#
        );
        let text = lowered(&source);
        for (global, symbol) in [
            ("cwd_value", "getcwd"),
            ("cwd_address", "getcwd"),
            ("read_value", "read"),
            ("read_address", "read"),
        ] {
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!("global @{global} ")))
                .unwrap();
            assert!(line.ends_with(&format!("relocs [0: @{symbol}]")), "{text}");
        }
        let relocations: Vec<_> = text
            .lines()
            .filter(|line| line.contains("relocs ["))
            .collect();
        assert_eq!(relocations.len(), 6, "{text}");
        assert!(
            relocations
                .iter()
                .all(|line| line.ends_with("relocs [0: @getcwd]")
                    || line.ends_with("relocs [0: @read]")),
            "{text}"
        );
        let values = text
            .split("function @values(")
            .nth(1)
            .unwrap()
            .split("\nfunction ")
            .next()
            .unwrap();
        assert!(values.contains("func_addr @getcwd\n"), "{text}");
        assert!(values.contains("func_addr @read\n"), "{text}");
        assert!(!values.contains("always_inline"), "{text}");
        let calls = text
            .split("function @calls(")
            .nth(1)
            .unwrap()
            .split("\nfunction ")
            .next()
            .unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("call @") && line.contains(".always_inline("))
                .count(),
            2,
            "{text}"
        );
        assert!(text.contains("call @getcwd("), "{text}");
        assert!(text.contains("call @read("), "{text}");
    }
}

/// Multiple addressable overloads need the pointer target to select the symbol,
/// including static relocations when another overload has a private inline
/// body.
#[test]
fn overload_addresses_follow_the_target_type() {
    let source = include_str!("../../../tests/fixtures/overloadable/address.c");
    for target in [Target::LinuxGnu, Target::WindowsMsvc] {
        let (int_symbol, double_symbol) = if target == Target::WindowsMsvc {
            ("\"?o@@$$J0YAHH@Z\"", "\"?o@@$$J0YAHN@Z\"")
        } else {
            ("_Z1oi", "_Z1od")
        };
        let text = lower_for(source, target).unwrap();
        for (global, symbol) in [
            ("global_int", int_symbol),
            ("global_double", double_symbol),
            ("global_decay_double", double_symbol),
        ] {
            let line = text
                .lines()
                .find(|line| line.starts_with(&format!("global @{global} ")))
                .unwrap();
            assert!(line.ends_with(&format!("relocs [0: @{symbol}]")), "{text}");
        }
        for (suffix, symbol) in [("int", int_symbol), ("double", double_symbol)] {
            let static_global = text
                .lines()
                .find(|line| {
                    line.starts_with("global @") && line.contains(&format!("static_{suffix}"))
                })
                .unwrap();
            assert!(
                static_global.ends_with(&format!("relocs [0: @{symbol}]")),
                "{text}"
            );
            for form in ["automatic", "assignment", "cast", "argument"] {
                let body = text
                    .split(&format!("function @{form}_{suffix}("))
                    .nth(1)
                    .unwrap()
                    .split("\nfunction ")
                    .next()
                    .unwrap();
                let addresses: Vec<_> = body
                    .lines()
                    .filter(|line| line.contains("func_addr @"))
                    .collect();
                assert_eq!(addresses.len(), 1, "{text}");
                assert!(
                    addresses[0].ends_with(&format!("func_addr @{symbol}")),
                    "{text}"
                );
            }
        }
    }
}

/// SQLite stores fortify wrappers in a generic syscall table using an explicit
/// function-pointer cast. Resolve the sole addressable overload before casting.
#[test]
fn fortify_wrapper_cast_initializers_keep_external_symbols() {
    let text = lowered(
        r"
extern char *getcwd(char *, unsigned long);
extern inline __attribute__((always_inline, gnu_inline, overloadable))
char *getcwd(char *const buf __attribute__((pass_object_size(1))), unsigned long size) { return buf; }
void (*syscalls[])(void) = { (void (*)(void))getcwd, (void (*)(void))&getcwd };
void (*value(void))(void) { return (void (*)(void))getcwd; }
",
    );
    assert!(text.contains("relocs [0: @getcwd, 8: @getcwd]"), "{text}");
    assert!(text.contains("func_addr @getcwd\n"), "{text}");
    assert!(!text.contains("function @_Z"), "{text}");
}

#[test]
fn gnu_extern_inline_void_values_keep_external_symbol() {
    let text = lowered(
        r"
extern inline __attribute__((always_inline, gnu_inline)) void f(void) {}
void (*p)() = f;
void (*q)() = &f;
void (*value(void))() { return f; }
void call(void) { f(); }
",
    );
    assert_eq!(text.matches("relocs [0: @f]").count(), 2, "{text}");
    assert!(text.contains("func_addr @f\n"), "{text}");
    assert!(text.contains("call @f.always_inline("), "{text}");
}

/// A plain inline-only body is lowered as a copy that names its external
/// function and that nothing calls: calls keep naming the external
/// function, and the LLVM back end makes the copy its
/// `available_externally` body, as Clang emits it when optimizing (#109).
/// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
#[test]
fn plain_inline_only_bodies_are_unreferenced_copies() {
    let text = lowered(
        "inline int f(int x) { return x ? x * f(x - 1) : 1; } int u(int x) { return f(x); }",
    );
    let copy = text
        .split("function @f.inline(")
        .nth(1)
        .unwrap_or_else(|| panic!("the copy is defined\n{text}"));
    assert!(
        copy.lines()
            .next()
            .is_some_and(|header| header.ends_with("internal inline_copy_of @f {")),
        "{text}"
    );
    // Its own recursive call names the external function too.
    assert!(!text.contains("call @f.inline("), "{text}");
    assert!(text.contains("call @f("), "{text}");
    assert!(
        text.contains("function @f(i32 noundef) -> i32 external\n"),
        "{text}"
    );
}

/// An `always_inline` wrapper that calls its own symbol through an
/// assembler label, like glibc's fortify wrappers before 2.40, keeps a
/// private copy that is not its external function's body: as that body,
/// LLVM would inline its call to the symbol into itself.
#[test]
fn always_inline_wrappers_of_their_own_symbol_stay_private() {
    let text = lowered(
        "extern int real(int) __asm__(\"f\"); extern inline __attribute__((always_inline, \
         gnu_inline)) int f(int x) { return real(x) + 1; } int u(int x) { return f(x); }",
    );
    let copy = text
        .split("function @f.always_inline(")
        .nth(1)
        .unwrap_or_else(|| panic!("the copy is defined\n{text}"));
    assert!(
        copy.lines()
            .next()
            .is_some_and(|header| !header.contains("inline_copy_of")),
        "{text}"
    );
    assert!(text.contains("call @f.always_inline("), "{text}");
}

/// No copy where Clang emits no `available_externally` body: an unused,
/// `noinline` or assembly body, or one that calls its own symbol through
/// another entity (an assembler label or a builtin's library name).
#[test]
fn plain_inline_copies_follow_clang_emission() {
    for source in [
        "inline int f(int x) { return x + 1; } int u(int x) { return x; }",
        "inline __attribute__((noinline)) int f(int x) { return x + 1; } int u(int x) { return \
         f(x); }",
        "inline int f(int x) { __asm__(\"\"); return x + 1; } int u(int x) { return f(x); }",
        "extern int real(int) __asm__(\"f\"); inline int f(int x) { return real(x); } int u(int \
         x) { return f(x); }",
        "inline int abs(int x) { return __builtin_abs(x); } int u(int x) { return abs(x); }",
    ] {
        let text = lowered(source);
        assert!(!text.contains(".inline("), "{source}\n{text}");
    }
}

/// Plain inline-only bodies were not lowered before #109: a construct that
/// lowering refuses in one keeps calling the external definition instead
/// of refusing the unit.
#[test]
fn a_refused_plain_inline_body_keeps_the_external_call() {
    let text = lowered(
        "extern inline __attribute__((gnu_inline)) int readgs(void) { return *(int \
         __attribute__((address_space(256))) *)0; } inline int f(int x) { return x + 1; } int \
         main(void) { return readgs() + f(1); }",
    );
    assert!(!text.contains("readgs.inline"), "{text}");
    assert!(text.contains("call @readgs("), "{text}");
    // Only the refused body is dropped (#109 review).
    assert!(text.contains("function @f.inline("), "{text}");
}

/// The `always_inline` bodies are declared in the order Clang creates
/// their LLVM functions, from the first reference, not in definition
/// order: LLVM's always-inliner visits them in module order, and the
/// order decides which recursive calls stay calls (#109 review).
#[test]
fn always_inline_bodies_follow_clang_creation_order() {
    let text = lowered(
        "int printf(const char *, ...); inline __attribute__((always_inline)) int f(int); inline \
         __attribute__((always_inline)) int g(int); inline __attribute__((always_inline)) int \
         h(int); inline __attribute__((always_inline)) int f(int x) { return x ? 2 * g(x - 1) : \
         1; } inline __attribute__((always_inline)) int h(int x) { return x ? 4 * f(x - 1) : 1; } \
         inline __attribute__((always_inline)) int g(int x) { return x ? 3 * h(x - 1) : 1; } int \
         main(void) { printf(\"%d\\n\", f(5)); return 0; }",
    );
    let position = |name: &str| {
        text.find(&format!("function @{name}.always_inline("))
            .unwrap_or_else(|| panic!("{name}\n{text}"))
    };
    assert!(
        position("f") < position("g") && position("g") < position("h"),
        "{text}"
    );
    // Code generation skips dead code, so a call there creates nothing:
    // the unreachable `h` call, the code after `return`, and the
    // constant-false operand leave Clang's order f, g, h (#109 review).
    for dead in [
        "if (0) h(0);",
        "(void)(0 && h(0));",
        "(void)(1 || h(0));",
        "(void)(0 ? h(0) : 0);",
    ] {
        let text = lowered(&format!(
            "int printf(const char *, ...); inline __attribute__((always_inline)) int f(int); \
             inline __attribute__((always_inline)) int g(int); inline \
             __attribute__((always_inline)) int h(int); inline __attribute__((always_inline)) int \
             f(int x) {{ return x ? 2 * g(x - 1) : 1; }} inline __attribute__((always_inline)) \
             int g(int x) {{ return x ? 3 * h(x - 1) : 1; }} inline \
             __attribute__((always_inline)) int h(int x) {{ return x ? 4 * f(x - 1) : 1; }} int \
             main(void) {{ {dead} printf(\"%d\\n\", f(6)); return 0; h(1); }}"
        ));
        let position = |name: &str| {
            text.find(&format!("function @{name}.always_inline("))
                .unwrap_or_else(|| panic!("{name}\n{text}"))
        };
        assert!(
            position("f") < position("g") && position("g") < position("h"),
            "{dead}\n{text}"
        );
    }
    // In an ordinary function before `main`, the forms whose skipped
    // references a review found counted: a generic selection's controlling
    // operand and unselected association, a comma condition that folds, and
    // the code after `__builtin_unreachable()` (#109 review, round 3).
    for dead in [
        "(void)_Generic(h(0), int: 0, default: 1);",
        "(void)_Generic(0, int: 0, default: h(0));",
        "if ((0, 0)) h(0);",
        "__builtin_unreachable(); h(0);",
    ] {
        let text = lowered(&format!(
            "int printf(const char *, ...); inline __attribute__((always_inline)) int f(int); \
             inline __attribute__((always_inline)) int g(int); inline \
             __attribute__((always_inline)) int h(int); inline __attribute__((always_inline)) int \
             f(int x) {{ return x ? 2 * g(x - 1) : 1; }} inline __attribute__((always_inline)) \
             int g(int x) {{ return x ? 3 * h(x - 1) : 1; }} inline \
             __attribute__((always_inline)) int h(int x) {{ return x ? 4 * f(x - 1) : 1; }} int \
             probe(void) {{ {dead} return 0; }} int main(void) {{ printf(\"%d\\n\", f(6)); return \
             0; }}"
        ));
        let position = |name: &str| {
            text.find(&format!("function @{name}.always_inline("))
                .unwrap_or_else(|| panic!("{name}\n{text}"))
        };
        assert!(
            position("f") < position("g") && position("g") < position("h"),
            "{dead}\n{text}"
        );
    }
    // The selected association and a live comma condition still count.
    for live in [
        "(void)_Generic(0, int: h(0), default: 0);",
        "if ((0, 1)) h(0);",
    ] {
        let text = lowered(&format!(
            "inline __attribute__((always_inline)) int f(int); inline \
             __attribute__((always_inline)) int g(int); inline __attribute__((always_inline)) int \
             h(int); inline __attribute__((always_inline)) int f(int x) {{ return x ? 2 * g(x - \
             1) : 1; }} inline __attribute__((always_inline)) int g(int x) {{ return x ? 3 * h(x \
             - 1) : 1; }} inline __attribute__((always_inline)) int h(int x) {{ return x ? 4 * \
             f(x - 1) : 1; }} int probe(void) {{ {live} return 0; }} int main(void) {{ return \
             f(6); }}"
        ));
        let position = |name: &str| {
            text.find(&format!("function @{name}.always_inline("))
                .unwrap_or_else(|| panic!("{name}\n{text}"))
        };
        assert!(position("h") < position("f"), "{live}\n{text}");
    }
}

/// A switch whose condition folds emits only the statements Clang's
/// `CollectStatementsForCase` selects, and an expression statement ending
/// in a call that does not return skips what follows. References there
/// create no function, so Clang's order stays f, g, h; references Clang
/// does emit put h first (#113). Each expectation matches Clang 23's
/// `-O0 -Xclang -disable-llvm-passes` module.
#[test]
fn always_inline_order_skips_unselected_switch_arms_and_code_after_noreturn_calls() {
    let h_first = |probe: &str, config: CompilerConfiguration| {
        let source = format!(
            "int c; int other(void); __attribute__((noreturn)) void die(void); _Noreturn void \
             stop(void); void exit(int); void _exit(int); void (*fp)(void) \
             __attribute__((noreturn)); typedef void nr(void) __attribute__((noreturn)); nr *fpt; \
             struct s {{ void (*m)(void) __attribute__((noreturn)); }} holder; inline \
             __attribute__((always_inline)) int f(int); inline __attribute__((always_inline)) int \
             g(int); inline __attribute__((always_inline)) int h(int); inline \
             __attribute__((always_inline)) int f(int x) {{ return x ? 2 * g(x - 1) : 1; }} \
             inline __attribute__((always_inline)) int g(int x) {{ return x ? 3 * h(x - 1) : 1; \
             }} inline __attribute__((always_inline)) int h(int x) {{ return x ? 4 * f(x - 1) : \
             1; }} int probe(void) {{ {probe} return 0; }} int main(void) {{ return f(6); }}"
        );
        let text = lower_with(&source, config).unwrap_or_else(|error| panic!("{error:?}\n{probe}"));
        let position = |name: &str| {
            text.find(&format!("function @{name}.always_inline("))
                .unwrap_or_else(|| panic!("{name}: {probe}\n{text}"))
        };
        assert!(position("f") < position("g"), "{probe}\n{text}");
        position("h") < position("f")
    };
    let c99 = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow);
    let gnu = c99.with_gnu_extensions(true);
    for (probe, config, emitted) in [
        // The unselected arms, the default fallback and an elided body.
        (
            "switch (0) { case 1: h(0); break; default: break; }",
            c99,
            false,
        ),
        ("switch (5) { case 1: h(0); }", c99, false),
        ("switch (2) { case 1: break; case 2: h(0); }", c99, true),
        ("switch (5) { case 1: break; default: h(0); }", c99, true),
        (
            "switch ((0, 1)) { case 0: h(0); break; case 1: break; }",
            c99,
            false,
        ),
        (
            "switch ((other(), 1)) { case 0: h(0); break; case 1: break; }",
            c99,
            true,
        ),
        // Selected statements fall through later labels, which resume
        // nothing; the code after the switch follows their end.
        ("switch (1) { case 1: return 0; case 2: h(0); }", c99, false),
        (
            "switch (5) { default: return 0; case 1: h(0); }",
            c99,
            false,
        ),
        ("switch (1) { case 1: return 0; } h(0);", c99, false),
        ("switch (1) { case 1: break; } h(0);", c99, true),
        // Case labels inside skipped statements do not keep them; a case
        // label Clang cannot reach through blocks, a skipped declaration
        // or a label the switch would skip leave it unfolded.
        (
            "switch (1) { case 0: if (h(0)) { case 3: break; } break; case 1: break; }",
            c99,
            false,
        ),
        (
            "switch (1) { case 0: h(0); if (c) { case 1: ; } }",
            c99,
            true,
        ),
        (
            "switch (1) { int y; case 0: h(0); break; case 1: y = 0; (void)y; }",
            c99,
            true,
        ),
        (
            "goto l; switch (0) { case 1: l: h(0); break; default: break; }",
            c99,
            true,
        ),
        (
            "switch (1) { case 2 ... 3: h(0); break; case 1: break; }",
            gnu,
            false,
        ),
        (
            "switch (1) { case 1: break; case 2 ... 3: h(0); }",
            gnu,
            true,
        ),
        // Calls that do not return: attributes, `_Noreturn`, library
        // functions, function pointers of a `noreturn` type, and casts of
        // a function designator.
        ("die(); h(0);", c99, false),
        ("stop(); h(0);", c99, false),
        ("exit(0); h(0);", c99, false),
        ("_exit(0); h(0);", gnu, false),
        ("_exit(0); h(0);", c99, true),
        ("exit(0); h(0);", c99.with_hosted(false), true),
        ("fp(); h(0);", c99, false),
        ("fpt(); h(0);", c99, false),
        ("holder.m(); h(0);", c99, false),
        ("__typeof__(die) *q = die; q(); h(0);", gnu, false),
        ("void (*p)(void) = die; p(); h(0);", c99, true),
        ("((void (*)(void))die)(); h(0);", c99, false),
        (
            "(void)_Generic(0, int: die(), default: 0); h(0);",
            c99,
            false,
        ),
        // The rest of the expression is emitted; a condition that does not
        // fold joins reachable code again; a label resumes code generation.
        ("die(), h(0);", c99, true),
        ("(die(), other()); h(0);", c99, false),
        ("c ? die() : (void)0; h(0);", c99, true),
        ("1 ? die() : (void)0; h(0);", c99, false),
        ("(void)(c && (die(), 1)); h(0);", c99, true),
        ("die(); l: h(0);", c99, true),
        ("int y = (die(), 1); h(y);", c99, true),
        ("do die(); while (0); h(0);", c99, true),
        // A statement expression's value statement is emitted after a jump.
        ("({ die(); }); h(0);", gnu, false),
        ("({ die(); (void)h(0); });", gnu, false),
        ("c = ({ die(); h(0); });", gnu, true),
        ("c = ({ return 0; h(0); });", gnu, true),
        // The #113 review's forms: a label or break in a statement
        // expression keeps the switch unfolded; standard `[[noreturn]]`
        // stays with its declaration; a hand-declared `longjmp` needs
        // `jmp_buf`; a complex product joins paths; unevaluated operands
        // change nothing; function types carry `noreturn` through any
        // callee; Clang's evaluator folds more conditions.
        (
            "switch (1) { case 1: ({ break; 0; }); case 2: h(0); }",
            gnu,
            true,
        ),
        (
            "switch (1) { case 1: break; case 2: ({ l: 0; }); h(0); }",
            gnu,
            true,
        ),
        ("switch (3) { case 2: ({ l: 0; }); h(0); }", gnu, true),
        (
            "[[noreturn]] void nd(void); __typeof__(nd) *p = 0; p(); h(0);",
            gnu,
            true,
        ),
        ("void longjmp(long *, int); longjmp(0, 1); h(0);", c99, true),
        ("_Complex double z = 1; die(), (z * z); h(0);", c99, true),
        ("die(), sizeof(c ? 1 : 0); h(0);", c99, false),
        ("die(), (1 ? 0 : (c ? 1 : 0)); h(0);", c99, false),
        ("nr *a[1] = {0}; a[0](); h(0);", c99, false),
        ("(c ? fpt : fpt)(); h(0);", c99, false),
        ("(c, fpt)(); h(0);", c99, false),
        ("((nr *)other)(); h(0);", c99, false),
        ("__typeof__(holder.m) q = 0; q(); h(0);", gnu, false),
        (
            "switch ((0.0, 1)) { case 1: break; default: h(0); }",
            c99,
            false,
        ),
        (
            "switch (({ 1; })) { case 1: break; default: h(0); }",
            gnu,
            false,
        ),
        (
            "switch (&c != 0) { case 1: break; default: h(0); }",
            c99,
            false,
        ),
        (
            "switch (1) { case 1: break; int v; case 2: h(0); }",
            c99,
            false,
        ),
        ("if (__builtin_constant_p(c)) h(0);", c99, true),
        ("if (__builtin_constant_p((0, 1)) - 1) h(0);", c99, false),
    ] {
        assert_eq!(h_first(probe, config), emitted, "{probe}");
    }
}

/// A weak inline definition is a weak definition, emitted where it is
/// referenced, as Clang emits it; a strong definition elsewhere wins. A
/// GNU89 plain inline definition is an external definition anyway.
#[test]
fn weak_inline_definitions_are_weak_definitions_when_referenced() {
    for gnu89 in [false, true] {
        let config = CompilerConfiguration::default().with_gnu89_inline(gnu89);
        for (source, referenced) in [
            (
                "inline __attribute__((weak)) int f(int x) { return x ? x * f(x - 1) : 1; } int \
                 u(int x) { return f(x); }",
                true,
            ),
            (
                "inline __attribute__((weak, always_inline)) int f(int x) { return x; } int u(int \
                 x) { return f(x); }",
                true,
            ),
            (
                "inline __attribute__((weak)) int f(int x) { return x; }",
                false,
            ),
        ] {
            let text = lower_with(source, config).unwrap_or_else(|error| panic!("{error:?}"));
            assert_eq!(
                text.contains("function @f(i32 noundef) -> i32 weak"),
                referenced || gnu89,
                "gnu89={gnu89}: {source}\n{text}"
            );
            assert!(!text.contains("@f.always_inline"), "{text}");
        }
    }
}
