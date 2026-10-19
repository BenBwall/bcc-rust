//! GNU inline reductions from MinGW's intrinsic declarations.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::TranslationError,
};

#[test]
fn gnu89_inline_emission_follows_the_target_and_explicit_attribute() {
    use crate::target::Target;
    for target in [Target::WindowsMsvc, Target::WindowsGnu, Target::LinuxGnu] {
        for standard in [CStandard::C89, CStandard::C99] {
            for attribute in [false, true] {
                let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_gnu_extensions(true)
                    .with_gnu89_inline(true)
                    .with_target(target);
                let source = format!(
                    "extern inline {}int f(void){{return 7;}} int main(void){{return f()!=7;}}",
                    if attribute {
                        "__attribute__((gnu_inline)) "
                    } else {
                        ""
                    }
                );
                with_configuration(&source, config, |context, unit| {
                    assert_eq!(unit.errors, 0, "{:?}", context.take_pending_errors());
                    let definition = unit
                        .definitions
                        .iter()
                        .find(|d| {
                            context.string_cache.at(unit.bindings[d.binding].name.name) == "f"
                        })
                        .unwrap();
                    assert_eq!(
                        definition.kind,
                        if target == Target::WindowsMsvc && !attribute {
                            DefinitionKind::Function
                        } else {
                            DefinitionKind::Inline
                        },
                        "{target:?} {standard:?} attribute={attribute}"
                    );
                });
            }
        }
    }
}

fn kinds(source: &str, configuration: CompilerConfiguration) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(source, configuration, |context, _| {
        for error in context.take_pending_errors() {
            if let TranslationError::Semantic(error) = error {
                kinds.push(error.kind);
            } else {
                panic!("{source}: {error:?}");
            }
        }
    });
    kinds
}

#[test]
fn gnu_extern_inline_can_be_redeclared_static_or_redefined() {
    for standard in [CStandard::C99, CStandard::C17, CStandard::C23] {
        let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow);
        for source in [
            "void f(void); extern inline __attribute__((gnu_inline)) void f(void) {} static \
             inline void f(void) {}\n",
            "extern inline __attribute__((__gnu_inline__)) void f(void); static void f(void) {}\n",
            "extern inline __attribute__((gnu_inline)) void f(void) {} void f(void) {}\n",
            "extern inline __attribute__((gnu_inline)) void f(void) {} extern inline \
             __attribute__((gnu_inline)) void f(void) {}\n",
        ] {
            assert_eq!(kinds(source, configuration), [], "{source}");
        }
        assert!(
            kinds(
                "void f(void); static inline void f(void) {}\n",
                configuration
            )
            .contains(&SemanticErrorKind::ConflictingLinkage)
        );
        assert!(
            kinds(
                "inline void f(void) {} static void f(void) {}\n",
                configuration
            )
            .contains(&SemanticErrorKind::ConflictingLinkage)
        );
        assert!(
            kinds("void f(void) {} void f(void) {}\n", configuration)
                .contains(&SemanticErrorKind::DuplicateDefinition)
        );
    }
}

#[test]
fn inline_mode_overrides_control_replacement_and_c99_body_restrictions() {
    let source = "static int x; inline int f(void) { static int y; return x + y; }";
    for standard in [CStandard::C89, CStandard::C99, CStandard::C17] {
        let config =
            CompilerConfiguration::new(standard, ExtensionPolicy::Allow).with_gnu_extensions(true);
        assert_eq!(kinds(source, config.with_gnu89_inline(true)), []);
        let errors = kinds(source, config.with_gnu89_inline(false));
        assert!(
            errors.contains(&SemanticErrorKind::InlineStaticObject),
            "{errors:?}"
        );
        assert!(
            errors.contains(&SemanticErrorKind::InlineInternalReference),
            "{errors:?}"
        );
        let replace = "extern inline int f(void) { return 1; } int f(void) { return 7; }";
        assert_eq!(kinds(replace, config.with_gnu89_inline(true)), []);
        assert!(
            kinds(replace, config.with_gnu89_inline(false))
                .contains(&SemanticErrorKind::DuplicateDefinition)
        );
    }
}

#[test]
fn gnu_extern_inline_with_inherited_internal_linkage_defines_the_function() {
    let configuration = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow);
    assert_eq!(
        kinds(
            "static void f(void); extern inline __attribute__((gnu_inline)) void f(void) {} int \
             main(void) { f(); }",
            configuration,
        ),
        [],
    );
    assert_eq!(
        kinds(
            "static void f(void); extern inline __attribute__((gnu_inline)) void f(void) {} void \
             f(void) {} int main(void) { f(); }",
            configuration,
        ),
        [],
    );
}

#[test]
fn invalid_or_late_gnu_inline_does_not_allow_redefinition() {
    for standard in [CStandard::C99, CStandard::C17, CStandard::C23] {
        for target in [
            crate::target::Target::LinuxGnu,
            crate::target::Target::WindowsGnu,
        ] {
            let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_target(target);
            for (source, expected) in [
                (
                    "__attribute__((gnu_inline)) void f(void); extern inline void f(void) {} void \
                     f(void) {}",
                    SemanticErrorKind::DuplicateDefinition,
                ),
                (
                    "void f(void) {} extern inline __attribute__((gnu_inline)) void f(void); \
                     static void f(void);",
                    SemanticErrorKind::ConflictingLinkage,
                ),
            ] {
                assert!(kinds(source, configuration).contains(&expected), "{source}");
            }
            assert_eq!(
                kinds(
                    "inline __attribute__((gnu_inline)) void f(void); extern inline void f(void) \
                     {} void f(void) {}",
                    configuration,
                ),
                [],
            );
            assert_eq!(
                kinds(
                    "inline __attribute__((gnu_inline)) void f(void); void f(void) {} extern \
                     inline void f(void); static void f(void);",
                    configuration,
                ),
                [],
            );
        }
    }
}
