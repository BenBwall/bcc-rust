//! GNU nested functions under the GCC identity, on every back end: the IR
//! interpreter (unoptimized and with bcc's optimizer), LLVM at `-O0` and
//! `-O2`, and the native back end. `tests/fixtures/programs/nested_functions.c`
//! prints what GCC prints. The Clang identity rejects them as Clang does,
//! and what bcc does not implement yet is refused, never miscompiled.

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

    const GCC: &str = "--compiler-identity=gcc";

    const EXPECTED: &str = "cleanup 42\n2 33 8\n7 10 8 16 120\n8 42\n42 122 27 15001\nexit 42\n";

    #[test]
    fn nested_functions_match_gcc_on_every_back_end() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs/nested_functions.c");
        let source = source.to_str().unwrap();
        for mode in [
            &["--interpret", "--opt=none"][..],
            &["--interpret", "--opt=bcc"],
        ] {
            let output = bcc(&[&[GCC, source][..], mode].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{mode:?}: {}",
                text(&output.stderr)
            );
            assert_eq!(text(&output.stdout), EXPECTED, "{mode:?}");
        }
        for mode in [&["-O0"][..], &["-O2"], &["--backend=native"]] {
            let executable = temporary("nested.exe");
            let built =
                bcc(&[&[GCC, source, "-o", executable.to_str().unwrap()][..], mode].concat());
            assert!(built.status.success(), "{mode:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{mode:?}");
            assert_eq!(text(&run.stdout), EXPECTED, "{mode:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// A module whose trampolines run on the stack marks it executable, as
    /// GCC's does; one without keeps it non-executable.
    #[test]
    fn trampolines_mark_the_stack_executable() {
        let source = "int apply(int (*f)(void));\nint main(void) { int n = 3; int get(void) { \
                      return n; } return apply(get) != 3; }\n";
        let plain =
            "int main(void) { int n = 3; int get(void) { return n; } return get() != 3; }\n";
        for (program, executable) in [(source, true), (plain, false)] {
            for backend in ["--backend=llvm", "--backend=native"] {
                let assembly = check(&[GCC, backend, "--emit=asm"], program);
                assert!(assembly.status.success(), "{}", text(&assembly.stderr));
                let marked = text(&assembly.stdout)
                    .lines()
                    .any(|line| line.contains(".note.GNU-stack") && line.contains("\"x\""));
                assert_eq!(marked, executable, "{backend}: {program}");
            }
        }
    }

    /// Only GCC has nested functions: Clang, TCC and `cl` each reject a
    /// definition in a block with their own message.
    #[test]
    fn other_identities_reject_nested_definitions() {
        for (identity, message) in [
            (
                "--compiler-identity=clang",
                "function definition is not allowed here",
            ),
            (
                "--compiler-identity=clang-cl",
                "function definition is not allowed here",
            ),
            ("--compiler-identity=tcc", "cannot use local functions"),
            (
                "--compiler-identity=msvc",
                "local function definitions are illegal",
            ),
        ] {
            let output = check(
                &[identity],
                "int main(void) { int f(void) { return 0; } return f(); }\n",
            );
            assert!(!output.status.success(), "{identity}");
            let stderr = text(&output.stderr);
            assert!(stderr.contains(message), "{identity}: {stderr}");
        }
    }

    /// A nested function's address is made at run time, so it is not an
    /// address constant, as GCC reports.
    #[test]
    fn a_nested_function_address_is_not_a_constant() {
        let output = check(
            &[GCC],
            "int f(int n) { int get(void) { return n; } static int (*p)(void) = get; return p(); \
             }\n",
        );
        assert!(!output.status.success());
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("static-storage initializer requires a constant expression"),
            "{stderr}"
        );
    }

    /// Objects compiled apart link as GCC's driver links them: an object
    /// whose note asks for an executable stack gets one from LLD too,
    /// whether bcc or the host GCC built it.
    #[test]
    fn separately_compiled_objects_link_with_an_executable_stack() {
        let main = temporary("main.c");
        let apply = temporary("apply.c");
        std::fs::write(
            &main,
            "#include <stdio.h>\nint apply(int (*f)(int), int v);\nint main(void) { int n = 40; \
             int add(int x) { return x + n; } printf(\"%d\\n\", apply(add, 2)); return 0; }\n",
        )
        .unwrap();
        std::fs::write(&apply, "int apply(int (*f)(int), int v) { return f(v); }\n").unwrap();
        let objects: Vec<PathBuf> = (0..3).map(|_| temporary("part.o")).collect();
        for (source, object, backend) in [
            (&main, &objects[0], "--backend=llvm"),
            (&apply, &objects[1], "--backend=native"),
        ] {
            let built = bcc(&[
                "--gcc-mode",
                "-Xbcc",
                backend,
                "-c",
                source.to_str().unwrap(),
                "-o",
                object.to_str().unwrap(),
            ]);
            assert!(built.status.success(), "{}", text(&built.stderr));
        }
        let mut sets = vec![[&objects[0], &objects[1]]];
        let gcc = Command::new("gcc")
            .args([
                "-c",
                main.to_str().unwrap(),
                "-o",
                objects[2].to_str().unwrap(),
            ])
            .output()
            .is_ok_and(|output| output.status.success());
        if gcc {
            sets.push([&objects[2], &objects[1]]);
        } else {
            eprintln!("skipping the GCC-built object: no host GCC");
        }
        for set in sets {
            let executable = temporary("linked.exe");
            let linked = bcc(&[
                "--gcc-mode",
                set[0].to_str().unwrap(),
                set[1].to_str().unwrap(),
                "-o",
                executable.to_str().unwrap(),
            ]);
            assert!(linked.status.success(), "{}", text(&linked.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(text(&run.stdout), "42\n");
            std::fs::remove_file(executable).unwrap();
        }
        for path in objects.iter().filter(|path| path.exists()) {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_file(main).unwrap();
        std::fs::remove_file(apply).unwrap();
    }

    /// A `goto` out of a nested function, and a use of a variably modified
    /// object of the function around it, are refused rather than
    /// miscompiled.
    #[test]
    fn unimplemented_forms_are_refused() {
        for (source, reason) in [
            (
                "int main(void) { __label__ out; void f(void) { goto out; } f(); return 1; out: \
                 return 0; }\n",
                "a jump out of a nested function",
            ),
            (
                "int main(void) { int n = 2; int a[n]; int f(void) { return a[1]; } return f(); \
                 }\n",
                "a nested function's use of a variably modified object of its definer",
            ),
            // The definer refuses the definition too, rather than reaching
            // the bound it would evaluate there (gcc.dg/nested-vla-2.c).
            (
                "char tmp[2]; int main(void) { int n = 1; __typeof__(char (*)[++n]) bar(void) { \
                 return &tmp; } return sizeof *bar() != 2; }\n",
                "a nested function's use of a variably modified object of its definer",
            ),
        ] {
            let output = check(&[GCC, "--emit=ir"], source);
            assert!(!output.status.success(), "{source}");
            let stderr = text(&output.stderr);
            assert!(stderr.contains(reason), "{stderr}");
            assert!(!stderr.contains("internal error"), "{stderr}");
        }
        // MSVC's `__chkstk` clobbers the `%r10` the chain arrives in.
        let output = check(
            &[GCC, "--target=x86_64-pc-windows-msvc", "--emit=llvm"],
            "int main(void) { int n = 3; int get(void) { return n; } return get(); }\n",
        );
        assert!(!output.status.success());
        assert!(
            text(&output.stderr).contains("nested functions on x86_64-pc-windows-msvc"),
            "{}",
            text(&output.stderr)
        );
        let output = check(
            &[GCC, "--target=wasm32-wasip1", "--emit=asm"],
            "__attribute__((noinline)) int apply(int (*f)(void)) { return f(); }\nint main(void) \
             { int n = 3; int get(void) { return n; } return apply(get); }\n",
        );
        assert!(!output.status.success());
        assert!(
            text(&output.stderr).contains("the address of a nested function"),
            "{}",
            text(&output.stderr)
        );
    }

    /// `source` compiled with `flags`, its output on stdout.
    fn check(flags: &[&str], source: &str) -> Output {
        let input = temporary("check.c");
        std::fs::write(&input, source).unwrap();
        let output = bcc(&[flags, &[input.to_str().unwrap()]].concat());
        std::fs::remove_file(input).unwrap();
        output
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH"),
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
            "bcc-nested-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
