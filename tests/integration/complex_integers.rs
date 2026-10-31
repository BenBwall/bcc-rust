//! GNU complex integer types on every back end. `complex_integers.c` runs
//! under both identities in `identity_programs.rs`, each against its own
//! compiler's build, since GCC and Clang divide complex integers by
//! different formulas. `complex_integers_abi.c` links halves built by bcc's
//! back ends, Clang and GCC in every pairing, and `complex_integers.c`
//! built for `wasm32-wasip1` prints what wasi-sdk's Clang build printed
//! (`complex_integers.wasm32.expected`).

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

    /// Complex integer arguments, results, structure members and variadic
    /// arguments pass between halves built by bcc (LLVM and native back
    /// ends), Clang and GCC in both directions, as the psABI passes a
    /// structure of the two parts; the interpreter runs the whole program.
    #[test]
    fn complex_integer_arguments_link_with_clang_and_gcc() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("complex_integers_abi.c");
        let expected = reference(CLANG, &source);
        let gcc = Command::new("gcc").arg("--version").output().is_ok();
        if gcc {
            assert_eq!(reference("gcc", &source), expected, "GCC agrees with Clang");
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
                .args(["-w", &source, which, "-c", "-o", object.to_str().unwrap()])
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
            let output = bcc(&[identity, "--interpret", &source]);
            assert_eq!(text(&output.stdout), expected, "{identity}");
        }
    }

    /// The program built by bcc's WebAssembly back end for `wasm32-wasip1`,
    /// unoptimized and with both optimizers, prints what wasi-sdk's Clang
    /// build prints, under Node's WASI; without `node` on the path this is
    /// skipped.
    #[test]
    fn complex_integers_match_clang_on_wasm32() {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let source = fixture("complex_integers.c");
        let expected =
            std::fs::read_to_string(fixture("complex_integers.wasm32.expected")).unwrap();
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for flags in [&["-O0"][..], &["-O2"], &["--opt=bcc"]] {
            let module = temporary("complex.wasm");
            let built = bcc(&[
                &[
                    "-std=gnu17",
                    "-w",
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
            assert_eq!(run.status.code(), Some(0), "wasm32 {flags:?}");
            assert_eq!(text(&run.stdout), expected, "wasm32 {flags:?}");
            std::fs::remove_file(module).unwrap();
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
            "bcc-complex-integer-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
