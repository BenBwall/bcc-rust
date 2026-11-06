//! `#pragma GCC diagnostic` and `#pragma clang diagnostic` against the command
//! line, each expectation read from GCC 14.2 and Clang 23.1. The fixtures
//! `pragma-diagnostic-*` under `tests/fixtures/diagnostics` hold the words and
//! the phases; this holds the flag combinations and the `-E` round trip.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The test harness owns subprocess output and temporary artifacts."
)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{
            Command,
            Output,
        },
    };

    const MODES: [&str; 2] = ["--gcc-mode", "--clang-mode"];

    fn directory(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "bcc-diagnostic-pragmas-{}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(directory: &PathBuf, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(arguments)
            .arg("-nostdlibinc")
            .current_dir(directory)
            .env("NO_COLOR", "1")
            .env("RUST_BACKTRACE", "0")
            .output()
            .expect("the bcc-rust test binary must run")
    }

    /// Each diagnostic's `line severity`, in order.
    fn lines(output: &Output) -> Vec<String> {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split(':');
                _ = fields.next()?;
                let number: u32 = fields.next()?.parse().ok()?;
                _ = fields.next()?;
                let severity = fields.next()?.trim();
                matches!(severity, "warning" | "error" | "fatal error")
                    .then(|| format!("{number} {severity}"))
            })
            .collect()
    }

    fn compile(name: &str, mode: &str, flags: &[&str], source: &str) -> Vec<String> {
        let root = directory(name);
        fs::write(root.join("input.c"), source).unwrap();
        let mut arguments = vec![mode, "-std=gnu17", "-c", "-o", "output.o"];
        arguments.extend_from_slice(flags);
        arguments.push("input.c");
        let output = run(&root, &arguments);
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        lines(&output)
    }

    fn undefined_under(kind: &str) -> String {
        format!("#pragma GCC diagnostic {kind} \"-Wundef\"\n#if UNDEFINED\n#endif\nint x;\n")
    }

    /// A pragma outranks every option but `-w`.
    #[test]
    fn a_pragma_outranks_the_command_line() {
        let flag_sets: [&[&str]; 7] = [
            &[],
            &["-Wundef"],
            &["-Wno-undef"],
            &["-Wundef", "-Werror"],
            &["-Werror=undef"],
            &["-Wundef", "-Wno-error=undef"],
            &["-Wno-undef", "-Wno-error"],
        ];
        for mode in MODES {
            for flags in flag_sets {
                let ignored = compile("outranks", mode, flags, &undefined_under("ignored"));
                assert!(ignored.is_empty(), "{mode} {flags:?}: {ignored:?}");
                assert_eq!(
                    compile("outranks", mode, flags, &undefined_under("warning")),
                    ["2 warning"],
                    "{mode} {flags:?}"
                );
                assert_eq!(
                    compile("outranks", mode, flags, &undefined_under("error")),
                    ["2 error"],
                    "{mode} {flags:?}"
                );
            }
            for flags in [&["-w"][..], &["-Wundef", "-w"], &["-Werror=undef", "-w"]] {
                for kind in ["ignored", "warning", "error"] {
                    let reported = compile("outranks", mode, flags, &undefined_under(kind));
                    assert!(reported.is_empty(), "{mode} {flags:?} {kind}: {reported:?}");
                }
            }
        }
    }

    /// A default error can be silenced or demoted, and `pop` gives it back.
    #[test]
    fn a_pragma_controls_a_default_error() {
        let source = "\
#pragma GCC diagnostic push
#pragma GCC diagnostic warning \"-Wint-conversion\"
char *a = 1;
#pragma GCC diagnostic ignored \"-Wint-conversion\"
char *b = 2;
#pragma GCC diagnostic pop
char *c = 3;
";
        for mode in MODES {
            assert_eq!(
                compile("default-error", mode, &[], source),
                ["3 warning", "7 error"],
                "{mode}"
            );
            assert_eq!(
                compile("default-error", mode, &["-Werror"], source),
                ["3 warning", "7 error"],
                "{mode}"
            );
        }
    }

    /// `pop` without `push`: GCC goes back to the command line and says
    /// nothing; Clang keeps the setting and warns.
    #[test]
    fn an_unmatched_pop_differs_between_the_compilers() {
        let source = "\
#pragma GCC diagnostic ignored \"-Wundef\"
#pragma GCC diagnostic pop
#if UNDEFINED
#endif
int x;
";
        assert_eq!(
            compile("pop", "--gcc-mode", &["-Wundef"], source),
            ["3 warning"]
        );
        assert_eq!(
            compile("pop", "--clang-mode", &["-Wundef"], source),
            ["2 warning"]
        );
    }

    /// `-Wno-pragmas` silences GCC's complaints, and Clang's other than an
    /// unknown group, which has its own option.
    #[test]
    fn the_pragmas_own_warnings_have_options() {
        let bad_kind = "#pragma GCC diagnostic bogus\nint x;\n";
        let unknown = "#pragma GCC diagnostic ignored \"-Wnosuch\"\nint x;\n";
        let cases: [(&str, &[&str], &str, &[&str]); 11] = [
            ("--gcc-mode", &["-Wno-pragmas"], bad_kind, &[]),
            ("--gcc-mode", &["-Wno-pragmas"], unknown, &[]),
            (
                "--gcc-mode",
                &["-Wno-unknown-pragmas"],
                bad_kind,
                &["1 warning"],
            ),
            ("--gcc-mode", &["-Werror=pragmas"], unknown, &["1 error"]),
            ("--gcc-mode", &["-Werror"], bad_kind, &["1 error"]),
            ("--clang-mode", &["-Wno-pragmas"], bad_kind, &[]),
            ("--clang-mode", &["-Wno-pragmas"], unknown, &["1 warning"]),
            ("--clang-mode", &["-Wno-unknown-pragmas"], bad_kind, &[]),
            (
                "--clang-mode",
                &["-Wno-unknown-warning-option"],
                unknown,
                &[],
            ),
            (
                "--clang-mode",
                &["-Werror=unknown-pragmas"],
                bad_kind,
                &["1 error"],
            ),
            ("--clang-mode", &["-Werror"], unknown, &["1 error"]),
        ];
        for (mode, flags, source, expected) in cases {
            assert_eq!(
                compile("own", mode, flags, source),
                expected,
                "{mode} {flags:?} {source}"
            );
        }
        for mode in MODES {
            let reported = compile("own", mode, &["-w"], unknown);
            assert!(reported.is_empty(), "{mode}: {reported:?}");
        }
    }

    /// `-Wpedantic` covers the extension diagnostics, enabled or not.
    #[test]
    fn a_pragma_controls_pedantic_diagnostics() {
        let source = "\
int a[0];
#pragma GCC diagnostic ignored \"-Wpedantic\"
int b[0];
#pragma GCC diagnostic warning \"-Wpedantic\"
int c[0];
#pragma GCC diagnostic error \"-Wpedantic\"
int d[0];
";
        let cases: [(&[&str], &[&str]); 4] = [
            (&[], &["5 warning", "7 error"]),
            (&["-pedantic"], &["1 warning", "5 warning", "7 error"]),
            (&["-pedantic-errors"], &["1 error", "5 warning", "7 error"]),
            (
                &["-pedantic", "-Werror"],
                &["1 error", "5 warning", "7 error"],
            ),
        ];
        for mode in MODES {
            for (flags, expected) in cases {
                assert_eq!(
                    compile("pedantic", mode, flags, source),
                    expected,
                    "{mode} {flags:?}"
                );
            }
        }
    }

    /// `-Wfatal-errors` stops GCC at the first error a pragma makes; Clang
    /// accepts the option and goes on.
    #[test]
    fn fatal_errors_stops_gcc_alone() {
        let source = "\
#pragma GCC diagnostic error \"-Wundef\"
#if FIRST
#endif
#if SECOND
#endif
int x;
";
        let root = directory("fatal");
        fs::write(root.join("input.c"), source).unwrap();
        let gcc = run(
            &root,
            &[
                "--gcc-mode",
                "-Wfatal-errors",
                "-c",
                "-o",
                "output.o",
                "input.c",
            ],
        );
        assert_eq!(lines(&gcc), ["2 error"]);
        assert!(String::from_utf8_lossy(&gcc.stderr).contains("compilation terminated."));
        assert_eq!(
            compile("fatal", "--clang-mode", &["-Wfatal-errors"], source),
            ["2 error", "4 error"]
        );
    }

    /// `-E` keeps each pragma, `_Pragma` operators as lines, and compiling its
    /// text keeps their effect, around macros expanded in that text.
    #[test]
    fn preprocessed_text_keeps_the_pragmas_and_their_effect() {
        let source = "\
#define QUIET(x) _Pragma(\"GCC diagnostic push\") _Pragma(\"GCC diagnostic ignored \
                      \\\"-Wint-conversion\\\"\") x _Pragma(\"GCC diagnostic pop\")
QUIET(char *a = 1;)
#pragma GCC diagnostic warning \"-Wint-conversion\"
char *b = 2;
#pragma GCC diagnostic ignored \"-Wint-conversion\"
char *c = 3;
";
        for mode in MODES {
            let root = directory(&format!("text{}", &mode[2..]));
            fs::write(root.join("input.c"), source).unwrap();
            let direct = run(&root, &[mode, "-c", "-o", "output.o", "input.c"]);
            let text = run(&root, &[mode, "-E", "-o", "input.i", "input.c"]);
            assert!(text.status.success(), "{mode}");
            let written = fs::read_to_string(root.join("input.i")).unwrap();
            assert!(
                written.contains("#pragma GCC diagnostic ignored \"-Wint-conversion\""),
                "{mode}: {written}"
            );
            assert!(
                written.contains("#pragma GCC diagnostic pop"),
                "{mode}: {written}"
            );
            let round_trip = run(&root, &[mode, "-c", "-o", "output.o", "input.i"]);
            assert_eq!(lines(&direct), lines(&round_trip), "{mode}: {written}");
            assert_eq!(lines(&direct).len(), 1, "{mode}");
        }
    }

    /// A preprocessor warning read before the pragma that follows it keeps
    /// the setting it was reported under, under `-E` as well.
    #[test]
    fn preprocessor_warnings_read_the_setting_where_they_arise() {
        let source = "\
#if BEFORE
#endif
#pragma GCC diagnostic ignored \"-Wundef\"
#if QUIET
#endif
int x;
";
        for mode in MODES {
            assert_eq!(
                compile("arise", mode, &["-Wundef"], source),
                ["1 warning"],
                "{mode}"
            );
            let root = directory(&format!("arise-e{}", &mode[2..]));
            fs::write(root.join("input.c"), source).unwrap();
            let text = run(&root, &[mode, "-E", "-Wundef", "-o", "input.i", "input.c"]);
            assert_eq!(lines(&text), ["1 warning"], "{mode}");
        }
    }

    /// GCC's `-Wall` holds `-Wmain`, which `error` and `warning` reach, while
    /// its `ignored "-Wall"` changes nothing; Clang's does not hold it.
    #[test]
    fn a_pragma_naming_wall_reaches_main_under_gcc() {
        let source = |kind: &str| {
            format!("#pragma GCC diagnostic {kind} \"-Wall\"\nchar main(void) {{ return 0; }}\n")
        };
        let gcc = |flags: &[&str], kind: &str| compile("wall", "--gcc-mode", flags, &source(kind));
        assert_eq!(gcc(&[], "error"), ["2 error"]);
        assert_eq!(gcc(&[], "warning"), ["2 warning"]);
        assert_eq!(gcc(&[], "ignored"), [""; 0]);
        assert_eq!(gcc(&["-Wall"], "ignored"), ["2 warning"]);
        assert_eq!(
            compile("wall", "--clang-mode", &[], &source("error")),
            ["2 warning"]
        );
    }

    /// A warning lowering reports, after the whole unit is analyzed, reads
    /// the settings where the call is.
    #[test]
    fn a_pragma_governs_a_call_warning_from_lowering() {
        let source = "\
__attribute__((warning(\"careful\"))) void careful(void);
void loud(void) { careful(); }
#pragma GCC diagnostic ignored \"-Wattribute-warning\"
void quiet(void) { careful(); }
#pragma GCC diagnostic error \"-Wattribute-warning\"
void promoted(void) { careful(); }
";
        for mode in MODES {
            assert_eq!(
                compile("call", mode, &[], source),
                ["2 warning", "6 error"],
                "{mode}"
            );
        }
    }

    /// `cl` applies the warning numbers bcc gives its warnings, and reads no
    /// other compiler's diagnostic pragma.
    #[test]
    fn cl_applies_warning_numbers() {
        let source = "\
#pragma warning(push, 3)
#pragma warning(error: 4013)
int loud(void) { return promoted(); }
#pragma warning(disable: 4013; error: 4047)
int quiet(void) { return disabled(); }
#pragma warning(pop)
#pragma warning(error: 4013)
int again(void) { return promoted_again(); }
#pragma GCC diagnostic ignored \"-Wimplicit-function-declaration\"
int still(void) { return promoted_still(); }
#pragma warning(bogus)
";
        let root = directory("cl");
        fs::write(root.join("input.c"), source).unwrap();
        let output = run(&root, &["--msvc-mode", "-c", "-Fooutput.obj", "input.c"]);
        let text = String::from_utf8_lossy(&output.stdout);
        let reported: Vec<_> = text
            .lines()
            .chain(String::from_utf8_lossy(&output.stderr).lines())
            .filter(|line| line.contains("error C4013"))
            .map(ToString::to_string)
            .collect();
        assert_eq!(reported.len(), 3, "{reported:?}");
        assert!(reported[0].contains("(3)"), "{reported:?}");
        assert!(reported[1].contains("(8)"), "{reported:?}");
        assert!(reported[2].contains("(10)"), "{reported:?}");
    }
}
