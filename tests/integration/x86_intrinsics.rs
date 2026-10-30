//! bcc's own x86 intrinsic headers (`src/headers/mmintrin.h` through
//! `smmintrin.h`, plain C over GNU vectors) against the host GCC's, which
//! compile to the instructions. `x86_intrinsics_sse2.c` covers MMX, SSE and
//! SSE2; `x86_intrinsics_sse4.c` SSE3 through SSE4.2 and needs a processor
//! with SSE4.2 to run GCC's build; `x86_intrinsics_programs.c` is typical
//! SSE2 code. Each is built under both identities,
//! interpreted and compiled through LLVM and the native back end, and must
//! print GCC's output.

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

    #[test]
    fn sse2_intrinsics_match_gcc() {
        check("x86_intrinsics_sse2.c", &[]);
    }

    #[test]
    fn sse2_programs_match_gcc() {
        check("x86_intrinsics_programs.c", &[]);
    }

    #[test]
    fn sse4_intrinsics_match_gcc() {
        check("x86_intrinsics_sse4.c", &["-msse4.2"]);
    }

    /// Builds `name` with the host GCC (`gcc_flags` enable its instruction
    /// sets) and compares every bcc arm's output with that build's.
    fn check(name: &str, gcc_flags: &[&str]) {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = fixture(name);
        let Some(expected) = reference(&source, gcc_flags) else {
            eprintln!("note: no host GCC build of {name} runs here; skipping");
            return;
        };
        for identity in ["--compiler-identity=gcc", "--compiler-identity=clang"] {
            for flags in [
                &["--interpret", "--opt=none"][..],
                &["--interpret", "--opt=bcc"],
            ] {
                let output = bcc(&[&[identity, &source][..], flags].concat());
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{identity} {flags:?}: {}",
                    text(&output.stderr)
                );
                assert_lines(
                    &text(&output.stdout),
                    &expected,
                    &format!("{identity} {flags:?}"),
                );
            }
            for flags in [
                &["-O0"][..],
                &["-O2"],
                &["--backend=native"],
                &["--backend=native", "--opt=bcc"],
            ] {
                let executable = temporary("intrinsics.exe");
                let built = bcc(&[
                    &[identity, &source, "-o", executable.to_str().unwrap()][..],
                    flags,
                ]
                .concat());
                assert!(
                    built.status.success(),
                    "{identity} {flags:?}: {}",
                    text(&built.stderr)
                );
                let run = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(run.status.code(), Some(0), "{identity} {flags:?}");
                assert_lines(
                    &text(&run.stdout),
                    &expected,
                    &format!("{identity} {flags:?}"),
                );
                std::fs::remove_file(executable).unwrap();
            }
        }
    }

    /// The output of the host GCC's build, or `None` if there is no GCC or
    /// the processor cannot run the build.
    fn reference(source: &str, flags: &[&str]) -> Option<String> {
        let executable = temporary("reference.exe");
        let built = Command::new("gcc")
            .args(flags)
            .args([source, "-O0", "-w", "-o"])
            .arg(&executable)
            .arg("-lm")
            .output()
            .ok()?;
        if !built.status.success() {
            return None;
        }
        let run = run_native(&mut Command::new(&executable)).unwrap();
        std::fs::remove_file(executable).unwrap();
        (run.status.code() == Some(0)).then(|| text(&run.stdout))
    }

    /// Compares output with expectations line by line, reporting the first
    /// differences.
    fn assert_lines(output: &str, expected: &str, context: &str) {
        if output == expected {
            return;
        }
        let mismatches: Vec<String> = output
            .lines()
            .zip(expected.lines())
            .filter(|(line, want)| line != want)
            .take(20)
            .map(|(line, want)| format!("expected {want}\n     got {line}"))
            .collect();
        panic!(
            "{context}: {} lines against {} expected\n{}",
            output.lines().count(),
            expected.lines().count(),
            mismatches.join("\n")
        );
    }

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
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
            "bcc-x86-intrinsics-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
