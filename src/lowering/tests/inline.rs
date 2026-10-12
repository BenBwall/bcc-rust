//! External emission follows C99 §6.7.4p6-7 and GCC's GNU89 inline rules.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
};

fn has_body(source: &str, config: CompilerConfiguration) -> bool {
    let text = lower_with(source, config).unwrap();
    let arena = Bump::new();
    let module = parse_module(&arena, &text).unwrap();
    let crate::ir::Symbol::Function(id) = module.symbol("f").unwrap() else {
        panic!("{text}");
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
                ("static inline int f(void) { return 7; }", true),
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
            true,
        ),
    ] {
        assert_eq!(
            has_body(source, CompilerConfiguration::default()),
            defined,
            "{source}"
        );
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
