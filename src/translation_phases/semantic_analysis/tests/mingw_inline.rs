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
