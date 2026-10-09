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
