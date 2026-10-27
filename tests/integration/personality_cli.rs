//! The driver personalities: `--gcc-mode`, `--clang-mode`, `--msvc-mode`,
//! `--clang-cl-mode`, `--tcc-mode` and the executable names that select
//! them. Each reads its compiler's command line, claims its identity in the
//! predefined macros, writes its diagnostic format, answers its version
//! queries, and refuses the options bcc lacks that would change the program.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
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

    /// A fresh working directory, removed afterwards.
    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-personality-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Runs bcc in this directory with `mode` first, unless it is empty.
        fn bcc(&self, mode: &str, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args((!mode.is_empty()).then_some(mode))
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .env_remove("INCLUDE")
                .output()
                .unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
    }

    fn succeeds(output: &Output) {
        assert!(
            output.status.success(),
            "{}\n{}",
            stdout(output),
            stderr(output)
        );
    }

    /// The `#define` lines `mode` predefines, from `-E -dM` (`/E` for the
    /// `cl` personalities, which list them through a probe).
    fn macros(mode: &str) -> String {
        macros_with(mode, &[])
    }

    /// The `#define` lines `mode` predefines with `options`.
    fn macros_with(mode: &str, options: &[&str]) -> String {
        let directory = Directory::new();
        directory.write("empty.c", "");
        let mut args = options.to_vec();
        args.extend(["-E", "-dM", "empty.c"]);
        let output = directory.bcc(mode, &args);
        succeeds(&output);
        stdout(&output)
    }

    fn defines(listing: &str, name: &str) -> Option<String> {
        listing.lines().find_map(|line| {
            let rest = line.strip_prefix("#define ")?.strip_prefix(name)?;
            rest.strip_prefix(' ')
                .map(str::to_owned)
                .or_else(|| rest.is_empty().then(String::new))
        })
    }

    /// Expands `names` in the `cl` personality `mode` with `options`.
    fn cl_expand(mode: &str, options: &[&str], names: &str) -> String {
        let directory = Directory::new();
        directory.write("probe.c", names);
        let mut args = vec!["/nologo", "/EP"];
        args.extend(options);
        args.push("probe.c");
        let output = directory.bcc(mode, &args);
        succeeds(&output);
        stdout(&output).trim().to_owned()
    }

    #[test]
    fn identity_macros_follow_the_personality() {
        let gcc = macros("--gcc-mode");
        assert_eq!(defines(&gcc, "__GNUC__").as_deref(), Some("14"));
        assert_eq!(defines(&gcc, "__GNUC_MINOR__").as_deref(), Some("2"));
        assert_eq!(defines(&gcc, "__VERSION__").as_deref(), Some("\"14.2.0\""));
        assert_eq!(
            defines(&gcc, "__STDC_VERSION__").as_deref(),
            Some("201710L")
        );
        assert_eq!(defines(&gcc, "__clang__"), None);
        assert_eq!(defines(&gcc, "__STRICT_ANSI__"), None);
        assert_eq!(defines(&gcc, "__bcc__").as_deref(), Some("1"));
        let clang = macros("--clang-mode");
        assert_eq!(defines(&clang, "__clang_major__").as_deref(), Some("23"));
        assert_eq!(defines(&clang, "__GNUC__").as_deref(), Some("4"));
        let tcc = macros("--tcc-mode");
        assert_eq!(defines(&tcc, "__TINYC__").as_deref(), Some("927"));
        assert_eq!(
            defines(&tcc, "__STDC_VERSION__").as_deref(),
            Some("199901L")
        );
        assert_eq!(defines(&tcc, "__GNUC__"), None);
        assert_eq!(defines(&tcc, "__clang__"), None);
        let probe = "_MSC_VER _MSC_FULL_VER _MT _DLL _WIN64 __STDC_VERSION__ __GNUC__ __clang__ \
                     _MSVC_TRADITIONAL";
        assert_eq!(
            cl_expand("--msvc-mode", &[], probe),
            "1933 193300000 1 _DLL 1 201710L __GNUC__ __clang__ 0"
        );
        assert_eq!(
            cl_expand("--msvc-mode", &["/MD", "/std:c11"], probe),
            "1933 193300000 1 1 1 201112L __GNUC__ __clang__ 0"
        );
        assert_eq!(
            cl_expand("--clang-cl-mode", &[], "_MSC_VER __GNUC__ __clang__"),
            "1933 __GNUC__ 1"
        );
        assert_eq!(cl_expand("--msvc-mode", &["/DX#5", "/DY=6"], "X Y"), "5 6");
    }

    /// `-funsigned-char` and its relatives in GCC, Clang and TCC, and `/J`
    /// in `cl` and clang-cl, make plain `char` unsigned, with the macros
    /// each compiler defines for `<limits.h>`; the last option wins.
    #[test]
    fn plain_char_signedness_follows_each_compilers_options() {
        let directory = Directory::new();
        directory.write(
            "probe.c",
            "#include <limits.h>\nCHAR_MIN (char)-1 < 0 __CHAR_UNSIGNED__ _CHAR_UNSIGNED\n",
        );
        let expand = |mode: &str, options: &[&str]| {
            let mut args = options.to_vec();
            if mode.ends_with("cl-mode") || mode == "--msvc-mode" {
                args.extend(["/nologo", "/EP", "probe.c"]);
            } else {
                args.extend(["-E", "-P", "probe.c"]);
            }
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            stdout(&output)
                .lines()
                .find(|line| line.contains("(char)"))
                .unwrap_or_default()
                .to_owned()
        };
        let signed = "(- 127 - 1) (char)-1 < 0 __CHAR_UNSIGNED__ _CHAR_UNSIGNED";
        let unsigned = "0 (char)-1 < 0 1 _CHAR_UNSIGNED";
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            assert_eq!(expand(mode, &[]), signed, "{mode}");
            for options in [
                &["-funsigned-char"][..],
                &["-fno-signed-char"],
                &["-fsigned-char", "-funsigned-char"],
            ] {
                assert_eq!(expand(mode, options), unsigned, "{mode} {options:?}");
            }
            for options in [
                &["-fno-unsigned-char"][..],
                &["-funsigned-char", "-fsigned-char"],
                &["-fno-signed-char", "-fno-unsigned-char"],
            ] {
                assert_eq!(expand(mode, options), signed, "{mode} {options:?}");
            }
        }
        // `cl /J` defines `_CHAR_UNSIGNED`, as clang-cl's does; bcc keeps
        // Clang's `__CHAR_UNSIGNED__` beside it, as clang-cl does.
        let signed_cl = "(- 127 - 1) (char)-1 < 0 __CHAR_UNSIGNED__ _CHAR_UNSIGNED";
        let unsigned_cl = "0 (char)-1 < 0 1 1";
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            assert_eq!(expand(mode, &[]), signed_cl, "{mode}");
            assert_eq!(expand(mode, &["/J"]), unsigned_cl, "{mode}");
        }
        // clang-cl reads `/clang:` options after its own, wherever they
        // stand, and ignores the char options given bare.
        for (options, expected) in [
            (&["/clang:-fsigned-char", "/J"][..], signed_cl),
            (&["/J", "/clang:-fsigned-char"], signed_cl),
            (&["/clang:-funsigned-char"], unsigned_cl),
            (
                &["/clang:-funsigned-char", "/clang:-fsigned-char", "/J"],
                signed_cl,
            ),
        ] {
            assert_eq!(expand("--clang-cl-mode", options), expected, "{options:?}");
        }
        for (options, expected) in [
            (&["-funsigned-char"][..], signed_cl),
            (&["-fsigned-char", "/J"], unsigned_cl),
        ] {
            assert_eq!(expand("--clang-cl-mode", options), expected, "{options:?}");
            let mut args = options.to_vec();
            args.extend(["/EP", "probe.c"]);
            let output = directory.bcc("--clang-cl-mode", &args);
            succeeds(&output);
            assert!(
                stderr(&output).contains("unknown argument ignored in clang-cl: '-f"),
                "{}",
                stderr(&output)
            );
        }
        // The program compiles with the signedness the macros claim.
        directory.write(
            "char.c",
            "#include <limits.h>\n_Static_assert(((char)-1 < 0) == (CHAR_MIN < 0), \
             \"\");\n_Static_assert(CHAR_MIN == 0, \"\");\n",
        );
        for (mode, options) in [
            ("--gcc-mode", &["-c", "-funsigned-char", "char.c"][..]),
            ("--clang-mode", &["-c", "-fno-signed-char", "char.c"]),
            ("--tcc-mode", &["-c", "-funsigned-char", "char.c"]),
            ("--msvc-mode", &["/nologo", "/c", "/J", "char.c"]),
            ("--clang-cl-mode", &["/c", "/J", "char.c"]),
        ] {
            succeeds(&directory.bcc(mode, options));
        }
    }

    /// GCC 7 and later provide `_Float32`, `_Float64`, `_Float32x`,
    /// `_Float64x` and `_Float128` as distinct types with `f32`-style
    /// suffixes and `__FLT32_*__`-family macros, in every mode; Clang, `cl`
    /// and TCC provide none of them in C, so the names stay identifiers
    /// for glibc's typedefs there.
    #[test]
    fn gcc_mode_provides_the_floatn_types_as_gcc_does() {
        let gcc = macros("--gcc-mode");
        for (name, value) in [
            ("__FLT32_MANT_DIG__", "24"),
            ("__FLT64_MAX_EXP__", "1024"),
            ("__FLT32X_DIG__", "15"),
            ("__FLT64X_MANT_DIG__", "64"),
            (
                "__FLT128_MAX__",
                "1.18973149535723176508575932662800702e+4932F128",
            ),
            ("__FLT_EVAL_METHOD_TS_18661_3__", "0"),
            ("__GCC_IEC_559", "2"),
            ("__LDBL_IS_IEC_60559__", "1"),
        ] {
            assert_eq!(defines(&gcc, name).as_deref(), Some(value), "{name}");
        }
        // GCC 14 defines no `__STDC_IEC_60559_TYPES__` for x86-64, and the
        // types are keywords, not macros.
        assert_eq!(defines(&gcc, "__STDC_IEC_60559_TYPES__"), None);
        assert_eq!(defines(&gcc, "_Float32"), None);
        for mode in ["--clang-mode", "--tcc-mode"] {
            let listing = macros(mode);
            assert_eq!(defines(&listing, "__FLT32_MANT_DIG__"), None, "{mode}");
            assert_eq!(defines(&listing, "__GCC_IEC_559"), None, "{mode}");
            assert_eq!(defines(&listing, "_Float32"), None, "{mode}");
        }
        assert_eq!(
            cl_expand("--msvc-mode", &[], "__FLT32_MANT_DIG__ _Float32"),
            "__FLT32_MANT_DIG__ _Float32"
        );
        let directory = Directory::new();
        directory.write(
            "types.c",
            "_Static_assert(_Generic(1.0f64, double: 0, _Float64: 1), \
             \"\");\n_Static_assert(_Generic((_Float32)1, float: 0, _Float32: 1), \
             \"\");\n_Static_assert(_Generic(1.0f64x, long double: 0, _Float64x: 1), \
             \"\");\n_Static_assert(sizeof(_Float128) == 16 && sizeof(_Float32x) == 8, \
             \"\");\n_Complex _Float32 z;\n",
        );
        for std in ["-std=gnu17", "-std=c11", "-std=c23"] {
            succeeds(&directory.bcc("--gcc-mode", &["-fsyntax-only", std, "types.c"]));
        }
        // Without the types the GCC identity spells them as the standard
        // types, which glibc's GCC paths then use, and rejects the suffixes.
        directory.write(
            "names.c",
            "_Float64 x = 1.0; long double *y = (_Float64x *)0;\n",
        );
        succeeds(&directory.bcc(
            "--gcc-mode",
            &["-fsyntax-only", "-fno-floatn-types", "names.c"],
        ));
        assert_eq!(
            directory
                .bcc(
                    "--gcc-mode",
                    &["-fsyntax-only", "-fno-floatn-types", "types.c"]
                )
                .status
                .code(),
            Some(1)
        );
        // Clang 23 knows none of these names in C.
        let clang = directory.bcc("--clang-mode", &["-fsyntax-only", "types.c"]);
        assert_eq!(clang.status.code(), Some(1), "{}", stderr(&clang));
        succeeds(&directory.bcc(
            "--clang-mode",
            &["-fsyntax-only", "-ffloatn-types", "types.c"],
        ));
    }

    /// `<tgmath.h>` takes `_FloatN` arguments under the GCC identity, with
    /// the host's glibc, giving the values GCC's `_FloatN` functions give.
    #[test]
    fn gcc_mode_tgmath_takes_floatn_arguments() {
        if !cfg!(all(target_os = "linux", target_env = "gnu"))
            || !Path::new("/usr/include/tgmath.h").is_file()
        {
            eprintln!("skipping: the test links with the Linux GNU host's C library");
            return;
        }
        let directory = Directory::new();
        directory.write(
            "tg.c",
            "#define _GNU_SOURCE\n#include <stdio.h>\n#include <tgmath.h>\n_Float32 root(_Float32 \
             x) { return sqrt(x); }\nint main(void) {\n  _Float64 y = pow(2.0f64, 10);\n  \
             _Complex _Float32 z = 3.0f32 + 4.0f32 * I;\n  printf(\"%g %g %g %g %g\\n\", \
             (double)root(2.25f32), (double)y, (double)fabs(z), (double)fabs(-1.5f32x), \
             (double)creal(sqrt(-4.0f64 + 0.0f64 * I)));\n  volatile _Float32 x = 1;\n  volatile \
             double w = 1;\n  printf(\"%d %d\\n\", !!signbit(cimag(conj(x))), \
             !!signbit(cimag(conj(w))));\n  return 0;\n}\n",
        );
        for standard in ["-std=gnu17", "-std=c11", "-std=c23"] {
            succeeds(&directory.bcc("--gcc-mode", &[standard, "tg.c", "-lm"]));
            let run = Command::new(directory.path("a.out")).output().unwrap();
            assert_eq!(stdout(&run), "1.5 1024 5 1.5 0\n1 1\n", "{standard}");
            std::fs::remove_file(directory.path("a.out")).unwrap();
        }
        // `conj` of a real argument keeps the imaginary +0 the pinned Clang
        // gives, in bcc's own mode and the Clang personality; GCC gives -0.
        directory.write(
            "conj.c",
            "#include <stdio.h>\n#include <tgmath.h>\nint main(void) {\n  volatile double w = \
             1;\n  printf(\"%d\\n\", !!signbit(cimag(conj(w))));\n  return 0;\n}\n",
        );
        for (mode, args) in [
            ("", &["conj.c", "-o", "a.out", "-lm"][..]),
            ("--clang-mode", &["conj.c", "-lm"]),
            ("--gcc-mode", &["conj.c", "-lm"]),
        ] {
            succeeds(&directory.bcc(mode, args));
            let run = Command::new(directory.path("a.out")).output().unwrap();
            let expected = if mode == "--gcc-mode" { "1\n" } else { "0\n" };
            assert_eq!(stdout(&run), expected, "{mode}");
            std::fs::remove_file(directory.path("a.out")).unwrap();
        }
    }

    /// The floating-point macros of the GCC identity are the host GCC's,
    /// apart from the type bcc lacks (`__bf16`), the standard
    /// types' constants, which bcc spells as Clang does, and what glibc's
    /// `stdc-predef.h` adds.
    #[test]
    fn gcc_mode_floating_macros_match_the_host_gcc() {
        if host_gcc_macros(&[]).is_none() {
            eprintln!("skipping: no x86-64 Linux GCC 13 or later on the host");
            return;
        }
        let compared = |listing: &str| -> Vec<String> {
            let mut lines: Vec<String> = listing
                .lines()
                .filter(|line| {
                    let Some(name) = line.strip_prefix("#define ") else {
                        return false;
                    };
                    [
                        "__FLT16",
                        "__FLT32",
                        "__FLT64",
                        "__FLT128",
                        "__FLT_EVAL_METHOD_TS",
                        "__GCC_IEC_559",
                        "__ASSOCIATIVE_MATH__",
                        "__NO_MATH_ERRNO__",
                        "__NO_SIGNED_ZEROS__",
                        "__NO_TRAPPING_MATH__",
                        "__RECIPROCAL_MATH__",
                    ]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                        || name.contains("_IS_IEC_60559__") && !name.starts_with("__BFLT16")
                })
                .map(str::to_owned)
                .collect();
            lines.sort();
            lines
        };
        let bcc = compared(&macros("--gcc-mode"));
        assert!(bcc.len() > 70, "{bcc:#?}");
        for options in FLOATING_POINT_OPTIONS {
            let host = host_gcc_macros(options).expect("the host GCC takes the options");
            assert_eq!(
                compared(&macros_with("--gcc-mode", options)),
                compared(&host),
                "{options:?}"
            );
        }
    }

    /// GCC's options that give up part of IEC 60559 semantics, alone and
    /// in the combinations whose order or interplay decides the macros.
    const FLOATING_POINT_OPTIONS: [&[&str]; 24] = [
        &[],
        &["-fno-signed-zeros"],
        &["-freciprocal-math"],
        &["-fno-trapping-math"],
        &["-ftrapping-math"],
        &["-fno-math-errno"],
        &["-fassociative-math"],
        &["-fno-signed-zeros", "-fassociative-math"],
        &[
            "-fno-signed-zeros",
            "-fno-trapping-math",
            "-fassociative-math",
        ],
        &["-fcx-limited-range"],
        &["-fcx-fortran-rules"],
        &["-fcx-limited-range", "-fno-cx-fortran-rules"],
        &["-fcx-fortran-rules", "-fno-cx-limited-range"],
        &["-fno-cx-fortran-rules", "-fcx-limited-range"],
        &[
            "-fcx-fortran-rules",
            "-fno-cx-fortran-rules",
            "-fcx-limited-range",
        ],
        &[
            "-fcx-limited-range",
            "-fno-cx-limited-range",
            "-fcx-fortran-rules",
        ],
        &[
            "-fcx-limited-range",
            "-fcx-fortran-rules",
            "-fno-cx-limited-range",
        ],
        &[
            "-fcx-limited-range",
            "-fcx-fortran-rules",
            "-fno-cx-fortran-rules",
        ],
        &["-fno-cx-limited-range"],
        &["-fno-signed-zeros", "-fsigned-zeros"],
        &["-fno-signed-zeros", "-fno-math-errno", "-fno-fast-math"],
        &["-fcx-limited-range", "-fno-fast-math"],
        &[
            "-freciprocal-math",
            "-fno-math-errno",
            "-fno-unsafe-math-optimizations",
        ],
        &[
            "-fassociative-math",
            "-fno-signed-zeros",
            "-fno-trapping-math",
            "-fsigned-zeros",
        ],
    ];

    /// The GCC identity reports the options that give up IEC 60559
    /// semantics as GCC 14 does, without the host GCC.
    #[test]
    fn gcc_mode_floating_macros_follow_the_options() {
        for (options, iec_559, complex, flags) in [
            (&[][..], "2", "2", &[][..]),
            (&["-fno-signed-zeros"], "0", "0", &["__NO_SIGNED_ZEROS__"]),
            (&["-freciprocal-math"], "0", "0", &["__RECIPROCAL_MATH__"]),
            (&["-fcx-limited-range"], "2", "0", &[]),
            // `-f[no-]cx-fortran-rules`, when given, decides alone.
            (
                &["-fcx-fortran-rules", "-fno-cx-limited-range"],
                "2",
                "0",
                &[],
            ),
            (
                &["-fno-cx-fortran-rules", "-fcx-limited-range"],
                "2",
                "2",
                &[],
            ),
            (
                &["-fcx-limited-range", "-fno-cx-limited-range"],
                "2",
                "2",
                &[],
            ),
            (&["-fno-trapping-math"], "2", "2", &["__NO_TRAPPING_MATH__"]),
            (&["-fno-math-errno"], "2", "2", &["__NO_MATH_ERRNO__"]),
            (
                &[
                    "-fno-signed-zeros",
                    "-fno-trapping-math",
                    "-fassociative-math",
                ],
                "0",
                "0",
                &[
                    "__ASSOCIATIVE_MATH__",
                    "__NO_SIGNED_ZEROS__",
                    "__NO_TRAPPING_MATH__",
                ],
            ),
            (&["-fassociative-math"], "2", "2", &[]),
            (&["-fno-signed-zeros", "-fno-fast-math"], "2", "2", &[]),
        ] {
            let listing = macros_with("--gcc-mode", options);
            assert_eq!(
                defines(&listing, "__GCC_IEC_559").as_deref(),
                Some(iec_559),
                "{options:?}"
            );
            assert_eq!(
                defines(&listing, "__GCC_IEC_559_COMPLEX").as_deref(),
                Some(complex),
                "{options:?}"
            );
            for name in [
                "__ASSOCIATIVE_MATH__",
                "__NO_MATH_ERRNO__",
                "__NO_SIGNED_ZEROS__",
                "__NO_TRAPPING_MATH__",
                "__RECIPROCAL_MATH__",
            ] {
                assert_eq!(
                    defines(&listing, name).as_deref(),
                    flags.contains(&name).then_some("1"),
                    "{options:?} {name}"
                );
            }
        }
        // Clang defines none of these but `__NO_MATH_ERRNO__`, which bcc's
        // Clang identity leaves out with Clang's target-dependent default.
        let clang = macros_with("--clang-mode", &["-fno-signed-zeros"]);
        assert_eq!(defines(&clang, "__GCC_IEC_559"), None);
        assert_eq!(defines(&clang, "__NO_SIGNED_ZEROS__"), None);
    }

    /// `gcc -std=gnu17 OPTIONS -dM -E` of an empty file, from a host GCC
    /// (not Clang under its name) that targets x86-64 Linux and defines the
    /// floating-point macros as GCC 13 and later do.
    fn host_gcc_macros(options: &[&str]) -> Option<String> {
        let gcc = |args: &[&str]| {
            Command::new("gcc")
                .args(args)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| stdout(&output))
        };
        let machine = gcc(&["-dumpmachine"])?;
        let major: u32 = gcc(&["-dumpversion"])?
            .trim()
            .split('.')
            .next()?
            .parse()
            .ok()?;
        if !machine.starts_with("x86_64-") || !machine.contains("linux") || major < 13 {
            return None;
        }
        let directory = Directory::new();
        directory.write("empty.c", "");
        let empty = directory.path("empty.c");
        let mut args = vec!["-std=gnu17"];
        args.extend(options);
        args.extend(["-dM", "-E", empty.to_str()?]);
        gcc(&args).filter(|listing| defines(listing, "__clang__").is_none())
    }

    #[test]
    fn version_queries_answer_like_their_compilers_and_name_bcc() {
        let directory = Directory::new();
        let gcc = stdout(&directory.bcc("--gcc-mode", &["--version"]));
        assert!(gcc.starts_with("gcc (bcc "), "{gcc}");
        assert!(gcc.lines().next().unwrap().ends_with(" 14.2.0"), "{gcc}");
        assert!(gcc.contains("It is not GCC"), "{gcc}");
        assert_eq!(
            stdout(&directory.bcc("--gcc-mode", &["-dumpversion"])),
            "14\n"
        );
        assert_eq!(
            stdout(&directory.bcc("--gcc-mode", &["-dumpfullversion"])),
            "14.2.0\n"
        );
        let clang = stdout(&directory.bcc("--clang-mode", &["--version"]));
        assert!(clang.starts_with("clang version 23.1.1 (bcc "), "{clang}");
        let tcc = stdout(&directory.bcc("--tcc-mode", &["-v"]));
        assert!(tcc.starts_with("tcc version 0.9.27 "), "{tcc}");
        assert!(tcc.contains("bcc"), "{tcc}");
        let cl = directory.bcc("--msvc-mode", &[]);
        succeeds(&cl);
        assert!(
            stderr(&cl).contains("Version 19.33.0 for x64"),
            "{}",
            stderr(&cl)
        );
        assert!(stderr(&cl).contains("bcc"), "{}", stderr(&cl));
        assert!(stderr(&cl).contains("usage: cl"), "{}", stderr(&cl));
    }

    #[test]
    fn unknown_options_are_diagnosed_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        // Options GCC knows that cannot change the program are accepted.
        succeeds(&directory.bcc(
            "--gcc-mode",
            &[
                "-c",
                "-shared",
                "-O2",
                "-g3",
                "-Wshadow",
                "-Wformat=2",
                "-Wno-not-a-warning",
                "-fstack-protector-strong",
                "-mtune=native",
                "-pipe",
                "-Wp,-D_FORTIFY_SOURCE=2",
                "x.c",
            ],
        ));
        for (mode, option, message) in [
            (
                "--gcc-mode",
                "-Wbogus",
                "gcc: error: unrecognized command-line option '-Wbogus'",
            ),
            (
                "--gcc-mode",
                "-fbogus",
                "gcc: error: unrecognized command-line option '-fbogus'",
            ),
            (
                "--clang-mode",
                "-fbogus",
                "clang: error: unknown argument: '-fbogus'",
            ),
            ("--tcc-mode", "-q", "tcc: error: invalid option -- '-q'"),
        ] {
            let output = directory.bcc(mode, &["-c", option, "x.c"]);
            assert_eq!(output.status.code(), Some(1), "{mode} {option}");
            assert!(stderr(&output).contains(message), "{}", stderr(&output));
        }
        // Clang warns about an unknown warning option and goes on.
        let clang = directory.bcc("--clang-mode", &["-c", "-Wbogus", "x.c"]);
        succeeds(&clang);
        assert!(stderr(&clang).contains("unknown warning option '-Wbogus'"));
        // TCC ignores the -W, -f and -m options it does not know.
        succeeds(&directory.bcc("--tcc-mode", &["-c", "-Wbogus", "-fbogus", "x.c"]));
        // cl warns (D9002) and goes on. Off Windows, `/bogus` would be a
        // path, as in clang-cl.
        let bogus = if cfg!(windows) { "/bogus" } else { "-bogus" };
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", bogus, "x.c"]);
        succeeds(&cl);
        assert!(
            stderr(&cl).contains(&format!(
                "Command line warning D9002 : ignoring unknown option '{bogus}'"
            )),
            "{}",
            stderr(&cl)
        );
        assert!(directory.path("x.obj").is_file());
    }

    /// Where paths start with `/`, the `cl` and clang-cl personalities read
    /// an argument that names no option clang-cl knows as an input, as
    /// clang-cl's option parser does: absolute sources, `/Fo` and `/I` with
    /// absolute paths, and `--` before a path clang-cl would read as an
    /// option. A missing one draws each compiler's missing-input error, and
    /// clang-cl suggests the option one edit away.
    #[test]
    fn cl_reads_absolute_unix_paths_as_clang_cl_does() {
        if cfg!(windows) {
            eprintln!("skipping: Windows paths do not start with '/'");
            return;
        }
        let directory = Directory::new();
        std::fs::create_dir(directory.path("include")).unwrap();
        std::fs::create_dir(directory.path("objects")).unwrap();
        std::fs::create_dir(directory.path("work")).unwrap();
        directory.write("include/value.h", "#define VALUE 3\n");
        directory.write(
            "t.c",
            "#include \"value.h\"\nint f(void) { return VALUE; }\n",
        );
        directory.write("plain.c", "int g(void) { return 0; }\n");
        // A file named like an option stays an option.
        directory.write("nologo", "");
        let absolute = |name: &str| directory.path(name).to_str().unwrap().to_owned();
        let (source, include) = (absolute("t.c"), absolute("include"));
        assert!(source.starts_with('/'), "{source}");
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            for (args, object) in [
                // Joined and separate `/I`, and `/Fo` naming a file, a
                // directory, and with `/Fo:` a file in the next argument.
                (
                    vec![
                        "/c".to_owned(),
                        source.clone(),
                        format!("/I{include}"),
                        format!("/Fo{}", absolute("objects/joined.obj")),
                    ],
                    "objects/joined.obj",
                ),
                (
                    vec![
                        "/c".to_owned(),
                        "/I".to_owned(),
                        include.clone(),
                        source.clone(),
                        format!("/Fo{}/", absolute("objects")),
                    ],
                    "objects/t.obj",
                ),
                (
                    vec![
                        "/c".to_owned(),
                        format!("-I{include}"),
                        "/Fo:".to_owned(),
                        absolute("objects/separate.obj"),
                        "--".to_owned(),
                        source.clone(),
                    ],
                    "objects/separate.obj",
                ),
                // Without `/Fo`, the object lands in the working directory.
                (
                    vec!["/c".to_owned(), format!("/I{include}"), source.clone()],
                    "t.obj",
                ),
            ] {
                let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
                args.insert(0, "/nologo");
                let output = directory.bcc(mode, &args);
                succeeds(&output);
                assert_eq!(stderr(&output), "", "{mode} {args:?}");
                let object = directory.path(object);
                assert!(object.is_file(), "{mode} {args:?}");
                std::fs::remove_file(object).unwrap();
            }
            // The same command with relative paths, from another directory.
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([
                    mode,
                    "/nologo",
                    "/c",
                    "../t.c",
                    "/I../include",
                    "/Fo../objects/",
                ])
                .current_dir(directory.path("work"))
                .env_remove("INCLUDE")
                .output()
                .unwrap();
            succeeds(&output);
            std::fs::remove_file(directory.path("objects/t.obj")).unwrap();
            // After `--`, a source named like an option is a source (`cl`'s
            // C1083 names it `./-W4.c`, as bcc's command line reads it).
            let output = directory.bcc(mode, &["/nologo", "/c", "--", "-W4.c"]);
            assert!(!output.status.success());
            assert!(stderr(&output).contains("-W4.c'"), "{}", stderr(&output));
            directory.write("-W4.c", "int w;\n");
            succeeds(&directory.bcc(mode, &["/nologo", "/c", "--", "-W4.c"]));
            std::fs::remove_file(directory.path("-W4.c")).unwrap();
            std::fs::remove_file(directory.path("-W4.obj")).unwrap();
        }
        let (missing, plain) = (absolute("missing.c"), absolute("plain.c"));
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", &missing, &plain]);
        assert_eq!(cl.status.code(), Some(2));
        assert_eq!(
            stderr(&cl),
            format!(
                "c1: fatal error C1083: Cannot open source file: '{missing}': No such file or \
                 directory\n"
            )
        );
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", &missing]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert_eq!(
            stderr(&clang_cl),
            format!(
                "clang-cl: error: no such file or directory: '{missing}'\nclang-cl: error: no \
                 input files\n"
            )
        );
        // clang-cl refuses a missing input starting with `/` even when it
        // is for the linker, and suggests an option one edit away.
        for (argument, message) in [
            ("/bogus", "no such file or directory: '/bogus'"),
            (
                "/Brepo",
                "no such file or directory: '/Brepo'; did you mean '/Brepro'?",
            ),
        ] {
            let clang_cl = directory.bcc("--clang-cl-mode", &["/c", argument, &plain]);
            assert_eq!(clang_cl.status.code(), Some(1));
            assert_eq!(stderr(&clang_cl), format!("clang-cl: error: {message}\n"));
        }
        // `cl` takes it for an object, as any input it does not know.
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "/bogus", &plain]);
        succeeds(&cl);
        assert_eq!(
            stderr(&cl),
            "cl : Command line warning D9024 : unrecognized source file type '/bogus', object \
             file assumed\ncl : Command line warning D9027 : source file '/bogus' ignored\n"
        );
        // The value of an option clang-cl knows and bcc does not is no input.
        let clang_cl = directory.bcc(
            "--clang-cl-mode",
            &["/c", "/imsvc", "/nonexistent/include", &plain],
        );
        succeeds(&clang_cl);
        assert_eq!(
            stderr(&clang_cl),
            "clang-cl: warning: unknown argument ignored in clang-cl: '/imsvc \
             /nonexistent/include'\n"
        );
    }

    #[test]
    fn diagnostics_use_each_compilers_location_format() {
        let directory = Directory::new();
        directory.write("bad.c", "int main(void) {\n  int x = ;\n}\n");
        for (mode, args, location, summary) in [
            (
                "--gcc-mode",
                &["-c", "bad.c"][..],
                "bad.c:2:11: error: ",
                false,
            ),
            (
                "--clang-mode",
                &["-c", "bad.c"][..],
                "bad.c:2:11: error: ",
                true,
            ),
            (
                "--tcc-mode",
                &["-c", "bad.c"][..],
                "bad.c:2: error: ",
                false,
            ),
            (
                "--msvc-mode",
                &["/nologo", "/c", "bad.c"][..],
                "bad.c(2,11): error: ",
                false,
            ),
            (
                "--msvc-mode",
                &["/nologo", "/c", "/diagnostics:classic", "bad.c"][..],
                "bad.c(2): error: ",
                false,
            ),
        ] {
            let output = directory.bcc(mode, args);
            // `cl` exits with 2 after an error.
            let status = if mode == "--msvc-mode" { 2 } else { 1 };
            assert_eq!(output.status.code(), Some(status), "{mode}");
            let text = stderr(&output);
            assert!(text.contains(location), "{mode}: {text}");
            assert!(!text.contains("-->"), "{mode}: {text}");
            assert_eq!(
                text.contains("1 error generated."),
                summary,
                "{mode}: {text}"
            );
        }
    }

    #[test]
    fn gcc_links_a_out_with_the_host_c_library() {
        if !cfg!(all(target_os = "linux", target_env = "gnu"))
            || !Path::new("/usr/include/stdio.h").is_file()
        {
            eprintln!("skipping: the test links with the Linux GNU host's C library");
            return;
        }
        let directory = Directory::new();
        directory.write(
            "hello.c",
            "#include <stdio.h>\n#include <math.h>\nint main(void) { printf(\"%.3f\\n\", \
             sqrt(2.0)); return 3; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&directory.bcc(mode, &["-O2", "-Wall", "hello.c", "-lm"]));
            let run = Command::new(directory.path("a.out")).output().unwrap();
            assert_eq!(run.status.code(), Some(3), "{mode}");
            assert_eq!(stdout(&run), "1.414\n", "{mode}");
            std::fs::remove_file(directory.path("a.out")).unwrap();
        }
        // Hello world needs no option in any mode; bcc's own mode links
        // where `-o` says.
        directory.write(
            "plain.c",
            "#include <stdio.h>\nint main(void) { printf(\"hello, world\\n\"); return 0; }\n",
        );
        for (mode, output) in [
            ("", "hello"),
            ("--gcc-mode", "a.out"),
            ("--clang-mode", "a.out"),
        ] {
            let args: &[&str] = if mode.is_empty() {
                &["plain.c", "-o", "hello"]
            } else {
                &["plain.c"]
            };
            succeeds(&directory.bcc(mode, args));
            let run = Command::new(directory.path(output)).output().unwrap();
            assert_eq!(stdout(&run), "hello, world\n", "{mode}");
            std::fs::remove_file(directory.path(output)).unwrap();
        }
        // A cross target searches no C library without a sysroot.
        let cross = directory.bcc(
            "--gcc-mode",
            &[
                "--target=x86_64-unknown-linux-musl",
                "-fsyntax-only",
                "plain.c",
            ],
        );
        assert_eq!(cross.status.code(), Some(1), "{}", stderr(&cross));
        directory.write(
            "args.c",
            "#include <stdio.h>\nint main(int argc, char **argv) { printf(\"%d %s\\n\", argc, \
             argv[argc - 1]); return 7; }\n",
        );
        let run = directory.bcc("--tcc-mode", &["-run", "args.c", "one", "two"]);
        assert_eq!(run.status.code(), Some(7), "{}", stderr(&run));
        assert_eq!(stdout(&run), "3 two\n");
    }

    #[test]
    #[cfg(unix)]
    fn executable_names_select_the_personality() {
        let directory = Directory::new();
        for (name, expected) in [
            ("gcc", "gcc (bcc "),
            ("x86_64-linux-gnu-gcc-14", "x86_64-linux-gnu-gcc-14 (bcc "),
            ("clang", "clang version 23.1.1 (bcc "),
        ] {
            let link = directory.path(name);
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &link).unwrap();
            let output = Command::new(&link).arg("--version").output().unwrap();
            succeeds(&output);
            assert!(stdout(&output).starts_with(expected), "{}", stdout(&output));
        }
        let link = directory.path("x86_64-w64-mingw32-gcc");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &link).unwrap();
        let output = Command::new(&link).arg("-dumpmachine").output().unwrap();
        assert_eq!(stdout(&output), "x86_64-w64-mingw32\n");
    }

    #[test]
    fn cl_names_objects_and_listings_after_the_sources() {
        let directory = Directory::new();
        directory.write("one.c", "int one(void) { return 1; }\n");
        directory.write("two.c", "int two(void) { return 2; }\n");
        std::fs::create_dir(directory.path("out")).unwrap();
        let output = directory.bcc(
            "--msvc-mode",
            &["/nologo", "/c", "one.c", "two.c", "/Foout/"],
        );
        succeeds(&output);
        assert_eq!(stdout(&output), "one.c\ntwo.c\n");
        assert!(directory.path("out/one.obj").is_file());
        assert!(directory.path("out/two.obj").is_file());
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/c", "/Fa", "one.c"]));
        let listing = std::fs::read_to_string(directory.path("one.asm")).unwrap();
        assert!(listing.contains("one:"), "{listing}");
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/P", "/Tcone.c"]));
        assert!(
            std::fs::read_to_string(directory.path("one.i"))
                .unwrap()
                .contains("int one(void)")
        );
        let cpp = directory.bcc("--msvc-mode", &["/nologo", "/c", "/TP", "one.c"]);
        assert_eq!(cpp.status.code(), Some(2));
    }

    #[test]
    fn options_that_change_the_program_are_refused() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        for (mode, option, message) in [
            (
                "--gcc-mode",
                "-fshort-enums",
                "gcc: error: '-fshort-enums' is not supported",
            ),
            (
                "--gcc-mode",
                "-fpack-struct=3",
                "gcc: error: '-fpack-struct=3' is not supported",
            ),
            (
                "--gcc-mode",
                "-mno-sse",
                "gcc: error: '-mno-sse' is not supported",
            ),
            (
                "--gcc-mode",
                "-Ofast",
                "gcc: error: '-Ofast' is not supported",
            ),
            (
                "--clang-mode",
                "-fshort-enums",
                "clang: error: '-fshort-enums' is not supported",
            ),
            (
                "--clang-mode",
                "-fno-delete-null-pointer-checks",
                "clang: error: '-fno-delete-null-pointer-checks' is not supported",
            ),
            (
                "--tcc-mode",
                "-fshort-enums",
                "tcc: error: '-fshort-enums' is not supported",
            ),
        ] {
            let output = directory.bcc(mode, &["-c", option, "x.c"]);
            assert_eq!(output.status.code(), Some(1), "{mode} {option}");
            assert!(stderr(&output).contains(message), "{}", stderr(&output));
        }
        // Hardening warns; a distribution's flags otherwise pass.
        let fedora = directory.bcc(
            "--gcc-mode",
            &[
                "-c",
                "-O2",
                "-flto=auto",
                "-ffat-lto-objects",
                "-g",
                "-grecord-gcc-switches",
                "-pipe",
                "-Wall",
                "-Werror=format-security",
                "-fstack-protector-strong",
                "-m64",
                "-march=x86-64",
                "-mtune=generic",
                "-fasynchronous-unwind-tables",
                "-fstack-clash-protection",
                "-fcf-protection",
                "-mtls-dialect=gnu2",
                "-fno-omit-frame-pointer",
                "-mno-omit-leaf-frame-pointer",
                "x.c",
            ],
        );
        succeeds(&fedora);
        let warnings = stderr(&fedora);
        for line in [
            "gcc: warning: '-fstack-protector-strong' is ignored: bcc does not emit stack \
             protectors",
            "gcc: warning: '-fstack-clash-protection' is ignored",
            "gcc: warning: '-fcf-protection' is ignored",
        ] {
            assert!(warnings.contains(line), "{warnings}");
        }
        assert_eq!(warnings.lines().count(), 3, "{warnings}");
        let tcc = directory.bcc("--tcc-mode", &["-c", "-fstack-protector", "x.c"]);
        succeeds(&tcc);
        assert!(stderr(&tcc).contains("tcc: warning: '-fstack-protector' is ignored"));
    }

    #[test]
    fn wrapv_makes_signed_overflow_wrap() {
        let directory = Directory::new();
        directory.write("w.c", "int f(int x) { return x + 1; }\n");
        let ir = |options: &[&str]| {
            let mut args = vec!["--emit=ir"];
            args.extend(options);
            args.push("w.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        assert!(ir(&[]).contains("iadd.i32 nsw"), "{}", ir(&[]));
        let wrapping = ir(&["-fwrapv"]);
        assert!(wrapping.contains("iadd.i32 v"), "{wrapping}");
        assert!(!wrapping.contains("nsw"), "{wrapping}");
        assert!(ir(&["-fwrapv", "-fno-wrapv"]).contains("nsw"));
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            directory.write(
                "m.c",
                "int f(int x) { return x + 1 < x; }\nint main(void) { return f(2147483647); }\n",
            );
            succeeds(&directory.bcc("--gcc-mode", &["-O2", "-fwrapv", "m.c", "-o", "m"]));
            let run = Command::new(directory.path("m")).output().unwrap();
            assert_eq!(run.status.code(), Some(1));
        }
    }

    #[test]
    fn packing_options_set_the_default_packing() {
        let directory = Directory::new();
        directory.write(
            "p.c",
            "struct s { char c; int i; };\n#pragma pack(push, 2)\nstruct t { char c; int i; \
             };\n#pragma pack(pop)\n#pragma pack()\nstruct u { char c; int i; \
             };\n_Static_assert(sizeof(struct s) == S, \"s\");\n_Static_assert(sizeof(struct t) \
             == 6, \"t\");\n_Static_assert(sizeof(struct u) == S, \"u\");\n",
        );
        succeeds(&directory.bcc("--gcc-mode", &["-fsyntax-only", "-DS=8", "p.c"]));
        succeeds(&directory.bcc(
            "--gcc-mode",
            &["-fsyntax-only", "-fpack-struct=1", "-DS=5", "p.c"],
        ));
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/Zp1", "/DS=5", "p.c"]));
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/Zp", "/DS=5", "p.c"]));
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/Zp2", "/DS=6", "p.c"]));
        // GCC's `-fpack-struct` without a value packs every structure as
        // the `packed` attribute does and ignores the pragmas (probed with
        // GCC 15.2), unlike `-fpack-struct=N`.
        directory.write(
            "all.c",
            "struct s { char c; int i; };\n#pragma pack(push, 4)\nstruct t { char c; double d; \
             };\n#pragma pack(pop)\nstruct x { char c; int i __attribute__((aligned(8))); \
             };\n_Static_assert(sizeof(struct s) == S, \"s\");\n_Static_assert(sizeof(struct t) \
             == T, \"t\");\n_Static_assert(sizeof(struct x) == X, \"x\");\n",
        );
        let all = directory.bcc(
            "--gcc-mode",
            &[
                "-fsyntax-only",
                "-fpack-struct",
                "-DS=5",
                "-DT=9",
                "-DX=16",
                "all.c",
            ],
        );
        succeeds(&all);
        assert!(
            stderr(&all).contains("'#pragma pack' has no effect with '-fpack-struct' - ignored"),
            "{}",
            stderr(&all)
        );
        succeeds(&directory.bcc(
            "--gcc-mode",
            &[
                "-fsyntax-only",
                "-fpack-struct=1",
                "-DS=5",
                "-DT=12",
                "-DX=5",
                "all.c",
            ],
        ));
        succeeds(&directory.bcc(
            "--gcc-mode",
            &[
                "-fsyntax-only",
                "-fpack-struct",
                "-fno-pack-struct",
                "-DS=8",
                "-DT=12",
                "-DX=16",
                "all.c",
            ],
        ));
        let invalid = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/Zp3", "p.c"]);
        assert_eq!(invalid.status.code(), Some(2));
        assert!(
            stderr(&invalid)
                .contains("cl : Command line error D8021 : invalid numeric argument '/Zp3'"),
            "{}",
            stderr(&invalid)
        );
    }

    #[test]
    fn cl_runtime_macros_follow_the_last_runtime_option() {
        let probe = "_MT _DLL _DEBUG";
        for (options, expected) in [
            (&[][..], "1 _DLL _DEBUG"),
            (&["/MD"], "1 1 _DEBUG"),
            (&["/MD", "/MT"], "1 _DLL _DEBUG"),
            (&["/MDd", "/MD"], "1 1 _DEBUG"),
            (&["/MTd"], "1 _DLL 1"),
            (&["/MDd"], "1 1 1"),
            // `/LD` keeps the static runtime unless `/MD` selects another.
            (&["/LD"], "1 _DLL _DEBUG"),
            (&["/LDd"], "1 _DLL 1"),
            (&["/LD", "/MD"], "1 1 _DEBUG"),
        ] {
            assert_eq!(
                cl_expand("--msvc-mode", options, probe),
                expected,
                "{options:?}"
            );
        }
    }

    #[test]
    fn cl_reads_a_command_line_of_only_file_names() {
        let directory = Directory::new();
        directory.write("bad.c", "int x = ;\n");
        // `cl` alone compiles C17 with its column diagnostics.
        let output = directory.bcc("--msvc-mode", &["bad.c"]);
        assert_ne!(output.status.code(), Some(0));
        assert!(
            stderr(&output).contains("bad.c(1,9): error:"),
            "{}",
            stderr(&output)
        );
        directory.write("v.c", "long v = __STDC_VERSION__;\n");
        let output = directory.bcc("--msvc-mode", &["v.c"]);
        assert!(directory.path("v.obj").is_file(), "{}", stderr(&output));
    }

    #[test]
    fn cl_syntax_checks_write_nothing() {
        let directory = Directory::new();
        directory.write("good.c", "int main(void) { return 0; }\n");
        directory.write("bad.c", "int x = ;\n");
        let good = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "good.c"]);
        succeeds(&good);
        assert_eq!(stdout(&good), "good.c\n");
        assert!(!directory.path("good.obj").exists());
        assert!(!directory.path("good.exe").exists());
        let bad = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "bad.c"]);
        assert_eq!(bad.status.code(), Some(2));
        assert!(
            stderr(&bad).contains("bad.c(1,9): error:"),
            "{}",
            stderr(&bad)
        );
    }

    #[test]
    fn cl_object_names_need_a_directory_for_several_sources() {
        let directory = Directory::new();
        directory.write("a.c", "int a;\n");
        directory.write("b.c", "int b;\n");
        let cl = directory.bcc(
            "--msvc-mode",
            &["/nologo", "/c", "a.c", "b.c", "/Foout.obj"],
        );
        assert_eq!(cl.status.code(), Some(2));
        assert!(
            stderr(&cl).contains(
                "cl : Command line error D8036 : '/Foout.obj' not allowed with multiple source \
                 files"
            ),
            "{}",
            stderr(&cl)
        );
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", "a.c", "b.c", "/Foout.obj"]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert!(
            stderr(&clang_cl).contains(
                "clang-cl: error: cannot specify '/Foout.obj' when compiling multiple source files"
            ),
            "{}",
            stderr(&clang_cl)
        );
        assert!(!directory.path("out.obj").exists());
        assert!(!directory.path("a.obj").exists());
        let listing = directory.bcc(
            "--msvc-mode",
            &["/nologo", "/c", "/Falist.asm", "a.c", "b.c"],
        );
        assert_eq!(listing.status.code(), Some(2));
        std::fs::create_dir(directory.path("objs")).unwrap();
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/c", "a.c", "b.c", "/Foobjs/"]));
        assert!(directory.path("objs/a.obj").is_file());
        assert!(directory.path("objs/b.obj").is_file());
    }

    #[test]
    fn cl_conformance_switches_change_the_language() {
        // `/Za` drops `_MSC_EXTENSIONS` and Microsoft's anonymous tagged
        // members; `/Ze` restores them.
        assert_eq!(
            cl_expand("--msvc-mode", &["/Za"], "_MSC_EXTENSIONS _MSC_VER"),
            "_MSC_EXTENSIONS 1933"
        );
        assert_eq!(
            cl_expand("--msvc-mode", &["/Za", "/Ze"], "_MSC_EXTENSIONS"),
            "1"
        );
        let directory = Directory::new();
        directory.write(
            "anon.c",
            "struct inner { int i; };\nstruct outer { struct inner; };\nint f(struct outer *o) { \
             return o->i; }\n",
        );
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/Zs", "anon.c"]));
        let strict = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/Za", "anon.c"]);
        assert_eq!(strict.status.code(), Some(2), "{}", stderr(&strict));
        // `cl` replaces trigraphs only under `/Zc:trigraphs`.
        assert_eq!(cl_expand("--msvc-mode", &[], "\"??=\""), "\"??=\"");
        assert_eq!(
            cl_expand("--msvc-mode", &["/Zc:trigraphs"], "\"??=\""),
            "\"#\""
        );
        assert_eq!(cl_expand("--clang-cl-mode", &[], "\"??=\""), "\"??=\"");
    }

    #[test]
    fn cl_options_that_change_the_program_are_refused() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        for (option, message) in [
            ("/Gv", "cl : Command line error : '/Gv' is not supported"),
            (
                "/arch:AVX2",
                "cl : Command line error : '/arch:AVX2' is not supported",
            ),
            (
                "/fp:strict",
                "cl : Command line error : '/fp:strict' is not supported",
            ),
            (
                "/volatile:ms",
                "cl : Command line error : '/volatile:ms' is not supported",
            ),
        ] {
            let output = directory.bcc("--msvc-mode", &["/nologo", "/c", option, "x.c"]);
            assert_eq!(output.status.code(), Some(2), "{option}");
            assert!(stderr(&output).contains(message), "{}", stderr(&output));
        }
        let hardened = directory.bcc(
            "--msvc-mode",
            &[
                "/nologo",
                "/c",
                "/GS",
                "/RTC1",
                "/guard:cf",
                "/sdl-",
                "/Zc:inline",
                "/arch:SSE2",
                "x.c",
            ],
        );
        succeeds(&hardened);
        let warnings = stderr(&hardened);
        assert!(
            warnings.contains("cl : Command line warning : '/GS' is ignored"),
            "{warnings}"
        );
        assert!(warnings.contains("'/RTC1' is ignored"), "{warnings}");
        assert!(warnings.contains("'/guard:cf' is ignored"), "{warnings}");
        assert_eq!(warnings.lines().count(), 3, "{warnings}");
        // `/external:I` is a system include directory, not a warning level.
        std::fs::create_dir(directory.path("ext")).unwrap();
        directory.write("ext/e.h", "#define E 1\n");
        directory.write("e.c", "#include <e.h>\nint e = E;\n");
        succeeds(&directory.bcc(
            "--msvc-mode",
            &["/nologo", "/c", "/external:I", "ext", "/external:W0", "e.c"],
        ));
    }

    #[test]
    fn sysroot_reaches_bcc_with_and_without_system_headers() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        for options in [
            &["--sysroot=/opt/root"][..],
            &["--sysroot", "/opt/root", "-nostdinc"],
        ] {
            let mut args = vec!["-###"];
            args.extend(options);
            args.extend(["-c", "x.c"]);
            let output = directory.bcc("--gcc-mode", &args);
            succeeds(&output);
            assert!(
                stderr(&output).contains("\"--sysroot=/opt/root\""),
                "{}",
                stderr(&output)
            );
        }
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            // The libraries come from the selected root too.
            std::fs::create_dir(directory.path("empty-root")).unwrap();
            directory.write("main.c", "int main(void) { return 0; }\n");
            let output = directory.bcc(
                "--gcc-mode",
                &["--sysroot=empty-root", "-nostdinc", "main.c"],
            );
            assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
            assert!(
                stderr(&output).contains("cannot build"),
                "{}",
                stderr(&output)
            );
        }
    }

    #[test]
    fn every_personality_fails_after_an_error() {
        let directory = Directory::new();
        directory.write("bad.c", "int x = ;\n");
        directory.write("stop.c", "#error stop\nint a;\n");
        directory.write("pedantic.c", "int a[0];\n");
        directory.write("good.c", "int a;\n");
        let status = |mode: &str, args: &[&str]| directory.bcc(mode, args).status.code();
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            for args in [
                &["-c", "bad.c"][..],
                &["-E", "stop.c"],
                &["bad.c"],
                &["-std=c99", "-pedantic-errors", "-c", "pedantic.c"],
            ] {
                assert_ne!(status(mode, args), Some(0), "{mode} {args:?}");
            }
            assert_eq!(status(mode, &["-c", "good.c"]), Some(0), "{mode}");
        }
        for mode in ["--gcc-mode", "--clang-mode"] {
            assert_eq!(status(mode, &["-fsyntax-only", "bad.c"]), Some(1), "{mode}");
            assert_eq!(
                status(
                    mode,
                    &[
                        "-fsyntax-only",
                        "-std=c99",
                        "-pedantic-errors",
                        "pedantic.c"
                    ]
                ),
                Some(1),
                "{mode}"
            );
            assert_eq!(
                status(mode, &["-fsyntax-only", "good.c"]),
                Some(0),
                "{mode}"
            );
        }
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            for args in [
                &["/nologo", "/Zs", "bad.c"][..],
                &["/nologo", "/c", "bad.c"],
                &["/nologo", "/E", "stop.c"],
            ] {
                assert_ne!(status(mode, args), Some(0), "{mode} {args:?}");
            }
            assert_eq!(
                status(mode, &["/nologo", "/Zs", "good.c"]),
                Some(0),
                "{mode}"
            );
        }
    }

    /// Each personality reports a missing input in its compiler's words,
    /// with its exit status, and compiles the other inputs as that compiler
    /// does: GCC's `cc1` and `cl`'s `c1` report it and go on with the others;
    /// Clang's and clang-cl's drivers refuse it before compiling any; TCC
    /// stops at it.
    #[test]
    fn missing_inputs_are_reported_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("good.c", "int good;\n");
        directory.write("main.c", "int main(void) { return 0; }\n");
        let missing = "No such file or directory";
        let run = |mode: &str, args: &[&str], objects: &[&str]| {
            for object in ["good.o", "good.obj", "main.o", "main.obj"] {
                drop(std::fs::remove_file(directory.path(object)));
            }
            let output = directory.bcc(mode, args);
            for object in ["good.o", "good.obj", "main.o", "main.obj"] {
                assert_eq!(
                    directory.path(object).exists(),
                    objects.contains(&object),
                    "{mode} {args:?}: {object}"
                );
            }
            output
        };
        let gcc = run("--gcc-mode", &["-c", "missing.c", "good.c"], &["good.o"]);
        assert_eq!(gcc.status.code(), Some(1));
        assert_eq!(
            stderr(&gcc),
            format!("cc1: fatal error: missing.c: {missing}\ncompilation terminated.\n")
        );
        let gcc = run("--gcc-mode", &["-fsyntax-only", "missing.c", "good.c"], &[]);
        assert_eq!(gcc.status.code(), Some(1));
        let clang = run("--clang-mode", &["-c", "missing.c", "good.c"], &[]);
        assert_eq!(clang.status.code(), Some(1));
        assert_eq!(
            stderr(&clang),
            "clang: error: no such file or directory: 'missing.c'\n"
        );
        // Only the inputs Clang keeps count as outputs of `-o`.
        let clang = run(
            "--clang-mode",
            &["-c", "missing.c", "good.c", "-o", "out.o"],
            &[],
        );
        assert_eq!(
            stderr(&clang),
            "clang: error: no such file or directory: 'missing.c'\n"
        );
        let clang = run("--clang-mode", &["-c", "missing.o"], &[]);
        assert_eq!(clang.status.code(), Some(1));
        assert_eq!(
            stderr(&clang),
            "clang: error: no such file or directory: 'missing.o'\nclang: error: no input files\n"
        );
        let clang_cl = run(
            "--clang-cl-mode",
            &["/nologo", "/c", "good.c", "missing.c"],
            &[],
        );
        assert_eq!(clang_cl.status.code(), Some(1));
        assert_eq!(
            stderr(&clang_cl),
            "clang-cl: error: no such file or directory: 'missing.c'\n"
        );
        let clang_cl = run("--clang-cl-mode", &["/nologo", "/c", "missing.c"], &[]);
        assert!(
            stderr(&clang_cl).ends_with("clang-cl: error: no input files\n"),
            "{}",
            stderr(&clang_cl)
        );
        // cl compiles every source, reports each missing one with error
        // C1083, skips the link and exits with 2.
        for args in [
            &["/nologo", "/c", "missing.c", "good.c"][..],
            &["/nologo", "missing.c", "main.c"],
        ] {
            let object = if args.contains(&"good.c") {
                "good.obj"
            } else {
                "main.obj"
            };
            let cl = run("--msvc-mode", args, &[object]);
            assert_eq!(cl.status.code(), Some(2), "{args:?}");
            assert_eq!(
                stderr(&cl),
                format!("c1: fatal error C1083: Cannot open source file: 'missing.c': {missing}\n")
            );
            assert_eq!(
                stdout(&cl),
                format!("missing.c\n{}\n", object.replace(".obj", ".c"))
            );
            assert!(!directory.path("missing.exe").exists());
        }
        let tcc = run(
            "--tcc-mode",
            &["-c", "good.c", "missing.c", "main.c"],
            &["good.o"],
        );
        assert_eq!(tcc.status.code(), Some(1));
        assert_eq!(stderr(&tcc), "tcc: error: file 'missing.c' not found\n");
        let tcc = run("--tcc-mode", &["-c", "missing.o", "good.c"], &[]);
        assert_eq!(tcc.status.code(), Some(1));
        assert_eq!(stderr(&tcc), "tcc: error: file 'missing.o' not found\n");
        // GCC's preprocessor reports a directory as a missing file.
        std::fs::create_dir(directory.path("dir.c")).unwrap();
        let gcc = run("--gcc-mode", &["-c", "dir.c"], &[]);
        assert_eq!(gcc.status.code(), Some(1));
        if cfg!(unix) {
            assert_eq!(
                stderr(&gcc),
                format!("cc1: fatal error: dir.c: {missing}\ncompilation terminated.\n")
            );
        }
        // TCC stops at a file it cannot open, also one that exists.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            directory.write("locked.c", "int locked;\n");
            std::fs::set_permissions(
                directory.path("locked.c"),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();
            // A privileged user reads it anyway.
            if std::fs::File::open(directory.path("locked.c")).is_err() {
                let tcc = run("--tcc-mode", &["-c", "locked.c", "good.c"], &[]);
                assert_eq!(tcc.status.code(), Some(1));
                assert_eq!(stderr(&tcc), "tcc: error: file 'locked.c' not found\n");
            }
        }
    }

    /// `cl` and clang-cl after a source that fails: `cl` compiles the rest,
    /// clang-cl stops. `cl` warns about a file it does not know, which it
    /// passes to the linker (warnings D9024 and D9027 under `/c`), and
    /// clang-cl that `/c` leaves it unused.
    #[test]
    fn cl_and_clang_cl_go_on_after_a_failing_source_as_each_does() {
        let directory = Directory::new();
        directory.write("bad.c", "int x = ;\n");
        directory.write("good.c", "int good;\n");
        directory.write("data", "not a source\n");
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "bad.c", "good.c"]);
        assert_eq!(cl.status.code(), Some(2));
        assert!(directory.path("good.obj").is_file());
        std::fs::remove_file(directory.path("good.obj")).unwrap();
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "bad.c", "good.c"]);
        assert_eq!(cl.status.code(), Some(2));
        assert_eq!(stdout(&cl), "bad.c\ngood.c\n");
        let clang_cl = directory.bcc("--clang-cl-mode", &["/nologo", "/c", "bad.c", "good.c"]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert!(!directory.path("good.obj").exists());
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "data", "good.c"]);
        succeeds(&cl);
        assert_eq!(
            stderr(&cl),
            "cl : Command line warning D9024 : unrecognized source file type 'data', object file \
             assumed\ncl : Command line warning D9027 : source file 'data' ignored\n"
        );
        let clang_cl = directory.bcc("--clang-cl-mode", &["/nologo", "/c", "data", "good.c"]);
        succeeds(&clang_cl);
        assert_eq!(
            stderr(&clang_cl),
            "clang-cl: warning: data: 'linker' input unused [-Wunused-command-line-argument]\n"
        );
    }

    /// `-fsyntax-only` writes nothing, also with `-c`, `-S` or `-o`, but `-E`
    /// and `-M` override it. GCC refuses `-o` with `-c`, `-S` or `-E` for
    /// several inputs, but not with `-M`; Clang refuses it for any stage that
    /// writes a file per input, and warns about the options it leaves unused.
    #[test]
    fn gcc_and_clang_choose_the_stage_and_output_as_each_does() {
        let directory = Directory::new();
        directory.write("a.c", "int a;\n");
        directory.write("b.c", "int b;\n");
        let run = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            assert!(!directory.path("out").exists(), "{mode} {args:?}");
            output
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&run(mode, &["-fsyntax-only", "a.c", "b.c", "-o", "out"]));
            succeeds(&run(mode, &["-fsyntax-only", "-c", "a.c", "-o", "out"]));
            assert!(!directory.path("a.o").exists());
            let missing = run(mode, &["-fsyntax-only", "missing.c", "a.c", "-o", "out"]);
            assert_eq!(missing.status.code(), Some(1), "{mode}");
            succeeds(&directory.bcc(mode, &["-fsyntax-only", "-E", "a.c", "-o", "pre.i"]));
            assert!(
                std::fs::read_to_string(directory.path("pre.i"))
                    .unwrap()
                    .contains("int a;")
            );
            let several = run(mode, &["-E", "a.c", "b.c", "-o", "out"]);
            assert_eq!(several.status.code(), Some(1), "{mode}");
        }
        let gcc = run(
            "--gcc-mode",
            &["-fsyntax-only", "-c", "a.c", "b.c", "-o", "out"],
        );
        assert_eq!(gcc.status.code(), Some(1));
        assert_eq!(
            stderr(&gcc),
            "gcc: fatal error: cannot specify '-o' with '-c', '-S' or '-E' with multiple \
             files\ncompilation terminated.\n"
        );
        let clang = run(
            "--clang-mode",
            &["-fsyntax-only", "-c", "a.c", "b.c", "-o", "out"],
        );
        succeeds(&clang);
        assert_eq!(
            stderr(&clang),
            "clang: warning: argument unused during compilation: '-c' \
             [-Wunused-command-line-argument]\n"
        );
        let clang = run("--clang-mode", &["-E", "a.c", "b.c", "-o", "out"]);
        assert_eq!(
            stderr(&clang),
            "clang: error: cannot specify -o when generating multiple output files\n"
        );
        succeeds(&directory.bcc("--gcc-mode", &["-M", "a.c", "b.c", "-o", "deps"]));
        assert!(
            std::fs::read_to_string(directory.path("deps"))
                .unwrap()
                .starts_with("b.o: b.c")
        );
        let clang = run("--clang-mode", &["-M", "a.c", "b.c", "-o", "out"]);
        assert_eq!(clang.status.code(), Some(1));
        let gcc = directory.bcc("--gcc-mode", &[]);
        assert_eq!(
            stderr(&gcc),
            "gcc: fatal error: no input files\ncompilation terminated.\n"
        );
    }

    /// `-fsyntax-only` checks only the C sources: GCC still assembles each
    /// assembly input under `-c`, to `-o` if given, and Clang leaves `.s`
    /// files unused, as it does the linker's inputs, with a warning.
    #[test]
    fn syntax_only_leaves_assembly_to_the_assembler_as_each_does() {
        let directory = Directory::new();
        directory.write("good.c", "int good;\n");
        directory.write("f.s", ".globl f\nf:\n ret\n");
        let gcc = directory.bcc("--gcc-mode", &["-fsyntax-only", "-c", "f.s", "-o", "f1.o"]);
        succeeds(&gcc);
        assert!(directory.path("f1.o").is_file());
        succeeds(&directory.bcc("--gcc-mode", &["-fsyntax-only", "-c", "good.c", "f.s"]));
        assert!(directory.path("f.o").is_file());
        assert!(!directory.path("good.o").exists());
        let missing = directory.bcc("--gcc-mode", &["-fsyntax-only", "-c", "missing.s"]);
        assert_eq!(missing.status.code(), Some(1), "{}", stderr(&missing));
        // Without `-c` GCC still assembles, but keeps nothing; `-S` stops
        // before the assembler, also with `-c`.
        std::fs::remove_file(directory.path("f.o")).unwrap();
        for args in [
            &["-fsyntax-only", "good.c", "f.s"][..],
            &["-fsyntax-only", "f.s", "-o", "out"],
            &["-fsyntax-only", "-c", "-S", "missing.s"],
            &["-fsyntax-only", "-S", "-c", "f.s"],
        ] {
            succeeds(&directory.bcc("--gcc-mode", args));
            for output in ["f.o", "out", "a.out", "f.s.o"] {
                assert!(!directory.path(output).exists(), "{args:?}: {output}");
            }
        }
        for args in [
            &["-fsyntax-only", "missing.s"][..],
            &["-fsyntax-only", "good.c", "missing.s"],
        ] {
            let missing = directory.bcc("--gcc-mode", args);
            assert_eq!(missing.status.code(), Some(1), "{args:?}");
        }
        let clang = directory.bcc(
            "--clang-mode",
            &["-fsyntax-only", "-c", "good.c", "f.s", "f1.o"],
        );
        succeeds(&clang);
        assert_eq!(
            stderr(&clang),
            "clang: warning: f.s: 'assembler' input unused \
             [-Wunused-command-line-argument]\nclang: warning: f1.o: 'linker' input unused \
             [-Wunused-command-line-argument]\nclang: warning: argument unused during \
             compilation: '-c' [-Wunused-command-line-argument]\n"
        );
        assert!(!directory.path("f.o").exists());
        for args in [&["-c", "good.c", "f1.o"][..], &["-c", "f1.o"]] {
            let clang = directory.bcc("--clang-mode", args);
            succeeds(&clang);
            assert_eq!(
                stderr(&clang),
                "clang: warning: f1.o: 'linker' input unused [-Wunused-command-line-argument]\n",
                "{args:?}"
            );
        }
    }

    /// Clang's warnings about unused arguments follow its warning options:
    /// `-w` and `-Wno-unused-command-line-argument` silence them, and
    /// `-Werror` or `-Werror=unused-command-line-argument` make them errors
    /// unless `-Wno-error=unused-command-line-argument` exempts them, and
    /// `-Weverything` does not undo an explicit `-Wno-`.
    #[test]
    fn clang_unused_argument_warnings_follow_the_warning_options() {
        let directory = Directory::new();
        directory.write("good.c", "int good;\n");
        // An object for `-c` compiles nothing, so only the driver warns.
        directory.write("x.o", "");
        let unused = "x.o: 'linker' input unused";
        for (options, expected, status) in [
            (
                &[][..],
                format!("clang: warning: {unused} [-Wunused-command-line-argument]\n"),
                0,
            ),
            (
                &["-Werror"],
                format!("clang: error: {unused} [-Werror,-Wunused-command-line-argument]\n"),
                1,
            ),
            (
                &["-Werror=unused-command-line-argument"],
                format!("clang: error: {unused} [-Werror,-Wunused-command-line-argument]\n"),
                1,
            ),
            (
                &["-Werror", "-Wno-error=unused-command-line-argument"],
                format!("clang: warning: {unused} [-Wunused-command-line-argument]\n"),
                0,
            ),
            (&["-w", "-Werror"], String::new(), 0),
            (
                &["-Werror", "-Wno-unused-command-line-argument"],
                String::new(),
                0,
            ),
            (
                &[
                    "-Wno-unused-command-line-argument",
                    "-Wunused-command-line-argument",
                ],
                format!("clang: warning: {unused} [-Wunused-command-line-argument]\n"),
                0,
            ),
            // `-Weverything` turns on only what no option set, and this
            // warning is on by default.
            (
                &[
                    "-Wno-unused-command-line-argument",
                    "-Weverything",
                    "-Werror",
                ],
                String::new(),
                0,
            ),
            (&["-Wno-everything", "-Weverything"], String::new(), 0),
            (
                &["-Wno-everything", "-Werror=unused-command-line-argument"],
                format!("clang: error: {unused} [-Werror,-Wunused-command-line-argument]\n"),
                1,
            ),
            (
                &[
                    "-Werror=unused-command-line-argument",
                    "-Wno-unused-command-line-argument",
                ],
                String::new(),
                0,
            ),
        ] {
            let output = directory.bcc("--clang-mode", &[options, &["-c", "x.o"]].concat());
            assert_eq!(output.status.code(), Some(status), "{options:?}");
            assert_eq!(stderr(&output), expected, "{options:?}");
        }
        let preprocessed = directory.bcc(
            "--clang-mode",
            &["-Werror", "-fsyntax-only", "-E", "good.c"],
        );
        assert_eq!(preprocessed.status.code(), Some(1));
        assert!(
            stderr(&preprocessed).contains("argument unused during compilation: '-fsyntax-only'"),
            "{}",
            stderr(&preprocessed)
        );
    }

    /// Standard input's outputs are named `-.o` and `-.s`, and clang-cl
    /// preprocesses it but compiles it only with `/Tc`.
    #[test]
    fn standard_input_is_read_as_each_compiler_reads_it() {
        use std::io::Write as _;
        let directory = Directory::new();
        let piped = |mode: &str, args: &[&str]| {
            let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(mode)
                .args(args)
                .current_dir(&directory.0)
                .env("NO_COLOR", "1")
                .env_remove("INCLUDE")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"int from_stdin;\n")
                .unwrap();
            child.wait_with_output().unwrap()
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&piped(mode, &["-c", "-x", "c", "-"]));
            assert!(directory.path("-.o").is_file(), "{mode}");
            std::fs::remove_file(directory.path("-.o")).unwrap();
        }
        let preprocessed = piped("--clang-cl-mode", &["/nologo", "/E", "-"]);
        succeeds(&preprocessed);
        assert!(
            stdout(&preprocessed).contains("int from_stdin;"),
            "{}",
            stdout(&preprocessed)
        );
        let compiled = piped("--clang-cl-mode", &["/nologo", "/c", "-"]);
        assert_eq!(compiled.status.code(), Some(1));
        assert_eq!(
            stderr(&compiled),
            "clang-cl: error: use /Tc or /Tp to set input type for standard input\n"
        );
        // The object is `-.obj`, and an output named like an option is
        // written as given.
        for args in [
            &["/nologo", "/c", "/Tc-"][..],
            &["/nologo", "/c", "/TC", "-"],
        ] {
            succeeds(&piped("--clang-cl-mode", args));
            assert!(directory.path("-.obj").is_file(), "{args:?}");
            std::fs::remove_file(directory.path("-.obj")).unwrap();
        }
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            succeeds(&piped(mode, &["-c", "-x", "c", "-", "-o", "-named.o"]));
            assert!(directory.path("-named.o").is_file(), "{mode}");
            std::fs::remove_file(directory.path("-named.o")).unwrap();
        }
        // An object named like an option is written and assembled too.
        directory.write("f.s", ".globl f\nf:\n ret\n");
        directory.write("f.c", "int g;\n");
        succeeds(&directory.bcc("--gcc-mode", &["-c", "f.s", "-o", "-f.o"]));
        assert!(directory.path("-f.o").is_file());
        succeeds(&directory.bcc("--msvc-mode", &["/nologo", "/c", "/Fo-g.obj", "f.c"]));
        assert!(directory.path("-g.obj").is_file());
    }

    /// A link whose output is named like an option: the temporary objects
    /// named after it reach the linker as files, not options.
    #[test]
    fn links_name_outputs_like_options() {
        if !cfg!(all(target_os = "linux", target_env = "gnu"))
            || !Path::new("/usr/include/stdio.h").is_file()
        {
            eprintln!("skipping: the test links with the Linux GNU host's C library");
            return;
        }
        let directory = Directory::new();
        directory.write(
            "main.c",
            "int helper(void);\nint main(void) { return helper(); }\n",
        );
        directory.write("helper.c", "int helper(void) { return 3; }\n");
        directory.write("solo.c", "int main(void) { return 3; }\n");
        directory.write("f.s", ".globl f\nf:\n ret\n");
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            // One source links from one temporary object; several, from
            // one each.
            for inputs in [
                &["solo.c"][..],
                &["main.c", "helper.c"],
                &["main.c", "helper.c", "f.s"],
            ] {
                let output = directory.bcc(mode, &[inputs, &["-o", "-app"]].concat());
                succeeds(&output);
                let run = Command::new(directory.path("-app")).output().unwrap();
                assert_eq!(run.status.code(), Some(3), "{mode} {inputs:?}");
                std::fs::remove_file(directory.path("-app")).unwrap();
            }
        }
        let names: Vec<String> = std::fs::read_dir(&directory.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|name| !name.contains(".bcc-")),
            "temporary objects remain: {names:?}"
        );
    }

    #[test]
    fn cl_outputs_never_replace_an_input() {
        let directory = Directory::new();
        directory.write("a.c", "int a(void) { return 1; }\n");
        directory.write("b.c", "int b(void) { return 2; }\n");
        directory.write("o.obj", "not an object\n");
        let refused = |mode: &str, args: &[&str], message: &str| {
            let output = directory.bcc(mode, args);
            // clang-cl exits with 1, as Clang does; `cl` with 2.
            let status = if mode == "--clang-cl-mode" { 1 } else { 2 };
            assert_eq!(output.status.code(), Some(status), "{args:?}");
            assert!(
                stderr(&output).contains(message),
                "{args:?}: {}",
                stderr(&output)
            );
        };
        // cl's steps check their own outputs only; the executable, or an
        // object that lands on another input, is checked across them.
        refused(
            "--msvc-mode",
            &["/nologo", "a.c", "b.c", "/Feb.c"],
            "cl : Command line error : output file 'b.c' is the same as input file 'b.c'",
        );
        refused(
            "--msvc-mode",
            &["/nologo", "/c", "a.c", "o.obj", "/Foo.obj"],
            "output file 'o.obj' is the same as input file 'o.obj'",
        );
        refused(
            "--msvc-mode",
            &["/nologo", "/c", "/FA", "a.c", "/Faa.c"],
            "output file 'a.c' is the same as input file 'a.c'",
        );
        refused(
            "--msvc-mode",
            &["/nologo", "/P", "/Tc", "a.c", "/Fia.c"],
            "output file 'a.c' is the same as input file 'a.c'",
        );
        refused(
            "--clang-cl-mode",
            &["a.c", "b.c", "/Feb.c"],
            "clang-cl: error: output file 'b.c' is the same as input file 'b.c'",
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("b.c", directory.path("alias.c")).unwrap();
            refused(
                "--msvc-mode",
                &["/nologo", "a.c", "b.c", "/Fealias.c"],
                "output file 'alias.c' is the same as input file 'b.c'",
            );
        }
        assert_eq!(
            std::fs::read_to_string(directory.path("a.c")).unwrap(),
            "int a(void) { return 1; }\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path("b.c")).unwrap(),
            "int b(void) { return 2; }\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path("o.obj")).unwrap(),
            "not an object\n"
        );
    }

    #[test]
    fn machine_options_follow_the_target_abi() {
        let directory = Directory::new();
        directory.write("x.c", "struct s { char c; int i : 3; } v;\n");
        let gcc = |args: &[&str]| directory.bcc("--gcc-mode", args);
        let refused = gcc(&["-c", "-mms-bitfields", "x.c", "-o", "x.o"]);
        assert_eq!(refused.status.code(), Some(1));
        assert!(
            stderr(&refused).contains("'-mms-bitfields' is not supported"),
            "{}",
            stderr(&refused)
        );
        // `--target` may come after the option.
        succeeds(&gcc(&[
            "-c",
            "-mms-bitfields",
            "--target=x86_64-w64-mingw32",
            "x.c",
            "-o",
            "w.o",
        ]));
        let windows = gcc(&[
            "-c",
            "--target=x86_64-w64-mingw32",
            "-mno-ms-bitfields",
            "x.c",
            "-o",
            "w.o",
        ]);
        assert_eq!(windows.status.code(), Some(1), "{}", stderr(&windows));
        succeeds(&gcc(&[
            "-c",
            "-mno-ms-bitfields",
            "-mlong-double-80",
            "x.c",
            "-o",
            "x.o",
        ]));
        // A newer processor warns: baseline code runs on it.
        for option in ["-march=native", "-march=x86-64-v3", "-mavx2"] {
            let output = gcc(&["-c", option, "x.c", "-o", "x.o"]);
            succeeds(&output);
            assert!(
                stderr(&output).contains(&format!("'{option}' is ignored: bcc generates baseline")),
                "{}",
                stderr(&output)
            );
        }
        succeeds(&gcc(&[
            "-c",
            "-march=x86-64",
            "-mtune=native",
            "x.c",
            "-o",
            "x.o",
        ]));
    }

    #[test]
    fn no_builtin_keeps_library_calls_as_written() {
        let directory = Directory::new();
        directory.write(
            "b.c",
            "double sqrt(double);\nvoid zero(char *p, unsigned long n) { for (unsigned long i = \
             0; i < n; i++) p[i] = 0; }\ndouble root(double x) { return sqrt(x); }\n",
        );
        let assembly = |options: &[&str]| {
            let mut args = vec!["-O2", "-S", "-o", "-"];
            args.extend(options);
            args.push("b.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        // LLVM recognizes library functions by name unless told otherwise.
        let builtin = assembly(&[]);
        assert!(builtin.contains("memset"), "{builtin}");
        assert!(builtin.contains("sqrtsd"), "{builtin}");
        for options in [&["-fno-builtin"][..], &["-ffreestanding"]] {
            let plain = assembly(options);
            assert!(!plain.contains("memset"), "{options:?}: {plain}");
            assert!(!plain.contains("sqrtsd"), "{options:?}: {plain}");
        }
        let memset_only = assembly(&["-fno-builtin-memset"]);
        assert!(!memset_only.contains("memset"), "{memset_only}");
        assert!(memset_only.contains("sqrtsd"), "{memset_only}");
        assert!(assembly(&["-fno-builtin", "-fbuiltin"]).contains("memset"));
        // `-fbuiltin` undoes `-fno-builtin` but keeps the named exclusions,
        // as in GCC.
        let named = assembly(&["-fno-builtin-memset", "-fbuiltin"]);
        assert!(!named.contains("memset"), "{named}");
        assert!(named.contains("sqrtsd"), "{named}");
        // An inline definition's body reaches LLVM as the external
        // function's `available_externally` body, which must carry the
        // controls too, or LLVM folds `strlen("abc")` there before inlining.
        directory.write(
            "copy.c",
            "unsigned long strlen(const char *);\ninline unsigned long h(void) { return \
             strlen(\"abc\"); }\nunsigned long g(void) { return h(); }\n",
        );
        let copy = |options: &[&str]| {
            let mut args = vec!["-O2", "-S", "-o", "-"];
            args.extend(options);
            args.push("copy.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        assert!(!copy(&[]).contains("strlen"), "{}", copy(&[]));
        for options in [
            &["-fno-builtin"][..],
            &["-fno-builtin-strlen"],
            &["-fno-builtin-strlen", "-fbuiltin"],
        ] {
            assert!(
                copy(options).contains("strlen"),
                "{options:?}: {}",
                copy(options)
            );
        }
        let copy_ir = directory.bcc("", &["--emit=llvm", "-O2", "-fno-builtin", "copy.c"]);
        succeeds(&copy_ir);
        let text = stdout(&copy_ir);
        let helper = text
            .lines()
            .find(|line| line.starts_with("define available_externally") && line.contains("@h("))
            .unwrap_or_else(|| panic!("{text}"));
        let group = helper.rsplit('#').next().unwrap().trim_end_matches(" {");
        let attributes = text
            .lines()
            .find(|line| line.starts_with(&format!("attributes #{group} ")))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(attributes.contains("\"no-builtins\""), "{attributes}");
        let ir = directory.bcc("", &["--emit=llvm", "-fno-builtin-sqrt", "b.c"]);
        assert!(
            stdout(&ir).contains("\"no-builtin-sqrt\""),
            "{}",
            stdout(&ir)
        );
        // GCC's loop-distribution switch maps onto the same attributes.
        let gcc = directory.bcc(
            "--gcc-mode",
            &[
                "-O2",
                "-S",
                "-o",
                "-",
                "-fno-tree-loop-distribute-patterns",
                "b.c",
            ],
        );
        succeeds(&gcc);
        assert!(!stdout(&gcc).contains("memset"), "{}", stdout(&gcc));
    }

    #[test]
    fn visibility_applies_to_definitions_without_one() {
        let directory = Directory::new();
        directory.write(
            "v.c",
            "extern int ext;\nint def = 1;\nstatic int priv = \
             2;\n__attribute__((visibility(\"default\"))) int exported(void) { return ext + priv; \
             }\nint helper(void) { return def; }\nint outside(void);\nint use(void) { return \
             outside(); }\n",
        );
        let ir = |options: &[&str]| {
            let mut args = vec!["--emit=llvm"];
            args.extend(options);
            args.push("v.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        let hidden = ir(&["-fvisibility=hidden"]);
        for line in [
            "@def = hidden global",
            "@priv = internal global",
            "@ext = external global",
            "define i32 @exported()",
            "define hidden i32 @helper()",
            "declare i32 @outside()",
        ] {
            assert!(hidden.contains(line), "{line}\n{hidden}");
        }
        assert!(ir(&["-fvisibility=protected"]).contains("define protected i32 @helper()"));
        assert!(!ir(&["-fvisibility=hidden", "-fvisibility=default"]).contains("hidden"));
    }

    #[test]
    fn typical_build_flags_build_and_run() {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            eprintln!("skipping: the test links with the Linux GNU host's C library");
            return;
        }
        let directory = Directory::new();
        directory.write(
            "app.c",
            "#include <stdio.h>\n#include <string.h>\n#include <pthread.h>\nstatic void *run(void \
             *p) { char b[16]; snprintf(b, sizeof b, \"%s\", (char *)p); printf(\"%s %zu\\n\", b, \
             strlen(b)); return 0; }\nint main(void) { pthread_t t; pthread_create(&t, 0, run, \
             \"hello\"); return pthread_join(t, 0); }\n",
        );
        // A CMake release build with distribution hardening, as GCC.
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(
                mode,
                &[
                    "-O2",
                    "-g",
                    "-DNDEBUG",
                    "-std=gnu11",
                    "-Wall",
                    "-Wextra",
                    "-fPIC",
                    "-fvisibility=hidden",
                    "-ffunction-sections",
                    "-fdata-sections",
                    "-fno-strict-aliasing",
                    "-fno-common",
                    "-fno-omit-frame-pointer",
                    "-fstack-protector-strong",
                    "-D_FORTIFY_SOURCE=2",
                    "-march=native",
                    "-mtune=native",
                    "-pthread",
                    "app.c",
                    "-o",
                    "app",
                ],
            );
            succeeds(&output);
            let run = Command::new(directory.path("app")).output().unwrap();
            assert_eq!(stdout(&run), "hello 5\n", "{mode}");
            assert_eq!(run.status.code(), Some(0), "{mode}");
        }
    }

    #[test]
    fn gcc_fortify_wrappers_reach_the_checked_functions() {
        if !cfg!(all(target_os = "linux", target_env = "gnu"))
            || !Path::new("/usr/include/x86_64-linux-gnu/bits/stdio2.h").is_file()
        {
            eprintln!("skipping: the test reads the Linux GNU host's fortify headers");
            return;
        }
        let directory = Directory::new();
        directory.write(
            "f.c",
            "#include <stdio.h>\n#include <string.h>\n#include <fcntl.h>\nint main(int c, char \
             **v) {\n  char b[8];\n  sprintf(b, \"%d\", c);\n  printf(\"%s\\n\", b);\n  int fd = \
             open(\"/dev/null\", O_RDONLY);\n  char d[4];\n  memcpy(d, \"abc\", 4);\n  return fd \
             < 0;\n}\n",
        );
        let flags = ["-O2", "-D_FORTIFY_SOURCE=2", "f.c", "-o", "f"];
        // glibc's GCC wrappers for sprintf, printf and open forward their
        // variadic arguments with __builtin_va_arg_pack, which bcc expands
        // at each call, so they build without a warning, even under
        // -Werror, and call the checked functions.
        let mut strict = vec!["-Werror"];
        strict.extend(flags);
        let output = directory.bcc("--gcc-mode", &strict);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let run = Command::new(directory.path("f")).output().unwrap();
        assert_eq!(stdout(&run), "1\n");
        assert_eq!(run.status.code(), Some(0));
        let output = directory.bcc(
            "--gcc-mode",
            &["-O2", "-D_FORTIFY_SOURCE=2", "-S", "-o", "-", "f.c"],
        );
        succeeds(&output);
        let assembly = stdout(&output);
        for checked in ["__sprintf_chk", "__printf_chk"] {
            assert!(assembly.contains(checked), "{checked}: {assembly}");
        }
        // Every form of calling a wrapper reaches it: through parentheses,
        // `*`, `&`, a block-scope extern declaration and compile-time
        // selections. A parameter or local named like a wrapper is another
        // function.
        directory.write(
            "forms.c",
            "#include <stdio.h>\nint a(char *b, const char *s) { return (*sprintf)(b, \"%s\", s); \
             }\nint c(char *b, const char *s) { return (&sprintf)(b, \"%s\", s); }\nint d(char \
             *b, const char *s) { return ((sprintf))(b, \"%s\", s); }\nint m(char *b) { extern \
             int sprintf(char *, const char *, ...); return sprintf(b, \"x\"); }\nint g(char *b, \
             const char *s) { return _Generic(0, int: sprintf)(b, \"%s\", s); }\nint e(char *b, \
             const char *s) { return __builtin_choose_expr(1, sprintf, 0)(b, \"%s\", s); }\nint \
             x(char *b, const char *s) { return (__extension__ sprintf)(b, \"%s\", s); }\nint \
             h(int (*sprintf)(int)) { return sprintf(7); }\nint k(void) { int (*printf)(int) = 0; \
             return printf ? printf(1) : 0; }\n",
        );
        let output = directory.bcc(
            "--gcc-mode",
            &[
                "-Werror",
                "-O2",
                "-D_FORTIFY_SOURCE=2",
                "-c",
                "forms.c",
                "-o",
                "forms.o",
            ],
        );
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        // glibc's Clang wrappers need no variadic pack.
        let output = directory.bcc("--clang-mode", &flags);
        succeeds(&output);
        assert!(
            !stderr(&output).contains("_FORTIFY_SOURCE"),
            "{}",
            stderr(&output)
        );
    }
}
