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
        // Clang folds an arithmetic constant that is no integer constant
        // expression; GCC does not, in any mode.
        ("_BitInt(1 ? 8 : n)", Some(NotConstant), None),
        ("_BitInt(0 ? n : 8)", Some(NotConstant), None),
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

#[test]
fn gcc_reports_the_type_before_c23_only_with_a_valid_width() {
    // GCC names the type with its width ("ISO C does not support
    // '_BitInt(8)' before C23"); Clang reports the keyword.
    for (identity, valid, invalid) in [
        (CompilerIdentity::Gcc, 1, 0),
        (CompilerIdentity::Clang, 1, 1),
    ] {
        for (source, expected) in [("_BitInt(8) x;", valid), ("_BitInt(1) x;", invalid)] {
            let mut extensions = 0;
            with_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Warn)
                    .with_compiler_identity(identity),
                |context, _| {
                    extensions = context
                        .take_pending_errors()
                        .iter()
                        .filter(|error| matches!(error, TranslationError::Extension(_)))
                        .count();
                },
            );
            assert_eq!(extensions, expected, "{identity:?}: {source}");
        }
    }
}

#[test]
fn bit_int_maxwidth_follows_the_identity() {
    for (identity, width) in [
        (CompilerIdentity::Gcc, 65535),
        (CompilerIdentity::Tcc, 65535),
        (CompilerIdentity::Msvc, 65535),
        (CompilerIdentity::Clang, 8_388_608),
        (CompilerIdentity::ClangCl, 8_388_608),
    ] {
        // The widest width the macro names is valid, and one more is not.
        assert_eq!(
            kinds(
                &format!(
                    "_Static_assert(__BITINT_MAXWIDTH__ == {width}, \"\"); unsigned \
                     _BitInt(__BITINT_MAXWIDTH__) x; unsigned _BitInt(__BITINT_MAXWIDTH__ + 1) y;"
                ),
                CStandard::C23,
                identity,
            ),
            [SemanticErrorKind::BitIntWidthTooLarge],
            "{identity:?}"
        );
    }
}
