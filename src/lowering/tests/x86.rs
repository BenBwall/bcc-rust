//! x86 builtins: lane code for the generic ones, one named `intrinsic`
//! instruction for the others, and guarded blocks for masked memory.

use super::*;

/// Each builtin lowers to the instructions its lanes need.
#[test]
fn x86_builtins_expand_into_lanes_or_named_instructions() {
    let v2di = "typedef long long V2 __attribute__((vector_size(16)));";
    let v16hi = "typedef short V16 __attribute__((vector_size(32)));";
    let v8si = "typedef int V8 __attribute__((vector_size(32)));";
    for (source, wanted) in [
        (
            format!(
                "{v2di}__attribute__((target(\"pclmul\"))) V2 f(V2 a, V2 b) {{ return \
                 __builtin_ia32_pclmulqdq128(a, b, 0x11); }}"
            ),
            "intrinsic x86.pclmulqdq 17,",
        ),
        (
            format!(
                "{v2di}__attribute__((target(\"aes\"))) V2 f(V2 a, V2 b) {{ return \
                 __builtin_ia32_aesenc128(a, b); }}"
            ),
            "intrinsic x86.aesenc v",
        ),
        (
            format!(
                "{v16hi}__attribute__((target(\"avx2\"))) V16 f(V16 a, V16 b) {{ return \
                 __builtin_ia32_pmulhw256(a, b); }}"
            ),
            "ashr.i32",
        ),
        (
            format!(
                "{v8si}__attribute__((target(\"avx2\"))) V8 f(const V8 *p, V8 m) {{ return \
                 __builtin_ia32_maskloadd256(p, m); }}"
            ),
            "brif",
        ),
        (
            format!(
                "{v8si}__attribute__((target(\"avx2\"))) V8 f(V8 a) {{ return \
                 __builtin_ia32_pshufd256(a, 0x1b); }}"
            ),
            "load.i32",
        ),
        (
            "__attribute__((target(\"avx\"))) void f(void) { __builtin_ia32_vzeroupper(); }"
                .to_owned(),
            "intrinsic x86.vzeroupper",
        ),
        (
            "unsigned char f(unsigned a, unsigned b, unsigned *out) { return \
             __builtin_ia32_addcarryx_u32(1, a, b, out); }"
                .to_owned(),
            "zext.i64",
        ),
    ] {
        let text = functions(&lowered(&source));
        assert!(text.contains(wanted), "{source}\n{text}");
    }
}

/// The expanded builtins keep Clang's check of the calling function's
/// features.
#[test]
fn expanded_x86_builtins_need_their_features() {
    let errors = lower(
        "typedef long long V2 __attribute__((vector_size(16))); V2 f(V2 a, V2 b) { return \
         __builtin_ia32_pclmulqdq128(a, b, 0); }",
    )
    .expect_err("refused");
    assert_eq!(
        errors,
        ["a builtin needs a target feature the calling function lacks"]
    );
}
