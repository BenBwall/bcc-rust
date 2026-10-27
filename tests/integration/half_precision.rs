//! `_Float16`, `__bf16` and `_Complex _Float16` programs on every back end.
//! `tests/fixtures/programs/half_precision.c`, `bfloat16.c` and
//! `complex_half.c` built by the bundled Clang are the reference for the Clang
//! identity and built by the host GCC (12 or later; 13 for `__bf16`) for the
//! GCC identity: the two keep a half's excess `float` precision in different
//! places, and bcc follows the compiler it claims. `half_precision_abi.c` and
//! `complex_half_abi.c` link halves built by bcc's back ends, Clang and GCC in
//! every pairing, for each type.

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

    /// Each identity's `_Float16` excess precision, conversions, constants
    /// and classification print what its compiler's build prints,
    /// interpreted before and after optimization and built through LLVM and
    /// the native back end.
    #[test]
    fn half_precision_programs_match_the_claimed_compiler() {
        programs_match_the_claimed_compiler("half_precision.c");
    }

    /// The same for `__bf16`, whose conversions from integers differ
    /// between the compilers too.
    #[test]
    fn bfloat16_programs_match_the_claimed_compiler() {
        programs_match_the_claimed_compiler("bfloat16.c");
    }

    /// `_Complex _Float16` arithmetic in `float _Complex`, with each
    /// identity's excess.
    #[test]
    fn complex_half_programs_match_the_claimed_compiler() {
        programs_match_the_claimed_compiler("complex_half.c");
    }

    /// Clang's `__fp16`, which GCC does not provide on x86-64, so only the
    /// Clang identity has a reference.
    #[test]
    fn storage_half_programs_match_clang() {
        programs_match_the_claimed_compiler("storage_half.c");
    }

    fn programs_match_the_claimed_compiler(name: &str) {
        if !linux_x86_64() {
            return;
        }
        let source = fixture(name);
        let mut references = vec![("--compiler-identity=clang", CLANG)];
        if host_gcc_compiles(&source, &[]) {
            references.push(("--compiler-identity=gcc", "gcc"));
        } else {
            eprintln!("skipping the GCC identity: no host GCC builds {name}");
        }
        for (identity, compiler) in references {
            let expected = reference(compiler, &[&source]);
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
                let executable = temporary("half.exe");
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

    /// Scalars, structures, a union and variadic arguments pass between
    /// halves built by bcc (LLVM and native back ends), Clang and GCC in
    /// both directions, as the psABI passes them.
    #[test]
    fn half_precision_arguments_link_with_clang_and_gcc() {
        arguments_link_with_clang_and_gcc("half_precision_abi.c", &[]);
    }

    /// The same calls with `__bf16`, which the psABI passes as `_Float16`.
    #[test]
    fn bfloat16_arguments_link_with_clang_and_gcc() {
        arguments_link_with_clang_and_gcc("half_precision_abi.c", &["-DHALF=__bf16"]);
    }

    /// `_Complex _Float16` arguments, results, members and variadic
    /// arguments.
    #[test]
    fn complex_half_arguments_link_with_clang_and_gcc() {
        arguments_link_with_clang_and_gcc("complex_half_abi.c", &[]);
    }

    fn arguments_link_with_clang_and_gcc(name: &str, defines: &[&str]) {
        if !linux_x86_64() {
            return;
        }
        let source = fixture(name);
        let expected = reference(CLANG, &[&[&source[..]][..], defines].concat());
        let gcc = host_gcc_compiles(&source, defines);
        if gcc {
            assert_eq!(
                reference("gcc", &[&[&source[..]][..], defines].concat()),
                expected,
                "GCC agrees with Clang"
            );
        }
        let bcc_binary = env!("CARGO_BIN_EXE_bcc-rust");
        let llvm = [bcc_binary, "--backend=llvm"];
        let native = [bcc_binary, "--backend=native"];
        let mut compilers: Vec<&[&str]> = vec![&[CLANG, "-O1"], &llvm, &native];
        if gcc {
            compilers.push(&["gcc", "-O1"]);
        }
        let half = |compiler: &[&str], which: &str| {
            let object = temporary(&format!("{which}.o"));
            let built = Command::new(compiler[0])
                .args(&compiler[1..])
                .args(defines)
                .args([&source, which, "-c", "-o", object.to_str().unwrap()])
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{compiler:?}: {}",
                text(&built.stderr)
            );
            object
        };
        for callee in &compilers {
            let definitions = half(callee, "-DCALLEE_ONLY");
            for caller in &compilers {
                let checks = half(caller, "-DCALLER_ONLY");
                let executable = temporary("abi.exe");
                let linked = Command::new(CLANG)
                    .args([&definitions, &checks])
                    .arg("-o")
                    .arg(&executable)
                    .output()
                    .unwrap();
                assert!(linked.status.success(), "{}", text(&linked.stderr));
                let run = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(text(&run.stdout), expected, "{callee:?} {caller:?}");
                std::fs::remove_file(executable).unwrap();
                std::fs::remove_file(checks).unwrap();
            }
            std::fs::remove_file(definitions).unwrap();
        }
        for identity in ["--compiler-identity=clang", "--compiler-identity=gcc"] {
            let output = bcc(&[&[identity, "--interpret", &source][..], defines].concat());
            assert_eq!(text(&output.stdout), expected, "{identity}");
        }
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

    /// Whether the host GCC builds `source`: GCC 12 and later provide
    /// `_Float16` on x86-64, and GCC 13 and later `__bf16`.
    fn host_gcc_compiles(source: &str, defines: &[&str]) -> bool {
        let object = temporary("probe.o");
        let built = Command::new("gcc")
            .args(defines)
            .args([source, "-c", "-o", object.to_str().unwrap()])
            .output()
            .is_ok_and(|output| output.status.success());
        if object.exists() {
            std::fs::remove_file(object).unwrap();
        }
        built
    }

    /// The output of `source` built by `compiler` without contraction, which
    /// bcc does not perform.
    fn reference(compiler: &str, source: &[&str]) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args(source)
            .args(["-std=gnu17", "-O0", "-ffp-contract=off", "-o"])
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
            "bcc-half-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
