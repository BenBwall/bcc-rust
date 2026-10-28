//! Compound assignment, increment and decrement of `_Atomic` arithmetic and
//! pointer objects on every back end (C11 §6.5.16.2p3).
//! `tests/fixtures/programs/atomic_compound.c` (floating objects) and
//! `atomic_half.c` (16-bit floating objects, built for each half type) print
//! what the compiler each identity claims prints, built by the bundled Clang
//! or the host GCC: the two keep a half's excess `float` precision in
//! different places. `atomic_compound_integers.c` (integer and pointer
//! objects) prints what GCC prints under every identity, because the bundled
//! Clang miscompiles it.

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
    const IDENTITIES: [(&str, &str); 2] = [
        ("--compiler-identity=clang", CLANG),
        ("--compiler-identity=gcc", "gcc"),
    ];

    /// `float`, `double`, x87 `long double` and `__float128` objects with
    /// operands of each floating and integer type, every operator, and the
    /// result's value and type.
    #[test]
    fn floating_objects_match_the_claimed_compiler() {
        for (identity, compiler) in IDENTITIES {
            if let Some(expected) = reference(compiler, "atomic_compound.c", &[]) {
                matches_on_every_back_end(identity, "atomic_compound.c", &[], &expected);
            }
        }
    }

    /// Integer objects with floating and other integer operands, `_Bool`,
    /// and pointer `+=` and `-=`, which the bundled Clang rejects.
    #[test]
    fn integer_objects_match_gcc_under_every_identity() {
        let Some(expected) = reference("gcc", "atomic_compound_integers.c", &[]) else {
            return;
        };
        for (identity, _) in IDENTITIES {
            matches_on_every_back_end(identity, "atomic_compound_integers.c", &[], &expected);
        }
    }

    /// `_Atomic _Float16`, `__bf16` and Clang's `__fp16` objects with each
    /// identity's excess precision, and integer objects with half operands.
    #[test]
    fn half_objects_match_the_claimed_compiler() {
        for half in ["-DHALF=_Float16", "-DHALF=__bf16", "-DHALF=__fp16"] {
            for (identity, compiler) in IDENTITIES {
                // GCC provides no `__fp16` on x86-64.
                if half == "-DHALF=__fp16" && compiler == "gcc" {
                    continue;
                }
                if let Some(expected) = reference(compiler, "atomic_half.c", &[half]) {
                    matches_on_every_back_end(identity, "atomic_half.c", &[half], &expected);
                }
            }
        }
    }

    fn matches_on_every_back_end(identity: &str, name: &str, defines: &[&str], expected: &str) {
        let source = fixture(name);
        for flags in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
        ] {
            let output = bcc(&[&[identity, &source][..], defines, flags].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{identity} {defines:?} {flags:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(
                text(&output.stdout),
                expected,
                "{identity} {defines:?} {flags:?}"
            );
        }
        for flags in [
            &["-O0"][..],
            &["-O2"],
            &["--backend=native"],
            &["--backend=native", "--opt=bcc"],
        ] {
            let executable = temporary("atomic.exe");
            let built = bcc(&[
                &[identity, &source, "-o", executable.to_str().unwrap()][..],
                defines,
                flags,
            ]
            .concat());
            assert!(
                built.status.success(),
                "{identity} {defines:?} {flags:?}: {}",
                text(&built.stderr)
            );
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(
                run.status.code(),
                Some(0),
                "{identity} {defines:?} {flags:?}"
            );
            assert_eq!(
                text(&run.stdout),
                expected,
                "{identity} {defines:?} {flags:?}"
            );
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// The output of fixture `name` built by `compiler` without contraction,
    /// which bcc does not perform, or `None` (skipping) off x86-64 Linux or
    /// when the host GCC cannot build it (GCC 13 and later provide `__bf16`).
    fn reference(compiler: &str, name: &str, defines: &[&str]) -> Option<String> {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return None;
        }
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args(defines)
            .arg(fixture(name))
            .args(["-std=gnu17", "-O0", "-ffp-contract=off", "-w", "-o"])
            .arg(&executable)
            .arg("-latomic")
            .output();
        match built {
            | Ok(output) if output.status.success() => {},
            | _ if compiler == "gcc" => {
                eprintln!("skipping the GCC reference: no host GCC builds {name} {defines:?}");
                return None;
            },
            | built => panic!("{compiler} {name}: {built:?}"),
        }
        let run = run_native(&mut Command::new(&executable)).unwrap();
        assert_eq!(run.status.code(), Some(0), "{compiler} {name}");
        std::fs::remove_file(executable).unwrap();
        Some(text(&run.stdout))
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
            "bcc-atomic-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
