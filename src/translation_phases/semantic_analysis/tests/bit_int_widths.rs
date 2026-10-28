//! `_BitInt(N)` widths: an integer constant expression, at least 2 for a
//! signed type and 1 for an unsigned one, and at most `BITINT_MAXWIDTH`,
//! checked as GCC and Clang read the width. C23 §6.7.3.1p4.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

/// The semantic error kinds of `source` in `standard` under `identity`.
fn kinds(source: &str, standard: CStandard, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
            .with_compiler_identity(identity),
        |context, _| {
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    result.push(error.kind);
                }
            }
        },
    );
    result
}

#[test]
fn widths_follow_each_identity() {
    use SemanticErrorKind::{
        BitIntWidthTooLarge as Large,
        InvalidConstant as NotConstant,
        NonPositiveBitIntWidth as NotPositive,
        SignedBitIntWidth as Signed,
    };
    // The type, GCC's error and Clang's error.
    for (ty, gcc, clang) in [
        ("_BitInt(2)", None, None),
        ("unsigned _BitInt(1)", None, None),
        ("_BitInt(3 + 5)", None, None),
        ("_BitInt((char)300)", None, None),
        ("_BitInt(sizeof(int) * 2)", None, None),
        ("_BitInt(65535)", None, None),
        ("_BitInt(1)", Some(Signed), Some(Signed)),
        ("signed _BitInt(1)", Some(Signed), Some(Signed)),
        ("_BitInt(0)", Some(NotPositive), Some(Signed)),
        ("unsigned _BitInt(0)", Some(NotPositive), Some(NotPositive)),
        // Clang reads -1 as the 32 bits of its `int`, too many bits.
        ("_BitInt(-1)", Some(NotPositive), Some(Large)),
        ("unsigned _BitInt(-1)", Some(NotPositive), Some(Large)),
        // ... and a `signed char` -1 as 255 bits, a `short` -2 as 65534.
        ("_BitInt((signed char)-1)", Some(NotPositive), None),
        ("_BitInt((short)-2)", Some(NotPositive), None),
        ("_BitInt(65536)", Some(Large), None),
        ("_BitInt(8388608)", Some(Large), None),
        ("_BitInt(8388609)", Some(Large), Some(Large)),
        ("_BitInt(4294967298LL)", Some(Large), Some(Large)),
        (
            "_BitInt(-9223372036854775807LL - 1)",
            Some(NotPositive),
            Some(Large),
        ),
        ("_BitInt(2.0)", Some(NotConstant), Some(NotConstant)),
        ("_BitInt(n)", Some(NotConstant), Some(NotConstant)),
    ] {
        for (identity, expected) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            for standard in [CStandard::C23, CStandard::C17] {
                assert_eq!(
                    kinds(&format!("int n; {ty} x;"), standard, identity),
                    expected.into_iter().collect::<Vec<_>>(),
                    "{identity:?} {standard:?}: {ty}"
                );
            }
        }
    }
}

#[test]
fn every_type_name_checks_its_width() {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            kinds(
                "typedef _BitInt(1) T; int s = sizeof(_BitInt(1)); int g = _Generic(0, \
                 _BitInt(1): 1, default: 0); void f(void) { (void)(_BitInt(1))3; int m = 4; \
                 (void)sizeof(unsigned _BitInt(m)); } struct S { _BitInt(1) member; };",
                CStandard::C23,
                identity,
            ),
            [
                SemanticErrorKind::SignedBitIntWidth,
                SemanticErrorKind::SignedBitIntWidth,
                SemanticErrorKind::SignedBitIntWidth,
                SemanticErrorKind::SignedBitIntWidth,
                SemanticErrorKind::InvalidConstant,
                SemanticErrorKind::SignedBitIntWidth,
            ],
            "{identity:?}"
        );
    }
}

#[test]
fn a_compound_literal_width_folds_only_for_clang() {
    // GCC: "'_BitInt' argument is not an integer constant expression";
    // Clang folds the literal, under `-Wgnu-folding-constant` with
    // `-pedantic`.
    assert_eq!(
        kinds(
            "_BitInt((int){8}) x;",
            CStandard::C23,
            CompilerIdentity::Gcc
        ),
        [SemanticErrorKind::InvalidConstant]
    );
    assert_eq!(
        kinds(
            "_BitInt((int){8}) x;",
            CStandard::C23,
            CompilerIdentity::Clang
        ),
        []
    );
}
