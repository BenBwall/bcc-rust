//! bcc's own x86 intrinsic headers (`src/headers/mmintrin.h` through
//! `smmintrin.h`, plain C over GNU vectors) against the host GCC's, which
//! compile to the instructions. `x86_intrinsics_sse2.c` covers MMX, SSE and
//! SSE2; `x86_intrinsics_sse4.c` SSE3 through SSE4.2 and needs a processor
//! with SSE4.2 to run GCC's build; `x86_intrinsics_programs.c` is typical
//! SSE2 code; `target_attribute.c` calls functions whose `target` attribute
//! turns on AVX2 and BMI2 where the processor has them. Each is built under
//! both identities,
//! interpreted and compiled through LLVM and the native back end, and must
//! print GCC's output. `x86_intrinsics_avx2.c` covers the AVX, AVX2, AES-NI
//! and PCLMULQDQ intrinsics whose builtins lowering expands, against both
//! GCC's and the pinned Clang's builds, and `avx_target_abi_callee.c` with
//! `avx_target_abi_caller.c` the vectors of functions whose `target`
//! attribute turns AVX on, linked with either compiler's half, as
//! `avx_target_variadic.c` does for a variadic one.

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

    #[test]
    fn sse2_intrinsics_match_gcc() {
        check("x86_intrinsics_sse2.c", &[], &[]);
    }

    #[test]
    fn sse2_programs_match_gcc() {
        check("x86_intrinsics_programs.c", &[], &[]);
    }

    #[test]
    fn sse4_intrinsics_match_gcc() {
        check("x86_intrinsics_sse4.c", &["-msse4.2"], &[]);
    }

    /// Functions whose `target` attribute turns on AVX2 or BMI2, called when
    /// `__builtin_cpu_supports` finds the feature, at -O0 and -O2 and through
    /// the native back end, which writes baseline code for them. The
    /// interpreter, which has no `__builtin_cpu_supports`, takes the fast
    /// paths unasked.
    #[test]
    fn target_attribute_paths_match_gcc() {
        check("target_attribute.c", &[], &["-DALWAYS_FAST"]);
    }

    /// The AVX, AVX2, AES-NI and PCLMULQDQ intrinsics print what GCC's and
    /// the pinned Clang's builds print, through LLVM at -O0 and -O2. The
    /// interpreter runs all but the approximate reciprocals, and the native
    /// back end all but the named instructions it refuses; each such arm is
    /// compared with references built with the same macros. A processor
    /// without AVX2, AES, PCLMULQDQ or SSE4.2 prints "skip" in every build,
    /// and runs no interpreter comparison, whose reference could not run.
    #[test]
    fn avx2_intrinsics_match_gcc_and_clang() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = fixture("x86_intrinsics_avx2.c");
        let mut references = vec![("--compiler-identity=clang", "target/llvm/bin/clang")];
        if Command::new("gcc").arg("--version").output().is_ok() {
            references.push(("--compiler-identity=gcc", "gcc"));
        }
        for (identity, compiler) in references {
            let expected = build_and_run(compiler, &source, &[]);
            let skipped = expected == "skip\n";
            let interpreted = ["-DALWAYS_FAST", "-DNO_APPROX"];
            let arms: [(&[&str], &[&str]); 6] = [
                (&[], &["-O0"]),
                (&[], &["-O2"]),
                (&interpreted, &["--interpret", "--opt=none"]),
                (&interpreted, &["--interpret", "--opt=bcc"]),
                (&["-DNO_NAMED"], &["--backend=native"]),
                (&["-DNO_NAMED"], &["--backend=native", "--opt=bcc"]),
            ];
            for (defines, flags) in arms {
                let interpreting = flags[0] == "--interpret";
                if interpreting && skipped {
                    continue;
                }
                let expected = if defines.is_empty() {
                    expected.clone()
                } else {
                    build_and_run(compiler, &source, defines)
                };
                let context = format!("{identity} {defines:?} {flags:?}");
                let stdout = if interpreting {
                    let output = bcc(&[&[identity, "-w", &source][..], defines, flags].concat());
                    assert_eq!(
                        output.status.code(),
                        Some(0),
                        "{context}: {}",
                        text(&output.stderr)
                    );
                    text(&output.stdout)
                } else {
                    let executable = temporary("avx2.exe");
                    let built = bcc(&[
                        &[identity, "-w", &source, "-o", executable.to_str().unwrap()][..],
                        defines,
                        flags,
                    ]
                    .concat());
                    assert!(built.status.success(), "{context}: {}", text(&built.stderr));
                    let run = run_native(&mut Command::new(&executable)).unwrap();
                    std::fs::remove_file(&executable).unwrap();
                    assert_eq!(run.status.code(), Some(0), "{context}");
                    text(&run.stdout)
                };
                assert_lines(&stdout, &expected, &context);
            }
        }
    }

    /// External functions whose `target` attribute turns AVX on, in units
    /// built without AVX, link with the claimed compiler's build of the
    /// other half, in both directions: GCC passes their 32-byte vectors in
    /// YMM registers, and Clang their arguments in memory and their result
    /// in YMM0. One is reached through a pointer.
    #[test]
    fn avx_target_functions_link_with_gcc_and_clang() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let definitions = fixture("avx_target_abi_callee.c");
        let calls = fixture("avx_target_abi_caller.c");
        let mut references = vec![("--compiler-identity=clang", "target/llvm/bin/clang")];
        if Command::new("gcc").arg("--version").output().is_ok() {
            references.push(("--compiler-identity=gcc", "gcc"));
        }
        for (identity, compiler) in references {
            let expected = build_and_run(compiler, &definitions, &[&calls]);
            for (ours, theirs) in [(&definitions, &calls), (&calls, &definitions)] {
                for flags in [&["-O0"][..], &["-O2"]] {
                    let context = format!("{identity} {flags:?} {ours}");
                    let object = temporary("part.o");
                    let built = bcc(&[
                        &[identity, "-w", "-c", ours, "-o", object.to_str().unwrap()][..],
                        flags,
                    ]
                    .concat());
                    assert!(built.status.success(), "{context}: {}", text(&built.stderr));
                    let output = build_and_run(compiler, theirs, &[object.to_str().unwrap()]);
                    std::fs::remove_file(object).unwrap();
                    assert_lines(&output, &expected, &context);
                }
            }
        }
    }

    /// A variadic function whose `target` attribute turns AVX on, with a
    /// named 32-byte vector: Clang's convention passes it in memory, as LLVM
    /// does, so each half bcc builds links with Clang's build of the other;
    /// GCC's passes it in YMM0, where LLVM cannot, so bcc refuses both the
    /// definition and the call under GCC's identity.
    #[test]
    fn avx_target_variadic_functions_follow_each_convention() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = fixture("avx_target_variadic.c");
        let clang = "target/llvm/bin/clang";
        let callee = temporary("variadic-callee.o");
        let built = Command::new(clang)
            .args(["-DCALLEE", "-c", "-O0", "-w", &source, "-o"])
            .arg(&callee)
            .output()
            .unwrap();
        assert!(built.status.success(), "{}", text(&built.stderr));
        let expected = build_and_run(clang, &source, &[callee.to_str().unwrap()]);
        std::fs::remove_file(&callee).unwrap();
        for flags in [&["-O0"][..], &["-O2"]] {
            for (ours, theirs) in [(&["-DCALLEE"][..], &[][..]), (&[], &["-DCALLEE"])] {
                let context = format!("{flags:?} {ours:?}");
                let object = temporary("variadic-part.o");
                let object = object.to_str().unwrap();
                let built = bcc(&[
                    &["--compiler-identity=clang", "-w", "-c", &source, "-o", object][..],
                    ours,
                    flags,
                ]
                .concat());
                assert!(built.status.success(), "{context}: {}", text(&built.stderr));
                let output = build_and_run(clang, &source, &[theirs, &[object]].concat());
                std::fs::remove_file(object).unwrap();
                assert_lines(&output, &expected, &context);
                let refused = bcc(&[
                    &["--compiler-identity=gcc", "-w", "-c", &source, "-o", object][..],
                    ours,
                    flags,
                ]
                .concat());
                assert!(!refused.status.success(), "{context}: GCC identity accepted");
                assert!(
                    text(&refused.stderr).contains("a named 32-byte vector parameter of a variadic"),
                    "{context}: {}",
                    text(&refused.stderr)
                );
            }
        }
    }

    /// The output of `source` built by `compiler` with `extra` arguments
    /// (macros, more sources and objects) at -O0.
    fn build_and_run(compiler: &str, source: &str, extra: &[&str]) -> String {
        let executable = temporary("reference.exe");
        let built = Command::new(compiler)
            .args(extra)
            .args([source, "-O0", "-w", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{compiler}: {}",
            text(&built.stderr)
        );
        let run = run_native(&mut Command::new(&executable)).unwrap();
        std::fs::remove_file(executable).unwrap();
        assert_eq!(run.status.code(), Some(0), "{compiler} {extra:?}");
        text(&run.stdout)
    }

    /// An out-of-range immediate is Clang's default-error
    /// `-Wargument-outside-range`, which `-w` silences, while GCC's
    /// `_mm_shuffle_pd` takes any constant; a variable is an error for both.
    /// `__m64` holds two `int`s for GCC and one `long long` for Clang.
    /// Freestanding, so `<mm_malloc.h>` needs no C library: bcc's default
    /// target is Linux on every host, a cross target on Windows.
    #[test]
    fn immediates_and_m64_follow_each_identity() {
        let source = temporary("immediates.c");
        std::fs::write(
            &source,
            "#include <emmintrin.h>\n__m128d f(__m128d a, __m128d b) { return _mm_shuffle_pd(a, \
             b, 0xab); }\n#ifdef VARIABLE\n__m128d g(__m128d a, __m128d b, int i) { return \
             _mm_shuffle_pd(a, b, i); }\n#endif\n#ifdef \
             __clang__\n_Static_assert(sizeof(((__m64){0})[0]) == 8, \
             \"\");\n#else\n_Static_assert(sizeof(((__m64){0})[0]) == 4, \"\");\n__m64_u u; \
             __v2sf w;\n#endif\n",
        )
        .unwrap();
        let source = source.to_str().unwrap();
        for (flags, accepted) in [
            (&["--compiler-identity=gcc"][..], true),
            (&["--compiler-identity=gcc", "-DVARIABLE"], false),
            (&["--compiler-identity=clang"], false),
            (&["--compiler-identity=clang", "-w"], true),
            (
                &["--compiler-identity=clang", "-Wno-argument-outside-range"],
                true,
            ),
            (&["--compiler-identity=clang", "-w", "-DVARIABLE"], false),
            (
                &[
                    "--compiler-identity=clang",
                    "-Werror",
                    "-Wno-error=argument-outside-range",
                ],
                true,
            ),
            // The headers' immediate checks are no extension of the caller.
            (&["--compiler-identity=gcc", "-pedantic-errors"], true),
            (
                &[
                    "--compiler-identity=clang",
                    "-pedantic-errors",
                    "-Wno-argument-outside-range",
                ],
                true,
            ),
        ] {
            let output = bcc(&[&["-ffreestanding", "-fsyntax-only", source][..], flags].concat());
            assert_eq!(
                output.status.success(),
                accepted,
                "{flags:?}: {}",
                text(&output.stderr)
            );
            let expected = match flags {
                | ["--compiler-identity=clang"] =>
                    "error: argument value 171 is outside the valid range [0, 3] \
                     [-Wargument-outside-range]",
                | [_, "-Werror", _] =>
                    "warning: argument value 171 is outside the valid range [0, 3] \
                     [-Wargument-outside-range]",
                | _ => "",
            };
            assert!(
                text(&output.stderr).contains(expected),
                "{flags:?}: {}",
                text(&output.stderr)
            );
            if flags.contains(&"-pedantic-errors") {
                assert!(
                    !text(&output.stderr).contains("extension"),
                    "{flags:?}: {}",
                    text(&output.stderr)
                );
            }
        }
    }

    /// Builds `name` with the host GCC and compares every bcc arm's output
    /// with that build's. `features` enable the instruction sets the program
    /// needs, in both compilers: an intrinsic whose header function has a
    /// `target` attribute needs its feature in the caller. The interpreter's
    /// arms add `interpreted`.
    fn check(name: &str, features: &[&str], interpreted: &[&str]) {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = fixture(name);
        let Some(expected) = reference(&source, features) else {
            eprintln!("note: no host GCC build of {name} runs here; skipping");
            return;
        };
        for identity in ["--compiler-identity=gcc", "--compiler-identity=clang"] {
            for flags in [
                &["--interpret", "--opt=none"][..],
                &["--interpret", "--opt=bcc"],
            ] {
                let output =
                    bcc(&[&[identity, &source][..], features, flags, interpreted].concat());
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
                let executable = temporary("intrinsics.exe");
                let built = bcc(&[
                    &[identity, &source, "-o", executable.to_str().unwrap()][..],
                    features,
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

    /// The output of the host GCC's build, or `None` if there is no GCC or
    /// the processor cannot run the build.
    fn reference(source: &str, flags: &[&str]) -> Option<String> {
        let executable = temporary("reference.exe");
        let built = Command::new("gcc")
            .args(flags)
            .args([source, "-O0", "-w", "-o"])
            .arg(&executable)
            .arg("-lm")
            .output()
            .ok()?;
        if !built.status.success() {
            return None;
        }
        let run = run_native(&mut Command::new(&executable)).unwrap();
        std::fs::remove_file(executable).unwrap();
        (run.status.code() == Some(0)).then(|| text(&run.stdout))
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
            "bcc-x86-intrinsics-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
