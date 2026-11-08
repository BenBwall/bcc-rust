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
            self.bcc_with(mode, args, &[])
        }

        /// Runs bcc as [`Self::bcc`] does, with the environment variables
        /// `variables` set.
        fn bcc_with(&self, mode: &str, args: &[&str], variables: &[(&str, &str)]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args((!mode.is_empty()).then_some(mode))
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .env_remove("INCLUDE")
                .env_remove("CL")
                .env_remove("_CL_")
                .envs(variables.iter().copied())
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

    /// The target bcc's GCC and Clang personalities compile for by default:
    /// the host's, MinGW-w64 on a Windows GNU host, as `--target=` names it
    /// and as GCC's `-v` names it.
    const HOST_TARGET: (&str, &str) = if cfg!(windows) {
        ("x86_64-w64-windows-gnu", "x86_64-w64-mingw32")
    } else {
        ("x86_64-unknown-linux-gnu", "x86_64-linux-gnu")
    };

    /// What the TCC `-run` programs include for `getchar`, `printf`, `puts`
    /// and `EOF`: the host's `<stdio.h>`, but on Windows declarations, since
    /// bcc has none of the headers TCC for Windows ships (`win32/include`).
    const TCC_STDIO: &str = if cfg!(windows) {
        "int getchar(void);\nint printf(const char *, ...);\nint puts(const char *);\n#define EOF \
         (-1)\n"
    } else {
        "#include <stdio.h>\n"
    };

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
        // `cl` 19.44 defines a build number of 0, Clang one of 1.
        assert_eq!(
            cl_expand(
                "--msvc-mode",
                &[],
                "_MSC_BUILD _MSC_EXTENSIONS _WIN32 _M_X64 _M_AMD64"
            ),
            "0 1 1 100 100"
        );
        assert_eq!(cl_expand("--clang-cl-mode", &[], "_MSC_BUILD"), "1");
        assert_eq!(cl_expand("--msvc-mode", &["/DX#5", "/DY=6"], "X Y"), "5 6");
    }

    /// Each compiler's `-E` leaves out the pragmas its own preprocessor
    /// carries out: GCC its `#pragma GCC warning`, Clang its `clang`
    /// pragmas and MSVC's `region`; GCC writes a space after an empty
    /// `#pragma`.
    #[test]
    fn preprocessing_leaves_out_the_pragmas_each_compiler_consumes() {
        let directory = Directory::new();
        directory.write(
            "pragmas.c",
            "#define X 1\na\n#pragma GCC warning \"w\"\n#pragma clang deprecated(X)\n#pragma \
             region r\n#pragma\n#pragma once\nb\n",
        );
        let pragmas = |mode: &str| {
            let output = directory.bcc(mode, &["-E", "-P", "-w", "pragmas.c"]);
            succeeds(&output);
            stdout(&output)
        };
        assert_eq!(
            pragmas("--gcc-mode"),
            "a\n#pragma clang deprecated(X)\n#pragma region r\n#pragma \nb\n"
        );
        assert_eq!(
            pragmas("--clang-mode"),
            "a\n#pragma GCC warning \"w\"\n#pragma\nb\n"
        );
    }

    /// Clang macro-replaces `#pragma pack` operands and GCC does not, which
    /// ignores `pack(N)` as an unknown action; `-E` writes them as spelled
    /// for both, so GCC's rule compiles its own text the same.
    #[test]
    fn pack_operands_are_replaced_as_each_compiler_replaces_them() {
        let directory = Directory::new();
        let source = |size: u32| {
            format!(
                "#define N 2\n#pragma pack(N)\nstruct s {{ char c; int i; \
                 }};\n_Static_assert(sizeof(struct s) == {size}, \"size\");\n"
            )
        };
        directory.write("gcc.c", &source(8));
        directory.write("clang.c", &source(6));
        for (mode, file) in [("--gcc-mode", "gcc.c"), ("--clang-mode", "clang.c")] {
            succeeds(&directory.bcc(mode, &["-fsyntax-only", "-w", file]));
        }
        let output = directory.bcc("--gcc-mode", &["-E", "gcc.c", "-o", "gcc.i"]);
        succeeds(&output);
        succeeds(&directory.bcc("--gcc-mode", &["-fsyntax-only", "-w", "gcc.i"]));
        // TCC replaces the operands too, and its `-E` writes them replaced,
        // so its text packs as the source does (tcc mob e23dabe: 6 5 5).
        directory.write(
            "tcc.c",
            "#define N 2\n#define PUSH push\n#define ONE 1\n#pragma pack(N)\nstruct a { char c; \
             int i; };\n#pragma pack()\n#pragma pack(PUSH, 1)\nstruct b { char c; int i; \
             };\n#pragma pack(pop)\n#pragma pack(push, ONE)\nstruct c { char c; int i; \
             };\n#pragma pack(pop)\n_Static_assert(sizeof(struct a) == 6 && sizeof(struct b) == 5 \
             && sizeof(struct c) == 5, \"size\");\n",
        );
        let direct = directory.bcc("--tcc-mode", &["-c", "tcc.c", "-o", "tcc.o"]);
        succeeds(&direct);
        assert_eq!(stderr(&direct), "");
        succeeds(&directory.bcc("--tcc-mode", &["-E", "tcc.c", "-o", "tcc.i"]));
        let text = std::fs::read_to_string(directory.path("tcc.i")).unwrap();
        assert!(text.contains("#pragma pack(2)\n"), "{text}");
        assert!(text.contains("#pragma pack(push, 1)\n"), "{text}");
        let round_trip = directory.bcc("--tcc-mode", &["-c", "tcc.i", "-o", "tcc-i.o"]);
        succeeds(&round_trip);
        assert_eq!(stderr(&round_trip), "");
    }

    /// GCC and Clang take `static int a[];` and `static struct S s;`, which
    /// C99 §6.9.2p3 forbids; Python's `PyDoc_VAR` relies on it.
    #[test]
    fn incomplete_internal_tentative_definitions_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "array.c",
            "static const char a[];\nconst char *p = a;\nstatic const char a[] = \"abc\";\n",
        );
        directory.write(
            "tag.c",
            "struct S;\nstatic struct S s;\nstruct S { int a; };\nint f(void) { return s.a; }\n",
        );
        directory.write("never.c", "struct S;\nstatic struct S s;\n");
        directory.write(
            "marked.c",
            "__extension__ static int a[];\nstatic int a[3];\n__extension__ static int z[];\nint \
             *p = z;\n",
        );
        for (mode, option) in [
            ("--gcc-mode", "-pedantic"),
            ("--gcc-mode", "-pedantic-errors"),
            ("--clang-mode", "-pedantic"),
            ("--clang-mode", "-pedantic-errors"),
        ] {
            let output = directory.bcc(mode, &[option, "-c", "marked.c", "-o", "marked.o"]);
            succeeds(&output);
            let text = stderr(&output);
            assert!(
                text.contains("assumed to have one element"),
                "{mode} {option}: {text}"
            );
            assert!(!text.contains("internal"), "{mode} {option}: {text}");
        }
        let refused = "internal tentative definition has incomplete type";
        for (mode, option, file, ok, message) in [
            ("--gcc-mode", "-w", "array.c", true, ""),
            ("--gcc-mode", "-w", "tag.c", true, ""),
            ("--gcc-mode", "-pedantic", "array.c", false, refused),
            ("--gcc-mode", "-pedantic", "tag.c", true, ""),
            ("--clang-mode", "-Wall", "array.c", true, ""),
            ("--clang-mode", "-pedantic", "array.c", true, "warning: "),
            ("--clang-mode", "-pedantic", "tag.c", true, "warning: "),
            (
                "--clang-mode",
                "-pedantic-errors",
                "array.c",
                false,
                "error: ",
            ),
            ("--tcc-mode", "-w", "array.c", true, ""),
            ("--gcc-mode", "-w", "never.c", false, "remains incomplete"),
            (
                "--clang-mode",
                "-Wall",
                "never.c",
                false,
                "remains incomplete",
            ),
        ] {
            let output = directory.bcc(mode, &[option, "-c", file, "-o", "out.o"]);
            let text = stderr(&output);
            assert_eq!(
                output.status.success(),
                ok,
                "{mode} {option} {file}: {text}"
            );
            assert!(text.contains(message), "{mode} {option} {file}: {text}");
            if message.is_empty() {
                assert_eq!(text, "", "{mode} {option} {file}");
            }
        }
    }

    /// GCC warns of an unclosed quote in a skipped group, compiling as well
    /// as under `-E`, and `-pedantic-errors` makes it an error; Clang and
    /// TCC do not.
    #[test]
    fn unclosed_quotes_in_skipped_groups_warn_as_gcc_does() {
        let directory = Directory::new();
        directory.write("skipped.c", "#if 0\nit's\n#endif\nint x;\n");
        for args in [
            &["-fsyntax-only", "skipped.c"][..],
            &["-E", "-P", "skipped.c"],
        ] {
            let gcc = directory.bcc("--gcc-mode", args);
            succeeds(&gcc);
            assert!(
                stderr(&gcc).contains("warning: missing terminating `'` character"),
                "{args:?}: {}",
                stderr(&gcc)
            );
            let clang = directory.bcc("--clang-mode", args);
            succeeds(&clang);
            assert_eq!(stderr(&clang), "", "{args:?}");
        }
        // TCC does not look either (tcc mob e23dabe is silent); its
        // personality compiles with `-c` rather than `-fsyntax-only`.
        for args in [
            &["-c", "skipped.c", "-o", "skipped.o"][..],
            &["-E", "-P", "skipped.c"],
        ] {
            let tcc = directory.bcc("--tcc-mode", args);
            succeeds(&tcc);
            assert_eq!(stderr(&tcc), "", "{args:?}");
        }
        // `-pedantic-errors` makes the warning an error, as in GCC.
        let strict = directory.bcc("--gcc-mode", &["-pedantic-errors", "-E", "-P", "skipped.c"]);
        assert!(!strict.status.success());
        assert!(stderr(&strict).contains("error: "), "{}", stderr(&strict));
    }

    /// C23 leaves the white space at the ends of a `__VA_OPT__` replacement
    /// to `#`, which GCC drops and Clang keeps.
    #[test]
    fn stringized_optional_replacements_space_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write(
            "va_opt.c",
            "#define S(x) #x\n#define F(a, ...) S(-__VA_OPT__( a )-)\nF(x, 1) F(x)\n",
        );
        let strings = |mode: &str| {
            let output = directory.bcc(mode, &["-std=c23", "-E", "-P", "va_opt.c"]);
            succeeds(&output);
            stdout(&output)
        };
        assert_eq!(strings("--gcc-mode"), "\"-x-\" \"--\"\n");
        assert_eq!(strings("--clang-mode"), "\"- x -\" \"- -\"\n");
    }

    /// GCC stores the scalars of a structure or union in the byte order its
    /// `scalar_storage_order` attribute, or the `#pragma
    /// scalar_storage_order` in effect where its definition ends, names.
    /// Every bcc target is little-endian, so the GCC identity accepts that
    /// order and refuses a big-endian record rather than store it the other
    /// way; it warns where GCC ignores the attribute and gives GCC's
    /// argument errors. TCC and Clang ignore the attribute and the pragma.
    #[test]
    fn scalar_storage_order_is_refused_where_gcc_honours_it() {
        const BIG: &str = "__attribute__((scalar_storage_order(\"big-endian\")))";
        let gcc = |source: &str| {
            let directory = Directory::new();
            directory.write("sso.c", source);
            directory.bcc("--gcc-mode", &["-fsyntax-only", "sso.c"])
        };
        let refused = [
            format!("struct {BIG} S {{ int x; }} s;\n"),
            format!("union {BIG} U {{ int x; }} u;\n"),
            format!("struct S {{ int x; }} {BIG} s;\n"),
            format!("typedef struct {BIG} {{ int x; }} T;\n"),
            format!("struct S {{ int x; }};\ntypedef struct S T {BIG};\n"),
            format!("int f(void) {{ return sizeof(struct {BIG} {{ int x; }}); }}\n"),
            "struct [[gnu::scalar_storage_order(\"big-endian\")]] S { int x; } s;\n".to_owned(),
            "#pragma scalar_storage_order big-endian\nstruct S { int x; } s;\n#pragma \
             scalar_storage_order default\n"
                .to_owned(),
            // GCC reads the pragma state where the definition ends.
            "struct S { int y;\n#pragma scalar_storage_order big-endian\nint x; } s;\n#pragma \
             scalar_storage_order default\n"
                .to_owned(),
            "_Pragma(\"scalar_storage_order big-endian\") struct S { int x; } s;\n".to_owned(),
        ];
        for source in &refused {
            let output = gcc(source);
            assert_eq!(output.status.code(), Some(1), "{source}{}", stderr(&output));
            let text = stderr(&output);
            assert!(
                text.contains("error: big-endian scalar storage order is not supported")
                    && !text.contains(": `scalar_storage_order`"),
                "{source}{text}"
            );
        }
        let accepted = [
            "struct __attribute__((scalar_storage_order(\"little-endian\"))) S { int x; } s;\n",
            "#pragma scalar_storage_order little-endian\nstruct S { int x; } s;\n",
            "#pragma scalar_storage_order big-endian\n#pragma scalar_storage_order \
             default\nstruct S { int x; } s;\n",
            // The attribute overrides the pragma.
            "#pragma scalar_storage_order big-endian\nstruct S { int x; } \
             __attribute__((scalar_storage_order(\"little-endian\"))) s;\n",
            // A tag that is only named takes no order, silently.
            "struct S { int x; };\nstruct __attribute__((scalar_storage_order(\"big-endian\"))) S \
             s;\n",
        ];
        for source in accepted {
            let output = gcc(source);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{source}");
        }
        let ignored = [
            format!("int {BIG} v;\n"),
            format!("typedef int {BIG} I;\n"),
            format!("struct S {{ int x {BIG}; }} s;\n"),
            format!("enum {BIG} E {{ A }} e;\n"),
            format!("void f(int {BIG} a);\n"),
        ];
        for source in &ignored {
            let output = gcc(source);
            succeeds(&output);
            assert!(
                stderr(&output).contains("warning: attribute ignored: `scalar_storage_order`"),
                "{source}{}",
                stderr(&output)
            );
        }
        for (argument, message) in [
            (
                "(\"middle\")",
                "argument must be one of \"big-endian\" or \"little-endian\"",
            ),
            (
                "(1)",
                "argument must be one of \"big-endian\" or \"little-endian\"",
            ),
            (
                "",
                "wrong number of arguments specified for `scalar_storage_order`",
            ),
            ("(\"big-endian\", 1)", "wrong number of arguments specified"),
        ] {
            let output = gcc(&format!(
                "struct __attribute__((scalar_storage_order{argument})) S {{ int x; }} s;\n"
            ));
            assert_eq!(output.status.code(), Some(1), "{argument}");
            assert!(stderr(&output).contains(message), "{}", stderr(&output));
        }
        for (operand, message) in [
            (
                "middle",
                "expected `big-endian`, `little-endian`, or `default`",
            ),
            (
                "\"big-endian\"",
                "missing `big-endian`, `little-endian`, or `default`",
            ),
        ] {
            let output = gcc(&format!(
                "#pragma scalar_storage_order {operand}\nstruct S {{ int x; }} s;\n"
            ));
            succeeds(&output);
            assert!(stderr(&output).contains(message), "{}", stderr(&output));
        }

        // `-E` writes the pragma back, so its text is refused as well.
        let directory = Directory::new();
        directory.write(
            "pragma.c",
            "#pragma scalar_storage_order big-endian\nstruct S { int x; } s;\n",
        );
        succeeds(&directory.bcc("--gcc-mode", &["-E", "pragma.c", "-o", "pragma.i"]));
        let text = std::fs::read_to_string(directory.path("pragma.i")).unwrap();
        assert!(
            text.contains("#pragma scalar_storage_order big-endian\n"),
            "{text}"
        );
        let round_trip = directory.bcc("--gcc-mode", &["-fsyntax-only", "pragma.i"]);
        assert_eq!(round_trip.status.code(), Some(1), "{}", stderr(&round_trip));

        // TCC compiles both in the native order; Clang ignores the
        // attribute with a warning and the pragma silently.
        directory.write("attribute.c", &refused[0]);
        for mode in ["--tcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-c", "-o", "attribute.o", "attribute.c"]);
            succeeds(&output);
            assert!(
                stderr(&output).contains("unknown attribute ignored: `scalar_storage_order`"),
                "{mode}: {}",
                stderr(&output)
            );
            let output = directory.bcc(mode, &["-c", "-o", "pragma.o", "pragma.c"]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
        }
    }

    /// Characters beyond ASCII in identifiers follow each compiler's sets:
    /// Clang takes C99's Annex D in C99, C11's in C11 and C17 and Unicode's
    /// in C23; GCC 14 takes any of them in every mode
    /// (gcc.dg/ucnid-5-utf8.c, ucnid-15-utf8.c).
    #[test]
    fn identifier_characters_follow_each_compiler() {
        let directory = Directory::new();
        directory.write("emoji.c", "int a\u{1F600} = 2;\nint b\\U0001F600 = 3;\n");
        directory.write("super.c", "int \u{3C0}\u{B2} = 9;\n");
        // With a pedantic option, GCC enforces the standard's own set; a
        // combining mark or middle dot continues a C23 identifier.
        directory.write("mark.c", "int a\\u0300;\nint b\\u00B7;\n");
        for (file, standard, options, gcc, clang) in [
            ("emoji.c", "c99", &["-pedantic"][..], false, false),
            ("emoji.c", "c11", &["-pedantic"], true, true),
            ("emoji.c", "c23", &["-pedantic"], false, false),
            ("mark.c", "c23", &[], true, true),
            ("mark.c", "c23", &["-pedantic"], true, true),
        ] {
            for (mode, accepted) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let std = format!("-std={standard}");
                let mut args = vec![std.as_str(), "-fsyntax-only"];
                args.extend_from_slice(options);
                args.push(file);
                let output = directory.bcc(mode, &args);
                assert_eq!(
                    output.status.success(),
                    accepted,
                    "{mode} {args:?}: {}",
                    stderr(&output)
                );
            }
        }
        // GCC reads `\u0024` as `$`; Clang refuses it.
        directory.write(
            "dollar.c",
            "int a$b(void) { return 1; }\nint main(void) { return a\\u0024b() - 1; }\n",
        );
        let output = directory.bcc("--gcc-mode", &["-std=c99", "-fsyntax-only", "dollar.c"]);
        succeeds(&output);
        let output = directory.bcc("--clang-mode", &["-std=c99", "-fsyntax-only", "dollar.c"]);
        assert!(!output.status.success());
        for (file, standard, gcc, clang) in [
            ("emoji.c", "c99", true, false),
            ("emoji.c", "c11", true, true),
            ("emoji.c", "c17", true, true),
            ("emoji.c", "c23", true, false),
            ("super.c", "c99", true, false),
            ("super.c", "c11", true, true),
            ("super.c", "c23", true, true),
        ] {
            for (mode, accepted) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let std = format!("-std={standard}");
                let output = directory.bcc(mode, &[std.as_str(), "-fsyntax-only", file]);
                assert_eq!(
                    output.status.success(),
                    accepted,
                    "{mode} {std} {file}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// U+20D0-U+20FF continue a C23 identifier only where they are
    /// `XID_Continue`: its nonspacing marks, not its enclosing marks.
    /// Clang 23 refuses `a` followed by U+20DD in C23, as GCC 14 does
    /// under `-pedantic`; GCC also takes C11's set, which holds it.
    #[test]
    fn enclosing_marks_continue_no_c23_identifier() {
        let directory = Directory::new();
        directory.write("enclosing.c", "int a\u{20DD};\n");
        directory.write("enclosing-ucn.c", "int a\\u20DD;\n");
        directory.write("nonspacing.c", "int a\u{20D0};\n");
        directory.write("nonspacing-ucn.c", "int a\\u20D0;\n");
        for (file, options, gcc, clang) in [
            ("enclosing.c", &[][..], true, false),
            ("enclosing.c", &["-pedantic"], false, false),
            ("enclosing-ucn.c", &[], true, false),
            ("enclosing-ucn.c", &["-pedantic"], false, false),
            ("nonspacing.c", &["-pedantic"], true, true),
            ("nonspacing-ucn.c", &["-pedantic"], true, true),
        ] {
            for (mode, accepted) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let mut args = vec!["-std=c23", "-fsyntax-only"];
                args.extend_from_slice(options);
                args.push(file);
                let output = directory.bcc(mode, &args);
                assert_eq!(
                    output.status.success(),
                    accepted,
                    "{mode} {args:?}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// No universal character name spells a basic or control character in
    /// an identifier (C99 and C23 §6.4.3p2): GCC 14 and Clang 23 refuse
    /// `\u0061`, `a\u0030` and the like in every mode. GCC alone reads
    /// `\u0024` as `$`.
    #[test]
    fn universal_character_names_spell_no_basic_character() {
        let directory = Directory::new();
        for (index, source) in [
            "int \\u0061;\n",
            "int \\U00000061;\n",
            "int a\\u0030;\n",
            "int a\\u005F;\n",
            "int a\\u0040;\n",
            "int a\\u0060;\n",
            "int a\\u0001;\n",
            "int a\\u009F;\n",
        ]
        .into_iter()
        .enumerate()
        {
            let file = format!("basic{index}.c");
            directory.write(&file, source);
            for standard in ["-std=c99", "-std=c17", "-std=c23", "-std=gnu17"] {
                for mode in ["--gcc-mode", "--clang-mode"] {
                    let output = directory.bcc(mode, &[standard, "-fsyntax-only", &file]);
                    assert!(
                        !output.status.success(),
                        "{mode} {standard} {source}: {}",
                        stderr(&output)
                    );
                }
            }
        }
        directory.write("dollar.c", "int a\\u0024;\n");
        for standard in ["-std=c99", "-std=c23"] {
            let output = directory.bcc("--gcc-mode", &[standard, "-fsyntax-only", "dollar.c"]);
            succeeds(&output);
            let output = directory.bcc("--clang-mode", &[standard, "-fsyntax-only", "dollar.c"]);
            assert!(!output.status.success());
        }
    }

    /// What GCC 14 finds outside Normalization Form C, raw and as universal
    /// character names.
    #[test]
    fn gcc_finds_every_kind_of_identifier_outside_normalization_form_c() {
        use std::fmt::Write as _;
        let directory = Directory::new();
        // Composition, also across a lower mark; marks out of canonical
        // order; characters the form never holds, wherever they stand; and
        // Hangul. Otherwise nothing composes or moves.
        for (raw, warned) in [
            ("a\u{301}", true),
            ("a\u{327}\u{301}", true),
            ("a\u{301}\u{301}", true),
            ("e\u{301}\u{302}", true),
            ("b\u{301}\u{327}", true),
            ("\u{212B}", true),
            ("\u{212B}x", true),
            ("x\u{212B}", true),
            ("\u{958}", true),
            ("\u{1100}\u{1161}", true),
            ("\u{AC00}\u{11A8}", true),
            ("b\u{327}\u{301}", false),
            ("x\u{301}", false),
            ("q\u{300}", false),
            ("\u{AC01}\u{11A8}", false),
            ("\u{C5}\u{327}", false),
            ("\u{E0}", false),
        ] {
            let mut universal = String::new();
            for character in raw.chars() {
                if character.is_ascii() {
                    universal.push(character);
                } else {
                    _ = write!(universal, "\\u{:04X}", u32::from(character));
                }
            }
            for spelling in [raw, &universal] {
                directory.write("id.c", &format!("int {spelling};\n"));
                let output = directory.bcc("--gcc-mode", &["-std=c23", "-fsyntax-only", "id.c"]);
                let message = stderr(&output);
                assert!(output.status.success(), "{spelling}: {message}");
                assert_eq!(
                    message.contains("is not in Normalization Form C"),
                    warned,
                    "{spelling}: {message}"
                );
            }
        }
    }

    /// GCC 14 warns of an identifier outside Normalization Form C
    /// (`-Wnormalized`), as `a` followed by U+0300 is; C23 requires the
    /// form, so there `-pedantic-errors` makes it an error. Clang has no
    /// such check, nor the option.
    #[test]
    fn gcc_reports_identifiers_outside_normalization_form_c() {
        let directory = Directory::new();
        directory.write("raw.c", "int a\u{300};\n");
        directory.write("ucn.c", "int a\\u0300;\n");
        directory.write("composed.c", "int \u{E0}; int \\u00E0x;\n");
        // (file, standard, options, accepted, warned) under GCC.
        for file in ["raw.c", "ucn.c"] {
            for (standard, options, accepted, warned) in [
                ("-std=c23", &[][..], true, true),
                ("-std=gnu23", &["-pedantic"], true, true),
                ("-std=c23", &["-pedantic-errors"], false, true),
                ("-std=gnu23", &["-pedantic-errors"], false, true),
                ("-std=c17", &["-pedantic-errors"], true, true),
                ("-std=c17", &["-Werror=normalized"], false, true),
                (
                    "-std=c23",
                    &["-pedantic-errors", "-Wno-error=normalized"],
                    true,
                    true,
                ),
                (
                    "-std=c23",
                    &["-pedantic-errors", "-Wno-normalized"],
                    true,
                    false,
                ),
                (
                    "-std=c23",
                    &["-Wnormalized=none", "-pedantic-errors"],
                    true,
                    false,
                ),
                ("-std=c23", &["-pedantic-errors", "-w"], true, false),
            ] {
                let mut args = vec![standard, "-fsyntax-only"];
                args.extend_from_slice(options);
                args.push(file);
                let output = directory.bcc("--gcc-mode", &args);
                let message = stderr(&output);
                assert_eq!(output.status.success(), accepted, "{args:?}: {message}");
                assert_eq!(
                    message.contains("is not in Normalization Form C"),
                    warned,
                    "{args:?}: {message}"
                );
            }
            let output = directory.bcc(
                "--clang-mode",
                &["-std=c23", "-pedantic-errors", "-fsyntax-only", file],
            );
            succeeds(&output);
            assert_eq!(stderr(&output), "");
        }
        let output = directory.bcc(
            "--gcc-mode",
            &[
                "-std=c23",
                "-pedantic-errors",
                "-fsyntax-only",
                "composed.c",
            ],
        );
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let output = directory.bcc(
            "--clang-mode",
            &["-std=c23", "-Wno-normalized", "-fsyntax-only", "composed.c"],
        );
        succeeds(&output);
        assert!(
            stderr(&output).contains("unknown warning option '-Wno-normalized'"),
            "{}",
            stderr(&output)
        );
    }

    /// `-W` is the old spelling of `-Wextra`, which redis and nginx still
    /// pass, and `-Wp,-MD,FILE` names the dependency file, as kbuild passes
    /// it, rather than an input.
    #[test]
    fn old_warning_and_dependency_spellings_reach_their_options() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-W", "-Wall", "-c", "x.c", "-o", "x.o"]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
            for (option, file) in [("-Wp,-MD,x.d", "x.d"), ("-Wp,-MMD,y.d", "y.d")] {
                let output = directory.bcc(mode, &[option, "-c", "x.c", "-o", "x.o"]);
                succeeds(&output);
                assert_eq!(stderr(&output), "", "{mode} {option}");
                let text = std::fs::read_to_string(directory.path(file)).unwrap();
                assert!(
                    text.starts_with("x.o:") && text.contains("x.c"),
                    "{mode}: {text}"
                );
            }
        }
    }

    /// The last `-MF` names the dependency file, but GCC's `-Wp,-MD,FILE`
    /// beats any `-MF`, wherever it stands, and Clang's does not.
    #[test]
    fn dependency_file_options_override_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        for (mode, options, wins, loses) in [
            (
                "--gcc-mode",
                &["-MD", "-MF", "a.d", "-MF", "b.d"][..],
                "b.d",
                "a.d",
            ),
            (
                "--clang-mode",
                &["-MD", "-MF", "c.d", "-MF", "d.d"][..],
                "d.d",
                "c.d",
            ),
            (
                "--gcc-mode",
                &["-Wp,-MD,e.d", "-MF", "f.d"][..],
                "e.d",
                "f.d",
            ),
            (
                "--gcc-mode",
                &["-MF", "g.d", "-Wp,-MD,h.d"][..],
                "h.d",
                "g.d",
            ),
            (
                "--gcc-mode",
                &["-Wp,-MD,i.d", "-Wp,-MD,j.d"][..],
                "j.d",
                "i.d",
            ),
            (
                "--clang-mode",
                &["-Wp,-MD,k.d", "-MF", "l.d"][..],
                "l.d",
                "k.d",
            ),
            (
                "--clang-mode",
                &["-MF", "m.d", "-Wp,-MD,n.d"][..],
                "n.d",
                "m.d",
            ),
        ] {
            let args = [options, &["-c", "x.c", "-o", "x.o"]].concat();
            succeeds(&directory.bcc(mode, &args));
            assert!(directory.path(wins).exists(), "{mode} {options:?}");
            assert!(!directory.path(loses).exists(), "{mode} {options:?}");
        }
        // `-MMD` leaves out system headers, in either order against `-MD`.
        directory.write("y.c", "#include <stddef.h>\nint y;\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            for options in [["-MD", "-MMD"], ["-MMD", "-MD"]] {
                let args = [&options[..], &["-c", "y.c", "-o", "y.o"]].concat();
                succeeds(&directory.bcc(mode, &args));
                let text = std::fs::read_to_string(directory.path("y.d")).unwrap();
                assert!(!text.contains("stddef.h"), "{mode} {options:?}: {text}");
            }
        }
    }

    /// `-r` links partially, as kbuild's `built-in.o` needs: no start files or
    /// libraries, and the output is another object to link.
    #[cfg(target_os = "linux")]
    #[test]
    fn partial_links_produce_an_object_to_link() {
        let directory = Directory::new();
        directory.write("a.c", "int b(void);\nint main(void) { return b(); }\n");
        directory.write("b.c", "int b(void) { return 42; }\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&directory.bcc(mode, &["-c", "a.c", "-o", "a.o"]));
            succeeds(&directory.bcc(mode, &["-c", "b.c", "-o", "b.o"]));
            let partial = directory.bcc(mode, &["-r", "a.o", "b.o", "-o", "both.o"]);
            succeeds(&partial);
            assert_eq!(stderr(&partial), "", "{mode}");
            succeeds(&directory.bcc(mode, &["both.o", "-o", "both.exe"]));
            let run = Command::new(directory.path("both.exe")).output().unwrap();
            assert_eq!(run.status.code(), Some(42), "{mode}");
        }
    }

    /// An element beyond the object is a warning in GCC and Clang, each with
    /// its own count, and is dropped without being evaluated. TCC and `cl`
    /// refuse it. C99 §6.7.8p2.
    #[test]
    fn excess_initializer_elements_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "m.c",
            "int a[2] = {1, 2, 3, 4};\nstruct S { int x; } s = {5, 6};\nunion U { int i; } u = \
             {7, 8};\nint v = {9, 10};\n",
        );
        for (mode, first) in [("--gcc-mode", 4), ("--clang-mode", 3)] {
            let output = directory.bcc(mode, &["-fsyntax-only", "m.c"]);
            succeeds(&output);
            let text = stderr(&output);
            assert_eq!(
                text.matches("warning: excess elements").count(),
                first + 1,
                "{mode}: {text}"
            );
            for object in ["array", "struct", "union", "scalar"] {
                assert!(
                    text.contains(&format!("excess elements in {object} initializer")),
                    "{text}"
                );
            }
            let silent = directory.bcc(mode, &["-w", "-fsyntax-only", "m.c"]);
            succeeds(&silent);
            assert!(stderr(&silent).is_empty(), "{mode}: {}", stderr(&silent));
            let strict = directory.bcc(mode, &["-pedantic-errors", "-fsyntax-only", "m.c"]);
            assert_eq!(strict.status.code(), Some(1), "{mode}: {}", stderr(&strict));
        }
        directory.write(
            "run.c",
            "int f(void) { __builtin_trap(); }\nint main(void) { int d[1] = {1, f()}; return d[0] \
             - 1; }\n",
        );
        for opt in ["--opt=none", "--opt=bcc"] {
            let output = directory.bcc("", &["--interpret", opt, "run.c"]);
            assert_eq!(output.status.code(), Some(0), "{opt}: {}", stderr(&output));
        }
        let output = directory.bcc("--tcc-mode", &["-c", "m.c"]);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        let output = directory.bcc("--msvc-mode", &["/c", "m.c"]);
        assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    }

    /// After an excess element Clang drops the rest of its list, so a later
    /// designator applies nothing and names no error; GCC goes on applying
    /// and checking them. C99 §6.7.8p2.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_later_designator_after_an_excess_element_follows_each_compiler() {
        let directory = Directory::new();
        for (source, gcc, clang) in [
            ("int a[1] = {1, 2, [0] = 3};", 3, 1),
            ("int a[2][1] = {{4, 5, [0] = 6}, {7, 8, [0] = 9}};", 6, 4),
        ] {
            directory.write(
                "later.c",
                &format!(
                    "{source}\nint main(void) {{ return a[0]{}; }}\n",
                    if source.contains("[2]") { "[0]" } else { "" }
                ),
            );
            for (mode, expected) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                succeeds(&directory.bcc(mode, &["-w", "later.c", "-o", "later.exe"]));
                let run = Command::new(directory.path("later.exe")).output().unwrap();
                assert_eq!(run.status.code(), Some(expected), "{mode}: {source}");
            }
        }
        directory.write("named.c", "struct S { int a; } s = {1, 2, .b = 3};\n");
        assert_eq!(
            directory
                .bcc("--gcc-mode", &["-w", "-fsyntax-only", "named.c"])
                .status
                .code(),
            Some(1)
        );
        succeeds(&directory.bcc("--clang-mode", &["-w", "-fsyntax-only", "named.c"]));
    }

    /// A store through a cast to what this unit only declares const reaches
    /// the other unit's object under GCC, whose optimizer keeps it, and Clang
    /// drops it at `-O2`. busybox sets its `bb_errno` this way on GCC.
    #[cfg(target_os = "linux")]
    #[test]
    fn stores_to_const_declarations_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "extern int *const e;\nint x = 7;\nvoid init(void) {\n  *(void **)&e = (void *)&x;\n  \
             __asm__ volatile(\"\" : : : \"memory\");\n}\n",
        );
        directory.write(
            "b.c",
            "int *e;\nvoid init(void);\nint main(void) {\n  init();\n  return e ? *e : 99;\n}\n",
        );
        for (mode, level, expected) in [
            ("--gcc-mode", "-O0", 7),
            ("--gcc-mode", "-O2", 7),
            ("--clang-mode", "-O0", 7),
            ("--clang-mode", "-O2", 99),
        ] {
            succeeds(&directory.bcc(mode, &[level, "a.c", "b.c", "-o", "const.exe"]));
            let run = Command::new(directory.path("const.exe")).output().unwrap();
            assert_eq!(run.status.code(), Some(expected), "{mode} {level}");
        }
        // A definition stays read-only, whichever compiler it follows.
        directory.write("d.c", "const int k = 1;\nconst int *p = &k;\n");
        let output = directory.bcc("--gcc-mode", &["-S", "d.c", "-o", "d.s"]);
        succeeds(&output);
        let text = std::fs::read_to_string(directory.path("d.s")).unwrap();
        assert!(text.contains(".rodata"), "{text}");
    }

    /// `__BASE_FILE__` names the main source file as the command line
    /// wrote it, wherever it is expanded, as in GCC, Clang and TCC.
    #[test]
    fn base_file_names_the_main_source() {
        let directory = Directory::new();
        std::fs::create_dir_all(directory.path("sub")).unwrap();
        directory.write("sub/h.h", "const char *h = __BASE_FILE__;\n");
        directory.write("forced.h", "const char *f = __BASE_FILE__;\n");
        directory.write(
            "m.c",
            "#include \"sub/h.h\"\nconst char *m = __BASE_FILE__;\n#ifdef __BASE_FILE__\nint \
             defined_ok;\n#endif\n",
        );
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let output = directory.bcc(mode, &["-E", "-P", "m.c"]);
            succeeds(&output);
            let text = stdout(&output);
            for line in [
                "const char *h = \"m.c\";",
                "const char *m = \"m.c\";",
                "int defined_ok;",
            ] {
                assert!(text.contains(line), "{mode}: {line} in {text}");
            }
        }
        // In a forced include GCC and TCC name the main file too (Clang
        // says `<built-in>` there, which bcc does not mimic).
        for mode in ["--gcc-mode", "--tcc-mode"] {
            let output = directory.bcc(mode, &["-E", "-P", "-include", "forced.h", "m.c"]);
            succeeds(&output);
            assert!(
                stdout(&output).contains("const char *f = \"m.c\";"),
                "{mode}: {}",
                stdout(&output)
            );
        }
    }

    /// A presumed file name that ends in a separator has an empty last
    /// component for GCC, and `.` for Clang, or `/` if it is all separators.
    #[test]
    fn file_name_macro_splits_as_each_compiler_does() {
        let directory = Directory::new();
        for (name, gcc, clang) in [
            ("foo/bar/", "\"\"", "\".\""),
            ("foo//", "\"\"", "\".\""),
            ("/", "\"\"", "\"/\""),
            ("foo/.", "\".\"", "\".\""),
            ("foo/..", "\"..\"", "\"..\""),
            ("foo/bar.c", "\"bar.c\"", "\"bar.c\""),
            ("", "\"\"", "\"\""),
        ] {
            directory.write("n.c", &format!("#line 1 \"{name}\"\n__FILE_NAME__\n"));
            for (mode, expected) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(mode, &["-E", "-P", "n.c"]);
                succeeds(&output);
                assert_eq!(stdout(&output).trim(), expected, "{mode} {name:?}");
            }
        }
    }

    /// `__FILE_NAME__` is the last component of the presumed file name and
    /// `__INCLUDE_LEVEL__` counts `#include`s from the main file, forced ones
    /// included, in GCC and Clang. Microsoft's and TCC's compilers have
    /// neither.
    #[test]
    fn file_name_and_include_level_follow_gcc_and_clang() {
        let directory = Directory::new();
        std::fs::create_dir_all(directory.path("sub")).unwrap();
        directory.write(
            "sub/h.h",
            "h = __INCLUDE_LEVEL__ __FILE_NAME__\n#include \"i.h\"\n",
        );
        directory.write("sub/i.h", "i = __INCLUDE_LEVEL__ __FILE_NAME__ EARLY\n");
        directory.write("forced.h", "f = __INCLUDE_LEVEL__ __FILE_NAME__\n");
        directory.write(
            "m.c",
            "#define EARLY __INCLUDE_LEVEL__\n#include \"sub/h.h\"\nm = __INCLUDE_LEVEL__ \
             __FILE_NAME__\n#line 9 \"zz/renamed.c\"\nr = __INCLUDE_LEVEL__ __FILE_NAME__\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-E", "-P", "-include", "forced.h", "m.c"]);
            succeeds(&output);
            assert_eq!(
                stdout(&output),
                "f = 1 \"forced.h\"\nh = 1 \"h.h\"\ni = 2 \"i.h\" 2\nm = 0 \"m.c\"\nr = 0 \
                 \"renamed.c\"\n",
                "{mode}"
            );
        }
        directory.write("q.c", "__FILE_NAME__ __INCLUDE_LEVEL__\n");
        for (mode, preprocess) in [("--msvc-mode", "/E"), ("--tcc-mode", "-E")] {
            let output = directory.bcc(mode, &[preprocess, "q.c"]);
            succeeds(&output);
            assert!(
                stdout(&output).contains("__FILE_NAME__ __INCLUDE_LEVEL__"),
                "{mode}: {}",
                stdout(&output)
            );
        }
    }

    /// `__TIMESTAMP__` spells the modification time of the file being read as
    /// `asctime` does, in every personality but TCC's, and
    /// `SOURCE_DATE_EPOCH` does not pin it.
    #[test]
    fn timestamp_is_the_modification_time_of_the_current_file() {
        let directory = Directory::new();
        directory.write("h.h", "__TIMESTAMP__\n");
        directory.write(
            "m.c",
            "#include \"h.h\"\n#line 3 \"elsewhere.c\"\n__TIMESTAMP__\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .current_dir(&directory.0)
                .args([mode, "-E", "-P", "m.c"])
                .env("SOURCE_DATE_EPOCH", "0")
                .output()
                .unwrap();
            succeeds(&output);
            let text = stdout(&output);
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 2, "{mode}: {text}");
            for line in lines {
                // "Thu Nov  5 23:13:48 2026" within quotes.
                let bytes = line.as_bytes();
                assert_eq!(bytes.len(), 26, "{mode}: {line}");
                assert_eq!(bytes[0], b'"');
                assert_eq!(&line[20..21], " ", "{mode}: {line}");
                assert!(!line.contains("??"), "{mode}: {line}");
                assert!(!line.ends_with("1970\""), "{mode}: {line}");
            }
        }
        directory.write("q.c", "__TIMESTAMP__\n");
        let output = directory.bcc("--tcc-mode", &["-E", "-P", "q.c"]);
        succeeds(&output);
        assert!(
            stdout(&output).contains("__TIMESTAMP__"),
            "{}",
            stdout(&output)
        );
    }

    /// Unlike the names C99 §6.10.8 predefines, these extensions can be
    /// redefined and undefined, with a warning, as in GCC and Clang.
    #[test]
    fn file_extension_macros_can_be_redefined() {
        let directory = Directory::new();
        directory.write(
            "m.c",
            "#define __BASE_FILE__ 1\n#define __FILE_NAME__ 2\n#define __INCLUDE_LEVEL__ \
             3\n#define __TIMESTAMP__ 4\n__BASE_FILE__ __FILE_NAME__ __INCLUDE_LEVEL__ \
             __TIMESTAMP__\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-E", "-P", "m.c"]);
            succeeds(&output);
            assert_eq!(stdout(&output), "1 2 3 4\n", "{mode}");
            assert!(
                stderr(&output).contains("warning"),
                "{mode}: {}",
                stderr(&output)
            );
        }
    }

    /// Clang's `__BASE_FILE__` is the main file's presumed name where the
    /// outermost `#include` (or the expansion itself) sits, so a `#line`
    /// there renames it; GCC and TCC keep the command line's name. Standard
    /// input is `<stdin>` in Clang, an empty name in GCC and `-` in TCC.
    #[test]
    fn base_file_follows_each_identity_for_line_and_stdin() {
        use std::io::Write as _;
        let directory = Directory::new();
        directory.write("h.h", "const char *h = __BASE_FILE__;\n");
        directory.write(
            "m.c",
            "#include \"h.h\"\n#line 9 \"z.c\"\nconst char *m = __BASE_FILE__;\n#include \"h.h\"\n",
        );
        for (mode, renamed) in [
            ("--clang-mode", true),
            ("--gcc-mode", false),
            ("--tcc-mode", false),
        ] {
            let output = directory.bcc(mode, &["-E", "-P", "m.c"]);
            succeeds(&output);
            let lines: Vec<_> = stdout(&output)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect();
            let late = if renamed { "z.c" } else { "m.c" };
            assert_eq!(
                lines,
                [
                    "const char *h = \"m.c\";".to_owned(),
                    format!("const char *m = \"{late}\";"),
                    format!("const char *h = \"{late}\";"),
                ],
                "{mode}"
            );
        }
        for (mode, name) in [
            ("--clang-mode", "<stdin>"),
            ("--gcc-mode", ""),
            ("--tcc-mode", "-"),
        ] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([mode, "-E", "-P", "-x", "c", "-"])
                .current_dir(&directory.0)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"__BASE_FILE__\n")
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{mode}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                format!("\"{name}\""),
                "{mode}"
            );
        }
    }

    /// `@FILE` reads arguments from a file in both modes, nested files
    /// included, with the differences between the two compilers.
    #[test]
    fn gnu_response_files_supply_arguments() {
        let directory = Directory::new();
        directory.write("a.c", "int a(void) { return 1; }\n");
        directory.write("b c.c", "int b(void) { return 2; }\n");
        directory.write("inner.rsp", "'b c.c'\n");
        directory.write("outer.rsp", "-c a.c\n@inner.rsp\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["@outer.rsp"]);
            succeeds(&output);
            for object in ["a.o", "b c.o"] {
                assert!(directory.path(object).exists(), "{mode}: {object}");
                std::fs::remove_file(directory.path(object)).unwrap();
            }
            // A file it cannot read stays as an input, reported missing.
            let output = directory.bcc(mode, &["-c", "@missing.rsp"]);
            assert!(!output.status.success(), "{mode}");
            assert!(stderr(&output).contains("@missing.rsp"), "{mode}");
        }

        // Either compiler resolves a nested name against the working directory,
        // not against the file naming it.
        std::fs::create_dir(directory.path("child")).unwrap();
        directory.write("nested.c", "int value = VALUE;\n");
        directory.write("child/outer.rsp", "@inner.rsp\n");
        directory.write("inner.rsp", "-E -P -DVALUE=11 nested.c\n");
        directory.write("child/inner.rsp", "-E -P -DVALUE=22 nested.c\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["@child/outer.rsp"]);
            succeeds(&output);
            assert_eq!(stdout(&output).trim_end(), "int value = 11;", "{mode}");
        }

        // A directory is an error in each compiler's words.
        let output = directory.bcc("--gcc-mode", &["@child", "-c", "a.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "gcc: error: @-file refers to a directory\n"
        );
        let output = directory.bcc("--clang-mode", &["@child", "-c", "a.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).starts_with("cannot not open file '"),
            "{}",
            stderr(&output)
        );

        directory.write("self.rsp", "@self.rsp\n");
        let output = directory.bcc("--clang-mode", &["@self.rsp"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).starts_with("recursive expansion of: '"));
        let output = directory.bcc("--gcc-mode", &["@self.rsp"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "gcc: error: too many @-files encountered\n"
        );
    }

    /// A response file's bytes are never replaced: a file name that is not
    /// UTF-8 is found, and a `-D` value bcc cannot take is refused rather than
    /// changed.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_response_files_keep_bytes_that_are_not_text() {
        use std::os::unix::ffi::OsStrExt as _;
        let directory = Directory::new();
        std::fs::write(
            directory.0.join(std::ffi::OsStr::from_bytes(b"\xff.c")),
            "int raw;\n",
        )
        .unwrap();
        std::fs::write(directory.path("name.rsp"), b"-E -P \xff.c".as_slice()).unwrap();
        directory.write("macro.c", "const char *s = MSG;\n");
        std::fs::write(
            directory.path("define.rsp"),
            b"-E -P -DMSG='\"\xff\"' macro.c".as_slice(),
        )
        .unwrap();
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["@name.rsp"]);
            succeeds(&output);
            assert_eq!(stdout(&output).trim_end(), "int raw;", "{mode}");
            let output = directory.bcc(mode, &["@define.rsp"]);
            assert!(!output.status.success(), "{mode}: {}", stdout(&output));
        }
    }

    /// What follows a NUL in a response file is no option: Clang ends the
    /// argument there and GCC stops reading the file.
    #[test]
    fn gnu_response_files_stop_at_a_nul() {
        let directory = Directory::new();
        directory.write(
            "t.c",
            "#ifndef OTHER\n#define OTHER 7\n#endif\nint value = OTHER;\n",
        );
        std::fs::write(
            directory.path("r.rsp"),
            b"-E -P t.c -DVALUE=1\0-DOTHER=2".as_slice(),
        )
        .unwrap();
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["@r.rsp"]);
            succeeds(&output);
            assert_eq!(stdout(&output).trim_end(), "int value = 7;", "{mode}");
        }
    }

    /// When several queries are given, the one each compiler answers, and
    /// `--print-` spelled for `-print-`.
    #[test]
    fn gnu_query_precedence_follows_each_compiler() {
        let directory = Directory::new();
        let answer = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            succeeds(&output);
            stdout(&output)
        };
        for (mode, winner, loser) in [
            ("--clang-mode", "-print-resource-dir", "-print-search-dirs"),
            ("--clang-mode", "-print-runtime-dir", "-print-file-name=x"),
            ("--clang-mode", "-print-runtime-dir", "-print-multi-lib"),
            ("--clang-mode", "-print-multi-lib", "-print-target-triple"),
            (
                "--gcc-mode",
                "-print-multiarch",
                "-print-multi-os-directory",
            ),
            ("--gcc-mode", "-print-multiarch", "-print-sysroot"),
            ("--gcc-mode", "-print-sysroot", "-print-multi-os-directory"),
        ] {
            let expected = answer(mode, &[winner]);
            assert_eq!(answer(mode, &[winner, loser]), expected, "{mode} {winner}");
            assert_eq!(answer(mode, &[loser, winner]), expected, "{mode} {winner}");
        }
        for (mode, query) in [
            ("--gcc-mode", "print-multi-lib"),
            ("--gcc-mode", "print-file-name=x"),
            ("--clang-mode", "print-multi-directory"),
            ("--clang-mode", "print-libgcc-file-name"),
        ] {
            assert_eq!(
                answer(mode, &[&format!("--{query}")]),
                answer(mode, &[&format!("-{query}")]),
                "{mode} {query}"
            );
        }
    }

    /// The probes libtool and other build systems ask about library layout.
    #[test]
    fn gnu_print_queries_describe_the_library_layout() {
        let directory = Directory::new();
        let answer = |mode: &str, query: &str| {
            let output = directory.bcc(mode, &[query]);
            succeeds(&output);
            stdout(&output)
        };
        assert_eq!(answer("--gcc-mode", "-print-multi-directory"), ".\n");
        assert_eq!(answer("--clang-mode", "-print-multi-directory"), ".\n");
        assert_eq!(
            answer("--gcc-mode", "-print-multi-os-directory"),
            "../lib\n"
        );
        assert_eq!(answer("--gcc-mode", "-print-multi-lib"), ".;\n");
        assert_eq!(answer("--clang-mode", "-print-multi-lib"), ".;@m64\n");
        let output = directory.bcc("--clang-mode", &["-print-multi-os-directory", "-c", "x.c"]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("unsupported option '-print-multi-os-directory'"),
            "{}",
            stderr(&output)
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let dirs = answer(mode, "-print-search-dirs");
            let programs = dirs
                .lines()
                .find_map(|line| line.strip_prefix("programs: ="))
                .unwrap();
            assert!(!programs.ends_with("ld.lld"), "{mode}: {dirs}");
            assert!(
                dirs.lines().any(|line| line.starts_with("libraries: =")),
                "{mode}: {dirs}"
            );
            assert_eq!(
                dirs.starts_with("install: "),
                mode == "--gcc-mode",
                "{mode}: {dirs}"
            );
        }
    }

    /// `-print-libgcc-file-name` finds the library as
    /// `-print-file-name=libgcc.a` does.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn gnu_print_libgcc_file_name_is_a_path() {
        let directory = Directory::new();
        for mode in ["--gcc-mode", "--clang-mode"] {
            let named = directory.bcc(mode, &["-print-file-name=libgcc.a"]);
            let output = directory.bcc(mode, &["-print-libgcc-file-name"]);
            succeeds(&output);
            assert_eq!(stdout(&output), stdout(&named), "{mode}");
        }
    }

    /// Only Clang reads `#pragma clang deprecated`. GCC keeps it in its `-E`
    /// text, where the macro it names no longer exists, and compiles that.
    #[test]
    fn gcc_ignores_clang_macro_pragmas() {
        let directory = Directory::new();
        directory.write("m.c", "#pragma clang deprecated(GONE)\nint x;\n");
        directory.write(
            "t.c",
            "#define OLD 1\n#pragma clang deprecated(OLD)\nint x = OLD;\n",
        );
        let output = directory.bcc("--gcc-mode", &["-Werror", "-c", "m.c"]);
        succeeds(&output);
        let output = directory.bcc("--gcc-mode", &["-Werror", "-c", "t.c"]);
        succeeds(&output);
        assert!(
            !directory
                .bcc("--clang-mode", &["-c", "m.c"])
                .status
                .success()
        );
        assert!(
            !directory
                .bcc("--clang-mode", &["-Werror", "-c", "t.c"])
                .status
                .success()
        );
    }

    /// `-E` marks every change of file, with the compiler's flags, and the
    /// tokens of two files never join: two includes of a one-line header
    /// used to print `xx`.
    #[test]
    fn gnu_line_markers_mark_each_file_change() {
        let directory = Directory::new();
        directory.write("a.h", "x\n");
        directory.write("empty.h", "");
        directory.write("dup.c", "#include \"a.h\"\n#include \"a.h\"\nend\n");
        directory.write("e.c", "#include \"empty.h\"\nz\n");
        let text = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            succeeds(&output);
            stdout(&output)
        };
        assert_eq!(
            text("--clang-mode", &["-E", "dup.c"]),
            "# 1 \"dup.c\"\n# 1 \"a.h\" 1\nx\n# 2 \"dup.c\" 2\n# 1 \"a.h\" 1\nx\n# 3 \"dup.c\" \
             2\nend\n"
        );
        assert!(
            text("--clang-mode", &["-E", "e.c"]).ends_with("# 1 \"empty.h\" 1\n# 2 \"e.c\" 2\nz\n")
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            assert_eq!(text(mode, &["-E", "-P", "dup.c"]), "x\nx\nend\n", "{mode}");
            assert!(
                text(mode, &["-E", "dup.c"]).contains("# 2 \"dup.c\" 2\n"),
                "{mode}"
            );
        }
        // Clang writes flag 3 for a system header and GCC adds 4.
        #[cfg(not(windows))]
        {
            std::fs::create_dir(directory.path("sys")).unwrap();
            directory.write("sys/s.h", "int s;\n");
            directory.write("u.c", "#include <s.h>\nint u;\n");
            for (mode, flags) in [("--clang-mode", "1 3"), ("--gcc-mode", "1 3 4")] {
                let output = text(mode, &["-E", "-isystem", "sys", "u.c"]);
                assert!(
                    output.contains(&format!("# 1 \"sys/s.h\" {flags}\nint s;\n# 2 \"u.c\" 2\n")),
                    "{mode}: {output}"
                );
                // The text reads back, in the same mode.
                directory.write("u.i", &output);
                succeeds(&directory.bcc(mode, &["-fsyntax-only", "u.i"]));
            }
        }
    }

    /// GCC's and Clang's snippet and their words for the commonest errors, and
    /// a related place as a note of its own.
    #[test]
    fn gnu_diagnostics_have_the_compilers_shape() {
        let directory = Directory::new();
        directory.write("a.c", "int f(void) { return x; }\n");
        directory.write("b.c", "int outside = y;\n");
        directory.write("c.c", "int f(void) { return 1 +; }\n");
        directory.write("d.c", "int y;\nchar y;\n");
        let text = |mode: &str, file: &str| {
            let output = directory.bcc(mode, &["-fsyntax-only", file]);
            assert_eq!(output.status.code(), Some(1), "{mode} {file}");
            stderr(&output)
        };
        assert_eq!(
            text("--gcc-mode", "a.c"),
            "a.c:1:22: error: 'x' undeclared (first use in this function)\n\x20   1 | int f(void) \
             { return x; }\n\x20     |                      ^\n"
        );
        assert_eq!(
            text("--clang-mode", "a.c"),
            "a.c:1:22: error: use of undeclared identifier 'x'\n\x20   1 | int f(void) { return \
             x; }\n\x20     |                      ^\n1 error generated.\n"
        );
        assert!(
            text("--gcc-mode", "b.c")
                .starts_with("b.c:1:15: error: 'y' undeclared here (not in a function)\n")
        );
        assert!(
            text("--gcc-mode", "c.c")
                .starts_with("c.c:1:25: error: expected expression before ';' token\n")
        );
        assert!(text("--clang-mode", "c.c").starts_with("c.c:1:25: error: expected expression\n"));
        let related = text("--clang-mode", "d.c");
        assert!(
            related.contains("d.c:1:5: note: ") && related.contains("    1 | int y;\n"),
            "{related}"
        );
        // A note reports where `#line` says the place is, and keeps every
        // line of its range.
        directory.write("l.c", "#line 44\nint x;\nchar x;\n");
        assert!(text("--gcc-mode", "l.c").contains("l.c:44:5: note: "));
        directory.write("r.c", "int first extra\njunk\nmiddle\nlast;\nint after;\n");
        let recovery = text("--gcc-mode", "r.c");
        for line in ["junk", "middle", "last;"] {
            assert!(
                recovery.contains(&format!("| {line}\n")),
                "{line}: {recovery}"
            );
        }
        // GCC spells the operator, not the digraph written for it.
        directory.write("t.c", "int f(void) { return 1 + :>; }\n");
        assert!(text("--gcc-mode", "t.c").contains("expected expression before ']' token"));
        // bcc's own format keeps its labels and notes.
        let own = text("", "a.c");
        assert!(
            own.contains("not declared") && own.contains("= note:"),
            "{own}"
        );
    }

    /// `AddressSanitizer` catches a real overflow, `no_sanitize` keeps a
    /// function out of it, and the macros and feature tests answer as the
    /// compilers do.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_sanitizers_instrument_and_link() {
        let directory = Directory::new();
        directory.write(
            "o.c",
            "#include <stdlib.h>\nvolatile int n = 8;\nint main(void) { char *p = malloc(8); int \
             r = p[n]; free(p); return r; }\n",
        );
        directory.write(
            "n.c",
            "int plain(int *p) { return *p; }\n__attribute__((no_sanitize_address)) int a(int *p) \
             { return *p; }\n__attribute__((no_sanitize(\"address\"))) int b(int *p) { return *p; \
             }\n__attribute__((no_sanitize_thread)) int c(int *p) { return *p; }\n",
        );
        directory.write(
            "m.c",
            "#if __has_feature(address_sanitizer)\nint asan;\n#endif\n#if \
             __has_feature(thread_sanitizer)\nint tsan;\n#endif\n#ifdef __SANITIZE_ADDRESS__\nint \
             address;\n#endif\n#ifdef __SANITIZE_THREAD__\nint thread;\n#endif\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let build = directory.bcc(mode, &["-fsanitize=address", "-g", "o.c", "-o", "o"]);
            // A bundled toolchain built without compiler-rt can't link it, and
            // says which library is missing. The rest is checked without one.
            if !stderr(&build).contains("libclang_rt.asan") {
                succeeds(&build);
                assert_eq!(stderr(&build), "", "{mode}");
                let run = Command::new(directory.path("o"))
                    .env("ASAN_OPTIONS", "detect_leaks=0")
                    .output()
                    .unwrap();
                assert!(!run.status.success(), "{mode}");
                assert!(
                    stderr(&run).contains("AddressSanitizer: heap-buffer-overflow"),
                    "{mode}: {}",
                    stderr(&run)
                );
            }
            // Taken back by a later option.
            let args = [
                "-fsanitize=address",
                "-fno-sanitize=address",
                "-S",
                "n.c",
                "-o",
                "-",
            ];
            assert!(
                !stdout(&directory.bcc(mode, &args)).contains("__asan"),
                "{mode}"
            );
            // The functions a sanitizer checks.
            let checked = |sanitizer: &str, reference: &str| -> Vec<String> {
                let args = [sanitizer, "-O1", "-S", "n.c", "-o", "-"];
                let text = stdout(&directory.bcc(mode, &args));
                let mut current = "";
                let mut names = Vec::new();
                for line in text.lines() {
                    if let Some(name) = line.strip_suffix(':')
                        && !name.starts_with('.')
                    {
                        current = name;
                    }
                    if line.contains(reference) && !names.iter().any(|known| known == current) {
                        names.push(current.to_owned());
                    }
                }
                names
            };
            directory.write(
                "a.c",
                "__attribute__((no_sanitize(\"all\"))) int a(int *p) { return *p; }\n",
            );
            let args = ["-fsanitize=address", "-O1", "-S", "a.c", "-o", "-"];
            assert!(
                !stdout(&directory.bcc(mode, &args)).contains("__asan_report"),
                "{mode}"
            );
            assert_eq!(
                checked("-fsanitize=address", "__asan_report"),
                ["plain", "c"],
                "{mode}"
            );
            assert_eq!(
                checked("-fsanitize=thread", "__tsan_read"),
                ["plain", "a", "b"],
                "{mode}"
            );
            let defined = |option: &str| {
                stdout(&directory.bcc(mode, &[option, "-E", "-P", "m.c"]))
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            assert_eq!(
                defined("-fsanitize=address"),
                "int asan; int address;",
                "{mode}"
            );
            assert_eq!(
                defined("-fsanitize=thread"),
                "int tsan; int thread;",
                "{mode}"
            );
            // The two together are refused, unless a later option takes one
            // back.
            let both = [
                "-fsanitize=address",
                "-fsanitize=thread",
                "-c",
                "n.c",
                "-o",
                "n.o",
            ];
            assert!(!directory.bcc(mode, &both).status.success(), "{mode}");
            succeeds(&directory.bcc(
                mode,
                &[
                    "-fsanitize=address",
                    "-fsanitize=thread",
                    "-fno-sanitize=thread",
                    "-c",
                    "n.c",
                    "-o",
                    "n.o",
                ],
            ));
            // On a nested function too, which only GCC has.
            if mode == "--gcc-mode" {
                directory.write(
                    "i.c",
                    "int f(int *p) {\n    __attribute__((no_sanitize_address)) int inner(int *q) \
                     {\n        return *q;\n    }\n    return inner(p);\n}\n",
                );
                let args = ["-fsanitize=address", "-S", "i.c", "-o", "-"];
                let output = stdout(&directory.bcc(mode, &args));
                assert!(!output.contains("__asan_report"), "{output}");
            }
            // The rest stay refused, and off Linux too.
            let refused = |args: &[&str]| {
                !directory
                    .bcc(mode, &[args, &["-c", "n.c", "-o", "n.o"]].concat())
                    .status
                    .success()
            };
            assert!(refused(&["-fsanitize=undefined"]), "{mode}");
            assert!(
                refused(&["--target=x86_64-w64-mingw32", "-fsanitize=address"]),
                "{mode}"
            );
        }
    }

    /// `-pg` builds a program `gprof` can read: each function calls `mcount`
    /// unless it says otherwise, and a run writes `gmon.out`.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_profiling_instruments_and_links() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "__attribute__((noinline)) int f(int x) { return x + 1; \
             }\n__attribute__((no_instrument_function)) int g(int x) { return x + 2; }\nint \
             main(void) { return f(1) + g(1) == 5 ? 0 : 1; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let text = stdout(&directory.bcc(mode, &["-pg", "-O1", "-S", "a.c", "-o", "-"]));
            let calls = text.matches("mcount").count();
            assert!(calls >= 2, "{mode}: {text}");
            let g = text
                .split("\ng:")
                .nth(1)
                .and_then(|rest| rest.split(".size").next());
            assert!(
                g.is_some_and(|body| !body.contains("mcount")),
                "{mode}: {text}"
            );
            let plain = stdout(&directory.bcc(mode, &["-O1", "-S", "a.c", "-o", "-"]));
            assert!(!plain.contains("mcount"), "{mode}");
            // A run writes the profile, where the toolchain has `gcrt1.o`.
            let build = directory.bcc(mode, &["-pg", "-O1", "a.c", "-o", "a"]);
            if build.status.success() {
                assert_eq!(stderr(&build), "", "{mode}");
                drop(std::fs::remove_file(directory.path("gmon.out")));
                let run = Command::new(directory.path("a"))
                    .current_dir(directory.path("."))
                    .status()
                    .unwrap();
                assert_eq!(run.code(), Some(0), "{mode}");
                assert!(directory.path("gmon.out").exists(), "{mode}");
            } else {
                assert!(stderr(&build).contains("gcrt1"), "{}", stderr(&build));
            }
            // Each compiler refuses it with no frame pointer, unless taken
            // back.
            let both = ["-pg", "-fomit-frame-pointer", "-c", "a.c", "-o", "a.o"];
            assert!(!directory.bcc(mode, &both).status.success(), "{mode}");
            let back = [
                "-pg",
                "-fomit-frame-pointer",
                "-fno-omit-frame-pointer",
                "-c",
                "a.c",
                "-o",
                "a.o",
            ];
            succeeds(&directory.bcc(mode, &back));
        }
    }

    /// `ifunc`: a resolver chooses the definition when the program loads.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_ifunc_runs_what_its_resolver_chooses() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "static int one(void) { return 1; }\nstatic int two(void) { return 2; }\nstatic int \
             (*resolve(void))(void) { return two; }\nint f(void) \
             __attribute__((ifunc(\"resolve\")));\nstatic int s(void) \
             __attribute__((ifunc(\"resolve\")));\nint main(void) {\n    (void)one;\n    int \
             (*p)(void) = f;\n    return f() + s() + p() == 6 ? 0 : 1;\n}\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            for optimization in ["-O0", "-O2"] {
                let args = [optimization, "a.c", "-o", "a"];
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("a")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {optimization}");
            }
            for (source, why) in [
                (
                    "int f(void) __attribute__((ifunc(\"r\")));\n",
                    "no resolver",
                ),
                (
                    "int r(void);\nint f(void) __attribute__((ifunc(\"r\")));\n",
                    "declared",
                ),
                (
                    "static int r(void) { return 0; }\nint f(void) \
                     __attribute__((ifunc(\"r\")));\n",
                    "no pointer",
                ),
                (
                    "static void *r(void) { return 0; }\nint f(void) \
                     __attribute__((ifunc(\"r\")));\nint f(void) { return 1; }\n",
                    "a definition too",
                ),
            ] {
                directory.write("e.c", source);
                let output = directory.bcc(mode, &["-c", "e.c", "-o", "e.o"]);
                assert!(!output.status.success(), "{mode} {why}");
            }
            // On an object it's ignored with a warning, as in both.
            directory.write(
                "v.c",
                "static void *r(void) { return 0; }\nint v __attribute__((ifunc(\"r\")));\n",
            );
            let output = directory.bcc(mode, &["-c", "v.c", "-o", "v.o"]);
            succeeds(&output);
            assert!(stderr(&output).contains("ifunc"), "{mode}");
            // A static alias has its target's definition too.
            directory.write(
                "s.c",
                "static int x(void) { return 1; }\nstatic int s(void) \
                 __attribute__((alias(\"x\")));\nint main(void) { return s() - 1; }\n",
            );
            succeeds(&directory.bcc(mode, &["s.c", "-o", "s"]));
            let run = Command::new(directory.path("s")).status().unwrap();
            assert_eq!(run.code(), Some(0), "{mode}");
            // Clang knows no such attribute off glibc's targets.
            directory.write(
                "w.c",
                "static void *r(void) { return 0; }\nint f(void) __attribute__((ifunc(\"r\")));\n",
            );
            let args = ["--target=x86_64-w64-mingw32", "-c", "w.c", "-o", "w.o"];
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            assert!(stderr(&output).contains("ifunc"), "{mode}");
        }
    }

    /// `weakref`: another name for a symbol, weak unless this unit declares it.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_weakref_names_a_symbol_that_may_be_absent() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "static int w(void) __attribute__((weakref(\"absent_function\")));\nstatic int v \
             __attribute__((weakref(\"absent_object\")));\nstatic int t(void) { return 7; \
             }\nstatic int same(void) __attribute__((weakref(\"t\")));\nstatic int also(void) \
             __attribute__((weakref, alias(\"t\")));\nint main(void) {\n    if (w) return 1;\n    \
             if (&v) return 2;\n    return same() + also() == 14 ? 0 : 3;\n}\n",
        );
        directory.write("b.c", "int absent_function(void) { return 1; }\n");
        directory.write(
            "c.c",
            "static int w(void) __attribute__((weakref(\"absent_function\")));\nint main(void) { \
             return w ? w() - 1 : 9; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            for optimization in ["-O0", "-O2"] {
                let args = [optimization, "a.c", "-o", "a"];
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("a")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {optimization}");
                // When another object defines it, the reference finds it.
                let args = [optimization, "c.c", "b.c", "-o", "c"];
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("c")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {optimization}");
            }
            // Only a static declaration can have one, in both compilers.
            directory.write("e.c", "int w(void) __attribute__((weakref(\"t\")));\n");
            let args = ["-c", "e.c", "-o", "e.o"];
            assert!(!directory.bcc(mode, &args).status.success(), "{mode}");
        }
        for mode in ["--gcc-mode", "--clang-mode"] {
            // A declaration of the target leaves it weak, for a function and
            // for an object, as both compilers do.
            for source in [
                "extern int t(void);\nstatic int w(void) __attribute__((weakref(\"t\")));\nint \
                 main(void) { return w ? w() : 0; }\n",
                "extern int t;\nstatic int w __attribute__((weakref(\"t\")));\nint main(void) { \
                 return &w ? 1 : 0; }\n",
            ] {
                directory.write("d.c", source);
                succeeds(&directory.bcc(mode, &["d.c", "-o", "d"]));
                let run = Command::new(directory.path("d")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {source}");
            }
            // A use of the target itself is an ordinary reference, which the
            // link then needs.
            for source in [
                "extern int t(void);\nstatic int w(void) __attribute__((weakref(\"t\")));\nint \
                 main(void) { t(); return w ? w() : 0; }\n",
                "extern int t;\nstatic int w __attribute__((weakref(\"t\")));\nint main(void) { \
                 return t + (&w ? 1 : 0); }\n",
            ] {
                directory.write("r.c", source);
                let output = directory.bcc(mode, &["r.c", "-o", "r"]);
                assert!(!output.status.success(), "{mode} {source}");
            }
        }
        // Another declaration of the same function has the same weakref.
        directory.write(
            "t.c",
            "static int w(void) __attribute__((weakref(\"absent\")));\nstatic int w(void);\nint \
             main(void) { return w ? 1 : 0; }\n",
        );
        succeeds(&directory.bcc("--gcc-mode", &["t.c", "-o", "t"]));
        let run = Command::new(directory.path("t")).status().unwrap();
        assert_eq!(run.code(), Some(0));
        // A cycle reaches nothing: GCC refuses it, and Clang's program faults
        // when it calls one, so bcc has to finish either way.
        directory.write(
            "y.c",
            "static int a(void) __attribute__((weakref(\"b\")));\nstatic int b(void) \
             __attribute__((weakref(\"a\")));\nint main(void) { return a(); }\n",
        );
        let args = ["-c", "y.c", "-o", "y.o"];
        let output = directory.bcc("--gcc-mode", &args);
        assert!(!output.status.success());
        assert!(stderr(&output).contains("cycle"), "{}", stderr(&output));
        succeeds(&directory.bcc("--clang-mode", &args));
        // Clang wants a target; GCC is left with a declaration.
        directory.write("n.c", "static int w(void) __attribute__((weakref));\n");
        let args = ["-c", "n.c", "-o", "n.o"];
        assert!(!directory.bcc("--clang-mode", &args).status.success());
        succeeds(&directory.bcc("--gcc-mode", &args));
    }

    /// A distribution's hardening options do what they say on Linux, with the
    /// macros each compiler numbers its own way.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_hardening_options_take_effect() {
        let directory = Directory::new();
        directory.write(
            "s.c",
            "void g(char *);\nint f(int n) { char a[16]; g(a); return n; }\nint leaf(int n) { \
             return n + 1; }\n",
        );
        let assembly = |mode: &str, options: &[&str]| {
            let args = [options, &["-O2", "-S", "s.c", "-o", "-"]].concat();
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode} {options:?}");
            stdout(&output)
        };
        let macros = |mode: &str, options: &[&str]| {
            let args = [options, &["-E", "-dM", "s.c"]].concat();
            stdout(&directory.bcc(mode, &args))
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            let plain = assembly(mode, &[]);
            assert!(!plain.contains("__stack_chk_fail"), "{mode}");
            assert!(!plain.contains("endbr64"), "{mode}");
            for option in [
                "-fstack-protector",
                "-fstack-protector-strong",
                "-fstack-protector-all",
            ] {
                assert!(
                    assembly(mode, &[option]).contains("__stack_chk_fail"),
                    "{mode} {option}"
                );
            }
            assert!(
                !assembly(mode, &["-fstack-protector-strong", "-fno-stack-protector"])
                    .contains("__stack_chk_fail"),
                "{mode}"
            );
            let protected = assembly(mode, &["-fcf-protection=branch"]);
            assert!(protected.contains("endbr64"), "{mode}");
            assert!(
                !assembly(mode, &["-fcf-protection=return"]).contains("endbr64"),
                "{mode}"
            );
            assert!(
                !assembly(mode, &["-fcf-protection=full", "-fcf-protection=none"])
                    .contains("endbr64"),
                "{mode}"
            );
            let framed = assembly(mode, &["-fno-omit-frame-pointer"]);
            assert!(framed.contains("%rbp"), "{mode}");
            assert!(!plain.contains("%rbp"), "{mode}");
            let macro_of = |options: &[&str], line: &str| macros(mode, options).contains(line);
            assert!(
                macro_of(&["-fstack-protector"], "#define __SSP__ 1\n"),
                "{mode}"
            );
            assert!(
                macro_of(&["-fcf-protection"], "#define __CET__ 3\n"),
                "{mode}"
            );
            assert!(
                macro_of(&["-fcf-protection=branch"], "#define __CET__ 1\n"),
                "{mode}"
            );
            assert!(
                macro_of(&["-fcf-protection=return"], "#define __CET__ 2\n"),
                "{mode}"
            );
            // GCC adds to what an earlier option asked for, and Clang replaces
            // it.
            let both = ["-fcf-protection=branch", "-fcf-protection=return"];
            let added = if mode == "--gcc-mode" { 3 } else { 2 };
            assert!(
                macro_of(&both, &format!("#define __CET__ {added}\n")),
                "{mode}"
            );
            let (strong, all) = if mode == "--gcc-mode" { (3, 2) } else { (2, 3) };
            assert!(
                macro_of(
                    &["-fstack-protector-strong"],
                    &format!("#define __SSP_STRONG__ {strong}\n")
                ),
                "{mode}"
            );
            assert!(
                macro_of(
                    &["-fstack-protector-all"],
                    &format!("#define __SSP_ALL__ {all}\n")
                ),
                "{mode}"
            );
            // The attribute keeps a function out, whatever the options ask.
            directory.write(
                "n.c",
                "void g(char *);\n__attribute__((no_stack_protector)) void a(void) { char b[32]; \
                 g(b); }\nvoid c(void) { char b[32]; g(b); }\n",
            );
            let args = ["-fstack-protector-all", "-O2", "-S", "n.c", "-o", "-"];
            let output = stdout(&directory.bcc(mode, &args));
            assert_eq!(
                output.matches("callq\t__stack_chk_fail").count(),
                1,
                "{mode}: {output}"
            );
            // On a nested function too, which only GCC has.
            if mode == "--gcc-mode" {
                directory.write(
                    "i.c",
                    "void g(char *);\nvoid f(void) {\n    __attribute__((no_stack_protector)) \
                     void inner(void) {\n        char b[32];\n        g(b);\n    }\n    \
                     inner();\n}\n",
                );
                let args = ["-fstack-protector-all", "-S", "i.c", "-o", "-"];
                let output = stdout(&directory.bcc(mode, &args));
                assert_eq!(
                    output.matches("callq\t__stack_chk_fail").count(),
                    1,
                    "{output}"
                );
            }
            // The program stops at a smashed canary, and runs without one.
            directory.write(
                "o.c",
                "#include <string.h>\nvolatile int n = 64;\nint main(void) { char a[8]; memset(a, \
                 1, n); return a[0] == 2; }\n",
            );
            let args = ["-O1", "-fstack-protector-all", "o.c", "-o", "o"];
            succeeds(&directory.bcc(mode, &args));
            assert_ne!(
                Command::new(directory.path("o"))
                    .output()
                    .unwrap()
                    .status
                    .code(),
                Some(0),
                "{mode}"
            );
            // Large frames probe each page.
            directory.write(
                "c.c",
                "void g(char *);\nvoid f(void) { char a[70000]; g(a); }\n",
            );
            let probed = |options: &[&str]| {
                let args = [options, &["-O2", "-S", "c.c", "-o", "-"]].concat();
                stdout(&directory.bcc(mode, &args)).contains("$4096")
            };
            assert!(probed(&["-fstack-clash-protection"]), "{mode}");
            assert!(!probed(&[]), "{mode}");
        }
        // Windows hasn't got them, and says so.
        let output = directory.bcc(
            "--gcc-mode",
            &[
                "--target=x86_64-w64-mingw32",
                "-fstack-protector-strong",
                "-c",
                "s.c",
                "-o",
                "w.o",
            ],
        );
        succeeds(&output);
        assert!(
            stderr(&output).contains("is ignored"),
            "{}",
            stderr(&output)
        );
    }

    /// Constant arithmetic C99 §6.6p4 gives no value, in a static object and
    /// in code, with the values GCC 13.3 and Clang 23.1 give it (`None`: no
    /// constant). A static `int` or `long long` prints each.
    #[cfg(target_os = "linux")]
    #[test]
    fn exceptional_constants_fold_to_each_compilers_value() {
        use std::fmt::Write as _;
        const MIN: &str = "-2147483648";
        const MIN64: &str = "-9223372036854775808";
        type Row = (&'static str, Option<&'static str>, Option<&'static str>);
        let ints: [Row; 37] = [
            ("2147483647+1", Some(MIN), Some(MIN)),
            ("-2147483647-2", Some("2147483647"), Some("2147483647")),
            ("-(-2147483647-1)", Some(MIN), Some(MIN)),
            ("2147483647*2", Some("-2"), Some("-2")),
            ("(-2147483647-1)/-1", Some(MIN), Some(MIN)),
            ("(-2147483647-1)%-1", Some("0"), Some("0")),
            ("1<<40", Some("0"), Some(MIN)),
            ("1<<32", Some("0"), Some(MIN)),
            ("1u<<32", Some("0"), Some(MIN)),
            ("(1<<40)+0", Some("0"), Some(MIN)),
            ("~0<<40", Some("0"), Some(MIN)),
            ("1>>40", Some("0"), Some("0")),
            ("-1>>40", Some("-1"), Some("-1")),
            ("-5>>33", Some("-1"), Some("-1")),
            ("1<<-1", None, Some("0")),
            ("1>>-1", None, Some("2")),
            ("4<<-1", None, Some("2")),
            ("4>>-1", None, Some("8")),
            ("1<<-40", None, Some("0")),
            ("-1<<-1", None, Some("-1")),
            ("-1>>-1", Some("-1"), Some("-2")),
            ("0<<-1", Some("0"), Some("0")),
            ("1<<(1LL<<33)", Some("1"), Some(MIN)),
            ("1<<(unsigned)4294967295u", None, Some(MIN)),
            ("1?(2147483647+1):2", Some(MIN), Some(MIN)),
            ("0?2:(1<<40)", Some("0"), Some(MIN)),
            ("(2147483647+1)/2", Some("-1073741824"), Some("-1073741824")),
            ("(1<<40)==0", Some("1"), Some("0")),
            ("(1<<40)!=0", Some("0"), Some("1")),
            ("0||(1<<40)", Some("0"), Some("1")),
            ("!(1<<40)", Some("1"), Some("0")),
            ("~(1<<40)", Some("-1"), Some("2147483647")),
            ("(1<<40)|1", Some("1"), Some("-2147483647")),
            ("(char)1<<40", Some("0"), Some(MIN)),
            ("1<<31", Some(MIN), Some(MIN)),
            ("3<<31", Some(MIN), Some(MIN)),
            ("-1<<1", Some("-2"), Some("-2")),
        ];
        let longs: [Row; 6] = [
            ("1LL<<64", Some("0"), Some(MIN64)),
            ("1ULL>>64", Some("0"), Some("0")),
            ("-1LL>>64", Some("-1"), Some("-1")),
            ("9223372036854775807LL*2", Some("-2"), Some("-2")),
            ("-(-9223372036854775807LL-1)", Some(MIN64), Some(MIN64)),
            ("1LL>>-1", None, Some("2")),
        ];
        let directory = Directory::new();
        for (mode, gcc) in [("--gcc-mode", true), ("--clang-mode", false)] {
            let select = |row: &Row| if gcc { row.1 } else { row.2 };
            let mut source = String::from("int printf(const char *, ...);\n");
            let mut body = String::new();
            let mut expected = String::new();
            let tables = [("int", "%d", &ints[..]), ("long long", "%lld", &longs[..])];
            for (table, (ty, format, rows)) in tables.into_iter().enumerate() {
                for (index, row) in rows.iter().enumerate() {
                    let probe = format!("{ty} x = {};\n", row.0);
                    directory.write("probe.c", &probe);
                    let output = directory.bcc(mode, &["-w", "-fsyntax-only", "probe.c"]);
                    assert_eq!(
                        output.status.success(),
                        select(row).is_some(),
                        "{mode} {}: {}",
                        row.0,
                        stderr(&output)
                    );
                    if let Some(value) = select(row) {
                        writeln!(source, "{ty} v{table}_{index} = {};", row.0).unwrap();
                        writeln!(body, "printf(\"{format}\\n\", v{table}_{index});").unwrap();
                        expected.push_str(value);
                        expected.push('\n');
                    }
                }
            }
            writeln!(source, "int main(void) {{\n{body}return 0;\n}}").unwrap();
            directory.write("statics.c", &source);
            succeeds(&directory.bcc(mode, &["-w", "statics.c", "-o", "statics"]));
            let run = Command::new(directory.path("statics")).output().unwrap();
            assert_eq!(stdout(&run), expected, "{mode}");

            // Code reads what GCC folds. Clang's code computes poison from
            // an undefined shift or division, which reads as whatever a
            // register held (551572928 in one run of its build), so that
            // column is `None`; bcc reads 0.
            let code: [(&str, &str, Option<&str>); 9] = [
                ("2147483647+1", MIN, Some(MIN)),
                ("-(-2147483647-1)", MIN, Some(MIN)),
                ("1<<40", "0", None),
                ("-1>>40", "-1", None),
                ("(1<<40)+1", "1", None),
                ("(-2147483647-1)/-1", MIN, None),
                ("(-2147483647-1)%-1", "0", None),
                ("(2147483647+1)>>1", "-1073741824", Some("-1073741824")),
                ("1?(2147483647+1):2", MIN, Some(MIN)),
            ];
            let mut source = String::from("int printf(const char *, ...);\nint main(void) {\n");
            for (expression, ..) in code {
                writeln!(source, "printf(\"%d\\n\", {expression});").unwrap();
            }
            source.push_str("return 0;\n}\n");
            directory.write("code.c", &source);
            succeeds(&directory.bcc(mode, &["-w", "code.c", "-o", "code"]));
            let run = Command::new(directory.path("code")).output().unwrap();
            let run = stdout(&run);
            let lines: Vec<&str> = run.lines().collect();
            assert_eq!(lines.len(), code.len(), "{mode}: {run}");
            for (line, (expression, gcc_value, clang_value)) in lines.iter().zip(code) {
                let expected = if gcc {
                    gcc_value
                } else {
                    clang_value.unwrap_or("0")
                };
                assert_eq!(*line, expected, "{mode} {expression}");
            }
        }
    }

    /// What GCC 14.2 and Clang 23.1 say of exceptional constants in each
    /// context (`W`: warns, `E`: refuses), by the first words of the reports.
    #[test]
    fn exceptional_constants_warn_in_each_compilers_words() {
        const GCC_ADD: &str =
            "integer overflow in expression of type 'int' results in '-2147483648' [-Woverflow]";
        const CLANG_ADD: &str =
            "overflow in expression; result is -2'147'483'648 with type 'int' [-Winteger-overflow]";
        const GCC_SHL: &str = "left shift count >= width of type [-Wshift-count-overflow]";
        const GCC_SHR: &str = "right shift count >= width of type [-Wshift-count-overflow]";
        const CLANG_COUNT: &str = "shift count >= width of type [-Wshift-count-overflow]";
        const GCC_NEGATIVE: &str = "left shift count is negative [-Wshift-count-negative]";
        const CLANG_NEGATIVE: &str = "shift count is negative [-Wshift-count-negative]";
        const GCC_ZERO: &str = "division by zero [-Wdiv-by-zero]";
        const CLANG_ZERO: &str = "division by zero is undefined [-Wdivision-by-zero]";
        const FOLDED: &str = "variable length array folded to constant array as an extension \
                              [-Wgnu-folding-constant]";
        type Row = (
            &'static str,
            bool,
            &'static [&'static str],
            bool,
            &'static [&'static str],
        );
        let table: [Row; 27] = [
            ("int x=2147483647+1;", true, &[GCC_ADD], true, &[CLANG_ADD]),
            (
                "int x=-(-2147483647-1);",
                true,
                &[
                    "integer overflow in expression '-2147483648' of type 'int' results in \
                     '-2147483648' [-Woverflow]",
                ],
                true,
                &[CLANG_ADD],
            ),
            (
                "long long x=9223372036854775807LL*2;",
                true,
                &[
                    "integer overflow in expression of type 'long long int' results in '-2' \
                     [-Woverflow]",
                ],
                true,
                &[
                    "overflow in expression; result is -2 with type 'long long' \
                     [-Winteger-overflow]",
                ],
            ),
            ("int x=(-2147483647-1)/-1;", true, &[GCC_ADD], true, &[]),
            ("int x=1<<40;", true, &[GCC_SHL], true, &[CLANG_COUNT]),
            ("int x=1>>40;", true, &[GCC_SHR], true, &[CLANG_COUNT]),
            (
                "int x=1<<-1;",
                false,
                &[GCC_NEGATIVE],
                true,
                &[CLANG_NEGATIVE],
            ),
            ("int x=1/0;", false, &[GCC_ZERO], false, &[CLANG_ZERO]),
            (
                "enum{A=2147483647+1};",
                true,
                &[GCC_ADD],
                true,
                &[CLANG_ADD],
            ),
            ("enum{A=1<<40};", true, &[GCC_SHL], true, &[]),
            ("enum{A=1<<-1};", false, &[GCC_NEGATIVE], true, &[]),
            ("enum{A=1/0};", false, &[GCC_ZERO], false, &[]),
            (
                "_Static_assert(2147483647+1<0,\"x\");",
                true,
                &[GCC_ADD],
                true,
                &[CLANG_ADD],
            ),
            ("int a[(1<<40)+1];", false, &[GCC_SHL], false, &[FOLDED]),
            (
                "int a[2147483647+1>0?1:2];",
                false,
                &[GCC_ADD],
                true,
                &[CLANG_ADD, FOLDED],
            ),
            (
                "void f(void){int a[(1>>40)+1];(void)a;}",
                true,
                &[GCC_SHR],
                true,
                &[FOLDED],
            ),
            ("int x=sizeof(2147483647+1);", true, &[], true, &[]),
            ("int x=sizeof(1/0);", true, &[], true, &[]),
            ("int x=0&&(2147483647+1);", true, &[], true, &[]),
            ("int x=0&&(1/0);", true, &[], true, &[]),
            ("int x=1?2:(1<<-1);", true, &[], true, &[]),
            ("int x=1?(2147483647+1):2;", true, &[GCC_ADD], true, &[]),
            (
                "int f(int y){return y<<40;}",
                true,
                &[GCC_SHL],
                true,
                &[CLANG_COUNT],
            ),
            (
                "int f(int y){return y/0;}",
                true,
                &[GCC_ZERO],
                true,
                &[CLANG_ZERO],
            ),
            (
                "int f(int y){return y?2147483647+1:0;}",
                true,
                &[GCC_ADD],
                true,
                &[],
            ),
            (
                "int f(int y){return y+(2147483647+1);}",
                true,
                &[GCC_ADD],
                true,
                &[CLANG_ADD],
            ),
            (
                "int f(int y){if(0)y=1<<40;return y;}",
                true,
                &[GCC_SHL],
                true,
                &[],
            ),
        ];
        let directory = Directory::new();
        for (source, gcc_accepts, gcc_words, clang_accepts, clang_words) in table {
            directory.write("c.c", &format!("{source}\n"));
            for (mode, accepts, words) in [
                ("--gcc-mode", gcc_accepts, gcc_words),
                ("--clang-mode", clang_accepts, clang_words),
            ] {
                let output = directory.bcc(mode, &["-fsyntax-only", "c.c"]);
                let text = stderr(&output);
                assert_eq!(output.status.success(), accepts, "{mode} {source}: {text}");
                let warnings = text
                    .lines()
                    .filter(|line| line.contains(": warning: "))
                    .count();
                assert_eq!(warnings, words.len(), "{mode} {source}: {text}");
                for word in words {
                    assert!(text.contains(word), "{mode} {source}: {text}");
                }
            }
        }
    }

    /// Wider types and bit-precise ones of any width fold the same way, as
    /// assertions accepted by GCC 14.2 and by Clang 23.1 show.
    #[test]
    fn exceptional_constants_fold_at_every_width() {
        let table = [
            ("((_BitInt(200))1<<200)==0", true, false),
            ("((_BitInt(200))1<<200)<0", false, true),
            ("((_BitInt(200))-1>>200)==-1", true, true),
            ("(((_BitInt(200))1<<198)*4)==0", true, true),
            ("(-(-((_BitInt(200))1<<198)*2))<0", true, true),
            ("((_BitInt(200))1<<-1)==0", false, true),
            ("((_BitInt(200))1>>-1)==2", false, true),
            ("(1>>-((_BitInt(256))1<<200))==(-2147483647-1)", false, true),
            ("(1<<-((_BitInt(256))1<<200))==0", false, true),
            ("((unsigned _BitInt(200))1<<200)==0", true, false),
            ("((_BitInt(8))127+(_BitInt(8))1)==-128", true, true),
            ("((_BitInt(8))1<<8)==0", true, false),
            ("((_BitInt(8))1<<8)==-128", false, true),
            ("((_BitInt(8))-128/(_BitInt(8))-1)==-128", true, true),
            ("((__int128)1<<130)==0", true, false),
            ("((__int128)1<<130)<0", false, true),
            ("(((__int128)1<<126)*4)==0", true, true),
        ];
        let directory = Directory::new();
        for (condition, gcc, clang) in table {
            directory.write("c.c", &format!("_Static_assert({condition},\"x\");\n"));
            for (mode, accepts) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(mode, &["-w", "-fsyntax-only", "c.c"]);
                assert_eq!(
                    output.status.success(),
                    accepts,
                    "{mode} {condition}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// The options and pragmas that silence or promote each warning, and
    /// that plain bcc keeps refusing these constants.
    #[test]
    fn exceptional_constant_warnings_follow_their_options() {
        let directory = Directory::new();
        directory.write(
            "c.c",
            "int a=2147483647+1;\nint b=1<<40;\nint c=-1>>-1;\nint f(int y){return y/0+(y<<-1);}\n",
        );
        let warnings = |mode: &str, flags: &[&str]| {
            let mut arguments = flags.to_vec();
            arguments.extend(["-fsyntax-only", "c.c"]);
            let output = directory.bcc(mode, &arguments);
            (output.status.success(), stderr(&output))
        };
        for (mode, overflow, division) in [
            ("--gcc-mode", "overflow", "div-by-zero"),
            ("--clang-mode", "integer-overflow", "division-by-zero"),
        ] {
            let (ok, text) = warnings(mode, &[]);
            assert!(ok, "{text}");
            assert!(text.contains(&format!("[-W{overflow}]")), "{text}");
            assert!(text.contains(&format!("[-W{division}]")), "{text}");
            let silence = format!("-Wno-{overflow}");
            let (ok, text) = warnings(mode, &[&silence]);
            assert!(ok && !text.contains(&format!("[-W{overflow}]")), "{text}");
            let (ok, text) = warnings(mode, &["-Wno-shift-count-overflow"]);
            assert!(ok && !text.contains("[-Wshift-count-overflow]"), "{text}");
            let (ok, text) = warnings(mode, &["-Wno-shift-count-negative"]);
            assert!(ok && !text.contains("[-Wshift-count-negative]"), "{text}");
            let (ok, text) = warnings(mode, &["-Wno-div-by-zero"]);
            assert!(ok && !text.contains(&format!("[-W{division}]")), "{text}");
            let (ok, text) = warnings(mode, &["-w"]);
            assert!(ok && text.is_empty(), "{text}");
            let (ok, text) = warnings(mode, &["-Werror"]);
            assert!(!ok && text.contains("error: "), "{text}");
            assert!(text.contains(overflow), "{text}");
            directory.write(
                "p.c",
                &format!(
                    "#pragma GCC diagnostic ignored \"-W{overflow}\"\nint a=2147483647+1;\n                     #pragma GCC diagnostic error \"-Wshift-count-overflow\"\nint b=1<<40;\n"
                ),
            );
            let output = directory.bcc(mode, &["-fsyntax-only", "p.c"]);
            let text = stderr(&output);
            assert!(!output.status.success(), "{text}");
            assert!(!text.contains(&format!("[-W{overflow}]")), "{text}");
            assert!(text.contains("shift count"), "{text}");
        }
        // GCC's option does not reach Clang's warning, or Clang's GCC's.
        let (_, text) = warnings("--clang-mode", &["-Wno-overflow"]);
        assert!(text.contains("[-Winteger-overflow]"), "{text}");
        let (_, text) = warnings("--gcc-mode", &["-Wno-integer-overflow"]);
        assert!(text.contains("[-Woverflow]"), "{text}");
        // Plain bcc keeps the constraint of C99 §6.6p4.
        for source in ["int x=2147483647+1;", "int x=1<<40;", "int x=1/0;"] {
            directory.write("plain.c", &format!("{source}\n"));
            let output = directory.bcc("", &["plain.c"]);
            assert!(!output.status.success(), "{source}");
            assert!(
                stderr(&output).contains("constant expression overflows"),
                "{source}: {}",
                stderr(&output)
            );
        }
    }

    /// What an address's truth gives a static object, with the values GCC
    /// 13.3 and Clang 23.1 store (`None`: refused). No symbol is referenced,
    /// so `g` and `f` need no definition.
    #[cfg(target_os = "linux")]
    #[test]
    fn address_truth_folds_in_static_initializers() {
        type Row = (
            &'static str,
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
        );
        let table: [Row; 33] = [
            ("extern int g;", "_Bool x=&g;", Some("1"), Some("1")),
            (
                "extern int g;",
                "struct{_Bool b;}s={&g};_Bool x=1;",
                Some("1"),
                Some("1"),
            ),
            ("void f(void);", "_Bool x=f;", Some("1"), Some("1")),
            ("void f(void);", "_Bool x=&f;", Some("1"), Some("1")),
            ("", "_Bool x=(void*)0;", Some("0"), Some("0")),
            ("", "_Bool x=(void*)1;", Some("1"), Some("1")),
            ("int a[3];", "_Bool x=a;", Some("1"), Some("1")),
            ("int a[3];", "_Bool x=a+1;", None, Some("1")),
            ("int a[3];", "_Bool x=&a[1];", None, Some("1")),
            ("int a[3];", "_Bool x=&a[0];", None, Some("1")),
            ("", "_Bool x=\"x\";", None, Some("1")),
            ("extern int g;", "_Bool x=(char*)&g;", Some("1"), Some("1")),
            ("extern int g;", "_Bool x=(long)&g;", None, Some("1")),
            (
                "__attribute__((weak)) int g;",
                "_Bool x=&g;",
                Some("1"),
                None,
            ),
            (
                "extern int g __attribute__((weak));",
                "_Bool x=&g;",
                None,
                None,
            ),
            (
                "void f(void) __attribute__((weak));",
                "_Bool x=f;",
                None,
                None,
            ),
            ("extern int g;", "int x=!&g;", Some("0"), Some("0")),
            ("extern int g;", "int x=&g?1:2;", Some("1"), Some("1")),
            ("extern int g;", "int x=!!&g;", Some("1"), Some("1")),
            ("extern int g;", "int x=&g==0;", Some("0"), Some("0")),
            ("extern int g;", "int x=&g!=0;", Some("1"), Some("1")),
            ("static int g;", "int x=&g&&1;", Some("1"), Some("1")),
            ("void f(void);", "int x=!f;", Some("0"), Some("0")),
            ("int a[3];", "int x=(a+1)!=0;", Some("1"), Some("1")),
            ("", "int x=\"x\"==0;", Some("0"), Some("0")),
            ("", "int x=(void*)0?1:2;", Some("2"), Some("2")),
            ("int g,h;", "int x=&g==&h;", Some("0"), Some("0")),
            ("int g;", "int x=&g<&g;", Some("0"), Some("0")),
            ("int g;", "int x=&g>0;", Some("1"), None),
            ("int g;", "int x=(long)&g!=0;", Some("1"), None),
            ("int g;", "int x=(!&g)+5;", Some("5"), Some("5")),
            ("int y;int g;", "int x=!&g&&y;", Some("0"), Some("0")),
            ("int y;int g;", "int x=y&&&g;", None, None),
        ];
        let directory = Directory::new();
        for (declarations, definition, gcc, clang) in table {
            directory.write(
                "c.c",
                &format!(
                    "int printf(const char *, ...);\n{declarations}\n{definition}\nint main(void) \
                     {{ printf(\"%d\", (int)x); return 0; }}\n"
                ),
            );
            for (mode, expected) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(mode, &["-w", "c.c", "-o", "c"]);
                assert_eq!(
                    output.status.success(),
                    expected.is_some(),
                    "{mode} {declarations} {definition}: {}",
                    stderr(&output)
                );
                if let Some(expected) = expected {
                    let run = Command::new(directory.path("c")).output().unwrap();
                    assert_eq!(stdout(&run), expected, "{mode} {definition}");
                }
            }
        }
        // A constant context reads it too, where the compiler folds it.
        for (source, gcc, clang) in [
            ("int g;enum{A=!&g};", true, true),
            ("int g;_Static_assert(&g!=0,\"x\");", true, true),
            ("int g;_Static_assert(!!&g,\"x\");", true, true),
            ("int g;_Static_assert(&g,\"x\");", false, true),
            ("int g;void h(int x){switch(x){case !&g:;}}", true, true),
            ("int g;struct{int b:!&g+1;}s;", true, true),
            ("void h(void){int g;int a[!&g+1];(void)a;}", true, true),
        ] {
            directory.write("c.c", &format!("{source}\n"));
            for (mode, accepts) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(mode, &["-w", "-fsyntax-only", "c.c"]);
                assert_eq!(
                    output.status.success(),
                    accepts,
                    "{mode} {source}: {}",
                    stderr(&output)
                );
            }
        }
        // Plain bcc stores `_Bool b = &g;` as the constraint allows, and
        // keeps refusing the other forms, which only §6.6p10 could admit.
        directory.write(
            "c.c",
            "extern int g;\n_Bool b = &g;\nint main(void) { return !b; }\n",
        );
        succeeds(&directory.bcc("", &["-w", "c.c", "-o", "c"]));
        assert!(
            Command::new(directory.path("c"))
                .status()
                .unwrap()
                .success()
        );
        directory.write("c.c", "extern int g;\nint i = !&g;\n");
        assert!(!directory.bcc("", &["-w", "c.c"]).status.success());
    }

    /// `-pedantic` at each standard, as GCC 14.2 and Clang 23.1 answer: whether
    /// the program is accepted, and whether it says anything then.
    #[test]
    fn pedantic_reports_follow_each_compiler() {
        const ALL: &[&str] = &["c89", "c99", "c11", "gnu89", "gnu17", "c23"];
        const GNU: &[&str] = &["gnu89", "gnu17"];
        const C89: &[&str] = &["c89", "gnu89"];
        const ISO_BEFORE_C23: &[&str] = &["c89", "c99", "c11"];
        const BEFORE_C23: &[&str] = &["c89", "c99", "c11", "gnu89", "gnu17"];
        /// What a compiler does with a program.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Answer {
            Silent,
            Warns,
            Refuses,
        }
        use Answer::{
            Refuses,
            Silent,
            Warns,
        };
        type Row = (&'static str, &'static [&'static str], Answer, Answer);
        let table: &[Row] = &[
            // Refusals of what bcc takes.
            ("void f(void){for(int i=0;i<1;i++);}", C89, Refuses, Warns),
            ("int a[]={};", ALL, Refuses, Warns),
            ("void f(void){int a[]={};(void)a;}", ALL, Refuses, Warns),
            ("void f(void){int a={};(void)a;}", &["gnu17"], Warns, Warns),
            ("#foo\nint x;", ALL, Refuses, Refuses),
            ("#if 0\n#foo\n#endif\nint x;", ALL, Silent, Silent),
            ("#elifdef A\nint x;", ISO_BEFORE_C23, Refuses, Refuses),
            ("#assert a(b)\nint x;", ALL, Warns, Refuses),
            ("#unassert a\nint x;", ALL, Warns, Refuses),
            ("_Decimal64 d;", BEFORE_C23, Warns, Refuses),
            ("_Decimal64 d;", &["c23"], Silent, Refuses),
            ("_Decimal32 d;", BEFORE_C23, Warns, Refuses),
            ("int a[(1,2)];", GNU, Refuses, Warns),
            ("enum{A=(1,2)};", GNU, Refuses, Warns),
            ("enum{A=(1,2)};", &["c99"], Refuses, Warns),
            (
                "void f(int x){switch(x){case (1,2):;}}",
                &["gnu17"],
                Refuses,
                Warns,
            ),
            ("_Static_assert((1,2),\"x\");", &["gnu17"], Refuses, Warns),
            // What neither, or only one, reports.
            ("__float128 q;", ALL, Silent, Silent),
            ("int c=__COUNTER__;", ALL, Silent, Warns),
            ("#define M(a,b) a\nint M(x,);", C89, Silent, Warns),
            (
                "int x=1<<31;",
                &["c99", "c11", "gnu17", "c23"],
                Warns,
                Silent,
            ),
            ("int x=1<<31;", C89, Silent, Silent),
            ("int $x;", &["c99", "c11", "gnu17", "c23"], Silent, Warns),
            ("typeof(1) x;", GNU, Silent, Warns),
            ("#if 0\n#elifdef A\n#endif\nint x;", GNU, Silent, Warns),
            (
                "#define A\n#if 0\n#elifdef A\n#endif\nint x;",
                GNU,
                Warns,
                Warns,
            ),
            ("#if 0\n#elifndef A\n#endif\nint x;", GNU, Warns, Warns),
            ("int x=0x1p3;", &["gnu89"], Silent, Warns),
            ("inline int f(void){return 0;}", &["gnu89"], Silent, Warns),
            ("// c\nint x;", &["gnu89"], Warns, Silent),
            (
                "#define M(a,...) a,##__VA_ARGS__\nint M(x);",
                &["gnu17"],
                Silent,
                Warns,
            ),
            ("#define M(a,...) a\nint M(x);", &["gnu17"], Silent, Warns),
            ("#define M(a,...) a\nint M(x);", &["c23"], Silent, Silent),
            (
                "#define M(a,...) a __VA_OPT__(,) __VA_ARGS__\nint M(x);",
                &["gnu17"],
                Silent,
                Warns,
            ),
            // What bcc itself says nothing of.
            ("struct {int x;};", ALL, Warns, Warns),
            ("int;", ALL, Warns, Warns),
            ("struct S{int x;};", ALL, Silent, Silent),
            ("enum{E};", ALL, Silent, Silent),
            (
                "void f(void){const char*s=__FUNCTION__;(void)s;}",
                ALL,
                Warns,
                Silent,
            ),
            (
                "void f(void){const char*s=__PRETTY_FUNCTION__;(void)s;}",
                ALL,
                Warns,
                Silent,
            ),
            (
                "void f(void){const char*s=__func__;(void)s;}",
                C89,
                Warns,
                Silent,
            ),
            (
                "void f(void){const char*s=__func__;(void)s;}",
                &["gnu17"],
                Silent,
                Silent,
            ),
            ("# 5 \"f.c\"\nint x;", ALL, Warns, Warns),
            ("int x='ab';", ALL, Warns, Warns),
            ("void f(int n){int a[n];(void)a;}", C89, Warns, Warns),
            (
                "void f(int n){int a[n];(void)a;}",
                &["gnu17"],
                Silent,
                Silent,
            ),
            ("int f(void){return sizeof(void);}", ALL, Warns, Warns),
            ("int g(void);int h=sizeof(g);", ALL, Warns, Warns),
        ];
        let directory = Directory::new();
        for (source, standards, gcc, clang) in table.iter().copied() {
            directory.write("c.c", &format!("{source}\n"));
            for standard in standards {
                let option = format!("-std={standard}");
                for (mode, expected) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                    let output =
                        directory.bcc(mode, &[&option, "-pedantic", "-fsyntax-only", "c.c"]);
                    let text = stderr(&output);
                    let answer = if !output.status.success() {
                        Refuses
                    } else if text.contains(": warning: ") {
                        Warns
                    } else {
                        Silent
                    };
                    assert_eq!(answer, expected, "{mode} {option} {source}: {text}");
                }
            }
        }
        // A comma expression as a static initializer, whose other half under
        // Clang is `-Wunused-value`.
        for (source, standard, accepted) in [
            ("int x=(1,2);", "gnu17", true),
            ("int x=(1,2)+1;", "gnu17", false),
            ("int x=(1,2)+1;", "c89", true),
        ] {
            directory.write("c.c", &format!("{source}\n"));
            let option = format!("-std={standard}");
            let output = directory.bcc(
                "--gcc-mode",
                &[&option, "-pedantic", "-fsyntax-only", "c.c"],
            );
            let text = stderr(&output);
            assert_eq!(output.status.success(), accepted, "{source}: {text}");
            assert_eq!(
                text.contains("initializer element is not constant"),
                accepted,
                "{source}: {text}"
            );
        }
        // Plain bcc keeps its own policy, which reports every extension.
        directory.write("c.c", "#foo\n__float128 q;\nint main(void) { return 0; }\n");
        let output = directory.bcc("", &["-pedantic", "c.c"]);
        succeeds(&output);
        let text = stderr(&output);
        assert!(text.contains("unknown preprocessing directive"), "{text}");
        assert!(text.contains("__float128"), "{text}");
    }

    /// `-ffunction-sections` and `-fdata-sections` give each definition a
    /// section of its own, so that `--gc-sections` removes the unused ones.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_sections_let_the_linker_remove_unused_definitions() {
        let directory = Directory::new();
        directory.write(
            "s.c",
            "int unused_data = 7;\nint used_data = 3;\nint unused_function(void) { return 1; \
             }\nint main(void) { return used_data - 3; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let sections = ["-ffunction-sections", "-fdata-sections"];
            let object = [&sections[..], &["-c", "s.c", "-o", "s.o"]].concat();
            succeeds(&directory.bcc(mode, &object));
            let names = Command::new("objdump")
                .args(["-h", directory.path("s.o").to_str().unwrap()])
                .output();
            // Without binutils the linked program below still has to run.
            if let Ok(names) = names {
                let names = stdout(&names);
                for section in [".text.unused_function", ".data.unused_data", ".text.main"] {
                    assert!(names.contains(section), "{mode}: {section}: {names}");
                }
            }
            let program = [&sections[..], &["-Wl,--gc-sections", "s.c", "-o", "s"]].concat();
            let output = directory.bcc(mode, &program);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
            let bytes = std::fs::read(directory.path("s")).unwrap();
            let has = |name: &str| {
                bytes
                    .windows(name.len())
                    .any(|window| window == name.as_bytes())
            };
            assert!(!has("unused_function") && !has("unused_data"), "{mode}");
            assert!(has("used_data"), "{mode}");
            assert_eq!(
                Command::new(directory.path("s")).status().unwrap().code(),
                Some(0),
                "{mode}"
            );
            // The last of a pair decides.
            let last = [
                "-ffunction-sections",
                "-fno-function-sections",
                "-c",
                "s.c",
                "-o",
                "t.o",
            ];
            succeeds(&directory.bcc(mode, &last));
            if let Ok(names) = Command::new("objdump")
                .args(["-h", directory.path("t.o").to_str().unwrap()])
                .output()
            {
                assert!(!stdout(&names).contains(".text.main"), "{mode}");
            }
        }
    }

    /// The options that choose what a diagnostic writes, each under the
    /// compiler that knows it, with the last of a pair deciding.
    #[test]
    fn gnu_diagnostic_presentation_options() {
        let directory = Directory::new();
        directory.write("e.c", "int x;\nchar x;\nint y = 1.0 + \"\";\n");
        let text = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, &[args, &["-c", "e.c", "-o", "e.o"]].concat());
            stderr(&output)
        };
        let plain = text("--gcc-mode", &[]);
        assert!(plain.contains("e.c:2:6: error: ") && plain.contains("    2 | char x;\n"));
        for (mode, off) in [
            ("--gcc-mode", "-fno-diagnostics-show-caret"),
            ("--gcc-mode", "-fdiagnostics-plain-output"),
            ("--clang-mode", "-fno-caret-diagnostics"),
        ] {
            let output = text(mode, &[off]);
            assert!(output.contains("e.c:2:6: error: "), "{off}: {output}");
            assert!(output.contains("e.c:1:5: note: "), "{off}: {output}");
            assert!(
                !output.contains('|') && !output.contains('^'),
                "{off}: {output}"
            );
        }
        assert!(
            text(
                "--gcc-mode",
                &["-fdiagnostics-plain-output", "-fdiagnostics-show-caret"]
            )
            .contains(" char x;\n"),
            "line numbers stay off, the caret comes back"
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = text(mode, &["-fno-show-column"]);
            assert!(output.contains("e.c:2: error: "), "{output}");
            assert!(output.contains("e.c:1: note: "), "{output}");
            assert!(output.contains("    2 | char x;\n"), "{output}");
            let output = text(mode, &["-fshow-column"]);
            assert!(output.contains("e.c:2:6: error: "), "{output}");
            let output = text(mode, &["-fdiagnostics-color=always"]);
            assert!(output.contains('\u{1b}'), "{output}");
            let output = text(
                mode,
                &["-fdiagnostics-color=always", "-fdiagnostics-color=never"],
            );
            assert!(!output.contains('\u{1b}'), "{output}");
        }
        // Clang's spelling, which GCC doesn't know, and GCC's, which Clang
        // doesn't.
        assert!(text("--clang-mode", &["-fcolor-diagnostics"]).contains('\u{1b}'));
        assert!(
            text("--gcc-mode", &["-fcolor-diagnostics"])
                .contains("unrecognized command-line option")
        );
        assert!(text("--clang-mode", &["-fdiagnostics-plain-output"]).contains("unknown argument"));
        // GCC keeps a space where line numbers would be; Clang doesn't.
        let bare = text("--gcc-mode", &["-fno-diagnostics-show-line-numbers"]);
        assert!(bare.contains("\n char x;\n      ^\n"), "{bare}");
        let bare = text("--clang-mode", &["-fno-diagnostics-show-line-numbers"]);
        assert!(bare.contains("\nchar x;\n     ^\n"), "{bare}");
        // Without the option's name.
        directory.write("w.c", "char *p = 1;\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let named = |args: &[&str]| {
                let output = directory.bcc(mode, &[args, &["-c", "w.c", "-o", "w.o"]].concat());
                stderr(&output).contains("[-W")
            };
            assert!(named(&[]), "{mode}");
            assert!(!named(&["-fno-diagnostics-show-option"]), "{mode}");
            assert!(
                named(&["-fno-diagnostics-show-option", "-fdiagnostics-show-option"]),
                "{mode}"
            );
        }
    }

    /// The `-g` options, read in order as GCC 14.2 and Clang 23 read them:
    /// whether each command line writes debug information.
    #[test]
    fn debug_options_are_read_in_order() {
        let directory = Directory::new();
        directory.write("t.c", "int t(int a) { return a + 1; }\n");
        let writes = |mode: &str, flags: &[&str]| {
            let mut args = vec![
                "--target=x86_64-unknown-linux-gnu",
                "-c",
                "t.c",
                "-o",
                "t.o",
            ];
            args.extend_from_slice(flags);
            let output = directory.bcc(mode, &args);
            assert!(
                output.status.success(),
                "{mode} {flags:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::read(directory.path("t.o"))
                .unwrap()
                .windows(b".debug_info".len())
                .any(|window| window == b".debug_info")
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            for (flags, expected) in [
                (&[][..], false),
                (&["-g"], true),
                (&["-g0"], false),
                (&["-g1"], true),
                (&["-g2"], true),
                (&["-g3"], true),
                (&["-ggdb"], true),
                (&["-ggdb3"], true),
                (&["-gdwarf"], true),
                (&["-gdwarf-4"], true),
                (&["-g", "-g0"], false),
                (&["-g0", "-g"], true),
                (&["-gdwarf-4", "-g0"], false),
                (&["-g0", "-gdwarf-4"], true),
                // These shape debug information; alone they do not ask for it.
                (&["-gsplit-dwarf"], false),
                (&["-gcolumn-info"], false),
                (&["-gno-column-info"], false),
                (&["-gstrict-dwarf"], false),
                (&["-gz"], false),
                (&["-grecord-gcc-switches"], false),
            ] {
                assert_eq!(writes(mode, flags), expected, "{mode} {flags:?}");
            }
        }
        assert!(writes("--gcc-mode", &["-gtoggle"]));
        assert!(!writes("--gcc-mode", &["-g", "-gtoggle"]));
        assert!(!writes("--gcc-mode", &["-gtoggle", "-g"]));
        assert!(writes("--clang-mode", &["-gline-tables-only"]));
        assert!(writes("--clang-mode", &["-gmlt"]));
        assert!(writes("--clang-mode", &["-gdwarf-6"]));
    }

    /// TCC takes `-g` as GCC does.
    #[test]
    fn tcc_debug_option_writes_debug_information() {
        let directory = Directory::new();
        directory.write("t.c", "int t(int a) { return a + 1; }\n");
        let output = directory.bcc("--tcc-mode", &["-g", "-c", "t.c", "-o", "t.o"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let object = std::fs::read(directory.path("t.o")).unwrap();
        assert!(object.windows(7).any(|window| window == b".debug_"));
    }

    /// A level or DWARF version the compiler refuses, in its words.
    #[test]
    fn debug_levels_and_versions_out_of_range_are_errors() {
        let directory = Directory::new();
        directory.write("t.c", "int t(void) { return 0; }\n");
        for (mode, flag, message) in [
            ("--gcc-mode", "-g4", "debug output level '4' is too high"),
            (
                "--gcc-mode",
                "-gdwarf-1",
                "dwarf version 1 is not supported",
            ),
            (
                "--gcc-mode",
                "-gdwarf-6",
                "dwarf version 6 is not supported",
            ),
            ("--clang-mode", "-g4", "unknown argument: '-g4'"),
            ("--clang-mode", "-gdwarf-1", "unknown argument: '-gdwarf-1'"),
        ] {
            let output = directory.bcc(mode, &[flag, "-c", "t.c", "-o", "t.o"]);
            assert!(!output.status.success(), "{mode} {flag}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(message), "{mode} {flag}: {stderr}");
        }
    }

    /// `/Zi`, `/Z7` and `/ZI` put `CodeView` in the object, as with clang-cl.
    #[test]
    fn msvc_debug_options_write_codeview() {
        let directory = Directory::new();
        directory.write("t.c", "int t(int a) { return a + 1; }\n");
        for flag in ["/Zi", "/Z7", "/ZI"] {
            let output = directory.bcc("--msvc-mode", &[flag, "/c", "t.c", "/Fot.obj"]);
            assert!(
                output.status.success(),
                "{flag}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let object = std::fs::read(directory.path("t.obj")).unwrap();
            assert!(
                object.windows(8).any(|window| window == b".debug$S"),
                "{flag}"
            );
        }
    }

    /// What follows a string in its array's braces is excess: a warning from
    /// Clang, which drops it, and an error from GCC and TCC.
    #[test]
    fn excess_elements_after_a_braced_string() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "char w[] = {\"ab\", \"c\", \"d\"};\nint n[sizeof w == 3 ? 1 : -1];\n",
        );
        let output = directory.bcc("--clang-mode", &["-fsyntax-only", "a.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output)
                .matches("warning: excess elements in char array initializer")
                .count(),
            1,
            "{}",
            stderr(&output)
        );
        let output = directory.bcc(
            "--clang-mode",
            &["-fsyntax-only", "-Wno-excess-initializers", "a.c"],
        );
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let output = directory.bcc("--gcc-mode", &["-fsyntax-only", "a.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output)
                .matches("error: excess elements in char array initializer")
                .count(),
            2,
            "{}",
            stderr(&output)
        );
        let output = directory.bcc("--tcc-mode", &["-c", "a.c"]);
        assert!(!output.status.success());
    }

    /// GCC reads an element for an aggregate with no subobjects as excess and
    /// goes on to the next member, where Clang requires braces.
    #[test]
    fn excess_element_for_a_zero_length_array_member() {
        let directory = Directory::new();
        directory.write(
            "z.c",
            "struct { int a; int z[0]; int y; } t = {1, 2, 3};\nint n[sizeof t == 8 ? 1 : -1];\n",
        );
        let output = directory.bcc("--gcc-mode", &["-fsyntax-only", "z.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output)
                .matches("warning: excess elements in array initializer")
                .count(),
            1,
            "{}",
            stderr(&output)
        );
        let output = directory.bcc("--clang-mode", &["-fsyntax-only", "z.c"]);
        assert_eq!(output.status.code(), Some(1));
    }

    /// GCC moves on from a member with no subobjects to the next one, inside
    /// whatever encloses it, and counts the array of unknown bound from there.
    #[cfg(target_os = "linux")]
    #[test]
    fn excess_elements_leave_the_enclosing_object_in_place() {
        let directory = Directory::new();
        directory.write(
            "n.c",
            "extern int printf(const char *, ...);\n\
             struct E { int z[0]; int y; };\n\
             struct E unknown[] = {1, 2, 3, 4};\n\
             struct { struct {} s; int y; } pairs[2] = {1, 2, 3, 4};\n\
             int main(void) {\n\
             \x20   struct { struct { int z[0]; int x; } s; int y; } t = {1, 2, 3};\n\
             \x20   static struct { struct { int z[0]; int x; } s; int y; } u = {5, 6, 7};\n\
             \x20   printf(\"%d %d %d %d %d %d %d %d\\n\", t.s.x, t.y, u.s.x, u.y,\n\
             \x20          (int)(sizeof unknown / sizeof *unknown), unknown[1].y,\n\
             \x20          pairs[0].y, pairs[1].y);\n\
             \x20   return 0;\n\
             }\n",
        );
        let output = directory.bcc("--gcc-mode", &["n.c", "-o", "n"]);
        succeeds(&output);
        let run = Command::new(directory.path("n")).output().unwrap();
        // gcc 13.3 prints the same.
        assert_eq!(String::from_utf8_lossy(&run.stdout), "2 3 6 7 2 4 2 4\n");
    }

    /// Each identity refuses an operand of `__builtin_clear_padding` as its
    /// compiler does, in its words (GCC 14.2 and Clang 23.1 print these) and
    /// at its position, and accepts what its compiler accepts.
    #[test]
    fn clear_padding_operands_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "c.c",
            "struct Inc; struct S { char a; int b; }; struct F { int a; char f[]; };\nstruct G { \
             struct F a; }; typedef void fn(void); void g(void);\nvoid f(int x, void *v, struct \
             Inc *inc, const struct S *cs,\n\x20      _Atomic struct S *as, struct S *s, struct F \
             *flex, struct G *nested,\n\x20      int (*open)[], const char (*row)[2], fn \
             *function, char *c) {\n\x20   __builtin_clear_padding(x);\n\x20   \
             __builtin_clear_padding(*s);\n\x20   __builtin_clear_padding(v);\n\x20   \
             __builtin_clear_padding(inc);\n\x20   __builtin_clear_padding(open);\n\x20   \
             __builtin_clear_padding(cs);\n\x20   __builtin_clear_padding(as);\n\x20   \
             __builtin_clear_padding(flex);\n\x20   __builtin_clear_padding(nested);\n\x20   \
             __builtin_clear_padding(row);\n\x20   __builtin_clear_padding(function);\n\x20   \
             __builtin_clear_padding(s, s);\n\x20   __builtin_clear_padding();\n\x20   \
             __builtin_clear_padding(g());\n}\n",
        );
        let lines = |mode: &str| {
            let output = directory.bcc(mode, &["-fsyntax-only", "-w", "c.c"]);
            assert_eq!(output.status.code(), Some(1));
            stderr(&output)
                .lines()
                .filter(|line| line.contains("error:"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let call = "argument 1 in call to function '__builtin_clear_padding'";
        let flexible = "flexible array member 'f' does not have well defined padding bits for \
                        '__builtin_clear_padding'";
        let gcc = lines("--gcc-mode");
        assert_eq!(
            gcc,
            [
                format!("c.c:6:29: error: {call} does not have pointer type"),
                format!("c.c:7:29: error: {call} does not have pointer type"),
                format!("c.c:8:29: error: {call} points to incomplete type"),
                format!("c.c:9:29: error: {call} points to incomplete type"),
                format!("c.c:10:29: error: {call} points to incomplete type"),
                format!(
                    "c.c:11:29: error: {call} has pointer to 'const' type ('const struct S *')"
                ),
                format!(
                    "c.c:12:29: error: {call} has pointer to '_Atomic' type ('_Atomic struct S *')"
                ),
                format!("c.c:13:5: error: {flexible}"),
                format!("c.c:14:5: error: {flexible}"),
                "c.c:17:5: error: too many arguments to function '__builtin_clear_padding'"
                    .to_owned(),
                "c.c:18:5: error: too few arguments to function '__builtin_clear_padding'"
                    .to_owned(),
                "c.c:19:29: error: invalid use of void expression".to_owned(),
            ],
            "{gcc:#?}"
        );
        let clang = lines("--clang-mode");
        let passing = |ty: &str| {
            format!(
                "passing '{ty}' to parameter of incompatible type pointer: type mismatch at 1st \
                 parameter ('{ty}' vs pointer)"
            )
        };
        assert_eq!(
            clang,
            [
                format!("c.c:6:29: error: {}", passing("int")),
                format!("c.c:7:29: error: {}", passing("struct S")),
                "c.c:8:29: error: variable has incomplete type 'void'".to_owned(),
                "c.c:9:29: error: variable has incomplete type 'struct Inc'".to_owned(),
                "c.c:10:29: error: variable has incomplete type 'int[]'".to_owned(),
                "c.c:11:29: error: read-only variable is not assignable".to_owned(),
                "c.c:13:29: error: 'struct F' has flexible array member, which is unsupported by \
                 __builtin_clear_padding"
                    .to_owned(),
                "c.c:14:29: error: 'struct G' has flexible array member, which is unsupported by \
                 __builtin_clear_padding"
                    .to_owned(),
                "c.c:15:29: error: read-only variable is not assignable".to_owned(),
                "c.c:16:29: error: argument to __builtin_clear_padding must be a pointer to a \
                 trivially-copyable type ('void (*)(void)' invalid)"
                    .to_owned(),
                "c.c:17:32: error: too many arguments to function call, expected 1, have 2"
                    .to_owned(),
                "c.c:18:5: error: too few arguments to function call, expected 1, have 0"
                    .to_owned(),
                format!("c.c:19:29: error: {}", passing("void")),
            ],
            "{clang:#?}"
        );
        // bcc's own format has Clang's words.
        let output = directory.bcc("", &["-fsyntax-only", "-w", "c.c"]);
        assert!(
            stderr(&output).contains("error: variable has incomplete type 'struct Inc'"),
            "{}",
            stderr(&output)
        );
    }

    /// A member of type `nullptr_t` is padding to GCC and value to Clang, and
    /// GCC keeps the bytes a bit-field of a union reaches; each identity
    /// leaves what its compiler left in GCC 14.2 and Clang 23.1. GCC does not
    /// look into a `nullptr_t` object itself.
    #[test]
    fn clear_padding_masks_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "m.c",
            "int printf(const char *, ...);\n#define SHOW(x) do { __builtin_memset(&x, 0xff, \
             sizeof x); \\\n\x20   __builtin_clear_padding(&x); \\\n\x20   for (unsigned long i = \
             0; i < sizeof x; i++) \\\n\x20       printf(\"%02x\", ((unsigned char *)&x)[i]); \
             \\\n\x20   printf(\"\\n\"); } while (0)\nint main(void) {\n\x20   union { int a : \
             13; } u;\n\x20   struct { char c; typeof(nullptr) n; } s;\n\x20   union { \
             typeof(nullptr) n; char c; } v;\n\x20   typeof(nullptr) a[2];\n\x20   \
             typeof(nullptr) n;\n\x20   SHOW(u); SHOW(s); SHOW(v); SHOW(a); SHOW(n);\n\x20   \
             return 0;\n}\n",
        );
        let ones = "ffffffffffffffff";
        for (mode, expected) in [
            (
                "--gcc-mode",
                format!(
                    "ffff0000\nff00000000000000{zeros}\nff00000000000000\n{ones}{ones}\n{ones}\n",
                    zeros = "0000000000000000"
                ),
            ),
            (
                "--clang-mode",
                format!("ff1f0000\nff00000000000000{ones}\n{ones}\n{ones}{ones}\n{ones}\n"),
            ),
        ] {
            let output = directory.bcc(mode, &["-std=gnu2x", "-w", "m.c", "-o", "m"]);
            succeeds(&output);
            let run = Command::new(directory.path("m")).output().unwrap();
            assert_eq!(stdout(&run), expected, "{mode}");
        }
    }

    /// GCC refuses a record of variable size, with its "sorry".
    #[test]
    fn clear_padding_of_a_record_of_variable_size_is_refused_under_gcc() {
        let directory = Directory::new();
        directory.write(
            "v.c",
            "void f(int n) {\n\x20   struct { char a; int b[n]; } v;\n\x20   \
             __builtin_clear_padding(&v);\n}\n",
        );
        // On Linux: MinGW's Microsoft record layout, the Windows default,
        // refuses such a record on its own terms when it lowers.
        let output = directory.bcc(
            "--gcc-mode",
            &["--target=x86_64-linux-gnu", "-fsyntax-only", "v.c"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains(
                "v.c:3:5: error: sorry, unimplemented: __builtin_clear_padding not supported for \
                 variable length aggregates"
            ),
            "{}",
            stderr(&output)
        );
    }

    /// With a type present, a typedef name before an attribute or `asm` label
    /// is the declarator, and without one it stays the type.
    #[test]
    fn typedef_names_before_attributes_follow_the_declaration() {
        let directory = Directory::new();
        directory.write(
            "t.c",
            "typedef int U;\ntypedef int U __attribute__((unused));\ntypedef int V;\nconst V \
             __attribute__((unused)) c = 1;\nV __attribute__((unused)) d = 2;\nint main(void) \
             {\n    int U __attribute__((unused)) = 3;\n    long V __asm__(\"inner\");\n    \
             (void)V;\n    return U - c - d;\n}\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-O1", "-c", "t.c", "-o", "t.o"]);
            succeeds(&output);
        }
        // The type stays one when another declarator follows the attribute.
        directory.write("e.c", "typedef int T;\nint T __attribute__((unused)) x;\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-c", "e.c", "-o", "e.o"]);
            assert!(!output.status.success(), "{mode}");
        }
    }

    /// `-save-temps` keeps the intermediate files, named as each compiler
    /// names them, and prints a warning once.
    #[test]
    fn gnu_save_temps_keeps_intermediate_files() {
        let cases: [(&str, &[&str], &[&str]); 6] = [
            (
                "--gcc-mode",
                &["-save-temps", "-c", "g.c"],
                &["g.i", "g.s", "g.o"],
            ),
            (
                "--gcc-mode",
                &["-save-temps", "-c", "g.c", "-o", "o/x.o"],
                &["o/x.i", "o/x.s", "o/x.o"],
            ),
            ("--gcc-mode", &["-save-temps", "-S", "g.c"], &["g.i", "g.s"]),
            (
                "--clang-mode",
                &["-save-temps", "-c", "g.c"],
                &["g.i", "g.s", "g.o"],
            ),
            (
                "--clang-mode",
                &["-save-temps=obj", "-c", "g.c", "-o", "o/x.o"],
                &["o/g.i", "o/g.s", "o/x.o"],
            ),
            ("--clang-mode", &["-save-temps", "-E", "g.c"], &[]),
        ];
        for (mode, args, files) in cases {
            let directory = Directory::new();
            std::fs::create_dir(directory.path("o")).unwrap();
            directory.write("g.c", "#define R 1\nint g(void) { return R; }\n");
            let output = directory.bcc(mode, args);
            succeeds(&output);
            for file in files {
                assert!(directory.path(file).exists(), "{mode} {args:?}: {file}");
            }
            if *files == ["g.i", "g.s", "g.o"] {
                let text = std::fs::read_to_string(directory.path("g.i")).unwrap();
                assert!(text.contains("return 1;"), "{mode}: {text}");
                assert!(!text.contains("define"), "{mode}: {text}");
            }
        }

        let directory = Directory::new();
        directory.write("w.c", "#warning careful\nint w;\n");
        let output = directory.bcc("--clang-mode", &["-save-temps", "-c", "w.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output).matches("warning:").count(),
            1,
            "{}",
            stderr(&output)
        );

        // A failed compile prints its error once and saves nothing.
        directory.write("e.c", "int e(void) { return x; }\n");
        let output = directory.bcc("--gcc-mode", &["-save-temps", "-c", "e.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output).matches("error:").count(),
            1,
            "{}",
            stderr(&output)
        );
        assert!(!directory.path("e.s").exists());
    }

    /// The cases where `-save-temps` could damage a build: the command's own
    /// output, and a header the saved text overwrites.
    #[test]
    fn gnu_save_temps_keeps_the_build_intact() {
        for mode in ["--gcc-mode", "--clang-mode"] {
            let directory = Directory::new();
            directory.write("g.c", "int g(void) { return 7; }\n");
            succeeds(&directory.bcc(mode, &["-save-temps", "-c", "g.c", "-o", "g.i"]));
            let object = std::fs::read(directory.path("g.i")).unwrap();
            let expected = directory.bcc(mode, &["-c", "g.c", "-o", "plain.o"]);
            succeeds(&expected);
            assert_eq!(
                object,
                std::fs::read(directory.path("plain.o")).unwrap(),
                "{mode}"
            );

            let directory = Directory::new();
            directory.write("g.i", "typedef int T;\n");
            directory.write("g.c", "#include \"g.i\"\nint g(void) { return 7; }\n");
            succeeds(&directory.bcc(mode, &["-save-temps", "-c", "g.c"]));
            assert!(directory.path("g.s").exists(), "{mode}");
        }
    }

    /// A saved stage never replaces the output however that output is spelled,
    /// and reads the saved text as preprocessed whatever `-x` says.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_save_temps_compares_files_and_reads_preprocessed_text() {
        for (mode, flag, output) in [
            ("--clang-mode", "-save-temps", "./g.i"),
            ("--clang-mode", "-save-temps", "./g.s"),
            ("--gcc-mode", "-save-temps=cwd", "./g.i"),
            ("--gcc-mode", "-save-temps=cwd", "o/../g.i"),
        ] {
            let directory = Directory::new();
            std::fs::create_dir(directory.path("o")).unwrap();
            directory.write("g.c", "int main(void) { return 7; }\n");
            succeeds(&directory.bcc(mode, &[flag, "-c", "g.c", "-o", output]));
            let object = std::fs::read(directory.path(output)).unwrap();
            assert_eq!(&object[..4], b"\x7fELF", "{mode} {flag} {output}");
        }

        for mode in ["--gcc-mode", "--clang-mode"] {
            let directory = Directory::new();
            directory.write("h.h", "struct T { int x; };\n");
            directory.write("g.c", "struct T t = {7};\nint main(void) { return t.x; }\n");
            succeeds(&directory.bcc(
                mode,
                &[
                    "-save-temps",
                    "-x",
                    "c",
                    "-include",
                    "h.h",
                    "g.c",
                    "-o",
                    "saved",
                ],
            ));
            let run = Command::new(directory.path("saved")).status().unwrap();
            assert_eq!(run.code(), Some(7), "{mode}");
        }
    }

    /// GCC's names follow the output, with `=cwd` leaving out its directory.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_save_temps_names_follow_gcc() {
        let cases: [(&[&str], &[&str]); 4] = [
            (
                &["-save-temps=cwd", "-c", "g.c", "-o", "o/x.o"],
                &["x.i", "x.s", "o/x.o"],
            ),
            (
                &["-save-temps", "g.c", "m.c", "-o", "o/prog.v1"],
                &[
                    "o/prog.v1-g.i",
                    "o/prog.v1-m.s",
                    "o/prog.v1-g.o",
                    "o/prog.v1",
                ],
            ),
            (
                &["-save-temps=cwd", "g.c", "m.c", "-o", "o/prog.v1"],
                &["prog.v1-g.i", "prog.v1-m.s", "prog.v1-g.o", "o/prog.v1"],
            ),
            (&["-save-temps", "-S", "g.c", "-o", "-"], &["g.i"]),
        ];
        for (args, files) in cases {
            let directory = Directory::new();
            std::fs::create_dir(directory.path("o")).unwrap();
            directory.write("g.c", "int g(void) { return 7; }\n");
            directory.write("m.c", "int g(void); int main(void) { return g(); }\n");
            succeeds(&directory.bcc("--gcc-mode", args));
            for file in files {
                assert!(directory.path(file).exists(), "{args:?}: {file}");
            }
        }
    }

    /// A linked program's intermediate files, and its dependency file left
    /// to the real compile.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnu_save_temps_keeps_a_programs_objects() {
        let directory = Directory::new();
        directory.write("m.c", "int main(void) { return 0; }\n");
        let output = directory.bcc("--gcc-mode", &["-save-temps", "-MD", "m.c", "-o", "p"]);
        succeeds(&output);
        for file in ["p", "p-m.i", "p-m.s", "p-m.o"] {
            assert!(directory.path(file).exists(), "{file}");
        }
    }

    /// GCC compiles a computed goto in a function that takes no label's
    /// address, and Clang refuses it.
    #[test]
    fn computed_goto_without_label_addresses_follows_each_compiler() {
        let directory = Directory::new();
        directory.write("g.c", "void go(void *p) { goto *p; }\n");
        succeeds(&directory.bcc("--gcc-mode", &["-c", "g.c"]));
        let output = directory.bcc("--clang-mode", &["-c", "g.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("computed goto in a function without label addresses"),
            "{}",
            stderr(&output)
        );
    }

    /// `-print-file-name=` and `-print-prog-name=` answer as GCC and Clang
    /// do, with a path to the file the link would use or the name itself.
    #[test]
    fn gnu_print_queries_name_files_and_programs() {
        let directory = Directory::new();
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-print-file-name=no-such-file.a"]);
            succeeds(&output);
            assert_eq!(stdout(&output), "no-such-file.a\n", "{mode}");
            if cfg!(all(target_os = "linux", target_env = "gnu")) {
                let output = directory.bcc(mode, &["-print-file-name=libc.so"]);
                succeeds(&output);
                let path = stdout(&output);
                let path = Path::new(path.trim_end());
                assert!(
                    path.is_absolute() && path.exists(),
                    "{mode}: {}",
                    path.display()
                );
            }
            let output = directory.bcc(mode, &["-print-prog-name=ld"]);
            succeeds(&output);
            assert!(
                stdout(&output).trim_end().ends_with("ld.lld") || cfg!(windows),
                "{mode}"
            );
        }
    }

    /// Both compilers read the whole command line before they answer a
    /// query, so the last `-print-file-name=` or `-print-prog-name=`
    /// counts, a later `--sysroot` applies, the query that ranks first in
    /// each compiler's own order answers, and the inputs go unread, as in
    /// GCC 13.3 and Clang 23. GCC answers a `-dump` option where it stands,
    /// before it reports an unrecognized option; Clang's driver errors make
    /// its status 1, GCC's do not.
    #[test]
    fn gnu_queries_answer_once_the_command_line_is_read() {
        let directory = Directory::new();
        let answer = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            (output.status.code(), stdout(&output), stderr(&output))
        };
        let help = |mode: &str| answer(mode, &["--help"]).1;
        let file = |name: &str| (Some(0), format!("{name}\n"), String::new());
        for mode in ["--gcc-mode", "--clang-mode"] {
            for args in [
                &["-print-file-name=first", "-print-file-name=second"][..],
                &["-print-prog-name=first", "-print-file-name=second"],
                &["-print-file-name=second", "-print-prog-name=first"],
                &["-print-libgcc-file-name", "-print-file-name=second"],
                &["-print-file-name=second", "missing.c", "-c", "-o", "x.o"],
            ] {
                assert_eq!(answer(mode, args), file("second"), "{mode} {args:?}");
            }
            assert_eq!(
                answer(mode, &["-print-prog-name=first", "-print-prog-name=second"]),
                file("second"),
                "{mode}"
            );
            assert!(!directory.path("x.o").exists(), "{mode}");
            let (status, text, _) = answer(mode, &["-print-file-name=x", "-print-search-dirs"]);
            assert_eq!(status, Some(0), "{mode}");
            assert!(text.contains("programs: ="), "{mode}: {text}");
            assert_eq!(
                answer(mode, &["--version", "--help"]).1,
                help(mode),
                "{mode}"
            );
            assert_eq!(
                answer(mode, &["--help", "--version"]).1,
                help(mode),
                "{mode}"
            );
            // A sysroot given after the query still applies.
            if cfg!(all(target_os = "linux", target_env = "gnu")) {
                std::fs::create_dir_all(directory.path("root/usr/lib")).unwrap();
                directory.write("root/usr/lib/libfoo.a", "");
                let expected = directory
                    .path("root/usr/lib/libfoo.a")
                    .canonicalize()
                    .unwrap();
                for args in [
                    ["-print-file-name=libfoo.a", "--sysroot=root"],
                    ["--sysroot=root", "-print-file-name=libfoo.a"],
                ] {
                    let (status, text, _) = answer(mode, &args);
                    assert_eq!(status, Some(0), "{mode} {args:?}");
                    let found = directory.path(text.trim_end()).canonicalize();
                    assert_eq!(
                        found.ok().as_ref(),
                        Some(&expected),
                        "{mode} {args:?}: {text}"
                    );
                }
            }
        }
        // GCC's `-print-libgcc-file-name` is its `-print-file-name=libgcc.a`,
        // so the later of the two answers; Clang's ranks below.
        assert_eq!(
            answer(
                "--gcc-mode",
                &["-print-file-name=x", "-print-libgcc-file-name"]
            ),
            answer("--gcc-mode", &["-print-file-name=libgcc.a"])
        );
        assert_eq!(
            answer(
                "--clang-mode",
                &["-print-file-name=x", "-print-libgcc-file-name"]
            ),
            file("x")
        );
        // GCC's `-dump` options answer at once; it reports the options it
        // does not recognize only after reading them all, and with a query
        // keeps status 0.
        let version = answer("--gcc-mode", &["-dumpversion"]).1;
        for args in [
            &["-print-file-name=x", "-dumpversion"][..],
            &["-fbogus", "-dumpversion", "-fbogus2"],
        ] {
            assert_eq!(
                answer("--gcc-mode", args),
                (Some(0), version.clone(), String::new())
            );
        }
        assert_eq!(
            answer("--gcc-mode", &["-fbogus", "-print-file-name=x"]),
            (
                Some(0),
                "x\n".into(),
                "gcc: error: unrecognized command-line option '-fbogus'\n".into()
            )
        );
        assert_eq!(
            answer("--gcc-mode", &["-print-sysroot"]),
            (Some(0), String::new(), String::new())
        );
        assert_eq!(
            answer("--gcc-mode", &["-print-sysroot", "--sysroot=root"]),
            file("root")
        );
        // Clang answers `-dumpmachine` before `-dumpversion` wherever each
        // stands; its driver errors still print, with status 1, and it
        // knows neither `-print-sysroot` nor `-dumpfullversion` as queries.
        let machine = answer("--clang-mode", &["-dumpmachine"]).1;
        assert_eq!(
            answer("--clang-mode", &["-dumpversion", "-dumpmachine"]),
            (Some(0), machine, String::new())
        );
        assert_eq!(
            answer("--clang-mode", &["-fbogus", "-print-file-name=x"]),
            (
                Some(1),
                "x\n".into(),
                "clang: error: unknown argument: '-fbogus'\n".into()
            )
        );
        assert_eq!(
            answer("--clang-mode", &["-print-multiarch", "-print-file-name=x"]),
            (
                Some(1),
                "x\n".into(),
                "clang: error: unsupported option '-print-multiarch'\n".into()
            )
        );
        assert_eq!(
            answer("--clang-mode", &["-std=bogus", "-print-file-name=x"]),
            file("x")
        );
        let (status, _, text) = answer("--clang-mode", &["-print-sysroot"]);
        assert_eq!(status, Some(1));
        assert!(
            text.contains("unknown argument: '-print-sysroot'"),
            "{text}"
        );
        directory.write("a.c", "int x;\n");
        assert_eq!(
            answer(
                "--clang-mode",
                &["-dumpfullversion", "-fsyntax-only", "a.c"]
            ),
            (
                Some(0),
                String::new(),
                "clang: warning: argument unused during compilation: '-dumpfullversion' \
                 [-Wunused-command-line-argument]\n"
                    .into()
            )
        );
    }

    /// An unknown `-std=` is GCC's unrecognized option wherever it stands,
    /// while Clang's compiler refuses only the last `-std=`, once for each C
    /// source it compiles, and not when its driver stops first, as in GCC
    /// 13.3 and Clang 23.
    #[test]
    fn gnu_unknown_standards_are_refused_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("a.c", "int x;\n");
        directory.write("b.c", "int y;\n");
        let answer = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            (output.status.code(), stdout(&output), stderr(&output))
        };
        let gcc_error = |option: &str| {
            (
                Some(1),
                String::new(),
                format!("gcc: error: unrecognized command-line option '{option}'\n"),
            )
        };
        for args in [
            &["-std=bogus", "-std=c17", "-fsyntax-only", "a.c"][..],
            &["-std=c17", "-std=bogus", "-fsyntax-only", "a.c"],
            &["-std=bogus", "-c", "a.c"],
            &["-std=bogus", "missing.c"],
        ] {
            assert_eq!(
                answer("--gcc-mode", args),
                gcc_error("-std=bogus"),
                "{args:?}"
            );
        }
        assert_eq!(
            answer(
                "--clang-mode",
                &["-std=bogus", "-std=c17", "-fsyntax-only", "a.c"]
            ),
            (Some(0), String::new(), String::new())
        );
        let invalid = "error: invalid value 'bogus' in '-std=bogus'\n";
        for (args, count) in [
            (&["-std=c17", "-std=bogus", "-fsyntax-only", "a.c"][..], 1),
            (&["-std=bogus", "-c", "a.c"], 1),
            (&["-std=bogus", "-fsyntax-only", "a.c", "b.c"], 2),
        ] {
            let (status, _, text) = answer("--clang-mode", args);
            assert_eq!(status, Some(1), "{args:?}");
            assert_eq!(text.matches(invalid).count(), count, "{args:?}: {text}");
            assert!(
                text.contains("note: use 'c99' or 'iso9899:1999'"),
                "{args:?}: {text}"
            );
        }
        assert_eq!(
            answer("--clang-mode", &["-std=bogus", "missing.c"]),
            (
                Some(1),
                String::new(),
                "clang: error: no such file or directory: 'missing.c'\nclang: error: no input \
                 files\n"
                    .into()
            )
        );
        let (status, _, text) = answer("--clang-mode", &["-std=bogus", "-c", "a.c", "-###"]);
        assert_eq!(status, Some(0));
        assert!(!text.contains("invalid value"), "{text}");
        // Clang reads the last of `-std=` and `-ansi`; GCC refuses the
        // unknown value whatever follows it.
        assert_eq!(
            answer(
                "--clang-mode",
                &["-std=bogus", "-ansi", "-fsyntax-only", "a.c"]
            ),
            (Some(0), String::new(), String::new())
        );
        let (status, _, text) = answer(
            "--clang-mode",
            &["-ansi", "-std=bogus", "-fsyntax-only", "a.c"],
        );
        assert_eq!(status, Some(1));
        assert_eq!(text.matches(invalid).count(), 1, "{text}");
        assert_eq!(
            answer(
                "--gcc-mode",
                &["-std=bogus", "-ansi", "-fsyntax-only", "a.c"]
            ),
            gcc_error("-std=bogus")
        );
    }

    /// GCC's `-v` or `-###` without inputs prints the driver's report and
    /// succeeds even after rejecting an option; without either, GCC adds
    /// that it has no inputs. Neither compiler shows a step for no inputs.
    /// As in GCC 13.3 and Clang 23.
    #[test]
    fn driver_reports_without_inputs_succeed_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("a.c", "int x;\n");
        let answer = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            (output.status.code(), stderr(&output))
        };
        let unrecognized = "gcc: error: unrecognized command-line option '-print-target-triple'\n";
        for report in ["-v", "-###"] {
            let (status, text) = answer("--gcc-mode", &["-print-target-triple", report]);
            assert_eq!(status, Some(0), "{report}: {text}");
            assert!(text.starts_with(unrecognized), "{report}: {text}");
            assert!(text.contains("\ngcc version 14.2.0 "), "{report}: {text}");
            assert!(!text.contains("--compiler-identity"), "{report}: {text}");
        }
        assert_eq!(
            answer("--gcc-mode", &["-print-target-triple"]),
            (
                Some(1),
                format!(
                    "{unrecognized}gcc: fatal error: no input files\ncompilation terminated.\n"
                )
            )
        );
        let (status, text) = answer(
            "--gcc-mode",
            &["-print-target-triple", "-v", "-fsyntax-only", "a.c"],
        );
        assert_eq!((status, text.as_str()), (Some(1), unrecognized));
        // A linker input is an input: GCC counts `-l`, `-Wl,` and
        // `-Xlinker` but not `-z`, Clang every linker input.
        let bogus = "gcc: error: unrecognized command-line option '-bogus'\n";
        for linker in [&["-lm"][..], &["-Wl,-x"], &["-Xlinker", "-x"]] {
            for report in [&["-v"][..], &["-###"], &[]] {
                let args = [&["-bogus"][..], report, linker].concat();
                let (status, text) = answer("--gcc-mode", &args);
                assert_eq!(status, Some(1), "{args:?}: {text}");
                assert!(!text.contains("no input files"), "{args:?}: {text}");
                assert!(text.contains(bogus), "{args:?}: {text}");
            }
        }
        let (status, text) = answer("--gcc-mode", &["-bogus", "-v", "-z", "now"]);
        assert_eq!(status, Some(0), "{text}");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let (status, text) = answer(mode, &["-###", "-lm"]);
            assert_eq!(status, Some(0), "{mode}: {text}");
            assert!(text.contains("--compiler-identity"), "{mode}: {text}");
        }
        let (_, text) = answer("--gcc-mode", &["-###", "-z", "now"]);
        assert!(!text.contains("--compiler-identity"), "{text}");
        let (_, text) = answer("--clang-mode", &["-###", "-z", "now"]);
        assert!(text.contains("--compiler-identity"), "{text}");
        // GCC's compiler checks `-MG`, so it waits for an input.
        let (status, text) = answer("--gcc-mode", &["-MG", "-v"]);
        assert_eq!(status, Some(0), "{text}");
        assert!(!text.contains("-MG"), "{text}");
        let (status, text) = answer("--clang-mode", &["-###"]);
        assert_eq!(status, Some(0), "{text}");
        assert!(text.contains("Thread model: posix\n"), "{text}");
        assert!(!text.contains("--compiler-identity"), "{text}");
    }

    /// GCC keeps every local current across a `__builtin_setjmp`'s second
    /// return, which `__builtin_longjmp` resumes: a local stored after the
    /// first return holds its new value there (gcc.dg/setjmp-3.c and
    /// setjmp-4.c, gcc.c-torture/execute/pr60003.c), on every back end.
    /// So do a variable length array and a structure parameter, which live
    /// at an address rather than in a stack slot.
    #[test]
    fn builtin_setjmp_keeps_locals_current_as_gcc_does() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "resume.c",
            "void *buf[5];\nvoid raise0(void) { __builtin_longjmp(buf, 1); }\nint after(int cmd) \
             {\n  int last = 0;\n  if (__builtin_setjmp(buf) == 0)\n    while (1) { last = 1; \
             raise0(); }\n  return last == 0 ? 0 : cmd;\n}\nint before(int cmd) {\n  int last = \
             0;\n  __builtin_setjmp(buf);\n  if (last == 0)\n    while (1) { last = 1; raise0(); \
             }\n  return last == 0 ? 0 : cmd;\n}\nint vla(int n) {\n  int a[n];\n  a[0] = 0;\n  \
             if (!__builtin_setjmp(buf)) { a[0] = 7; raise0(); }\n  return a[0] - 7;\n}\nstruct S \
             { int x, y[4]; };\nint param(struct S s) {\n  if (!__builtin_setjmp(buf)) { s.x = 9; \
             raise0(); }\n  return s.x - 9;\n}\nint main(void) {\n  struct S s = {0};\n  return \
             after(3) + before(4) + vla(3) + param(s);\n}\n",
        );
        for options in [
            &["-O0"][..],
            &["-O"],
            &["-O2"],
            &["-Xbcc", "--backend=native", "-O0"],
            &["-Xbcc", "--backend=native", "-O2"],
        ] {
            let mut args = options.to_vec();
            args.extend(["resume.c", "-o", "resume"]);
            succeeds(&directory.bcc("--gcc-mode", &args));
            let run = Command::new(directory.path("resume")).output().unwrap();
            assert_eq!(run.status.code(), Some(7), "{options:?}");
        }
    }

    /// GCC names the x87 extended format `__float80`, the same type as
    /// `long double` on x86-64 Linux; Clang knows no such name.
    #[test]
    fn float80_is_gccs_name_for_long_double() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "f80.c",
            "__float80 x = 1.5;\n_Static_assert(sizeof(__float80) == sizeof(long double), \
             \"\");\nint main(void) { __float80 y = x * 2; return (int)y + _Generic((__float80)0, \
             long double: 0, default: 10); }\n",
        );
        let output = directory.bcc("--gcc-mode", &["-pedantic-errors", "f80.c", "-o", "f80"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let run = Command::new(directory.path("f80")).output().unwrap();
        assert_eq!(run.status.code(), Some(3));
        let output = directory.bcc("--clang-mode", &["-fsyntax-only", "f80.c"]);
        assert!(!output.status.success());
        assert!(stderr(&output).contains("__float80"), "{}", stderr(&output));
        // A program's own typedef of the name hides GCC's, and a local
        // object may take the name; a file-scope one redeclares it.
        directory.write(
            "shadow.c",
            "int main(void) {\n  typedef int __float80;\n  __float80 x = 3;\n  \
             _Static_assert(sizeof(x) == sizeof(int), \"shadow\");\n  { int __float80 = 1; x += \
             __float80; }\n  return x;\n}\n",
        );
        let output = directory.bcc("--gcc-mode", &["shadow.c", "-o", "shadow"]);
        succeeds(&output);
        let run = Command::new(directory.path("shadow")).output().unwrap();
        assert_eq!(run.status.code(), Some(4));
        // (source, GCC accepts, Clang accepts), as GCC 14 and Clang 23 do.
        for (source, gcc, clang) in [
            ("int __float80;\n", false, true),
            ("int __int128_t;\n", false, false),
            ("int __int128_t(void);\n", false, false),
            ("enum E { __float80 = 3 };\n", true, true),
            ("enum E { __int128_t = 3 };\n", true, false),
            ("typedef double __int128_t;\n", true, false),
            ("typedef int __uint128_t;\n", true, false),
            ("typedef __int128 __int128_t;\n", true, true),
            ("void f(void) { enum { __int128_t }; }\n", true, true),
        ] {
            directory.write("redeclared.c", source);
            for (mode, accepted) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(mode, &["-std=c17", "-fsyntax-only", "redeclared.c"]);
                assert_eq!(output.status.success(), accepted, "{mode} {source}");
            }
        }
    }

    /// GCC checks what the end of a unit leaves unused as it compiles the
    /// unit, not under `-fsyntax-only`; Clang does either way. `-Werror` and a
    /// diagnostic pragma cannot bring a report back that was never made. Both
    /// resolve an alias as they generate code.
    #[test]
    fn end_of_unit_checks_wait_for_code_generation() {
        let directory = Directory::new();
        for (source, syntax_gcc, syntax_clang, real_gcc, real_clang) in [
            ("static int x;\n", true, false, false, false),
            ("static const int x = 1;\n", true, false, false, false),
            ("static void f(void) {}\n", true, false, false, false),
            (
                "#pragma GCC diagnostic error \"-Wunused-function\"\nstatic void f(void) {}\n",
                true,
                false,
                false,
                false,
            ),
            // Reported as the file scope closes, either way.
            ("static void f(void);\n", false, false, false, false),
            ("void f(void) { int a; }\n", false, false, false, false),
            (
                "static int x; void f(void) { extern int x; }\n",
                false,
                false,
                false,
                false,
            ),
            ("int x;\n", true, true, true, true),
        ] {
            directory.write("end.c", source);
            for (mode, stage, clean) in [
                (
                    "--gcc-mode",
                    ["-fsyntax-only", "-Wall"].as_slice(),
                    syntax_gcc,
                ),
                (
                    "--clang-mode",
                    ["-fsyntax-only", "-Wall"].as_slice(),
                    syntax_clang,
                ),
                (
                    "--gcc-mode",
                    ["-S", "-o", "end.s", "-Wall"].as_slice(),
                    real_gcc,
                ),
                (
                    "--clang-mode",
                    ["-S", "-o", "end.s", "-Wall"].as_slice(),
                    real_clang,
                ),
            ] {
                let mut args = stage.to_vec();
                args.extend(["-Wextra", "-Werror", "end.c"]);
                let output = directory.bcc(mode, &args);
                assert_eq!(
                    output.status.success(),
                    clean,
                    "{mode} {stage:?} {source}: {}",
                    stderr(&output)
                );
            }
        }
        directory.write(
            "alias.c",
            "void f(void) __attribute__((alias(\"missing\")));\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            succeeds(&directory.bcc(mode, &["-fsyntax-only", "alias.c"]));
            assert!(
                !directory
                    .bcc(mode, &["-S", "-o", "alias.s", "alias.c"])
                    .status
                    .success()
            );
        }
    }

    /// GCC withholds its unused warnings when a system header spells the
    /// token, as a system macro's body does, but not when the user wrote it
    /// as an argument; its label warning goes by the invocation. Clang goes
    /// by the invocation for all, and skips a value a macro's body wrote.
    #[test]
    fn unused_warnings_follow_each_compilers_system_macro_rule() {
        let directory = Directory::new();
        std::fs::create_dir(directory.path("sys")).unwrap();
        directory.write(
            "sys/sys.h",
            "#define NOOP(x) ((x) + 1)\n#define LOCAL int macro_local;\n#define STATICFN static \
             void macro_fn(void) {}\n#define STATICOBJ static int macro_obj;\n#define PARAM void \
             macro_param(int p) {}\n#define PAREN(x) (x)\n#define SET int s; s = 1;\n#define \
             WRAP(x) x\n#define LABEL lab:;\n",
        );
        for (source, in_gcc, in_clang) in [
            ("void f(int x) { NOOP(x); }", false, false),
            ("void f(int x) { PAREN(x); }", false, true),
            ("void f(void) { LOCAL }", false, true),
            ("void f(void) { SET }", false, true),
            ("STATICFN", false, true),
            ("STATICOBJ", false, true),
            ("PARAM", false, true),
            ("void f(int x) { WRAP(x + 1); }", true, true),
            ("void f(int x) { PAREN(x + 1); }", true, true),
            ("void f(void) { WRAP(int inner;) }", true, true),
            ("void f(void) { LABEL }", true, true),
        ] {
            directory.write("use.c", &format!("#include <sys.h>\n{source}\n"));
            for (mode, warns) in [("--gcc-mode", in_gcc), ("--clang-mode", in_clang)] {
                let output = directory.bcc(
                    mode,
                    &[
                        "-isystem",
                        "sys",
                        "-fsyntax-only",
                        "-Wall",
                        "-Wextra",
                        "use.c",
                    ],
                );
                succeeds(&output);
                assert_eq!(
                    stderr(&output).contains("[-Wunused"),
                    warns,
                    "{mode} {source}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// GCC also withholds its comparison and initializer warnings when a
    /// system header spells the operator or the closing brace; Clang goes by
    /// the invocation.
    #[test]
    fn extra_warnings_follow_each_compilers_system_macro_rule() {
        let directory = Directory::new();
        std::fs::create_dir(directory.path("sys")).unwrap();
        directory.write(
            "sys/sys.h",
            "#define LT(a, b) ((a) < (b))\n#define CHOOSE(c, a, b) ((c) ? (a) : (b))\n#define \
             INIT(v) struct S v = {1}\n#define WRAP(x) x\nstruct S { int a; int b; };\n",
        );
        for (source, in_gcc, in_clang) in [
            ("int f(int a, unsigned b) { return LT(a, b); }", false, true),
            (
                "unsigned f(int c, int a, unsigned b) { return CHOOSE(c, a, b); }",
                false,
                false,
            ),
            ("void f(void) { INIT(v); (void)v; }", false, true),
            (
                "int f(int a, unsigned b) { return WRAP(a < b); }",
                true,
                true,
            ),
            (
                "int f(int a, unsigned b) { return WRAP(a) < WRAP(b); }",
                true,
                true,
            ),
            ("struct S s = { WRAP(1) };", true, true),
        ] {
            directory.write("use.c", &format!("#include <sys.h>\n{source}\n"));
            for (mode, warns) in [("--gcc-mode", in_gcc), ("--clang-mode", in_clang)] {
                let output = directory.bcc(
                    mode,
                    &["-isystem", "sys", "-fsyntax-only", "-Wextra", "use.c"],
                );
                succeeds(&output);
                assert_eq!(
                    stderr(&output).contains("warning"),
                    warns,
                    "{mode} {source}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// The format checks of each compiler and the groups its options turn
    /// on: GCC's `-Wall` is its `-Wformat`, whose groups report only while
    /// it is on and which `-Wformat=2` extends with `-Wformat-security`;
    /// Clang reports the whole group by default, each part on its own.
    #[test]
    fn format_warnings_follow_their_options() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "int printf(const char *, ...);\nvoid f(long l, char *s, int i) {\n    \
             printf(\"%d\\n\", l);\n    printf(\"%d\\n\", i, i);\n    printf(\"\", i);\n    \
             printf(s);\n    printf(\"%d %d\\n\", i);\n    printf(\"%y\\n\");\n    \
             printf(\"a\\0b\");\n}\n",
        );
        let flags = |mode: &str, target: Option<&str>, args: &[&str]| {
            let args: Vec<&str> = target
                .into_iter()
                .chain(args.iter().copied())
                .chain(["-fsyntax-only", "a.c"])
                .collect();
            let output = directory.bcc(mode, &args);
            let text = stderr(&output);
            let mut flags: Vec<String> = text
                .lines()
                .filter(|line| line.starts_with("a.c:"))
                .filter_map(|line| line.rsplit_once("[-W"))
                .map(|(_, flag)| flag.trim_end_matches(']').to_owned())
                .collect();
            flags.sort_unstable();
            flags.dedup();
            flags.join(" ")
        };
        let gcc = "format-contains-nul format-extra-args format-zero-length format=";
        let clang = "format format-extra-args format-insufficient-args format-invalid-specifier \
                     format-security format-zero-length";
        // Each row was measured against GCC 14.2 and Clang 23.1, and for the
        // Windows targets, where GCC checks `ms_printf` and Clang the
        // Microsoft C runtime's formats, against MinGW's GCC 13.3 and Clang
        // 23.1 for `x86_64-pc-windows-msvc`. The host's own target is a
        // Windows one on a Windows host.
        let rows = [
            ("--gcc-mode", &[][..], ""),
            ("--gcc-mode", &["-Wall"][..], gcc),
            ("--gcc-mode", &["-Wformat"][..], gcc),
            (
                "--gcc-mode",
                &["-Wformat=2"][..],
                "format-contains-nul format-extra-args format-security format-zero-length format=",
            ),
            ("--gcc-mode", &["-Wformat=2", "-Wformat"][..], gcc),
            ("--gcc-mode", &["-Wall", "-Wno-format"][..], ""),
            ("--gcc-mode", &["-Wno-format", "-Wall"][..], ""),
            ("--gcc-mode", &["-Wformat-security"][..], ""),
            (
                "--gcc-mode",
                &["-Wall", "-Wformat-security"][..],
                "format-contains-nul format-extra-args format-security format-zero-length format=",
            ),
            (
                "--gcc-mode",
                &["-Wno-format-extra-args", "-Wall"][..],
                "format-contains-nul format-zero-length format=",
            ),
            (
                "--gcc-mode",
                &["-Wall", "-Werror=format"][..],
                "error=format-contains-nul error=format-extra-args error=format-zero-length \
                 error=format=",
            ),
            (
                "--gcc-mode",
                &["-Wall", "-Werror", "-Wno-error=format"][..],
                "error=format-contains-nul error=format-extra-args error=format-zero-length \
                 format=",
            ),
            ("--clang-mode", &[][..], clang),
            ("--clang-mode", &["-Wno-format"][..], ""),
            ("--clang-mode", &["-Wformat=0"][..], ""),
            ("--clang-mode", &["-Wno-format", "-Wall"][..], clang),
            ("--clang-mode", &["-Wall", "-Wno-format"][..], ""),
            ("--clang-mode", &["-Wformat=2"][..], clang),
            (
                "--clang-mode",
                &["-Wno-format", "-Wformat-security"][..],
                "format-security",
            ),
            (
                "--clang-mode",
                &["-Wno-format-security"][..],
                "format format-extra-args format-insufficient-args format-invalid-specifier \
                 format-zero-length",
            ),
            (
                "--clang-mode",
                &["-Werror=format"][..],
                "error,-Wformat error,-Wformat-extra-args error,-Wformat-insufficient-args \
                 error,-Wformat-invalid-specifier error,-Wformat-security \
                 error,-Wformat-zero-length",
            ),
            ("--clang-mode", &["-Werror", "-Wno-error=format"][..], clang),
        ];
        for target in [
            None,
            Some("--target=x86_64-w64-mingw32"),
            Some("--target=x86_64-pc-windows-msvc"),
        ] {
            for (mode, args, expected) in rows {
                assert_eq!(
                    flags(mode, target, args),
                    expected,
                    "{mode} {target:?} {args:?}"
                );
            }
        }
        // Each compiler's own groups are unknown to the other.
        let output = directory.bcc(
            "--gcc-mode",
            &["-Wformat-invalid-specifier", "-fsyntax-only", "a.c"],
        );
        assert!(!output.status.success(), "{}", stderr(&output));
        let output = directory.bcc(
            "--clang-mode",
            &["-Wformat-contains-nul", "-fsyntax-only", "a.c"],
        );
        assert!(
            stderr(&output).contains("unknown warning option '-Wformat-contains-nul'"),
            "{}",
            stderr(&output)
        );
    }

    /// Plain `_Complex` means `_Complex double` to GCC and Clang: Clang
    /// warns, GCC only under `-pedantic`, and `-pedantic-errors` refuses it
    /// in both (gcc.c-torture/execute/20070614-1.c).
    #[test]
    fn plain_complex_means_complex_double() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "plain.c",
            "_Complex v = 3.0 + 1.0i;\n_Static_assert(sizeof v == 2 * sizeof(double), \
             \"\");\n_Complex twice(_Complex z) { return z + z; }\nint main(void) { return \
             (int)__real__ twice(v) + (int)__imag__ twice(v); }\n",
        );
        for (mode, options, warns) in [
            ("--gcc-mode", &[][..], false),
            ("--gcc-mode", &["-pedantic"], true),
            ("--clang-mode", &[], true),
        ] {
            let mut args = options.to_vec();
            args.extend(["plain.c", "-o", "plain"]);
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            assert_eq!(
                stderr(&output).contains("_Complex"),
                warns,
                "{mode} {options:?}: {}",
                stderr(&output)
            );
            let run = Command::new(directory.path("plain")).output().unwrap();
            assert_eq!(run.status.code(), Some(8), "{mode} {options:?}");
        }
        // `-w` and a system header silence it before `-pedantic-errors`
        // can make it an error.
        std::fs::create_dir_all(directory.path("sys")).unwrap();
        directory.write("sys/plain.h", "_Complex in_header;\n");
        directory.write("header.c", "#include <plain.h>\n");
        // A bare declaration: `plain.c`'s imaginary constant is another
        // pedantic error, which GCC's `-w` does not silence.
        directory.write("bare.c", "_Complex bare;\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-pedantic-errors", "-fsyntax-only", "bare.c"]);
            assert!(!output.status.success(), "{mode}");
            let output =
                directory.bcc(mode, &["-w", "-pedantic-errors", "-fsyntax-only", "bare.c"]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode} -w");
            let output = directory.bcc(
                mode,
                &[
                    "-pedantic-errors",
                    "-isystem",
                    "sys",
                    "-fsyntax-only",
                    "header.c",
                ],
            );
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode} system header");
        }
        // GCC's `-Wno-error=pedantic` keeps its warning one, and a later
        // `-Wno-pedantic` silences it; `-Werror=pedantic` makes it an
        // error. Clang's error outlasts both, and is no `-Wpedantic`
        // diagnostic for `-Werror=pedantic` to promote.
        for (mode, options, status) in [
            (
                "--gcc-mode",
                &["-pedantic-errors", "-Wno-error=pedantic"][..],
                0,
            ),
            (
                "--gcc-mode",
                &["-Wno-error=pedantic", "-pedantic-errors"],
                1,
            ),
            ("--gcc-mode", &["-pedantic-errors", "-Wno-pedantic"], 0),
            ("--gcc-mode", &["-Werror=pedantic"], 1),
            ("--gcc-mode", &["-Werror=pedantic", "-w"], 0),
            (
                "--gcc-mode",
                &["-Werror=pedantic", "-Wno-error=pedantic"],
                0,
            ),
            ("--gcc-mode", &["-Werror=pedantic", "-Wno-pedantic"], 0),
            ("--gcc-mode", &["-Wno-pedantic", "-Werror=pedantic"], 1),
            (
                "--gcc-mode",
                &["-pedantic-errors", "-Wno-pedantic", "-Wpedantic"],
                1,
            ),
            // `-Wno-pedantic` stops the diagnostic but keeps its error state.
            (
                "--gcc-mode",
                &["-Werror=pedantic", "-Wno-pedantic", "-Wpedantic"],
                1,
            ),
            (
                "--gcc-mode",
                &["-Werror=pedantic", "-Wno-pedantic", "-pedantic"],
                1,
            ),
            (
                "--gcc-mode",
                &[
                    "-Werror=pedantic",
                    "-Wno-error=pedantic",
                    "-Wno-pedantic",
                    "-Wpedantic",
                ],
                0,
            ),
            (
                "--gcc-mode",
                &[
                    "-Werror=pedantic",
                    "-Wno-pedantic",
                    "-Wno-error=pedantic",
                    "-Wpedantic",
                ],
                0,
            ),
            ("--clang-mode", &["-Werror=pedantic"], 0),
            ("--clang-mode", &["-pedantic-errors", "-Wno-pedantic"], 1),
            (
                "--clang-mode",
                &["-pedantic-errors", "-Wno-error=pedantic"],
                1,
            ),
        ] {
            let mut args = options.to_vec();
            args.extend(["-fsyntax-only", "bare.c"]);
            let output = directory.bcc(mode, &args);
            assert_eq!(output.status.code(), Some(status), "{mode} {options:?}");
        }
        // TCC refuses `_Complex` altogether.
        let output = directory.bcc("--tcc-mode", &["-c", "plain.c", "-o", "plain.o"]);
        assert!(!output.status.success());
    }

    /// GCC reads its pedantic options in order. Clang reads `-pedantic` and
    /// `-pedantic-errors` before every `-W` option, so `-Wno-error=pedantic`
    /// and `-Wno-pedantic` outrank them wherever they appear. In both,
    /// `-Wpedantic` keeps pedantic errors errors. `-Werror=pedantic` makes
    /// only GCC's plain `_Complex` diagnostic an error so far: it leaves the
    /// other extensions as they were, and `-w` still silences it.
    #[test]
    fn pedantic_options_combine_as_each_compiler_reads_them() {
        let directory = Directory::new();
        directory.write("empty.c", "int a[1] = {};\n");
        for (mode, options, status) in [
            (
                "--gcc-mode",
                &["-Wno-error=pedantic", "-pedantic-errors"][..],
                1,
            ),
            (
                "--gcc-mode",
                &["-pedantic-errors", "-Wno-error=pedantic"],
                0,
            ),
            ("--gcc-mode", &["-Werror=pedantic", "-Wno-pedantic"], 0),
            (
                "--gcc-mode",
                &["-Werror=pedantic", "-Wno-error=pedantic"],
                0,
            ),
            ("--gcc-mode", &["-pedantic-errors", "-Wpedantic"], 1),
            (
                "--clang-mode",
                &["-Wno-error=pedantic", "-pedantic-errors"],
                0,
            ),
            (
                "--clang-mode",
                &["-pedantic-errors", "-Wno-error=pedantic"],
                0,
            ),
            ("--clang-mode", &["-Wno-pedantic", "-pedantic-errors"], 0),
            ("--clang-mode", &["-pedantic-errors", "-Wpedantic"], 1),
        ] {
            let mut args = vec!["-std=c17"];
            args.extend(options);
            args.extend(["-fsyntax-only", "empty.c"]);
            let output = directory.bcc(mode, &args);
            assert_eq!(
                output.status.code(),
                Some(status),
                "{mode} {options:?}: {}",
                stderr(&output)
            );
        }
        let output = directory.bcc(
            "--clang-mode",
            &[
                "-std=c17",
                "-Wno-pedantic",
                "-pedantic-errors",
                "-fsyntax-only",
                "empty.c",
            ],
        );
        assert_eq!(stderr(&output), "");
        // `-Werror=pedantic` neither outlasts `-w` nor reaches a diagnostic
        // GCC files under another option (`-Wlong-long`).
        directory.write("zero.c", "int a[0];\n");
        directory.write("ll.c", "long long x;\n");
        for (mode, args) in [
            (
                "--gcc-mode",
                &["-std=c17", "-Werror=pedantic", "-w", "zero.c"][..],
            ),
            (
                "--clang-mode",
                &["-std=c17", "-Werror=pedantic", "-w", "zero.c"],
            ),
            (
                "--gcc-mode",
                &["-std=c17", "-w", "-Werror=pedantic", "zero.c"],
            ),
            ("--gcc-mode", &["-std=c89", "-Werror=pedantic", "ll.c"]),
        ] {
            let mut args = args.to_vec();
            args.push("-fsyntax-only");
            let output = directory.bcc(mode, &args);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{mode} {args:?}: {}",
                stderr(&output)
            );
        }
    }

    /// TCC names no warning options, so its diagnostics carry no `[-W…]`
    /// suffix, as GCC's and Clang's do.
    #[test]
    fn tcc_diagnostics_name_no_warning_options() {
        let directory = Directory::new();
        directory.write(
            "warn.c",
            "int main(void) { long b = 1; char *p = b; return 0; }\n",
        );
        let tcc = directory.bcc("--tcc-mode", &["-c", "warn.c"]);
        succeeds(&tcc);
        assert!(
            stderr(&tcc).starts_with("warn.c:1: warning: "),
            "{}",
            stderr(&tcc)
        );
        assert!(!stderr(&tcc).contains("[-W"), "{}", stderr(&tcc));
        let gcc = directory.bcc("--gcc-mode", &["-c", "warn.c"]);
        assert!(stderr(&gcc).contains("[-W"), "{}", stderr(&gcc));
        // The diagnostics TCC's users meet most read in TCC's words (TCC mob
        // e23dabe), each stopping the compilation at the first error.
        for (source, expected) in [
            (
                "int main(void){ return x; }",
                "e.c:1: error: 'x' undeclared\n",
            ),
            (
                "int main(void){ int *p; char *q = p; return 0; }",
                "e.c:1: warning: assignment from incompatible pointer type\n",
            ),
            (
                "int main(void){ int a; int a; return 0; }",
                "e.c:1: error: redeclaration of 'a'\n",
            ),
            ("int main(void){ break; }", "e.c:1: error: cannot break\n"),
            (
                "int main(void){ goto L; }",
                "e.c:1: error: label 'L' used but not defined\n",
            ),
            (
                "int main(void){ int a[2]; a = 0; return 0; }",
                "e.c:1: error: lvalue expected\n",
            ),
            (
                "int g(void){ 1 = 2; return 0; }",
                "e.c:1: error: lvalue expected\n",
            ),
            (
                "int main(void){ for (;;) {} continue; }",
                "e.c:1: error: cannot continue\n",
            ),
            (
                "typedef int T; typedef float T;",
                "e.c:1: error: incompatible redefinition of 'T'\n",
            ),
            (
                "int T; typedef int T;",
                "e.c:1: error: incompatible redefinition of 'T'\n",
            ),
            (
                "int f(void); float f(void);",
                "e.c:1: error: incompatible types for redefinition of 'f'\n",
            ),
            (
                "int x = 1; int x = 2;",
                "e.c:1: error: redefinition of 'x'\n",
            ),
            // A user's text keeps what only looks like an option.
            (
                "#warning see [-Wfoo]\nint x;",
                "e.c:1: warning: #warning see [-Wfoo]\n",
            ),
        ] {
            directory.write("e.c", &format!("{source}\n"));
            let output = directory.bcc("--tcc-mode", &["-c", "e.c"]);
            assert_eq!(stderr(&output), expected, "{source}");
        }
        // TCC writes through a const-qualified lvalue with a warning, and
        // assigns a structure with a const member silently.
        directory.write(
            "ro.c",
            "int printf(const char *, ...);\nstruct S { const int m; int n; } s1 = {1, 2}, s2 = \
             {3, 4};\nint main(void) {\n  const int c = 1;\n  int *q = (int *)&c;\n  c = 5;\n  s1 \
             = s2;\n  printf(\"%d %d %d\\n\", *q, s1.m, s1.n);\n  return 0;\n}\n",
        );
        let output = directory.bcc("--tcc-mode", &["-c", "ro.c"]);
        assert_eq!(
            stderr(&output),
            "ro.c:6: warning: assignment of read-only location\n"
        );
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            succeeds(&directory.bcc("--tcc-mode", &["-w", "ro.c", "-o", "ro"]));
            let run = Command::new(directory.path("ro")).output().unwrap();
            assert_eq!(stdout(&run), "5 3 4\n");
        }
        // Other identities keep C's constraint.
        let gcc = directory.bcc("--gcc-mode", &["-c", "ro.c"]);
        assert!(!gcc.status.success());
    }

    /// GCC 14 refuses an implicit function declaration in C99 and later
    /// unless `-fpermissive` or `-Wno-error=` makes it a warning, and
    /// accepts it in GNU89; Clang refuses it whatever `-fpermissive` and
    /// `-w` say; TCC warns, and `-Wno-implicit-function-declaration`
    /// silences it.
    #[test]
    fn implicit_declarations_follow_each_driver() {
        let directory = Directory::new();
        directory.write(
            "i.c",
            "int main(void) { return f(1); }\nint f(int x) { return x - 1; }\n",
        );
        let compile = |mode: &str, options: &[&str]| {
            let mut args = vec!["-c", "i.c", "-o", "i.o"];
            args.extend_from_slice(options);
            directory.bcc(mode, &args)
        };
        let gcc_error = "i.c:1:25: error: implicit declaration of function 'f' \
                         [-Wimplicit-function-declaration]";
        let refused = compile("--gcc-mode", &[]);
        assert!(!refused.status.success());
        assert!(stderr(&refused).contains(gcc_error), "{}", stderr(&refused));
        let refused = compile("--gcc-mode", &["-w"]);
        assert!(!refused.status.success(), "{}", stderr(&refused));
        for options in [
            &["-fpermissive"][..],
            &["-Wno-error=implicit-function-declaration"],
        ] {
            let warned = compile("--gcc-mode", options);
            succeeds(&warned);
            assert!(
                stderr(&warned).contains(&gcc_error.replace("error:", "warning:")),
                "{options:?}: {}",
                stderr(&warned)
            );
        }
        for options in [&["-std=gnu89"][..], &["-Wno-implicit-function-declaration"]] {
            let silent = compile("--gcc-mode", options);
            succeeds(&silent);
            assert_eq!(stderr(&silent), "", "{options:?}");
        }
        for options in [&[][..], &["-fpermissive"], &["-w"]] {
            let refused = compile("--clang-mode", options);
            assert!(!refused.status.success(), "{options:?}");
            assert!(
                stderr(&refused).contains(
                    "error: call to undeclared function 'f'; ISO C99 and later do not support \
                     implicit function declarations [-Wimplicit-function-declaration]"
                ),
                "{options:?}: {}",
                stderr(&refused)
            );
        }
        succeeds(&compile(
            "--clang-mode",
            &["-Wno-error=implicit-function-declaration"],
        ));
        let tcc = compile("--tcc-mode", &[]);
        succeeds(&tcc);
        assert_eq!(
            stderr(&tcc),
            "i.c:1: warning: implicit declaration of function 'f'\n"
        );
        let tcc = compile("--tcc-mode", &["-Wno-implicit-function-declaration"]);
        succeeds(&tcc);
        assert_eq!(stderr(&tcc), "");
    }

    /// GCC 14 reports its permerrors in a system header too, when they are
    /// errors; Clang withholds its default errors there, and every warning
    /// stays withheld.
    #[test]
    fn permerrors_in_system_headers_follow_each_compiler() {
        let directory = Directory::new();
        std::fs::create_dir_all(directory.path("sys")).unwrap();
        directory.write("main.c", "#include <bad.h>\nint main(void) { return 0; }\n");
        for (header, gcc_message) in [
            (
                "void f(int *p){long *q=p;}",
                "error: initialization of 'long int *' from incompatible pointer type 'int *' \
                 [-Wincompatible-pointer-types]",
            ),
            (
                "int f(void){return frob();}",
                "error: implicit declaration of function 'frob' [-Wimplicit-function-declaration]",
            ),
            (
                "extern x;",
                "error: type defaults to 'int' in declaration of 'x' [-Wimplicit-int]",
            ),
            (
                "int f(void){return;}",
                "error: 'return' with no value, in function returning non-void [-Wreturn-mismatch]",
            ),
            (
                "void f(int c,int *p,long *q){void *x=c?p:q;}",
                "error: pointer type mismatch in conditional expression \
                 [-Wincompatible-pointer-types]",
            ),
        ] {
            directory.write("sys/bad.h", &format!("{header}\n"));
            let compile = |mode: &str, options: &[&str]| {
                let mut args = vec!["-std=gnu17", "-isystem", "sys", "-fsyntax-only"];
                args.extend_from_slice(options);
                args.push("main.c");
                directory.bcc(mode, &args)
            };
            let gcc = compile("--gcc-mode", &[]);
            assert!(!gcc.status.success(), "{header}");
            assert!(
                stderr(&gcc).contains(gcc_message),
                "{header}: {}",
                stderr(&gcc)
            );
            for (mode, options) in [("--gcc-mode", &["-fpermissive"][..]), ("--clang-mode", &[])] {
                let quiet = compile(mode, options);
                succeeds(&quiet);
                assert_eq!(stderr(&quiet), "", "{header} {mode} {options:?}");
            }
        }
    }

    /// GCC looks past null statements for a statement expression's value, and
    /// Clang gives that block none.
    #[test]
    fn statement_expression_value_before_null_statements_follows_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "s.c",
            "#include <stdio.h>\nstatic int f(void) { return ({ int x = 9; x;; }); }\nint \
             main(void) {\n  int a = ({ 7;; });\n  int b = ({ l: 8; ; ; });\n  int c = sizeof(({ \
             1;; }));\n  if (({ 0;; })) puts(\"yes\"); else puts(\"no\");\n  printf(\"%d %d %d \
             %d\\n\", a, b, f(), c);\n  (void)&&l;\n  return 0;\n}\n",
        );
        for options in [&["-O0"][..], &["-O2"]] {
            let args = [options, &["-w", "s.c", "-o", "s"]].concat();
            succeeds(&directory.bcc("--gcc-mode", &args));
            let run = Command::new(directory.path("s")).output().unwrap();
            assert_eq!(stdout(&run), "no\n7 8 9 4\n", "{options:?}");
        }
        directory.write("v.c", "int f(void) { return ({ 1;; }); }\n");
        assert!(
            !directory
                .bcc("--clang-mode", &["-c", "v.c", "-o", "v.o"])
                .status
                .success()
        );
        // Other statements after the value leave GCC's block without one too.
        directory.write("n.c", "int f(void) { return ({ 1; {} }); }\n");
        assert!(
            !directory
                .bcc("--gcc-mode", &["-c", "n.c", "-o", "n.o"])
                .status
                .success()
        );
    }

    /// `-m` options and `-march=` define each compiler's macros and let LLVM
    /// use the instructions, with what a feature needs turned on and what needs
    /// it turned off.
    #[test]
    fn gnu_target_features_define_macros_and_generate_code() {
        let directory = Directory::new();
        directory.write("m.c", "");
        directory.write(
            "p.c",
            "int f(unsigned x) { return __builtin_popcount(x); }\nvoid g(int *restrict a, int \
             *restrict b) { for (int i = 0; i < 64; i++) a[i] += b[i]; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            let has = |options: &[&str], name: &str| {
                let args = [options, &["-E", "-dM", "m.c"]].concat();
                stdout(&directory.bcc(mode, &args)).contains(&format!("#define {name} 1\n"))
            };
            for name in [
                "__AVX2__",
                "__AVX__",
                "__SSE4_2__",
                "__SSE3__",
                "__POPCNT__",
                "__XSAVE__",
            ] {
                assert!(has(&["-mavx2"], name), "{mode} {name}");
            }
            assert!(!has(&[], "__AVX2__"), "{mode}");
            assert!(!has(&["-mavx2"], "__BMI__"), "{mode}");
            // Turning a feature off takes what needs it along.
            assert!(!has(&["-mavx2", "-mno-avx"], "__AVX2__"), "{mode}");
            assert!(has(&["-mavx2", "-mno-avx"], "__SSE4_2__"), "{mode}");
            assert!(!has(&["-mavx2", "-mno-avx2"], "__AVX2__"), "{mode}");
            assert!(has(&["-msse4"], "__SSE4_2__"), "{mode}");
            assert!(!has(&["-msse4", "-mno-sse4"], "__SSE4_1__"), "{mode}");
            assert!(has(&["-msse4", "-mno-sse4"], "__SSSE3__"), "{mode}");
            assert!(has(&["-march=x86-64-v2"], "__SSE4_2__"), "{mode}");
            assert!(!has(&["-march=x86-64-v2"], "__AVX__"), "{mode}");
            for name in [
                "__AVX2__",
                "__FMA__",
                "__BMI2__",
                "__MOVBE__",
                "__LAHF_SAHF__",
            ] {
                assert!(has(&["-march=x86-64-v3"], name), "{mode} {name}");
            }
            assert_eq!(
                has(&["-mfma"], "__FP_FAST_FMA"),
                mode == "--gcc-mode",
                "{mode}"
            );
            assert!(
                has(&["-march=x86-64-v2"], "__GCC_HAVE_SYNC_COMPARE_AND_SWAP_16"),
                "{mode}"
            );
            assert!(
                !has(
                    &["-march=x86-64-v2", "-march=x86-64"],
                    "__GCC_HAVE_SYNC_COMPARE_AND_SWAP_16"
                ),
                "{mode}"
            );
            // The last `-march=` names defaults; explicit options outrank them.
            assert!(
                !has(&["-march=x86-64-v3", "-march=x86-64"], "__AVX2__"),
                "{mode}"
            );
            assert!(
                has(&["-mavx2", "-march=x86-64-v3", "-march=x86-64"], "__AVX2__"),
                "{mode}"
            );
            assert!(
                has(&["-march=x86-64-v3", "-march=x86-64", "-mavx2"], "__AVX2__"),
                "{mode}"
            );
            assert!(
                !has(&["-mno-avx", "-march=x86-64-v3"], "__AVX2__"),
                "{mode}"
            );
            assert!(has(&["-mno-avx", "-march=x86-64-v3"], "__BMI2__"), "{mode}");
            // POPCNT and CRC32 come with SSE4.2, which doesn't need them.
            for options in [&["-mavx2", "-mno-popcnt"], &["-mno-popcnt", "-mavx2"]] {
                assert!(has(options, "__AVX2__"), "{mode} {options:?}");
                assert!(!has(options, "__POPCNT__"), "{mode} {options:?}");
                assert!(has(options, "__CRC32__"), "{mode} {options:?}");
            }
            assert!(has(&["-mavx", "-mno-crc32"], "__AVX__"), "{mode}");
            assert!(!has(&["-mavx", "-mno-crc32"], "__CRC32__"), "{mode}");
            // AVX needs XSAVE in GCC; in Clang it comes along unless refused.
            assert_eq!(
                has(&["-mavx", "-mno-xsave"], "__AVX__"),
                mode == "--clang-mode"
            );
            assert_eq!(
                has(&["-mavx2", "-mno-avx"], "__XSAVE__"),
                mode == "--gcc-mode"
            );
            let assembly = |options: &[&str]| {
                let args = [options, &["-O2", "-S", "p.c", "-o", "-"]].concat();
                let output = directory.bcc(mode, &args);
                succeeds(&output);
                assert_eq!(stderr(&output), "", "{mode} {options:?}");
                stdout(&output)
            };
            assert!(!assembly(&[]).contains("popcnt"), "{mode}");
            assert!(assembly(&["-mpopcnt"]).contains("popcnt"), "{mode}");
            assert!(assembly(&["-mavx2"]).contains("vpaddd"), "{mode}");
            assert!(assembly(&["-march=x86-64-v3"]).contains("vpaddd"), "{mode}");
            assert!(
                !assembly(&["-mavx2", "-mno-avx2", "-mno-avx"]).contains("vpaddd"),
                "{mode}"
            );
            // The processor would have them; `-mno-avx` has to tell LLVM.
            assert!(
                !assembly(&["-march=x86-64-v3", "-mno-avx"]).contains("vpaddd"),
                "{mode}"
            );
            assert!(assembly(&["-march=x86-64-v3", "-mno-avx"]).contains("paddd"));
            assert!(
                !assembly(&["-march=x86-64-v3", "-mno-popcnt"]).contains("popcnt"),
                "{mode}"
            );
            assert!(
                !assembly(&["-march=x86-64-v3", "-march=x86-64"]).contains("popcnt"),
                "{mode}"
            );
            // `native` is the running processor, whatever that has.
            succeeds(&directory.bcc(mode, &["-march=native", "-O2", "-c", "p.c", "-o", "p.o"]));
        }
    }

    /// x86 feature options on a WebAssembly target are refused in each
    /// compiler's words and leave no macros behind. Clang leaves `-msse4.2` and
    /// `-msse4` unused; GCC has no such target, and says what a GCC for another
    /// processor says of options it doesn't know.
    #[test]
    fn x86_feature_options_are_refused_on_webassembly() {
        let directory = Directory::new();
        directory.write("m.c", "");
        let run = |mode: &str, option: &str| {
            directory.bcc(
                mode,
                &[
                    "--target=wasm32-unknown-unknown",
                    option,
                    "-E",
                    "-dM",
                    "m.c",
                ],
            )
        };
        let triple = "wasm32-unknown-unknown";
        for (option, words) in [
            ("-mavx2", "-mavx2".to_owned()),
            ("-mno-avx2", "-mno-avx2".to_owned()),
            ("-mno-sse4", "-mno-sse4.1".to_owned()),
            ("-mavx512f", "-mavx512f".to_owned()),
            ("-mmmx", "-mmmx".to_owned()),
            ("-march=x86-64-v3", "-march=".to_owned()),
            ("-march=haswell", "-march=".to_owned()),
        ] {
            let output = run("--clang-mode", option);
            assert_eq!(output.status.code(), Some(1), "{option}");
            assert!(
                stderr(&output).contains(&format!(
                    "unsupported option '{words}' for target '{triple}'"
                )),
                "{}",
                stderr(&output)
            );
            assert!(!stdout(&output).contains("__AVX"), "{option}");
            let output = run("--gcc-mode", option);
            assert_eq!(output.status.code(), Some(1), "{option}");
            let wanted = option.strip_prefix("-march=").map_or_else(
                || format!("unrecognized command-line option '{option}'"),
                |name| format!("bad value '{name}' for '-march=' switch"),
            );
            assert!(stderr(&output).contains(&wanted), "{}", stderr(&output));
        }
        for option in ["-msse4.2", "-msse4"] {
            let output = run("--clang-mode", option);
            succeeds(&output);
            assert!(
                stderr(&output)
                    .contains(&format!("argument unused during compilation: '{option}'")),
                "{}",
                stderr(&output)
            );
            assert!(!stdout(&output).contains("__SSE4"), "{option}");
        }
    }

    /// An option with a later opposite doesn't count at all in either compiler:
    /// `-mavx -mno-avx` leaves no SSE4.2, while `-mavx2 -mno-avx` does, and an
    /// earlier `-mno-avx` turned around by `-mavx` doesn't stop `-march=` from
    /// turning on what needs AVX.
    #[test]
    fn a_later_opposite_option_cancels_the_earlier() {
        let directory = Directory::new();
        directory.write("m.c", "");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let has = |options: &[&str], name: &str| {
                let args = [options, &["-E", "-dM", "m.c"]].concat();
                stdout(&directory.bcc(mode, &args)).contains(&format!("#define {name} 1\n"))
            };
            assert!(!has(&["-mavx", "-mno-avx"], "__SSE4_2__"), "{mode}");
            assert!(has(&["-mavx2", "-mno-avx"], "__SSE4_2__"), "{mode}");
            assert!(!has(&["-mssse3", "-mno-ssse3"], "__SSE3__"), "{mode}");
            assert!(
                has(&["-mno-avx", "-mavx", "-march=x86-64-v3"], "__FMA__"),
                "{mode}"
            );
            assert!(!has(&["-msse4.2", "-mno-sse4.2"], "__SSE4_1__"), "{mode}");
            assert!(has(&["-mno-sse4.1", "-msse4.2"], "__SSE4_1__"), "{mode}");
            assert!(!has(&["-msse4.2", "-mno-sse4.1"], "__SSE4_1__"), "{mode}");
        }
        // Clang reads `-msse4` as `-msse4.2`, GCC as an option of its own.
        let has = |mode: &str, options: &[&str], name: &str| {
            let args = [options, &["-E", "-dM", "m.c"]].concat();
            stdout(&directory.bcc(mode, &args)).contains(&format!("#define {name} 1\n"))
        };
        assert!(!has(
            "--clang-mode",
            &["-msse4", "-mno-sse4.2"],
            "__SSE4_1__"
        ));
        assert!(has("--gcc-mode", &["-msse4", "-mno-sse4.2"], "__SSE4_1__"));
    }

    /// With AVX, 32-byte vectors and their wrappers travel in YMM registers, so
    /// bcc's objects link with GCC's and Clang's both ways, at `-O0` and `-O2`.
    /// Skipped without AVX on the machine or a reference compiler.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn avx_vectors_cross_objects_of_each_compiler() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let directory = Directory::new();
        let types = "typedef int V __attribute__((vector_size(32)));\ntypedef int W \
                     __attribute__((vector_size(16)));\ntypedef struct { V v; } S;\ntypedef union \
                     { V v; float f; } U;\ntypedef struct { double d[4]; } M;\n";
        directory.write(
            "lib.c",
            &format!(
                "#include <stdarg.h>\n{types}\nV add(V a, V b) {{ return a + b; }}\nW small(W a, \
                 double d) {{ return a + (int)d; }}\nS wrap(int i, S s, V v) {{ s.v += v + i; \
                 return s; }}\nU un(U u) {{ u.v += u.v; return u; }}\nM mem(M m) {{ m.d[3] += 1; \
                 return m; }}\nint last(V a, V b, V c, V d, V e, V f, V g, V h, V i, int k) {{ \
                 return a[0] + h[7] + i[1] + k; }}\nint var(int n, ...) {{ va_list ap; \
                 va_start(ap, n); V v = va_arg(ap, V); int i = va_arg(ap, int); S s = va_arg(ap, \
                 S); va_end(ap); return v[7] + i + s.v[1]; }}\n"
            ),
        );
        directory.write(
            "client.c",
            &format!(
                "#include <stdio.h>\n{types}V add(V, V); W small(W, double); S wrap(int, S, V); U \
                 un(U); M mem(M); int last(V, V, V, V, V, V, V, V, V, int); int var(int,                  ...);\nint main(void) {{ V a = {{1, 2, 3, 4, 5, 6, 7, 8}}, b = {{10, 20, 30, 40,                  50, 60, 70, 80}}; V r = add(a, b); W w = small((W){{1, 2, 3, 4}}, 5.0); S s =                  wrap(1, (S){{a}}, b); U u = un((U){{.v = a}}); M m = mem((M){{{{1, 2, 3, 4}}}});                  printf(\"%d %d %d %d %g %d %d\\n\", r[7], w[3], s.v[7], u.v[3], m.d[3], last(a,                  a, a, a, a, a, a, a, a, 100), var(1, a, 9, (S){{b}})); return 0; }}\n"
            ),
        );
        let clang = concat!(env!("CARGO_MANIFEST_DIR"), "/target/llvm/bin/clang");
        let references = [("--gcc-mode", "gcc"), ("--clang-mode", clang)];
        for (mode, compiler) in references {
            let available = Command::new(compiler).arg("--version").output();
            if !available.is_ok_and(|output| output.status.success()) {
                continue;
            }
            for flags in [["-O0", "-mavx2"], ["-O2", "-march=x86-64-v3"]] {
                let reference = |source: &str, object: &str| {
                    let output = Command::new(compiler)
                        .current_dir(directory.path("."))
                        .args(flags)
                        .args(["-w", "-c", source, "-o", object])
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "{}", stderr(&output));
                };
                let bcc = |source: &str, object: &str| {
                    let args = [&flags[..], &["-w", "-c", source, "-o", object]].concat();
                    succeeds(&directory.bcc(mode, &args));
                };
                reference("lib.c", "r-lib.o");
                reference("client.c", "r-client.o");
                bcc("lib.c", "b-lib.o");
                bcc("client.c", "b-client.o");
                for (library, client) in [
                    ("b-lib.o", "r-client.o"),
                    ("r-lib.o", "b-client.o"),
                    ("b-lib.o", "b-client.o"),
                ] {
                    succeeds(&directory.bcc(mode, &[library, client, "-o", "mixed"]));
                    let run = Command::new(directory.path("mixed")).output().unwrap();
                    assert_eq!(
                        String::from_utf8_lossy(&run.stdout),
                        "88 9 89 8 5 111 37\n",
                        "{mode} {flags:?} {library} {client}"
                    );
                }
            }
        }
    }

    /// Both compilers place a vector on the stack by its size, whatever
    /// `aligned` its typedef says, in every direction across objects, named or
    /// not. Skipped without AVX on the machine or a reference compiler.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn aligned_vector_typedefs_keep_their_stack_slots() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let directory = Directory::new();
        let clang = concat!(env!("CARGO_MANIFEST_DIR"), "/target/llvm/bin/clang");
        for attributes in [
            "vector_size(32), aligned(1)",
            "vector_size(32), aligned(64)",
            "vector_size(16), aligned(1)",
            "vector_size(8), aligned(32)",
            "vector_size(64), aligned(8)",
            "vector_size(128), aligned(1)",
        ] {
            let types = format!(
                "typedef int T __attribute__(({attributes}));\n#define N (int)(sizeof(T) / \
                 sizeof(int))\n"
            );
            directory.write(
                "lib.c",
                &format!(
                    "#include <stdarg.h>\n{types}int low(long a, long b, long c, long d, long e, \
                     long f, long g, T v0, T v1, T v2, T v3, T v4, T v5, T v6, T v7, T v8, long \
                     z) {{ return g != 7 || z != 99 || v0[0] != 1 || v8[N - 1] != N || (unsigned \
                     long)&v8 % _Alignof(T) != 0; }}\nint var(long g, ...) {{ va_list ap; \
                     va_start(ap, g); int bad = g != 1; for (int i = 2; i <= 7; i++) bad |= \
                     va_arg(ap, long) != i; for (int i = 0; i < 9; i++) {{ T v = va_arg(ap, T); \
                     bad |= v[N - 1] != N; }} bad |= va_arg(ap, long) != 99; va_end(ap); return \
                     bad; }}\n"
                ),
            );
            directory.write(
                "client.c",
                &format!(
                    "{types}int low(long, long, long, long, long, long, long, T, T, T, T, T, T, \
                     T, T, T, long); int var(long, ...);\nint main(void) {{ T v; for (int i = 0; \
                     i < N; i++) v[i] = i + 1; return low(1, 2, 3, 4, 5, 6, 7, v, v, v, v, v, v, \
                     v, v, v, 99) || var(1L, 2L, 3L, 4L, 5L, 6L, 7L, v, v, v, v, v, v, v, v, v, \
                     99L); }}\n"
                ),
            );
            for (mode, compiler) in [("--gcc-mode", "gcc"), ("--clang-mode", clang)] {
                let available = Command::new(compiler).arg("--version").output();
                if !available.is_ok_and(|output| output.status.success()) {
                    continue;
                }
                for flags in [&["-O0", "-mavx2"][..], &["-O2"]] {
                    let reference = |source: &str, object: &str| {
                        let output = Command::new(compiler)
                            .current_dir(directory.path("."))
                            .args(flags)
                            .args(["-w", "-c", source, "-o", object])
                            .output()
                            .unwrap();
                        assert!(output.status.success(), "{}", stderr(&output));
                    };
                    let bcc = |source: &str, object: &str| {
                        let args = [flags, &["-w", "-c", source, "-o", object]].concat();
                        succeeds(&directory.bcc(mode, &args));
                    };
                    reference("lib.c", "r-lib.o");
                    reference("client.c", "r-client.o");
                    bcc("lib.c", "b-lib.o");
                    bcc("client.c", "b-client.o");
                    for (library, client) in [
                        ("b-lib.o", "r-client.o"),
                        ("r-lib.o", "b-client.o"),
                        ("b-lib.o", "b-client.o"),
                    ] {
                        succeeds(&directory.bcc(mode, &[library, client, "-o", "mixed"]));
                        let run = Command::new(directory.path("mixed")).output().unwrap();
                        assert_eq!(
                            run.status.code(),
                            Some(0),
                            "{attributes} {mode} {flags:?} {library} {client}"
                        );
                    }
                }
            }
        }
    }

    /// What the driver leaves out is still read in order: an option outside the
    /// table refuses a vector its convention changes until AVX is off in that
    /// compiler or the option's own opposite follows, wherever the options
    /// stand. Each row was measured with `-dM -E` on GCC 14.2 and Clang 23.1.1.
    #[test]
    fn unknown_vector_options_refuse_as_each_compiler_reads_them() {
        let directory = Directory::new();
        directory.write(
            "v.c",
            "typedef int V __attribute__((vector_size(32)));\nV add(V a, V b) { return a + b; }\n",
        );
        let reads = |mode: &str, options: &[&str]| {
            let args = [options, &["-w", "-c", "v.c", "-o", "v.o"]].concat();
            let output = directory.bcc(mode, &args);
            let refused = stderr(&output).contains("ignores changes how it travels");
            assert_eq!(output.status.success(), !refused, "{}", stderr(&output));
            refused
        };
        for mode in ["--gcc-mode", "--clang-mode"] {
            for options in [
                &["-mavx512f", "-mno-avx512bw"][..],
                &["-mavx512bw", "-mno-avx512f"],
                &["-mavx512f", "-mavx512bw", "-mno-avx512f"],
                &["-mxop", "-mno-fma4"],
                &["-mavx512f", "-mno-avx2"],
                &["-mavx512f", "-mno-avx", "-mavx"],
                &["-mno-avx", "-mavx512f"],
                &["-march=haswell"],
                &["-march=haswell", "-mno-bmi"],
            ] {
                assert!(reads(mode, options), "{mode} {options:?}");
            }
            for options in [
                &["-mavx512f", "-mno-avx512f"][..],
                &["-mavx512f", "-mno-avx"],
                &["-mavx512f", "-mno-sse4.2"],
                &["-mavx", "-mavx512f", "-mno-avx"],
                &["-march=haswell", "-mno-avx"],
                &["-mno-avx", "-march=haswell"],
                &["-march=haswell", "-march=x86-64"],
            ] {
                assert!(!reads(mode, options), "{mode} {options:?}");
            }
            for options in [
                &["-mavx512f", "-mno-xsave"][..],
                &["-march=haswell", "-mno-xsave"],
            ] {
                assert_eq!(
                    reads(mode, options),
                    mode == "--clang-mode",
                    "{mode} {options:?}"
                );
            }
        }
        assert!(!reads("--gcc-mode", &["-mavx256-split-unaligned-load"]));
        assert!(reads("--gcc-mode", &["-msse5"]));
        // clang-cl returns the vector in YMM0 under these, as `clang
        // --driver-mode=cl` was measured to, and takes the same options.
        for (options, refused) in [
            (&["-mavx512f"][..], true),
            (&["-mavx512f", "-mno-avx512bw"], true),
            (&["-march=haswell"], true),
            (&["-mavx512f", "-mno-avx512f"], false),
            (&["-mavx512f", "-mno-avx"], false),
            (&["-mavx2"], false),
        ] {
            let args = [options, &["/c", "v.c"]].concat();
            let output = directory.bcc("--clang-cl-mode", &args);
            assert_eq!(
                stderr(&output).contains("ignores changes how it travels"),
                refused,
                "{options:?}: {}",
                stderr(&output)
            );
            assert_eq!(output.status.success(), !refused, "{options:?}");
        }
    }

    /// `__builtin_cpu_supports` and `__builtin_cpu_is` read libgcc's data as
    /// each compiler does: GCC gives the bit where it stands and Clang 1, and
    /// each words a bad name its own way. A function with a `target` attribute
    /// that the query guards runs, computed in software.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn cpu_builtins_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "dispatch.c",
            "#include <immintrin.h>\n#include <stdio.h>\n__attribute__((target(\"avx2\"))) static \
             int sum(const int *p) {\n__m256i v = _mm256_loadu_si256((const __m256i \
             *)p);\n__m256i w = _mm256_add_epi32(v, v);\nint o[8]; _mm256_storeu_si256((__m256i \
             *)o, w);\nreturn o[0] + o[7];\n}\nint main(void) { int d[8] = {1,2,3,4,5,6,7,8};\nif \
             (__builtin_cpu_supports(\"avx2\")) printf(\"%d\\n\", sum(d)); else puts(\"none\"); \
             return 0; }\n",
        );
        directory.write(
            "compare.c",
            "#include <stdio.h>\n#define S(n) printf(n \" %d\\n\", \
             __builtin_cpu_supports(n))\n#define I(n) printf(n \" %d\\n\", \
             __builtin_cpu_is(n))\nint main(void) { S(\"cmov\"); S(\"mmx\"); S(\"popcnt\"); \
             S(\"sse4.2\"); S(\"avx\"); S(\"avx2\"); S(\"fma\"); S(\"bmi2\"); S(\"avx512f\"); \
             S(\"avx512vl\"); S(\"avx512vbmi2\"); S(\"gfni\"); S(\"aes\"); S(\"f16c\"); \
             S(\"lzcnt\"); S(\"movbe\"); S(\"rdrnd\"); S(\"sha\"); S(\"xsave\"); S(\"x86-64\"); \
             S(\"x86-64-v2\"); S(\"x86-64-v3\"); S(\"x86-64-v4\"); I(\"intel\"); I(\"amd\"); \
             I(\"corei7\"); I(\"amdfam19h\"); I(\"znver4\"); I(\"haswell\"); return 0; }\n",
        );
        directory.write(
            "bad.c",
            "int f(const char *s) {\nint a = __builtin_cpu_supports(\"bogus\");\nint b = \
             __builtin_cpu_supports(s);\nint c = __builtin_cpu_is(s);\nreturn a + b + c + \
             __builtin_cpu_is(\"bogus\");\n}\n",
        );
        directory.write(
            "unknown.c",
            "int main(void) { return __builtin_cpu_supports(\"bogus\"); }\n",
        );
        directory.write(
            "has.c",
            "#if __has_builtin(__builtin_cpu_supports) && __has_builtin(__builtin_cpu_is) && \
             __has_builtin(__builtin_cpu_init)\nyes\n#else\nno\n#endif\n",
        );
        let sum = if std::is_x86_feature_detected!("avx2") {
            "18\n"
        } else {
            "none\n"
        };
        let clang = concat!(env!("CARGO_MANIFEST_DIR"), "/target/llvm/bin/clang");
        for (mode, compiler) in [("--gcc-mode", "gcc"), ("--clang-mode", clang)] {
            for flags in [
                &["-O0"][..],
                &["-O2"],
                &["-O2", "-static"],
                &["-O0", "-fPIE", "-pie"],
            ] {
                let mut args = flags.to_vec();
                args.extend(["-w", "dispatch.c", "-o", "dispatch"]);
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("dispatch")).output().unwrap();
                assert_eq!(
                    String::from_utf8_lossy(&run.stdout),
                    sum,
                    "{mode} {flags:?}"
                );
            }
            // The feature bits of the table, against the compiler's own code.
            succeeds(&directory.bcc(mode, &["-w", "compare.c", "-o", "ours"]));
            let reference = Command::new(compiler)
                .current_dir(directory.path("."))
                .args(["-w", "compare.c", "-o", "theirs"])
                .output();
            if reference.is_ok_and(|output| output.status.success()) {
                let ours = Command::new(directory.path("ours")).output().unwrap();
                let theirs = Command::new(directory.path("theirs")).output().unwrap();
                assert_eq!(
                    String::from_utf8_lossy(&ours.stdout),
                    String::from_utf8_lossy(&theirs.stdout),
                    "{mode}"
                );
            }
            let output = directory.bcc(mode, &["-E", "has.c"]);
            assert!(stdout(&output).contains("yes"), "{mode}");
            let output = directory.bcc(mode, &["--target=wasm32-unknown-unknown", "-E", "has.c"]);
            assert!(stdout(&output).contains("no"), "{mode}");
            let output = directory.bcc(
                mode,
                &[
                    "--target=wasm32-unknown-unknown",
                    "-S",
                    "unknown.c",
                    "-o",
                    "unknown.s",
                ],
            );
            assert_eq!(output.status.code(), Some(1), "{mode}");
            assert!(
                stderr(&output).contains("error: builtin is not supported on this target"),
                "{}",
                stderr(&output)
            );
        }
        let value = |mode: &str, name: &str, expected: &str| {
            directory.write(
                "value.c",
                &format!(
                    "#include <stdio.h>\nint main(void) {{ printf(\"%d\\n\",                      \
                     __builtin_cpu_supports(\"{name}\")); return 0; }}\n"
                ),
            );
            succeeds(&directory.bcc(mode, &["-w", "value.c", "-o", "value"]));
            let run = Command::new(directory.path("value")).output().unwrap();
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                expected,
                "{mode} {name}"
            );
        };
        value("--gcc-mode", "sse2", "16\n");
        value("--clang-mode", "sse2", "1\n");
        // GCC reads a C string; Clang all of it, which names no feature.
        value("--gcc-mode", "sse2\\0junk", "16\n");
        value("--gcc-mode", "sse2\\0\\377", "16\n");
        value("--clang-mode", "sse2\\0junk", "0\n");
        directory.write(
            "utf8.c",
            "#include <stdio.h>\nint main(void) { printf(\"%d\\n\", \
             __builtin_cpu_supports(u8\"sse2\")); return 0; }\n",
        );
        for (mode, expected) in [("--gcc-mode", "16\n"), ("--clang-mode", "1\n")] {
            succeeds(&directory.bcc(mode, &["-w", "utf8.c", "-o", "utf8"]));
            let run = Command::new(directory.path("utf8")).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{mode}");
        }
        directory.write(
            "empty.c",
            "int main(void) { return __builtin_cpu_supports(\"\\0sse2\"); }\n",
        );
        let output = directory.bcc("--gcc-mode", &["-c", "empty.c", "-o", "empty.o"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("error: parameter to builtin not valid: \n"),
            "{}",
            stderr(&output)
        );
        let output = directory.bcc("--gcc-mode", &["-c", "bad.c", "-o", "bad.o"]);
        assert_eq!(output.status.code(), Some(1));
        let words = stderr(&output);
        assert!(
            words.contains("error: parameter to builtin not valid: bogus"),
            "{words}"
        );
        assert!(
            words.contains("error: parameter to builtin must be a string constant or literal"),
            "{words}"
        );
        let output = directory.bcc("--clang-mode", &["-c", "bad.c", "-o", "bad.o"]);
        assert_eq!(output.status.code(), Some(1));
        let words = stderr(&output);
        assert!(
            words.contains("warning: invalid cpu feature string for builtin"),
            "{words}"
        );
        assert!(
            words.contains("error: invalid cpu name for builtin"),
            "{words}"
        );
        assert!(
            words.contains("error: expression is not a string literal"),
            "{words}"
        );
        // Clang's unknown feature warns and answers 0.
        let output = directory.bcc("--clang-mode", &["unknown.c", "-o", "unknown"]);
        succeeds(&output);
        assert!(stderr(&output).contains("invalid cpu feature string"));
        let run = Command::new(directory.path("unknown")).status().unwrap();
        assert_eq!(run.code(), Some(0));
        // `__builtin_cpu_init` is `int` in GCC and `void` in Clang.
        directory.write(
            "init.c",
            "#include <stdio.h>\nint main(void) { printf(\"%zu\\n\", \
             sizeof(__builtin_cpu_init()));              return 0; }\n",
        );
        for (mode, expected) in [("--gcc-mode", "4\n"), ("--clang-mode", "1\n")] {
            succeeds(&directory.bcc(mode, &["-w", "init.c", "-o", "init"]));
            let run = Command::new(directory.path("init")).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{mode}");
        }
    }

    /// `-masm=intel` writes Intel syntax and reads `asm` statements in it. The
    /// checks name no register, since the argument's differs on Windows.
    #[test]
    fn gnu_intel_syntax_reads_and_writes_assembly() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "int add(int a, int b) {\n    int r;\n    __asm__(\"mov %0, %1\\n\\tadd %0, %2\" : \
             \"=&r\"(r) : \"r\"(a), \"r\"(b));\n    return r;\n}\nint load(int *p) {\n    int \
             v;\n    __asm__(\"mov %0, DWORD PTR [%1]\" : \"=r\"(v) : \"r\"(p));\n    return \
             v;\n}\nint main(void) {\n    int g = 5;\n    return add(2, 3) == 5 && load(&g) == 5 \
             ? 0 : 1;\n}\n",
        );
        directory.write("p.c", "int f(int *p) { return *p; }\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let intel =
                stdout(&directory.bcc(mode, &["-masm=intel", "-O1", "-S", "p.c", "-o", "-"]));
            assert!(intel.contains(".intel_syntax noprefix"), "{mode}: {intel}");
            assert!(intel.contains("dword ptr [r"), "{mode}: {intel}");
            let plain = stdout(&directory.bcc(
                mode,
                &["-masm=intel", "-masm=att", "-O1", "-S", "p.c", "-o", "-"],
            ));
            assert!(plain.contains("(%r"), "{mode}: {plain}");
            // Under GCC, `%z` names no size in Intel syntax.
            directory.write(
                "z.c",
                "int f(short v) {\n    short r;\n    __asm__(\"mov%z0 %0, %1\" : \"=r\"(r) : \
                 \"r\"(v));\n    return r;\n}\n",
            );
            if mode == "--gcc-mode" {
                let args = ["-masm=intel", "-O1", "-S", "z.c", "-o", "-"];
                assert!(stdout(&directory.bcc(mode, &args)).contains("mov\tax, "));
                // Of any operand, which AT&T's `%z` would refuse.
                directory.write(
                    "i.c",
                    "int main(void) {\n    __asm__ volatile(\"nop%z0\" : : \"i\"(3));\n    return \
                     0;\n}\n",
                );
                succeeds(&directory.bcc(mode, &["-masm=intel", "-c", "i.c", "-o", "i.o"]));
                // A naked function's frame, as GCC builds it.
                directory.write(
                    "n.c",
                    "extern int sink(int *);\n__attribute__((naked)) void f(void) {\n    int x = \
                     3;\n    sink(&x);\n    __asm__(\"ret\");\n}\n",
                );
                #[cfg(target_os = "linux")]
                succeeds(&directory.bcc(mode, &["-masm=intel", "-c", "n.c", "-o", "n.o"]));
            }
            // What bcc writes around an `asm` follows too, and so does
            // file-scope `asm`.
            directory.write(
                "g.c",
                "__asm__(\".globl seven\\nseven:\\nmov eax, 7\\nret\");\nextern int \
                 seven(void);\nint main(void) {\n    __asm__ goto(\"jmp %l0\" : : : : hit);\n    \
                 return 1;\nhit:\n    return seven() != 7;\n}\n",
            );
            for optimization in ["-O0", "-O2"] {
                let args = ["-masm=intel", optimization, "g.c", "-o", "g"];
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("g")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {optimization}");
            }
            // A group's alternative follows the syntax in use.
            directory.write(
                "d.c",
                "int main(void) {\n    int a = 3, b, c = 0;\n    __asm__(\"{movl %1, %0|mov %0, \
                 %1}\" : \"=r\"(b) : \"r\"(a));\n    __asm__(\"{addl $2, %0|add %0, 2}\" : \
                 \"+r\"(c));\n    return b + c == 5 ? 0 : 1;\n}\n",
            );
            for options in [&["-masm=intel"][..], &[]] {
                let args = [options, &["-O1", "d.c", "-o", "d"]].concat();
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("d")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {options:?}");
            }
            for optimization in ["-O0", "-O2"] {
                let args = ["-masm=intel", optimization, "a.c", "-o", "a"];
                succeeds(&directory.bcc(mode, &args));
                let run = Command::new(directory.path("a")).status().unwrap();
                assert_eq!(run.code(), Some(0), "{mode} {optimization}");
            }
        }
    }

    /// clang-cl takes Clang's options, which bcc reads or treats as its Clang
    /// personality does, without its own argument parser showing through.
    #[test]
    fn clang_cl_treats_clang_options_as_clang_does() {
        let directory = Directory::new();
        directory.write("a.c", "int add(int a, int b) { return a + b; }\n");
        let run =
            |args: &[&str]| directory.bcc("--clang-cl-mode", &[args, &["/c", "a.c"]].concat());
        for option in ["/GR", "/GR-", "/Zl", "/Y-"] {
            let output = run(&[option]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{option}");
        }
        let output = run(&["-fms-compatibility"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let output = run(&["-fsyntax-only"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        assert!(run(&["-march=native"]).status.success());
        for option in ["-m32", "-fbogus-option"] {
            let text = stderr(&run(&[option]));
            assert!(!text.contains("unexpected argument"), "{option}: {text}");
            assert!(!text.contains("Usage:"), "{option}: {text}");
        }
    }

    /// The options around GCC 14's permerrors and Clang's default errors,
    /// as each compiler orders them: `-Werror` does not reach into a system
    /// header, `-fpermissive` beats `-pedantic-errors`, GCC's `-Wno-X`
    /// keeps an earlier `-Werror=X`, Clang's `-Wno-error=X` keeps a later
    /// `-Wall`, and an enabled `-Wimplicit-function-declaration` reports a
    /// C89 library function GCC otherwise passes.
    #[test]
    fn permerror_options_follow_each_compiler() {
        let directory = Directory::new();
        std::fs::create_dir_all(directory.path("sys")).unwrap();
        directory.write("sys/bad.h", "void f(int *p){long *q=p;(void)q;}\n");
        directory.write("main.c", "#include <bad.h>\nint main(void) { return 0; }\n");
        directory.write("pointer.c", "void f(int *p){long *q=p;(void)q;}\n");
        directory.write("library.c", "int f(void){return abs(-1);}\n");
        directory.write("undeclared.c", "int f(void){return frob();}\n");
        directory.write("implicit.c", "extern x;\n");
        let pointer = "initialization of 'long int *' from incompatible pointer type 'int *' \
                       [-Wincompatible-pointer-types]";
        // (mode, options, file, exit status, expected stderr fragment or "").
        for (mode, options, file, status, message) in [
            (
                "--gcc-mode",
                &["-std=gnu89", "-Werror"][..],
                "main.c",
                0,
                "",
            ),
            (
                "--gcc-mode",
                &["-std=gnu89", "-Werror=incompatible-pointer-types"],
                "main.c",
                0,
                "",
            ),
            (
                "--gcc-mode",
                &["-std=gnu17", "-fpermissive", "-Werror"],
                "main.c",
                0,
                "",
            ),
            (
                "--gcc-mode",
                &["-std=gnu89", "-fno-permissive"],
                "main.c",
                1,
                pointer,
            ),
            (
                "--gcc-mode",
                &["-std=gnu89", "-pedantic-errors"],
                "main.c",
                1,
                pointer,
            ),
            (
                "--gcc-mode",
                &["-std=gnu17", "-fpermissive", "-pedantic-errors"],
                "main.c",
                0,
                "",
            ),
            (
                "--gcc-mode",
                &["-std=gnu17", "-fpermissive", "-pedantic-errors"],
                "pointer.c",
                0,
                pointer,
            ),
            (
                "--gcc-mode",
                &["-std=gnu89", "-Werror=implicit-function-declaration"],
                "library.c",
                1,
                "error: implicit declaration of function 'abs'",
            ),
            (
                "--gcc-mode",
                &["-std=gnu89", "-Wall"],
                "library.c",
                0,
                "warning: implicit declaration of function 'abs' [-Wimplicit-function-declaration]",
            ),
            ("--gcc-mode", &["-std=gnu89"], "library.c", 0, ""),
            (
                "--gcc-mode",
                &[
                    "-std=gnu89",
                    "-Werror=implicit-int",
                    "-Wno-implicit-int",
                    "-Wimplicit-int",
                ],
                "implicit.c",
                1,
                "error: type defaults to 'int' in declaration of 'x'",
            ),
            (
                "--clang-mode",
                &[
                    "-std=gnu17",
                    "-Wno-implicit-function-declaration",
                    "-Wall",
                    "-Wno-error=implicit-function-declaration",
                ],
                "undeclared.c",
                0,
                "warning: call to undeclared function 'frob'",
            ),
            (
                "--clang-mode",
                &[
                    "-std=gnu17",
                    "-Werror=implicit-function-declaration",
                    "-Wno-implicit-function-declaration",
                    "-Wimplicit-function-declaration",
                ],
                "undeclared.c",
                0,
                "warning: call to undeclared function 'frob'",
            ),
            // GCC's C89 implicit int is a pedantic warning once enabled,
            // and GCC's pedantic errors outlast `-w`.
            (
                "--gcc-mode",
                &["-std=c89", "-Wall", "-pedantic-errors"],
                "implicit.c",
                1,
                "error: type defaults to 'int' in declaration of 'x'",
            ),
            (
                "--gcc-mode",
                &["-std=c89", "-Wall", "-fno-permissive"],
                "implicit.c",
                1,
                "error: type defaults to 'int' in declaration of 'x'",
            ),
            (
                "--gcc-mode",
                &["-std=c89", "-w", "-pedantic-errors"],
                "pointer.c",
                1,
                pointer,
            ),
            (
                "--gcc-mode",
                &["-std=c89", "-pedantic-errors", "-w"],
                "pointer.c",
                1,
                pointer,
            ),
            (
                "--gcc-mode",
                &["-std=c89", "-Wall", "-pedantic-errors"],
                "library.c",
                0,
                "warning: implicit declaration of function 'abs'",
            ),
            // Clang's C89 library function is a pedantic warning, which
            // its `-w` silences first; in C23 it stays a demotable error.
            (
                "--clang-mode",
                &["-std=c89", "-pedantic-errors"],
                "library.c",
                1,
                "abs",
            ),
            (
                "--clang-mode",
                &["-std=c89", "-w", "-pedantic-errors"],
                "library.c",
                0,
                "",
            ),
            (
                "--clang-mode",
                &["-std=c23", "-Wno-error=implicit-function-declaration"],
                "library.c",
                0,
                "warning: call to undeclared library function 'abs'",
            ),
            (
                "--clang-mode",
                &["-std=c23", "-Wno-implicit-function-declaration"],
                "library.c",
                0,
                "",
            ),
            ("--clang-mode", &["-std=c23"], "library.c", 1, "abs"),
            ("--clang-mode", &["-std=c23"], "undeclared.c", 1, "frob"),
        ] {
            let mut args = vec!["-isystem", "sys", "-fsyntax-only"];
            args.extend_from_slice(options);
            args.push(file);
            let output = directory.bcc(mode, &args);
            let stderr = stderr(&output);
            assert_eq!(
                output.status.code(),
                Some(status),
                "{mode} {args:?}: {stderr}"
            );
            if message.is_empty() {
                assert_eq!(stderr, "", "{mode} {args:?}");
            } else {
                assert!(stderr.contains(message), "{mode} {args:?}: {stderr}");
            }
        }
    }

    /// TCC knows only its own few `__builtin_` names (TCC mob `tcctok.h`
    /// and `tccdefs.h`): a call to any other is an implicit declaration of
    /// an ordinary function, which the program may define. Its builtins
    /// keep their meaning.
    #[test]
    fn tcc_unknown_builtins_are_ordinary_functions() {
        let directory = Directory::new();
        for (source, name) in [
            (
                "int main(void){return __builtin_finite(1.0)!=7;}\nint __builtin_finite(double \
                 x){return 7;}\n",
                "__builtin_finite",
            ),
            (
                "int main(void){return __builtin_frob(2);}\nint __builtin_frob(int x){return \
                 x-2;}\n",
                "__builtin_frob",
            ),
        ] {
            directory.write("b.c", source);
            let output = directory.bcc("--tcc-mode", &["-c", "b.c", "-o", "b.o"]);
            succeeds(&output);
            assert_eq!(
                stderr(&output),
                format!("b.c:1: warning: implicit declaration of function '{name}'\n")
            );
            if cfg!(all(target_os = "linux", target_env = "gnu")) {
                succeeds(&directory.bcc("--tcc-mode", &["-w", "b.c", "-o", "b"]));
                let run = Command::new(directory.path("b")).output().unwrap();
                assert_eq!(run.status.code(), Some(0), "{source}");
            }
        }
        directory.write(
            "known.c",
            "#include <stdarg.h>\nstruct S { int a, b; };\nstatic int first(int n, ...) { va_list \
             ap; va_start(ap, n); int r = va_arg(ap, int); va_end(ap); return r; }\nint \
             main(void) { return __builtin_expect(first(1, 0), 0) + !__builtin_constant_p(3) + \
             (__builtin_frame_address(0) == 0) + __builtin_popcount(0) + \
             (int)__builtin_offsetof(struct S, a); }\n",
        );
        let output = directory.bcc("--tcc-mode", &["-c", "known.c", "-o", "known.o"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        // `tccdefs.h` binds the library aliases (`__builtin_alloca` to
        // `alloca`) only off Windows.
        directory.write(
            "alloca.c",
            "void *alloca(__SIZE_TYPE__);\nint main(void) { char *p = __builtin_alloca(4); p[0] = \
             0; return p[0]; }\n",
        );
        let output = directory.bcc("--tcc-mode", &["-c", "alloca.c", "-o", "alloca.o"]);
        succeeds(&output);
        if cfg!(windows) {
            assert!(
                stderr(&output)
                    .contains("warning: implicit declaration of function '__builtin_alloca'"),
                "{}",
                stderr(&output)
            );
        } else {
            assert_eq!(stderr(&output), "");
        }
        // Other identities keep the GCC builtin.
        directory.write("gcc.c", "int main(void){return !__builtin_finite(1.0);}\n");
        let output = directory.bcc("--gcc-mode", &["-c", "gcc.c", "-o", "gcc.o"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
    }

    /// An empty translation unit, such as a source whose text is all
    /// conditioned out, compiles in every personality, as in GCC, Clang
    /// and TCC; `-pedantic` reports it in each compiler's words, and
    /// `-pedantic-errors` refuses it.
    #[test]
    fn empty_translation_units_compile_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("empty.c", "#ifdef NEVER\nint x;\n#endif\n");
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let output = directory.bcc(mode, &["-c", "empty.c", "-o", "empty.o"]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
        }
        for (mode, message) in [
            (
                "--gcc-mode",
                "ISO C forbids an empty translation unit [-Wpedantic]",
            ),
            (
                "--clang-mode",
                "ISO C requires a translation unit to contain at least one declaration \
                 [-Wempty-translation-unit]",
            ),
        ] {
            let warned = directory.bcc(mode, &["-pedantic", "-c", "empty.c", "-o", "empty.o"]);
            succeeds(&warned);
            assert!(
                stderr(&warned).contains(&format!("warning: {message}")),
                "{mode}: {}",
                stderr(&warned)
            );
            let refused = directory.bcc(
                mode,
                &["-pedantic-errors", "-c", "empty.c", "-o", "empty.o"],
            );
            assert!(!refused.status.success(), "{mode}");
            assert!(
                stderr(&refused).contains("error: "),
                "{mode}: {}",
                stderr(&refused)
            );
        }
        // A pragma the compiler's parser sees is content, as in each
        // compiler: `pack` in both, `GCC diagnostic` only in GCC's.
        for (source, gcc, clang) in [
            ("#pragma pack(1)\n", true, true),
            ("#pragma GCC diagnostic push\n", true, false),
            ("#pragma GCC system_header\n", false, false),
            ("#pragma STDC FP_CONTRACT ON\n", false, true),
            ("#pragma STDC FOO\n", false, false),
            ("#pragma clang fp contract(fast)\n", false, true),
        ] {
            directory.write("pragma.c", source);
            for (mode, content) in [("--gcc-mode", gcc), ("--clang-mode", clang)] {
                let output = directory.bcc(
                    mode,
                    &["-pedantic-errors", "-c", "pragma.c", "-o", "pragma.o"],
                );
                assert_eq!(
                    output.status.success(),
                    content,
                    "{mode} {source:?}: {}",
                    stderr(&output)
                );
            }
        }
    }

    /// GCC's `__builtin_finite` family is `isfinite` of one fixed type;
    /// Clang has none of it.
    #[test]
    fn finite_builtins_follow_gcc() {
        let directory = Directory::new();
        directory.write(
            "f.c",
            "volatile double inf = __builtin_inf(), qnan = __builtin_nan(\"\");\nvolatile long \
             double tiny = 1e-4940L;\nint main(void) {\nreturn __builtin_finite(1.0) && \
             !__builtin_finite(inf) && !__builtin_finitef((float)qnan) && __builtin_finitel(tiny) \
             && __builtin_finitef(1e300) == 0 ? 0 : 1;\n}\n",
        );
        directory.write("h.c", "int x = __has_builtin(__builtin_finite);\n");
        let gcc = directory.bcc("--gcc-mode", &["-E", "-P", "h.c"]);
        assert!(
            stdout(&gcc).replace(' ', "").contains("intx=1;"),
            "{}",
            stdout(&gcc)
        );
        let clang = directory.bcc("--clang-mode", &["-E", "-P", "h.c"]);
        assert!(
            stdout(&clang).replace(' ', "").contains("intx=0;"),
            "{}",
            stdout(&clang)
        );
        let refused = directory.bcc("--clang-mode", &["-fsyntax-only", "f.c"]);
        assert!(!refused.status.success(), "{}", stderr(&refused));
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            for options in [
                &["-O0"][..],
                &["-O2"],
                &["-O2", "-Xbcc", "--backend=native"],
            ] {
                let mut args = vec!["f.c", "-o", "f"];
                args.extend(options);
                succeeds(&directory.bcc("--gcc-mode", &args));
                let run = Command::new(directory.path("f")).output().unwrap();
                assert_eq!(run.status.code(), Some(0), "{options:?}");
            }
        }
    }

    /// A sign after `p` continues a preprocessing number from C99. Before
    /// C99, GCC reads it so only in its GNU modes and Clang only after a
    /// hexadecimal prefix, so strict C90 splits `0x1p+f` (GCC's
    /// gcc.dg/c90-hexfloat-2.c).
    #[test]
    fn hexadecimal_float_signs_split_numbers_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write(
            "numbers.c",
            "#define f (\n#define l )\n0x1p+f 0x1p+l 1p+f 1p+l 1e+f\n",
        );
        // Where the sign ends the number, `f` and `l` are replaced; the
        // spacing between the tokens is not compared. The numbers are not
        // valid constants, which `-E` leaves to the compilation, as GCC's
        // and Clang's do.
        let expand = |mode: &str, standard: &str| {
            let output = directory.bcc(mode, &[standard, "-E", "-P", "numbers.c"]);
            succeeds(&output);
            stdout(&output).split_whitespace().collect::<String>()
        };
        let joined = "0x1p+f0x1p+l1p+f1p+l1e+f";
        for mode in ["--gcc-mode", "--clang-mode"] {
            assert_eq!(expand(mode, "-std=c99"), joined, "{mode}");
        }
        assert_eq!(expand("--gcc-mode", "-std=gnu89"), joined);
        assert_eq!(expand("--gcc-mode", "-std=c90"), "0x1p+(0x1p+)1p+(1p+)1e+f");
        for standard in ["-std=c90", "-std=gnu89"] {
            assert_eq!(
                expand("--clang-mode", standard),
                "0x1p+f0x1p+l1p+(1p+)1e+f",
                "{standard}"
            );
        }
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
        // GCC spells `__SCHAR_MAX__` in hex.
        let signed = |mode: &str| {
            // GCC's own listing exists for the Linux targets only.
            let max = if mode == "--gcc-mode" && cfg!(target_os = "linux") {
                "0x7f"
            } else {
                "127"
            };
            format!("(-{max} - 1) (char)-1 < 0 __CHAR_UNSIGNED__ _CHAR_UNSIGNED")
        };
        let unsigned = "0 (char)-1 < 0 1 _CHAR_UNSIGNED";
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            assert_eq!(expand(mode, &[]), signed(mode), "{mode}");
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
                assert_eq!(expand(mode, options), signed(mode), "{mode} {options:?}");
            }
        }
        // `cl /J` defines `_CHAR_UNSIGNED`, as clang-cl's does; bcc keeps
        // Clang's `__CHAR_UNSIGNED__` beside it, as clang-cl does.
        let signed_cl = "(-127 - 1) (char)-1 < 0 __CHAR_UNSIGNED__ _CHAR_UNSIGNED";
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

    /// The floating-point macros of the GCC identity are the host GCC's
    /// under each floating-point option.
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
                        "__BFLT16",
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
                        "__SUPPORT_SNAN__",
                        "__FAST_MATH__",
                        "__FINITE_MATH_ONLY__",
                    ]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                        || name.contains("_IS_IEC_60559__")
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

    /// The GCC identity predefines exactly the host GCC's macros, spelled as
    /// GCC spells them, in each language mode: GCC 14.2's frozen target
    /// listing, the macros that follow the options, and what glibc's
    /// `<stdc-predef.h>`, which GCC reads first, defines. Left out are the
    /// GCC version, which bcc claims as 14.2, the frozen listing's
    /// documented exclusions that bcc does not define, Ubuntu's driver
    /// defaults (`-fstack-protector-strong`, `-fcf-protection`), and bcc's
    /// own and protected macros.
    #[test]
    fn gcc_mode_predefines_the_host_gccs_macros() {
        if host_gcc_macros(&[]).is_none() || !Path::new("/usr/include/stdc-predef.h").is_file() {
            eprintln!("skipping: no x86-64 Linux GCC 13 or later with glibc on the host");
            return;
        }
        let excluded: Vec<&str> = include_str!("../../src/target/gcc/macro-exclusions.tsv")
            .lines()
            .map(|line| line.split_once('\t').unwrap().0)
            .collect();
        let name = |line: &str| -> String {
            line.strip_prefix("#define ")
                .unwrap_or(line)
                .split([' ', '('])
                .next()
                .unwrap()
                .to_owned()
        };
        for standard in [
            "-std=c99",
            "-std=gnu89",
            "-std=c11",
            "-std=gnu17",
            "-std=c2x",
            "-O2",
        ] {
            let bcc = macros_with("--gcc-mode", &[standard]);
            let host = host_gcc_macros(&[standard]).unwrap();
            let bcc_names: std::collections::BTreeSet<String> = bcc.lines().map(name).collect();
            let compared = |listing: &str| -> std::collections::BTreeSet<String> {
                listing
                    .lines()
                    .filter(|line| {
                        let name = name(line);
                        let ignored = name.starts_with("__GNUC")
                            || name.starts_with("__bcc")
                            || name.starts_with("__STDC_EMBED_")
                            || matches!(
                                name.as_str(),
                                "__VERSION__"
                                    | "__GXX_ABI_VERSION"
                                    | "__BITINT_MAXWIDTH__"
                                    | "__SSP_STRONG__"
                                    | "__CET__"
                                    | "__GCC_HAVE_DWARF2_CFI_ASM"
                                    | "__pic__"
                                    | "__PIC__"
                                    | "__pie__"
                                    | "__PIE__"
                                    | "_FORTIFY_SOURCE"
                                    | "__STDC_MB_MIGHT_NEQ_WC__"
                            );
                        // An exclusion counts only where bcc defines it.
                        !ignored
                            && (!excluded.contains(&name.as_str()) || bcc_names.contains(&name))
                    })
                    .map(str::to_owned)
                    .collect()
            };
            let (bcc, host) = (compared(&bcc), compared(&host));
            assert_eq!(
                bcc.difference(&host).collect::<Vec<_>>(),
                Vec::<&String>::new(),
                "{standard}: bcc only"
            );
            assert_eq!(
                host.difference(&bcc).collect::<Vec<_>>(),
                Vec::<&String>::new(),
                "{standard}: GCC only"
            );
        }
    }

    /// Where the GCC identity reads Clang's target macros, Clang's lock-free
    /// macros take GCC's spelling, which `<stdatomic.h>` then reads, even
    /// on the MSVC target, whose Clang macros have no GCC spelling.
    #[test]
    fn gcc_mode_spells_clangs_atomic_macros_as_gccs() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "#include <stdatomic.h>\n_Static_assert(ATOMIC_INT_LOCK_FREE == 2, \
             \"atomic\");\n_Static_assert(ATOMIC_CHAR8_T_LOCK_FREE == 2, \"char8\");\n",
        );
        for target in [
            "x86_64-pc-windows-msvc",
            "x86_64-w64-windows-gnu",
            "wasm32-wasip1",
            "x86_64-unknown-linux-gnu",
        ] {
            let target = format!("--target={target}");
            let output = directory.bcc(
                "--gcc-mode",
                &[
                    &target,
                    "-std=c23",
                    "-ffreestanding",
                    "-fsyntax-only",
                    "a.c",
                ],
            );
            succeeds(&output);
            let listing = macros_with("--gcc-mode", &[&target, "-std=c23"]);
            assert_eq!(
                defines(&listing, "__GCC_ATOMIC_INT_LOCK_FREE").as_deref(),
                Some("2"),
                "{target}"
            );
            assert!(!listing.contains("__CLANG_ATOMIC"), "{target}");
        }
    }

    /// `-H` leaves out the `-include` files and the headers they include,
    /// as GCC and Clang do, while `/showIncludes` lists the `/FI` ones, as
    /// clang-cl does.
    #[test]
    fn header_trees_leave_out_forced_includes() {
        let directory = Directory::new();
        directory.write("forced.h", "#include \"sub.h\"\n");
        directory.write("sub.h", "int s;\n");
        directory.write("main.h", "int mh;\n");
        directory.write("m.c", "#include \"main.h\"\nint m;\n");
        let tree = |output: &Output| -> Vec<String> {
            stderr(output)
                .lines()
                .filter(|line| line.starts_with(". ") || line.starts_with("Note: including"))
                .map(str::to_owned)
                .collect()
        };
        // Clang names a header beside the main file `./`, `.\\` escaped on
        // Windows, and clang-cl's `/showIncludes` `.\` unescaped.
        let (here, cl_here) = if cfg!(windows) {
            (r".\\", r".\")
        } else {
            ("./", "./")
        };
        let clang_main = format!(". {here}main.h");
        for (mode, expected) in [
            ("--gcc-mode", ". main.h"),
            ("--clang-mode", clang_main.as_str()),
        ] {
            let output = directory.bcc(
                mode,
                &["-H", "-fsyntax-only", "-include", "forced.h", "m.c"],
            );
            succeeds(&output);
            assert_eq!(tree(&output), [expected], "{mode}: {}", stderr(&output));
        }
        // A guarded header forced and then included by the main file: GCC
        // lists the main file's include, as its only listed one; Clang
        // lists neither.
        directory.write(
            "guarded.h",
            "#ifndef GUARDED_H\n#define GUARDED_H\n#include \"sub.h\"\n#endif\n",
        );
        directory.write("g.c", "#include \"guarded.h\"\nint g;\n");
        for (mode, expected) in [("--gcc-mode", &[". guarded.h"][..]), ("--clang-mode", &[])] {
            let output = directory.bcc(
                mode,
                &["-H", "-fsyntax-only", "-include", "guarded.h", "g.c"],
            );
            succeeds(&output);
            assert_eq!(tree(&output), expected, "{mode}: {}", stderr(&output));
        }
        let output = directory.bcc(
            "--clang-cl-mode",
            &["/nologo", "/Zs", "/showIncludes", "/FIforced.h", "m.c"],
        );
        succeeds(&output);
        assert_eq!(
            tree(&output),
            [
                format!("Note: including file: {cl_here}forced.h"),
                format!("Note: including file:  {cl_here}sub.h"),
                format!("Note: including file: {cl_here}main.h")
            ],
            "{}",
            stderr(&output)
        );
    }

    /// `-H` leaves out GCC's default header and every header it includes,
    /// as GCC does, and lists the main file's headers after it.
    #[test]
    fn gcc_mode_header_tree_leaves_out_the_default_header() {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            return;
        }
        let directory = Directory::new();
        std::fs::create_dir(directory.path("inc")).unwrap();
        directory.write("inc/stdc-predef.h", "#include \"sub.h\"\n");
        directory.write("inc/sub.h", "int sub;\n");
        directory.write("own.h", "#include \"inc/sub.h\"\n");
        directory.write("m.c", "#include \"own.h\"\nint m;\n");
        let output = directory.bcc("--gcc-mode", &["-H", "-fsyntax-only", "-Iinc", "m.c"]);
        succeeds(&output);
        let tree: Vec<String> = stderr(&output)
            .lines()
            .filter(|line| line.starts_with('.'))
            .map(str::to_owned)
            .collect();
        assert_eq!(tree, [". own.h", ".. inc/sub.h"], "{}", stderr(&output));
    }

    /// The GCC identity defines GCC's evaluation method, its spellings of
    /// the `signed char` and `long long` widths, and from C11 the UTF
    /// literal macros, as the host GCC does where there is one.
    #[test]
    fn gcc_mode_defines_gccs_environment_macros() {
        const NAMES: [&str; 5] = [
            "__FLT_EVAL_METHOD__",
            "__SCHAR_WIDTH__",
            "__LONG_LONG_WIDTH__",
            "__STDC_UTF_16__",
            "__STDC_UTF_32__",
        ];
        for (standard, utf) in [
            ("-std=gnu89", None),
            ("-std=c99", None),
            ("-std=gnu99", Some("1")),
            ("-std=gnu11", Some("1")),
        ] {
            let listing = macros_with("--gcc-mode", &[standard]);
            let expected = [Some("0"), Some("8"), Some("64"), utf, utf];
            for (name, value) in NAMES.iter().zip(expected) {
                assert_eq!(
                    defines(&listing, name).as_deref(),
                    value,
                    "{standard} {name}"
                );
            }
            if let Some(host) = host_gcc_macros(&[standard]) {
                for name in NAMES {
                    assert_eq!(
                        defines(&listing, name),
                        defines(&host, name),
                        "{standard} {name}"
                    );
                }
            }
        }
        let clang = macros_with("--clang-mode", &[]);
        assert_eq!(defines(&clang, "__LONG_LONG_WIDTH__"), None);
        // GCC takes the literals the macros describe from gnu99 on; Clang
        // from C11 only.
        let directory = Directory::new();
        directory.write(
            "u.c",
            "const void *p = u\"x\", *q = U\"y\", *r = u8\"z\";\n",
        );
        for (mode, standard, accepted) in [
            ("--gcc-mode", "-std=gnu99", true),
            ("--gcc-mode", "-std=c99", false),
            ("--gcc-mode", "-std=gnu89", false),
            ("--clang-mode", "-std=gnu99", false),
            ("--clang-mode", "-std=c11", true),
        ] {
            let output = directory.bcc(mode, &["-fsyntax-only", standard, "u.c"]);
            assert_eq!(
                output.status.success(),
                accepted,
                "{mode} {standard}: {}",
                stderr(&output)
            );
        }
    }

    /// GCC's options that give up part of IEC 60559 semantics, alone and
    /// in the combinations whose order or interplay decides the macros.
    const FLOATING_POINT_OPTIONS: [&[&str]; 57] = [
        &[],
        &["-fsignaling-nans"],
        &["-fsignaling-nans", "-fno-signaling-nans"],
        // Signaling NaNs keep trapping math, in either order.
        &["-fsignaling-nans", "-fno-trapping-math"],
        &["-fno-trapping-math", "-fsignaling-nans"],
        &[
            "-fsignaling-nans",
            "-fno-signed-zeros",
            "-fno-trapping-math",
            "-fassociative-math",
        ],
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
        // `-ffast-math`, `-Ofast` and the options they imply, which the
        // last option given decides, but for the complex arithmetic, which
        // an explicit `-f[no-]cx-limited-range` keeps.
        &["-ffast-math"],
        &["-Ofast"],
        &["-ffinite-math-only"],
        &["-funsafe-math-optimizations"],
        &["-ffast-math", "-fno-finite-math-only"],
        &["-fno-finite-math-only", "-ffast-math"],
        &["-ffast-math", "-fmath-errno"],
        &["-ffast-math", "-fno-fast-math"],
        &["-ffast-math", "-fsigned-zeros"],
        &["-ffast-math", "-fsignaling-nans"],
        &["-fsignaling-nans", "-ffast-math"],
        &["-ffast-math", "-fexcess-precision=standard"],
        &["-fexcess-precision=standard", "-ffast-math"],
        &["-fsignaling-nans", "-ffast-math", "-fno-fast-math"],
        // The options between a cancelled `-ffast-math` and its negation
        // stand, and every `-ffast-math` before it is cancelled.
        &[
            "-fsignaling-nans",
            "-ffast-math",
            "-fexcess-precision=standard",
            "-fno-fast-math",
        ],
        &[
            "-fsignaling-nans",
            "-ffast-math",
            "-ffast-math",
            "-fno-fast-math",
        ],
        &[
            "-ffast-math",
            "-fsignaling-nans",
            "-ffast-math",
            "-fno-fast-math",
        ],
        &["-ffast-math", "-fno-unsafe-math-optimizations"],
        &["-fcx-limited-range", "-ffast-math", "-fno-fast-math"],
        &["-fno-finite-math-only", "-Ofast"],
        &["-Ofast", "-O2"],
        // GCC's `-Ofast` limits complex arithmetic as a level of its own.
        &["-Ofast", "-fno-fast-math"],
        &["-Ofast", "-fno-fast-math", "-fno-cx-limited-range"],
        &["-Ofast", "-funsafe-math-optimizations", "-fno-fast-math"],
        &["-Ofast", "-ffast-math", "-fno-fast-math"],
        &["-fsigned-zeros", "-funsafe-math-optimizations"],
        &[
            "-funsafe-math-optimizations",
            "-ffinite-math-only",
            "-fno-math-errno",
        ],
        &[
            "-fassociative-math",
            "-freciprocal-math",
            "-fno-signed-zeros",
            "-fno-trapping-math",
            "-ffinite-math-only",
            "-fno-math-errno",
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
            (&["-fsignaling-nans"], "2", "2", &["__SUPPORT_SNAN__"]),
            (
                &["-fno-trapping-math", "-fsignaling-nans"],
                "2",
                "2",
                &["__SUPPORT_SNAN__"],
            ),
            (&["-fsignaling-nans", "-fno-signaling-nans"], "2", "2", &[]),
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
                "__SUPPORT_SNAN__",
            ] {
                assert_eq!(
                    defines(&listing, name).as_deref(),
                    flags.contains(&name).then_some("1"),
                    "{options:?} {name}"
                );
            }
        }
        // Clang defines none of these (its `__NO_MATH_ERRNO__` follows
        // `-fno-math-errno`, as
        // `clang_mode_fast_math_macros_follow_the_options` checks).
        let clang = macros_with("--clang-mode", &["-fno-signed-zeros", "-fsignaling-nans"]);
        assert_eq!(defines(&clang, "__GCC_IEC_559"), None);
        assert_eq!(defines(&clang, "__NO_SIGNED_ZEROS__"), None);
        assert_eq!(defines(&clang, "__SUPPORT_SNAN__"), None);
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
        // `cl` alone prints its banner and, to stdout, its usage; with an
        // option and no file it stops with error D8003, as clang-cl stops
        // with its driver's error.
        let cl = directory.bcc("--msvc-mode", &[]);
        succeeds(&cl);
        assert!(
            stderr(&cl).contains("Version 19.33.0 for x64"),
            "{}",
            stderr(&cl)
        );
        assert!(stderr(&cl).contains("bcc"), "{}", stderr(&cl));
        assert_eq!(
            stdout(&cl),
            "usage: cl [ option... ] filename... [ /link linkoption... ]\n"
        );
        let cl = directory.bcc("--msvc-mode", &["/nologo"]);
        assert_eq!(cl.status.code(), Some(2));
        assert_eq!(
            stderr(&cl),
            "cl : Command line error D8003 : missing source filename\n"
        );
        for arguments in [&[][..], &["/nologo"]] {
            let clang_cl = directory.bcc("--clang-cl-mode", arguments);
            assert_eq!(clang_cl.status.code(), Some(1));
            assert_eq!(stderr(&clang_cl), "clang-cl: error: no input files\n");
        }
        // Queries need no input: clang-cl prints its version for
        // `--version` (stdout) and `-v` or `-###` (stderr), as build systems
        // probe it, and both compilers print help for `/HELP`.
        for (arguments, stream) in [
            (&["--version"][..], "stdout"),
            (&["/nologo", "--version"], "stdout"),
            (&["-v"], "stderr"),
            (&["-###"], "stderr"),
        ] {
            let clang_cl = directory.bcc("--clang-cl-mode", arguments);
            succeeds(&clang_cl);
            let text = if stream == "stdout" {
                stdout(&clang_cl)
            } else {
                stderr(&clang_cl)
            };
            assert!(
                text.starts_with("clang version 23.1.1 (bcc ")
                    && text.contains("\nTarget: x86_64-pc-windows-msvc\n"),
                "{arguments:?}: {text}"
            );
        }
        let triple = directory.bcc("--clang-cl-mode", &["-print-target-triple"]);
        succeeds(&triple);
        assert_eq!(stdout(&triple), "x86_64-pc-windows-msvc\n");
        let search = directory.bcc("--clang-cl-mode", &["-print-search-dirs"]);
        succeeds(&search);
        assert!(
            stdout(&search).starts_with("programs: =")
                && stdout(&search).contains("\nlibraries: ="),
            "{}",
            stdout(&search)
        );
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            let help = directory.bcc(mode, &["/nologo", "/HELP"]);
            succeeds(&help);
            assert!(
                stdout(&help).starts_with("usage: "),
                "{mode}: {}",
                stdout(&help)
            );
        }
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
        // Whether an argument is an option is decided before bcc reads it,
        // so no option bcc matches by a prefix reads a path.
        let analyze = absolute("analyzeme.c");
        assert!(!directory.path("analyzeme.c").exists());
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", &analyze, &plain]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert_eq!(
            stderr(&clang_cl),
            format!("clang-cl: error: no such file or directory: '{analyze}'\n")
        );
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", &analyze, &plain]);
        assert_eq!(cl.status.code(), Some(2));
        assert!(stderr(&cl).contains("fatal error C1083"), "{}", stderr(&cl));
        // `cl`'s own options that clang-cl does not know stay options there.
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "/Zf", "/analyze", &plain]);
        succeeds(&cl);
        assert_eq!(stderr(&cl), "");
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", "/Zf", &plain]);
        assert_eq!(
            stderr(&clang_cl),
            "clang-cl: error: no such file or directory: '/Zf'\n"
        );
        // A bare option clang-cl reads with the next argument takes it, also
        // where bcc ignores the option, but `/Fo` takes its value joined.
        let pch = absolute("missing/x.pch");
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            let output = directory.bcc(mode, &["/nologo", "/Zs", "/Fp:", &pch, &plain]);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
        }
        let object = absolute("missing/x.obj");
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", &plain, "/Fo", &object]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert_eq!(
            stderr(&clang_cl),
            format!("clang-cl: error: no such file or directory: '{object}'\n")
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
            // `cl` writes the column only under `/diagnostics:column`.
            (
                "--msvc-mode",
                &["/nologo", "/c", "bad.c"][..],
                "bad.c(2): error: ",
                false,
            ),
            (
                "--msvc-mode",
                &["/nologo", "/c", "/diagnostics:column", "bad.c"][..],
                "bad.c(2,11): error: ",
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
        // `cl` says it generates code after compiling several sources.
        assert_eq!(stdout(&output), "one.c\ntwo.c\nGenerating Code...\n");
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

    /// Under the `cl` identity the diagnostics `cl`'s users meet most take
    /// its code and words, without bcc's notes, and its warning levels:
    /// C4013 is a level 3 warning, which `cl`'s default `/W1` hides, and
    /// `/wd`, `/we` and `/wN` name a warning by its number. `cl` only warns
    /// about a conversion between a pointer and an integer or between
    /// incompatible pointers, so it succeeds unless `/WX` is given.
    /// clang-cl keeps Clang's rules and bcc's words.
    #[test]
    fn cl_diagnostics_take_cls_codes_words_and_levels() {
        let directory = Directory::new();
        directory.write("bad.c", "int f(void) {\n    return missing;\n}\n");
        directory.write(
            "syntax.c",
            "int f(void) {\n    int x = 1\n    int y = 2;\n    x = 3\n    y = x;\n    return \
             x\n}\n",
        );
        directory.write("implicit.c", "int main(void) { return g(1); }\n");
        directory.write(
            "pointers.c",
            "int main(void) {\n    int *p = 5;\n    float *q = 0;\n    char **r = 0;\n    p = \
             q;\n    p = r;\n    return 0;\n}\n",
        );
        directory.write(
            "casts.c",
            "int f(int *p) { return (int)p; }\nint *g(int i) { return (int *)i; }\n",
        );
        let cl = |args: &[&str]| {
            let mut all = vec!["/nologo", "/c"];
            all.extend(args);
            directory.bcc("--msvc-mode", &all)
        };
        for (args, status, lines) in [
            (
                &["bad.c"][..],
                2,
                &["bad.c(2): error C2065: 'missing': undeclared identifier"][..],
            ),
            (
                &["/diagnostics:column", "bad.c"],
                2,
                &["bad.c(2,12): error C2065: 'missing': undeclared identifier"],
            ),
            (
                &["syntax.c"],
                2,
                &[
                    "syntax.c(3): error C2143: syntax error: missing ';' before 'type'",
                    "syntax.c(5): error C2146: syntax error: missing ';' before identifier 'y'",
                    "syntax.c(7): error C2143: syntax error: missing ';' before '}'",
                ],
            ),
            (&["implicit.c"], 0, &[]),
            (&["/WX", "implicit.c"], 0, &[]),
            (
                &["/W3", "implicit.c"],
                0,
                &["implicit.c(1): warning C4013: 'g' undefined; assuming extern returning int"],
            ),
            (
                &["/w14013", "implicit.c"],
                0,
                &["implicit.c(1): warning C4013: 'g' undefined; assuming extern returning int"],
            ),
            (&["/W4", "/wd4013", "implicit.c"], 0, &[]),
            (
                &["/we4013", "implicit.c"],
                2,
                &["implicit.c(1): error C4013: 'g' undefined; assuming extern returning int"],
            ),
            (
                &["pointers.c"],
                0,
                &[
                    "pointers.c(2): warning C4047: 'initializing': 'int *' differs in levels of \
                     indirection from 'int'",
                    "pointers.c(5): warning C4133: '=': incompatible types - from 'float *' to \
                     'int *'",
                    "pointers.c(6): warning C4047: '=': 'int *' differs in levels of indirection \
                     from 'char **'",
                ],
            ),
            (
                &["/wd4047", "/WX", "pointers.c"],
                2,
                &[
                    "pointers.c(5): error C4133: '=': incompatible types - from 'float *' to 'int \
                     *'",
                ],
            ),
            (&["/W0", "pointers.c"], 0, &[]),
            // The rightmost level wins.
            (
                &["/W0", "/W3", "implicit.c"],
                0,
                &["implicit.c(1): warning C4013: 'g' undefined; assuming extern returning int"],
            ),
            (&["/W3", "/W0", "implicit.c"], 0, &[]),
            (&["/W4", "/W1", "implicit.c"], 0, &[]),
            (
                &["casts.c"],
                0,
                &[
                    "casts.c(1): warning C4311: 'type cast': pointer truncation from 'int *' to \
                     'int'",
                    "casts.c(2): warning C4312: 'type cast': conversion from 'int' to 'int *' of \
                     greater size",
                ],
            ),
        ] {
            let output = cl(args);
            assert_eq!(
                output.status.code(),
                Some(status),
                "{args:?}: {}",
                stderr(&output)
            );
            let printed: Vec<String> = stderr(&output).lines().map(str::to_owned).collect();
            assert_eq!(printed, lines, "{args:?}");
        }
        assert_eq!(
            cl(&["/W0", "/W3", "/WX", "implicit.c"]).status.code(),
            Some(2)
        );
        // `cl` reports function pointers (C4113) and lost qualifiers (C4090)
        // apart from C4133, in bcc's words here: C4133's settings leave
        // them, and their own control them.
        directory.write(
            "apart.c",
            "int f(int x) { return x; }\nint (*p)(double) = f;\nconst char *c;\nchar *d(void) { \
             return c; }\n",
        );
        let warnings = |args: &[&str]| {
            let mut all = args.to_vec();
            all.push("apart.c");
            let output = cl(&all);
            (
                output.status.code(),
                stderr(&output).matches("): warning:").count(),
            )
        };
        assert_eq!(warnings(&[]), (Some(0), 2));
        assert_eq!(warnings(&["/wd4133"]), (Some(0), 2));
        assert_eq!(warnings(&["/w34133"]), (Some(0), 2));
        assert_eq!(warnings(&["/we4133"]), (Some(0), 2));
        assert_eq!(warnings(&["/wd4113"]), (Some(0), 1));
        assert_eq!(warnings(&["/wd4090"]), (Some(0), 1));
        assert_eq!(warnings(&["/wd4113", "/wd4090"]), (Some(0), 0));
        assert_eq!(warnings(&["/W0"]), (Some(0), 0));
        assert_eq!(warnings(&["/we4113"]).0, Some(2));
        assert_eq!(warnings(&["/wd4133", "/WX"]).0, Some(2));
        assert_eq!(warnings(&["/wd4113", "/wd4090", "/WX"]).0, Some(0));
        // clang-cl writes Clang's errors in bcc's words.
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", "pointers.c"]);
        assert_eq!(clang_cl.status.code(), Some(1));
        let text = stderr(&clang_cl);
        assert!(!text.contains(" C40"), "{text}");
        assert!(text.contains("pointers.c(2,"), "{text}");
        assert!(text.contains("): error: "), "{text}");
    }

    /// `cl` reads its options from command files (`@file`, UTF-8 or UTF-16
    /// with a byte order mark, as `MSBuild` writes them) and from the `CL`
    /// and `_CL_` environment variables, before and after the command
    /// line. The rest of a command file's line after `/link` is the
    /// linker's; the next line is the compiler's again. A command file that
    /// cannot be opened is error D8022, and one cannot name another, which
    /// clang-cl allows.
    #[test]
    fn cl_reads_command_files_and_environment_variables() {
        let directory = Directory::new();
        directory.write("x.c", "int x = X;\n");
        directory.write(
            "options.rsp",
            "/nologo /c\n\"x.c\" /link /SUBSYSTEM:CONSOLE\n/DX#2 /Fofile.obj\n",
        );
        let output = directory.bcc("--msvc-mode", &["@options.rsp"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        assert!(directory.path("file.obj").is_file());
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "/nologo /c /DX=3 x.c /Fowide.obj\r\n".encode_utf16() {
            utf16.extend(unit.to_le_bytes());
        }
        std::fs::write(directory.path("wide.rsp"), utf16).unwrap();
        succeeds(&directory.bcc("--msvc-mode", &["@wide.rsp"]));
        assert!(directory.path("wide.obj").is_file());
        let missing = directory.bcc("--msvc-mode", &["/nologo", "/c", "@missing.rsp", "x.c"]);
        assert_eq!(missing.status.code(), Some(2));
        assert_eq!(
            stderr(&missing),
            "cl : Command line error D8022 : cannot open 'missing.rsp'\n"
        );
        // `CL` comes before the command line and `_CL_` after it, so the
        // latter's `/Fo` wins.
        let environment = directory.bcc_with(
            "--msvc-mode",
            &["/Fofirst.obj", "x.c"],
            &[("CL", "/nologo /c /DX#4"), ("_CL_", "/Foenvironment.obj")],
        );
        succeeds(&environment);
        assert!(directory.path("environment.obj").is_file());
        assert!(!directory.path("first.obj").exists());
        directory.write("nested.rsp", "@options.rsp\n");
        let nested = directory.bcc(
            "--msvc-mode",
            &[
                "/nologo",
                "/c",
                "/DX=5",
                "@nested.rsp",
                "x.c",
                "/Fonested.obj",
            ],
        );
        succeeds(&nested);
        assert!(
            stderr(&nested).contains("unrecognized source file type '@options.rsp'"),
            "{}",
            stderr(&nested)
        );
        directory.write("inner.rsp", "/c x.c /DX=6\n");
        directory.write("clang.rsp", "@inner.rsp /Foclang.obj\n");
        succeeds(&directory.bcc("--clang-cl-mode", &["@clang.rsp"]));
        // clang-cl refuses a file that names itself, directly or through
        // others, however it branches, in LLVM's words; naming one file
        // twice is no cycle.
        directory.write("self.rsp", "@self.rsp\n");
        directory.write("fork.rsp", "@fork.rsp @fork.rsp\n");
        directory.write("one.rsp", "@two.rsp\n");
        directory.write("two.rsp", "@one.rsp\n");
        for (file, named) in [
            ("@self.rsp", "self.rsp"),
            ("@fork.rsp", "fork.rsp"),
            ("@one.rsp", "one.rsp"),
        ] {
            let cycle = directory.bcc("--clang-cl-mode", &["/c", file]);
            assert_eq!(cycle.status.code(), Some(1), "{file}");
            let text = stderr(&cycle);
            assert!(text.starts_with("recursive expansion of: '"), "{text}");
            assert!(text.trim_end().ends_with(&format!("{named}'")), "{text}");
        }
        directory.write("empty.rsp", "\n");
        directory.write(
            "twice.rsp",
            "@empty.rsp @empty.rsp /c x.c /DX=6 /Fotwice.obj\n",
        );
        succeeds(&directory.bcc("--clang-cl-mode", &["@twice.rsp"]));
        assert!(directory.path("clang.obj").is_file());
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
                "--clang-mode",
                "-fshort-enums",
                "clang: error: '-fshort-enums' is not supported",
            ),
            (
                "--clang-mode",
                "-frounding-math",
                "clang: error: '-frounding-math' is not supported",
            ),
            // An option TCC implements and bcc does not.
            (
                "--tcc-mode",
                "-mno-sse",
                "tcc: error: '-mno-sse' is not supported",
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
        // Linux has these checks, and bcc doesn't give them to Windows.
        if cfg!(windows) {
            for line in [
                "gcc: warning: '-fstack-protector-strong' is ignored: bcc does not emit stack \
                 protectors",
                "gcc: warning: '-fstack-clash-protection' is ignored",
                "gcc: warning: '-fcf-protection' is ignored",
            ] {
                assert!(warnings.contains(line), "{warnings}");
            }
            assert_eq!(warnings.lines().count(), 3, "{warnings}");
        } else {
            assert_eq!(warnings, "", "{warnings}");
        }
        // TCC ignores the options it does not know, so they change nothing
        // in the program it builds either.
        let tcc = directory.bcc(
            "--tcc-mode",
            &["-c", "-fstack-protector", "-fshort-enums", "x.c"],
        );
        succeeds(&tcc);
        assert_eq!(stderr(&tcc), "");
    }

    /// Reaching the end of a function, under the options each compiler has for
    /// it. Each row was measured against GCC 14.2 and Clang 23.1.
    #[test]
    fn gnu_return_type_warnings_follow_their_options() {
        let directory = Directory::new();
        directory.write("a.c", "int f(int x) { if (x) return 1; }\n");
        let warns = |mode: &str, args: &[&str]| {
            let args: Vec<&str> = args
                .iter()
                .copied()
                .chain(["-fsyntax-only", "a.c"])
                .collect();
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            stderr(&output).contains("[-Wreturn-type]")
        };
        for (mode, args, expected) in [
            ("--gcc-mode", &[][..], false),
            ("--gcc-mode", &["-Wall"][..], true),
            ("--gcc-mode", &["-Wreturn-type"][..], true),
            ("--gcc-mode", &["-Wall", "-Wno-return-type"][..], false),
            ("--clang-mode", &[][..], true),
            ("--clang-mode", &["-Wno-return-type"][..], false),
            ("--clang-mode", &["-w"][..], false),
        ] {
            assert_eq!(warns(mode, args), expected, "{mode} {args:?}");
        }
        // Clang's analysis stops at the unit's first error, where GCC's goes
        // on.
        directory.write(
            "e.c",
            "int f(int x) { if (x) return 1; }\nint g(void) { return h; }\nint k(void) { }\n",
        );
        let output = directory.bcc("--clang-mode", &["-fsyntax-only", "e.c"]);
        let text = stderr(&output);
        assert!(text.contains("e.c:1:"), "{text}");
        assert!(!text.contains("e.c:3:"), "{text}");
        let output = directory.bcc("--gcc-mode", &["-Wall", "-fsyntax-only", "e.c"]);
        let text = stderr(&output);
        assert!(text.contains("e.c:1:"), "{text}");
        // `main` falls off its end with a value from C99 on, and GCC holds a
        // hosted one to C89 too.
        directory.write("m.c", "int main(void) { }\n");
        let args = ["-Wall", "-std=c89", "-fsyntax-only", "m.c"];
        assert!(stderr(&directory.bcc("--gcc-mode", &args)).contains("control reaches"));
        assert_eq!(stderr(&directory.bcc("--clang-mode", &args)), "");
        let args = ["-Wall", "-std=c99", "-fsyntax-only", "m.c"];
        assert_eq!(stderr(&directory.bcc("--gcc-mode", &args)), "");
        let args = ["-ffreestanding", "-fsyntax-only", "m.c"];
        assert!(stderr(&directory.bcc("--clang-mode", &args)).contains("does not return"));
    }

    /// What a `switch` handles, under the options each compiler has for it.
    /// Each row was measured against GCC 14.2 and Clang 23.1.
    #[test]
    fn gnu_switch_warnings_follow_their_options() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "enum e { A, B, C };\nint f(enum e x) { switch (x) { case A: return 1; } return 0; \
             }\nint g(enum e x) { switch (x) { case A: return 1; default: return 0; } }\nint \
             h(unsigned char x) { switch (x) { case 300: return 1; } return 0; }\n",
        );
        let flags = |mode: &str, args: &[&str]| {
            let args: Vec<&str> = args
                .iter()
                .copied()
                .chain(["-fsyntax-only", "a.c"])
                .collect();
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            let mut flags: Vec<String> = stderr(&output)
                .lines()
                .filter_map(|line| line.rsplit_once("[-W"))
                .map(|(_, flag)| flag.trim_end_matches(']').to_owned())
                .collect();
            flags.sort_unstable();
            flags.dedup();
            flags.join(" ")
        };
        for (mode, args, expected) in [
            ("--gcc-mode", &[][..], "switch-outside-range"),
            ("--gcc-mode", &["-Wall"][..], "switch switch-outside-range"),
            (
                "--gcc-mode",
                &["-Wall", "-Wno-switch"][..],
                "switch-outside-range",
            ),
            (
                "--gcc-mode",
                &["-Wall", "-Wswitch-enum", "-Wswitch-default"][..],
                "switch switch-default switch-enum switch-outside-range",
            ),
            ("--clang-mode", &[][..], "switch"),
            ("--clang-mode", &["-Wno-switch"][..], ""),
            (
                "--clang-mode",
                &["-Wswitch-enum", "-Wswitch-default"][..],
                "switch switch-default switch-enum",
            ),
        ] {
            assert_eq!(flags(mode, args), expected, "{mode} {args:?}");
        }
        // Clang names the enumerators together, GCC one at a time.
        let output = directory.bcc("--clang-mode", &["-fsyntax-only", "a.c"]);
        assert!(
            stderr(&output).contains("enumeration values 'B' and 'C' not handled in switch"),
            "{}",
            stderr(&output)
        );
        let output = directory.bcc("--gcc-mode", &["-Wall", "-fsyntax-only", "a.c"]);
        assert!(
            stderr(&output).contains("enumeration value 'C' not handled in switch"),
            "{}",
            stderr(&output)
        );
    }

    /// Valid switches both compilers accept under `-Werror` (PR #322's
    /// review): a constant condition its case matches, a `case` that
    /// matches an enumerator once converted, and an enumerator marked
    /// unused. GCC names `-Wswitch-enum` for a `case` outside the
    /// enumeration when `-Wswitch` is off, and Clang's constant condition no
    /// `case` matches has no option. Each row was measured against GCC 14.2
    /// and Clang 23.1.
    #[test]
    fn gnu_switch_warnings_spare_constant_and_converted_cases() {
        let directory = Directory::new();
        directory.write(
            "valid.c",
            "enum e { A, B, C __attribute__((unused)) };
enum u { U = 0xffffffffU, V = 0 };
int f(void) { switch ((enum e)A) { case A: return 1; } return 0; }
int g(enum u x) { switch (x) { case -1: case 0: return 1; } return 0; }
int h(enum e x) { switch (x) { case A: case B: return 1; } return 0; }
",
        );
        for (mode, args) in [
            ("", &["-Werror"][..]),
            ("--clang-mode", &["-Werror"][..]),
            ("--gcc-mode", &["-Wall", "-Werror"][..]),
        ] {
            let mut args = args.to_vec();
            args.extend(["-fsyntax-only", "valid.c"]);
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode} {args:?}");
        }
        directory.write(
            "outside.c",
            "enum e { A, B };
int f(enum e x) { switch (x) { case A: case B: case 5: return 1; } return 0; }
int g(void) { switch ((enum e)B) { case A: return 1; } return 0; }
",
        );
        let output = directory.bcc(
            "--gcc-mode",
            &["-Wswitch-enum", "-fsyntax-only", "outside.c"],
        );
        succeeds(&output);
        assert!(
            stderr(&output).contains(
                "outside.c:2:48: warning: case value '5' not in enumerated type 'enum e' \
                 [-Wswitch-enum]"
            ),
            "{}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("enumeration value 'B' not handled in switch [-Wswitch-enum]"),
            "{}",
            stderr(&output)
        );
        for args in [&[][..], &["-Wno-switch"][..]] {
            let mut args = args.to_vec();
            args.extend(["-fsyntax-only", "outside.c"]);
            let output = directory.bcc("--clang-mode", &args);
            succeeds(&output);
            assert!(
                stderr(&output).contains(
                    "outside.c:3:23: warning: no case matching constant switch condition '1'\n"
                ),
                "{args:?}: {}",
                stderr(&output)
            );
        }
    }

    /// A prototype's `float` for a K&R `float` parameter: GCC and TCC accept
    /// it silently and Clang with a warning, and callers pass the prototype's
    /// type. TCC's own `133_old_func` test calls such a function.
    #[cfg(target_os = "linux")]
    #[test]
    fn knr_definitions_keep_a_prototypes_unpromoted_types() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "#include <stdio.h>\nvoid func(float a, short s);\nvoid func(a, s)\nfloat a; short \
             s;\n{ printf(\"%g %d\\n\", a, s); }\nint main(void) { func(2.5f, 7); func(10.0, 3); \
             return 0; }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let output = directory.bcc(mode, &["a.c", "-o", "a"]);
            succeeds(&output);
            let warned = stderr(&output).contains("[-Wknr-promoted-parameter]");
            assert_eq!(
                warned,
                mode == "--clang-mode",
                "{mode}: {}",
                stderr(&output)
            );
            let run = Command::new(directory.path("a")).output().unwrap();
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                "2.5 7\n10 3\n",
                "{mode}"
            );
        }
        // A declaration that is not a definition still conflicts.
        directory.write("b.c", "void g(float);\nvoid g();\n");
        let output = directory.bcc("--gcc-mode", &["-fsyntax-only", "b.c"]);
        assert!(!output.status.success());
    }

    /// `#pragma once` marks a file, not a spelling of its path: GCC, Clang and
    /// TCC (its own `18_include` test) read the header once however it is
    /// reached. bcc knows a file's identity on Unix hosts only.
    #[cfg(unix)]
    #[test]
    fn pragma_once_marks_the_file_whatever_path_reaches_it() {
        let directory = Directory::new();
        std::fs::create_dir_all(directory.path("sub")).unwrap();
        directory.write("sub/h.h", "#pragma once\nint counter_seen;\n");
        directory.write(
            "sub/a.c",
            "#include \"h.h\"\n#include \"./h.h\"\n#include \"../sub/h.h\"\n",
        );
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let output = directory.bcc(mode, &["-E", "-P", "sub/a.c"]);
            succeeds(&output);
            let text = String::from_utf8_lossy(&output.stdout);
            assert_eq!(text.matches("counter_seen").count(), 1, "{mode}: {text}");
        }
    }

    /// A returned local address and a constant a conversion changes, which
    /// both compilers report by default. Each row was measured against GCC
    /// 14.2 and Clang 23.1.
    #[test]
    fn gnu_value_warnings_follow_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "a.c",
            "int *f(void) { int x; return &x; }\nchar c = 1000;\nchar d = 255;\nunsigned char e = \
             -1;\n",
        );
        let text = |mode: &str, args: &[&str]| {
            let args: Vec<&str> = args
                .iter()
                .copied()
                .chain(["-fsyntax-only", "a.c"])
                .collect();
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            stderr(&output)
        };
        let gcc = text("--gcc-mode", &[]);
        assert!(
            gcc.contains("function returns address of local variable [-Wreturn-local-addr]"),
            "{gcc}"
        );
        assert!(
            gcc.contains("changes value from '1000' to '-24' [-Woverflow]"),
            "{gcc}"
        );
        // GCC leaves a value either signedness holds to -Wsign-conversion.
        assert!(!gcc.contains("'255'"), "{gcc}");
        let clang = text("--clang-mode", &[]);
        assert!(
            clang.contains("address of stack memory associated with local variable 'x' returned"),
            "{clang}"
        );
        assert!(
            clang.contains("changes value from 1000 to -24 [-Wconstant-conversion]"),
            "{clang}"
        );
        assert!(clang.contains("changes value from 255 to -1"), "{clang}");
        assert!(!clang.contains("from -1 to 255"), "{clang}");
        assert_eq!(
            text("--gcc-mode", &["-Wno-return-local-addr", "-Wno-overflow"]),
            ""
        );
        let args = ["-Wno-return-stack-address", "-Wno-constant-conversion"];
        assert_eq!(text("--clang-mode", &args), "");
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

    /// Under `-fsignaling-nans` an operation on a signaling NaN, a constant
    /// one included, is kept and raises the invalid exception at run time,
    /// as in GCC; LLVM's identities such as `x * 1.0` no longer drop it.
    #[test]
    fn signaling_nans_keep_their_operations() {
        let directory = Directory::new();
        directory.write("f.c", "double f(double x) { return x * 1.0; }\n");
        let llvm = directory.bcc("", &["--emit=llvm", "-O2", "-fsignaling-nans", "f.c"]);
        succeeds(&llvm);
        assert!(
            stdout(&llvm).contains("llvm.experimental.constrained.fmul"),
            "{}",
            stdout(&llvm)
        );
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            return;
        }
        directory.write(
            "s.c",
            "#include <fenv.h>\n__attribute__((noinline)) double one(double x) { return x * 1.0; \
             }\n__attribute__((noinline)) double folded(void) { return __builtin_nans(\"\") * \
             2.0; }\nstatic int raised(double r) {\n  int invalid = fetestexcept(FE_INVALID) != \
             0;\n  feclearexcept(FE_ALL_EXCEPT);\n  return invalid && r != r && \
             !__builtin_issignaling(r);\n}\nint main(void) {\n  volatile double s = \
             __builtin_nans(\"\");\n  feclearexcept(FE_ALL_EXCEPT);\n  if (!raised(one(s)))\n    \
             return 1;\n  if (!raised(folded()))\n    return 2;\n  return 0;\n}\n",
        );
        for options in [
            &["-O0"][..],
            &["-O2"],
            &["-O2", "-Xbcc", "--backend=native"],
        ] {
            let mut args = vec!["-fsignaling-nans", "s.c", "-lm", "-o", "s"];
            args.extend(options);
            succeeds(&directory.bcc("--gcc-mode", &args));
            let run = Command::new(directory.path("s")).output().unwrap();
            assert_eq!(run.status.code(), Some(0), "{options:?}");
        }
        // A module with `__bf16` values keeps LLVM's ordinary floating
        // semantics, since LLVM 23 miscompiles `bf16` in `strictfp` code.
        directory.write(
            "b.c",
            "#include <stdio.h>\nvolatile float vf = 1.015625f;\nvolatile int vi = 3;\nint \
             main(void) {\n  __bf16 b = (__bf16)vf;\n  __bf16 c = (__bf16)vi;\n  printf(\"%a %a \
             %a\\n\", (float)b, (float)c, (float)(b + b));\n}\n",
        );
        for options in [
            &["-O0"][..],
            &["-O2"],
            &["-O2", "-Xbcc", "--backend=native"],
        ] {
            let mut args = vec!["-fsignaling-nans", "b.c", "-o", "b"];
            args.extend(options);
            succeeds(&directory.bcc("--gcc-mode", &args));
            let run = Command::new(directory.path("b")).output().unwrap();
            assert_eq!(
                stdout(&run),
                "0x1.04p+0 0x1.8p+1 0x1.04p+1\n",
                "{options:?}"
            );
        }
    }

    /// The Clang identity defines `__FAST_MATH__`, `__FINITE_MATH_ONLY__`
    /// and `__NO_MATH_ERRNO__` as the pinned Clang 23 does, whichever
    /// options gave the semantics up, and its `-Ofast` is `-O3
    /// -ffast-math`, deprecated.
    #[test]
    fn clang_mode_fast_math_macros_follow_the_options() {
        for (options, fast, finite, errno) in [
            (&[][..], false, false, false),
            (&["-ffast-math"], true, true, true),
            (&["-Ofast"], true, true, true),
            (&["-O2", "-Ofast", "-fno-fast-math"], false, false, false),
            (&["-ffinite-math-only"], false, true, false),
            (&["-funsafe-math-optimizations"], false, false, false),
            (
                &["-ffast-math", "-fno-finite-math-only"],
                false,
                false,
                true,
            ),
            (&["-ffast-math", "-fmath-errno"], false, true, false),
            (&["-ffast-math", "-fno-fast-math"], false, false, false),
            (&["-ffast-math", "-fno-approx-func"], false, true, true),
            (&["-ffast-math", "-fsignaling-nans"], true, true, true),
            (&["-fno-honor-nans"], false, false, false),
            (
                &["-fno-honor-nans", "-fno-honor-infinities"],
                false,
                true,
                false,
            ),
            (&["-fno-math-errno"], false, false, true),
            // Clang applies `-Ofast` where it stands, if no `-O` follows.
            (&["-fno-finite-math-only", "-Ofast"], true, true, true),
            (&["-fno-fast-math", "-Ofast"], true, true, true),
            (&["-Ofast", "-O2"], false, false, false),
            // Every `-Ofast` holds if the last `-O` option is one.
            (&["-Ofast", "-Ofast", "-O2"], false, false, false),
            (&["-Ofast", "-O2", "-Ofast"], true, true, true),
            (&["-ffast-math", "-ffp-model=precise"], false, false, false),
            (&["-ffp-model=fast"], false, false, true),
            (&["-ffp-model=aggressive"], true, true, true),
            // A model after a live `-Ofast` loses to it, and `fast` then
            // keeps Clang's default semantics.
            (&["-Ofast", "-ffp-model=fast"], false, false, false),
            // Each model then keeps the default semantics.
            (
                &[
                    "-Ofast",
                    "-ffp-model=fast",
                    "-ffast-math",
                    "-ffp-model=fast",
                ],
                false,
                false,
                false,
            ),
            (
                &["-Ofast", "-ffast-math", "-ffp-model=fast"],
                false,
                false,
                false,
            ),
            (&["-ffp-model=fast", "-Ofast", "-O2"], false, false, true),
            // A later `-O` takes the `-Ofast` back, and the model stands.
            (&["-Ofast", "-ffp-model=fast", "-O2"], false, false, true),
            (
                &["-Ofast", "-ffp-model=fast", "-fmath-errno", "-O2"],
                false,
                false,
                false,
            ),
            (&["-ffp-model=fast", "-Ofast"], true, true, true),
            (
                &[
                    "-fno-signed-zeros",
                    "-fno-trapping-math",
                    "-fassociative-math",
                    "-freciprocal-math",
                    "-fno-math-errno",
                    "-ffinite-math-only",
                    "-fapprox-func",
                ],
                true,
                true,
                true,
            ),
        ] {
            let listing = macros_with("--clang-mode", options);
            assert_eq!(
                defines(&listing, "__FAST_MATH__").as_deref(),
                fast.then_some("1"),
                "{options:?}"
            );
            assert_eq!(
                defines(&listing, "__FINITE_MATH_ONLY__").as_deref(),
                Some(if finite { "1" } else { "0" }),
                "{options:?}"
            );
            assert_eq!(
                defines(&listing, "__NO_MATH_ERRNO__").as_deref(),
                errno.then_some("1"),
                "{options:?}"
            );
        }
        let directory = Directory::new();
        directory.write("x.c", "int x;\n");
        let deprecated = "clang: warning: argument '-Ofast' is deprecated; use '-O3' to enable \
                          only conforming optimizations, or consult the documentation for the \
                          same behavior [-Wdeprecated-ofast]\n";
        for (options, expected) in [
            (&["-Ofast"][..], deprecated.to_owned()),
            (&["-Ofast", "-Ofast"], deprecated.to_owned()),
            (&["-Ofast", "-w"], String::new()),
            (&["-Ofast", "-Wno-deprecated-ofast"], String::new()),
            (&["-Ofast", "-Qunused-arguments"], deprecated.to_owned()),
        ] {
            let mut args = vec!["-c", "x.c"];
            args.extend(options);
            let output = directory.bcc("--clang-mode", &args);
            succeeds(&output);
            assert_eq!(stderr(&output), expected, "{options:?}");
        }
        let error = directory.bcc("--clang-mode", &["-c", "x.c", "-Ofast", "-Werror"]);
        assert!(!error.status.success());
        assert!(
            stderr(&error).contains("clang: error: argument '-Ofast' is deprecated"),
            "{}",
            stderr(&error)
        );
        // Clang's driver warns of a floating-point option a later one
        // overrides.
        for (options, warning) in [
            (
                &["-ffp-model=fast", "-ffp-model=precise"][..],
                "clang: warning: overriding '-ffp-model=fast' option with '-ffp-model=precise' \
                 [-Woverriding-option]",
            ),
            (
                &["-Ofast", "-ffp-model=precise", "-Wno-deprecated"],
                "clang: warning: overriding '-ffp-model=precise' option with '-Ofast' \
                 [-Woverriding-option]",
            ),
            (
                &["-ffast-math", "-ffp-model=fast"],
                "clang: warning: '-ffp-model=fast' sets complex range to \"promoted\" overriding \
                 the setting of \"basic\" that was implied by '-ffast-math' \
                 [-Woverriding-complex-range]",
            ),
        ] {
            let mut args = vec!["-c", "x.c"];
            args.extend(options);
            let output = directory.bcc("--clang-mode", &args);
            succeeds(&output);
            assert_eq!(stderr(&output), format!("{warning}\n"), "{options:?}");
        }
        let quiet = directory.bcc("--clang-mode", &["-c", "x.c", "-Ofast", "-Wno-deprecated"]);
        succeeds(&quiet);
        assert_eq!(stderr(&quiet), "");
        // Clang refuses an evaluation method beside rearranged operations.
        let eval = directory.bcc(
            "--clang-mode",
            &["-c", "x.c", "-ffast-math", "-ffp-eval-method=source"],
        );
        assert!(!eval.status.success());
        for name in ["fapprox-func", "mreassociate", "freciprocal"] {
            assert!(
                stderr(&eval).contains(&format!(
                    "option 'ffp-eval-method' cannot be used with option '{name}'"
                )),
                "{}",
                stderr(&eval)
            );
        }
        succeeds(&directory.bcc("--clang-mode", &["-c", "x.c", "-ffp-eval-method=source"]));
        // Clang's musl and WebAssembly targets leave `errno` alone by
        // default.
        for (options, errno) in [
            (&["--target=x86_64-linux-musl"][..], true),
            (&["--target=x86_64-linux-musl", "-fmath-errno"], false),
            (&["--target=x86_64-linux-gnu"], false),
        ] {
            let listing = macros_with("--clang-mode", options);
            assert_eq!(
                defines(&listing, "__NO_MATH_ERRNO__").is_some(),
                errno,
                "{options:?}"
            );
        }
        // GCC knows neither Clang's `-fno-honor-*` nor `-fapprox-func`.
        for option in ["-fno-honor-nans", "-fapprox-func"] {
            let output = directory.bcc("--gcc-mode", &["-c", "x.c", option]);
            assert!(!output.status.success(), "{option}");
            assert!(
                stderr(&output).contains(&format!(
                    "gcc: error: unrecognized command-line option '{option}'"
                )),
                "{}",
                stderr(&output)
            );
        }
    }

    /// `-ffast-math` and the options it implies give LLVM the fast-math
    /// flags Clang gives it: on each floating operation, and as the
    /// function attributes LLVM's code generator reads. Under
    /// `-fno-math-errno` the math functions read and write no memory, but
    /// where the program names them for its own (`-fno-builtin`).
    #[test]
    fn fast_math_gives_llvm_the_flags_clang_does() {
        let directory = Directory::new();
        directory.write(
            "f.c",
            "double sqrt(double);\ndouble f(double a, double b) { return a * b + a; }\ndouble \
             g(double x) { return sqrt(x); }\n",
        );
        let llvm = |options: &[&str]| {
            let mut args = vec!["--emit=llvm", "-O0"];
            args.extend(options);
            args.push("f.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        // The attribute group of `sqrt`'s declaration.
        let sqrt_attributes = |text: &str| -> String {
            let group = text
                .lines()
                .find(|line| line.starts_with("declare double @sqrt("))
                .and_then(|line| line.rsplit_once(" #"))
                .map(|(_, group)| group.trim().to_owned());
            group
                .and_then(|group| {
                    text.lines()
                        .find(|line| line.starts_with(&format!("attributes #{group} = ")))
                        .map(str::to_owned)
                })
                .unwrap_or_default()
        };
        for (options, multiply, add) in [
            (&[][..], "fmul double", "fadd double"),
            (&["-ffast-math"], "fmul fast double", "fadd fast double"),
            (
                &["-ffinite-math-only"],
                "fmul nnan ninf double",
                "fadd nnan ninf double",
            ),
            (
                &["-funsafe-math-optimizations"],
                "fmul reassoc nsz arcp contract afn double",
                "fadd reassoc nsz arcp contract afn double",
            ),
            // Reassociation needs signed zeros and traps given up too.
            (&["-fassociative-math"], "fmul double", "fadd double"),
            (
                &[
                    "-fassociative-math",
                    "-fno-signed-zeros",
                    "-fno-trapping-math",
                ],
                "fmul reassoc nsz double",
                "fadd reassoc nsz double",
            ),
            (
                &["-freciprocal-math"],
                "fmul arcp double",
                "fadd arcp double",
            ),
            (
                &["-ffast-math", "-fno-fast-math"],
                "fmul double",
                "fadd double",
            ),
            // Signaling NaNs keep every operation, with its exceptions.
            (
                &["-ffast-math", "-fsignaling-nans"],
                "llvm.experimental.constrained.fmul",
                "llvm.experimental.constrained.fadd",
            ),
        ] {
            let text = llvm(options);
            assert!(text.contains(multiply), "{options:?}\n{text}");
            assert!(text.contains(add), "{options:?}\n{text}");
        }
        let fast = llvm(&["-ffast-math"]);
        assert!(
            fast.contains("\"no-signed-zeros-fp-math\"=\"true\"")
                && fast.contains("\"no-trapping-math\"=\"true\""),
            "{fast}"
        );
        assert!(sqrt_attributes(&fast).contains("memory(none)"), "{fast}");
        let plain = llvm(&[]);
        assert!(!plain.contains("no-signed-zeros-fp-math"), "{plain}");
        assert!(!sqrt_attributes(&plain).contains("memory(none)"), "{plain}");
        let errno = llvm(&["-fno-math-errno"]);
        assert!(sqrt_attributes(&errno).contains("memory(none)"), "{errno}");
        for own in [
            &["-fno-math-errno", "-fno-builtin"][..],
            &["-fno-math-errno", "-fno-builtin-sqrt"],
        ] {
            let text = llvm(own);
            assert!(
                !sqrt_attributes(&text).contains("memory(none)"),
                "{own:?}\n{text}"
            );
        }
    }

    /// A program built with `-ffast-math` computes as GCC's and Clang's
    /// builds of it do: NaNs are assumed away, so `isnan` of a NaN is
    /// false once optimized, and the executable links `crtfastmath.o`,
    /// which flushes subnormal results to zero. Both drivers link it when
    /// the last `-f[no-]fast-math` or `-f[no-]unsafe-math-optimizations`
    /// gives the semantics up. bcc's own back end keeps IEC 60559
    /// semantics in its code, which still links `crtfastmath.o`.
    #[test]
    fn fast_math_programs_compute_as_gcc_and_clang_build_them() {
        if !cfg!(all(
            target_os = "linux",
            target_env = "gnu",
            target_arch = "x86_64"
        )) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "r.c",
            "#include <math.h>\nvolatile double zero = 0.0, smallest = \
             2.2250738585072014e-308;\nint main(void) {\n  double n = zero / zero;\n  double half \
             = smallest / 2;\n  return (isnan(n) ? 1 : 0) + (half == 0 ? 2 : 0);\n}\n",
        );
        // The status is 1 for a NaN `isnan` finds, plus 2 for a subnormal
        // flushed to zero, as each compiler's own build exits.
        for (mode, options, status) in [
            ("--gcc-mode", &["-O2"][..], 1),
            ("--gcc-mode", &["-O2", "-ffast-math"], 2),
            ("--gcc-mode", &["-Ofast"], 2),
            ("--gcc-mode", &["-O2", "-ffinite-math-only"], 0),
            ("--gcc-mode", &["-O2", "-ffast-math", "-fno-fast-math"], 1),
            ("--gcc-mode", &["-O2", "-funsafe-math-optimizations"], 3),
            ("--clang-mode", &["-O2", "-ffast-math"], 2),
            ("--clang-mode", &["-Ofast"], 2),
            ("--clang-mode", &["-O2", "-ffast-math", "-fno-fast-math"], 1),
            ("--clang-mode", &["-O2", "-funsafe-math-optimizations"], 3),
            // Both drivers link `crtfastmath.o` after a final `-Ofast`,
            // whatever follows it.
            ("--gcc-mode", &["-Ofast", "-fno-fast-math"], 3),
            ("--clang-mode", &["-Ofast", "-fno-fast-math"], 3),
            ("--gcc-mode", &["-O2", "-fno-finite-math-only", "-Ofast"], 3),
            (
                "--clang-mode",
                &["-O2", "-fno-finite-math-only", "-Ofast"],
                2,
            ),
            (
                "--clang-mode",
                &["-O2", "-ffast-math", "-ffp-model=precise"],
                1,
            ),
            (
                "--gcc-mode",
                &["-O2", "-ffast-math", "-Xbcc", "--backend=native"],
                3,
            ),
        ] {
            let mut args = vec!["r.c", "-lm", "-o", "r"];
            args.extend(options);
            succeeds(&directory.bcc(mode, &args));
            let run = Command::new(directory.path("r")).output().unwrap();
            assert_eq!(run.status.code(), Some(status), "{mode} {options:?}");
        }
    }

    /// `-ftrapv` traps on signed `+`, `-` and `*` overflow and on `abs` of
    /// the most negative value instead of marking them `nsw`; with
    /// `-fwrapv` the last option given decides under the GCC identity, as
    /// in GCC, where `-fno-wrapv` only turns `-fwrapv` off, and `-ftrapv`
    /// under the Clang identity, as in Clang, which knows no `-fno-trapv`.
    #[test]
    fn trapv_traps_on_signed_overflow() {
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
        let trapping = ir(&["-ftrapv"]);
        assert!(trapping.contains("trap"), "{trapping}");
        assert!(!trapping.contains("nsw"), "{trapping}");
        for (options, gcc, clang) in [
            (&["-ftrapv", "-fwrapv"][..], false, true),
            (&["-fwrapv", "-ftrapv"], true, true),
            (&["-ftrapv", "-fwrapv", "-fno-wrapv"], true, true),
            (&["-ftrapv", "-fno-trapv"], false, false),
            (&["-ftrapv", "-fno-strict-overflow"], false, true),
        ] {
            let mut with_gcc = vec!["--compiler-identity=gcc"];
            with_gcc.extend(options);
            assert_eq!(ir(&with_gcc).contains("trap"), gcc, "gcc {options:?}");
            assert_eq!(ir(options).contains("trap"), clang, "clang {options:?}");
        }
        let unknown = directory.bcc("--clang-mode", &["-c", "-fno-trapv", "w.c"]);
        assert!(
            stderr(&unknown).contains("unknown argument: '-fno-trapv'"),
            "{}",
            stderr(&unknown)
        );
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            directory.write(
                "m.c",
                "#include <limits.h>\n#include <stdlib.h>\n__attribute__((noinline)) int add(int \
                 a, int b) { return a + b; }\n__attribute__((noinline)) long absolute(long a) { \
                 return labs(a); }\n__attribute__((noinline)) long mul(long a, long b) { return a \
                 * b; }\n__attribute__((noinline)) __int128 mul128(__int128 a, __int128 b) { \
                 return a * b; }\nint main(int argc, char **argv) {\n  (void)argv;\n  __int128 \
                 min = (__int128)1 << 127;\n  if (add(INT_MAX, -1) != INT_MAX - 1 || \
                 mul(LONG_MIN, 1) != LONG_MIN || mul128(-1, min + 1) != -(min + 1) || \
                 absolute(LONG_MIN + 1) != LONG_MAX)\n    return 2;\n  if (argc == 2)\n    return \
                 add(INT_MAX, 1);\n  if (argc == 3)\n    return (int)mul(LONG_MIN, -1);\n  if \
                 (argc == 4)\n    return (int)mul128(-1, min);\n  if (argc == 5)\n    return \
                 (int)absolute(LONG_MIN);\n  return 0;\n}\n",
            );
            for backend in [&[][..], &["-Xbcc", "--backend=native"]] {
                let mut args = vec!["-O2", "-ftrapv", "m.c", "-o", "m"];
                args.extend(backend);
                succeeds(&directory.bcc("--gcc-mode", &args));
                let run = |arguments: &[&str]| {
                    Command::new(directory.path("m"))
                        .args(arguments)
                        .output()
                        .unwrap()
                        .status
                };
                assert_eq!(run(&[]).code(), Some(0), "{backend:?}");
                for arguments in [
                    &["a"][..],
                    &["a", "b"],
                    &["a", "b", "c"],
                    &["a", "b", "c", "d"],
                ] {
                    let status = run(arguments);
                    assert!(
                        status.code().is_none(),
                        "{backend:?} {arguments:?}: {status:?}"
                    );
                }
            }
            // GNU complex integers' sums and products are checked in their
            // parts, as GCC checks them; a `_Complex short` is not.
            directory.write(
                "c.c",
                "__attribute__((noinline)) _Complex int mul(_Complex int a, _Complex int b) { \
                 return a * b; }\n__attribute__((noinline)) _Complex short smul(_Complex short a, \
                 _Complex short b) { return a * b; }\nint main(int argc, char **argv) {\n  \
                 (void)argv;\n  _Complex int z = mul(3 + 4i, 3 + 4i);\n  if (__real__ z != -7 || \
                 __imag__ z != 24)\n    return 2;\n  _Complex short s = smul(30000, 30000);\n  \
                 (void)s;\n  if (argc > 1)\n    z = mul(2147483647, 2147483647);\n  return \
                 __real__ z == 5;\n}\n",
            );
            succeeds(&directory.bcc("--gcc-mode", &["-O2", "-ftrapv", "c.c", "-o", "c"]));
            let run = |arguments: &[&str]| {
                Command::new(directory.path("c"))
                    .args(arguments)
                    .output()
                    .unwrap()
                    .status
            };
            assert_eq!(run(&[]).code(), Some(0));
            assert!(run(&["a"]).code().is_none());
            // So are their negation and conjugate, and `__builtin_abs` even
            // where `-fno-builtin` makes `abs` the library's alone.
            directory.write(
                "n.c",
                "#include <limits.h>\n#include <stdlib.h>\n__attribute__((noinline)) _Complex int \
                 negate(_Complex int z) { return -z; }\n__attribute__((noinline)) _Complex int \
                 conjugate(_Complex int z) { return ~z; }\n__attribute__((noinline)) int \
                 builtin(int x) { return __builtin_abs(x); }\n__attribute__((noinline)) int \
                 library(int x) { return abs(x); }\nint main(int argc, char **argv) {\n  \
                 (void)argv;\n  if (argc == 2)\n    return __real__ negate(INT_MIN) == 5;\n  if \
                 (argc == 3)\n    return __imag__ conjugate(INT_MIN * 1i) == 5;\n  if (argc == \
                 4)\n    return builtin(INT_MIN) == 5;\n  return library(INT_MIN) == INT_MIN && \
                 __real__ negate(3) == -3 ? 0 : 1;\n}\n",
            );
            succeeds(&directory.bcc(
                "--gcc-mode",
                &["-O2", "-ftrapv", "-fno-builtin", "n.c", "-o", "n"],
            ));
            let run = |arguments: &[&str]| {
                Command::new(directory.path("n"))
                    .args(arguments)
                    .output()
                    .unwrap()
                    .status
            };
            assert_eq!(run(&[]).code(), Some(0));
            for arguments in [&["a"][..], &["a", "b"], &["a", "b", "c"]] {
                assert!(run(arguments).code().is_none(), "{arguments:?}");
            }
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
        // `cl` alone compiles C17 with its classic diagnostics.
        let output = directory.bcc("--msvc-mode", &["bad.c"]);
        assert_ne!(output.status.code(), Some(0));
        assert!(
            stderr(&output).contains("bad.c(1): error:"),
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
            stderr(&bad).contains("bad.c(1): error:"),
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
                format!(
                    "missing.c\n{}\nGenerating Code...\n",
                    object.replace(".obj", ".c")
                )
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

    /// GCC and Clang have their feature-test operators in every language mode,
    /// and neither calls them extensions under `-pedantic`.
    #[test]
    fn gnu_feature_tests_exist_in_every_language_mode() {
        let directory = Directory::new();
        directory.write(
            "q.c",
            "#ifndef __has_include\n#error include\n#endif\n#ifndef __has_c_attribute\n#error \
             attribute\n#endif\n#if !__has_include(<stddef.h>) || \
             __has_include(\"missing.h\")\n#error found\n#endif\n#if \
             !__has_c_attribute(nodiscard)\n#error nodiscard\n#endif\n#if \
             !__has_attribute(unused) || !__has_builtin(__builtin_expect)\n#error \
             gnu\n#endif\nint x;\n",
        );
        for mode in ["--gcc-mode", "--clang-mode"] {
            for standard in ["-std=c89", "-std=c99", "-std=c11", "-std=gnu17", "-std=c23"] {
                let output = directory.bcc(
                    mode,
                    &[
                        standard,
                        "-pedantic-errors",
                        "-Wall",
                        "-c",
                        "q.c",
                        "-o",
                        "q.o",
                    ],
                );
                succeeds(&output);
                assert_eq!(stderr(&output), "", "{mode} {standard}");
            }
        }
        // `-std=c99` leaves these on in both, and `-pedantic-errors` refuses
        // them.
        for (name, source) in [
            ("v.c", "void g(void);\nvoid f(void) { return g(); }\n"),
            ("b.c", "int x = 0b101;\n"),
            (
                "r.c",
                "int f(int x) { switch (x) { case 1 ... 2: return 1; } return 0; }\n",
            ),
            (
                "o.c",
                "#define M(a, ...) a __VA_OPT__(,) __VA_ARGS__\nint M(x);\n",
            ),
        ] {
            directory.write(name, source);
            for mode in ["--gcc-mode", "--clang-mode"] {
                for standard in ["-std=c99", "-std=c11"] {
                    let object = "e.o";
                    succeeds(&directory.bcc(mode, &[standard, "-c", name, "-o", object]));
                    assert!(
                        !directory
                            .bcc(
                                mode,
                                &[standard, "-pedantic-errors", "-c", name, "-o", object]
                            )
                            .status
                            .success(),
                        "{mode} {standard} {name}"
                    );
                }
            }
        }
        // Before `*`, C89 keeps its division, in Clang too.
        directory.write("d.c", "int main(void) { return 4//**/2\n; }\n");
        let args = ["-std=c89", "d.c", "-o", "d"];
        succeeds(&directory.bcc("--clang-mode", &args));
        let status = Command::new(directory.path("d")).status().unwrap();
        assert_eq!(status.code(), Some(2));
        // `//` in C89 is Clang's alone.
        directory.write("c.c", "// comment\nint x;\n");
        let args = ["-std=c89", "-c", "c.c", "-o", "c.o"];
        succeeds(&directory.bcc("--clang-mode", &args));
        assert!(!directory.bcc("--gcc-mode", &args).status.success());
        // GCC 14 has no `__has_embed`, and Clang does.
        directory.write(
            "e.c",
            "#ifdef __has_embed\nint has;\n#else\nint has_not;\n#endif\n",
        );
        let text = |mode: &str| {
            let output = directory.bcc(mode, &["-std=c99", "-E", "-P", "e.c"]);
            succeeds(&output);
            stdout(&output)
        };
        assert_eq!(text("--gcc-mode").trim(), "int has_not;");
        assert_eq!(text("--clang-mode").trim(), "int has;");
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
            // A driver that refuses standard input exits without reading
            // it, which may close the pipe before this write.
            let written = child.stdin.take().unwrap().write_all(b"int from_stdin;\n");
            if let Err(error) = written {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
            }
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
        // MinGW GCC, the default on a Windows host, takes the Microsoft
        // options, so the Linux cases name their target.
        let linux = |args: &[&str]| {
            let mut options = vec!["--target=x86_64-linux-gnu"];
            options.extend(args);
            gcc(&options)
        };
        let refused = linux(&["-c", "-mms-bitfields", "x.c", "-o", "x.o"]);
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
        succeeds(&linux(&[
            "-c",
            "-mno-ms-bitfields",
            "-mlong-double-80",
            "x.c",
            "-o",
            "x.o",
        ]));
        // A processor or feature bcc doesn't model warns: baseline code runs on
        // it.
        for option in ["-march=haswell", "-march=x86-64-v4", "-mavx512f"] {
            let output = linux(&["-c", option, "x.c", "-o", "x.o"]);
            succeeds(&output);
            assert!(
                stderr(&output).contains(&format!("'{option}' is ignored: bcc generates baseline")),
                "{}",
                stderr(&output)
            );
        }
        succeeds(&linux(&[
            "-c",
            "-march=x86-64",
            "-mtune=native",
            "x.c",
            "-o",
            "x.o",
        ]));
    }

    /// bcc's code uses no MMX register, and its MMX intrinsics work in SSE
    /// registers, as GCC's and Clang's do on x86-64, so `-mno-mmx` only
    /// leaves `__MMX__` undefined, as in both compilers: the last of
    /// `-mmmx` and `-mno-mmx` decides, and the command line's `-D` and `-U`
    /// come after.
    #[test]
    fn no_mmx_leaves_the_mmx_macro_undefined() {
        for mode in ["--gcc-mode", "--clang-mode"] {
            for (options, defined) in [
                (&[][..], Some("1")),
                (&["-mno-mmx"], None),
                (&["-mno-mmx", "-mmmx"], Some("1")),
                (&["-mmmx", "-mno-mmx"], None),
                (&["-D__MMX__=2", "-mno-mmx"], Some("2")),
            ] {
                let listing = macros_with(mode, options);
                assert_eq!(
                    defines(&listing, "__MMX__").as_deref(),
                    defined,
                    "{mode} {options:?}"
                );
            }
        }
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            let directory = Directory::new();
            directory.write(
                "m.c",
                "#include <mmintrin.h>\nint main(void) {\n  __m64 a = _mm_set_pi16(1, 2, 3, \
                 4);\n  __m64 b = _mm_add_pi16(a, a);\n  _mm_empty();\n  return \
                 _mm_cvtm64_si64(b) == 0x0002000400060008 ? 0 : 1;\n}\n",
            );
            for backend in [&[][..], &["-Xbcc", "--backend=native"]] {
                let mut args = vec!["-O2", "-mno-mmx", "m.c", "-o", "m"];
                args.extend(backend);
                succeeds(&directory.bcc("--gcc-mode", &args));
                let run = Command::new(directory.path("m")).output().unwrap();
                assert_eq!(run.status.code(), Some(0), "{backend:?}");
            }
        }
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

    /// Messages from bcc's own driver name the driver as the compiler does:
    /// `gcc: warning: ...`, `tcc: ...`, `cl : Command line warning : ...`,
    /// and the name the driver was run as; bcc's own command line names
    /// none.
    #[test]
    fn driver_messages_name_the_driver() {
        let directory = Directory::new();
        directory.write("a.c", "int a;\n");
        directory.write("b.o", "");
        let message = "b.o: linker input file unused because linking not done\n";
        for (mode, prefix) in [
            ("--gcc-mode", "gcc: warning: "),
            ("--tcc-mode", "tcc: warning: "),
            ("", "warning: "),
        ] {
            let output = directory.bcc(mode, &["-c", "a.c", "b.o"]);
            succeeds(&output);
            assert_eq!(stderr(&output), format!("{prefix}{message}"), "{mode}");
        }
        // TCC words `-o` with `-c` and several inputs itself.
        directory.write("b.c", "int b;\n");
        let output = directory.bcc("--tcc-mode", &["-c", "a.c", "b.c", "-o", "x.o"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "tcc: error: cannot specify output file with -c many files\n"
        );
        // `cl`'s form has no invented code. Without the Windows SDK's
        // libraries the link fails in bcc's driver.
        directory.write("m.c", "int main(void) { return 0; }\n");
        for (mode, prefix) in [
            (
                "--msvc-mode",
                "cl : Command line error : cannot build `m.exe`: ",
            ),
            ("--clang-cl-mode", "clang-cl: error: cannot build `m.exe`: "),
        ] {
            let output = directory.bcc(mode, &["/nologo", "m.c"]);
            if !output.status.success() {
                assert!(stderr(&output).starts_with(prefix), "{}", stderr(&output));
            }
        }
        // Run under another name, the driver uses that name.
        #[cfg(unix)]
        {
            let gcc = directory.path("x86_64-linux-gnu-gcc");
            if std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &gcc).is_ok() {
                let output = Command::new(&gcc)
                    .args(["-c", "a.c", "b.o"])
                    .current_dir(&directory.0)
                    .output()
                    .unwrap();
                succeeds(&output);
                assert_eq!(
                    stderr(&output),
                    format!("x86_64-linux-gnu-gcc: warning: {message}")
                );
            }
        }
    }

    /// Clang reports the linker options and the options of the link that a
    /// command line which does not link leaves unused, the linker inputs
    /// first and in order with the input files, then the other unused
    /// arguments in order; GCC reports neither.
    #[test]
    fn clang_reports_unused_link_options() {
        let directory = Directory::new();
        directory.write("a.c", "int a;\n");
        directory.write("b.o", "");
        let output = directory.bcc(
            "--clang-mode",
            &[
                "-c",
                "a.c",
                "-shared-libgcc",
                "-T",
                "x.ld",
                "-u",
                "sym",
                "-L",
                "/a",
                "-Xlinker",
                "-y",
                "-l",
                "m",
                "b.o",
                "-z",
                "now",
                "-Wl,-a,-b",
                "-s",
            ],
        );
        succeeds(&output);
        let unused =
            |what: &str| format!("clang: warning: {what} [-Wunused-command-line-argument]\n");
        let expected: String = [
            "-Xlinker -y: 'linker' input unused",
            "-lm: 'linker' input unused",
            "b.o: 'linker' input unused",
            "-z now: 'linker' input unused",
            "-Wl,-a,-b: 'linker' input unused",
            "argument unused during compilation: '-shared-libgcc'",
            "argument unused during compilation: '-T x.ld'",
            "argument unused during compilation: '-u sym'",
            "argument unused during compilation: '-L/a'",
            "argument unused during compilation: '-s'",
        ]
        .into_iter()
        .map(unused)
        .collect();
        assert_eq!(stderr(&output), expected);
        // `-fsyntax-only` leaves the stage unused; `-E` leaves
        // `-fsyntax-only` unused, in order with the link's options.
        let output = directory.bcc("--clang-mode", &["-L/tmp", "-fsyntax-only", "-c", "a.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            [
                "argument unused during compilation: '-L/tmp'",
                "argument unused during compilation: '-c'",
            ]
            .into_iter()
            .map(unused)
            .collect::<String>()
        );
        let output = directory.bcc(
            "--clang-mode",
            &["-fsyntax-only", "-L/tmp", "-E", "a.c", "-o", "a.i", "-lm"],
        );
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            [
                "-lm: 'linker' input unused",
                "argument unused during compilation: '-fsyntax-only'",
                "argument unused during compilation: '-L/tmp'",
            ]
            .into_iter()
            .map(unused)
            .collect::<String>()
        );
        // `-Werror` makes the first one an error.
        let output = directory.bcc("--clang-mode", &["-c", "-lm", "a.c", "-Werror"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "clang: error: -lm: 'linker' input unused [-Werror,-Wunused-command-line-argument]\n"
        );
        // A command line that links uses them all, and GCC reports none.
        let output = directory.bcc("--gcc-mode", &["-c", "a.c", "-lm", "-L/tmp", "-Wl,-x"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
    }

    /// TCC skips a `#!` line that starts a file, so a script such as
    /// `#!/usr/bin/tcc -run` runs; GCC rejects the line.
    #[test]
    fn tcc_skips_a_script_line() {
        let directory = Directory::new();
        directory.write(
            "script.c",
            &format!(
                "#!/usr/bin/tcc -run\n{TCC_STDIO}int main(int argc, char **argv) {{\n  \
                 printf(\"%d %s\\n\", argc, argv[1]);\n  return 3;\n}}\n"
            ),
        );
        let output = directory.bcc("--tcc-mode", &["-run", "script.c", "one"]);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        assert_eq!(stdout(&output), "2 one\n");
        assert_eq!(stderr(&output), "");
        // Only at the start of the file.
        directory.write("late.c", "int x;\n#!/usr/bin/tcc -run\n");
        let output = directory.bcc("--tcc-mode", &["-c", "late.c"]);
        assert!(stderr(&output).contains("late.c:2:"), "{}", stderr(&output));
        let output = directory.bcc("--gcc-mode", &["-c", "script.c"]);
        assert!(
            stderr(&output).contains("script.c:1:"),
            "{}",
            stderr(&output)
        );
    }

    /// The TCC personality reads its command line as `tcc_parse_args`
    /// does: TCC's table in TCC's order, `@FILE`, `-Wp,`, the last of
    /// `-c`, `-E`, `-r`, `-shared` and `-run` winning with TCC's warning,
    /// and the options TCC ignores accepted silently.
    #[test]
    fn tcc_reads_its_command_line_as_tcc_does() {
        let directory = Directory::new();
        directory.write("x.c", "int x = X;\n");
        directory.write("inc.h", "#define VALUE 7\n");
        directory.write("args.txt", "-c \"x.c\" -DX=2 -o fromfile.o\n");
        let output = directory.bcc("--tcc-mode", &["@args.txt"]);
        succeeds(&output);
        assert!(directory.path("fromfile.o").is_file());
        let output = directory.bcc("--tcc-mode", &["@missing.txt"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "tcc: error: listfile 'missing.txt' not found\n"
        );
        // TCC never ends a listfile that names itself; bcc stops.
        directory.write("self.txt", "@self.txt\n");
        let output = directory.bcc("--tcc-mode", &["@self.txt"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "tcc: error: listfile 'self.txt' not read: more than 4096 listfiles (does one name \
             itself?)\n"
        );
        directory.write("two.txt", "@two.txt @two.txt\n");
        let output = directory.bcc("--tcc-mode", &["@two.txt"]);
        assert_eq!(output.status.code(), Some(1));
        // A value that starts with `-` stays a value, as in TCC: here a
        // directory named `-c`.
        std::fs::create_dir(directory.path("-c")).unwrap();
        directory.write("-c/dash.h", "#define X 4\n");
        directory.write("dash.c", "#include \"dash.h\"\nint x = X;\n");
        for args in [
            &["-I", "-c", "-c", "dash.c"][..],
            &["-isystem", "-c", "-c", "dash.c"][..],
        ] {
            let output = directory.bcc("--tcc-mode", args);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{args:?}");
        }
        // `-Wp,` splits its operand into options; `-ofile` is joined.
        let output = directory.bcc("--tcc-mode", &["-Wp,-DX=3,-UY", "-c", "x.c", "-ojoined.o"]);
        succeeds(&output);
        assert!(directory.path("joined.o").is_file());
        // The last action wins, with a warning for each after the first.
        let output = directory.bcc("--tcc-mode", &["-DX=1", "-E", "-c", "x.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "tcc: warning: -c: overriding compiler action already specified\n"
        );
        assert!(directory.path("x.o").is_file());
        let output = directory.bcc(
            "--tcc-mode",
            &["-DX=1", "-c", "-c", "x.c", "-o", "a.o", "-o", "b.o"],
        );
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "tcc: warning: -c: overriding compiler action already specified\ntcc: warning: \
             multiple -o option\n"
        );
        assert!(directory.path("b.o").is_file());
        // TCC's no-op options, and the ones it does not know, are ignored;
        // `-Wunsupported` names them.
        let ignored = [
            "-pedantic",
            "-pipe",
            "-s",
            "-traditional",
            "-C",
            "--param",
            "x=1",
            "-arch",
            "x86_64",
            "-pie",
            "-fbogus",
            "-mbogus",
            "-Wbogus",
            "-g3",
            "-gdwarf-4",
            "-O2",
            "-Os",
        ];
        let mut args = ignored.to_vec();
        args.extend(["-DX=1", "-c", "x.c"]);
        let output = directory.bcc("--tcc-mode", &args);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let output = directory.bcc(
            "--tcc-mode",
            &["-Wunsupported", "-fbogus", "-pipe", "-DX=1", "-c", "x.c"],
        );
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "tcc: warning: unsupported option '-fbogus'\ntcc: warning: unsupported option \
             '-pipe'\n"
        );
        // An option TCC's table does not hold, as TCC words it.
        for (args, message) in [
            (
                &["-MT", "x", "x.c"][..],
                "tcc: error: invalid option -- '-MT'\n",
            ),
            (
                &["-pedantic-errors", "x.c"][..],
                "tcc: error: invalid option -- '-pedantic-errors'\n",
            ),
            (&["-o"][..], "tcc: error: argument to '-o' is missing\n"),
            (&["x.c", "-h"][..], "tcc: error: cannot parse -h here\n"),
            (&["-run"][..], "tcc: error: no input files\n"),
            (&["-c"][..], "tcc: error: no input files\n"),
        ] {
            let output = directory.bcc("--tcc-mode", args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stderr(&output), message, "{args:?}");
        }
        // No arguments print the help; `-v` alone the version; `-vv` alone
        // the version and the search directories; `-dumpmachine`, as `mob`.
        let output = directory.bcc("--tcc-mode", &[]);
        succeeds(&output);
        assert!(
            stdout(&output).contains("Usage: tcc [options...]"),
            "{}",
            stdout(&output)
        );
        let output = directory.bcc("--tcc-mode", &["-vv"]);
        succeeds(&output);
        let text = stdout(&output);
        assert!(text.starts_with("tcc version 0.9.27 "), "{text}");
        assert!(
            text.contains("\ninstall: ") && text.contains("\ninclude:\n"),
            "{text}"
        );
        let output = directory.bcc("--tcc-mode", &["-dumpmachine"]);
        succeeds(&output);
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            assert_eq!(stdout(&output), "x86_64-pc-linux-gnu\n");
        }
        // `-v` with inputs says what it reads and writes, on stdout.
        let output = directory.bcc("--tcc-mode", &["-v", "-DX=1", "-c", "x.c"]);
        succeeds(&output);
        assert!(
            stdout(&output).ends_with("\n-> x.c\n<- x.o\n"),
            "{}",
            stdout(&output)
        );
    }

    /// `tcc -ar` checks its arguments as TCC's own archiver does
    /// (`tcc_tool_ar`): its usage for a missing library or an operation it
    /// lacks, names alone for `t`/`tv`, status 2 for a file it cannot open,
    /// and a fresh library of exactly the given objects.
    #[test]
    fn tcc_ar_reads_its_arguments_as_tcc_does() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let directory = Directory::new();
        directory.write("a.c", "int a(void) { return 1; }\n");
        directory.write("b.c", "int b(void) { return 2; }\n");
        succeeds(&directory.bcc("--tcc-mode", &["-c", "a.c", "b.c"]));
        let usage =
            "usage: tcc -ar [crstvx] lib [files]\ncreate library ([abdiopN] not supported).\n";
        for args in [
            &["-ar"][..],
            &["-ar", "rcs"],
            &["-ar", "d", "lib.a", "a.o"],
            &["-ar", "-x.y", "lib.a"],
        ] {
            let output = directory.bcc("--tcc-mode", args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stderr(&output), usage, "{args:?}");
        }
        succeeds(&directory.bcc("--tcc-mode", &["-ar", "rcs", "lib.a", "a.o", "b.o"]));
        for op in ["t", "tv"] {
            let output = directory.bcc("--tcc-mode", &["-ar", op, "lib.a"]);
            succeeds(&output);
            assert_eq!(stdout(&output), "a.o\nb.o\n", "{op}");
        }
        // A new library replaces the old one rather than updating it.
        succeeds(&directory.bcc("--tcc-mode", &["-ar", "rcs", "lib.a", "b.o"]));
        let output = directory.bcc("--tcc-mode", &["-ar", "t", "lib.a"]);
        assert_eq!(stdout(&output), "b.o\n");
        let output = directory.bcc("--tcc-mode", &["-ar", "cr", "new.a", "missing.o"]);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(stderr(&output), "tcc: ar: can't open file missing.o \n");
        // An input that is no ELF object of TCC's class leaves no library;
        // `v` names each object as it is read.
        directory.write("bad.o", "not an object\n");
        let output = directory.bcc("--tcc-mode", &["-ar", "rcs", "bad.a", "a.o", "bad.o"]);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(stderr(&output), "tcc: ar: Unsupported Elf Class: bad.o\n");
        assert!(!directory.path("bad.a").exists());
        let output = directory.bcc("--tcc-mode", &["-ar", "rcsv", "v.a", "a.o", "b.o"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "a - a.o\na - b.o\n");
        let output = directory.bcc("--tcc-mode", &["-ar", "t", "absent.a"]);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(stderr(&output), "tcc: ar: can't open file absent.a\n");
        let output = directory.bcc("--tcc-mode", &["-ar", "t", "a.c"]);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(stderr(&output), "tcc: ar: not an ar archive a.c\n");
    }

    /// TCC names its outputs after its inputs (`a.c` → `a.o`, standard
    /// input → `a.out`), compiles each `-c` input on its own and stops at
    /// the first that fails, and writes make rules one file to a line.
    #[test]
    fn tcc_names_outputs_and_rules_as_tcc_does() {
        use std::io::Write as _;
        let directory = Directory::new();
        directory.write("a.c", "int b(void);\nint main(void) { return b(); }\n");
        directory.write("b.c", "#include \"inc.h\"\nint b(void) { return VALUE; }\n");
        directory.write("inc.h", "#define VALUE 7\n");
        directory.write("bad.c", "int f(void) { return y; }\n");
        let output = directory.bcc("--tcc-mode", &["-c", "a.c", "b.c"]);
        succeeds(&output);
        assert!(directory.path("a.o").is_file() && directory.path("b.o").is_file());
        // A failing input stops the others, and says so once.
        let output = directory.bcc("--tcc-mode", &["-c", "bad.c", "b.c", "-MD"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stderr(&output), "bad.c:1: error: 'y' undeclared\n");
        let output = directory.bcc("--tcc-mode", &["-c", "a.c", "-lm"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "tcc: error: cannot specify libraries with -c\n"
        );
        // Standard input's object is `a.out`, as TCC names it.
        let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--tcc-mode", "-c", "-"])
            .current_dir(&directory.0)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"int f(void) { return 1; }\n")
            .unwrap();
        assert!(child.wait().unwrap().success());
        let object = std::fs::read(directory.path("a.out")).unwrap();
        // An ELF relocatable object (`e_type` 1).
        if cfg!(target_os = "linux") {
            assert_eq!(&object[..4], b"\x7fELF");
            assert_eq!(object[16], 1);
        }
        // `-r` links the inputs into one relocatable object.
        if cfg!(target_os = "linux") {
            let output = directory.bcc("--tcc-mode", &["-r", "a.c", "b.c", "-o", "r.o"]);
            succeeds(&output);
            let merged = std::fs::read(directory.path("r.o")).unwrap();
            assert_eq!(merged[16], 1);
            let output = directory.bcc("--tcc-mode", &["r.o", "-o", "linked"]);
            succeeds(&output);
            let run = Command::new(directory.path("linked")).output().unwrap();
            assert_eq!(run.status.code(), Some(7));
        }
        // TCC's rules: a file to a line, named after the output.
        let output = directory.bcc("--tcc-mode", &["-M", "b.c"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "b.o: \\\n  b.c \\\n  inc.h\n");
        let output = directory.bcc("--tcc-mode", &["-MD", "-MP", "-c", "b.c", "-o", "out.o"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("out.d")).unwrap(),
            "out.o: \\\n  b.c \\\n  inc.h\ninc.h:\n"
        );
        // A link writes one rule, named after the output, listing every
        // file its inputs read in order; `-MP` lists each after the first,
        // the later inputs too, as `tcc` 0.9.27 writes them.
        let output = directory.bcc("--tcc-mode", &["-MMD", "b.c", "a.c", "-o", "prog"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("prog.d")).unwrap(),
            "prog: \\\n  b.c \\\n  inc.h \\\n  a.c\n"
        );
        let output = directory.bcc("--tcc-mode", &["-MD", "-MP", "b.c", "a.c", "-o", "prog"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("prog.d")).unwrap(),
            "prog: \\\n  b.c \\\n  inc.h \\\n  a.c\ninc.h:\na.c:\n"
        );
        let output = directory.bcc("--tcc-mode", &["-MD", "b.c", "a.c", "-MF", "deps.out"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("deps.out")).unwrap(),
            "a.out: \\\n  b.c \\\n  inc.h \\\n  a.c\n"
        );
        // Without `-o`, the rule still names the output, in `a.d`.
        let output = directory.bcc("--tcc-mode", &["-MD", "b.c", "a.c"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("a.d")).unwrap(),
            "a.out: \\\n  b.c \\\n  inc.h \\\n  a.c\n"
        );
        assert!(!directory.path("b.d").exists());
        std::fs::remove_file(directory.path("a.d")).unwrap();
        directory.write("main.c", "int main(void) { return 0; }\n");
        let output = directory.bcc("--tcc-mode", &["-MD", "main.c"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("a.d")).unwrap(),
            "a.out: \\\n  main.c\n"
        );
        // A failed link writes no rule.
        let output = directory.bcc("--tcc-mode", &["-MD", "bad.c", "b.c", "-o", "failed"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(!directory.path("failed.d").exists());
        // Nor does a lone source whose link fails.
        directory.write(
            "unlinked.c",
            "int f(void);\nint main(void) { return f(); }\n",
        );
        let output = directory.bcc("--tcc-mode", &["-MD", "-MF", "rule.d", "unlinked.c"]);
        assert!(!output.status.success());
        assert!(!directory.path("rule.d").exists());
        // A header two inputs read is listed once, and standard input is
        // `-`, first.
        directory.write("c.c", "#include \"inc.h\"\nint c(void) { return VALUE; }\n");
        let output = directory.bcc(
            "--tcc-mode",
            &[
                "-MD", "-MP", "-MF", "shared.d", "a.c", "b.c", "c.c", "-o", "shared",
            ],
        );
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("shared.d")).unwrap(),
            "shared: \\\n  a.c \\\n  b.c \\\n  inc.h \\\n  c.c\nb.c:\ninc.h:\nc.c:\n"
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--tcc-mode", "-MD", "-c", "-", "-o", "piped.o"])
            .current_dir(&directory.0)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"#include \"inc.h\"\nint p(void) { return VALUE; }\n")
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read_to_string(directory.path("piped.d")).unwrap(),
            "piped.o: \\\n  - \\\n  inc.h\n"
        );
        // An assembly input is listed as a C source is.
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            directory.write("s.s", ".globl s\ns:\n ret\n");
            let output = directory.bcc("--tcc-mode", &["-MD", "b.c", "s.s", "a.c", "-o", "p"]);
            succeeds(&output);
            assert_eq!(
                std::fs::read_to_string(directory.path("p.d")).unwrap(),
                "p: \\\n  b.c \\\n  inc.h \\\n  s.s \\\n  a.c\n"
            );
        }
        let output = directory.bcc("--tcc-mode", &["-MD", "b.c", "-shared", "-o", "lib.so"]);
        succeeds(&output);
        assert_eq!(
            std::fs::read_to_string(directory.path("lib.d")).unwrap(),
            "lib.so: \\\n  b.c \\\n  inc.h\n"
        );
    }

    /// Without `-o`, a link's dependency rules name each source and its
    /// object: GCC writes them to `a-SOURCE.d`, after its default output's
    /// dump base, and Clang to `SOURCE.d`, link options or not; with `-o`,
    /// a lone input's rule names the output.
    #[test]
    fn link_dependencies_without_an_output_name_the_sources() {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            return;
        }
        let directory = Directory::new();
        directory.write("inc.h", "int x;\n");
        directory.write("c.c", "#include \"inc.h\"\nint main(void) { return x; }\n");
        directory.write("s.S", ".globl main\nmain:\n xor %eax, %eax\n ret\n");
        for (mode, prefix) in [("--gcc-mode", "a-"), ("--clang-mode", "")] {
            for (source, rule) in [("c.c", "c.o: c.c inc.h\n"), ("s.S", "s.o: s.S\n")] {
                for link in [&[][..], &["-lm"]] {
                    let mut args = vec!["-MMD", source];
                    args.extend(link);
                    let output = directory.bcc(mode, &args);
                    succeeds(&output);
                    let stem = source.split('.').next().unwrap();
                    let file = directory.path(&format!("{prefix}{stem}.d"));
                    assert_eq!(
                        std::fs::read_to_string(&file).unwrap_or_default(),
                        rule,
                        "{mode} {args:?}"
                    );
                    std::fs::remove_file(file).unwrap();
                    assert!(!directory.path("a.d").exists(), "{mode} {args:?}");
                }
                let output = directory.bcc(mode, &["-MMD", source, "-lm", "-o", "prog"]);
                succeeds(&output);
                let named = std::fs::read_to_string(directory.path("prog.d")).unwrap();
                assert_eq!(
                    named,
                    rule.replacen(&format!("{}.o", &source[..1]), "prog", 1)
                );
            }
        }
    }

    /// `-run FILE ARGS...` reads the options before FILE as its own, gives
    /// the program the arguments after it, and `tcc FILES -run -- ARGS`
    /// runs FILES; `-rstdin` gives the program a file as its input.
    #[test]
    fn tcc_run_reads_options_and_arguments_as_tcc_does() {
        let directory = Directory::new();
        directory.write(
            "args.c",
            &format!(
                "{TCC_STDIO}int main(int argc, char **argv) {{\n  int c, n = 0;\n  while ((c = \
                 getchar()) != EOF) n++;\n  printf(\"%d %d %s\\n\", N, n, argv[argc - 1]);\n  \
                 return argc;\n}}\n"
            ),
        );
        directory.write("input.txt", "abc");
        directory.write(
            "name.c",
            &format!("{TCC_STDIO}int main(int argc, char **argv) {{ puts(argv[0]); return 0; }}\n"),
        );
        // The program's `argv[0]` is the argument TCC stopped at.
        for (args, name) in [
            (&["-run", "name.c", "x"][..], "name.c\n"),
            (&["name.c", "-run", "--", "x"][..], "--\n"),
        ] {
            let output = directory.bcc("--tcc-mode", args);
            succeeds(&output);
            if cfg!(unix) {
                assert_eq!(stdout(&output), name, "{args:?}");
            }
        }
        // TCC opens `-rstdin`'s file before it compiles anything.
        let output = directory.bcc("--tcc-mode", &["-rstdin", "nope.txt", "-run", "name.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "tcc: error: failed to reopen stdin from 'nope.txt'\n"
        );
        let output = directory.bcc(
            "--tcc-mode",
            &[
                "-run",
                "-DN=4",
                "-rstdin",
                "input.txt",
                "args.c",
                "-v",
                "last",
            ],
        );
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        assert_eq!(stdout(&output), "4 3 last\n");
        directory.write("n.c", "#define N 5\n#include \"args.c\"\n");
        let output = directory.bcc("--tcc-mode", &["n.c", "-run", "--", "x", "y"]);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        assert_eq!(stdout(&output), "5 0 y\n");
        // A `#!/usr/bin/tcc -run -DN=6` line hands its options over as one
        // argument.
        let output = directory.bcc("--tcc-mode", &["-run -DN=6", "args.c", "z"]);
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(stdout(&output), "6 0 z\n");
    }

    /// The TCC identity claims what TCC claims and keeps TCC's warnings:
    /// glibc's `__REDIRECT` family, no `__int128` size macros, Microsoft's
    /// anonymous members by default, pointer/integer assignments as
    /// warnings, and one line per diagnostic up to the first error.
    #[test]
    fn tcc_identity_follows_tcc() {
        // TCC for Windows, the default on a Windows host, claims Windows
        // and leaves the glibc family out (`tccdefs.h`, `!defined _WIN32`).
        for (target, unix) in [
            ("x86_64-unknown-linux-gnu", true),
            ("x86_64-w64-windows-gnu", false),
        ] {
            let tcc = macros_with(
                "",
                &["--compiler-identity=tcc", &format!("--target={target}")],
            );
            assert_eq!(
                defines(&tcc, "__REDIRECT(name,proto,alias)").as_deref(),
                unix.then_some("name proto __asm__(#alias)"),
                "{target}"
            );
            assert_eq!(
                defines(&tcc, "__unix__").as_deref(),
                unix.then_some("1"),
                "{target}"
            );
            assert_eq!(
                defines(&tcc, "_WIN32").as_deref(),
                (!unix).then_some("1"),
                "{target}"
            );
        }
        let tcc = macros("--tcc-mode");
        let windows = cfg!(windows);
        assert_eq!(
            defines(&tcc, "__REDIRECT(name,proto,alias)").is_some(),
            !windows
        );
        for name in [
            "__SIZEOF_INT128__",
            "__SIZEOF_FLOAT128__",
            "__FLOAT128__",
            "linux",
            "__NO_INLINE__",
        ] {
            assert_eq!(defines(&tcc, name), None, "{name}");
        }
        assert_eq!(
            defines(&tcc, "__unix__").as_deref(),
            (!windows).then_some("1")
        );
        assert_eq!(defines(&tcc, "_WIN32").as_deref(), windows.then_some("1"));
        let c11 = macros_with("--tcc-mode", &["-std=c11"]);
        assert_eq!(
            defines(&c11, "__STDC_VERSION__").as_deref(),
            Some("201112L")
        );
        let directory = Directory::new();
        directory.write(
            "ms.c",
            "struct A { int x; };\nstruct B { struct A; int y; };\nint main(void) { struct B b = \
             { { 1 }, 2 }; return b.x + b.y; }\n",
        );
        let output = directory.bcc("--tcc-mode", &["-run", "ms.c"]);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        let output = directory.bcc("--tcc-mode", &["-fno-ms-extensions", "-c", "ms.c"]);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        // TCC warns where GCC 14 stops; `-Werror` stops again.
        directory.write(
            "ptr.c",
            "int main(void) { int i = 0; int *p = 5; char *q = &i; return 0; }\n",
        );
        let output = directory.bcc("--tcc-mode", &["-c", "ptr.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "ptr.c:1: warning: incompatible integer and pointer conversion\nptr.c:1: warning: \
             assignment from incompatible pointer type\n"
        );
        let output = directory.bcc("--tcc-mode", &["-Werror", "-c", "ptr.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "ptr.c:1: error: incompatible integer and pointer conversion\n"
        );
        // `$` follows `-f[no-]dollars-in-identifiers`, in every personality.
        directory.write("dollar.c", "int a$b = 3;\n");
        succeeds(&directory.bcc("--tcc-mode", &["-c", "dollar.c"]));
        for mode in ["--tcc-mode", "--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-fno-dollars-in-identifiers", "-c", "dollar.c"]);
            assert_eq!(output.status.code(), Some(1), "{mode}: {}", stderr(&output));
        }
    }

    /// `tcc -ar` runs the bundled archiver with the rest of the command
    /// line, as TCC runs its own.
    #[test]
    fn tcc_archives_with_the_bundled_archiver() {
        let directory = Directory::new();
        directory.write("lib.c", "int value(void) { return 42; }\n");
        directory.write(
            "use.c",
            "int value(void);\nint main(void) { return value(); }\n",
        );
        succeeds(&directory.bcc("--tcc-mode", &["-c", "lib.c"]));
        let output = directory.bcc("--tcc-mode", &["-ar", "rcs", "libv.a", "lib.o"]);
        succeeds(&output);
        assert!(directory.path("libv.a").is_file());
        let output = directory.bcc("--tcc-mode", &["-c", "-ar", "rcs", "x.a"]);
        assert_eq!(stderr(&output), "tcc: error: cannot parse -ar here\n");
        if cfg!(target_os = "linux") {
            succeeds(&directory.bcc("--tcc-mode", &["use.c", "-L.", "-lv", "-o", "use"]));
            let run = Command::new(directory.path("use")).output().unwrap();
            assert_eq!(run.status.code(), Some(42));
        }
    }

    /// With `BCC_TCC` naming a real `tcc`, compares it with the TCC
    /// personality on the command lines where they are meant to agree:
    /// exit status, stdout, stderr, the files written, and what the
    /// programs built print. The Linux CI job names Ubuntu's `tcc`; the test
    /// passes without one. `docs/design/driver-modes.md` lists the
    /// differences that remain.
    #[test]
    fn tcc_parity_with_a_real_tcc() {
        let Some(tcc) = std::env::var_os("BCC_TCC") else {
            eprintln!("skipping: BCC_TCC names no tcc binary");
            return;
        };
        let fixtures: &[(&str, &str)] = &[
            (
                "hello.c",
                "#include <stdio.h>\nint main(int argc, char **argv) {\n  for (int i = 1; i < \
                 argc; i++) printf(\"%s\\n\", argv[i]);\n  return argc;\n}\n",
            ),
            ("a.c", "int b(void);\nint main(void) { return b(); }\n"),
            ("b.c", "#include \"inc.h\"\nint b(void) { return VALUE; }\n"),
            ("inc.h", "#define VALUE 7\n"),
            (
                "macros.c",
                "#ifdef FOO\nint foo = FOO;\n#endif\nint v = __STDC_VERSION__;\nint main(void) { \
                 char c = (char)200; return c < 0; }\n",
            ),
            (
                "ms.c",
                "struct A { int x; };\nstruct B { struct A; int y; };\nint main(void) { struct B \
                 b; b.x = 1; b.y = 2; return b.x + b.y; }\n",
            ),
            ("dollar.c", "int a$b = 3;\nint main(void) { return a$b; }\n"),
            (
                "script.c",
                "#!/usr/bin/tcc -run\n#include <stdio.h>\nint main(int argc, char **argv) { \
                 printf(\"script %d\\n\", argc); return 0; }\n",
            ),
            ("lib.c", "int libval(void) { return 42; }\n"),
            ("args.txt", "-c hello.c -o fromfile.o\n"),
        ];
        let cases: &[&[&str]] = &[
            &["-c"],
            &["hello.c"],
            &["hello.c", "-o", "prog"],
            &["-c", "hello.c"],
            &["-c", "a.c", "b.c"],
            &["-c", "a.c", "b.c", "-o", "x.o"],
            &["a.c", "b.c"],
            &["-r", "a.c", "b.c", "-o", "r.o"],
            &["missing.c"],
            &["-c", "a.c", "missing.c", "b.c"],
            &["missing.o", "a.c"],
            &["-E", "-P", "b.c"],
            &["-MD", "-c", "b.c"],
            &["-MD", "-MF", "deps.txt", "-c", "b.c"],
            &["-run", "hello.c", "one", "-v"],
            &["-run"],
            &["-run", "missing.c"],
            &["-run", "script.c", "a"],
            &["-DFOO=1", "-run", "macros.c"],
            &["-funsigned-char", "-run", "macros.c"],
            &["-std=c11", "-E", "-P", "macros.c"],
            &["-std=c17", "-E", "-P", "macros.c"],
            &["-run", "ms.c"],
            &["-run", "dollar.c"],
            &["-shared", "lib.c", "-o", "libx.so"],
            &["-soname", "libq.so", "-shared", "lib.c", "-o", "libq.so"],
            &[
                "-Wbogus",
                "-fbogus",
                "-mbogus",
                "-fshort-enums",
                "-c",
                "hello.c",
            ],
            &[
                "-pedantic",
                "-pipe",
                "-s",
                "-traditional",
                "--param",
                "x=1",
                "-c",
                "hello.c",
            ],
            &["-q", "hello.c"],
            &["--bogus", "hello.c"],
            &["-o"],
            &["-c", "-c", "hello.c"],
            &["@args.txt"],
            &["-Wp,-DFOO=5", "-E", "-P", "macros.c"],
            &["-I", "-c", "-c", "hello.c"],
        ];
        let run = |program: &std::ffi::OsStr, mode: Option<&str>, args: &[&str]| {
            let directory = Directory::new();
            for (name, text) in fixtures {
                directory.write(name, text);
            }
            let output = Command::new(program)
                .args(mode)
                .args(args)
                .current_dir(&directory.0)
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            let mut written: Vec<String> = std::fs::read_dir(&directory.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    !fixtures
                        .iter()
                        .any(|(fixture, _)| *fixture == name.as_str())
                })
                .collect();
            written.sort();
            let mut ran = Vec::new();
            for name in &written {
                if name == "a.out" || name == "prog" {
                    let program = Command::new(directory.path(name)).output().unwrap();
                    ran.push((program.status.code(), stdout(&program)));
                }
            }
            // 0.9.27 ends `no input files` with an empty line.
            let stderr = stderr(&output).trim_end().to_owned();
            (output.status.code(), stdout(&output), stderr, written, ran)
        };
        let mut failures = Vec::new();
        for args in cases {
            let expected = run(&tcc, None, args);
            let actual = run(
                std::ffi::OsStr::new(env!("CARGO_BIN_EXE_bcc-rust")),
                Some("--tcc-mode"),
                args,
            );
            if expected != actual {
                failures.push(format!(
                    "tcc {}:\n  tcc: {expected:?}\n  bcc: {actual:?}",
                    args.join(" ")
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// `-###` prints each step as GCC and Clang print a subprocess, naming
    /// the running bcc, which runs it in process; `-v` prints each step
    /// before running it, and bcc's driver then prints its header search
    /// and the commands it runs, the bundled clang's link among them.
    #[test]
    fn verbose_and_dry_run_print_the_commands() {
        let directory = Directory::new();
        directory.write("a.c", "int main(void) { return 0; }\n");
        directory.write("b.c", "int b;\n");
        let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_bcc-rust")).unwrap();
        // Windows spells a canonical path verbatim (`\\?\C:\…`); the running
        // bcc names itself without the prefix.
        let exe = exe.to_str().unwrap();
        let exe = exe.strip_prefix(r"\\?\").unwrap_or(exe);
        // Clang's `-###` escapes a backslash in a quoted word.
        let line = |options: &str| format!(" \"{}\" {options}\n", exe.replace('\\', r"\\"));
        let (triple, gcc_triple) = HOST_TARGET;
        let output = directory.bcc("--clang-mode", &["-###", "-c", "a.c"]);
        succeeds(&output);
        assert!(
            stderr(&output).ends_with(&line(&format!(
                "\"--compiler-extensions\" \"--compiler-identity=clang\" \
                 \"--diagnostic-format=clang\" \"--target={triple}\" \"--driver-name=clang\" \
                 \"a.c\" \"-c\""
            ))),
            "{}",
            stderr(&output)
        );
        assert!(!directory.path("a.o").exists());
        let output = directory.bcc("--gcc-mode", &["-v", "-c", "a.c"]);
        succeeds(&output);
        let text = stderr(&output);
        assert!(
            text.starts_with(&format!("Target: {gcc_triple}\n")),
            "{text}"
        );
        // GCC's `-v` quotes nothing, its `-###` only what the shell needs.
        assert!(
            text.contains(&format!(
                " {exe} -v --compiler-extensions --compiler-identity=gcc --diagnostic-format=gcc \
                 --target={triple} --driver-name=gcc a.c -c\n"
            )),
            "{text}"
        );
        assert!(
            text.contains(
                "#include \"...\" search starts here:\n#include <...> search starts here:\n"
            ),
            "{text}"
        );
        assert!(text.ends_with("End of search list.\n"), "{text}");
        assert!(directory.path("a.o").exists());
        // bcc's own `-v` prints the commands it runs.
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            let output = directory.bcc("", &["-v", "a.c", "b.c", "-o", "ab"]);
            succeeds(&output);
            let text = stderr(&output);
            let clang = text
                .lines()
                .find(|printed| printed.contains("\"-fuse-ld=lld\""))
                .unwrap_or_else(|| panic!("{text}"));
            assert!(clang.starts_with(" \""), "{clang}");
            assert!(clang.contains("\"-o\" \"ab\""), "{clang}");
        }
    }

    /// `-H` prints each header an `#include` enters, after a dot per level
    /// of nesting, and leaves out an inclusion whose include guard is
    /// already defined, as GCC and Clang do; Clang names a header beside
    /// the main file `./a.h`. `cl /showIncludes` indents by the nesting.
    #[test]
    fn header_tree_follows_each_compiler() {
        let directory = Directory::new();
        directory.write(
            "guarded.h",
            "/* c */\n#ifndef G\n#define G\n#include \"b.h\"\n#endif\n",
        );
        directory.write("defined.h", "#if !defined(D)\n#define D\n#endif\n\n");
        directory.write("open.h", "#ifndef O\n#define O\n#endif\nint o;\n");
        directory.write("b.h", "");
        directory.write(
            "h.c",
            "#include \"guarded.h\"\n#include \"guarded.h\"\n#include \"defined.h\"\n#include \
             \"defined.h\"\n#include \"open.h\"\n#include \"open.h\"\n#include \"b.h\"\n",
        );
        let output = directory.bcc("--gcc-mode", &["-H", "-fsyntax-only", "h.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            ". guarded.h\n.. b.h\n. defined.h\n. open.h\n. open.h\n. b.h\n"
        );
        let output = directory.bcc("--clang-mode", &["-H", "-fsyntax-only", "h.c"]);
        succeeds(&output);
        let here = if cfg!(windows) { r".\\" } else { "./" };
        assert_eq!(
            stderr(&output),
            format!(
                ". {here}guarded.h\n.. {here}b.h\n. {here}defined.h\n. {here}open.h\n. \
                 {here}open.h\n. {here}b.h\n"
            )
        );
        let output = directory.bcc("--msvc-mode", &["/nologo", "/Zs", "/showIncludes", "h.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "Note: including file: guarded.h\nNote: including file:  b.h\nNote: including file: \
             defined.h\nNote: including file: open.h\nNote: including file: open.h\nNote: \
             including file: b.h\n"
        );
    }

    /// `-MG` takes a header that cannot be found to be generated: the
    /// `-M` and `-MM` rules list it as written, in order, without an error.
    /// GCC's `-MM` leaves out a `<...>` one; Clang lists it. Without `-M`
    /// or `-MM` each compiler refuses it.
    #[test]
    fn missing_headers_are_generated_under_mg() {
        let directory = Directory::new();
        directory.write("a.h", "#include \"b.h\"\n");
        directory.write("b.h", "int b;\n");
        directory.write(
            "g.c",
            "#include \"gen.h\"\n#include <missing/sys.h>\n#include \"a.h\"\nint y;\n",
        );
        // GCC first reads glibc's `<stdc-predef.h>`, which `-M` lists.
        let predef = if cfg!(all(target_os = "linux", target_env = "gnu"))
            && Path::new("/usr/include/stdc-predef.h").is_file()
        {
            " /usr/include/stdc-predef.h"
        } else {
            ""
        };
        let gcc_all = format!("g.o: g.c{predef} gen.h missing/sys.h a.h b.h\n");
        for (mode, option, expected) in [
            ("--gcc-mode", "-M", gcc_all.as_str()),
            ("--gcc-mode", "-MM", "g.o: g.c gen.h a.h b.h\n"),
            (
                "--clang-mode",
                "-M",
                "g.o: g.c gen.h missing/sys.h a.h b.h\n",
            ),
            (
                "--clang-mode",
                "-MM",
                "g.o: g.c gen.h missing/sys.h a.h b.h\n",
            ),
        ] {
            let output = directory.bcc(mode, &[option, "-MG", "g.c"]);
            succeeds(&output);
            assert_eq!(stdout(&output), expected, "{mode} {option}");
            assert_eq!(stderr(&output), "", "{mode} {option}");
        }
        for (mode, expected) in [
            (
                "--gcc-mode",
                "gcc: error: '-MG' may only be used with '-M' or '-MM'\n",
            ),
            (
                "--clang-mode",
                "clang: error: option '-MG' requires '-M' or '-MM'\n",
            ),
        ] {
            let output = directory.bcc(mode, &["-MD", "-MG", "-c", "g.c"]);
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(stderr(&output), expected, "{mode}");
        }
        // Without `-MG` the missing header is an error.
        let output = directory.bcc("--gcc-mode", &["-M", "g.c"]);
        assert_eq!(output.status.code(), Some(1));
    }

    /// C23's `<float.h>` additions (`FLT_NORM_MAX`, `FLT_SNAN`, `INFINITY`,
    /// `NAN`), with GCC's `FLT_IS_IEC_60559` family and
    /// `__STDC_VERSION_FLOAT_H__` for the GCC identity; Clang's header also
    /// gives them outside strict ISO modes, GCC's only in C23. The values
    /// are GCC 15's and the pinned Clang's, with `<math.h>` before or after.
    #[test]
    fn c23_float_h_follows_each_compiler() {
        let directory = Directory::new();
        let program = "#include <stdio.h>\nint main(void) {\n  printf(\"%a %a %La %d %d %d \
                       %d\\n\", FLT_NORM_MAX, DBL_NORM_MAX, LDBL_NORM_MAX, isnan(FLT_SNAN), \
                       isnan(LDBL_SNAN), isinf(INFINITY), isnan(NAN));\n#ifdef FLT_IS_IEC_60559\n  \
                       printf(\"%d %d %d %ld\\n\", FLT_IS_IEC_60559, DBL_IS_IEC_60559, \
                       LDBL_IS_IEC_60559, (long)__STDC_VERSION_FLOAT_H__);\n#endif\n  return \
                       0;\n}\n";
        directory.write(
            "first.c",
            &format!("#include <float.h>\n#include <math.h>\n{program}"),
        );
        directory.write(
            "last.c",
            &format!("#include <math.h>\n#include <float.h>\n{program}"),
        );
        let values = "0x1.fffffep+127 0x1.fffffffffffffp+1023 0xf.fffffffffffffffp+16380 1 1 1 1\n";
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            for (mode, expected) in [
                ("--gcc-mode", format!("{values}1 1 1 202311\n")),
                ("--clang-mode", values.to_owned()),
            ] {
                for source in ["first.c", "last.c"] {
                    let output =
                        directory.bcc(mode, &["-std=c23", "-Wall", "-Werror", source, "-o", "f"]);
                    succeeds(&output);
                    let run = Command::new(directory.path("f")).output().unwrap();
                    assert_eq!(stdout(&run), expected, "{mode} {source}");
                }
            }
        }
        let float_macros = |mode: &str, standard: &str| {
            directory.write("float.c", "#include <float.h>\n");
            let output = directory.bcc(mode, &[standard, "-E", "-dM", "float.c"]);
            succeeds(&output);
            stdout(&output)
        };
        for (mode, standard, defined) in [
            ("--gcc-mode", "-std=gnu17", false),
            ("--gcc-mode", "-std=c23", true),
            ("--clang-mode", "-std=gnu17", true),
            ("--clang-mode", "-std=c17", false),
            ("--clang-mode", "-std=c23", true),
        ] {
            let listing = float_macros(mode, standard);
            for name in ["FLT_SNAN", "LDBL_NORM_MAX", "INFINITY", "NAN"] {
                assert_eq!(
                    defines(&listing, name).is_some(),
                    defined,
                    "{mode} {standard} {name}"
                );
            }
            assert_eq!(
                defines(&listing, "FLT_IS_IEC_60559").is_some(),
                defined && mode == "--gcc-mode",
                "{mode} {standard}"
            );
        }
        // C99's DECIMAL_DIG and C11's FLT_TRUE_MIN, which Clang's header
        // gives in GNU89 too.
        for (mode, defined) in [("--gcc-mode", false), ("--clang-mode", true)] {
            let listing = float_macros(mode, "-std=gnu89");
            assert_eq!(
                defines(&listing, "DECIMAL_DIG").is_some(),
                defined,
                "{mode}"
            );
            assert_eq!(
                defines(&listing, "FLT_TRUE_MIN").is_some(),
                defined,
                "{mode}"
            );
            assert!(defines(&listing, "FLT_EVAL_METHOD").is_none(), "{mode}");
        }
    }

    /// `-fno-delete-null-pointer-checks` marks every function
    /// `null_pointer_is_valid`, as Clang does, so LLVM keeps a null check
    /// after a dereference; `-fwrapv-pointer` and `-fno-strict-overflow`
    /// drop `inbounds` from pointer arithmetic, so it wraps; the last flag
    /// of each pair wins, and every personality passes them on.
    #[test]
    fn pointer_assumptions_reach_llvm() {
        let directory = Directory::new();
        directory.write(
            "p.c",
            "int f(int *p) { int x = *p; if (!p) return -1; return x; }\nchar *h(char *p, long n) \
             { return p + n; }\n",
        );
        let llvm = |options: &[&str]| {
            let mut args = vec!["--emit=llvm"];
            args.extend(options);
            args.push("p.c");
            let output = directory.bcc("", &args);
            succeeds(&output);
            stdout(&output)
        };
        let plain = llvm(&[]);
        assert!(!plain.contains("null_pointer_is_valid"), "{plain}");
        assert!(plain.contains("getelementptr inbounds i8"), "{plain}");
        let null = llvm(&["-fno-delete-null-pointer-checks"]);
        assert!(
            null.contains("attributes #0 = { null_pointer_is_valid }"),
            "{null}"
        );
        let restored = llvm(&[
            "-fno-delete-null-pointer-checks",
            "-fdelete-null-pointer-checks",
        ]);
        assert!(!restored.contains("null_pointer_is_valid"), "{restored}");
        for options in [&["-fwrapv-pointer"][..], &["-fno-strict-overflow"]] {
            let wrapping = llvm(options);
            assert!(
                wrapping.contains("getelementptr i8"),
                "{options:?}: {wrapping}"
            );
            assert!(!wrapping.contains("inbounds"), "{options:?}: {wrapping}");
        }
        let strict = llvm(&["-fno-strict-overflow", "-fstrict-overflow"]);
        assert!(strict.contains("getelementptr inbounds i8"), "{strict}");
        // `-fno-strict-overflow` implies `-fwrapv`, as in GCC.
        directory.write("w.c", "int w(int a) { return a + 1; }\n");
        let output = directory.bcc("", &["--emit=llvm", "-fno-strict-overflow", "w.c"]);
        succeeds(&output);
        assert!(!stdout(&output).contains("add nsw"), "{}", stdout(&output));
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let output = directory.bcc(
                mode,
                &[
                    "-c",
                    "-fno-delete-null-pointer-checks",
                    "-fwrapv-pointer",
                    "-fno-strict-overflow",
                    "p.c",
                ],
            );
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{mode}");
        }
    }

    /// The memory accesses `function` makes through a pointer, in AT&T syntax,
    /// as `l` or `s` and a width: not those of its own frame.
    fn memory_accesses(assembly: &str, function: &str) -> Vec<String> {
        let label = format!("{function}:");
        let mut accesses = Vec::new();
        for line in assembly
            .lines()
            .skip_while(|line| *line != label)
            .skip(1)
            .take_while(|line| !line.contains(".cfi_endproc") && !line.contains(".size"))
        {
            let mut words = line.split_whitespace();
            let (Some(mnemonic), operands) = (words.next(), words.collect::<Vec<_>>().join(" "))
            else {
                continue;
            };
            let frame = operands.contains("(%rbp") || operands.contains("(%rsp");
            if !operands.contains('(') || frame || mnemonic.starts_with("lea") {
                continue;
            }
            let width = if let Some(extending) = mnemonic
                .strip_prefix("movz")
                .or_else(|| mnemonic.strip_prefix("movs"))
            {
                extending.chars().next()
            } else {
                mnemonic.chars().last()
            };
            let width = match width {
                | Some('b') => 8,
                | Some('w') => 16,
                | Some('l') => 32,
                | Some('q') => 64,
                | _ => panic!("{line}"),
            };
            let destination = operands
                .rsplit(' ')
                .next()
                .is_some_and(|last| last.contains('('));
            if !destination {
                accesses.push(format!("l{width}"));
            } else if mnemonic.starts_with("mov") {
                accesses.push(format!("s{width}"));
            } else {
                accesses.push(format!("l{width}"));
                accesses.push(format!("s{width}"));
            }
        }
        accesses
    }

    /// `-fstrict-volatile-bitfields` makes each back end read and write a
    /// `volatile` bit-field with one access of its declared type: GCC's
    /// option, off by default and the last of the pair winning, which Clang
    /// does not know and TCC ignores. The widths are what GCC 14 emits.
    #[test]
    fn strict_volatile_bitfields_set_the_access_width() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "w.c",
            "struct S { volatile unsigned a : 8, b : 8, c : 16; };\nint r(struct S *p) { return \
             p->b; }\nvoid w(struct S *p) { p->b = 3; }\n",
        );
        let assembly = |mode: &str, options: &[&str]| {
            let mut args = vec!["-O2", "-S", "-o", "-", "w.c"];
            args.extend(options);
            let output = directory.bcc(mode, &args);
            succeeds(&output);
            stdout(&output)
        };
        let strict = "-fstrict-volatile-bitfields";
        let plain = "-fno-strict-volatile-bitfields";
        for backend in [&[][..], &["-Xbcc", "--backend=native"]] {
            let gcc = |options: &[&str]| assembly("--gcc-mode", &[options, backend].concat());
            let wide = gcc(&[strict]);
            // Two loads, one of which may fold into the mask, and a store.
            let accesses: Vec<&str> = wide.lines().filter(|l| l.contains("(%rdi)")).collect();
            assert_eq!(accesses.len(), 3, "{wide}");
            assert!(
                accesses.iter().all(|line| {
                    let line = line.trim_start();
                    line.contains("\t(%rdi)") || line.ends_with(", (%rdi)")
                }),
                "{wide}"
            );
            assert!(
                accesses.iter().all(|line| line
                    .trim_start()
                    .split('\t')
                    .next()
                    .unwrap()
                    .ends_with('l')
                    && !line.contains("movz")),
                "{wide}"
            );
            for narrow in [gcc(&[]), gcc(&[strict, plain])] {
                assert!(narrow.contains("1(%rdi)"), "{narrow}");
                assert!(!narrow.contains("movl\t(%rdi)"), "{narrow}");
            }
            assert_eq!(gcc(&[plain, strict]), wide);
        }
        let clang = directory.bcc("--clang-mode", &["-c", strict, "w.c"]);
        assert!(!clang.status.success());
        assert!(
            stderr(&clang).contains(&format!("unknown argument: '{strict}'")),
            "{}",
            stderr(&clang)
        );
        // TCC ignores it, and so does clang-cl, with its usual warning.
        let ignored = directory.bcc("--tcc-mode", &["-c", strict, "w.c", "-o", "t.o"]);
        succeeds(&ignored);
        assert_eq!(stderr(&ignored), "");
        succeeds(&directory.bcc("--tcc-mode", &["-c", "w.c", "-o", "plain.o"]));
        assert_eq!(
            std::fs::read(directory.path("t.o")).unwrap(),
            std::fs::read(directory.path("plain.o")).unwrap()
        );
        let clang_cl = directory.bcc("--clang-cl-mode", &["-c", strict, "w.c"]);
        succeeds(&clang_cl);
        assert!(
            stderr(&clang_cl).contains("unknown argument ignored in clang-cl"),
            "{}",
            stderr(&clang_cl)
        );
    }

    /// Where the option's single access does not apply, each back end and
    /// level performs the accesses GCC 13.3 and 14.2 do, which a device sees:
    /// a memory-destination instruction is one read and one write.
    #[test]
    fn strict_volatile_bitfields_fall_back_on_gccs_accesses() {
        const WANTED: &[(&str, &[&str])] = &[
            ("a", &["l16", "s16"]),
            ("ra", &["l32"]),
            ("ca", &["l32", "l16", "s16"]),
            ("b", &["l8", "s8", "l8", "s8"]),
            ("c", &["l16", "s16", "l8", "s8"]),
            ("rd", &["l8", "l8"]),
            ("re", &["l32", "l32"]),
            ("e", &["l32", "s32", "l32", "s32"]),
            ("f", &["s64", "l64", "s64"]),
            ("g", &["l16", "s16"]),
        ];
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "f.c",
            "struct A { volatile unsigned a : 12; };\nvoid a(struct A *p) { p->a = 3; }\nunsigned \
             ra(struct A *p) { return p->a; }\nvoid ca(struct A *p) { p->a += 1; }\nstruct B { \
             char c; volatile unsigned a : 12; };\nvoid b(struct B *p) { p->a = 3; }\nstruct C { \
             volatile unsigned a : 17; char c; };\nvoid c(struct C *p) { p->a = 3; }\nstruct D { \
             volatile unsigned a : 12; } __attribute__((packed));\nunsigned rd(struct D *p) { \
             return p->a; }\nstruct E { char x : 7; volatile unsigned y : 26; } \
             __attribute__((packed, aligned(4)));\nunsigned re(struct E *p) { return p->y; \
             }\nvoid e(struct E *p) { p->y = 3; }\nstruct F { volatile __int128 a : 70; };\nvoid \
             f(struct F *p) { p->a = 3; }\nstruct G { unsigned char b : 8; volatile unsigned a : \
             8; } __attribute__((packed));\nvoid g(struct G *p) { p->a = 3; }\n",
        );
        for backend in [&[][..], &["-Xbcc", "--backend=native"]] {
            for level in ["-O0", "-O2"] {
                let mut args = vec!["-fstrict-volatile-bitfields", level, "-S", "-o", "-", "f.c"];
                args.extend(backend);
                let output = directory.bcc("--gcc-mode", &args);
                succeeds(&output);
                let assembly = stdout(&output);
                for (function, wanted) in WANTED {
                    assert_eq!(
                        &memory_accesses(&assembly, function),
                        wanted,
                        "{function} {level} {backend:?}\n{assembly}"
                    );
                }
            }
        }
        // Without the option the same writes stay in the field's own bytes.
        let output = directory.bcc("--gcc-mode", &["-O2", "-S", "-o", "-", "f.c"]);
        succeeds(&output);
        assert_eq!(memory_accesses(&stdout(&output), "ra"), ["l16"]);
    }

    /// What the back ends cannot make GCC's accesses is a clear error under
    /// the option and compiles without it.
    #[test]
    fn strict_volatile_bitfields_refuse_what_they_cannot_match() {
        let directory = Directory::new();
        directory.write(
            "r.c",
            "struct S { volatile __int128 a : 128; };\nvoid w(struct S *p) { p->a = 3; }\n",
        );
        let strict = directory.bcc("--gcc-mode", &["-fstrict-volatile-bitfields", "-c", "r.c"]);
        assert!(!strict.status.success());
        assert!(
            stderr(&strict).contains("r.c:2:")
                && stderr(&strict).contains("volatile bit-field under -fstrict-volatile-bitfields"),
            "{}",
            stderr(&strict)
        );
        succeeds(&directory.bcc("--gcc-mode", &["-c", "r.c", "-o", "plain.o"]));
    }

    /// A wider access keeps the neighbors' bits and the field's value on
    /// every back end, optimized or not, where GCC's runs agree. Memory that
    /// was never written beside a written field stays out of the result,
    /// though the load covers it.
    #[test]
    fn strict_volatile_bitfields_keep_every_value() {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "v.c",
            "struct T { volatile unsigned a : 8, b : 8, c : 16; };\nstruct N { char c; volatile \
             int a : 8; volatile int b : 3; char d; };\nstruct P { char x; volatile unsigned y : \
             31; } __attribute__((packed));\nunion U { struct T t; unsigned char bytes[4]; \
             };\nstruct T global = {1, 2, 3};\nvolatile struct N neighbors = {1, -2, -3, \
             4};\nstruct P packed;\nint check(void) {\n  /* Each field beside its neighbors. */\n  \
             global.b = 0xab;\n  if (global.a != 1 || global.b != 0xab || global.c != 3) return \
             1;\n  if ((global.c = 0x12345) != 0x2345 || global.a != 1) return 2;\n  \
             global.a++;\n  if (global.a != 2 || global.b != 0xab) return 3;\n  /* The unit \
             starts before the field; signed values extend. */\n  if (neighbors.a != -2 || \
             neighbors.b != -3) return 4;\n  neighbors.a = 5;\n  if (neighbors.c != 1 || \
             neighbors.a != 5 || neighbors.b != -3 || neighbors.d != 4) return 5;\n  if \
             ((neighbors.b = 7) != -1) return 6;\n  /* A packed field keeps its ordinary access. \
             */\n  packed.y = 0x7fffffff;\n  if (packed.y != 0x7fffffff || packed.x != 0) return \
             7;\n  /* An automatic object with bytes nobody wrote. */\n  struct { volatile \
             unsigned a : 8; char c; } local;\n  local.a = 0x5a;\n  if (local.a != 0x5a) return \
             8;\n  union U u;\n  u.bytes[0] = 7;\n  u.t.b = 9;\n  if (u.bytes[0] != 7 || \
             u.bytes[1] != 9 || u.t.b != 9) return 9;\n  return 0;\n}\nint main(void) { return \
             check(); }\n",
        );
        let strict = "-fstrict-volatile-bitfields";
        if Command::new("gcc").arg("--version").output().is_ok() {
            let built = Command::new("gcc")
                .args(["-O2", strict, "v.c", "-o", "reference"])
                .current_dir(&directory.0)
                .output()
                .unwrap();
            succeeds(&built);
            let run = Command::new(directory.path("reference")).output().unwrap();
            assert_eq!(run.status.code(), Some(0), "gcc");
        }
        for options in [
            &["-O0"][..],
            &["-O2"],
            &["--opt=bcc"],
            &["--backend=native", "-O0"],
            &["--backend=native", "-O2"],
        ] {
            let mut args = vec![strict, "v.c", "-o", "program"];
            args.extend(options);
            succeeds(&directory.bcc("", &args));
            let run = Command::new(directory.path("program")).output().unwrap();
            assert_eq!(run.status.code(), Some(0), "{options:?}");
        }
        for options in [&["-O0"][..], &["-O2"]] {
            let mut args = vec![strict, "--interpret", "v.c"];
            args.extend(options);
            let run = directory.bcc("", &args);
            assert_eq!(run.status.code(), Some(0), "interpreter {options:?}");
        }
        if !Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            eprintln!("skipping wasm32: node is not on the path");
            return;
        }
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs");
        for options in [&["-O0"][..], &["-O2"]] {
            let mut args = vec![strict, "--target=wasm32-wasip1", "v.c", "-o", "v.wasm"];
            args.extend(options);
            succeeds(&directory.bcc("", &args));
            let run = Command::new("node")
                .arg("--no-warnings")
                .arg(&runner)
                .arg(directory.path("v.wasm"))
                .output()
                .unwrap();
            assert_eq!(run.status.code(), Some(0), "wasm32 {options:?}");
        }
    }

    /// `-fcommon` makes an external tentative definition a common symbol on
    /// the LLVM and native back ends, so two units' `int x;` link, and one
    /// links with GCC's `-fcommon` objects, an initialized definition
    /// winning; without it (the default, as in GCC 10 and Clang 11) they
    /// clash. Thread-local objects and objects in a section stay ordinary
    /// definitions, as in Clang.
    #[test]
    fn common_symbols_link_as_gcc_links_them() {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "a.c",
            "int x;\nint arr[10];\n__thread int t;\n__attribute__((section(\".mysec\"))) int \
             sec;\nint get(void) { return x + arr[2] + t + sec; }\n",
        );
        directory.write(
            "b.c",
            "#include <stdio.h>\nint x;\nint arr[10];\nint get(void);\nint main(void) {\n  arr[2] \
             = 5;\n  printf(\"%d\\n\", get());\n  return 0;\n}\n",
        );
        directory.write("init.c", "int x = 7;\nint arr[20];\n");
        for backend in ["--backend=llvm", "--backend=native"] {
            for (source, object) in [("a.c", "a.o"), ("b.c", "b.o")] {
                let output = directory.bcc("", &[backend, "-fcommon", "-c", source, "-o", object]);
                succeeds(&output);
            }
            let symbols = Command::new("nm").arg(directory.path("a.o")).output();
            if let Ok(symbols) = symbols {
                let symbols = stdout(&symbols);
                for common in [" C x\n", " C arr\n"] {
                    assert!(symbols.contains(common), "{backend}: {symbols}");
                }
                assert!(!symbols.contains(" C t\n"), "{backend}: {symbols}");
                assert!(!symbols.contains(" C sec\n"), "{backend}: {symbols}");
            }
            let output = directory.bcc("", &["a.o", "b.o", "-o", "ab"]);
            succeeds(&output);
            let run = Command::new(directory.path("ab")).output().unwrap();
            assert_eq!(stdout(&run), "5\n", "{backend}");
            if Command::new("gcc").arg("--version").output().is_ok() {
                let gcc = Command::new("gcc")
                    .args(["-fcommon", "-c", "init.c", "-o", "init.o"])
                    .current_dir(&directory.0)
                    .status()
                    .unwrap();
                assert!(gcc.success());
                let output = directory.bcc("", &["a.o", "b.o", "init.o", "-o", "abi"]);
                succeeds(&output);
                let run = Command::new(directory.path("abi")).output().unwrap();
                assert_eq!(stdout(&run), "12\n", "{backend}");
            }
        }
        // `-fno-common`, the default, defines `x` in each unit.
        for (source, object) in [("a.c", "a.o"), ("b.c", "b.o")] {
            let output =
                directory.bcc("", &["-fcommon", "-fno-common", "-c", source, "-o", object]);
            succeeds(&output);
        }
        let output = directory.bcc("", &["a.o", "b.o", "-o", "ab"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("duplicate symbol: x"),
            "{}",
            stderr(&output)
        );
        // The personalities pass both on.
        let output = directory.bcc("--gcc-mode", &["-fcommon", "a.c", "b.c", "-o", "gab"]);
        succeeds(&output);
        assert_eq!(stderr(&output), "");
    }

    /// GCC's `-###` quotes a word only when it has a character other than a
    /// letter, digit, `_`, `/`, `-` or `.`, escaping `"`, `\` and `$`.
    #[test]
    fn gcc_dry_run_quotes_as_gcc_does() {
        let directory = Directory::new();
        directory.write("a b.c", "int a;\n");
        let output = directory.bcc(
            "--gcc-mode",
            &["-###", "-c", "a b.c", "-DX=\"y z\"", "-o", "o$.o"],
        );
        succeeds(&output);
        let text = stderr(&output);
        assert!(
            text.ends_with(&format!(
                " --compiler-extensions \"--compiler-identity=gcc\" \"--diagnostic-format=gcc\" \
                 \"--target={}\" \"--driver-name=gcc\" \"a b.c\" -D \"X=\\\"y z\\\"\" -c \
                 \"--output=o\\$.o\"\n",
                HOST_TARGET.0
            )),
            "{text}"
        );
    }

    /// Clang reports every unused linker input, but no unused argument once
    /// an error has occurred, a `-Werror` one included; `-Qunused-arguments`
    /// silences both, and GCC refuses it.
    #[test]
    fn clang_stops_reporting_unused_arguments_after_an_error() {
        let directory = Directory::new();
        directory.write("a.c", "int a;\n");
        let output = directory.bcc(
            "--clang-mode",
            &[
                "-c", "a.c", "-o", "a.o", "-Werror", "-lm", "-Wl,x", "-L/tmp", "-s",
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "clang: error: -lm: 'linker' input unused \
             [-Werror,-Wunused-command-line-argument]\nclang: error: -Wl,x: 'linker' input unused \
             [-Werror,-Wunused-command-line-argument]\n"
        );
        for options in [
            &[
                "-Qunused-arguments",
                "-c",
                "a.c",
                "-o",
                "a.o",
                "-lm",
                "-L/tmp",
                "-s",
            ][..],
            &["-Qunused-arguments", "-fsyntax-only", "-c", "a.c"],
        ] {
            let output = directory.bcc("--clang-mode", options);
            succeeds(&output);
            assert_eq!(stderr(&output), "", "{options:?}");
        }
        // `-Werror=` and `-Wno-error=` name the driver's warning group, which
        // Clang knows, so they are no unknown warning options.
        let output = directory.bcc(
            "--clang-mode",
            &[
                "-Werror",
                "-Werror=unused-command-line-argument",
                "-c",
                "a.c",
                "-o",
                "a.o",
            ],
        );
        succeeds(&output);
        assert_eq!(stderr(&output), "");
        let output = directory.bcc(
            "--clang-mode",
            &[
                "-Werror",
                "-Wno-error=unused-command-line-argument",
                "-c",
                "a.c",
                "-lm",
            ],
        );
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            "clang: warning: -lm: 'linker' input unused [-Wunused-command-line-argument]\n"
        );
        let output = directory.bcc("--gcc-mode", &["-Qunused-arguments", "-c", "a.c"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            "gcc: error: unrecognized command-line option '-Qunused-arguments'\n"
        );
    }

    /// Clang begins every path it reaches from a main file in the working
    /// directory through its includers' directories with `./`, in `-H`
    /// and clang-cl's `/showIncludes`; a header found through `-I` keeps
    /// its directory's spelling. Clang joins with the host's separator,
    /// so `.\sub/s.h` and `.\sub\../d.h` on Windows, and `-H` escapes
    /// each backslash; GCC joins with `/`.
    #[test]
    fn clang_names_working_directory_headers_with_dot() {
        let directory = Directory::new();
        std::fs::create_dir(directory.path("sub")).unwrap();
        std::fs::create_dir(directory.path("inc")).unwrap();
        directory.write(
            "m.c",
            "#include \"sub/s.h\"\n#include \"a.h\"\n#include \"inc/i.h\"\n#include <j.h>\n",
        );
        directory.write("sub/s.h", "#include \"../d.h\"\n#include \"t.h\"\n");
        directory.write("sub/t.h", "int t;\n");
        directory.write("d.h", "int d;\n");
        directory.write("a.h", "int a;\n");
        directory.write("inc/i.h", "#include \"k.h\"\n");
        directory.write("inc/k.h", "int k;\n");
        directory.write("inc/j.h", "int j;\n");
        let output = directory.bcc("--clang-mode", &["-H", "-fsyntax-only", "-Iinc", "m.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            // As in a string literal (`Lexer::Stringify`).
            if cfg!(windows) {
                concat!(
                    r". .\\sub/s.h",
                    "\n",
                    r".. .\\sub\\../d.h",
                    "\n",
                    r".. .\\sub\\t.h",
                    "\n",
                    r". .\\a.h",
                    "\n",
                    r". .\\inc/i.h",
                    "\n",
                    r".. .\\inc\\k.h",
                    "\n",
                    r". inc\\j.h",
                    "\n"
                )
            } else {
                ". ./sub/s.h\n.. ./sub/../d.h\n.. ./sub/t.h\n. ./a.h\n. ./inc/i.h\n.. ./inc/k.h\n. \
                 inc/j.h\n"
            }
        );
        // GCC joins with `/` on every host.
        let output = directory.bcc("--gcc-mode", &["-H", "-fsyntax-only", "-Iinc", "m.c"]);
        succeeds(&output);
        assert_eq!(
            stderr(&output),
            ". sub/s.h\n.. sub/../d.h\n.. sub/t.h\n. a.h\n. inc/i.h\n.. inc/k.h\n. inc/j.h\n"
        );
        let output = directory.bcc(
            "--clang-cl-mode",
            &["/nologo", "/Zs", "/showIncludes", "/Iinc", "m.c"],
        );
        succeeds(&output);
        assert!(
            stderr(&output).starts_with(if cfg!(windows) {
                "Note: including file: .\\sub/s.h\nNote: including file:  .\\sub\\../d.h\n"
            } else {
                "Note: including file: ./sub/s.h\nNote: including file:  ./sub/../d.h\n"
            }),
            "{}",
            stderr(&output)
        );
    }

    /// The common symbols an object file defines, by `nm`'s `C` letter.
    fn common_symbols(object: &Path) -> Option<Vec<String>> {
        let listing = Command::new("nm").arg(object).output().ok()?;
        Some(
            stdout(&listing)
                .lines()
                .filter_map(|line| {
                    let mut words = line.split_whitespace().rev();
                    let name = words.next()?;
                    (words.next()? == "C").then(|| name.to_owned())
                })
                .collect(),
        )
    }

    /// A `const` tentative definition is common, and so not constant, as in
    /// Clang; GNU `common` makes an object common under `-fno-common` and
    /// `nocommon` keeps one a definition under `-fcommon`, and `common` on
    /// any declaration wins over `nocommon` without a warning, as in Clang
    /// (`nocommon` on an `extern` declaration still counts), on both back
    /// ends.
    #[test]
    fn common_attributes_and_constant_objects() {
        if !cfg!(all(target_os = "linux", target_env = "gnu")) {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "c.c",
            "const int cint;\n__attribute__((common)) int ca;\n__attribute__((nocommon)) int \
             na;\n__attribute__((nocommon)) int last;\n__attribute__((common)) int last;\nint \
             pa;\nint __attribute__((common, nocommon)) at_cn;\nint __attribute__((nocommon, \
             common)) at_nc;\nint redecl1 __attribute__((common));\nint redecl1 \
             __attribute__((nocommon));\nextern int redecl3 __attribute__((nocommon));\nint \
             redecl3;\n",
        );
        for backend in ["--backend=llvm", "--backend=native"] {
            for (flag, expected) in [
                (
                    "-fcommon",
                    &["cint", "ca", "last", "pa", "at_cn", "at_nc", "redecl1"][..],
                ),
                ("-fno-common", &["ca", "last", "at_cn", "at_nc", "redecl1"]),
            ] {
                let output = directory.bcc("", &[backend, flag, "-c", "c.c", "-o", "c.o"]);
                succeeds(&output);
                assert_eq!(stderr(&output), "", "{backend} {flag}");
                let Some(mut symbols) = common_symbols(&directory.path("c.o")) else {
                    return;
                };
                symbols.sort();
                let mut expected = expected.to_vec();
                expected.sort_unstable();
                assert_eq!(symbols, expected, "{backend} {flag}");
            }
        }
    }

    /// On the Windows targets common symbols are COFF communal symbols
    /// (section 0, the size as the value), as Clang emits them: on MinGW
    /// every tentative definition, with `-aligncomm` above a byte and a
    /// `dllexport` one exported; on the MSVC target not one whose alignment
    /// its declaration, typedef, record or a member requires, since the
    /// linker aligns a communal symbol by its size alone.
    #[test]
    fn coff_common_symbols_follow_clang() {
        let directory = Directory::new();
        directory.write(
            "w.c",
            "struct B { char q[64]; } __attribute__((aligned(64))) b64;\ntypedef int ai16 \
             __attribute__((aligned(16)));\nai16 ai;\ntypedef long long al8 \
             __attribute__((aligned(8)));\nal8 aarr[4];\nstruct F { int x \
             __attribute__((aligned(8))); } f;\nstruct G { ai16 y; } g;\nstruct H { struct F f; } \
             h;\nstruct K { struct B b; } k;\n__declspec(dllexport) int dexp;\nchar onebyte;\nint \
             plain;\n__attribute__((aligned(16))) int own;\ntypedef float v16 \
             __attribute__((vector_size(64)));\nv16 vec64;\ntypedef float v8 \
             __attribute__((vector_size(32)));\nv8 vec32;\nstruct SV { v16 x; } sv;\nv16 \
             varr[2];\nshort s2;\nchar c3[3];\n",
        );
        let communal = |object: &Path| -> Option<Vec<String>> {
            let listing = Command::new("objdump")
                .arg("-t")
                .arg(object)
                .output()
                .ok()?;
            if !listing.status.success() {
                return None;
            }
            let mut names: Vec<String> = stdout(&listing)
                .lines()
                .filter(|line| line.contains("(sec  0)") && line.contains("(scl   2)"))
                .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
                .filter(|name| !name.starts_with('_') && !name.starts_with('.'))
                .collect();
            names.sort();
            Some(names)
        };
        let directives = |object: &Path| -> String {
            Command::new("objdump")
                .args(["-s", "-j", ".drectve"])
                .arg(object)
                .output()
                .map(|output| {
                    stdout(&output)
                        .lines()
                        .filter(|line| line.starts_with(' '))
                        .map(|line| line.get(43..).unwrap_or(""))
                        .collect()
                })
                .unwrap_or_default()
        };
        for (target, expected) in [
            (
                "x86_64-w64-windows-gnu",
                &[
                    "aarr", "ai", "b64", "c3", "dexp", "f", "g", "h", "k", "onebyte", "own",
                    "plain", "s2", "sv", "varr", "vec32", "vec64",
                ][..],
            ),
            // A 32-byte vector stays common there; a 64-byte one does not.
            (
                "x86_64-pc-windows-msvc",
                &["c3", "dexp", "h", "onebyte", "plain", "s2", "vec32"],
            ),
        ] {
            for backend in ["--backend=llvm", "--backend=native"] {
                let output = directory.bcc(
                    "",
                    &[
                        &format!("--target={target}"),
                        backend,
                        "-fms-extensions",
                        "-fcommon",
                        "-c",
                        "w.c",
                        "-o",
                        "w.obj",
                    ],
                );
                succeeds(&output);
                let Some(names) = communal(&directory.path("w.obj")) else {
                    return;
                };
                assert_eq!(names, expected, "{target} {backend}");
                // Microsoft's linker aligns a communal symbol by its size,
                // so it is rounded up to the alignment there, as LLVM does.
                let size = if target.ends_with("msvc") { "4" } else { "" };
                let listing = Command::new("objdump")
                    .arg("-t")
                    .arg(directory.path("w.obj"))
                    .output()
                    .unwrap();
                for name in ["s2", "c3"] {
                    let line = stdout(&listing)
                        .lines()
                        .find(|line| line.ends_with(&format!(" {name}")))
                        .map(str::to_owned)
                        .unwrap_or_default();
                    let expected = match (name, size) {
                        | ("s2", "") => "0x0000000000000002",
                        | ("c3", "") => "0x0000000000000003",
                        | _ => "0x0000000000000004",
                    };
                    assert!(line.contains(expected), "{target} {backend}: {line}");
                }
                let drectve = directives(&directory.path("w.obj"));
                if target.ends_with("gnu") {
                    assert!(
                        drectve.contains("-export:dexp,data"),
                        "{backend}: {drectve}"
                    );
                    assert!(drectve.contains("-aligncomm:"), "{backend}: {drectve}");
                    assert!(!drectve.contains("onebyte"), "{backend}: {drectve}");
                } else {
                    assert!(
                        drectve.contains("/EXPORT:dexp,DATA"),
                        "{backend}: {drectve}"
                    );
                }
            }
        }
    }

    /// Where `long double` is binary128 (WebAssembly), its NaN and infinity
    /// builtins fold to binary128 encodings, as Clang folds them, so C23's
    /// `LDBL_SNAN` initializes a static object.
    #[test]
    fn long_double_constants_follow_the_format() {
        let directory = Directory::new();
        directory.write(
            "l.c",
            "long double s = __builtin_nansl(\"\");\nlong double p = \
             __builtin_nansl(\"0x7\");\nlong double q = __builtin_nanl(\"\");\nlong double i = \
             __builtin_infl();\nlong double h = __builtin_huge_vall();\n",
        );
        for (target, expected) in [
            (
                "wasm32-unknown-unknown",
                [
                    "0000000000000000000000000040ff7f",
                    "0700000000000000000000000000ff7f",
                    "0000000000000000000000000080ff7f",
                    "0000000000000000000000000000ff7f",
                    "0000000000000000000000000000ff7f",
                ],
            ),
            (
                "x86_64-unknown-linux-gnu",
                [
                    "00000000000000a0ff7f000000000000",
                    "0700000000000080ff7f000000000000",
                    "00000000000000c0ff7f000000000000",
                    "0000000000000080ff7f000000000000",
                    "0000000000000080ff7f000000000000",
                ],
            ),
        ] {
            let output = directory.bcc("", &[&format!("--target={target}"), "--emit=ir", "l.c"]);
            succeeds(&output);
            let ir = stdout(&output);
            for (name, bytes) in ["s", "p", "q", "i", "h"].into_iter().zip(expected) {
                assert!(
                    ir.contains(&format!(
                        "global @{name} external size 16, align 16 = bytes \"{bytes}\""
                    )),
                    "{target} {name}: {ir}"
                );
            }
        }
    }

    /// GCC's `CR_DECIMAL_DIG` comes with the IEC 60559 extension macros
    /// under its identity; Clang's header has none.
    #[test]
    fn cr_decimal_dig_follows_gcc() {
        let directory = Directory::new();
        directory.write("float.c", "#include <float.h>\n");
        for (mode, defined) in [("--gcc-mode", true), ("--clang-mode", false)] {
            for want in [
                "-D__STDC_WANT_IEC_60559_EXT__",
                "-D__STDC_WANT_IEC_60559_BFP_EXT__",
            ] {
                let output = directory.bcc(mode, &[want, "-E", "-dM", "float.c"]);
                succeeds(&output);
                assert_eq!(
                    defines(&stdout(&output), "CR_DECIMAL_DIG").as_deref(),
                    defined.then_some("__UINTMAX_MAX__"),
                    "{mode} {want}"
                );
            }
            let output = directory.bcc(mode, &["-E", "-dM", "float.c"]);
            assert!(
                defines(&stdout(&output), "CR_DECIMAL_DIG").is_none(),
                "{mode}"
            );
        }
    }

    /// TCC's `libtcc1.a` defines the `<stdatomic.h>` fences and flag
    /// operations, so a TCC program may take their addresses (tcc's
    /// `tests2/151_atomic_header.c`); under `--tcc-mode` each unit carries
    /// the ones it references as weak definitions, which a program's own
    /// definition replaces. GCC and Clang leave them to libatomic.
    #[cfg(target_os = "linux")]
    #[test]
    fn tcc_programs_reach_the_libtcc1_atomic_functions() {
        let directory = Directory::new();
        directory.write(
            "flags.c",
            "#include <stdatomic.h>\n#include <stdio.h>\nint main(void) {\n  atomic_flag f = \
             ATOMIC_FLAG_INIT;\n  _Bool (*set)(volatile atomic_flag *) = \
             &atomic_flag_test_and_set;\n  void (*clear)(volatile atomic_flag *, memory_order) = \
             &atomic_flag_clear_explicit;\n  void (*thread)(memory_order) = \
             &atomic_thread_fence;\n  void (*signal)(memory_order) = &atomic_signal_fence;\n  int \
             a = set(&f), b = set(&f);\n  clear(&f, memory_order_release);\n  int c = \
             (atomic_flag_test_and_set)(&f);\n  thread(memory_order_seq_cst);\n  \
             signal(memory_order_acquire);\n  printf(\"%d %d %d\\n\", a, b, c);\n  return 0;\n}\n",
        );
        succeeds(&directory.bcc("--tcc-mode", &["flags.c", "-o", "flags.exe"]));
        let run = Command::new(directory.path("flags.exe")).output().unwrap();
        assert_eq!(stdout(&run), "0 1 0\n");
        directory.write(
            "use.c",
            "#include <stdatomic.h>\nint main(void) {\n  void (*p)(memory_order) = \
             atomic_signal_fence;\n  p(memory_order_acquire);\n  \
             (atomic_signal_fence)(memory_order_release);\n  return 0;\n}\n",
        );
        directory.write(
            "own.c",
            "#include <stdio.h>\nvoid atomic_signal_fence(int order) { printf(\"own %d\\n\", \
             order); }\n",
        );
        succeeds(&directory.bcc("--tcc-mode", &["use.c", "own.c", "-o", "own.exe"]));
        let run = Command::new(directory.path("own.exe")).output().unwrap();
        assert_eq!(stdout(&run), "own 2\nown 3\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["use.c", "-o", "use.exe"]);
            assert!(!output.status.success(), "{mode}");
            assert!(stderr(&output).contains("atomic_signal_fence"), "{mode}");
        }
    }
}
