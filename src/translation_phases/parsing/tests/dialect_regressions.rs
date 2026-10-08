//! Regressions for language-mode syntax: GNU `__extension__` scoping, C23
//! storage-class and alignment rules, and the lookahead that separates
//! specifiers, attributes, and labels.

use super::{
    Parsed,
    parser_errors,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::TranslationError,
};
fn mode(standard: CStandard, gnu: bool, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu)
}

fn extensions(parsed: &Parsed<'_, '_>) -> Vec<String> {
    parsed
        .errors
        .iter()
        .filter_map(|error| match error {
            | TranslationError::Extension(extension) => Some(extension.to_string()),
            | _ => None,
        })
        .collect()
}

fn assert_clean_parse(parsed: &Parsed<'_, '_>, source: &str) {
    assert_eq!(
        parser_errors(parsed).count(),
        0,
        "{source}\n{:?}",
        parsed.errors
    );
    assert!(parsed.parser.frames.is_empty(), "{source}");
    assert_eq!(parsed.parser.pedantic_suppression, 0, "{source}");
}

#[test]
fn extension_marker_before_a_member_terminates_and_suppresses_only_that_member() {
    // glibc's `bits/atomic_wide_counter.h` shape.
    let glibc = "typedef union { __extension__ unsigned long long int __value64; struct { \
                 unsigned int __low, __high; } __value32; } W;\nstruct S { __extension__ union \
                 { int a; float b; }; int c; };\n";
    for (standard, gnu) in [
        (CStandard::C89, false),
        (CStandard::C99, false),
        (CStandard::C11, true),
    ] {
        with_parse_configuration(glibc, mode(standard, gnu, ExtensionPolicy::Deny), |p| {
            assert_clean_parse(p, glibc);
            assert_eq!(p.items.len(), 2);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        });
    }
    let scoped = "struct S { __extension__ long long a; long long b; };\n";
    with_parse_configuration(
        scoped,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, scoped);
            let extensions = extensions(p);
            assert_eq!(extensions.len(), 1, "{extensions:?}");
            assert!(extensions[0].contains("long long"));
        },
    );
}

