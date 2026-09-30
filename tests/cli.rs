#![expect(
    missing_docs,
    reason = "This integration test crate contains no public API."
)]
#![expect(
    unused_crate_dependencies,
    reason = "Integration tests inherit package dependencies but exercise the compiled binary."
)]

#[cfg(test)]
mod tests {
    use std::process::{
        Command,
        Output,
    };

    fn run(arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(arguments)
            .output()
            .expect("the bcc-rust test binary must run")
    }

    #[test]
    fn cli_help_prints_usage_to_stdout_and_succeeds_without_input() {
        for flag in ["--help", "-h"] {
            let output = run(&[flag]);
            let stdout = String::from_utf8_lossy(&output.stdout);

            assert!(output.status.success(), "{flag}: {output:?}");
            assert!(output.stderr.is_empty(), "{flag}: {output:?}");
            for expected in ["Usage:", "INPUT_FILE", "--input", "--syntax-tree"] {
                assert!(stdout.contains(expected), "{flag}: {stdout}");
            }
        }
    }

    #[test]
    fn cli_version_prints_to_stdout_and_succeeds_without_input() {
        for flag in ["--version", "-V"] {
            let output = run(&[flag]);
            let stdout = String::from_utf8_lossy(&output.stdout);

            assert!(output.status.success(), "{flag}: {output:?}");
            assert!(output.stderr.is_empty(), "{flag}: {output:?}");
            assert_eq!(
                stdout.trim(),
                concat!("bcc-rust ", env!("CARGO_PKG_VERSION")),
                "{flag}: {stdout}"
            );
        }
    }

    #[test]
    fn cli_argument_errors_print_to_stderr_and_use_the_usage_exit_code() {
        for arguments in [&[][..], &["--unknown-option"][..]] {
            let output = run(arguments);
            let stderr = String::from_utf8_lossy(&output.stderr);

            assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
            assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
            assert!(stderr.contains("Usage:"), "{arguments:?}: {stderr}");
            assert!(stderr.contains("--help"), "{arguments:?}: {stderr}");
        }
    }

    #[test]
    fn parser_output_modes_are_wired_through_the_cli() {
        let default = run(&["--input", "int value;\n"]);
        assert!(default.status.success());
        assert!(default.stderr.is_empty(), "{default:?}");

        let tree = run(&["--syntax-tree", "--input", "int value;\n"]);
        assert!(tree.status.success());
        assert!(
            String::from_utf8_lossy(&tree.stderr).contains("declarator value"),
            "{tree:?}"
        );

        let locations = run(&[
            "--syntax-tree",
            "--syntax-locations",
            "--input",
            "int value;\n",
        ]);
        assert!(locations.status.success());
        assert!(
            String::from_utf8_lossy(&locations.stderr).contains("@1:1"),
            "{locations:?}"
        );

        let raw = run(&["--raw-syntax", "--input", "int value;\n"]);
        assert!(raw.status.success());
        assert!(
            String::from_utf8_lossy(&raw.stderr).contains("SyntaxStore"),
            "{raw:?}"
        );
    }

    #[test]
    fn cli_rejects_contradictory_or_incomplete_output_modes() {
        let missing_tree = run(&["--syntax-locations", "--input", "int value;\n"]);
        assert!(!missing_tree.status.success());

        for parser_mode in ["--syntax-tree", "--raw-syntax"] {
            let contradictory = run(&["--tokens", parser_mode, "--input", "int value;\n"]);
            assert!(!contradictory.status.success(), "{contradictory:?}");
        }
    }

    #[test]
    fn parser_diagnostics_use_the_included_files_primary_index() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-programs")
            .join("once.c");
        let output = run(&[
            "--syntax-tree",
            source.to_str().expect("fixture path is valid UTF-8"),
        ]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "{output:?}");
        let header = std::path::Path::new("test-programs").join("once.h");
        assert!(
            stderr.contains(&format!(
                "--> {}:3:1",
                source.with_file_name("once.h").display()
            )) || stderr.contains(&format!("{}:3:1", header.display())),
            "{stderr}"
        );
        assert!(stderr.contains("3 | 123"), "{stderr}");
        assert!(!stderr.contains("SourceVector"), "{stderr}");
    }

    #[test]
    fn recovery_diagnostic_details_are_wired_through_the_cli() {
        let output = run(&[
            "--syntax-tree",
            "--input",
            "int first extra junk; int after;\n",
        ]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "{output:?}");
        assert!(
            stderr
                .contains("error: expected `,`, `=`, `;`, or a function body after the declarator"),
            "{stderr}"
        );
        assert!(stderr.contains("^^^^^ ---- skipped to recover"), "{stderr}");
        assert!(stderr.contains("declarator after"), "{stderr}");
        for internal in ["context:", "frame=", "owner=", "SourceVector"] {
            assert!(!stderr.contains(internal), "{stderr}");
        }
    }

    #[test]
    fn diagnostics_quote_source_instead_of_internal_representation() {
        for (source, expected) in [
            ("int x \"abc\";\n", "found string literal `\"abc\"`"),
            ("int x 1.5L;\n", "found floating constant `1.5L`"),
            (
                "static extern int x;\n",
                "cannot combine storage classes `static` and `extern`",
            ),
            ("int int x;\n", "duplicate `int`"),
            (
                "#if defined 1\n#endif\n",
                "expected a macro name after `defined`, found number `1`",
            ),
        ] {
            let output = run(&["--input", source]);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(expected), "{source:?}: {stderr}");
            for internal in [
                "StringCacheId",
                "LongDouble",
                "Keyword(",
                "Operator(",
                "Number\n",
            ] {
                assert!(!stderr.contains(internal), "{source:?}: {stderr}");
            }
        }
    }

    #[test]
    fn missing_semicolons_point_at_the_insertion_point() {
        let output = run(&["--input", "struct S { int a\nint b; };\n"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("error: expected `;`, found keyword `int`"),
            "{stderr}"
        );
        assert!(stderr.contains("1 | struct S { int a\n"), "{stderr}");
        assert!(stderr.contains("- help: add `;` here"), "{stderr}");
    }

    #[test]
    fn floating_constants_in_if_are_diagnosed_without_panicking() {
        let output = run(&["--input", "#if 1.5L\n#endif\nint x;\n"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "{output:?}");
        assert!(
            stderr.contains("error: floating constant in `#if` expression"),
            "{stderr}"
        );
        assert!(stderr.contains("1 error generated."), "{stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
    }

    #[test]
    fn token_dump_shows_spellings_values_and_locations_only() {
        let output = run(&["--tokens", "--input", "L'b' 1.5L 7u \"s\"\n"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(
            stderr,
            "<input>:1:1: character constant `L'b'` = 98 (wchar_t)\n<input>:1:6: floating \
             constant `1.5L` = 0x1.8p+0 (long double)\n<input>:1:11: integer constant `7u` = 7 \
             (unsigned int)\n<input>:1:14: string literal \"s\"\n"
        );
    }

    #[test]
    fn missing_input_files_name_the_path() {
        let output = run(&["does-not-exist.c"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success());
        assert!(
            stderr.starts_with("error: cannot read `does-not-exist.c`: "),
            "{stderr}"
        );
    }

    #[test]
    fn conflicting_type_specifier_diagnostics_render_source_spellings() {
        let output = run(&["--input", "typedef int T; T long value;\n"]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("error: cannot combine `long` with `T`"),
            "{stderr}"
        );
        for internal in ["VectorSlice", "StringCacheId", "TypedefName"] {
            assert!(!stderr.contains(internal), "{stderr}");
        }
    }

    #[test]
    fn repeated_specifier_warnings_can_be_suppressed_through_the_cli() {
        let default = run(&["--syntax-tree", "--input", "const const int value;\n"]);
        let suppressed = run(&[
            "--syntax-tree",
            "--no-repeated-specifier-warnings",
            "--input",
            "const const int value;\n",
        ]);
        let default_stderr = String::from_utf8_lossy(&default.stderr);
        let suppressed_stderr = String::from_utf8_lossy(&suppressed.stderr);

        assert!(default.status.success(), "{default:?}");
        assert!(suppressed.status.success(), "{suppressed:?}");
        assert!(
            default_stderr.contains("warning: duplicate `const`")
                && default_stderr.contains("--no-repeated-specifier-warnings"),
            "{default_stderr}"
        );
        assert!(
            !suppressed_stderr.contains("duplicate `const`"),
            "{suppressed_stderr}"
        );
        assert!(
            suppressed_stderr.contains("declarator value"),
            "{suppressed_stderr}"
        );
    }
}
