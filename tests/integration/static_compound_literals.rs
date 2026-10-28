//! Compound literals in static initializers on every back end.
//! `tests/fixtures/programs/static_compound_literals.c` built by the bundled
//! Clang is the reference for the Clang identity, whose evaluator also reads
//! a scalar literal's value in any constant expression, and built by the
//! host GCC for the GCC identity, which accepts a literal only as the whole
//! initializer, inside the wrappers its folder removes.

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

    /// Each identity's accepted initializers give the values its compiler's
    /// build prints, interpreted before and after optimization and built
    /// through LLVM and the native back end.
    #[test]
    fn static_compound_literals_match_the_claimed_compiler() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs/static_compound_literals.c")
            .to_str()
            .unwrap()
            .to_owned();
        let mut references = vec![("--compiler-identity=clang", CLANG)];
        if Command::new("gcc").arg("--version").output().is_ok() {
            references.push(("--compiler-identity=gcc", "gcc"));
        } else {
            eprintln!("skipping the GCC identity: no host GCC");
        }
        for (identity, compiler) in references {
            let expected = reference(compiler, &source);
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
                assert_eq!(text(&output.stdout), expected, "{identity} {flags:?}");
            }
            for flags in [
                &["-O0"][..],
                &["-O2"],
                &["--backend=native"],
                &["--backend=native", "--opt=bcc"],
            ] {
                let executable = temporary("literals.exe");
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
                assert_eq!(text(&run.stdout), expected, "{identity} {flags:?}");
                std::fs::remove_file(executable).unwrap();
            }
        }
    }

    /// The output of `source` built by `compiler`.
    fn reference(compiler: &str, source: &str) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args([source, "-std=gnu17", "-O0", "-w", "-o"])
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
            "bcc-literals-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
