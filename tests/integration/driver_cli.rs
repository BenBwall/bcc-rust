//! The compiler driver: `-c`, `-S` and `-E` jobs and their output names,
//! refusing an output that is an input, linking several inputs in order,
//! `--sysroot` linking, assembly inputs, `-M` dependency rules,
//! `-fsyntax-only`, stdin, and reading preprocessed output back.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
            Stdio,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    const HEADER: &str = "#define TWICE 2\nint helper(int);\n";
    const HELPER: &str = "#include \"h.h\"\nint helper(int x) { return x * TWICE; }\n";
    const MAIN: &str = "#include \"h.h\"\n#define F(x) ((x)+1)\nint main(void) {\n  return \
                        helper(F(20)) - 40;\n}\n";

    /// A fresh directory holding the three sources, removed afterwards.
    struct Project(PathBuf);

    impl Project {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-driver-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            for (name, text) in [("h.h", HEADER), ("a.c", HELPER), ("main.c", MAIN)] {
                std::fs::write(directory.join(name), text).unwrap();
            }
            Self(directory)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// Runs bcc in the project directory.
        fn bcc(&self, args: &[&str]) -> Output {
            self.command(args).output().unwrap()
        }

        fn command(&self, args: &[&str]) -> Command {
            let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
            _ = command
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH");
            command
        }
    }

    impl Drop for Project {
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
        assert!(output.status.success(), "{}", stderr(output));
    }

    /// The host's target, when the tests can run what bcc links for it.
    fn linkable_host() -> bool {
        cfg!(all(target_os = "linux", target_env = "gnu")) && Path::new("/usr/lib").is_dir()
    }

    #[test]
    fn compile_and_assemble_name_outputs_after_the_sources() {
        let project = Project::new();
        succeeds(&project.bcc(&["-c", "a.c", "main.c"]));
        assert!(project.path("a.o").is_file());
        assert!(project.path("main.o").is_file());
        succeeds(&project.bcc(&["-S", "main.c"]));
        let assembly = std::fs::read_to_string(project.path("main.s")).unwrap();
        assert!(assembly.contains("main:"), "{assembly}");
        let to_stdout = project.bcc(&["-S", "main.c", "-o", "-"]);
        succeeds(&to_stdout);
        assert!(stdout(&to_stdout).contains("main:"));
        succeeds(&project.bcc(&["-c", "main.c", "-o", "renamed.o"]));
        assert!(project.path("renamed.o").is_file());
        let both = project.bcc(&["-c", "a.c", "main.c", "-o", "x.o"]);
        assert_eq!(both.status.code(), Some(1));
        assert!(
            stderr(&both)
                .contains("cannot specify '-o' with '-c', '-S' or '-E' with multiple files"),
            "{}",
            stderr(&both)
        );
    }

    #[test]
    fn several_inputs_link_in_order() {
        if !linkable_host() {
            eprintln!("skipping: the test links for the Linux GNU host");
            return;
        }
        let project = Project::new();
        succeeds(&project.bcc(&["-c", "a.c"]));
        succeeds(&project.bcc(&["main.c", "a.o", "-lm", "-o", "app"]));
        let run = Command::new(project.path("app")).output().unwrap();
        assert_eq!(run.status.code(), Some(2));
        // Two sources, each compiled to a temporary object, then linked.
        succeeds(&project.bcc(&["a.c", "main.c", "-o", "app2", "-Wl,--as-needed"]));
        let run = Command::new(project.path("app2")).output().unwrap();
        assert_eq!(run.status.code(), Some(2));
        let leftovers: Vec<_> = std::fs::read_dir(&project.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".bcc-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // A failed job stops before the link.
        std::fs::write(
            project.path("bad.c"),
            "int f(void) { return ; }\nint x = ;\n",
        )
        .unwrap();
        let failed = project.bcc(&["bad.c", "main.c", "-o", "app3"]);
        assert_eq!(failed.status.code(), Some(1));
        assert!(!project.path("app3").exists());
    }

    /// One failing input fails the run whichever position it holds, and the
    /// driver still runs the other jobs but not the link, as GCC does. An
    /// input that cannot be read fails only its own job.
    #[test]
    fn a_failing_input_fails_a_multi_file_run() {
        let project = Project::new();
        std::fs::write(project.path("bad.c"), "int x = ;\n").unwrap();
        std::fs::write(project.path("stop.c"), "#error stop\n").unwrap();
        // Without a failing input, these link.
        if linkable_host() {
            succeeds(&project.bcc(&["main.c", "a.c", "-o", "linked"]));
        }
        for bad in ["bad.c", "stop.c", "missing.c"] {
            for sources in [[bad, "main.c", "a.c"], ["main.c", "a.c", bad]] {
                for stage in [
                    &["-c"][..],
                    &["-S"],
                    &["-fsyntax-only"],
                    &[],
                    &["-o", "app"],
                ] {
                    let failed = project.bcc(&[stage, &sources[..]].concat());
                    assert_eq!(failed.status.code(), Some(1), "{stage:?} {sources:?}");
                }
                assert!(!project.path("app").exists(), "{sources:?}");
                for compiler in [&[][..], &["--gcc-mode"]] {
                    drop(std::fs::remove_file(project.path("a.o")));
                    let failed = project.bcc(&[compiler, &["-c"], &sources[..]].concat());
                    assert_eq!(failed.status.code(), Some(1), "{compiler:?} {sources:?}");
                    assert!(project.path("a.o").is_file(), "{compiler:?} {sources:?}");
                    assert!(!project.path(&bad.replace(".c", ".o")).exists());
                }
            }
            // A syntax error is not a preprocessing error.
            if bad == "bad.c" {
                continue;
            }
            for sources in [[bad, "a.c"], ["a.c", bad]] {
                let failed = project.bcc(&[&["-E"][..], &sources[..]].concat());
                assert_eq!(failed.status.code(), Some(1), "{sources:?}");
                assert!(stdout(&failed).contains("helper"), "{}", stdout(&failed));
            }
        }
        let missing = project.bcc(&["-c", "missing.c", "a.c"]);
        assert!(
            stderr(&missing).contains("error: cannot read `missing.c`"),
            "{}",
            stderr(&missing)
        );
    }

    /// An input that cannot be read is reported in the words of the compiler
    /// whose diagnostic format is selected, and fails its own job alone.
    #[test]
    fn unreadable_inputs_are_reported_in_the_formats_words() {
        let project = Project::new();
        std::fs::create_dir(project.path("dir.c")).unwrap();
        let missing = "No such file or directory";
        for (format, expected) in [
            (
                "gcc",
                format!("cc1: fatal error: missing.c: {missing}\ncompilation terminated.\n"),
            ),
            (
                "clang",
                format!("error: error reading 'missing.c': {missing}\n1 error generated.\n"),
            ),
            (
                "msvc",
                format!("c1: fatal error C1083: Cannot open source file: 'missing.c': {missing}\n"),
            ),
            ("tcc", "tcc: error: file 'missing.c' not found\n".into()),
            (
                "bcc",
                format!("error: cannot read `missing.c`: {missing} (os error 2)\n"),
            ),
        ] {
            let option = format!("--diagnostic-format={format}");
            drop(std::fs::remove_file(project.path("a.o")));
            let output = project.bcc(&[&option, "-c", "missing.c", "a.c"]);
            assert_eq!(output.status.code(), Some(1), "{format}");
            if cfg!(unix) {
                assert_eq!(stderr(&output), expected, "{format}");
            }
            assert!(project.path("a.o").is_file(), "{format}");
            // A file that exists but cannot be read is reported the same way.
            let directory = project.bcc(&[&option, "--syntax-only", "dir.c"]);
            assert_eq!(directory.status.code(), Some(1), "{format}");
            assert!(
                stderr(&directory).contains("dir.c"),
                "{}",
                stderr(&directory)
            );
        }
        // clang-cl writes Clang's words in MSVC's format.
        let clang_cl = project.bcc(&[
            "--diagnostic-format=msvc",
            "--compiler-identity=clang-cl",
            "--syntax-only",
            "missing.c",
        ]);
        assert!(
            stderr(&clang_cl).starts_with("error: error reading 'missing.c': "),
            "{}",
            stderr(&clang_cl)
        );
    }

    /// `-c`, `-S` and `-E` refuse `-o` for several inputs, but `-M` and `-MM`
    /// without `-E` take it, each unit's rules replacing the last, as in
    /// GCC. A linker input they leave unused must still exist.
    #[test]
    fn stages_that_stop_early_take_outputs_and_linker_inputs_as_gcc_does() {
        let project = Project::new();
        for stage in [
            &["-c"][..],
            &["-S"],
            &["-E"],
            &["-E", "-M"],
            &["-M", "-c"],
            &["-MM", "-S"],
        ] {
            let refused = project.bcc(&[stage, &["a.c", "main.c", "-o", "out"]].concat());
            assert_eq!(refused.status.code(), Some(1), "{stage:?}");
            assert!(
                stderr(&refused).contains("cannot specify '-o' with '-c', '-S' or '-E'"),
                "{}",
                stderr(&refused)
            );
        }
        for rules in ["-M", "-MM"] {
            succeeds(&project.bcc(&[rules, "a.c", "main.c", "-o", "deps"]));
            assert_eq!(
                std::fs::read_to_string(project.path("deps")).unwrap(),
                "main.o: main.c h.h\n"
            );
        }
        succeeds(&project.bcc(&["-c", "a.c", "-o", "a.o"]));
        let unused = project.bcc(&["-c", "main.c", "a.o"]);
        succeeds(&unused);
        assert!(
            stderr(&unused).contains("warning: a.o: linker input file unused"),
            "{}",
            stderr(&unused)
        );
        let missing = project.bcc(&["-c", "main.c", "missing.o"]);
        assert_eq!(missing.status.code(), Some(1));
        assert!(
            stderr(&missing).contains("error: missing.o: linker input file not found: "),
            "{}",
            stderr(&missing)
        );
        assert!(project.path("main.o").is_file());
    }

    /// An interpreted program's status reaches the shell as a native build's
    /// would: the OS keeps its low byte on POSIX and all 32 bits on Windows.
    #[test]
    fn interpreted_status_reaches_the_shell_as_a_native_build_would() {
        let project = Project::new();
        for (name, body, status) in [
            ("negative.c", "return -1;", -1),
            ("wraps.c", "return 256;", 256),
            ("truncated.c", "return 300;", 300),
            ("exits.c", "exit(-2);", -2),
        ] {
            std::fs::write(
                project.path(name),
                format!("void exit(int);\nint main(void) {{ {body} }}\n"),
            )
            .unwrap();
            let expected = if cfg!(windows) { status } else { status & 255 };
            let run = project.bcc(&["--interpret", name]);
            assert_eq!(
                run.status.code(),
                Some(expected),
                "{name}: {}",
                stderr(&run)
            );
            if linkable_host() {
                let program = name.trim_end_matches(".c");
                succeeds(&project.bcc(&[name, "-o", program]));
                let native = Command::new(project.path(program)).output().unwrap();
                assert_eq!(native.status.code(), Some(expected), "{name}");
            }
        }
    }

    #[test]
    fn preprocessing_writes_an_expansion_where_its_invocation_is() {
        let project = Project::new();
        std::fs::write(
            project.path("swap.c"),
            "#define SWAP(a, b) do { \\\n    int t = (a); (a) = (b); (b) = t; } while (0)\nvoid \
             f(int x, int y) {\n    if (x > y) {\n        SWAP(x, y);\n    }\n}\n",
        )
        .unwrap();
        // The replacement list is spelled on line 1 of the same file, but
        // its tokens belong on the invocation's line, indented like it, as
        // GCC writes them.
        for flags in [&["-E", "-P"][..], &["-E"]] {
            let output = project.bcc(&[flags, &["swap.c"]].concat());
            succeeds(&output);
            let text = stdout(&output);
            assert!(
                text.contains("    if (x > y) {\n        do { int t ="),
                "{text}"
            );
        }
    }

    #[test]
    fn preprocessing_keeps_adjacent_string_literals_apart() {
        let project = Project::new();
        std::fs::write(
            project.path("strings.c"),
            "#define GREETING \"hello, \" \"world\"\nconst char *s = GREETING;\nconst char *t = \
             \"one\"\n    \"two\";\nconst char *u = u8\"x\" \"y\";\n",
        )
        .unwrap();
        // Phase 6 joins adjacent string literals; `-E` stops before it, so
        // each literal stays where it was written, as GCC and Clang print it.
        let output = project.bcc(&["-E", "-P", "strings.c"]);
        succeeds(&output);
        let text = stdout(&output);
        for expected in [
            "const char *s = \"hello, \" \"world\";",
            "const char *t = \"one\"\n    \"two\";",
            "const char *u = u8\"x\" \"y\";",
        ] {
            assert!(text.contains(expected), "{expected}\n{text}");
        }
        // The text compiles to the joined strings.
        std::fs::write(project.path("strings.i"), text).unwrap();
        succeeds(&project.bcc(&["-c", "strings.i", "-o", "strings.o"]));
    }

    #[test]
    fn preprocessing_prints_tokens_as_spelled_without_checking_them() {
        let project = Project::new();
        let source = "a @ b ` c 08 1e+f 0x1p \\ $x\nchar *s = \"\\x41\\101\\q\"; int c = \'ab\' + \
                      \'\\q\';\n";
        std::fs::write(project.path("spelled.c"), source).unwrap();
        // Phases 5 to 7 come after `-E`'s text: a character no C token starts
        // with, a number that is no constant and an escape C does not define
        // pass through as written, unreported, as GCC and Clang print them.
        let output = project.bcc(&["-E", "-P", "spelled.c"]);
        succeeds(&output);
        assert_eq!(stdout(&output), source);
        assert_eq!(stderr(&output), "");
    }

    #[test]
    fn preprocessing_keeps_text_that_begins_no_directive_or_token() {
        let project = Project::new();
        let source = "a # b %: c ## d\n#define H #\nH define X 1\nX\ndon't stop\nsay \"hi\n";
        std::fs::write(project.path("text.c"), source).unwrap();
        // C99 6.10p2: a `#` that does not start a line, or that a macro
        // produced, begins no directive; an unclosed quote keeps the rest of
        // its line with a warning. GCC and Clang print the text as written.
        let output = project.bcc(&["-E", "-P", "text.c"]);
        succeeds(&output);
        assert_eq!(
            stdout(&output),
            "a # b %: c ## d\n# define X 1\nX\ndon't stop\nsay \"hi\n"
        );
        let diagnostics = stderr(&output);
        for quote in ["'", "\""] {
            let warning = format!("warning: missing terminating `{quote}` character");
            assert!(diagnostics.contains(&warning), "{diagnostics}");
        }
        // Compiled, the stray `#` is an error and the line is no directive,
        // and the unclosed literal is still an error.
        std::fs::write(project.path("stray.c"), "int value; # define X 1\n").unwrap();
        let output = project.bcc(&["-fsyntax-only", "stray.c"]);
        assert!(!output.status.success());
        assert!(stderr(&output).contains("error: stray `#` in program"));
        std::fs::write(project.path("quote.c"), "char c = 'a;\n").unwrap();
        let output = project.bcc(&["-fsyntax-only", "quote.c"]);
        assert!(!output.status.success());
        assert!(stderr(&output).contains("error: unterminated character constant"));
    }

    #[test]
    fn preprocessing_writes_pragmas_back_as_spelled() {
        let project = Project::new();
        let source = "#define N 4\na\n#  pragma   pack ( N ) // c\n#pragma omp parallel  for \
                      N\n#pragma GCC diagnostic push\nb _Pragma(\"message(\\\"m\\\")\") \
                      c\n#pragma push_macro(\"N\")\nd\n";
        std::fs::write(project.path("pragmas.c"), source).unwrap();
        // GCC and Clang print each pragma later phases may read on its own
        // line, with its tokens as written, and leave out those that
        // preprocessing alone carries out.
        // The token after a `_Pragma` operator starts a line of its own,
        // indented to its column; the indentation is not compared.
        let output = project.bcc(&["-E", "-P", "pragmas.c"]);
        succeeds(&output);
        let text = stdout(&output);
        assert_eq!(
            text.lines().map(str::trim_start).collect::<Vec<_>>(),
            [
                "a",
                "#pragma pack ( N )",
                "#pragma omp parallel for N",
                "#pragma GCC diagnostic push",
                "b",
                "#pragma message(\"m\")",
                "c",
                "d",
            ],
            "{text}"
        );
    }

    #[test]
    fn a_pragma_in_a_macro_argument_applies_where_the_argument_lands() {
        let project = Project::new();
        let source = "#define FOO(x) x\nFOO(struct a { char c; int i; }; _Pragma(\"pack(1)\") \
                      struct b { char c; int i; };)\n_Static_assert(sizeof(struct a) == 8, \
                      \"a\");\n_Static_assert(sizeof(struct b) == 5, \"b\");\n";
        std::fs::write(project.path("arg.c"), source).unwrap();
        // GCC and Clang carry out a `_Pragma` in a macro argument where the
        // replaced argument lands, not when its prescan meets it, both when
        // compiling and in -E's text, which then compiles the same.
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&project.bcc(&[mode, "-fsyntax-only", "arg.c"]));
            let output = project.bcc(&[mode, "-E", "-P", "arg.c", "-o", "arg.i"]);
            succeeds(&output);
            let text = std::fs::read_to_string(project.path("arg.i")).unwrap();
            assert_eq!(
                text.lines()
                    .map(str::trim_start)
                    .take(3)
                    .collect::<Vec<_>>(),
                [
                    "struct a { char c; int i; };",
                    "#pragma pack(1)",
                    "struct b { char c; int i; };",
                ],
                "{mode}: {text}"
            );
            succeeds(&project.bcc(&[mode, "-fsyntax-only", "arg.i"]));
        }
    }

    #[test]
    fn preprocessing_writes_source_text_with_line_markers() {
        let project = Project::new();
        let output = project.bcc(&["-E", "main.c"]);
        succeeds(&output);
        let text = stdout(&output);
        assert!(text.starts_with("# 1 \"main.c\"\n"), "{text}");
        assert!(text.contains("int helper(int);"), "{text}");
        assert!(text.contains("return helper( ((20)+1)) - 40;"), "{text}");
        assert!(text.contains("# 3 \"main.c\"\n"), "{text}");
        let plain = project.bcc(&["-E", "-P", "main.c"]);
        assert!(!stdout(&plain).contains('#'), "{}", stdout(&plain));
        // The output compiles again, with its diagnostics at the original
        // lines.
        std::fs::write(project.path("main.i"), text.replace("40;", "40 +;")).unwrap();
        let reread = project.bcc(&["-c", "main.i", "-o", "main.o"]);
        assert_eq!(reread.status.code(), Some(1));
        assert!(
            stderr(&reread).contains("--> main.c:4:"),
            "{}",
            stderr(&reread)
        );
        assert!(!stderr(&reread).contains("warning:"), "{}", stderr(&reread));
        let macros = project.bcc(&["-E", "-dM", "-DEXTRA=1", "main.c"]);
        succeeds(&macros);
        let listing = stdout(&macros);
        for line in [
            "#define F(x) ((x)+1)",
            "#define TWICE 2",
            "#define EXTRA 1",
            "#define __STDC_VERSION__ 201710L",
            "#define __STDC__ 1",
        ] {
            assert!(listing.lines().any(|l| l == line), "{line}\n{listing}");
        }
        let mut sorted: Vec<_> = listing.lines().collect();
        sorted.sort_unstable();
        assert_eq!(sorted, listing.lines().collect::<Vec<_>>());
    }

    /// Flag 3 of a line marker makes the following lines a system header
    /// until the next marker, so `-E` output compiles with the system
    /// header's warnings withheld, as GCC and Clang read it; the user's
    /// own warnings remain, including in a system macro's expansion.
    #[test]
    fn line_marker_flag_three_withholds_system_header_warnings() {
        let project = Project::new();
        std::fs::create_dir_all(project.path("sys")).unwrap();
        std::fs::write(
            project.path("sys/s.h"),
            "static int s(void) { return ({ 1; }); }\n#define SYS ({ 2; })\n",
        )
        .unwrap();
        std::fs::write(
            project.path("u.c"),
            "#include <s.h>\nint u(void) { return ({ 3; }); }\nint m(void) { return SYS; }\n",
        )
        .unwrap();
        let output = project.bcc(&["-E", "-isystem", "sys", "u.c", "-o", "u.i"]);
        succeeds(&output);
        let text = std::fs::read_to_string(project.path("u.i")).unwrap();
        // Clang joins with the host's separator and escapes the name as a
        // string literal's contents.
        let line_marker = if cfg!(windows) {
            r#"# 1 "sys\\s.h" 3"#
        } else {
            r#"# 1 "sys/s.h" 3"#
        };
        assert!(text.contains(&format!("{line_marker}\n")), "{text}");
        let warnings = |output: &Output| {
            stderr(output)
                .lines()
                .filter_map(|line| line.trim().strip_prefix("--> "))
                .map(|location| location.split(':').take(2).collect::<Vec<_>>().join(":"))
                .collect::<Vec<_>>()
        };
        let reread = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "u.i"]);
        succeeds(&reread);
        assert_eq!(warnings(&reread), ["u.c:2", "u.c:3"], "{}", stderr(&reread));
        // Without the flag the same text is user code.
        std::fs::write(project.path("plain.i"), text.replace("\" 3\n", "\"\n")).unwrap();
        let plain = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "plain.i"]);
        let header = if cfg!(windows) {
            r"sys\s.h:1"
        } else {
            "sys/s.h:1"
        };
        assert_eq!(
            warnings(&plain),
            [header, "u.c:2", "u.c:3"],
            "{}",
            stderr(&plain)
        );
        // Ordinary source takes the markers too; a marker without the flag
        // ends the region.
        std::fs::write(
            project.path("m.c"),
            "# 1 \"h.h\" 1 3\nint h(void) { return ({ 1; }); }\n# 9 \"m.c\" 2\nint g(void) { \
             return ({ 2; }); }\n",
        )
        .unwrap();
        let marked = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "m.c"]);
        assert_eq!(warnings(&marked), ["m.c:9"], "{}", stderr(&marked));
        // A quoted include from a marked region is a system header too, as
        // in GCC and Clang, and one after the region is not.
        std::fs::write(
            project.path("q.h"),
            "static int q(void) { return ({ 1; }); }\n",
        )
        .unwrap();
        std::fs::write(
            project.path("r.h"),
            "static int r(void) { return ({ 2; }); }\n",
        )
        .unwrap();
        std::fs::write(
            project.path("inc.c"),
            "# 1 \"h.h\" 1 3\n#include \"q.h\"\n# 5 \"inc.c\" 2\n#include \"r.h\"\n",
        )
        .unwrap();
        let included = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "inc.c"]);
        assert_eq!(warnings(&included), ["r.h:1"], "{}", stderr(&included));
        // So is one found through `-I` or named by an absolute path, as GCC
        // takes the including position's system status on every path.
        std::fs::create_dir_all(project.path("idir")).unwrap();
        for (name, function) in [("idir/i.h", "i"), ("abs.h", "a")] {
            std::fs::write(
                project.path(name),
                format!("static int {function}(void) {{ return ({{ 3; }}); }}\n"),
            )
            .unwrap();
        }
        // Spelled with `/`, which Windows also reads, since a backslash in
        // a header name is an extension that `-pedantic` reports.
        let absolute = project
            .path("abs.h")
            .display()
            .to_string()
            .replace('\\', "/");
        std::fs::write(
            project.path("paths.c"),
            format!(
                "# 1 \"h.h\" 1 3\n#include \"i.h\"\n#include \"{absolute}\"\n# 5 \"paths.c\" \
                 2\nint x;\n"
            ),
        )
        .unwrap();
        let paths = project.bcc(&[
            "-fsyntax-only",
            "-std=c11",
            "-pedantic",
            "-I",
            "idir",
            "paths.c",
        ]);
        succeeds(&paths);
        assert!(warnings(&paths).is_empty(), "{}", stderr(&paths));
        std::fs::write(
            project.path("user.c"),
            format!("#include \"i.h\"\n#include \"{absolute}\"\nint x;\n"),
        )
        .unwrap();
        let user = project.bcc(&[
            "-fsyntax-only",
            "-std=c11",
            "-pedantic",
            "-I",
            "idir",
            "user.c",
        ]);
        assert_eq!(warnings(&user).len(), 2, "{}", stderr(&user));
        // `#warning` is still emitted inside the region, as GCC and Clang do.
        std::fs::write(
            project.path("warn.c"),
            "# 1 \"h.h\" 1 3\n#warning still shown\n# 3 \"warn.c\" 2\nint x;\n",
        )
        .unwrap();
        // The resource `<x86intrin.h>` does not reach the deprecated
        // `<mm3dnow.h>`, whose `#warning` would now show, as in Clang.
        std::fs::write(project.path("intrin.c"), "#include <x86intrin.h>\nint x;\n").unwrap();
        let intrin = project.bcc(&["-fsyntax-only", "-ffreestanding", "intrin.c"]);
        succeeds(&intrin);
        assert!(!stderr(&intrin).contains("warning"), "{}", stderr(&intrin));
        std::fs::write(project.path("dnow.c"), "#include <mm3dnow.h>\nint x;\n").unwrap();
        let dnow = project.bcc(&["-fsyntax-only", "-ffreestanding", "dnow.c"]);
        assert!(
            stderr(&dnow).contains("header is deprecated"),
            "{}",
            stderr(&dnow)
        );
        let warned = project.bcc(&["-fsyntax-only", "warn.c"]);
        succeeds(&warned);
        assert!(
            stderr(&warned).contains("still shown"),
            "{}",
            stderr(&warned)
        );
    }

    /// Line-marker flags follow GCC's order `[1|2] [3 [4]]`: anything else
    /// is an error that stops reading them, so a malformed marker makes no
    /// system header.
    #[test]
    fn line_marker_flags_must_follow_gcc_order() {
        let project = Project::new();
        let body = "static int f(void) { return ({ 1; }); }\n";
        for (flags, system) in [
            ("3", true),
            ("1 3", true),
            ("2 3 4", true),
            ("3 4", true),
            ("1", false),
        ] {
            std::fs::write(project.path("ok.c"), format!("# 1 \"h.h\" {flags}\n{body}")).unwrap();
            let output = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "ok.c"]);
            succeeds(&output);
            assert_eq!(
                stderr(&output).contains("warning:"),
                !system,
                "{flags}: {}",
                stderr(&output)
            );
        }
        for (flags, bad) in [
            ("1 2 3", "2"),
            ("1 wat 3", "wat"),
            ("3 1", "1"),
            ("4", "4"),
            ("1 4", "4"),
            ("3 3", "3"),
        ] {
            std::fs::write(
                project.path("bad.c"),
                format!("# 1 \"h.h\" {flags}\n{body}"),
            )
            .unwrap();
            let output = project.bcc(&["-fsyntax-only", "-std=c11", "-pedantic", "bad.c"]);
            let text = stderr(&output);
            assert_eq!(output.status.code(), Some(1), "{flags}: {text}");
            assert!(
                text.contains(&format!("error: invalid flag `{bad}` in line marker")),
                "{flags}: {text}"
            );
            // A 3 read before the bad flag still applies, as in GCC.
            assert_eq!(
                text.contains("warning:"),
                !flags.starts_with('3'),
                "{flags}: {text}"
            );
        }
    }

    #[test]
    fn preprocessed_output_keeps_pack_and_weak_pragmas() {
        let project = Project::new();
        std::fs::write(
            project.path("p.c"),
            "#pragma pack(push, 1)\nstruct s { char c; int i; };\n#pragma pack(pop)\n#pragma weak \
             w\nint w;\n_Static_assert(sizeof(struct s) == 5, \"packed\");\n",
        )
        .unwrap();
        let output = project.bcc(&["-E", "p.c", "-o", "p.i"]);
        succeeds(&output);
        let text = std::fs::read_to_string(project.path("p.i")).unwrap();
        assert!(text.contains("#pragma pack(push, 1)\n"), "{text}");
        assert!(text.contains("#pragma weak w\n"), "{text}");
        succeeds(&project.bcc(&["-c", "p.i", "-o", "p.o"]));
    }

    #[test]
    fn dependency_rules_follow_gcc() {
        let project = Project::new();
        std::fs::create_dir(project.path("sys")).unwrap();
        std::fs::write(project.path("sys/s.h"), "#define S 1\n").unwrap();
        std::fs::write(project.path("u.c"), "#include <s.h>\n#include \"h.h\"\n").unwrap();
        let all = project.bcc(&["-isystem", "sys", "-M", "u.c"]);
        succeeds(&all);
        // Clang joins with the host's separator.
        let header = if cfg!(windows) { r"sys\s.h" } else { "sys/s.h" };
        assert_eq!(stdout(&all), format!("u.o: u.c {header} h.h\n"));
        let user = project.bcc(&["-isystem", "sys", "-MM", "-MT", "obj/u.o", "-MP", "u.c"]);
        assert_eq!(stdout(&user), "obj/u.o: u.c h.h\n\nh.h:\n");
        let quoted = project.bcc(&["-MM", "-MQ", "a b$", "main.c"]);
        assert_eq!(stdout(&quoted), "a\\ b$$: main.c h.h\n");
        succeeds(&project.bcc(&["-c", "-MMD", "a.c", "-o", "lib.o"]));
        assert_eq!(
            std::fs::read_to_string(project.path("lib.d")).unwrap(),
            "lib.o: a.c h.h\n"
        );
        succeeds(&project.bcc(&["-c", "-MD", "-MF", "deps.mk", "main.c"]));
        assert_eq!(
            std::fs::read_to_string(project.path("deps.mk")).unwrap(),
            "main.o: main.c h.h\n"
        );
    }

    #[test]
    fn syntax_checks_and_stdin_input() {
        let project = Project::new();
        std::fs::write(project.path("bad.c"), "int x = ;\n").unwrap();
        assert_eq!(
            project.bcc(&["-fsyntax-only", "bad.c"]).status.code(),
            Some(1)
        );
        succeeds(&project.bcc(&["-fsyntax-only", "main.c"]));
        // So does the default analysis, as in any compiler.
        assert_eq!(project.bcc(&["bad.c"]).status.code(), Some(1));
        let mut child = project
            .command(&["-E", "-P", "-x", "c", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"#define N 3\nint n = N;\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        succeeds(&output);
        assert_eq!(stdout(&output).trim(), "int n = 3;");
        let unknown = project.bcc(&["-x", "fortran", "main.c"]);
        assert_eq!(unknown.status.code(), Some(1));
        assert!(stderr(&unknown).contains("language fortran not recognized"));
    }

    #[test]
    fn an_output_that_is_an_input_is_refused_before_any_job() {
        let project = Project::new();
        let check = |args: &[&str], input: &str| {
            let output = project.bcc(args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(
                stderr(&output).contains(&format!("is the same as input file '{input}'")),
                "{args:?}: {}",
                stderr(&output)
            );
        };
        // Compilation writes temporary objects, so only this check stops the
        // link from replacing a source.
        check(&["a.c", "main.c", "-o", "a.c"], "a.c");
        check(&["main.c", "a.c", "-o", "a.c"], "a.c");
        succeeds(&project.bcc(&["-c", "a.c"]));
        check(&["main.c", "a.o", "-o", "a.o"], "a.o");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("a.c", project.path("alias.c")).unwrap();
            check(&["main.c", "a.c", "-o", "alias.c"], "a.c");
            std::fs::hard_link(project.path("main.c"), project.path("hard.c")).unwrap();
            check(&["main.c", "a.c", "-o", "hard.c"], "main.c");
        }
        assert_eq!(
            std::fs::read_to_string(project.path("a.c")).unwrap(),
            HELPER
        );
        assert_eq!(
            std::fs::read_to_string(project.path("main.c")).unwrap(),
            MAIN
        );
    }

    #[test]
    fn dependency_files_are_named_after_the_stage_and_output() {
        let project = Project::new();
        let read = |name: &str| std::fs::read_to_string(project.path(name)).unwrap();
        succeeds(&project.bcc(&["-S", "-MD", "main.c"]));
        assert_eq!(read("main.d"), "main.o: main.c h.h\n");
        succeeds(&project.bcc(&["-S", "-MD", "main.c", "-o", "out.s"]));
        assert_eq!(read("out.d"), "out.s: main.c h.h\n");
        succeeds(&project.bcc(&["-E", "-MD", "a.c", "-o", "pre.i"]));
        assert_eq!(read("pre.d"), "a.o: a.c h.h\n");
        // `-o -` is stdout, not a name.
        succeeds(&project.bcc(&["-S", "-MMD", "a.c", "-o", "-"]));
        assert_eq!(read("a.d"), "a.o: a.c h.h\n");
        assert!(!project.path("-.d").exists());
        if !linkable_host() {
            eprintln!("skipping the link cases: they link for the Linux GNU host");
            return;
        }
        std::fs::write(
            project.path("solo.c"),
            "#include \"h.h\"\nint main(void) { return 0; }\n",
        )
        .unwrap();
        succeeds(&project.bcc(&["-MD", "solo.c", "-o", "app"]));
        assert_eq!(read("app.d"), "app: solo.c h.h\n");
        // With --sysroot one source links in two steps; the rule is the same.
        succeeds(&project.bcc(&["--sysroot=/", "-MD", "solo.c", "-o", "rooted"]));
        assert_eq!(read("rooted.d"), "rooted: solo.c h.h\n");
        // Several sources each write their own rule.
        succeeds(&project.bcc(&["-MMD", "a.c", "main.c", "-o", "app2"]));
        assert_eq!(read("a.d"), "a.o: a.c h.h\n");
        assert_eq!(read("main.d"), "main.o: main.c h.h\n");
        assert!(!project.path("app2.d").exists());
    }

    #[test]
    fn dash_dependency_destinations_write_to_stdout() {
        let project = Project::new();
        for args in [
            &["-M", "-MF", "-", "main.c"][..],
            &["-M", "main.c", "-o", "-"],
            &["-c", "-MD", "-MF", "-", "main.c"],
        ] {
            let output = project.bcc(args);
            succeeds(&output);
            assert_eq!(stdout(&output), "main.o: main.c h.h\n", "{args:?}");
            assert!(!project.path("-").exists(), "{args:?}");
        }
    }

    #[test]
    fn assembly_keeps_its_language_and_preprocessing_options() {
        let project = Project::new();
        std::fs::create_dir(project.path("inc")).unwrap();
        std::fs::write(project.path("inc/inc.h"), "#define INCLUDED 7\n").unwrap();
        std::fs::write(project.path("forced.h"), "#define FORCED 1\n").unwrap();
        let assembly = "#if !defined VALUE || !defined FORCED\n#error missing \
                        options\n#endif\n#include \"inc.h\"\n.globl f\nf:\n movl $INCLUDED, \
                        %eax\n ret\n";
        std::fs::write(project.path("f.txt"), assembly).unwrap();
        std::fs::write(project.path("f.S"), assembly).unwrap();
        let options = ["-DVALUE=1", "-Iinc", "-include", "forced.h"];
        // `-x assembler-with-cpp` reaches clang for an unknown extension.
        let mut args = vec!["-c", "-x", "assembler-with-cpp", "f.txt", "-o", "f1.o"];
        args.extend(options);
        succeeds(&project.bcc(&args));
        assert!(project.path("f1.o").is_file());
        let mut args = vec!["-c", "f.S", "-o", "f2.o"];
        args.extend(options);
        succeeds(&project.bcc(&args));
        // Without the options the preprocessor stops at `#error`.
        assert_eq!(
            project.bcc(&["-c", "f.S", "-o", "f3.o"]).status.code(),
            Some(1)
        );
        // `-x assembler` does not preprocess: the assembler reads the
        // directives as comments.
        succeeds(&project.bcc(&["-c", "-x", "assembler", "f.S", "-o", "f4.o"]));
        // Assembler options reach assembly but not C sources, which bcc
        // assembles itself; `--noexecstack` is already true of them.
        let warned = project.bcc(&["-c", "-Wa,--noexecstack,-L", "a.c"]);
        succeeds(&warned);
        assert_eq!(
            stderr(&warned),
            "warning: assembler option '-L' is not applied to C sources, which bcc assembles \
             itself\n"
        );
    }

    #[test]
    fn preprocessed_input_is_not_preprocessed_again() {
        let project = Project::new();
        // GCC and Clang pass no -D to `.i` input and replace no macro in it.
        std::fs::write(
            project.path("p.i"),
            "# 1 \"orig.c\"\n#define X 3\nint X;\nint *p = &X;\n",
        )
        .unwrap();
        std::fs::write(project.path("breaks.h"), "#error forced include\n").unwrap();
        succeeds(&project.bcc(&["-c", "-DX=1", "-include", "breaks.h", "p.i", "-o", "p.o"]));
        let mut child = project
            .command(&["-fsyntax-only", "-DX=1", "-x", "cpp-output", "-"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"int X;\n").unwrap();
        succeeds(&child.wait_with_output().unwrap());
        // Builtin macros are not replaced either.
        std::fs::write(project.path("line.i"), "int line = __LINE__;\n").unwrap();
        let line = project.bcc(&["-fsyntax-only", "line.i"]);
        assert_eq!(line.status.code(), Some(1));
        assert!(stderr(&line).contains("__LINE__"), "{}", stderr(&line));
        // Only line markers, #pragma, #ident, #define and #undef are read.
        std::fs::write(project.path("inc.i"), "#include \"h.h\"\nint y;\n").unwrap();
        let include = project.bcc(&["-fsyntax-only", "inc.i"]);
        assert_eq!(include.status.code(), Some(1));
        assert!(
            stderr(&include).contains("preprocessed input takes no directive"),
            "{}",
            stderr(&include)
        );
        // Phases 1 and 2 ran when it was written: GCC replaces no trigraph
        // and splices no line again, and reads a directive only in column 1.
        // UCNs still spell identifiers.
        std::fs::write(
            project.path("lexed.i"),
            "# 1 \"lexed.c\"\nconst char s[] = \"??=\";\nint caf\\u00e9 = \
             2;\n_Static_assert(sizeof s == 4, \"trigraph kept\");\n#ident \"id\"\n#pragma \
             pack(1)\nstruct p { char c; int i; };\n_Static_assert(sizeof(struct p) == 5, \
             \"pragma\");\n",
        )
        .unwrap();
        succeeds(&project.bcc(&["-std=c99", "-fsyntax-only", "lexed.i"]));
        std::fs::write(project.path("splice.i"), "int sp\\\nlice;\n").unwrap();
        let splice = project.bcc(&["-fsyntax-only", "splice.i"]);
        assert_eq!(splice.status.code(), Some(1));
        assert!(
            stderr(&splice).contains("unexpected character `\\`"),
            "{}",
            stderr(&splice)
        );
        std::fs::write(project.path("indented.i"), "  #pragma pack(1)\nint i;\n").unwrap();
        let indented = project.bcc(&["-fsyntax-only", "indented.i"]);
        assert_eq!(indented.status.code(), Some(1));
        assert!(
            stderr(&indented).contains("a directive only at the start of a line"),
            "{}",
            stderr(&indented)
        );
        // `-E` skips it, as GCC does, with Clang's warning.
        let again = project.bcc(&["-E", "p.i"]);
        succeeds(&again);
        assert_eq!(stdout(&again), "");
        assert!(stderr(&again).contains("p.i: previously preprocessed input"));
    }

    #[test]
    fn sysroot_reaches_the_link() {
        if !linkable_host() {
            eprintln!("skipping: the test links for the Linux GNU host");
            return;
        }
        let project = Project::new();
        std::fs::create_dir(project.path("empty-root")).unwrap();
        for sources in [&["main.c", "a.c"][..], &["a.c", "main.c"]] {
            let mut args = vec!["--sysroot=empty-root"];
            args.extend(sources);
            args.extend(["-o", "app"]);
            let failed = project.bcc(&args);
            assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
            assert!(
                stderr(&failed).contains("cannot build"),
                "{}",
                stderr(&failed)
            );
        }
        std::fs::write(project.path("solo.c"), "int main(void) { return 3; }\n").unwrap();
        let failed = project.bcc(&["--sysroot=empty-root", "solo.c", "-o", "solo"]);
        assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
        succeeds(&project.bcc(&["--sysroot=/", "solo.c", "-o", "solo"]));
        let run = Command::new(project.path("solo")).output().unwrap();
        assert_eq!(run.status.code(), Some(3));
    }

    #[test]
    fn the_host_c_library_needs_no_options() {
        let project = Project::new();
        std::fs::write(
            project.path("hello.c"),
            "#include <stdio.h>\nint main(void) {\n  printf(\"hello, world\\n\");\n  return \
             0;\n}\n",
        )
        .unwrap();
        let missing = |output: &Output| {
            assert_eq!(output.status.code(), Some(1));
            assert!(
                stderr(output).contains("cannot find header `stdio.h`"),
                "{}",
                stderr(output)
            );
        };
        // A cross target searches no C library unless told where it is.
        let cross = if cfg!(windows) {
            "--target=x86_64-unknown-linux-gnu"
        } else {
            "--target=x86_64-w64-windows-gnu"
        };
        missing(&project.bcc(&[cross, "-fsyntax-only", "hello.c"]));
        // On Windows the MinGW-w64 installation bcc links with is the host
        // C library, as for MinGW GCC.
        if cfg!(windows) {
            let mingw = std::env::var_os("MINGW_ROOT").is_some()
                || std::env::var_os("PATH").is_some_and(|path| {
                    std::env::split_paths(&path)
                        .any(|directory| directory.join("gcc.exe").is_file())
                });
            if !mingw {
                eprintln!("skipping the host cases: no MinGW-w64 on the host");
                return;
            }
            // bcc's own default target is Linux on every host.
            succeeds(&project.bcc(&[
                "--target=x86_64-w64-windows-gnu",
                "hello.c",
                "-o",
                "hello.exe",
            ]));
            let run =
                crate::native::run_native(&mut Command::new(project.path("hello.exe"))).unwrap();
            assert_eq!(stdout(&run), "hello, world\n");
            return;
        }
        if !linkable_host() {
            eprintln!("skipping the host cases: they need a Linux GNU host");
            return;
        }
        succeeds(&project.bcc(&["hello.c", "-o", "hello"]));
        let run = Command::new(project.path("hello")).output().unwrap();
        assert_eq!(stdout(&run), "hello, world\n");
        // musl is a cross target on a glibc host.
        missing(&project.bcc(&[
            "--target=x86_64-unknown-linux-musl",
            "-fsyntax-only",
            "hello.c",
        ]));
        // -nostdinc and -nostdlibinc remove the host's directories, and a
        // --sysroot replaces them.
        missing(&project.bcc(&["-nostdinc", "-fsyntax-only", "hello.c"]));
        missing(&project.bcc(&["-nostdlibinc", "-fsyntax-only", "hello.c"]));
        std::fs::create_dir(project.path("empty-root")).unwrap();
        missing(&project.bcc(&["--sysroot=empty-root", "-fsyntax-only", "hello.c"]));
        succeeds(&project.bcc(&["--sysroot=/", "hello.c", "-o", "rooted"]));
        // The multiarch directory follows the sysroot, as in Clang.
        let rules = project.bcc(&["--sysroot=/", "-M", "hello.c"]);
        succeeds(&rules);
        assert!(
            stdout(&rules).contains("/usr/include/x86_64-linux-gnu/"),
            "{}",
            stdout(&rules)
        );
    }

    #[test]
    fn every_mode_fails_after_an_error() {
        let project = Project::new();
        let write = |name: &str, text: &str| std::fs::write(project.path(name), text).unwrap();
        write("syntax.c", "int x = ;\n");
        write("semantic.c", "int main(void) { return missing; }\n");
        write("directive.c", "#error stop\nint a;\n");
        write("pedantic.c", "int a[0];\n");
        write("warned.c", "#warning careful\nint a;\n");
        write("clean.c", "int a;\n");
        let status = |args: &[&str]| project.bcc(args).status.code();
        for args in [
            &["syntax.c"][..],
            &["--syntax-tree", "syntax.c"],
            &["--raw-syntax", "syntax.c"],
            &["--semantic-types", "semantic.c"],
            &["semantic.c"],
            &["-fsyntax-only", "semantic.c"],
            &["--tokens", "directive.c"],
            &["-E", "directive.c"],
            &["-M", "directive.c"],
            &["-c", "syntax.c"],
            &["-S", "semantic.c", "-o", "-"],
            &["--emit=ir", "semantic.c"],
            // An extension is an error under -pedantic-errors, and a warning
            // under -Werror.
            &["-std=c99", "-pedantic-errors", "pedantic.c"],
            &[
                "-std=c99",
                "-pedantic-errors",
                "-fsyntax-only",
                "pedantic.c",
            ],
            &["-std=c99", "-pedantic-errors", "-c", "pedantic.c"],
            &["-Werror", "warned.c"],
        ] {
            assert_eq!(status(args), Some(1), "{args:?}");
        }
        assert!(!project.path("syntax.o").exists());
        for args in [
            &["clean.c"][..],
            &["--syntax-tree", "clean.c"],
            &["--tokens", "syntax.c"],
            &["-E", "syntax.c"],
            &["-std=c99", "-pedantic", "pedantic.c"],
            &["-std=c99", "-pedantic-errors", "-E", "pedantic.c"],
            &["warned.c"],
            &["-c", "clean.c"],
        ] {
            assert_eq!(status(args), Some(0), "{args:?}");
        }
    }

    #[test]
    fn pthread_defines_reentrant_as_well_as_linking() {
        let project = Project::new();
        std::fs::write(
            project.path("r.c"),
            "#ifndef _REENTRANT\n#error no _REENTRANT\n#endif\nint main(void) { return 0; }\n",
        )
        .unwrap();
        succeeds(&project.bcc(&["-pthread", "-fsyntax-only", "r.c"]));
        assert_eq!(
            project.bcc(&["-fsyntax-only", "r.c"]).status.code(),
            Some(1)
        );
        let text = project.bcc(&["-pthread", "-E", "-dM", "r.c"]);
        assert!(
            stdout(&text).contains("#define _REENTRANT 1"),
            "{}",
            stdout(&text)
        );
        if linkable_host() {
            succeeds(&project.bcc(&["-pthread", "r.c", "-o", "r"]));
        }
    }
}
