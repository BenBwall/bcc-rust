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
fn arithmetic_wider_than_128_bits_folds_where_the_model_holds_it() {
    // A wide value stands for its 128-bit pattern sign-extended, which
    // arithmetic that does not overflow `__int128` keeps.
    holds_on_x86_64(
        "typedef _BitInt(200) W; typedef unsigned _BitInt(200) U;\nenum { A = (int)((W)1 + 1), B \
         = (int)((W)-5 * 7), C = (int)(((U)0 - 3) & 0xff) };\n_Static_assert(A == 2 && B == -35 \
         && C == 253, \"\");\n_Static_assert((W)-1 < 0 && (U)-1 > 0 && (U)-1 > (U)5, \
         \"\");\n_Static_assert((W)100 / -7 == -14 && (W)100 % -7 == 2, \
         \"\");\n_Static_assert((int)~(U)5 == -6 && -(W)9 == -9, \"\");\n_Static_assert(((W)1 << \
         100 >> 98) == 4 && (1 << (W)3) == 8, \"\");\n_Static_assert((W)-3 >= (W)-4 && !(W)0 && \
         ((W)1 && 1), \"\");\n_Static_assert(__builtin_add_overflow_p((W)0x7fffffff, 1, (int)0), \
         \"\");\n_Static_assert(sizeof(int[(W)3 * 2]) == 6 * sizeof(int), \
         \"\");\n_Static_assert(__builtin_choose_expr((W)1 + 1, 1, 0), \"\");\n_Alignas((W)8 * 2) \
         int aligned;\n_Static_assert(_Alignof(aligned) == 16, \"\");\nstatic long long product = \
         (W)1000000000000LL * 3 + 1;\nint bounds[(W)1 + 3];\nstruct S { int a[(W)1 + 3]; int x : \
         (W)2 + 3; };\nint f(int x) { switch (x) { case (W)1 * 3: return 1; } return 0; }",
    );
}

#[test]
fn wide_constants_the_model_cannot_hold_are_refused_clearly() {
    use SemanticErrorKind::{
        ConstantOverflow as Overflow,
        UnsupportedWideConstant as Wide,
    };
    // A huge unsigned value divided, shifted right or past 128 bits, and an
    // unsigned 128-bit value with its top bit set, zero-extended, have no
    // folded value; every context that requires a constant says so. `H`
    // stands for such a constant.
    for (source, expected) in [
        ("enum { E = H };", &[Wide][..]),
        ("int a[H + 1];", &[Wide]),
        ("struct S { int a[H + 1]; };", &[Wide]),
        (
            "int f(void) { static int b[H + 1]; return sizeof b; }",
            &[Wide],
        ),
        ("_Alignas(H ? 8 : 16) int x;", &[Wide]),
        ("static long long m = H;", &[Wide]),
        ("int c = __builtin_choose_expr(H, 1, 2);", &[Wide]),
        ("_Static_assert(sizeof(int[H + 1]) == 8, \"\");", &[Wide]),
        ("struct T { int x : H + 1; };", &[Wide]),
        (
            "int f(int x) { switch (x) { case (_BitInt(200))1 << 150: return 1; } return 0; }",
            &[Wide],
        ),
        (
            "_Static_assert((unsigned _BitInt(129))(unsigned __int128)-1 != 0, \"\");",
            &[Wide],
        ),
        // Undefined at any width.
        ("enum { E = (int)((_BitInt(200))1 / 0) };", &[Overflow]),
        // Conversions fold, so these are constants.
        ("enum { F = (_BitInt(200))1 };", &[]),
        ("int a[(_BitInt(200))3 ? 2 : 1];", &[]),
        ("static unsigned _BitInt(200) t = -1;", &[]),
        (
            "int f(void) { switch ((_BitInt(200))1) { case 1: return 1; } return 0; }",
            &[],
        ),
    ] {
        let source = format!("#define H ((unsigned _BitInt(200))-1 / 2 > 1)\n{source}");
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            assert_eq!(
                kinds(&source, identity, crate::target::Target::LinuxGnu),
                expected,
                "{identity:?}: {source}"
            );
        }
    }
}

#[test]
fn shifts_check_the_bit_precise_width() {
    // The count and the result take the `_BitInt`'s width, not the `int`
    // its operand computes in: these are undefined.
    for source in [
        "enum { E = (int)((_BitInt(31))3 << 30) };",
        "enum { E = (int)((_BitInt(31))7 << 29) };",
        "enum { E = (int)((unsigned _BitInt(8))1 << 8) };",
        "enum { E = (int)((unsigned _BitInt(7))100 >> 7) };",
    ] {
        assert_eq!(
            kinds(
                source,
                CompilerIdentity::Gcc,
                crate::target::Target::LinuxGnu
            ),
            [SemanticErrorKind::ConstantOverflow],
            "{source}"
        );
    }
    holds_on_x86_64(
        "_Static_assert(((_BitInt(31))1 << 30) == -1073741824, \"\");\n_Static_assert(((unsigned \
         _BitInt(8))1 << 7) == 128, \"\");\n_Static_assert(((_BitInt(8))-128 >> 7) == -1, \
         \"\");\n_Static_assert(((_BitInt(128))1 << 126) > 0, \"\");",
    );
}

#[test]
fn atomic_builtins_take_a_bit_int_as_each_compiler_does() {
    use SemanticErrorKind::{
        AtomicBuiltinBitInt as BitInt,
        AtomicOperandSize as Size,
    };
    // Clang refuses the `__atomic` value builtins and a `__sync` builtin
    // on a width that is no power of two; GCC accepts every one.
    for (operation, eight, seven) in [
        ("__atomic_fetch_add(&p, 1, 5)", Some(BitInt), Some(BitInt)),
        ("__atomic_load_n(&p, 5)", Some(BitInt), Some(BitInt)),
        ("__atomic_test_and_set(&p, 5)", None, None),
        ("__sync_fetch_and_add(&p, 1)", None, Some(Size)),
        ("__sync_val_compare_and_swap(&p, 1, 2)", None, Some(Size)),
        ("__sync_lock_test_and_set(&p, 1)", None, Some(Size)),
    ] {
        for (width, expected) in [(8, eight), (7, seven)] {
            let source = format!(
                "unsigned _BitInt({width}) p; int f(void) {{ return (int)({operation}); }}"
            );
            assert_eq!(
                kinds(
                    &source,
                    CompilerIdentity::Gcc,
                    crate::target::Target::LinuxGnu
                ),
                [],
                "{source}"
            );
            assert_eq!(
                kinds(
                    &source,
                    CompilerIdentity::Clang,
                    crate::target::Target::LinuxGnu
                ),
                expected.into_iter().collect::<Vec<_>>(),
                "{source}"
            );
        }
    }
}

/// Clang refuses `__builtin_mul_overflow` with a signed `_BitInt` of more
/// than 128 bits among its operands and result; GCC accepts every one, as
/// both accept the unsigned ones, narrower ones and the other operations.
#[test]
fn clang_refuses_wide_signed_overflow_products() {
    for (call, refused) in [
        ("__builtin_mul_overflow(a, a, &r)", true),
        ("__builtin_mul_overflow(1, 2, &r)", true),
        ("__builtin_mul_overflow(1, a, &i)", true),
        ("__builtin_mul_overflow(u, 2, &i)", false),
        ("__builtin_mul_overflow(1, 2, &u)", false),
        ("__builtin_mul_overflow(n, n, &n)", false),
        ("__builtin_add_overflow(a, a, &r)", false),
        ("__builtin_sub_overflow(a, 1, &r)", false),
    ] {
        let source = format!(
            "_BitInt(200) a, r; unsigned _BitInt(200) u; _BitInt(128) n; int i;\n\
             int f(void) {{ return {call}; }}"
        );
        assert_eq!(
            kinds(&source, CompilerIdentity::Gcc, crate::target::Target::LinuxGnu),
            [],
            "{source}"
        );
        let expected: &[SemanticErrorKind] = if refused {
            &[SemanticErrorKind::OverflowBuiltinBitInt]
        } else {
            &[]
        };
        assert_eq!(
            kinds(&source, CompilerIdentity::Clang, crate::target::Target::LinuxGnu),
            expected,
            "{source}"
        );
    }
}

/// Asserts that `source` has no semantic errors for either identity on
/// Linux x86-64.
fn holds_on_x86_64(source: &str) {
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            kinds(source, identity, crate::target::Target::LinuxGnu),
            [],
            "{identity:?}: {source}"
        );
    }
}

#[test]
fn generated_wide_constants_fold_to_their_exact_values() {
    // Each assertion holds in GCC 15 and Clang; a wrong folded value fails
    // it.
    holds_on_x86_64(include_str!(
        "../../../../tests/fixtures/semantic/bit_int_wide_folded.c"
    ));
    // These the pattern model cannot hold: each is refused, none folds to
    // a wrong value.
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        let errors = kinds(
            include_str!("../../../../tests/fixtures/semantic/bit_int_wide_refused.c"),
            identity,
            crate::target::Target::LinuxGnu,
        );
        assert!(errors.len() >= 40, "{identity:?}: {errors:?}");
        assert!(
            errors
                .iter()
                .all(|&kind| kind == SemanticErrorKind::UnsupportedWideConstant),
            "{identity:?}: {errors:?}"
        );
    }
}

#[test]
fn a_wide_unsigned_pattern_zero_extends() {
    // `(unsigned _BitInt(150))-1` is 2^150 - 1: converted to a wider type
    // it zero-extends, which no sign-extended 128-bit pattern holds, so it
    // is refused rather than folded as -1 (GCC and Clang give E=1, g=0).
    for source in [
        "typedef unsigned _BitInt(150) u150; enum { E = (_BitInt(200))(u150)-1 > 0 };",
        "typedef unsigned _BitInt(150) u150; static const int g = (unsigned _BitInt(256))(u150)-1 \
         == (unsigned _BitInt(256))-1;",
    ] {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            assert_eq!(
                kinds(source, identity, crate::target::Target::LinuxGnu),
                [SemanticErrorKind::UnsupportedWideConstant],
                "{identity:?}: {source}"
            );
        }
    }
    // Within its own width, or narrower, the pattern still folds.
    holds_on_x86_64(
        "_Static_assert((unsigned _BitInt(150))-1 > 0, \"\");\n_Static_assert((unsigned long \
         long)(unsigned _BitInt(150))-1 == -1ULL, \
         \"\");\n_Static_assert((_BitInt(200))(_BitInt(150))-1 < 0, \"\");",
    );
}

#[test]
fn sync_builtins_refuse_a_bit_int_above_16_bytes() {
    for width in [200, 256, 512] {
        let source = format!(
            "unsigned _BitInt({width}) p; int f(void) {{ return (int)__sync_fetch_and_add(&p, 1); \
             }}"
        );
        assert_eq!(
            kinds(
                &source,
                CompilerIdentity::Clang,
                crate::target::Target::LinuxGnu
            ),
            [SemanticErrorKind::AtomicOperandBytes],
            "{source}"
        );
        assert_eq!(
            kinds(
                &source,
                CompilerIdentity::Gcc,
                crate::target::Target::LinuxGnu
            ),
            [SemanticErrorKind::InvalidAtomicOperand],
            "{source}"
        );
    }
}
