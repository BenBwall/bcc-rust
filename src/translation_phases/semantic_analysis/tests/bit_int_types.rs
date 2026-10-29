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

/// The semantic error kinds of `source` in GNU C23 under `identity` on
/// Linux x86-64, where GCC's folding extensions apply.
fn gnu_kinds(source: &str, identity: CompilerIdentity) -> Vec<SemanticErrorKind> {
    let mut result = Vec::new();
    with_configuration(
        source,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity)
            .with_target(crate::target::Target::LinuxGnu),
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
fn wide_constants_fold_in_every_constant_context() {
    use SemanticErrorKind::{
        ConstantOverflow as Overflow,
        InvalidConstant as Invalid,
    };
    // A huge unsigned value divided, shifted right or past 128 bits, and an
    // unsigned 128-bit value with its top bit set, zero-extended, fold
    // exactly in every context that requires a constant. `H` stands for
    // such a constant, which is 1.
    for (source, expected) in [
        ("enum { E = H };", &[][..]),
        ("int a[H + 1];", &[]),
        ("struct S { int a[H + 1]; };", &[]),
        ("int f(void) { static int b[H + 1]; return sizeof b; }", &[]),
        ("_Alignas(H ? 8 : 16) int x;", &[]),
        ("static long long m = H;", &[]),
        ("int c = __builtin_choose_expr(H, 1, 2);", &[]),
        ("_Static_assert(sizeof(int[H + 1]) == 8, \"\");", &[]),
        ("struct T { int x : H + 1; };", &[]),
        (
            "int f(int x) { switch (x) { case (int)((_BitInt(200))1 << 150 >> 149): return 1; } \
             return 0; }",
            &[],
        ),
        (
            "_Static_assert((unsigned _BitInt(129))(unsigned __int128)-1 != 0, \"\");",
            &[],
        ),
        // Undefined at any width.
        ("enum { E = (int)((_BitInt(200))1 / 0) };", &[Overflow]),
        ("enum { E = (int)((_BitInt(200))1 % 0) };", &[Overflow]),
        (
            "enum { E = (int)(-((_BitInt(200))1 << 199) / -1) };",
            &[Overflow],
        ),
        (
            "enum { E = (int)(((_BitInt(200))1 << 198) * 2) };",
            &[Overflow],
        ),
        (
            "enum { E = (int)(-(-((_BitInt(200))1 << 199))) };",
            &[Overflow],
        ),
        ("enum { E = (int)((_BitInt(200))1 << 200) };", &[Overflow]),
        ("enum { E = (int)((_BitInt(200))-1 << 1) };", &[Overflow]),
        ("enum { E = (int)((_BitInt(200))3 << 199) };", &[Overflow]),
        // A floating value out of range is no constant, as for `(int)1e70`.
        ("enum { E = (int)(_BitInt(200))1e70 };", &[Invalid]),
        ("enum { E = (int)(unsigned _BitInt(200))-1.0 };", &[Invalid]),
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
}

#[test]
fn a_wide_unsigned_value_zero_extends() {
    // `(unsigned _BitInt(150))-1` is 2^150 - 1: converted to a wider type
    // it zero-extends, within its width or narrower it truncates.
    holds_on_x86_64(
        "typedef unsigned _BitInt(150) u150;\n_Static_assert((_BitInt(200))(u150)-1 > 0, \
         \"\");\n_Static_assert((unsigned _BitInt(256))(u150)-1 != (unsigned _BitInt(256))-1, \
         \"\");\n_Static_assert((unsigned _BitInt(256))(u150)-1 == ((unsigned _BitInt(256))1 << \
         150) - 1, \"\");\n_Static_assert((unsigned _BitInt(150))-1 > 0, \
         \"\");\n_Static_assert((unsigned long long)(unsigned _BitInt(150))-1 == -1ULL, \
         \"\");\n_Static_assert((_BitInt(200))(_BitInt(150))-1 < 0, \"\");",
    );
}

#[test]
fn wide_values_fold_exactly_up_to_the_maximum_width() {
    // Division takes Knuth's algorithm D: quotients and remainders of
    // multi-limb operands, with a divisor whose top limb needs normalizing,
    // put back together exactly.
    holds_on_x86_64(
        "typedef unsigned _BitInt(4096) u;\ntypedef _BitInt(4096) s;\n#define A (((u)1 << 2500) - \
         12345)\n#define D (((u)1 << 1200) + ((u)1 << 600) + 7)\n#define R (((u)1 << 1100) + \
         99)\n_Static_assert((A * D + R) / D == A && (A * D + R) % D == R, \
         \"\");\n_Static_assert((s)-A / (s)D == -(s)(A / D) && (s)-A % (s)D == -(s)(A % D), \
         \"\");\n_Static_assert(A / A == 1 && A % A == 0 && D / A == 0 && D % A == D, \
         \"\");\n_Static_assert((((u)1 << 4095) / ((u)1 << 2047)) == (u)1 << 2048, \
         \"\");\n_Static_assert(((u)-1 >> 4000) == ((u)1 << 96) - 1 && ((s)-1 >> 4000) == -1, \
         \"\");\n_Static_assert((u)-1 * (u)-1 == 1 && (u)0 - 1 == (u)-1, \"\");",
    );
    // GCC's limit: 1024 limbs.
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(
            kinds(
                "typedef unsigned _BitInt(65535) u;\n_Static_assert(((u)1 << 65534) / ((u)1 << \
                 32767) == (u)1 << 32767, \"\");\n_Static_assert((u)-1 % 3 == 1 && (u)-1 / 3 * 3 \
                 + 1 == (u)-1, \"\");\n_Static_assert((_BitInt(65535))((u)1 << 65534) < 0, \
                 \"\");\n_Static_assert(-(_BitInt(65535))((u)1 << 65533) * 2 < 0, \"\");",
                identity,
                crate::target::Target::LinuxGnu
            ),
            [],
            "{identity:?}"
        );
    }
}

#[test]
fn wide_values_convert_to_and_from_floating_types_exactly() {
    // Negated floating constants and floating comparisons are GNU folding
    // extensions in an integer constant expression (C99 §6.6p6).
    let source =
        "typedef unsigned _BitInt(2000) u;\ntypedef _BitInt(16500) s;\n_Static_assert((u)0x1p1023 \
         == (u)1 << 1023, \"\");\n_Static_assert((u)0x1.fffffffffffffp1023 == ((u)1 << 1024) - \
         ((u)1 << 971), \"\");\n_Static_assert((_BitInt(2000))-0x1.0000000000001p200 == \
         -(((_BitInt(2000))1 << 200) + ((_BitInt(2000))1 << 148)), \
         \"\");\n_Static_assert((s)0x1p16383L == (s)1 << 16383, \
         \"\");\n_Static_assert((s)-0x1.8p16000L == -((s)3 << 15999), \
         \"\");\n_Static_assert((_BitInt(300))-0x1.0000000000000000000000000001p200Q == \
         -(((_BitInt(300))1 << 200) + ((_BitInt(300))1 << 88)), \
         \"\");\n_Static_assert((_BitInt(200))-1.5 == -1 && (unsigned _BitInt(200))0.9 == 0, \
         \"\");\n_Static_assert((double)((u)1 << 1023) == 0x1p1023 && (double)(((u)1 << 1024) - \
         ((u)1 << 970)) > 0x1.fffffffffffffp1023, \"\");\n_Static_assert((float)(((u)1 << 100) + \
         ((u)1 << 76)) == 0x1p100f && (float)(((u)1 << 100) + ((u)1 << 76) + 1) > 0x1p100f, \
         \"\");\n_Static_assert((long double)-((s)1 << 16000) == -0x1p16000L, \"\");";
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        assert_eq!(gnu_kinds(source, identity), [], "{identity:?}");
    }
    // Out of range is undefined, so no constant, as for `(int)1e10`.
    for source in [
        "enum { A = (int)(_BitInt(1024))0x1p1023 };",
        "enum { B = (int)(unsigned _BitInt(1023))0x1p1023 };",
        "enum { C = (int)(_BitInt(200))-0x1.0000000000001p199 };",
    ] {
        assert_eq!(
            gnu_kinds(source, CompilerIdentity::Gcc),
            [SemanticErrorKind::InvalidConstant],
            "{source}"
        );
    }
    assert_eq!(
        gnu_kinds(
            "enum { C = (int)((_BitInt(200))-0x1p199 < 0) };",
            CompilerIdentity::Gcc
        ),
        []
    );
}

#[test]
fn overflow_builtins_check_wide_results_exactly() {
    // Clang has no `__builtin_*_overflow_p`.
    let source = "typedef unsigned _BitInt(300) u;\n_Static_assert(!__builtin_add_overflow_p((u)1 \
                  << 298, (u)1 << 298, (u)0), \
                  \"\");\n_Static_assert(__builtin_add_overflow_p((u)1 << 299, (u)1 << 299, \
                  (u)0), \"\");\n_Static_assert(__builtin_mul_overflow_p((u)1 << 150, (u)1 << \
                  150, (u)0), \"\");\n_Static_assert(!__builtin_mul_overflow_p((u)1 << 150, (u)1 \
                  << 149, (u)0), \"\");\n_Static_assert(__builtin_sub_overflow_p((u)0, (u)1, \
                  (u)0), \"\");\n_Static_assert(!__builtin_sub_overflow_p((u)0, (u)1, \
                  (_BitInt(300))0), \"\");\n_Static_assert(__builtin_mul_overflow_p((u)1 << 150, \
                  (u)1 << 149, (_BitInt(300))0), \
                  \"\");\n_Static_assert(__builtin_add_overflow_p((u)1 << 200, 0, 0LL), \"\");";
    assert_eq!(
        kinds(
            source,
            CompilerIdentity::Gcc,
            crate::target::Target::LinuxGnu
        ),
        []
    );
}

#[test]
fn wide_case_labels_compare_exactly() {
    // The labels share their low 128 bits: neither is a duplicate of the
    // other, and the ranges order by the whole value.
    let switch = |labels: &str| {
        format!(
            "typedef unsigned _BitInt(200) u;\nint f(u x) {{ switch (x) {{ {labels} return 1; }} \
             return 0; }}"
        )
    };
    for (labels, expected) in [
        (
            "case 1: case ((u)1 << 150) + 1: case ((u)1 << 160) ... ((u)1 << 161):",
            &[][..],
        ),
        (
            "case ((u)1 << 150) + 1: case ((u)1 << 150) + 1:",
            &[SemanticErrorKind::DuplicateCase],
        ),
        (
            "case ((u)1 << 160) ... ((u)1 << 161): case ((u)1 << 160) + 5:",
            &[SemanticErrorKind::DuplicateCase],
        ),
        (
            "case ((u)1 << 161) ... ((u)1 << 160):",
            &[SemanticErrorKind::EmptyCaseRange],
        ),
    ] {
        assert_eq!(
            gnu_kinds(&switch(labels), CompilerIdentity::Gcc),
            expected,
            "{labels}"
        );
    }
}

#[test]
fn folded_wide_values_reach_lowering_as_limbs() {
    with_configuration(
        "static const unsigned _BitInt(200) k = ((unsigned _BitInt(200))1 << 150) - 3;\nstatic \
         const _BitInt(300) m = -((_BitInt(300))1 << 200) + 5;\nstatic const _BitInt(300) n = -7;",
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow),
        |context, sema| {
            let value = |name: &str| {
                sema.bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .and_then(|b| b.value)
                    .unwrap()
            };
            let mut limbs = [0_u64; 5];
            sema.integer_limbs(value("k"), &mut limbs);
            assert_eq!(limbs, [u64::MAX - 2, u64::MAX, (1 << 22) - 1, 0, 0]);
            sema.integer_limbs(value("m"), &mut limbs);
            assert_eq!(limbs, [5, 0, 0, 0xFFFF_FFFF_FFFF_FF00, u64::MAX]);
            sema.integer_limbs(value("n"), &mut limbs);
            assert_eq!(
                limbs,
                [-7_i64 as u64, u64::MAX, u64::MAX, u64::MAX, u64::MAX]
            );
            assert_eq!(
                format!("{}", sema.integer_display(value("k"))),
                "1427247692705959881058285969449495136382746621"
            );
            assert_eq!(
                format!("{}", sema.integer_display(value("m"))),
                "-1606938044258990275541962092341162602522202993782792835301371"
            );
        },
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
