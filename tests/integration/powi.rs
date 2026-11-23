//! `__builtin_powi`, `__builtin_powif` and `__builtin_powil` on every back
//! end. `tests/fixtures/powi/powi.c` raises bases at their edges to
//! exponents at theirs; `powi.x86_64.expected` is what GCC 14 and Clang 23
//! print for it on x86-64 Linux at `-O0` and `-O2`, which libgcc's
//! `__powidf2` and its relatives compute. `powil_binary128.c` does the same
//! for wasm32's binary128 `long double`, against libgcc's algorithm on
//! `__float128`.

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

    /// Every back end at `-O0` and `-O2`, and the interpreter, print the
    /// bits GCC and Clang print.
    #[test]
    fn powi_matches_gcc_and_clang_on_every_back_end() {
        if !linux_x86_64() {
            return;
        }
        let source = fixture("powi.c");
        let expected = std::fs::read_to_string(fixture("powi.x86_64.expected")).unwrap();
        for backend in ["llvm", "native"] {
            for level in ["-O0", "-O2"] {
                let executable = temporary("powi");
                let built = bcc(&[
                    &source,
                    &format!("--backend={backend}"),
                    level,
                    "-o",
                    executable.to_str().unwrap(),
                ]);
                assert!(
                    built.status.success(),
                    "{backend} {level}: {}",
                    text(&built.stderr)
                );
                let run = run_native(&mut Command::new(&executable)).unwrap();
                assert_eq!(run.status.code(), Some(0), "{backend} {level}");
                assert_eq!(text(&run.stdout), expected, "{backend} {level}");
                std::fs::remove_file(executable).unwrap();
            }
        }
        let interpreted = bcc(&["--interpret", &source]);
        assert_eq!(
            interpreted.status.code(),
            Some(0),
            "{}",
            text(&interpreted.stderr)
        );
        assert_eq!(text(&interpreted.stdout), expected, "interpreter");
    }

    /// The WebAssembly back end calls the bundled runtime's `__powitf2`,
    /// whose bits are libgcc's; without `node` on the path this is skipped.
    #[test]
    fn powil_matches_libgcc_on_wasm32() {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let source = fixture("powil_binary128.c");
        let expected = std::fs::read_to_string(fixture("powil_binary128.expected")).unwrap();
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for level in ["-O0", "-O2"] {
            let module = temporary("powil.wasm");
            let built = bcc(&[
                "--target=wasm32-wasip1",
                level,
                &source,
                "-o",
                module.to_str().unwrap(),
            ]);
            assert!(
                built.status.success(),
                "wasm32 {level}: {}",
                text(&built.stderr)
            );
            let run = run_native(
                Command::new("node")
                    .arg("--no-warnings")
                    .arg(&runner)
                    .arg(&module),
            )
            .unwrap();
            assert_eq!(run.status.code(), Some(0), "wasm32 {level}");
            assert_eq!(text(&run.stdout), expected, "wasm32 {level}");
            std::fs::remove_file(module).unwrap();
        }
    }

    /// Off Windows the call is the IR's `powi`, which the LLVM back end
    /// emits as `llvm.powi`. On Windows the call is the one each compiler's
    /// code makes: Clang's `llvm.powi` becomes the C library's `pow` of the
    /// converted exponent there, and GCC calls libgcc's `__powidf2`.
    #[test]
    fn powi_calls_what_each_compiler_calls_on_each_target() {
        let source = temporary("powi.c");
        std::fs::write(
            &source,
            "double d(double x, int n) { return __builtin_powi(x, n); }\nfloat f(float x, int n) \
             { return __builtin_powif(x, n); }\nlong double l(long double x, int n) { return \
             __builtin_powil(x, n); }\n",
        )
        .unwrap();
        let source = source.to_str().unwrap();
        let emit = |args: &[&str]| {
            let output = bcc(&[args, &[source]].concat());
            assert!(
                output.status.success(),
                "{args:?}: {}",
                text(&output.stderr)
            );
            text(&output.stdout)
        };
        let ir = emit(&["--emit=ir", "--target=x86_64-unknown-linux-gnu"]);
        for intrinsic in ["powi.f64", "powi.f32", "powi.f80"] {
            assert!(ir.contains(&format!("intrinsic {intrinsic} ")), "{ir}");
        }
        let llvm = emit(&["--emit=llvm", "--target=x86_64-unknown-linux-gnu"]);
        for overload in ["f64.i32", "f32.i32", "f80.i32"] {
            assert!(llvm.contains(&format!("@llvm.powi.{overload}(")), "{llvm}");
        }
        for target in ["x86_64-w64-windows-gnu", "x86_64-pc-windows-msvc"] {
            let target = format!("--target={target}");
            let clang = emit(&["--emit=ir", &target]);
            assert!(clang.contains("call @pow("), "{clang}");
            assert!(clang.contains("call @powf("), "{clang}");
            assert!(!clang.contains("powi"), "{clang}");
            let gcc = emit(&["--emit=ir", "--compiler-identity=gcc", &target]);
            assert!(gcc.contains("call @__powidf2("), "{gcc}");
            assert!(gcc.contains("call @__powisf2("), "{gcc}");
        }
    }

    /// An `__assume` operand that calls `__builtin_powi` compiles as in
    /// Clang, which reads the builtin as `const`; a builtin bcc refuses is
    /// named, also as an operand of an operator.
    #[test]
    fn assumptions_of_powers_compile_and_refused_builtins_are_named() {
        let source = temporary("assume.c");
        std::fs::write(
            &source,
            "int f(double d) { __assume(__builtin_powi(d, 2) > 0); return d > 0; }\n",
        )
        .unwrap();
        let object = temporary("assume.obj");
        let output = bcc(&[
            "--clang-cl-mode",
            "/c",
            source.to_str().unwrap(),
            &format!("/Fo{}", object.display()),
        ]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(text(&output.stderr), "");
        // Also as an operand of `?:`, `sizeof` and `_Alignof`.
        for body in [
            "int f(void) { return __builtin_stack_save() != 0; }\n",
            "int f(int x) { return x ? __builtin_stack_save() != 0 : 1; }\n",
            "void *f(int x) { return x ? __builtin_stack_save() : 0; }\n",
            "unsigned long f(void) { return sizeof(__builtin_stack_save()); }\n",
            "unsigned long f(void) { return _Alignof(__builtin_stack_save()); }\n",
        ] {
            let source = temporary("refused.c");
            std::fs::write(&source, body).unwrap();
            let output = bcc(&[
                "-c",
                source.to_str().unwrap(),
                "-o",
                temporary("refused.o").to_str().unwrap(),
            ]);
            assert_eq!(output.status.code(), Some(1), "{body}");
            assert!(
                text(&output.stderr)
                    .contains("error: not yet supported by lowering: `__builtin_stack_save`"),
                "{body}: {}",
                text(&output.stderr)
            );
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
            .join("tests/fixtures/powi")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// A fresh path in the system's temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "bcc-powi-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(name)
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
}
