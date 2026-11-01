//! `u8`, `u` and `U` string literals through lowering: static and automatic
//! arrays completed from them (#58), pointers to them, and their code units,
//! in every stage of the interpreter and natively on the host.

use super::*;
use crate::configuration::{
    CStandard,
    ExtensionPolicy,
};

/// Lowers `SOURCE` as C11 (encoded literals are a C11 feature) for `target`.
fn with_c11<R>(
    target: Target,
    optimize: bool,
    profile: Profile,
    then: impl FnOnce(&Module<'_>, Target) -> R,
) -> R {
    with_configured_module(
        SOURCE,
        target,
        optimize,
        profile,
        CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow).with_target(target),
        then,
    )
}

const SOURCE: &str = r#"
typedef unsigned short char16_t;
typedef unsigned int char32_t;
char16_t a[] = u"ab";
char32_t b[] = U"xyz";
char c[] = u8"hé";
char16_t d[] = u"\U0001F600";
const char32_t *p = U"pq";
struct S { char16_t s[4]; int n; } st = { u"hi", 7 };
int main(void) {
    char16_t la[] = u"local";
    char32_t lb[3] = U"ab";
    const char16_t *q = u"\x1234z";
    if (sizeof a != 6 || sizeof b != 16 || sizeof c != 4 || sizeof d != 6) return 1;
    if (a[0] != 'a' || a[1] != 'b' || a[2] != 0) return 2;
    if (b[2] != 'z' || b[3] != 0) return 3;
    if ((unsigned char)c[1] != 0xc3 || (unsigned char)c[2] != 0xa9 || c[3] != 0) return 4;
    if (d[0] != 0xd83d || d[1] != 0xde00 || d[2] != 0) return 5;
    if (p[1] != 'q' || p[2] != 0) return 6;
    if (st.s[1] != 'i' || st.s[2] != 0 || st.s[3] != 0 || st.n != 7) return 7;
    if (sizeof la != 12 || la[4] != 'l' || la[5] != 0) return 8;
    if (lb[1] != 'b' || lb[2] != 0) return 9;
    if (q[0] != 0x1234 || q[1] != 'z' || q[2] != 0) return 10;
    if (sizeof(u'x') != 2 || sizeof(U'x') != 4 || U'\xffffffff' < 0) return 11;
    return 0;
}
"#;

#[test]
fn encoded_strings_initialize_and_address_arrays_in_all_stages() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            let actual = with_c11(target, optimize, profile, interpret);
            assert_eq!(actual.0, 0, "{target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn encoded_strings_match_native_execution() {
    let Some(target) = host() else { return };
    let _toolchain = Toolchain::bundled(target).expect("the bundled LLVM toolchain");
    let directory = TempDir::new("encoded-strings");
    for arm in ARMS {
        let executable = directory.join(format!("encoded-{}.exe", arm.name));
        with_c11(target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &executable,
                LlvmOptions {
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                    code: crate::target::CodeOptions::default(),
                    fast_math_runtime: false,
                },
            )
            .unwrap();
        });
        assert_eq!(run_native(&executable).0, 0, "{}", arm.name);
    }
}
