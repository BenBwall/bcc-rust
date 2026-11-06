//! `__builtin_clear_padding` through the optimizer and the interpreter: objects
//! filled with ones and cleared, compared with the bytes GCC 14.2 and Clang
//! 23.1 leave. The executable arms are in
//! `tests/integration/code_generation_cli.rs`. GNU extension: GCC manual,
//! "Other Builtins".

use super::*;
use crate::configuration::{
    CStandard,
    CompilerIdentity,
    ExtensionPolicy,
};

fn status(
    source: &str,
    target: Target,
    identity: CompilerIdentity,
    optimize: bool,
) -> (i32, String) {
    with_configured_module(
        source,
        target,
        optimize,
        Profile::PostAbi,
        CompilerConfiguration::new(CStandard::C23, ExtensionPolicy::Allow)
            .with_target(target)
            .with_gnu_extensions(true)
            .with_compiler_identity(identity),
        interpret,
    )
}

#[test]
fn the_shared_program_clears_what_gcc_and_clang_clear() {
    let source = include_str!("../../../../tests/fixtures/programs/builtin_clear_padding.c");
    for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
        for optimize in [false, true] {
            let (status, output) = status(source, Target::LinuxGnu, identity, optimize);
            assert_eq!(status, 0, "{identity:?} optimize={optimize}: {output}");
            assert_eq!(output, "failures: 0\n", "{identity:?} optimize={optimize}");
        }
    }
}

/// `long double` is binary64 for Microsoft's targets and has no padding, and
/// records take Microsoft's layout, which Clang 23.1 clears the same way
/// (compared through its IR for `x86_64-pc-windows-msvc`).
#[test]
fn value_bits_follow_the_microsoft_layout() {
    let source = "int memcmp(const void *, const void *, unsigned long long);
        struct b { int a : 3; long long b : 40; char c; };
        int main(void) {
            long double d;
            struct b s;
            static const unsigned char ones[8] = {255, 255, 255, 255, 255, 255, 255, 255};
            static const unsigned char expected[24] = {
                7, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0,
                255, 0, 0, 0, 0, 0, 0, 0};
            __builtin_memset(&d, 255, sizeof d);
            __builtin_memset(&s, 255, sizeof s);
            __builtin_clear_padding(&d);
            __builtin_clear_padding(&s);
            return sizeof d != 8 || sizeof s != 24 || memcmp(&d, ones, 8) != 0
                || memcmp(&s, expected, 24) != 0;
        }";
    for optimize in [false, true] {
        assert_eq!(
            status(
                source,
                Target::WindowsMsvc,
                CompilerIdentity::Clang,
                optimize
            )
            .0,
            0,
            "optimize={optimize}"
        );
    }
}
