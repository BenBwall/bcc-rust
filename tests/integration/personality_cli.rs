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
        let directory = Directory::new();
        directory.write("empty.c", "");
        let output = directory.bcc(mode, &["-E", "-dM", "empty.c"]);
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
            (
                "--gcc-mode",
                "-funsigned-char",
                "plain char is always signed in bcc",
            ),
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
        // cl warns (D9002) and goes on.
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "/bogus", "x.c"]);
        succeeds(&cl);
        assert!(
            stderr(&cl).contains("Command line warning D9002 : ignoring unknown option '/bogus'"),
            "{}",
            stderr(&cl)
        );
        assert!(directory.path("x.obj").is_file());
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
            assert_eq!(output.status.code(), Some(1), "{mode}");
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
        assert_eq!(bad.status.code(), Some(1));
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
        assert_eq!(clang_cl.status.code(), Some(2));
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
        assert_eq!(strict.status.code(), Some(1), "{}", stderr(&strict));
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

    #[test]
    fn cl_outputs_never_replace_an_input() {
        let directory = Directory::new();
        directory.write("a.c", "int a(void) { return 1; }\n");
        directory.write("b.c", "int b(void) { return 2; }\n");
        directory.write("o.obj", "not an object\n");
        let refused = |mode: &str, args: &[&str], message: &str| {
            let output = directory.bcc(mode, args);
            assert_eq!(output.status.code(), Some(2), "{args:?}");
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
