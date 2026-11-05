//! Startup definitions, undefinitions and forced includes through the CLI.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Tests own command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        process::{
            Command,
            Output,
            Stdio,
        },
    };

    fn bcc(args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap()
    }

    fn text(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn clean(output: &Output) {
        let text = text(output);
        assert!(
            output.status.success() && !text.contains("error:") && !text.contains("warning:"),
            "{text}"
        );
    }

    /// `-E -P` under each compiler personality, for `source`.
    fn preprocessed(personality: &str, source: &str) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args([personality, "-std=gnu23", "-E", "-P", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", text(&output));
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
    }

    /// The lines `-E -P` prints for `source`, each with its indentation.
    fn expect_spacing(source: &str, expected: &str) {
        for personality in ["--gcc-mode", "--clang-mode"] {
            assert_eq!(preprocessed(personality, source), expected, "{personality}");
        }
    }

    /// The tokens around a macro's expansion and its arguments are spaced as
    /// GCC 13.3's and Clang 23's `-E -P` space them.
    #[test]
    fn expansions_are_spaced_as_gcc_and_clang_space_them() {
        expect_spacing(
            concat!(
                "#define H(x) x\n",
                "#define O(x)(x)\n",
                "#define Q -x\n",
                "#define P(a,b) a+b\n",
                "#define P2(a,b) a + b\n",
                "#define ID(x) x\n",
                "#define W(x) [x]\n",
                "#define F(...) #__VA_OPT__(x)\n",
                "#define G(...) __VA_OPT__(x)\n",
                "h = H(1);\n",
                "o = O(2);\n",
                "t=H(1);\n",
                "u=Q;\n",
                "p = P(x , y);\n",
                "p2 = P2(x,y);\n",
                "i = ID(ID(1))+ID( 2 );\n",
                "w = W( a b );\n",
                "n = ID(-)ID(-);\n",
                "b = F(1);\n",
                "c = G(1);\n",
            ),
            concat!(
                "h = 1;\n",
                "o = (2);\n",
                "t=1;\n",
                "u=-x;\n",
                "p = x+y;\n",
                "p2 = x + y;\n",
                "i = 1 +2;\n",
                "w = [a b];\n",
                "n = - -;\n",
                "b = \"x\";\n",
                "c = x;\n",
            ),
        );
    }

    /// GCC's own tests of `-E` spacing, gcc.dg/cpp/spacing1.c without the
    /// invocation that spans lines, whose line placement differs, and
    /// spacing2.c: padding survives a failed search for a macro's `(`, and an
    /// empty macro leaves no space before the next line.
    #[test]
    fn padding_survives_looking_for_a_macro_call() {
        expect_spacing(
            concat!(
                "#define str(x) #x\n",
                "#define f(x) x\n",
                "#define glue(x, y) x ## y\n",
                "#define EMPTY\n",
                "#define A(x) B x\n",
                "#define B(x)\n",
                "#define C A\n",
                "#define D() A\n",
                "#define foo bar\n",
                "glue (EMPTY 4, 4) EMPTY;\n",
                "A(Q) C(Q) D()Q D():\n",
                "f\n",
                "bar\n",
                "A\n",
                "bad\n",
                "a = EMPTY\n",
                "foo..\n",
            ),
            " 44 ;\nB Q B Q A Q A:\nf\nbar\nA\nbad\na =\nbar..\n",
        );
    }

    /// GCC's gcc.dg/cpp/avoidpaste1.c and avoidpaste2.c: tokens that are
    /// already together stay together, and macros and arguments do not
    /// paste what surrounds them. `__COUNTER__` stands in for
    /// `__INCLUDE_LEVEL__`, which is not implemented.
    #[test]
    fn replacement_does_not_paste_what_surrounds_it() {
        expect_spacing(
            concat!(
                "#define f(x) x\n",
                "#define g\n",
                "#define tricky 1.0e ## -1\n",
                ":: :g: :f(): :f(^): tricky\n",
                ":f(:): .. .__COUNTER__ __COUNTER__.\n",
            ),
            ":: : : : : :^: 1.0e- 1\n: : : .. . 0 1 .\n",
        );
        expect_spacing(
            concat!(
                "#define EMPTY_WITH_LEADING_SPACE\n",
                "#define f(x, y) :x: -y##> -##y>\n",
                "#define g(x, y) :x: :y##2 2##y:\n",
                " EMPTY_WITH_LEADING_SPACE\n",
                "f(:,) \n",
                "g(2, 2)\n",
            ),
            ": : : - > - >\n:2: :22 22:\n",
        );
    }

    /// A wider matrix of expansions, arguments, `#`, `##`, `__VA_OPT__`,
    /// built-in macros and token pairs. The expected text is `gcc -E -P
    /// -std=gnu2x` from GCC 13.3, whose model both personalities follow; see
    /// the PR that added it for where Clang's own printer differs.
    #[test]
    fn expansion_matrix_is_spaced_as_gcc_spaces_it() {
        let source = include_str!("../fixtures/preprocessing/expansion_spacing.c")
            .replace("\r\n", "\n");
        let expected = include_str!("../fixtures/preprocessing/expansion_spacing.expected")
            .replace("\r\n", "\n");
        expect_spacing(&source, &expected);
    }

    /// `#` drops a final `\\` that would escape the closing quote and warns
    /// in GCC's and Clang's words, in every revision, through `__VA_OPT__`
    /// too (gcc.dg/cpp/strify2.c). C99 §6.10.3.2p2.
    #[test]
    fn stringizing_drops_a_final_backslash_with_a_warning() {
        let source = "#define S(x) #x\n#define V(...) #__VA_OPT__(\\)\n_Static_assert(sizeof \
                      S(\\) == 1, \"\");\n_Static_assert(sizeof S(a\\) == 2, \
                      \"\");\n_Static_assert(sizeof S(\\\\) == 2, \"\");\n_Static_assert(sizeof \
                      V(1) == 1, \"\");\n";
        let output = bcc(&["--std", "c23", "--input", source]);
        let warnings = text(&output);
        assert!(output.status.success(), "{warnings}");
        assert_eq!(
            warnings
                .matches("warning: invalid string literal, ignoring final '\\'")
                .count(),
            3,
            "{warnings}"
        );
        let output = bcc(&[
            "--std",
            "c99",
            "--input",
            "#define S(x) #x\nchar s[sizeof S(\\)];\n",
        ]);
        assert!(output.status.success(), "{}", text(&output));
    }

    #[test]
    fn definitions_keep_values_and_follow_command_line_order() {
        for standard in ["c17", "gnu17", "c23"] {
            clean(&bcc(&[
                "--std",
                standard,
                "-D",
                "DEFAULT",
                "-DVALUE=3",
                "-U",
                "VALUE",
                "-D",
                "VALUE=7",
                "-DF(x)=((x) + VALUE)",
                "-DTEXT=\"a = b\"",
                "-DEMPTY=",
                "--input",
                "#if DEFAULT != 1 || F(2) != 9\n#error order\n#endif\nconst char *s = TEXT; EMPTY \
                 int value;\n",
            ]));
        }
        clean(&bcc(&[
            "-DVALUE=3",
            "-UVALUE",
            "--input",
            "#ifdef VALUE\n#error still defined\n#endif\nint x;\n",
        ]));
        clean(&bcc(&[
            "-UWIN32",
            "--target=x86_64-w64-windows-gnu",
            "--input",
            "#ifdef WIN32\n#error predefined order\n#endif\nint x;\n",
        ]));
        clean(&bcc(&[
            "-DVALUE=7\n#error injected",
            "--input",
            "#if VALUE != 7\n#error value\n#endif\nint x;\n",
        ]));
    }

    #[test]
    #[expect(
        clippy::disallowed_macros,
        reason = "Test arguments contain literal line endings."
    )]
    fn definitions_truncate_at_line_endings() {
        for standard in ["c17", "gnu17", "c23"] {
            for ending in ["\r", "\n", "\r\n"] {
                for tail in ["", "#error injected"] {
                    let definition = format!("-DVALUE=7{ending}{tail}");
                    clean(&bcc(&[
                        "--std",
                        standard,
                        &definition,
                        "-DFOLLOWING=11",
                        "--input",
                        "#if VALUE != 7 || FOLLOWING != 11\n#error wrong definition\n#endif\nint \
                         n = VALUE;\n",
                    ]));
                }
            }
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_macros,
        reason = "Test arguments contain literal line endings."
    )]
    fn undefinitions_truncate_at_line_endings() {
        for standard in ["c17", "gnu17", "c23"] {
            for ending in ["\r", "\n", "\r\n"] {
                for tail in ["", "#error injected"] {
                    let undefinition = format!("VALUE{ending}{tail}");
                    clean(&bcc(&[
                        "--std",
                        standard,
                        "-DVALUE=7",
                        "-U",
                        &undefinition,
                        "-DFOLLOWING=11",
                        "--input",
                        "#ifdef VALUE\n#error still defined\n#endif\n#if FOLLOWING != 11\n#error \
                         lost following option\n#endif\nint n = FOLLOWING;\n",
                    ]));
                }
            }
        }
    }

    #[test]
    fn invalid_definitions_and_builtin_protections_are_diagnosed() {
        for option in ["-D1x", "-D", "-U"] {
            let result = if option == "-D" || option == "-U" {
                bcc(&[option])
            } else {
                bcc(&[option, "--input", "int following;\n"])
            };
            assert!(text(&result).contains("error:"), "{}", text(&result));
        }
        for option in ["-D__LINE__=7", "-U__LINE__", "-D__STDC__=0", "-U__STDC__"] {
            let result = bcc(&[option, "--input", "int following;\n"]);
            assert!(
                text(&result).contains("predefined macro"),
                "{}",
                text(&result)
            );
            assert!(
                text(&result).contains("<command line>"),
                "{}",
                text(&result)
            );
        }
    }

    #[test]
    fn definitions_enter_at_phase_three_without_splicing_or_trigraphs() {
        for definition in ["-DX=foo\\", "-DX=foo??/"] {
            clean(&bcc(&[
                "-std=c17",
                definition,
                "-DY=5",
                "--input",
                "#if Y != 5\n#error lost following option\n#endif\nint y = Y;\n",
            ]));
        }
        let result = bcc(&[
            "-std=c17",
            "-DTEXT=\"??=\"",
            "--tokens",
            "--input",
            "TEXT\n",
        ]);
        clean(&result);
        assert!(text(&result).contains("??="), "{}", text(&result));
    }

    #[test]
    fn command_line_token_provenance_is_stable() {
        let args = ["-DVALUE=unique_name", "--tokens", "--input", "VALUE\n"];
        let first = bcc(&args);
        clean(&first);
        assert_eq!(first.stderr, bcc(&args).stderr);
        assert!(
            text(&first).contains("<command line>:1:"),
            "{}",
            text(&first)
        );
    }

    #[test]
    fn forced_includes_follow_definitions_and_use_normal_lookup() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/preprocessing-cli");
        std::fs::create_dir_all(directory.join("main")).unwrap();
        std::fs::write(
            directory.join("first header.h"),
            "#if VALUE != 9\n#error definition order\n#endif\n#define FROM_FIRST 5\nint \
             forced_declaration;\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("second.h"),
            "#if FROM_FIRST != 5\n#error include order\n#endif\n#define FROM_SECOND 1\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("main/input.c"),
            "#ifndef FROM_SECOND\n#error missing include\n#endif\nint main_declaration;\n",
        )
        .unwrap();
        let input = directory.join("main/input.c");
        let result = bcc(&[
            "--tokens",
            "-include",
            "first header.h",
            "-DVALUE=9",
            "-I",
            directory.to_str().unwrap(),
            "-include",
            "second.h",
            input.to_str().unwrap(),
        ]);
        clean(&result);
        assert!(
            text(&result).contains("forced_declaration"),
            "{}",
            text(&result)
        );
        let cwd_header = "target/preprocessing-cli/second.h";
        clean(&bcc(&[
            "-DFROM_FIRST=5",
            "-include",
            cwd_header,
            input.to_str().unwrap(),
        ]));
        let missing = bcc(&[
            "-include",
            "definitely-missing-forced-header.h",
            "--input",
            "int following;\n",
        ]);
        assert!(text(&missing).contains("error:"), "{}", text(&missing));
        assert!(
            text(&missing).contains("<command line>"),
            "{}",
            text(&missing)
        );
    }
}
