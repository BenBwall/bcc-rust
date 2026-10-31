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

    /// GCC's and Clang's library aliases (`__builtin_strcat`,
    /// `__builtin_mempcpy`, `__builtin_bcopy`, ...), carry, aligned
    /// `alloca`, complex-part and `_Float16`/`__float128` sign builtins,
    /// and the NaN and infinity constructors, with GCC's `q`, `bswap128`,
    /// `<ctype.h>` and `assoc_barrier` builtins under its identity.
    #[test]
    fn gnu_builtins_match_the_claimed_compiler() {
        matches_the_claimed_compiler("gnu_builtins.c");
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

    /// GCC 13 folds `__builtin_assoc_barrier` of a constant but does not
    /// make it an integer constant expression: a `case` label,
    /// `_Static_assert`, enumerator, bit-field width and static initializer
    /// take its value, and an array it bounds is variably modified.
    #[test]
    fn assoc_barrier_of_a_constant_folds_but_is_no_integer_constant() {
        let compile = |source_text: &str, flags: &[&str]| {
            let source = temporary("barrier.c");
            std::fs::write(&source, source_text).unwrap();
            let object = temporary("barrier.o");
            let output = bcc(&[
                &[
                    "--compiler-identity=gcc",
                    "-c",
                    source.to_str().unwrap(),
                    "-o",
                    object.to_str().unwrap(),
                ][..],
                flags,
            ]
            .concat());
            drop(std::fs::remove_file(object));
            std::fs::remove_file(source).unwrap();
            (output.status.success(), text(&output.stderr))
        };
        let folds =
            "int f(int x) {\nswitch (x) { case __builtin_assoc_barrier(3): return 1; }\nreturn \
             0;\n}\n_Static_assert(__builtin_assoc_barrier(3) == 3, \"folded\");\nenum { E = \
             __builtin_assoc_barrier(3) };\nstruct s { int b : __builtin_assoc_barrier(3); \
             };\nstatic int i = __builtin_assoc_barrier(3);\nstatic double d = \
             __builtin_assoc_barrier(1.5);\nint a[1 ? 2 : __builtin_assoc_barrier(3)];\nstatic \
             int *p = (int *)__builtin_assoc_barrier(3);\n";
        let (accepted, stderr) = compile(folds, &[]);
        assert!(accepted, "{stderr}");
        assert_eq!(stderr, "", "folding is silent without -pedantic");
        let (accepted, stderr) = compile(folds, &["-pedantic-errors"]);
        assert!(!accepted, "-pedantic-errors refuses the folding: {stderr}");
        for refused in [
            "int a[__builtin_assoc_barrier(3)];\n",
            "void f(void) { int a[__builtin_assoc_barrier(3)] = {0}; (void)a; }\n",
            "void f(void) { goto l; int a[__builtin_assoc_barrier(3)]; (void)a; l:; }\n",
            "void f(void) { static int t[__builtin_assoc_barrier(3)]; (void)t; }\n",
        ] {
            let (accepted, stderr) = compile(refused, &[]);
            assert!(!accepted, "{refused}: {stderr}");
        }
    }

    /// Builtin operand checks as GCC 14 and Clang 23 make them: a carry
    /// pointer to the signed type or to a `const` one draws a warning and
    /// one to another width an error; `__builtin_alloca_with_align` takes a
    /// constant power of two from 8 bits to 2^31 (GCC) or 2^30 (Clang), and
    /// `_and_max` a constant bound; and a refused builtin is named also
    /// under a cast to `void`.
    #[test]
    fn builtin_operands_are_checked_as_gcc_and_clang_check_them() {
        let check = |identity: &str, body: &str| {
            let source = temporary("operands.c");
            std::fs::write(
                &source,
                format!("void *f(unsigned long n) {{\n{body}\nreturn 0;\n}}\n"),
            )
            .unwrap();
            let object = temporary("operands.o");
            let output = bcc(&[
                identity,
                "-c",
                source.to_str().unwrap(),
                "-o",
                object.to_str().unwrap(),
            ]);
            drop(std::fs::remove_file(object));
            std::fs::remove_file(source).unwrap();
            (output.status.success(), text(&output.stderr))
        };
        for identity in ["--compiler-identity=gcc", "--compiler-identity=clang"] {
            let (accepted, stderr) = check(
                identity,
                "int s; const unsigned c = 0; __builtin_addc(1u, 1u, 0u, &s); __builtin_subc(1u, \
                 1u, 0u, &c);",
            );
            assert!(accepted, "{identity}: {stderr}");
            assert!(stderr.contains("-Wpointer-sign"), "{identity}: {stderr}");
            assert!(
                stderr.contains("discards qualifiers"),
                "{identity}: {stderr}"
            );
            let (accepted, stderr) =
                check(identity, "unsigned long c; __builtin_addc(1u, 1u, 0u, &c);");
            assert!(!accepted, "{identity}: {stderr}");
            for alignment in ["4", "24", "n", "1ul << 40"] {
                let (accepted, stderr) = check(
                    identity,
                    &format!("__builtin_alloca_with_align(8, {alignment});"),
                );
                assert!(!accepted, "{identity} {alignment}: {stderr}");
            }
            let (accepted, stderr) = check(identity, "__builtin_alloca_with_align(8, 1ul << 29);");
            assert!(accepted, "{identity}: {stderr}");
            let (accepted, _) = check(identity, "__builtin_alloca_with_align(8, 1ul << 31);");
            assert_eq!(accepted, identity.ends_with("gcc"), "{identity}: 2^31 bits");
        }
        let (accepted, stderr) = check(
            "--compiler-identity=gcc",
            "__builtin_alloca_with_align_and_max(8, 64, n);",
        );
        assert!(!accepted, "{stderr}");
        assert!(
            stderr.contains("the third argument must be a constant integer"),
            "{stderr}"
        );
        for bound in ["1.5", "-1", "1 ? 2 : n", "__builtin_assoc_barrier(3)"] {
            let (accepted, stderr) = check(
                "--compiler-identity=gcc",
                &format!("__builtin_alloca_with_align_and_max(8, 64, {bound});"),
            );
            assert!(accepted, "{bound}: {stderr}");
        }
        let (accepted, stderr) = check("--compiler-identity=gcc", "(void) __builtin_apply_args();");
        assert!(!accepted, "{stderr}");
        assert!(stderr.contains("`__builtin_apply_args`"), "{stderr}");
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
