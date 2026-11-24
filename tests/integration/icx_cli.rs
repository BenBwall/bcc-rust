//! The icx and icx-cl personalities (`--icx-mode`, `--icx-cl-mode`, or an
//! executable named `icx`, `icx-cc` or `icx-cl`): Intel oneAPI DPC++/C++
//! Compiler 2026.2.0's identity, defaults, classic-option warning and own
//! options. Every expectation was measured with icx 2026.2.0 on x86-64
//! Linux; `icx_parity_with_a_real_icx` repeats the comparison when `BCC_ICX`
//! names an `icx` binary. `docs/design/driver-modes.md` ("icx") lists what
//! differs.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
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
                "bcc-icx-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            let directory = Self(directory);
            directory.write("e.c", "int main(void) { return 0; }\n");
            directory
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        /// Runs `program` in this directory with `args`.
        fn run(&self, program: impl AsRef<OsStr>, args: &[&str]) -> Output {
            Command::new(program)
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .env_remove("INCLUDE")
                .env_remove("CL")
                .env_remove("_CL_")
                .output()
                .unwrap()
        }

        /// Runs bcc in `mode` with `args`.
        fn bcc(&self, mode: &str, args: &[&str]) -> Output {
            let mut all = vec![mode];
            all.extend_from_slice(args);
            self.run(env!("CARGO_BIN_EXE_bcc-rust"), &all)
        }

        /// Runs `bcc --icx-mode` with `args`.
        fn icx(&self, args: &[&str]) -> Output {
            self.bcc("--icx-mode", args)
        }

        /// The macros `bcc --icx-mode ARGS -dM -E e.c` defines.
        fn macros(&self, args: &[&str]) -> String {
            let mut all = args.to_vec();
            all.extend_from_slice(&["-dM", "-E", "e.c"]);
            let output = self.icx(&all);
            succeeds(&output);
            stdout(&output)
        }
    }

    /// icx's Linux driver, whatever the host: on Windows Intel's `icx` is
    /// `icx-cl`, and bcc's default MinGW target differs (its code is always
    /// position-independent), so the identity tests name the Linux target.
    const LINUX: &str = "--target=x86_64-unknown-linux-gnu";

    /// The macros `bcc --icx-mode --target=x86_64-unknown-linux-gnu ARGS -dM
    /// -E e.c` defines.
    fn linux_macros(directory: &Directory, args: &[&str]) -> String {
        let mut all = vec![LINUX];
        all.extend_from_slice(args);
        directory.macros(&all)
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn succeeds(output: &Output) {
        assert!(
            output.status.success(),
            "status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            stdout(output),
            stderr(output)
        );
    }

    /// Asserts the exit status and exactly what the run wrote to stderr.
    fn reports(output: &Output, status: i32, message: &str) {
        assert_eq!(
            output.status.code(),
            Some(status),
            "stderr:\n{}",
            stderr(output)
        );
        assert_eq!(stderr(output), message);
    }

    fn defines(macros: &str, definition: &str) -> bool {
        macros.lines().any(|line| line == definition)
    }

    fn names(macros: &str, name: &str) -> bool {
        macros.lines().any(|line| {
            line.strip_prefix("#define ")
                .and_then(|rest| rest.split([' ', '(']).next())
                == Some(name)
        })
    }

    /// The warning icx's wrapper gives for the classic `icc` options it
    /// dropped, listing them.
    fn classic_warning(options: &[&str]) -> String {
        let mut text = String::from(
            "icx: command line warning #10430: Unsupported command line options \
             encountered\nThese options as listed are not supported.\nFor more information, use \
             '-qnextgen-diag'.\noption list: \n",
        );
        for option in options {
            text.push('\t');
            text.push_str(option);
            text.push('\n');
        }
        text
    }

    const REMARK: &str = "icx: remark: note that use of '-g' without any optimization-level \
                          option will turn off most compiler optimizations similar to use of \
                          '-O0'; use '-Rno-debug-disables-optimization' to disable this remark \
                          [-Rdebug-disables-optimization]\n";

    #[test]
    fn version_queries_name_intel_and_bcc() {
        let directory = Directory::new();
        let output = directory.icx(&[LINUX, "--version"]);
        succeeds(&output);
        let text = stdout(&output);
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(
                "Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001) (bcc 0.1.0, an \
                 icx-compatible C compiler; not icx)"
            )
        );
        assert_eq!(lines.next(), Some("Target: x86_64-unknown-linux-gnu"));
        assert_eq!(lines.next(), Some("Thread model: posix"));
        assert!(
            lines
                .next()
                .is_some_and(|line| line.starts_with("InstalledDir: "))
        );
        // Meson takes a banner that mentions Clang for Clang's.
        assert!(!text.to_ascii_lowercase().contains("clang"), "{text}");
        let output = directory.icx(&[LINUX, "-dumpversion"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "23.1.1\n");
        let output = directory.icx(&[LINUX, "-dumpmachine"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "x86_64-unknown-linux-gnu\n");
        let output = directory.icx(&[LINUX, "-v"]);
        succeeds(&output);
        assert!(stderr(&output).starts_with("Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0"));
    }

    /// Intel's identity, `-O2`, the `fast` model without finite math, and
    /// code at a fixed address, and what the options change of them.
    #[test]
    fn identity_and_defaults_are_intels() {
        let directory = Directory::new();
        let macros = linux_macros(&directory, &[]);
        for definition in [
            "#define __INTEL_LLVM_COMPILER 20260200",
            "#define __INTEL_CLANG_COMPILER 20260200",
            "#define __clang__ 1",
            "#define __clang_major__ 23",
            "#define __clang_patchlevel__ 1",
            "#define __clang_version__ \"23.1.1 (icx 2026.2.0.20261001)\"",
            "#define __VERSION__ \"Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 \
             (2026.2.0.20261001)\"",
            "#define __GNUC__ 4",
            "#define __DPCPP__ 1",
            "#define __dpcpp_major__ 2026",
            "#define __dpcpp_minor__ 2",
            "#define __dpcpp_patchlevel__ 0",
            "#define __AMX_SUPPORTED__ 1",
            "#define __I__ 1j",
            "#define __LONG_DOUBLE_SIZE__ 80",
            "#define __OPTIMIZE__ 1",
            "#define __NO_MATH_ERRNO__ 1",
            "#define __FINITE_MATH_ONLY__ 0",
        ] {
            assert!(defines(&macros, definition), "{definition}\n{macros}");
        }
        for name in [
            "__INTEL_COMPILER",
            "__FAST_MATH__",
            "__NO_INLINE__",
            "__pic__",
            "__PIE__",
            "_OPENMP",
        ] {
            assert!(!names(&macros, name), "{name}");
        }
        // `-O0`, or a debug option without an `-O` option, turns
        // optimization off.
        for args in [&["-O0"][..], &["-g"], &["-gdwarf-4"], &["-debug", "full"]] {
            let macros = linux_macros(&directory, args);
            assert!(defines(&macros, "#define __NO_INLINE__ 1"), "{args:?}");
            assert!(!names(&macros, "__OPTIMIZE__"), "{args:?}");
        }
        let macros = linux_macros(&directory, &["-g", "-O2"]);
        assert!(defines(&macros, "#define __OPTIMIZE__ 1"));
        let macros = linux_macros(&directory, &["-Os"]);
        assert!(defines(&macros, "#define __OPTIMIZE_SIZE__ 1"));
        for model in [
            "-fp-model=precise",
            "-fp-model=consistent",
            "-fno-fast-math",
        ] {
            let macros = linux_macros(&directory, &[model]);
            assert!(!names(&macros, "__NO_MATH_ERRNO__"), "{model}");
        }
        for args in [&["-fp-model=fast=2"][..], &["-ffast-math"], &["-Ofast"]] {
            let macros = linux_macros(&directory, args);
            assert!(defines(&macros, "#define __FAST_MATH__ 1"), "{args:?}");
            assert!(
                defines(&macros, "#define __FINITE_MATH_ONLY__ 0"),
                "{args:?}"
            );
        }
        let macros = linux_macros(&directory, &["-fPIC"]);
        assert!(defines(&macros, "#define __PIC__ 2"));
        assert!(!names(&macros, "__PIE__"));
        let macros = linux_macros(&directory, &["-fPIE"]);
        assert!(defines(&macros, "#define __PIE__ 2"));
        let macros = linux_macros(&directory, &["-xCORE-AVX2"]);
        for definition in [
            "#define __AVX2__ 1",
            "#define __FMA__ 1",
            "#define __PCLMUL__ 1",
        ] {
            assert!(defines(&macros, definition), "{definition}");
        }
        let macros = linux_macros(&directory, &["-xsse4.2"]);
        assert!(defines(&macros, "#define __SSE4_2__ 1"));
        assert!(!names(&macros, "__AVX__"));
        // Further code paths leave the baseline as it is.
        let macros = linux_macros(&directory, &["-axCORE-AVX2,SKYLAKE-AVX512"]);
        assert!(!names(&macros, "__AVX__"));
        let macros = linux_macros(&directory, &["-qopenmp-simd"]);
        assert!(defines(&macros, "#define __INTEL_HOST_OPENMP 1"));
        assert!(!names(&macros, "_OPENMP"));
        let macros = linux_macros(&directory, &["-strict-ansi"]);
        assert!(defines(&macros, "#define __STRICT_ANSI__ 1"));
        let macros = linux_macros(&directory, &["-fstack-security-check"]);
        assert!(defines(&macros, "#define __SSP_STRONG__ 2"));
    }

    /// The classic `icc` options icx dropped: one warning before anything
    /// else, listing each as spelled, which `-diag-disable=10430` silences.
    #[test]
    fn classic_options_draw_warning_10430() {
        let directory = Directory::new();
        reports(
            &directory.icx(&["-ip", "-static-intel", "-w2", "-wd10441", "-c", "e.c"]),
            0,
            &(classic_warning(&["-ip", "-w2", "-wd10441"])
                + "icx: warning: argument unused during compilation: '-static-intel' \
                   [-Wunused-command-line-argument]\n"),
        );
        reports(
            &directory.icx(&[
                "-tcheck",
                "foo",
                "-Winline",
                "-par-num-threads=4",
                "-c",
                "e.c",
            ]),
            0,
            &classic_warning(&["-tcheck foo", "-Winline", "-par-num-threads=4"]),
        );
        reports(
            &directory.icx(&["-ip", "-fbogus", "-c", "e.c"]),
            1,
            &(classic_warning(&["-ip"]) + "icx: error: unknown argument: '-fbogus'\n"),
        );
        reports(
            &directory.icx(&["-ip"]),
            1,
            &(classic_warning(&["-ip"]) + "icx: error: no input files\n"),
        );
        let output = directory.icx(&["-ip", "--version"]);
        reports(&output, 0, &classic_warning(&["-ip"]));
        assert!(stdout(&output).starts_with("Intel(R) oneAPI"));
        reports(
            &directory.icx(&["-diag-disable=10430", "-ip", "-c", "e.c"]),
            0,
            "",
        );
        // A value the table does not take is no classic option.
        reports(
            &directory.icx(&["-w=foo", "-c", "e.c"]),
            1,
            "icx: error: unknown argument: '-w=foo'\n",
        );
    }

    /// The remark about a debug option without an `-O` option, and the
    /// errors and warnings of Intel's options, in icx's words and order.
    #[test]
    fn driver_messages_are_intels() {
        let directory = Directory::new();
        for (args, status, message) in [
            (&["-g", "-c", "e.c"][..], 0, REMARK.to_owned()),
            (&["-g", "-O1", "-c", "e.c"], 0, String::new()),
            (
                &["-Rno-debug-disables-optimization", "-g", "-c", "e.c"],
                0,
                String::new(),
            ),
            (
                &["-g", "-fbogus", "-c", "e.c"],
                1,
                format!("icx: error: unknown argument: '-fbogus'\n{REMARK}"),
            ),
            (
                &["-gdwarf-4", "-ggdb", "-c", "e.c"],
                0,
                REMARK.replace("'-g'", "'-ggdb'"),
            ),
            (
                &["-debug=bogus", "-c", "e.c"],
                1,
                REMARK.replace("'-g'", "'-debug'")
                    + "icx: error: unsupported argument 'bogus' to option '-debug='\n",
            ),
            (
                &["-axbogus", "-fbogus", "-c", "e.c"],
                1,
                "icx: error: unknown argument: '-fbogus'\nicx: error: unknown target CPU 'bogus' \
                 for option 'ax'\n"
                    .to_owned(),
            ),
            (
                &["-mauto-arch", "-c", "e.c"],
                1,
                "icx: error: unknown target CPU '' for option 'mauto-arch'\n".to_owned(),
            ),
            (
                &["-fintelfpga", "-fbogus", "-c", "e.c"],
                1,
                "icx: error: option '-fintelfpga' is not supported and has been removed from the \
                 compiler. Please see the compiler documentation for more details\nicx: error: \
                 unknown argument: '-fbogus'\n"
                    .to_owned(),
            ),
            (
                &["-fp-model=bogus", "-c", "e.c"],
                1,
                "icx: error: unsupported argument 'bogus' to option '-ffp-model='\n".to_owned(),
            ),
            (
                &["-fp-model", "-c", "e.c"],
                1,
                "icx: error: unsupported argument '-c' to option '-ffp-model='\n".to_owned(),
            ),
            (
                &["-fp-model", "source", "-c", "e.c"],
                1,
                "icx: error: unsupported argument 'source' to option '-ffp-model='\n".to_owned(),
            ),
            // icx 2026.2.0 has none of the classic compiler's other models,
            // in either spelling.
            (
                &["-fp-model=source", "-c", "e.c"],
                1,
                "icx: error: unsupported argument 'source' to option '-ffp-model='\n".to_owned(),
            ),
            (
                &["-fp-model=except", "-c", "e.c"],
                1,
                "icx: error: unsupported argument 'except' to option '-ffp-model='\n".to_owned(),
            ),
            (
                &["-fp-model=precise", "-fp-model=fast", "-c", "e.c"],
                0,
                "icx: warning: overriding '-ffp-model=precise' option with '-ffp-model=fast' \
                 [-Woverriding-option]\n"
                    .to_owned(),
            ),
            (
                &["-Ofast", "-O2", "-c", "e.c"],
                0,
                "icx: warning: argument '-Ofast' is deprecated; use '-O3' to enable only \
                 conforming optimizations, or consult the documentation for the same behavior \
                 [-Wdeprecated-ofast]\nicx: warning: '-Ofast' sets complex range to \"basic\" \
                 overriding the setting of \"promoted\" that was implied by '-ffp-model=fast' \
                 [-Woverriding-complex-range]\n"
                    .to_owned(),
            ),
            (
                &[
                    "-ipo",
                    "-no-ipo",
                    "-qopt-report",
                    "-qopt-zmm-usage=high",
                    "-unroll=4",
                    "-fimf-precision=high",
                    "-ftz",
                    "-traceback",
                    "-diag-disable=10441",
                    "-Wno-recommended-option",
                    "-c",
                    "e.c",
                ],
                0,
                String::new(),
            ),
        ] {
            reports(&directory.icx(args), status, &message);
        }
    }

    /// Options that would change the program in ways bcc cannot follow:
    /// OpenMP, SYCL, Intel's libraries and the strict model.
    #[test]
    fn options_bcc_cannot_follow_are_refused() {
        let directory = Directory::new();
        let not_implemented = "bcc does not implement it, and ignoring it could change the program";
        for (args, message) in [
            (
                &["-qopenmp", "-c", "e.c"][..],
                format!("icx: error: '-qopenmp' is not supported: {not_implemented}\n"),
            ),
            (
                &["-fopenmp", "-c", "e.c"],
                format!(
                    "icx: warning: use of '-qopenmp' recommended over '-fopenmp' \
                     [-Wrecommended-option]\nicx: error: '-fopenmp' is not supported: \
                     {not_implemented}\n"
                ),
            ),
            (
                &["-qmkl", "-c", "e.c"],
                "icx: error: '-qmkl' is not supported: bcc has none of Intel's libraries\n"
                    .to_owned(),
            ),
            (
                &["-fsycl", "-c", "e.c"],
                "icx: error: '-fsycl' is not supported: bcc compiles C, not SYCL or device code\n"
                    .to_owned(),
            ),
            (
                &["-fp-model=strict", "-c", "e.c"],
                format!("icx: error: '-ffp-model=strict' is not supported: {not_implemented}\n"),
            ),
        ] {
            reports(&directory.icx(args), 1, &message);
        }
    }

    /// The type of the ELF file at `path`: 2 for an executable at a fixed
    /// address, 3 for a position-independent one.
    #[cfg(target_os = "linux")]
    fn elf_type(path: &Path) -> u16 {
        let bytes = std::fs::read(path).unwrap();
        u16::from_le_bytes([bytes[16], bytes[17]])
    }

    /// icx links executables at a fixed address unless `-pie` asks
    /// otherwise, and the math library whether or not `-lm` asks for it.
    #[cfg(target_os = "linux")]
    #[test]
    fn links_fixed_executables_with_the_math_library() {
        let directory = Directory::new();
        directory.write(
            "sqrt.c",
            "#include <math.h>\n#include <stdio.h>\nint main(int argc, char **argv) {\n  \
             (void)argv;\n  printf(\"%.3f\\n\", sqrt((double)argc + 1.0));\n  return 0;\n}\n",
        );
        succeeds(&directory.icx(&["sqrt.c", "-o", "fixed"]));
        assert_eq!(elf_type(&directory.0.join("fixed")), 2);
        let run = directory.run(directory.0.join("fixed"), &[]);
        succeeds(&run);
        assert_eq!(stdout(&run), "1.414\n");
        succeeds(&directory.icx(&["-fPIE", "sqrt.c", "-o", "pie-code"]));
        assert_eq!(elf_type(&directory.0.join("pie-code")), 2);
        succeeds(&directory.icx(&["-fPIE", "-pie", "sqrt.c", "-o", "pie"]));
        assert_eq!(elf_type(&directory.0.join("pie")), 3);
        succeeds(&directory.icx(&["-shared", "-fPIC", "e.c", "-o", "e.so"]));
        assert_eq!(elf_type(&directory.0.join("e.so")), 3);
        // `-static-intel` asks only how to link Intel's libraries.
        succeeds(&directory.icx(&["-static-intel", "sqrt.c", "-o", "intel"]));
    }

    /// A program that says whether `DBL_MIN / 4`, computed in another unit,
    /// survives.
    const SUBNORMAL_MAIN: &str = "#include <float.h>\n#include <stdio.h>\ndouble \
                                  quarter(double);\nint main(void) {\n  volatile double y = \
                                  quarter(DBL_MIN);\n  printf(\"%s\\n\", y != 0 ? \"kept\" : \
                                  \"flushed\");\n  return 0;\n}\n";
    const SUBNORMAL_LIBRARY: &str =
        "double quarter(double x) { volatile double v = x; return v / 4; }\n";

    /// icx flushes subnormals from an optimized `main` compiled under a
    /// model that flushes, whatever the link command says, and links
    /// `crtfastmath.o` only where the link command's own options ask for it.
    /// Each expectation is what icx 2026.2.0's programs print.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn subnormals_flush_where_icx_flushes_them() {
        let directory = Directory::new();
        directory.write("main.c", SUBNORMAL_MAIN);
        directory.write("lib.c", SUBNORMAL_LIBRARY);
        let mut failures = Vec::new();
        let mut check = |steps: &[&[&str]], expected: &str| {
            for step in steps {
                succeeds(&directory.icx(step));
            }
            let run = directory.run(directory.0.join("a.out"), &[]);
            succeeds(&run);
            if stdout(&run) != format!("{expected}\n") {
                failures.push(format!("{steps:?}: {}", stdout(&run)));
            }
            drop(std::fs::remove_file(directory.0.join("a.out")));
        };
        for (options, expected) in [
            (&[][..], "flushed"),
            (&["-O0"], "kept"),
            (&["-O2"], "flushed"),
            (&["-g"], "kept"),
            (&["-g", "-O2"], "flushed"),
            (&["-no-ftz"], "kept"),
            (&["-ftz", "-no-ftz"], "kept"),
            (&["-fp-model=precise"], "kept"),
            (&["-fp-model=precise", "-ftz"], "flushed"),
            (&["-ftz", "-fp-model=precise"], "kept"),
            (&["-O0", "-ftz"], "kept"),
            (&["-fno-fast-math"], "kept"),
            // `-ffast-math` links `crtfastmath.o`, as Clang's driver does.
            (&["-O0", "-ffast-math"], "flushed"),
            (&["-Ofast", "-O0"], "flushed"),
            (&["-fdenormal-fp-math=ieee"], "kept"),
        ] {
            let mut step = options.to_vec();
            step.extend_from_slice(&["main.c", "lib.c"]);
            check(&[step.as_slice()], expected);
        }
        // Separate compilation and link, as build systems run them: the
        // compile options decide the flush, and the link adds the runtime
        // only for its own options.
        for (compile, link, expected) in [
            (&[][..], &[][..], "flushed"),
            (&["-O0"], &[], "kept"),
            (&["-g"], &[], "kept"),
            (&["-O0", "-g"], &["-O0", "-g"], "kept"),
            (&["-fp-model=precise"], &[], "kept"),
            (&["-no-ftz"], &[], "kept"),
            (&[], &["-fp-model=precise"], "flushed"),
            (&["-O0"], &["-ftz"], "kept"),
            (&["-O0"], &["-ffast-math"], "flushed"),
            (&["-fp-model=precise"], &["-Ofast"], "flushed"),
        ] {
            let mut first = compile.to_vec();
            first.extend_from_slice(&["-c", "main.c", "lib.c"]);
            let mut second = link.to_vec();
            second.extend_from_slice(&["main.o", "lib.o"]);
            check(&[first.as_slice(), second.as_slice()], expected);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// `icx-cl`: its logo first, whatever the command line, the identity
    /// with a 64-bit `long double`, `/O2` and `/fp:fast`, Intel's `/Q`
    /// options, and the static libraries `CMake` makes through it.
    #[test]
    fn icx_cl_has_intels_logo_identity_and_defaults() {
        let directory = Directory::new();
        let logo = "Intel(R) oneAPI DPC++/C++ Compiler for applications running on Intel(R) 64, \
                    Version 2026.2.0 Build 20261001\nbcc 0.1.0, an icx-cl-compatible C compiler; \
                    not an Intel product.\n\n";
        reports(
            &directory.bcc("--icx-cl-mode", &["/nologo", "/c", "e.c"]),
            0,
            logo,
        );
        assert!(directory.0.join("e.obj").exists());
        let output = directory.bcc("--icx-cl-mode", &["--version"]);
        reports(&output, 0, logo);
        assert!(stdout(&output).starts_with(
            "Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001) (bcc 0.1.0, an \
             icx-compatible C compiler; not icx)\nTarget: x86_64-pc-windows-msvc\n"
        ));
        let macros = |args: &[&str]| {
            let mut all = args.to_vec();
            all.extend_from_slice(&["/clang:-dM", "/E", "e.c"]);
            let output = directory.bcc("--icx-cl-mode", &all);
            succeeds(&output);
            stdout(&output)
        };
        let defaults = macros(&[]);
        for definition in [
            "#define __INTEL_LLVM_COMPILER 20260200",
            "#define _MSC_VER 1933",
            "#define __LONG_DOUBLE_SIZE__ 64",
            "#define _ISO_VOLATILE 1",
            "#define _UCRT_NOISY_NAN 1",
            "#define _M_FP_FAST 1",
            "#define _M_FP_CONTRACT 1",
            "#define __NO_MATH_ERRNO__ 1",
            "#define __OPTIMIZE__ 1",
        ] {
            assert!(defines(&defaults, definition), "{definition}\n{defaults}");
        }
        assert!(!names(&defaults, "_M_FP_PRECISE"));
        let precise = macros(&["/fp:precise", "/Od"]);
        assert!(defines(&precise, "#define _M_FP_PRECISE 1"));
        assert!(!names(&precise, "__NO_MATH_ERRNO__"));
        assert!(!names(&precise, "__OPTIMIZE__"));
        assert!(defines(&macros(&["/QxCORE-AVX2"]), "#define __AVX2__ 1"));
        reports(
            &directory.bcc("--icx-cl-mode", &["/Zi", "/c", "e.c"]),
            0,
            &format!(
                "{logo}icx-cl: remark: note that use of '/Zi' without any optimization-level \
                 option will turn off most compiler optimizations similar to use of '/Od'; use \
                 '-Rno-debug-disables-optimization' to disable this remark \
                 [-Rdebug-disables-optimization]\n"
            ),
        );
        reports(
            &directory.bcc("--icx-cl-mode", &["/Qopenmp", "/c", "e.c"]),
            1,
            &format!(
                "{logo}icx-cl: error: '/Qopenmp' is not supported: bcc does not implement it, and \
                 ignoring it could change the program\n"
            ),
        );
        // CMake's rules for icx-cl.
        succeeds(&directory.bcc(
            "--icx-cl-mode",
            &[
                "/nologo",
                "-Rno-debug-disables-optimization",
                "-QMD",
                "-QMT",
                "e.obj",
                "-QMF",
                "e.obj.d",
                "/Foe.obj",
                "-c",
                "e.c",
            ],
        ));
        assert!(directory.0.join("e.obj.d").exists());
        succeeds(&directory.bcc(
            "--icx-cl-mode",
            &[
                "/nologo",
                "e.obj",
                "-fuse-ld=llvm-lib",
                "-o",
                "e.lib",
                "/Qoption,link,/machine:x64",
            ],
        ));
        let archive = std::fs::read(directory.0.join("e.lib")).unwrap();
        assert!(archive.starts_with(b"!<arch>\n"));
        // `-o` names the object under `/c`, where the last of it and `/Fo`
        // wins.
        succeeds(&directory.bcc("--icx-cl-mode", &["/c", "e.c", "-o", "foo.obj"]));
        assert!(directory.0.join("foo.obj").exists());
        succeeds(&directory.bcc(
            "--icx-cl-mode",
            &["-o", "bar.obj", "/Fobaz.obj", "/c", "e.c"],
        ));
        assert!(directory.0.join("baz.obj").exists());
        assert!(!directory.0.join("bar.obj").exists());
        // An optimized `main` sets MXCSR's flush bits under `/fp:fast` and
        // `/Qftz`, as icx-cl's does.
        for (options, flushes) in [
            (&[][..], true),
            (&["/Od"], false),
            (&["/Zi"], false),
            (&["/fp:precise"], false),
            (&["/fp:precise", "/Qftz"], true),
            (&["/Qftz-"], false),
        ] {
            let mut all = options.to_vec();
            all.extend_from_slice(&["/c", "/FA", "/Fae.asm", "e.c"]);
            succeeds(&directory.bcc("--icx-cl-mode", &all));
            let listing = std::fs::read_to_string(directory.0.join("e.asm")).unwrap();
            assert_eq!(
                listing.contains("ldmxcsr"),
                flushes,
                "{options:?}\n{listing}"
            );
        }
    }

    /// icx-cl's `/O3` and `-O3` are Intel's level 3, which clang-cl does not
    /// have: "Possible values are 1, 2, or 3", and "The last O option
    /// specified on the command line takes precedence over any others"
    /// (Intel oneAPI DPC++/C++ Compiler Developer Guide and Reference
    /// 2026.2, "O"). They draw no warning.
    #[test]
    fn icx_cl_reads_o3_as_intels_level_3() {
        let directory = Directory::new();
        let logo = "Intel(R) oneAPI DPC++/C++ Compiler for applications running on Intel(R) 64, \
                    Version 2026.2.0 Build 20261001\nbcc 0.1.0, an icx-cl-compatible C compiler; \
                    not an Intel product.\n\n";
        for (options, optimized, size) in [
            (&["/O3"][..], true, false),
            (&["-O3"], true, false),
            (&["/Zi", "/O3"], true, false),
            (&["/O3", "/Od"], false, false),
            (&["/Od", "/O3"], true, false),
            (&["/O3", "/O1"], true, true),
            (&["/O1", "/O3"], true, false),
            (&["/O2s", "/O3"], true, false),
            (&["/O3", "/Os"], true, true),
        ] {
            let mut all = options.to_vec();
            all.extend_from_slice(&["/clang:-dM", "/E", "e.c"]);
            let output = directory.bcc("--icx-cl-mode", &all);
            reports(&output, 0, logo);
            let macros = stdout(&output);
            assert_eq!(names(&macros, "__OPTIMIZE__"), optimized, "{options:?}");
            assert_eq!(names(&macros, "__OPTIMIZE_SIZE__"), size, "{options:?}");
            assert_eq!(names(&macros, "__NO_INLINE__"), !optimized, "{options:?}");
        }
        // clang-cl's `/O3` sets nothing, with its warning.
        let output = directory.bcc("--clang-cl-mode", &["/O3", "/clang:-dM", "/E", "e.c"]);
        reports(
            &output,
            0,
            "clang-cl: warning: argument unused during compilation: '/O3' \
             [-Wunused-command-line-argument]\n",
        );
        assert!(!names(&stdout(&output), "__OPTIMIZE__"));
    }

    /// The executable names: `icx` and `icx-cc` are icx (`icx` is icx-cl on
    /// Windows, as Intel's own is), `icx-cl` is icx-cl, and `icc` is not
    /// icx.
    #[cfg(unix)]
    #[test]
    fn executable_names_select_the_personalities() {
        let directory = Directory::new();
        for name in ["icx", "icx-cc", "bcc-icx", "icx-2026"] {
            let program = directory.0.join(name);
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &program).unwrap();
            let output = directory.run(&program, &["-dumpversion"]);
            succeeds(&output);
            assert_eq!(stdout(&output), "23.1.1\n", "{name}");
            reports(
                &directory.run(&program, &["-fbogus", "-c", "e.c"]),
                1,
                "icx: error: unknown argument: '-fbogus'\n",
            );
        }
        let icx_cl = directory.0.join("icx-cl");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &icx_cl).unwrap();
        let output = directory.run(&icx_cl, &["/c", "e.c"]);
        succeeds(&output);
        assert!(stderr(&output).starts_with("Intel(R) oneAPI DPC++/C++ Compiler for applications"));
        let icc = directory.0.join("icc");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &icc).unwrap();
        let output = directory.run(&icc, &["-dumpversion"]);
        assert!(!output.status.success() || stdout(&output) != "23.1.1\n");
    }

    /// Compares bcc with the `icx` that `BCC_ICX` names, byte for byte after
    /// each compiler's install directory, bcc's banner note and the
    /// configuration file icx's banner names are replaced. The cases are
    /// those the comparison in `driver-modes.md` found identical.
    #[test]
    fn icx_parity_with_a_real_icx() {
        let Some(icx) = std::env::var_os("BCC_ICX") else {
            eprintln!("skipping: BCC_ICX names no icx binary");
            return;
        };
        let cases: &[&[&str]] = &[
            &["--version"],
            &["-dumpversion"],
            &["-dumpmachine"],
            &[],
            &["-c", "missing.c"],
            &["-fbogus", "-fother", "-c", "e.c"],
            &["-qbogus", "-c", "e.c"],
            &["-axbogus", "-fbogus", "-c", "e.c"],
            &["-fintelfpga", "-fbogus", "-c", "e.c"],
            &["-ip", "-static-intel", "-w2", "-wd10441", "-c", "e.c"],
            &["-tcheck", "foo", "-c", "e.c"],
            &["-ip", "--version"],
            &["-diag-disable=10430", "-ip", "-c", "e.c"],
            &["-ipo", "-qopt-report", "-unroll=4", "-ftz", "-c", "e.c"],
            &["-fp-model=bogus", "-c", "e.c"],
            &["-fp-model", "source", "-c", "e.c"],
            &["-fp-model=precise", "-fp-model=fast", "-c", "e.c"],
            &["-Ofast", "-O2", "-c", "e.c"],
            &["-g", "-c", "e.c"],
            &["-g", "-fbogus", "-c", "e.c"],
            &["-debug=bogus", "-c", "e.c"],
            &["-Wno-recommended-option", "-Wintel-compat", "-c", "e.c"],
            &["hello.c", "-o", "out"],
            &["sqrt.c", "-o", "out"],
            &["main.c", "lib.c", "-o", "out"],
            &["-O0", "main.c", "lib.c", "-o", "out"],
            &["-g", "main.c", "lib.c", "-o", "out"],
            &["-no-ftz", "main.c", "lib.c", "-o", "out"],
            &["-fp-model=precise", "main.c", "lib.c", "-o", "out"],
            &["-fp-model=source", "-c", "e.c"],
        ];
        let normalize = |text: &[u8], program: &Path, directory: &Path| {
            let installed = std::fs::canonicalize(program)
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let mut text = String::from_utf8_lossy(text)
                .replace(&directory.display().to_string(), "<cwd>")
                .replace(
                    &installed.join("compiler").display().to_string(),
                    "<installed>",
                )
                .replace(&installed.display().to_string(), "<installed>")
                .replace(" (bcc 0.1.0, an icx-compatible C compiler; not icx)", "");
            if let Some(start) = text.find("Configuration file: ") {
                let end = text[start..]
                    .find('\n')
                    .map_or(text.len(), |end| start + end + 1);
                text.replace_range(start..end, "");
            }
            text
        };
        let mut failures = Vec::new();
        for args in cases {
            let mut outcomes = Vec::new();
            for real in [true, false] {
                let directory = Directory::new();
                directory.write(
                    "hello.c",
                    "#include <stdio.h>\nint main(void) { printf(\"hi\\n\"); return 0; }\n",
                );
                directory.write(
                    "sqrt.c",
                    "#include <math.h>\n#include <stdio.h>\nint main(int argc, char **argv) {\n  \
                     (void)argv;\n  printf(\"%.3f\\n\", sqrt((double)argc + 1.0));\n  return \
                     0;\n}\n",
                );
                directory.write("main.c", SUBNORMAL_MAIN);
                directory.write("lib.c", SUBNORMAL_LIBRARY);
                let (program, mut all): (PathBuf, Vec<&str>) = if real {
                    (PathBuf::from(&icx), Vec::new())
                } else {
                    (
                        PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust")),
                        vec!["--icx-mode"],
                    )
                };
                all.extend_from_slice(args);
                let output = directory.run(&program, &all);
                let mut ran = String::new();
                if output.status.success() && directory.0.join("out").exists() {
                    let run = directory.run(directory.0.join("out"), &[]);
                    ran = format!("{:?} {}", run.status.code(), stdout(&run));
                }
                outcomes.push((
                    output.status.code(),
                    normalize(&output.stdout, &program, &directory.0),
                    normalize(&output.stderr, &program, &directory.0),
                    ran,
                ));
            }
            if outcomes[0] != outcomes[1] {
                failures.push(format!(
                    "icx {}\nicx: {:?}\nbcc: {:?}",
                    args.join(" "),
                    outcomes[0],
                    outcomes[1]
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
