//! Warning options under each driver personality: what GCC 14, Clang 23,
//! clang-cl and TCC 0.9.27 say about `-W`, `-Wno-`, `-Werror`,
//! `-Werror=`, `-Wno-error=`, `-Wfatal-errors`, `-Weverything`, `-w` and
//! `-Wp,`, known, unknown, misspelled and empty, with and without another
//! diagnostic. Each expected text is the real compiler's stderr and exit
//! status for the same command line, recorded on 2026-11-19
//! (`docs/design/driver-modes.md`, "Warning options").
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    /// A fresh working directory with the sources the cases compile,
    /// removed afterwards: `clean.c` draws no diagnostic, `warn.c` a
    /// `#warning`, `err.c` an error inside a function, and `unused.c` an
    /// unused variable under `-Wall`.
    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-warning-options-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            for (name, text) in [
                ("clean.c", "int main(void) { return 0; }\n"),
                ("warn.c", "#warning hi\nint main(void) { return 0; }\n"),
                ("err.c", "int main(void) { return y; }\n"),
                ("unused.c", "int main(void) { int x; return 0; }\n"),
                ("macro.c", "X\n"),
            ] {
                std::fs::write(directory.join(name), text).unwrap();
            }
            Self(directory)
        }

        /// Runs bcc in `mode` with `args`, returning its stderr and exit
        /// status.
        fn run(&self, mode: &str, args: &[&str]) -> (String, i32) {
            let output = self.output(mode, args);
            (
                String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
                output.status.code().unwrap_or(-1),
            )
        }

        fn output(&self, mode: &str, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .arg(mode)
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

        /// Checks each case: its arguments, and the stderr and exit status
        /// the compiler gives for them.
        fn check(&self, mode: &str, cases: &[(&[&str], &str, i32)]) {
            for &(args, expected, status) in cases {
                assert_eq!(
                    self.run(mode, args),
                    (expected.to_owned(), status),
                    "{mode} {args:?}"
                );
            }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    const GCC_WARNING: &str =
        "warn.c:1:2: warning: #warning hi [-Wcpp]\n    1 | #warning hi\n      |  ^~~~~~~\n";
    const GCC_WARNING_ERROR: &str =
        "warn.c:1:2: error: #warning hi [-Werror=cpp]\n    1 | #warning hi\n      |  ^~~~~~~\n";
    const GCC_ERROR: &str =
        "err.c: In function 'main':\nerr.c:1:25: error: 'y' undeclared (first use in this \
         function)\n    1 | int main(void) { return y; }\n      |                         ^\n";
    const CLANG_WARNING: &str =
        "warn.c:1:2: warning: hi [-W#warnings]\n    1 | #warning hi\n      |  ^\n";

    /// GCC's driver refuses an empty `-Wno-`, a joined option without its
    /// value or with a bad one, the `no-` form of a joined option, and an
    /// unknown warning, Clang's included, before anything is compiled.
    #[test]
    fn gcc_refuses_malformed_warning_options() {
        let directory = Directory::new();
        directory.check(
            "--gcc-mode",
            &[
                (
                    &["-c", "-Wno-", "warn.c"],
                    "gcc: error: unrecognized command-line option '-Wno-'\n",
                    1,
                ),
                (
                    &["-c", "-Werror=", "warn.c"],
                    "gcc: error: missing argument to '-Werror='\n",
                    1,
                ),
                (
                    &["-c", "-Wno-error=", "warn.c"],
                    "gcc: error: missing argument to '-Wno-error='\n",
                    1,
                ),
                (
                    &["-c", "-Wformat=", "clean.c"],
                    "gcc: error: missing argument to '-Wformat='\n",
                    1,
                ),
                (
                    &["-c", "-Wformat=3", "clean.c"],
                    "gcc: error: argument to '-Wformat=' is not between 0 and 2\n",
                    1,
                ),
                (
                    &["-c", "-Wformat=x", "clean.c"],
                    "gcc: error: argument to '-Wformat=' should be a non-negative integer\n",
                    1,
                ),
                (
                    &["-c", "-Wunused-const-variable=3", "clean.c"],
                    "gcc: error: argument to '-Wunused-const-variable=' is not between 0 and 2\n",
                    1,
                ),
                (
                    &["-c", "-Wnormalized=bogus", "clean.c"],
                    "gcc: error: argument 'bogus' to '-Wnormalized' not recognized\ngcc: note: \
                     valid arguments to '-Wnormalized=' are: id nfc nfkc none\n",
                    1,
                ),
                (
                    &["-c", "-Wno-format=2", "clean.c"],
                    "gcc: error: unrecognized command-line option '-Wno-format=2'\n",
                    1,
                ),
                (
                    &["-c", "-Weverything", "clean.c"],
                    "gcc: error: unrecognized command-line option '-Weverything'\n",
                    1,
                ),
                // The missing argument comes as GCC reads it; the
                // unrecognized options after the whole command line.
                (
                    &["-c", "-Wfoo", "-Wno-", "-Werror=", "clean.c"],
                    "gcc: error: missing argument to '-Werror='\ngcc: error: unrecognized \
                     command-line option '-Wfoo'\ngcc: error: unrecognized command-line option \
                     '-Wno-'\n",
                    1,
                ),
                // `-Wformat=02` is level 2, and `-Wa,` with nothing passes
                // nothing to the assembler.
                (&["-c", "-Wformat=02", "clean.c"], "", 0),
                (&["-c", "-Wa,", "clean.c"], "", 0),
            ],
        );
    }

    /// GCC's compiler refuses a `-Werror=` or `-Wno-error=` naming no
    /// warning it has (a `no-` form is none), and an empty `-Wp,` operand,
    /// once for each C source, compiling nothing.
    #[test]
    fn gcc_compiler_refuses_unknown_werror_groups() {
        let directory = Directory::new();
        directory.check(
            "--gcc-mode",
            &[
                (
                    &["-c", "-Werror=foo", "warn.c"],
                    "cc1: error: '-Werror=foo': no option '-Wfoo'\n",
                    1,
                ),
                (
                    &["-c", "-Wno-error=foo", "-Werror", "warn.c"],
                    "cc1: error: '-Wno-error=foo': no option '-Wfoo'\n",
                    1,
                ),
                (
                    &["-c", "-Werror=no-shadow", "warn.c"],
                    "cc1: error: '-Werror=no-shadow': no option '-Wno-shadow'\n",
                    1,
                ),
                // Clang's name for the group is no GCC option.
                (
                    &["-c", "-Werror=#warnings", "warn.c"],
                    "cc1: error: '-Werror=#warnings': no option '-W#warnings'\n",
                    1,
                ),
                (
                    &["-c", "-Werror=foo", "-Werror=bar", "clean.c", "warn.c"],
                    "cc1: error: '-Werror=foo': no option '-Wfoo'\ncc1: error: '-Werror=bar': no \
                     option '-Wbar'\ncc1: error: '-Werror=foo': no option '-Wfoo'\ncc1: error: \
                     '-Werror=bar': no option '-Wbar'\n",
                    1,
                ),
                (
                    &["-c", "-Wp,", "clean.c"],
                    "cc1: error: too many filenames given; type 'cc1 --help' for usage\n",
                    1,
                ),
                (&["-c", "-Werror=all", "clean.c"], "", 0),
                (&["-c", "-Werror=format=2", "clean.c"], "", 0),
            ],
        );
    }

    /// GCC ignores an unknown `-Wno-` option, Clang's names included, and
    /// notes it after the diagnostics of a compilation that has any, last
    /// first and at top level; under `-w` there are none.
    #[test]
    fn gcc_notes_ignored_wno_options_after_other_diagnostics() {
        let note = |option: &str| {
            format!(
                "cc1: note: unrecognized command-line option '{option}' may have been intended to \
                 silence earlier diagnostics\n"
            )
        };
        let directory = Directory::new();
        let cases: &[(&[&str], String, i32)] = &[
            (&["-c", "-Wno-foo", "clean.c"], String::new(), 0),
            (
                &["-c", "-Wno-foo", "warn.c"],
                format!("{GCC_WARNING}{}", note("-Wno-foo")),
                0,
            ),
            (
                &["-c", "-Wno-foo", "err.c"],
                format!("{GCC_ERROR}At top level:\n{}", note("-Wno-foo")),
                1,
            ),
            (
                &["-c", "-Wno-foo", "-Wno-bar", "warn.c"],
                format!("{GCC_WARNING}{}{}", note("-Wno-bar"), note("-Wno-foo")),
                0,
            ),
            // `#warnings` is Clang's name for GCC's `cpp`.
            (
                &["-c", "-Wno-#warnings", "warn.c"],
                format!("{GCC_WARNING}{}", note("-Wno-#warnings")),
                0,
            ),
            (
                &["-c", "-Wno-foo", "-Werror", "warn.c"],
                format!(
                    "{GCC_WARNING_ERROR}{}cc1: all warnings being treated as errors\n",
                    note("-Wno-foo")
                ),
                1,
            ),
            (
                &["-c", "-w", "-Wno-foo", "unused.c", "-Wall"],
                String::new(),
                0,
            ),
            (
                &["-c", "-Wno-foo", "clean.c", "warn.c"],
                format!("{GCC_WARNING}{}", note("-Wno-foo")),
                0,
            ),
        ];
        for (args, expected, status) in cases {
            assert_eq!(
                directory.run("--gcc-mode", args),
                (expected.clone(), *status),
                "{args:?}"
            );
        }
    }

    /// GCC ends a compilation whose warnings `-Werror` made errors with a
    /// line saying so, `some` when only a `-Werror=` did, and one that
    /// `-Wfatal-errors` ended with a line naming it.
    #[test]
    fn gcc_reports_werror_and_fatal_errors() {
        let directory = Directory::new();
        directory.check(
            "--gcc-mode",
            &[
                (
                    &["-c", "-Werror", "warn.c"],
                    &format!("{GCC_WARNING_ERROR}cc1: all warnings being treated as errors\n"),
                    1,
                ),
                (
                    &["-c", "-Werror=cpp", "warn.c"],
                    &format!("{GCC_WARNING_ERROR}cc1: some warnings being treated as errors\n"),
                    1,
                ),
                (
                    &["-c", "-Werror=cpp", "-Werror", "-Wno-error", "warn.c"],
                    &format!("{GCC_WARNING_ERROR}cc1: some warnings being treated as errors\n"),
                    1,
                ),
                (
                    &["-c", "-Wall", "-Werror", "unused.c"],
                    "unused.c: In function 'main':\nunused.c:1:22: error: unused variable 'x' \
                     [-Werror=unused-variable]\n    1 | int main(void) { int x; return 0; }\n      \
                     |                      ^\ncc1: all warnings being treated as errors\n",
                    1,
                ),
                (&["-c", "-Werror", "-w", "warn.c"], "", 0),
                (
                    &["-c", "-Wfatal-errors", "err.c"],
                    &format!("{GCC_ERROR}compilation terminated due to -Wfatal-errors.\n"),
                    1,
                ),
                (
                    &["-c", "-Wno-foo", "-Wfatal-errors", "-Werror", "warn.c"],
                    &format!(
                        "{GCC_WARNING_ERROR}compilation terminated due to -Wfatal-errors.\ncc1: \
                         all warnings being treated as errors\n"
                    ),
                    1,
                ),
            ],
        );
    }

    /// Clang's front end warns about each warning option it does not
    /// know, for each source it compiles, counting the warning with the
    /// others, and suggests the group nearest a misspelled one; `-Werror`
    /// makes the warning an error, which leaves no object.
    #[test]
    fn clang_warns_about_unknown_warning_options() {
        let generated = |text: &str, count: &str| format!("{text}{count} generated.\n");
        let unknown = |option: &str| {
            format!("warning: unknown warning option '{option}' [-Wunknown-warning-option]\n")
        };
        let directory = Directory::new();
        let cases: &[(&[&str], String, i32)] = &[
            (
                &["-c", "-Wfoo", "clean.c"],
                generated(&unknown("-Wfoo"), "1 warning"),
                0,
            ),
            (
                &["-c", "-Wno-foo", "warn.c"],
                generated(
                    &format!("{}{CLANG_WARNING}", unknown("-Wno-foo")),
                    "2 warnings",
                ),
                0,
            ),
            (
                &["-c", "-Wshadwo", "clean.c"],
                generated(
                    "warning: unknown warning option '-Wshadwo'; did you mean '-Wshadow'? \
                     [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wno-unsued-variable", "clean.c"],
                generated(
                    "warning: unknown warning option '-Wno-unsued-variable'; did you mean \
                     '-Wno-unused-variable'? [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wno-no-shadow", "clean.c"],
                generated(
                    "warning: unknown warning option '-Wno-no-shadow'; did you mean \
                     '-Wno-shadow'? [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            // Clang names a `-Wno-error=` group `-Werror=`.
            (
                &["-c", "-Wno-error=shadwo", "clean.c"],
                generated(
                    "warning: unknown warning option '-Werror=shadwo'; did you mean \
                     '-Werror=shadow'? [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wformat=1", "clean.c"],
                generated(
                    "warning: unknown warning option '-Wformat=1'; did you mean '-Wformat=2'? \
                     [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wformat=", "clean.c"],
                generated(&unknown("-Wformat="), "1 warning"),
                0,
            ),
            (
                &["-c", "-Werror-", "clean.c"],
                generated(
                    "warning: unknown -Werror warning specifier: '-Werror-' \
                     [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wno-error-", "clean.c"],
                generated(
                    "warning: unknown -Werror warning specifier: '-Wno-error-' \
                     [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wfatal-errors=", "clean.c"],
                generated(
                    "warning: unknown -Wfatal-errors warning specifier: '-Wfatal-errors=' \
                     [-Wunknown-warning-option]\n",
                    "1 warning",
                ),
                0,
            ),
            (
                &["-c", "-Wfatal-errors=x", "clean.c"],
                generated(&unknown("-Wfatal-errors=x"), "1 warning"),
                0,
            ),
            (
                &["-c", "-Wno-foo", "-Werror", "clean.c"],
                generated(
                    "error: unknown warning option '-Wno-foo' [-Werror,-Wunknown-warning-option]\n",
                    "1 error",
                ),
                1,
            ),
            (
                &[
                    "-c",
                    "-Werror=unknown-warning-option",
                    "-Wno-error=foo",
                    "clean.c",
                ],
                generated(
                    "error: unknown warning option '-Werror=foo' \
                     [-Werror,-Wunknown-warning-option]\n",
                    "1 error",
                ),
                1,
            ),
            (
                &["-c", "-Wfoo", "clean.c", "clean.c"],
                format!(
                    "{}{}",
                    generated(&unknown("-Wfoo"), "1 warning"),
                    generated(&unknown("-Wfoo"), "1 warning")
                ),
                0,
            ),
            (&["-c", "-w", "-Wfoo", "clean.c"], String::new(), 0),
            (
                &["-c", "-Wfoo", "-Wno-unknown-warning-option", "clean.c"],
                String::new(),
                0,
            ),
            // Known to Clang, if not to bcc: `-Wno-` names the unnamed
            // group its table begins with.
            (
                &["-c", "-Wno-", "-Werror=", "-Wsystem-headers", "clean.c"],
                String::new(),
                0,
            ),
            (
                &["-c", "-Wfatal-errors=shadow", "-Wformat=0", "clean.c"],
                String::new(),
                0,
            ),
        ];
        for (args, expected, status) in cases {
            assert_eq!(
                directory.run("--clang-mode", args),
                (expected.clone(), *status),
                "{args:?}"
            );
        }
        // The error leaves no object.
        drop(std::fs::remove_file(directory.0.join("clean.o")));
        let (_, status) = directory.run("--clang-mode", &["-c", "-Wfoo", "-Werror", "clean.c"]);
        assert_eq!(status, 1);
        assert!(!directory.0.join("clean.o").exists());
    }

    /// Clang's `-Wfatal-errors` makes the first error fatal, its `-Werror`
    /// gone from the option it names, and ends the compilation there.
    #[test]
    fn clang_fatal_errors_end_the_compilation() {
        let directory = Directory::new();
        directory.check(
            "--clang-mode",
            &[
                (
                    &["-c", "-Wfatal-errors", "err.c"],
                    "err.c:1:25: fatal error: use of undeclared identifier 'y'\n    1 | int \
                     main(void) { return y; }\n      |                         ^\n1 error \
                     generated.\n",
                    1,
                ),
                (
                    &["-c", "-Wfatal-errors", "-Wfoo", "-Werror", "warn.c"],
                    "fatal error: unknown warning option '-Wfoo' [-Wunknown-warning-option]\n1 \
                     error generated.\n",
                    1,
                ),
                (
                    &["-c", "-Wfatal-errors", "-Werror", "warn.c"],
                    "warn.c:1:2: fatal error: hi [-W#warnings]\n    1 | #warning hi\n      |  \
                     ^\n1 error generated.\n",
                    1,
                ),
            ],
        );
    }

    /// Clang's `-Weverything` turns on every warning no option has set,
    /// `-Wno-everything` turns every one off, and a group's own option
    /// outranks either.
    #[test]
    fn clang_everything_turns_warnings_on_and_off() {
        let unused = "unused.c:1:22: warning: unused variable 'x' [-Wunused-variable]\n    1 | \
                      int main(void) { int x; return 0; }\n      |                      ^\n1 \
                      warning generated.\n";
        let directory = Directory::new();
        directory.check(
            "--clang-mode",
            &[
                (&["-c", "-Weverything", "unused.c"], unused, 0),
                (
                    &["-c", "-Wno-everything", "-Wunused-variable", "unused.c"],
                    unused,
                    0,
                ),
                (
                    &["-c", "-Wno-unused-variable", "-Weverything", "unused.c"],
                    "",
                    0,
                ),
                (
                    &["-c", "-Wno-everything", "-Weverything", "unused.c"],
                    "",
                    0,
                ),
                (&["-c", "-Wno-everything", "warn.c"], "", 0),
                (&["-c", "-Weverything", "clean.c"], "", 0),
            ],
        );
        // Clang has no group for bcc's own `-Wconstant-promotion`, so
        // `-Weverything` leaves an unsuffixed constant that becomes `long`
        // alone, even under `-Werror`.
        std::fs::write(
            directory.0.join("literal.c"),
            "int main(void) { return sizeof(2147483648) == 8 ? 0 : 1; }\n",
        )
        .unwrap();
        directory.check(
            "--clang-mode",
            &[(&["-c", "-Weverything", "-Werror", "literal.c"], "", 0)],
        );
    }

    /// clang-cl's driver has no `-Wl,`, `-Wa,` or `-Wp,`, so its front end
    /// reads each as a warning option it does not know.
    #[test]
    fn clang_cl_reads_linker_options_as_warning_options() {
        let directory = Directory::new();
        directory.check(
            "--clang-cl-mode",
            &[(
                &["/nologo", "/c", "-Wl,x", "-Wa,", "clean.c"],
                "warning: unknown warning option '-Wl,x' [-Wunknown-warning-option]\nwarning: \
                 unknown warning option '-Wa,' [-Wunknown-warning-option]\n2 warnings generated.\n",
                0,
            )],
        );
    }

    /// TCC takes back `-w` at any later `-W` option, known or not, and reads
    /// what follows `-Wp,` as one argument, commas and all.
    #[test]
    fn tcc_reads_w_and_wp_in_order() {
        let directory = Directory::new();
        directory.check(
            "--tcc-mode",
            &[
                (&["-c", "-w", "warn.c"], "", 0),
                (
                    &["-c", "-w", "-Wfoo", "warn.c"],
                    "warn.c:1: warning: #warning hi\n",
                    0,
                ),
                (&["-c", "-Wall", "-w", "warn.c"], "", 0),
                (
                    &["-c", "-w", "-Werror", "warn.c"],
                    "warn.c:1: error: #warning hi\n",
                    1,
                ),
                (
                    &["-c", "-Wp,", "clean.c"],
                    "tcc: error: file '' not found\n",
                    1,
                ),
            ],
        );
        let output = directory.output("--tcc-mode", &["-Wp,-DX=1,-DY=2", "-E", "-P", "macro.c"]);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1,-DY=2");
    }
}
