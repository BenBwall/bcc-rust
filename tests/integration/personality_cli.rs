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
            assert!(text.starts_with("install: "), "{mode}: {text}");
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
            file("libgcc.a")
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
        for line in [
            "gcc: warning: '-fstack-protector-strong' is ignored: bcc does not emit stack \
             protectors",
            "gcc: warning: '-fstack-clash-protection' is ignored",
            "gcc: warning: '-fcf-protection' is ignored",
        ] {
            assert!(warnings.contains(line), "{warnings}");
        }
        assert_eq!(warnings.lines().count(), 3, "{warnings}");
        // TCC ignores the options it does not know, so they change nothing
        // in the program it builds either.
        let tcc = directory.bcc(
            "--tcc-mode",
            &["-c", "-fstack-protector", "-fshort-enums", "x.c"],
        );
        succeeds(&tcc);
        assert_eq!(stderr(&tcc), "");
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
        // A newer processor warns: baseline code runs on it.
        for option in ["-march=native", "-march=x86-64-v3", "-mavx2"] {
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
                "\"--compiler-identity=clang\" \"--diagnostic-format=clang\" \
                 \"--target={triple}\" \"--driver-name=clang\" \"a.c\" \"-c\""
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
                " {exe} -v --compiler-identity=gcc --diagnostic-format=gcc --target={triple} \
                 --driver-name=gcc a.c -c\n"
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
                " \"--compiler-identity=gcc\" \"--diagnostic-format=gcc\" \"--target={}\" \
                 \"--driver-name=gcc\" \"a b.c\" -D \"X=\\\"y z\\\"\" -c \"--output=o\\$.o\"\n",
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
}
