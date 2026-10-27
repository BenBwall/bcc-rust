//! Binary128 (`__float128` and `_Float128`) programs on every back end:
//! `tests/fixtures/programs/binary128.c` built by the bundled Clang against
//! libgcc's soft-float routines and glibc is the reference, and the
//! interpreter, LLVM and the native back end must print the same bits.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use crate::native::run_native;

    /// Arithmetic, comparisons, conversions, calls, variadic arguments,
    /// folded initializers and glibc's `_Float128` functions print what the
    /// Clang-built program prints, interpreted before and after optimization
    /// under both compiler identities, and built through LLVM and the
    /// native back end.
    #[test]
    fn binary128_programs_match_libgcc_on_every_back_end() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/programs/binary128.c");
        let source = source.to_str().unwrap();
        let reference = temporary("reference.exe");
        let built = Command::new("target/llvm/bin/clang")
            .args([source, "-std=c99", "-O0", "-fno-builtin", "-lm", "-o"])
            .arg(&reference)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        let expected = run_native(&mut Command::new(&reference)).unwrap();
        assert_eq!(expected.status.code(), Some(0));
        let expected = text(&expected.stdout);
        std::fs::remove_file(reference).unwrap();
        for flags in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
            &["--interpret", "--compiler-identity=gcc"],
        ] {
            let output = bcc(&[&[source][..], flags].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "{flags:?}");
        }
        for flags in [
            &["-O0"][..],
            &["-O2"],
            &["--backend=native"],
            &["--backend=native", "--opt=bcc"],
        ] {
            let executable = temporary("binary128.exe");
            let built = bcc(&[
                &[source, "-lm", "-o", executable.to_str().unwrap()][..],
                flags,
            ]
            .concat());
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{flags:?}");
            assert_eq!(text(&run.stdout), expected, "{flags:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// Under the GCC identity `_Float128` is a keyword with `f128`
    /// constants, glibc's headers declare its functions, and `<float.h>`
    /// gives GCC's `FLT128_*` names when TS 18661-3 is requested; the
    /// limits print as GCC 15 prints them.
    #[test]
    fn gcc_identity_float128_names_and_limits_match_gcc() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = "#define __STDC_WANT_IEC_60559_TYPES_EXT__ 1
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
static void show(const char *label, _Float128 value) {
    char text[64];
    strfromf128(text, sizeof text, \"%.36g\", value);
    printf(\"%s %s\\n\", label, text);
}
static const _Float128 third = 1.0f128 / 3;
int main(void) {
    show(\"max\", FLT128_MAX);
    show(\"min\", FLT128_MIN);
    show(\"true_min\", FLT128_TRUE_MIN);
    show(\"epsilon\", FLT128_EPSILON);
    show(\"third\", third);
    show(\"sqrt\", sqrtf128(2.0f128));
    show(\"strto\", strtof128(\"0.1\", NULL));
    printf(\"%d %d %d %d %d\\n\", FLT128_MANT_DIG, FLT128_DIG, FLT128_MIN_EXP,
           FLT128_MAX_10_EXP, FLT128_DECIMAL_DIG);
    printf(\"%d\\n\", (int)sizeof(_Float128) + _Generic(1.0f128, __float128: 100, default: 0));
    return 0;
}
";
        let expected = "max 1.18973149535723176508575932662800702e+4932
min 3.3621031431120935062626778173217526e-4932
true_min 6.47517511943802511092443895822764655e-4966
epsilon 1.92592994438723585305597794258492732e-34
third 0.333333333333333333333333333333333317
sqrt 1.41421356237309504880168872420969798
strto 0.100000000000000000000000000000000005
113 33 -16381 4932 36
116
";
        for flags in [
            &["--interpret"][..],
            &["-O0", "-o"],
            &["--backend=native", "-o"],
        ] {
            let executable = temporary("gcc-identity.exe");
            let mut arguments = vec![
                "--compiler-identity=gcc",
                "-std=gnu17",
                "--input",
                source,
                "-lm",
            ];
            arguments.extend_from_slice(flags);
            let output = if flags.contains(&"-o") {
                arguments.push(executable.to_str().unwrap());
                let built = bcc(&arguments);
                assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
                let run = run_native(&mut Command::new(&executable)).unwrap();
                std::fs::remove_file(&executable).unwrap();
                run
            } else {
                bcc(&arguments)
            };
            assert_eq!(
                output.status.code(),
                Some(0),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), expected, "{flags:?}");
        }
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE"),
        )
        .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "bcc-binary128-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
