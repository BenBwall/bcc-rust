//! Programs whose results depend on the claimed compiler, run on every back
//! end under the GCC and Clang identities. Each fixture built by the host
//! GCC is the reference for the GCC identity, and built by the bundled
//! Clang for the Clang identity; a fixture tests `__clang__` for the forms
//! only one compiler accepts.

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

    /// An array initialized by a compound literal of its type: static
    /// objects under both identities, automatic ones under Clang, elements
    /// and members under GCC.
    #[test]
    fn array_compound_literals_match_the_claimed_compiler() {
        matches_the_claimed_compiler("array_compound_literals.c");
    }

    /// Each identity's value of `fixture` matches its compiler's build,
    /// interpreted before and after optimization and built through LLVM
    /// and the native back end.
    fn matches_the_claimed_compiler(fixture: &str) {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs")
            .join(fixture)
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
                let output = bcc(&[&[identity, "-std=gnu17", &source][..], flags].concat());
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{fixture} {identity} {flags:?}: {}",
                    text(&output.stderr)
                );
                assert_eq!(
                    text(&output.stdout),
                    expected,
                    "{fixture} {identity} {flags:?}"
                );
            }
            for flags in [
                &["-O0"][..],
                &["-O2"],
                &["--backend=native"],
                &["--backend=native", "--opt=bcc"],
            ] {
                let executable = temporary("program.exe");
                let built = bcc(&[
                    &[
                        identity,
                        "-std=gnu17",
                        &source,
                        "-o",
                        executable.to_str().unwrap(),
                    ][..],
                    flags,
                ]
                .concat());
                assert!(
                    built.status.success(),
                    "{fixture} {identity} {flags:?}: {}",
                    text(&built.stderr)
                );
                let run = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(run.status.code(), Some(0), "{fixture} {identity} {flags:?}");
                assert_eq!(
                    text(&run.stdout),
                    expected,
                    "{fixture} {identity} {flags:?}"
                );
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
            "bcc-identity-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
