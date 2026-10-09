//! Startup definitions, undefinitions and forced includes through the CLI.
#![expect(unused_crate_dependencies, reason = "Integration tests run the CLI.")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Tests own command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::process::{
        Command,
        Output,
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
