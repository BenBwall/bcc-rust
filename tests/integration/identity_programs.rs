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

    /// Floating constants converted out of an integer type's range in
    /// static initializers store each compiler's saturated value.
    #[test]
    fn float_integer_initializers_match_the_claimed_compiler() {
        matches_the_claimed_compiler("float_integer_initializers.c");
    }

    /// Imaginary interchange constants, with each compiler's excess
    /// precision for `_Float16`.
    #[test]
    fn imaginary_interchange_constants_match_the_claimed_compiler() {
        matches_the_claimed_compiler("imaginary_interchange_constants.c");
    }

    /// The C99 §7.12.14 comparison builtins glibc's `<math.h>` uses, with
    /// `__builtin_issignaling` and `__builtin_roundeven`.
    #[test]
    fn float_comparisons_match_the_claimed_compiler() {
        matches_the_claimed_compiler("float_comparisons.c");
    }

    /// GNU complex integer types: arithmetic, the conjugate, conversions,
    /// component lvalues, static initializers and calls; GCC and Clang
    /// divide them differently.
    #[test]
    fn complex_integers_match_the_claimed_compiler() {
        matches_the_claimed_compiler("complex_integers.c");
    }

    /// GCC's and Clang's library aliases (`__builtin_strcat`,
    /// `__builtin_mempcpy`, `__builtin_bcopy`, ...), carry, aligned
    /// `alloca`, complex-part and `_Float16`/`__float128` sign builtins,
    /// and the NaN and infinity constructors, with GCC's `q`, `bswap128`,
    /// `<ctype.h>` and `assoc_barrier` builtins under its identity.
    #[test]
    fn gnu_builtins_match_the_claimed_compiler() {
        matches_the_claimed_compiler("gnu_builtins.c");
    }

    /// GNU `__auto_type` objects of every kind of type, in the macros that
    /// use it, with Clang keeping an atomic operand's `_Atomic` and GCC's
    /// initializer seeing an outer declaration of the name; on wasm32 too.
    #[test]
    fn auto_type_matches_the_claimed_compiler() {
        matches_the_claimed_compiler_in("auto_type.c", "-std=gnu17", true);
    }

    /// C23 `auto` objects at file and block scope, with storage classes and
    /// qualifiers, and Clang's deduction through pointer and function
    /// declarators; on wasm32 too.
    #[test]
    fn inferred_auto_matches_the_claimed_compiler() {
        matches_the_claimed_compiler_in("inferred_auto.c", "-std=gnu2x", true);
    }

    /// Signed left shifts that wrap, and `__LINE__` as a macro argument in
    /// a replacement list.
    #[test]
    fn torture_semantics_match_the_claimed_compiler() {
        matches_the_claimed_compiler("torture_semantics.c");
    }

    /// `-fno-builtin` and `-fno-builtin-index` make `index` an ordinary
    /// name, so a GNU89 call before the program's own `int index(int)` is
    /// an implicit declaration, as in GCC and Clang; with library builtins
    /// on, the call gets the library's `char *index(const char *, int)`,
    /// whose two arguments both compilers then require.
    #[test]
    fn no_builtin_leaves_library_names_ordinary() {
        let source = temporary("index.c");
        std::fs::write(
            &source,
            "int main(void) { return index(3) - 4; }\nint index(int x) { return x + 1; }\n",
        )
        .unwrap();
        let source = source.to_str().unwrap();
        for identity in ["--compiler-identity=clang", "--compiler-identity=gcc"] {
            for (flag, accepted) in [
                ("-fno-builtin", true),
                ("-fno-builtin-index", true),
                ("-fbuiltin", false),
            ] {
                let output = bcc(&[identity, "-std=gnu89", "-w", flag, "--interpret", source]);
                if accepted {
                    assert_eq!(
                        output.status.code(),
                        Some(0),
                        "{identity} {flag}: {}",
                        text(&output.stderr)
                    );
                } else {
                    assert!(
                        text(&output.stderr).contains("wrong number of arguments"),
                        "{identity} {flag}: {}",
                        text(&output.stderr)
                    );
                }
            }
        }
        std::fs::remove_file(source).unwrap();
    }

    /// `__builtin_bcopy`, `__builtin_bzero` and `__builtin_mempcpy` lower to
    /// LLVM's memory intrinsics on every target, as Clang lowers them, so
    /// the Microsoft C runtime, which has none of the three, needs no such
    /// symbol.
    #[test]
    fn memory_aliases_lower_to_memory_intrinsics() {
        let source = temporary("memory.c");
        std::fs::write(
            &source,
            "char *f(char *d, const char *s, unsigned long n) {\n__builtin_bcopy(s, d, \
             n);\n__builtin_bzero(d, n);\nreturn __builtin_mempcpy(d, s, n);\n}\n",
        )
        .unwrap();
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "x86_64-w64-windows-gnu",
        ] {
            let target = format!("--target={target}");
            let output = bcc(&[&target, "--emit=llvm", source.to_str().unwrap()]);
            let ir = text(&output.stdout);
            assert!(
                output.status.success(),
                "{target}: {}",
                text(&output.stderr)
            );
            for intrinsic in ["@llvm.memmove", "@llvm.memset", "@llvm.memcpy"] {
                assert!(ir.contains(intrinsic), "{target} {intrinsic}: {ir}");
            }
            for symbol in ["@bcopy", "@bzero", "@mempcpy"] {
                assert!(!ir.contains(symbol), "{target} {symbol}: {ir}");
            }
        }
        std::fs::remove_file(source).unwrap();
    }

    /// A wasm32 program's own `index`, `bcmp` and `stpcpy` replace the WASI
    /// runtime's weak definitions instead of clashing with them, under
    /// both identities and optimizers; without `node` this is skipped.
    #[test]
    fn wasm32_programs_define_posix_string_names() {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs/posix_string_names.c")
            .to_str()
            .unwrap()
            .to_owned();
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for identity in ["--compiler-identity=clang", "--compiler-identity=gcc"] {
            for optimization in ["-O0", "-O2"] {
                let module = temporary("names.wasm");
                let built = bcc(&[
                    identity,
                    optimization,
                    "-w",
                    "--target=wasm32-wasip1",
                    &source,
                    "-o",
                    module.to_str().unwrap(),
                ]);
                assert!(
                    built.status.success(),
                    "{identity} {optimization}: {}",
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
                    text(&run.stdout),
                    "abcd 3 7\nxy\n",
                    "{identity} {optimization}"
                );
                std::fs::remove_file(module).unwrap();
            }
        }
    }

    /// Each identity's value of `fixture` matches its compiler's build,
    /// interpreted before and after optimization and built through LLVM
    /// and the native back end.
    fn matches_the_claimed_compiler(fixture: &str) {
        matches_the_claimed_compiler_in(fixture, "-std=gnu17", false);
    }

    /// [`matches_the_claimed_compiler`] in the language mode `standard`;
    /// with `wasm32`, also built for `wasm32-wasip1` and run by `node`
    /// where it is on the path, printing the reference's lines except
    /// those starting `x86:`, which use types wasm32 lacks.
    fn matches_the_claimed_compiler_in(fixture: &str, standard: &str, wasm32: bool) {
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
            let expected = reference(compiler, &source, standard);
            for flags in [
                &["--interpret", "--opt=none"][..],
                &["--interpret", "--opt=bcc"],
            ] {
                let output = bcc(&[&[identity, standard, &source][..], flags].concat());
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
                        standard,
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
            if wasm32 {
                let expected: String = expected
                    .lines()
                    .filter(|line| !line.starts_with("x86:"))
                    .flat_map(|line| [line, "\n"])
                    .collect();
                matches_on_wasm32(fixture, &source, identity, standard, &expected);
            }
        }
    }

    /// `source` built for `wasm32-wasip1` under `identity`, unoptimized and
    /// optimized, prints `expected`; without `node` this is skipped.
    fn matches_on_wasm32(
        fixture: &str,
        source: &str,
        identity: &str,
        standard: &str,
        expected: &str,
    ) {
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for optimization in ["-O0", "-O2"] {
            let module = temporary("program.wasm");
            let built = bcc(&[
                identity,
                standard,
                optimization,
                "-w",
                "--target=wasm32-wasip1",
                source,
                "-o",
                module.to_str().unwrap(),
            ]);
            assert!(
                built.status.success(),
                "{fixture} {identity} wasm32 {optimization}: {}",
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
                "{fixture} {identity} wasm32 {optimization}"
            );
            assert_eq!(
                text(&run.stdout),
                expected,
                "{fixture} {identity} wasm32 {optimization}"
            );
            std::fs::remove_file(module).unwrap();
        }
    }

    /// The output of `source` built by `compiler` in the language mode
    /// `standard`.
    fn reference(compiler: &str, source: &str, standard: &str) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args([source, standard, "-O0", "-w", "-o"])
            .arg(&executable)
            .arg("-lm")
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
