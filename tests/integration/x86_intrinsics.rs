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
//! GCC's and the pinned Clang's builds, `x86_named_instructions.c` the
//! named instructions in their SSE and VEX forms and `_xgetbv`, and
//! `avx_target_abi_callee.c` with
//! `avx_target_abi_caller.c` the vectors of functions whose `target`
//! attribute turns AVX on, linked with either compiler's half, as
//! `avx_target_variadic.c` does for a variadic one, and `avx_vector_abi.c`
//! the vectors of a unit built with `-mavx2` in every place the psABI
//! gives them. `x86_wide_immediates.c` passes immediates that change value
//! when they convert to their builtin's parameter type.

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
    /// the pinned Clang's builds print, through LLVM at -O0 and -O2 and
    /// through the native back end, with and without bcc's optimizer and as
    /// a direct object. The interpreter runs all but the approximate
    /// reciprocals, compared with references built without them. A
    /// processor without AVX2, AES, PCLMULQDQ or SSE4.2 prints "skip" in
    /// every build, and runs no interpreter comparison, whose reference
    /// could not run.
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
            for (defines, flags) in arms(&interpreted) {
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
                let stdout = run_arm(identity, &source, defines, flags, &context);
                assert_lines(&stdout, &expected, &context);
            }
        }
    }

    /// The named instructions in their SSE and VEX forms, and `_xgetbv`,
    /// print what GCC's and the pinned Clang's builds print, in every arm
    /// of [`arms`]. Without a processor feature they need, which
    /// `/proc/cpuinfo` lists, the test says so and checks nothing.
    #[test]
    fn named_instructions_match_gcc_and_clang() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let flags = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let flags: Vec<&str> = flags
            .lines()
            .find_map(|line| line.strip_prefix("flags"))
            .map_or_else(Vec::new, |line| line.split_whitespace().collect());
        let missing: Vec<&str> = ["aes", "pclmulqdq", "sse4_2", "avx", "xsave"]
            .into_iter()
            .filter(|feature| !flags.contains(feature))
            .collect();
        if !missing.is_empty() {
            eprintln!("note: the processor lacks {missing:?}; skipping the named instructions");
            return;
        }
        let source = fixture("x86_named_instructions.c");
        let mut references = vec![("--compiler-identity=clang", "target/llvm/bin/clang")];
        if Command::new("gcc").arg("--version").output().is_ok() {
            references.push(("--compiler-identity=gcc", "gcc"));
        }
        for (identity, compiler) in references {
            let expected = build_and_run(compiler, &source, &[]);
            for (defines, flags) in arms(&["-DALWAYS_FAST"]) {
                let context = format!("{identity} {defines:?} {flags:?}");
                let stdout = run_arm(identity, &source, defines, flags, &context);
                assert_lines(&stdout, &expected, &context);
            }
        }
    }

    /// The bcc arms of the intrinsic programs, with their extra macros:
    /// LLVM at -O0 and -O2, the interpreter with `interpreted` (with and
    /// without bcc's optimizer), and the native back end, with and without
    /// it and as a direct object.
    fn arms<'a>(interpreted: &'a [&'a str]) -> [(&'a [&'a str], &'static [&'static str]); 7] {
        [
            (&[], &["-O0"]),
            (&[], &["-O2"]),
            (interpreted, &["--interpret", "--opt=none"]),
            (interpreted, &["--interpret", "--opt=bcc"]),
            (&[], &["--backend=native"]),
            (&[], &["--backend=native", "--opt=bcc"]),
            (&[], &["--backend=native", "--native-object"]),
        ]
    }

    /// The output of `source` in one arm of [`arms`].
    fn run_arm(
        identity: &str,
        source: &str,
        defines: &[&str],
        flags: &[&str],
        context: &str,
    ) -> String {
        if flags[0] == "--interpret" {
            let output = bcc(&[&[identity, "-w", source][..], defines, flags].concat());
            assert_eq!(
                output.status.code(),
                Some(0),
                "{context}: {}",
                text(&output.stderr)
            );
            return text(&output.stdout);
        }
        let executable = temporary("arm.exe");
        let built = bcc(&[
            &[identity, "-w", source, "-o", executable.to_str().unwrap()][..],
            defines,
            flags,
        ]
        .concat());
        assert!(built.status.success(), "{context}: {}", text(&built.stderr));
        let run = run_native(&mut Command::new(&executable)).unwrap();
        std::fs::remove_file(&executable).unwrap();
        assert_eq!(run.status.code(), Some(0), "{context}");
        text(&run.stdout)
    }

    /// `_xgetbv` under each identity: Clang's macro names the builtin a
    /// caller without XSAVE may not call, and GCC's function fails to be
    /// inlined there; with XSAVE both compile, GCC's returning `long long`
    /// and Clang's `unsigned long long`. GCC declares it in
    /// <x86gprintrin.h> as well. Freestanding, as
    /// [`immediates_and_m64_follow_each_identity`] is, so `<mm_malloc.h>`
    /// needs no C library on a Windows host.
    #[test]
    fn xgetbv_follows_each_identity() {
        let source = temporary("xgetbv.c");
        std::fs::write(
            &source,
            r#"#include <immintrin.h>
#ifdef GATED
unsigned long long f(void) { return _xgetbv(0); }
#endif
__attribute__((target("xsave"))) unsigned long long g(void) { return _xgetbv(0); }
#ifdef __clang__
_Static_assert(__builtin_types_compatible_p(__typeof__(_xgetbv(0)), unsigned long long), "");
#else
_Static_assert(__builtin_types_compatible_p(__typeof__(_xgetbv(0)), long long), "");
#endif
"#,
        )
        .unwrap();
        let source = source.to_str().unwrap();
        let object = temporary("xgetbv.o");
        let object = object.to_str().unwrap();
        for (flags, expected) in [
            (&["--compiler-identity=gcc"][..], None),
            (&["--compiler-identity=clang"], None),
            (&["--compiler-identity=gcc", "-mxsave", "-DGATED"], None),
            (
                &["--compiler-identity=gcc", "-DGATED"],
                Some(
                    "inlining failed in call to 'always_inline' '_xgetbv': target specific option \
                     mismatch",
                ),
            ),
            (
                &["--compiler-identity=clang", "-DGATED"],
                Some("'__builtin_ia32_xgetbv' needs target feature xsave"),
            ),
        ] {
            let output =
                bcc(&[&["-ffreestanding", "-c", source, "-o", object][..], flags].concat());
            let stderr = text(&output.stderr);
            assert_eq!(
                output.status.success(),
                expected.is_none(),
                "{flags:?}: {stderr}"
            );
            assert!(
                stderr.contains(expected.unwrap_or("")),
                "{flags:?}: {stderr}"
            );
        }
        let gpr = temporary("xgetbv-gpr.c");
        std::fs::write(
            &gpr,
            "#include <x86gprintrin.h>\n__attribute__((target(\"xsave\"))) long long h(void) { \
             return _xgetbv(0); }\n",
        )
        .unwrap();
        let gpr = gpr.to_str().unwrap();
        let gcc = bcc(&[
            "--compiler-identity=gcc",
            "-ffreestanding",
            "-c",
            gpr,
            "-o",
            object,
        ]);
        assert!(gcc.status.success(), "{}", text(&gcc.stderr));
        std::fs::remove_file(object).unwrap();
        let clang = bcc(&[
            "--compiler-identity=clang",
            "-ffreestanding",
            "-c",
            gpr,
            "-o",
            object,
        ]);
        assert!(
            !clang.status.success() && text(&clang.stderr).contains("_xgetbv"),
            "{}",
            text(&clang.stderr)
        );
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(gpr).unwrap();
    }

    /// External functions whose `target` attribute turns AVX on, in units
    /// built without AVX, link with the claimed compiler's build of the
    /// other half, in both directions, through LLVM and the native back
    /// end: GCC passes their 32-byte vectors in YMM registers, and Clang
    /// their arguments in memory and their result in YMM0, but a record
    /// holding one in memory. One is reached through a pointer.
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
                for flags in COMPILED {
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
    /// does, so each half bcc builds, through LLVM or the native back end,
    /// links with Clang's build of the other;
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
        for flags in COMPILED {
            for (ours, theirs) in [(&["-DCALLEE"][..], &[][..]), (&[], &["-DCALLEE"])] {
                let context = format!("{flags:?} {ours:?}");
                let object = temporary("variadic-part.o");
                let object = object.to_str().unwrap();
                let built = bcc(&[
                    &[
                        "--compiler-identity=clang",
                        "-w",
                        "-c",
                        &source,
                        "-o",
                        object,
                    ][..],
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
                assert!(
                    !refused.status.success(),
                    "{context}: GCC identity accepted"
                );
                assert!(
                    text(&refused.stderr)
                        .contains("a named 32-byte vector parameter of a variadic"),
                    "{context}: {}",
                    text(&refused.stderr)
                );
            }
        }
    }

    /// A unit built with `-mavx2` passes its 32-byte vectors in YMM
    /// registers and on the stack, returns them in YMM0 (and a 64-byte one
    /// in YMM0 and YMM1 under Clang's convention), and reads unnamed ones
    /// with `va_arg`, as the claimed compiler does: each half bcc builds
    /// through LLVM or the native back end links with that compiler's build
    /// of the other, in both directions, and with bcc's own build of the
    /// other half through the other back end; the interpreter runs both
    /// halves as one unit. A processor without AVX2 prints "skip" in every
    /// build and runs no interpreter comparison.
    #[test]
    fn avx_vectors_link_with_gcc_and_clang() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let source = fixture("avx_vector_abi.c");
        let mut references = vec![("--compiler-identity=clang", "target/llvm/bin/clang")];
        if Command::new("gcc").arg("--version").output().is_ok() {
            references.push(("--compiler-identity=gcc", "gcc"));
        }
        let object = |compiler: &str, defines: &[&str]| {
            let object = temporary("vector-part.o");
            let built = Command::new(compiler)
                .args(defines)
                .args(["-mavx2", "-O0", "-w", "-c", &source, "-o"])
                .arg(&object)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{compiler}: {}",
                text(&built.stderr)
            );
            object
        };
        let ours = |identity: &str, defines: &[&str], flags: &[&str], context: &str| {
            let object = temporary("vector-ours.o");
            let built = bcc(&[
                &[identity, "-mavx2", "-w", "-c", &source, "-o"][..],
                &[object.to_str().unwrap()],
                defines,
                flags,
            ]
            .concat());
            assert!(built.status.success(), "{context}: {}", text(&built.stderr));
            object
        };
        for (identity, compiler) in references {
            let callee = object(compiler, &["-DCALLEE"]);
            let expected = build_and_run(compiler, &source, &["-mavx2", callee.to_str().unwrap()]);
            std::fs::remove_file(&callee).unwrap();
            for flags in COMPILED {
                for (half, other) in [(&["-DCALLEE"][..], &[][..]), (&[], &["-DCALLEE"])] {
                    let context = format!("{identity} {flags:?} {half:?}");
                    let mine = ours(identity, half, flags, &context);
                    let theirs = object(compiler, other);
                    let output = build_and_run(
                        compiler,
                        mine.to_str().unwrap(),
                        &[theirs.to_str().unwrap()],
                    );
                    std::fs::remove_file(mine).unwrap();
                    std::fs::remove_file(theirs).unwrap();
                    assert_lines(&output, &expected, &context);
                }
            }
            // Both halves bcc's, one through each back end.
            for (callee, caller) in [
                (&["--backend=native"][..], &["-O2"][..]),
                (&["-O2"], &["--backend=native", "--opt=bcc"]),
            ] {
                let context = format!("{identity} callee {callee:?} caller {caller:?}");
                let definitions = ours(identity, &["-DCALLEE"], callee, &context);
                let calls = ours(identity, &[], caller, &context);
                let output = build_and_run(
                    compiler,
                    definitions.to_str().unwrap(),
                    &[calls.to_str().unwrap()],
                );
                std::fs::remove_file(definitions).unwrap();
                std::fs::remove_file(calls).unwrap();
                assert_lines(&output, &expected, &context);
            }
            if expected == "skip\n" {
                continue;
            }
            for flags in [&["--opt=none"][..], &["--opt=bcc"]] {
                let context = format!("{identity} --interpret {flags:?}");
                let output = bcc(&[
                    &[identity, "-mavx2", "-w", "-DWHOLE", "-DALWAYS_FAST"][..],
                    &["--interpret", &source],
                    flags,
                ]
                .concat());
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{context}: {}",
                    text(&output.stderr)
                );
                assert_lines(&text(&output.stdout), &expected, &context);
            }
        }
    }

    /// The compiled bcc arms of the ABI tests: LLVM at -O0 and -O2, and the
    /// native back end with and without bcc's optimizer and as a direct
    /// object.
    const COMPILED: [&[&str]; 5] = [
        &["-O0"],
        &["-O2"],
        &["--backend=native"],
        &["--backend=native", "--opt=bcc"],
        &["--backend=native", "--native-object"],
    ];

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

    /// Immediates wider than their builtin's `int` parameter, negative, or
    /// above a byte take the value of their conversion to the parameter,
    /// whose low byte the instruction encodes: `0x8000000000000101ULL`
    /// shuffles as 257. GCC 14 accepts every call in
    /// `x86_wide_immediates.c`, and its builds and the pinned Clang's under
    /// `-w` print `expected` at -O0 and -O2. bcc must print it through LLVM
    /// and the native back end at both levels, and through the interpreter,
    /// under both identities; Clang's refuses the values outside its limits
    /// unless `-w` silences that.
    #[test]
    fn wide_immediates_take_their_converted_value() {
        if !cfg!(all(
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )) {
            return;
        }
        let cpu = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let flags: Vec<&str> = cpu
            .lines()
            .find_map(|line| line.strip_prefix("flags"))
            .map_or_else(Vec::new, |line| line.split_whitespace().collect());
        if !["avx2", "pclmulqdq"]
            .iter()
            .all(|feature| flags.contains(feature))
        {
            eprintln!("note: the processor lacks AVX2 or PCLMULQDQ; skipping");
            return;
        }
        let expected = concat!(
            "pshufd-257 2 1 1 1\n",
            "pblendd-17 5 2 3 4\n",
            "pshufd-neg1 4 4 4 4\n",
            "pshufd-neg255 2 1 1 1\n",
            "pshufd-0x31b 4 3 2 1\n",
            "pshuflw-wide 13 12 11 10 14 15 16 17\n",
            "pshufhw-wide 10 11 12 13 17 16 15 14\n",
            "shufps-wide 3 4 5 6\n",
            "shufpd-wide 2 3\n",
            "pblendw-wide 10 11 12 13 114 115 116 117\n",
            "vec-ext-wide 3\n",
            "pclmul-wide 27 0\n",
        );
        let source = fixture("x86_wide_immediates.c");
        for identity in [
            &["--compiler-identity=gcc"][..],
            &["--compiler-identity=clang", "-w"],
        ] {
            let common = [identity, &[source.as_str(), "-mavx2", "-mpclmul"]].concat();
            let interpreted = bcc(&[&common[..], &["--interpret"]].concat());
            assert_eq!(
                interpreted.status.code(),
                Some(0),
                "{identity:?} --interpret: {}",
                text(&interpreted.stderr)
            );
            assert_lines(
                &text(&interpreted.stdout),
                expected,
                &format!("{identity:?} --interpret"),
            );
            for flags in [
                &["-O0"][..],
                &["-O2"],
                &["--backend=native", "-O0"],
                &["--backend=native", "-O2"],
            ] {
                let context = format!("{identity:?} {flags:?}");
                let executable = temporary("wide.exe");
                let built =
                    bcc(&[&common[..], flags, &["-o", executable.to_str().unwrap()]].concat());
                assert!(built.status.success(), "{context}: {}", text(&built.stderr));
                let run = run_native(&mut Command::new(&executable)).unwrap();
                std::fs::remove_file(&executable).unwrap();
                assert_eq!(run.status.code(), Some(0), "{context}");
                assert_lines(&text(&run.stdout), expected, &context);
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
