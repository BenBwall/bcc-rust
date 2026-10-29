//! The type-generic bit builtins and `<stdbit.h>` on every back end.
//! `tests/fixtures/programs/stdbit.c` runs every builtin GCC and Clang
//! share and every `<stdbit.h>` function at every unsigned width, with
//! glibc's header on the host and with bcc's own in a freestanding build;
//! the bundled Clang's build is the reference for both identities, and the
//! host GCC's (15 or later) must agree with it. `stdbit.wasm32.expected` is
//! what wasi-sdk's Clang build prints for `wasm32-wasip1`, with bcc's
//! header and bcc's WASI `stdbit.c`. `stdbit_gcc.c` covers what only GCC
//! does (`stdbit_gcc.expected` is GCC 15's output) and `stdbit_clang.c`
//! what only Clang does, each under its identity.

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

    const CLANG: &str = "target/llvm/bin/clang";

    /// Every shared builtin and `<stdbit.h>` function prints what Clang's
    /// and GCC's builds print, under each identity, interpreted before and
    /// after optimization and built through LLVM and the native back end;
    /// and with bcc's own header in a freestanding build.
    #[test]
    fn stdbit_programs_match_clang_and_gcc() {
        if !linux_x86_64() || !compiles(CLANG, &fixture("stdbit.c")) {
            eprintln!("skipping: no x86-64 Linux C library with <stdbit.h>");
            return;
        }
        let source = fixture("stdbit.c");
        let expected = reference(CLANG, &source);
        if compiles("gcc", &source) {
            assert_eq!(reference("gcc", &source), expected, "GCC agrees with Clang");
        } else {
            eprintln!("note: no host GCC 15; the GCC identity uses Clang's output");
        }
        for identity in ["--compiler-identity=clang", "--compiler-identity=gcc"] {
            check(&source, identity, &[], &expected);
        }
        check(
            &source,
            "--compiler-identity=clang",
            &["-ffreestanding"],
            &expected,
        );
    }

    /// The same program built by bcc's WebAssembly back end for
    /// `wasm32-wasip1`, unoptimized and with both optimizers, prints what
    /// wasi-sdk's Clang build prints, under Node's WASI; without `node`
    /// on the path this is skipped.
    #[test]
    fn stdbit_programs_match_clang_on_wasm32() {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let source = fixture("stdbit.c");
        let expected = std::fs::read_to_string(fixture("stdbit.wasm32.expected")).unwrap();
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for flags in [&["-O0"][..], &["-O2"], &["--opt=bcc"]] {
            let module = temporary("stdbit.wasm");
            let built = bcc(&[
                &[
                    "-std=c23",
                    "--target=wasm32-wasip1",
                    &source,
                    "-o",
                    module.to_str().unwrap(),
                ][..],
                flags,
            ]
            .concat());
            assert!(
                built.status.success(),
                "wasm32 {flags:?}: {}",
                text(&built.stderr)
            );
            let run = run_native(
                Command::new("node")
                    .arg("--no-warnings")
                    .arg(&runner)
                    .arg(&module),
            )
            .unwrap();
            assert_eq!(
                run.status.code(),
                Some(0),
                "wasm32 {flags:?}: {}",
                text(&run.stderr)
            );
            assert_lines(&text(&run.stdout), &expected, &format!("wasm32 {flags:?}"));
            std::fs::remove_file(module).unwrap();
        }
    }

    /// GCC's signed and parity builtins, bit-field operands at their
    /// width, zero counts folded to the width and wide rotations by
    /// run-time counts print what GCC 15 printed, under the GCC identity.
    #[test]
    fn gcc_bit_builtins_match_gcc() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("stdbit_gcc.c");
        let expected = std::fs::read_to_string(fixture("stdbit_gcc.expected")).unwrap();
        if compiles("gcc", &source) {
            assert_eq!(
                reference("gcc", &source),
                expected,
                "GCC prints the pinned output"
            );
        }
        check(&source, "--compiler-identity=gcc", &[], &expected);
    }

    /// Clang's `bool` and enumerated operands, bit-fields at their declared
    /// width and negative rotation counts print what Clang's build prints,
    /// under the Clang identity.
    #[test]
    fn clang_bit_builtins_match_clang() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("stdbit_clang.c");
        let expected = reference(CLANG, &source);
        check(&source, "--compiler-identity=clang", &[], &expected);
    }

    /// Runs `source` under `identity` with `extra` flags, interpreted
    /// before and after optimization and built through LLVM and the native
    /// back end, and compares each output with `expected`.
    fn check(source: &str, identity: &str, extra: &[&str], expected: &str) {
        for flags in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
        ] {
            let output = bcc(&[&[identity, "-std=c23", source][..], extra, flags].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{identity} {extra:?} {flags:?}: {}",
                text(&output.stderr)
            );
            assert_lines(
                &text(&output.stdout),
                expected,
                &format!("{identity} {extra:?} {flags:?}"),
            );
        }
        for flags in [
            &["-O0"][..],
            &["-O2"],
            &["--backend=native"],
            &["--backend=native", "--opt=bcc"],
        ] {
            let executable = temporary("stdbit.exe");
            let built = bcc(&[
                &[
                    identity,
                    "-std=c23",
                    source,
                    "-o",
                    executable.to_str().unwrap(),
                ][..],
                extra,
                flags,
            ]
            .concat());
            assert!(
                built.status.success(),
                "{identity} {extra:?} {flags:?}: {}",
                text(&built.stderr)
            );
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{identity} {extra:?} {flags:?}");
            assert_lines(
                &text(&run.stdout),
                expected,
                &format!("{identity} {extra:?} {flags:?}"),
            );
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// Compares line by line, naming the first line that differs.
    fn assert_lines(output: &str, expected: &str, context: &str) {
        for (index, (line, wanted)) in output.lines().zip(expected.lines()).enumerate() {
            assert_eq!(line, wanted, "{context}: line {}", index + 1);
        }
        assert_eq!(
            output.lines().count(),
            expected.lines().count(),
            "{context}: line count"
        );
    }

    fn linux_x86_64() -> bool {
        cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        ))
    }

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Whether `compiler` builds `source`: a GCC older than 15 lacks
    /// `__builtin_stdc_rotate_left`, and a C library older than glibc 2.39
    /// lacks `<stdbit.h>`.
    fn compiles(compiler: &str, source: &str) -> bool {
        let object = temporary("probe.o");
        let built = Command::new(compiler)
            .args([
                "-std=c23",
                "-w",
                source,
                "-c",
                "-o",
                object.to_str().unwrap(),
            ])
            .output()
            .is_ok_and(|output| output.status.success());
        if object.exists() {
            std::fs::remove_file(object).unwrap();
        }
        built
    }

    /// The output of `source` built by `compiler`.
    fn reference(compiler: &str, source: &str) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args([source, "-std=c23", "-O0", "-w", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{compiler}: {}",
            text(&built.stderr)
        );
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(0), "{compiler}");
        std::fs::remove_file(executable).unwrap();
        text(&run.stdout)
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
            "bcc-stdbit-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
