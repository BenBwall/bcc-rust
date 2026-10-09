//! MinGW reductions checked through the parser pipeline.

use super::*;

#[test]
fn tagged_member_declarations_warn_without_declaring_a_member() {
    let source =
        "struct Outer { struct Inner { int x; }; struct Forward; enum E { A }; int y; };\n";
    with_parse(source, |p| {
        assert_eq!(p.errors.len(), 3, "{:?}", p.errors);
        assert!(
            p.errors
                .iter()
                .all(|e| e.severity() == ErrorSeverity::Warning),
            "{:?}",
            p.errors
        );
    });
}

#[test]
fn typedef_and_scalar_member_declarations_follow_clang_policy() {
    use crate::configuration::{
        CStandard,
        ExtensionPolicy,
    };
    let source = "typedef struct { int x; } T; struct S { T; int; int y; };\n";
    for standard in [CStandard::C99, CStandard::C17, CStandard::C23] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(source, CompilerConfiguration::new(standard, policy), |p| {
                assert_eq!(p.errors.len(), 2, "{:?}", p.errors);
                let severity = if policy == ExtensionPolicy::Deny {
                    ErrorSeverity::Error
                } else {
                    ErrorSeverity::Warning
                };
                assert!(
                    p.errors.iter().all(|e| e.severity() == severity),
                    "{:?}",
                    p.errors
                );
            });
        }
    }
}
