//! `u8`, `u` and `U` literals: element types, array completion and the
//! initializer compatibility rules (C11 §6.4.5p6, §6.7.9p14-15, p22).

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
};

/// Semantic error kinds of `source` for `target` in `standard`.
fn kinds(source: &str, standard: CStandard, target: Target) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(standard, ExtensionPolicy::Allow).with_target(target),
        |context, _| {
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(e) => kinds.push(e.kind),
                    | other => panic!("{source}: unexpected {other:?}"),
                }
            }
        },
    );
    kinds
}

const TARGETS: [Target; 3] = [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc];

#[test]
fn encoded_strings_complete_arrays_of_unknown_size() {
    // #58: `L""` completed an array; `u""` and `U""` left it incomplete.
    let source = r#"
        typedef unsigned short char16_t; typedef unsigned int char32_t;
        char16_t a[] = u"ab"; char32_t b[] = U"xyz"; char c[] = u8"hé";
        char16_t d[] = u"\U0001F600"; const char32_t e[] = {U"q"};
        struct S { char16_t s[4]; int n; } st = { u"hi", 7 };
        _Static_assert(sizeof a == 6 && sizeof b == 16 && sizeof c == 4, "sizes");
        _Static_assert(sizeof d == 6 && sizeof e == 8, "surrogates and braces");
        _Static_assert(_Generic(u"a"[0], unsigned short: 1, default: 0), "char16_t");
        _Static_assert(_Generic(U"a"[0], unsigned int: 1, default: 0), "char32_t");
        _Static_assert(_Generic(u'a', unsigned short: 1, default: 0), "u constant");
        _Static_assert(_Generic(U'a', unsigned int: 1, default: 0), "U constant");
        _Static_assert(sizeof(u8"ab") == 3 && sizeof(u"ab") == 6 && sizeof(U"ab") == 12, "literals");
        void f(void) { char16_t l[] = u"local"; char32_t m[3] = U"ab"; (void)l; (void)m; }
    "#;
    for target in TARGETS {
        assert_eq!(kinds(source, CStandard::C11, target), [], "{target:?}");
    }
}

#[test]
fn utf8_strings_have_char8_t_elements_in_c23() {
    let source = r#"
        _Static_assert(_Generic(u8"a"[0], char: 1, default: 0) == C11, "C11 char");
        _Static_assert(_Generic(u8"a"[0], unsigned char: 1, default: 0) != C11, "C23 char8_t");
        char a[] = u8"x"; unsigned char b[] = u8"x"; signed char c[] = u8"x";
    "#;
    assert_eq!(
        kinds(
            &format!("#define C11 1\n{source}"),
            CStandard::C11,
            Target::LinuxGnu
        ),
        []
    );
    assert_eq!(
        kinds(
            &format!("#define C11 0\n{source}"),
            CStandard::C23,
            Target::LinuxGnu
        ),
        []
    );
}

#[test]
fn wide_arrays_take_only_their_own_encoding() {
    // C11 §6.7.9p15: a `char32_t` array is not initialized by `u""`, nor a
    // `char16_t` array by `U""` or a narrow string.
    for (source, target) in [
        ("unsigned int a[] = u\"x\";", Target::WindowsGnu),
        ("unsigned short a[] = U\"x\";", Target::LinuxGnu),
        ("unsigned short a[] = \"x\";", Target::LinuxGnu),
        ("int a[] = u\"x\";", Target::LinuxGnu),
    ] {
        assert!(
            !kinds(source, CStandard::C11, target).is_empty(),
            "{source} {target:?}"
        );
    }
}
