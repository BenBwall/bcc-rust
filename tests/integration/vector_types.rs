//! GNU vector types on every back end. `tests/fixtures/programs/vector_types.c`
//! runs every vector operator, conversion and builtin; the host GCC's build
//! is the reference for the GCC identity and the bundled Clang's for the
//! Clang identity, since the two accept different operators (GCC's `++` and
//! `&v[i]`, Clang's lax conversions). `vector_types.wasm32.expected` is what
//! wasi-sdk's Clang build prints for `wasm32-wasip1`. `vector_types_abi.c`
//! links halves built by bcc's back ends and by the claimed compiler in
//! every pairing.

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

    /// The identities, each with the compiler whose build is its reference.
    fn identities() -> Vec<(&'static str, &'static str)> {
        let mut identities = vec![("--compiler-identity=clang", CLANG)];
        if host_gcc() {
            identities.push(("--compiler-identity=gcc", "gcc"));
        } else {
            eprintln!("note: no host GCC; skipping the GCC identity");
        }
        identities
    }

    /// Every operator, conversion and builtin prints what the claimed
    /// compiler's build prints, interpreted before and after optimization
    /// and built through LLVM and the native back end.
    #[test]
    fn vector_programs_match_clang_and_gcc() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("vector_types.c");
        for (identity, compiler) in identities() {
            let expected = reference(compiler, &source, &[]);
            for flags in [
                &["--interpret", "--opt=none"][..],
                &["--interpret", "--opt=bcc"],
            ] {
                let output = bcc(&[&[identity, "-std=gnu17", &source][..], flags].concat());
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
                let executable = temporary("vector_types.exe");
                let built = bcc(&[
                    &[
                        identity,
                        "-std=gnu17",
                        &source,
                        "-o",
                        executable.to_str().unwrap(),
                        "-latomic",
                    ][..],
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

    /// The same program built by bcc's WebAssembly back end for
    /// `wasm32-wasip1`, unoptimized and with both optimizers, prints what
    /// wasi-sdk's Clang build prints, under Node's WASI; without `node`
    /// on the path this is skipped.
    #[test]
    fn vector_programs_match_clang_on_wasm32() {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let source = fixture("vector_types.c");
        let expected = std::fs::read_to_string(fixture("vector_types.wasm32.expected")).unwrap();
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for flags in [&["-O0"][..], &["-O2"], &["--opt=bcc"]] {
            let module = temporary("vector_types.wasm");
            let built = bcc(&[
                &[
                    "-std=gnu17",
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

    /// Vector arguments and results cross between halves built by the
    /// claimed compiler and by bcc's LLVM and native back ends, in every
    /// pairing, under each identity's System V convention; the whole
    /// program also runs under bcc's interpreter.
    #[test]
    fn vector_arguments_link_with_clang_and_gcc() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("vector_types_abi.c");
        let bcc_binary = env!("CARGO_BIN_EXE_bcc-rust");
        for (identity, compiler) in identities() {
            let expected = reference(compiler, &source, &[]);
            let reference_compiler = [compiler, "-O1", "-w", "-Wno-psabi"];
            let llvm = [bcc_binary, identity, "--backend=llvm"];
            let llvm_optimized = [bcc_binary, identity, "--backend=llvm", "-O2"];
            let native = [bcc_binary, identity, "--backend=native"];
            let native_optimized = [bcc_binary, identity, "--backend=native", "--opt=bcc"];
            let compilers: [&[&str]; 5] = [
                &reference_compiler,
                &llvm,
                &llvm_optimized,
                &native,
                &native_optimized,
            ];
            let half = |compiler: &[&str], which: &str| {
                let object = temporary(&format!("{which}.o"));
                let built = Command::new(compiler[0])
                    .args(&compiler[1..])
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
            for callee in compilers {
                let definitions = half(callee, "-DCALLEE_ONLY");
                for caller in compilers {
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
            let output = bcc(&[identity, "--interpret", &source]);
            assert_eq!(text(&output.stdout), expected, "{identity}");
        }
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

    /// Whether a host GCC runs.
    fn host_gcc() -> bool {
        Command::new("gcc")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    /// The output of `source` built by `compiler`, with libatomic for the
    /// 16-byte atomic vector.
    fn reference(compiler: &str, source: &str, extra: &[&str]) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args(extra)
            .args([source, "-std=gnu17", "-O0", "-w", "-Wno-psabi", "-o"])
            .arg(&executable)
            .arg("-latomic")
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
            "bcc-vector-types-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
