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
        assert!(text.contains("#pragma pack ( push , 1 )\n"), "{text}");
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
        assert_eq!(stdout(&all), "u.o: u.c sys/s.h h.h\n");
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
        // The inspection views keep exiting with zero.
        succeeds(&project.bcc(&["bad.c"]));
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
        missing(&project.bcc(&[
            "--target=x86_64-w64-windows-gnu",
            "-fsyntax-only",
            "hello.c",
        ]));
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
}
