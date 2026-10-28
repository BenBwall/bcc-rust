//! `_BitInt(N)` as a type: its layout on each target, the usual
//! arithmetic conversions with bit-precise ranks, the types of `wb`
//! constants, folded arithmetic at its width, and Clang's refusal of an
//! atomic one. C23 §6.2.5p5-8, §6.3.1.1p1-2, §6.3.1.8p1, §6.4.4.2p6.

use super::*;
use crate::configuration::{
    CStandard,
    CompilerConfiguration,
    CompilerIdentity,
    ExtensionPolicy,
};

/// The semantic error kinds of `source` in C23 under `identity` for
/// `target`.
fn kinds(
    source: &str,
    identity: CompilerIdentity,
    target: crate::target::Target,
) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_compiler_identity(identity)
            .with_target(target),
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

/// Asserts that `source` has no semantic errors for any identity on any
/// target.
fn holds(source: &str) {
    for target in [
        crate::target::Target::LinuxGnu,
        crate::target::Target::WindowsMsvc,
        crate::target::Target::Wasm32Wasi,
    ] {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            assert_eq!(
                kinds(source, identity, target),
                [],
                "{identity:?} {target:?}: {source}"
            );
        }
    }
}

#[test]
fn layout_follows_the_psabi_on_every_target() {
    // Up to 64 bits the smallest of char, short, int and long long; beyond,
    // 64-bit limbs aligned to 8 bytes, as GCC and Clang lay them out.
    for (width, size, align) in [
        (2, 1, 1),
        (8, 1, 1),
        (9, 2, 2),
        (16, 2, 2),
        (17, 4, 4),
        (32, 4, 4),
        (33, 8, 8),
        (64, 8, 8),
        (65, 16, 8),
        (127, 16, 8),
        (128, 16, 8),
    ] {
        holds(&format!(
            "_Static_assert(sizeof(_BitInt({width})) == {size}, \"\"); \
             _Static_assert(_Alignof(unsigned _BitInt({width})) == {align}, \"\"); struct s {{ \
             char c; _BitInt({width}) b; }}; _Static_assert(sizeof(struct s) == {size} + {align}, \
             \"\");"
        ));
    }
    let wide = "_Static_assert(sizeof(_BitInt(129)) == 24, \"\"); \
                _Static_assert(sizeof(_BitInt(1000)) == 128, \"\"); \
                _Static_assert(_Alignof(_BitInt(1000)) == 8, \"\");";
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(kinds(wide, identity, crate::target::Target::LinuxGnu), []);
    }
}

#[test]
fn a_sizeof_in_a_width_is_range_checked() {
    use SemanticErrorKind::{
        BitIntWidthTooLarge as Large,
        NonPositiveBitIntWidth as NotPositive,
        SignedBitIntWidth as Signed,
    };
    for (ty, gcc, clang) in [
        ("_BitInt(sizeof(_BitInt(16)) * 65536)", Some(Large), None),
        (
            "_BitInt(sizeof(_BitInt(16)) * 0)",
            Some(NotPositive),
            Some(Signed),
        ),
        ("_BitInt(sizeof(_BitInt(16)) * 4)", None, None),
    ] {
        for (identity, expected) in [
            (CompilerIdentity::Gcc, gcc),
            (CompilerIdentity::Clang, clang),
        ] {
            assert_eq!(
                kinds(
                    &format!("{ty} x;"),
                    identity,
                    crate::target::Target::LinuxGnu
                ),
                expected.into_iter().collect::<Vec<_>>(),
                "{identity:?}: {ty}"
            );
        }
    }
}

#[test]
fn the_usual_arithmetic_conversions_use_bit_precise_ranks() {
    // A bit-precise type is not promoted; of equal widths the standard type
    // ranks higher, and a signed type wins only if it is wider.
    holds(
        "#define IS(e, t) _Static_assert(_Generic((e), t: 1, default: 0), #e)\nIS((_BitInt(8))0 + \
         (_BitInt(8))0, _BitInt(8));\nIS(-(_BitInt(8))1, _BitInt(8));\nIS(~(unsigned \
         _BitInt(7))1, unsigned _BitInt(7));\nIS((_BitInt(8))1 << 3L, \
         _BitInt(8));\nIS((_BitInt(8))0 + (char)0, int);\nIS((_BitInt(7))0 + (unsigned \
         _BitInt(7))0, unsigned _BitInt(7));\nIS((_BitInt(32))0 + 0, int);\nIS((_BitInt(32))0 + \
         0U, unsigned);\nIS((unsigned _BitInt(32))0 + 0, unsigned);\nIS((_BitInt(33))0 + 0U, \
         _BitInt(33));\nIS((unsigned _BitInt(33))0 + 0, unsigned _BitInt(33));\nIS((_BitInt(64))0 \
         + 0LL, long long);\nIS((_BitInt(64))0 + 0ULL, unsigned long long);\nIS((_BitInt(65))0 + \
         0ULL, _BitInt(65));\nIS((_BitInt(128))0 + (__int128)0, __int128);\nIS((_BitInt(128))0 + \
         (unsigned __int128)0, unsigned __int128);\nIS((unsigned _BitInt(128))0 + (__int128)0, \
         unsigned __int128);\nIS((_BitInt(40))0 + (unsigned _BitInt(33))0, \
         _BitInt(40));\nIS((_BitInt(33))0 + (unsigned _BitInt(40))0, unsigned \
         _BitInt(40));\nIS((_BitInt(40))0 + (unsigned _BitInt(40))0, unsigned \
         _BitInt(40));\nIS((_BitInt(10))0 * 1.0f, float);\nIS(1 ? (_BitInt(8))1 : (_BitInt(10))2, \
         _BitInt(10));\nstruct f { _BitInt(40) a : 10; } s;\nIS(s.a + s.a, _BitInt(40));",
    );
}

#[test]
fn bit_int_constants_have_the_narrowest_width() {
    holds(
        "#define IS(e, t) _Static_assert(_Generic((e), t: 1, default: 0), #e)\nIS(0wb, \
         _BitInt(2));\nIS(0uwb, unsigned _BitInt(1));\nIS(1uwb, unsigned _BitInt(1));\nIS(3wb, \
         _BitInt(3));\nIS(3uwb, unsigned _BitInt(2));\nIS(-1wb, \
         _BitInt(2));\nIS(0x7fffffffffffffffwb, _BitInt(64));\nIS(0xffffffffffffffffwb, \
         _BitInt(65));\nIS(0xffffffffffffffffuwb, unsigned _BitInt(64));\n_Static_assert(255uwb + \
         1uwb == 0, \"\");",
    );
}

#[test]
fn constants_fold_at_their_width() {
    holds(
        "_Static_assert((unsigned _BitInt(7))0 - (unsigned _BitInt(7))1 == 127, \
         \"\");\n_Static_assert(~(unsigned _BitInt(7))5 == 122, \"\");\n_Static_assert((unsigned \
         _BitInt(7))100 * (unsigned _BitInt(7))3 == 44, \"\");\n_Static_assert((unsigned \
         _BitInt(7))100 * 3 == 300, \"\");\n_Static_assert((_BitInt(7))-64 / 2 == -32, \
         \"\");\n_Static_assert((unsigned _BitInt(65))-1 >> 64 == 1, \
         \"\");\n_Static_assert((_BitInt(128))-1 < 0, \"\");\n_Static_assert((unsigned \
         _BitInt(128))-1 > 0, \"\");\n_Static_assert((unsigned _BitInt(1))3 == 1, \
         \"\");\n_Static_assert((_BitInt(9))255 == 255, \"\");\n_Static_assert((_BitInt(9))256 == \
         -256, \"\");\nenum { E = (unsigned _BitInt(4))20 };\n_Static_assert(E == 4, \"\");\nint \
         bound[(unsigned _BitInt(3))9];\n_Static_assert(sizeof bound == sizeof(int), \"\");",
    );
}

#[test]
fn a_signed_overflow_at_the_width_is_no_constant() {
    // 63 + 1 overflows `_BitInt(7)`, though not the `int` the constant
    // evaluator computes a narrow operand in.
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_ne!(
            kinds(
                "enum { E = (_BitInt(7))63 + (_BitInt(7))1 };",
                identity,
                crate::target::Target::LinuxGnu
            ),
            [],
            "{identity:?}"
        );
    }
}

#[test]
fn clang_rejects_an_atomic_bit_int() {
    let source = "_Atomic _BitInt(8) a; _Atomic(unsigned _BitInt(65)) b;";
    assert_eq!(
        kinds(
            source,
            CompilerIdentity::Gcc,
            crate::target::Target::LinuxGnu
        ),
        []
    );
    assert_eq!(
        kinds(
            source,
            CompilerIdentity::Clang,
            crate::target::Target::LinuxGnu
        ),
        [
            SemanticErrorKind::InvalidAtomicType,
            SemanticErrorKind::InvalidAtomicType
        ]
    );
}

#[test]
fn a_shift_into_the_sign_bit_folds_as_for_int() {
    // Both compilers fold `(_BitInt(8))1 << 7` to -128 wherever a constant
    // is required, as they fold `1 << 31`.
    holds(
        "enum { A = (_BitInt(8))1 << 7, B = (_BitInt(3))1 << 2 };\n_Static_assert(A == -128 && B \
         == -4, \"\");\n_Static_assert(((_BitInt(8))1 << 7) == -128, \"\");\nstatic _BitInt(8) s \
         = (_BitInt(8))1 << 7;\nint f(_BitInt(8) x) { switch (x) { case (_BitInt(8))1 << 7: \
         return 1; } return 0; }",
    );
    // A shift past the sign bit still overflows.
    assert_ne!(
        kinds(
            "enum { C = (_BitInt(8))3 << 7 };",
            CompilerIdentity::Gcc,
            crate::target::Target::LinuxGnu
        ),
        []
    );
}

#[test]
fn arithmetic_wider_than_128_bits_is_refused_without_panicking() {
    use SemanticErrorKind::UnsupportedWideConstant as Wide;
    for (source, expected) in [
        ("enum { E = (_BitInt(200))1 + 1 };", &[Wide][..]),
        ("_Static_assert(-(_BitInt(200))5 < 0, \"\");", &[Wide]),
        (
            "int f(_BitInt(200) x) { switch (x) { case (_BitInt(200))1 << 150: return 1; } return \
             0; }",
            &[Wide],
        ),
        ("static _BitInt(200) s = (_BitInt(200))1 + 1;", &[Wide]),
        // Conversions fold, so these are constants.
        ("enum { F = (_BitInt(200))1 };", &[]),
        ("int a[(_BitInt(200))3 ? 2 : 1];", &[]),
        ("static unsigned _BitInt(200) t = -1;", &[]),
        (
            "int f(void) { switch ((_BitInt(200))1) { case 1: return 1; } return 0; }",
            &[],
        ),
    ] {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            assert_eq!(
                kinds(source, identity, crate::target::Target::LinuxGnu),
                expected,
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn clang_rejects_atomic_builtins_on_a_bit_int() {
    let source = "unsigned _BitInt(7) p; int f(void) { return __atomic_fetch_add(&p, 1, 5) + \
                  __sync_fetch_and_add(&p, 1); }";
    assert_eq!(
        kinds(
            source,
            CompilerIdentity::Gcc,
            crate::target::Target::LinuxGnu
        ),
        []
    );
    assert_eq!(
        kinds(
            source,
            CompilerIdentity::Clang,
            crate::target::Target::LinuxGnu
        ),
        [
            SemanticErrorKind::InvalidAtomicOperand,
            SemanticErrorKind::InvalidAtomicOperand
        ]
    );
}
