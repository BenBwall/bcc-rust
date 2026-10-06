#![expect(
    missing_docs,
    reason = "This integration test crate contains no public API."
)]
#![expect(
    unused_crate_dependencies,
    reason = "Integration tests inherit package dependencies but exercise the compiled binary."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
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
    fn lookahead_warnings_follow_earlier_parser_errors() {
        let output = run(&[
            "--input",
            "void f(int a){\n  a = { 1, 2\n#warning late\n  };\n}\n",
        ]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = stderr
            .find("error: expected an expression")
            .expect("parser error");
        let warning = stderr
            .find("warning: unknown preprocessing directive")
            .expect("warning");
        assert!(error < warning, "{stderr}");
        assert!(
            stderr.contains("1 error and 1 warning generated"),
            "{stderr}"
        );
    }

    #[test]
    fn diagnostic_order_uses_macro_invocations_instead_of_definitions() {
        let output = run(&[
            "--input",
            "#define BAD )\nvoid f(){\nint a = ;\n#warning middle\nint b = BAD;\n}\n",
        ]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first = stderr
            .find("error: expected an expression, found `;`")
            .expect("first error");
        let warning = stderr
            .find("warning: unknown preprocessing directive")
            .expect("warning");
        let last = stderr
            .find("error: expected an expression, found `)`")
            .expect("macro error");
        assert!(first < warning && warning < last, "{stderr}");
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
            String::from_utf8_lossy(&raw.stderr).contains("ParsedTranslationUnit"),
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

    fn stderr_of(arguments: &[&str]) -> String {
        let output = run(arguments);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{arguments:?}: {output:?}");
        assert!(!stderr.contains("panicked"), "{arguments:?}: {stderr}");
        stderr
    }

    #[test]
    fn token_dump_renders_a_trailing_preprocessing_error() {
        let stderr = stderr_of(&["--tokens", "--input", "int x;\n#error boom\n"]);

        assert!(stderr.contains("error: #error boom"), "{stderr}");
        assert!(stderr.contains("2 | #error boom"), "{stderr}");
    }

    #[test]
    fn token_dump_reports_conditionals_left_open_across_tokens() {
        let stderr = stderr_of(&["--tokens", "--input", "#if 1\nint x;\n#if 1\nint y;\n"]);

        assert_eq!(
            stderr.matches("error: unterminated `#if`").count(),
            2,
            "{stderr}"
        );
        assert!(stderr.contains(" --> <input>:1:2"), "{stderr}");
        assert!(stderr.contains(" --> <input>:3:2"), "{stderr}");
    }

    #[test]
    fn pragma_operator_diagnostics_quote_their_own_payload() {
        let stderr = stderr_of(&[
            "--input",
            // Unknown pragmas are ignored whole (C99 6.10.6), so malformed
            // standard pragmas provide the diagnostics.
            "_Pragma(\"STDC aaaaaaaa(\")\n_Pragma(\"STDC b(\")\n_Pragma(\"STDC \
             \u{e9}\u{20ac}(\")\nint x;\n",
        ]);

        assert!(stderr.contains("1 | STDC aaaaaaaa(\n"), "{stderr}");
        assert!(stderr.contains("1 | STDC b(\n"), "{stderr}");
        assert!(stderr.contains("1 | STDC \u{e9}\u{20ac}(\n"), "{stderr}");
    }

    #[test]
    fn macro_expanded_quoted_includes_search_beside_the_directive() {
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("macro-include");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("def.h"), "#define H \"sib.h\"\n").unwrap();
        std::fs::write(sub.join("sib.h"), "int from_definition_directory;\n").unwrap();
        std::fs::write(root.join("sib.h"), "int from_directive_directory;\n").unwrap();
        let main = root.join("main.c");
        std::fs::write(&main, "#include \"sub/def.h\"\n#include H\n").unwrap();

        let stderr = stderr_of(&["--tokens", main.to_str().unwrap()]);

        assert!(
            stderr.contains("identifier `from_directive_directory`"),
            "{stderr}"
        );
    }

    #[test]
    fn suffixed_exact_zero_floating_constants_do_not_underflow() {
        for constant in ["0.0f", "0.0F", "0.e0f", "0x0.0p0f"] {
            let source = format!("float x = {constant};\n");
            let stderr = stderr_of(&["--input", &source]);
            assert!(stderr.is_empty(), "{constant}: {stderr}");
        }
        let stderr = stderr_of(&["--input", "float x = 1e-999f;\n"]);
        assert!(stderr.contains("too small for `float`"), "{stderr}");
    }

    #[test]
    fn unknown_characters_after_a_line_splice_are_reported_at_themselves() {
        let stderr = stderr_of(&["--input", "int x = 1 \\\n@;\n"]);

        assert!(stderr.contains("unexpected character `@`"), "{stderr}");
        assert!(stderr.contains(" --> <input>:2:1"), "{stderr}");
        assert!(stderr.contains("2 | @;"), "{stderr}");
    }

    #[test]
    fn snippets_index_lone_carriage_returns_as_lines() {
        let stderr = stderr_of(&["--input", "int a;\rint b;\rint c = ;\r"]);

        assert!(stderr.contains(" --> <input>:3:9"), "{stderr}");
        assert!(stderr.contains("3 | int c = ;\n"), "{stderr}");
    }

    #[test]
    fn token_dump_keeps_nul_escapes_apart_from_following_digits() {
        let stderr = stderr_of(&["--tokens", "--input", "\"\\0\" \"1\"\n"]);

        assert!(stderr.contains("string literal \"\\0001\""), "{stderr}");
    }

    #[test]
    fn plain_line_directives_are_accepted_without_a_diagnostic() {
        let stderr = stderr_of(&["--input", "#line 12\nint x;\n"]);

        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn errors_at_one_token_fold_into_the_first() {
        for input in [
            // Each unterminated constant also draws a parser error at the
            // same token, which is folded into the lexer error.
            "'a\n'b\n",
            // A preprocessing diagnostic lands between two parser errors at
            // one token.
            "int x[] = {1 B\n#error e\n};\n",
        ] {
            let stderr = stderr_of(&["--input", input]);
            assert!(
                stderr.contains("2 errors generated."),
                "{input:?}: {stderr}"
            );
        }
    }

    #[test]
    fn a_label_spanning_many_lines_renders_in_linear_time() {
        // Too long for a command line, so it goes through a file.
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("long-skipped-range.c");
        let input = format!("int a = 1 2\n{}{}", "x\n".repeat(60_000), ";\nint b;\n");
        std::fs::write(&path, input).expect("the test input must be writable");
        let started = std::time::Instant::now();
        let stderr = stderr_of(&[path.to_str().expect("the temporary path is UTF-8")]);

        assert!(stderr.contains("1 error generated."), "{stderr}");
        // Rescanning every mark for each rendered line took about 15 s here.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "rendering took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn errors_in_two_uses_of_one_macro_are_reported_separately() {
        let stderr = stderr_of(&["--input", "#define SEMI ;\nint a = SEMI\nint b = SEMI\n"]);

        assert!(stderr.contains("2 errors generated."), "{stderr}");
    }

    /// Dumps the tokens of `__DATE__` and `__TIME__` with `arguments` added,
    /// and `SOURCE_DATE_EPOCH` set to `variable`, or unset.
    fn run_timestamp(arguments: &[&str], variable: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command
            .args(arguments)
            .args(["--tokens", "--input", "__DATE__\n,\n__TIME__\n"]);
        _ = match variable {
            | Some(value) => command.env("SOURCE_DATE_EPOCH", value),
            | None => command.env_remove("SOURCE_DATE_EPOCH"),
        };
        command.output().expect("the bcc-rust test binary must run")
    }

    /// The `__DATE__` and `__TIME__` spellings of a successful
    /// [`run_timestamp`].
    fn timestamp(output: &Output) -> (String, String) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{output:?}");
        let literals: Vec<&str> = stderr
            .lines()
            .filter_map(|line| {
                line.split_once(": string literal ")
                    .map(|(_, literal)| literal)
            })
            .collect();
        match literals[..] {
            | [date, time] => (date.to_owned(), time.to_owned()),
            | _ => panic!("expected two string literals: {stderr}"),
        }
    }

    fn pinned(date: &str, time: &str) -> (String, String) {
        (format!("\"{date}\""), format!("\"{time}\""))
    }

    /// Whether a timestamp spells `"Mmm dd yyyy"` (the day space-padded) and
    /// `"hh:mm:ss"`.
    fn is_timestamp_spelling((date, time): &(String, String)) -> bool {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
        let date = date.trim_matches('"');
        let time = time.trim_matches('"');
        date.len() == 11
            && MONTHS.contains(&&date[..3])
            && &date[3..4] == " "
            && (&date[4..5] == " " || digits(&date[4..5]))
            && digits(&date[5..6])
            && &date[6..7] == " "
            && digits(&date[7..])
            && time.len() == 8
            && time.split(':').count() == 3
            && time.split(':').all(|part| part.len() == 2 && digits(part))
    }

    /// `text` without the ANSI styles clap always adds.
    fn without_styles(text: &str) -> String {
        let mut plain = String::new();
        let mut rest = text;
        while let Some((before, escape)) = rest.split_once('\u{1b}') {
            plain.push_str(before);
            rest = escape.split_once('m').map_or("", |(_, after)| after);
        }
        plain.push_str(rest);
        plain
    }

    #[test]
    fn source_date_epoch_flag_pins_date_and_time_in_utc() {
        for (seconds, date, time) in [
            ("0", "Jan  1 1970", "00:00:00"),
            ("1700000000", "Nov 14 2023", "22:13:20"),
            ("-1", "Dec 31 1969", "23:59:59"),
        ] {
            let output = run_timestamp(&["--source-date-epoch", seconds], None);
            assert_eq!(timestamp(&output), pinned(date, time), "{seconds}");
        }
    }

    #[test]
    fn source_date_epoch_variable_pins_date_and_time_in_utc() {
        // As before the CLI read it: an optional sign and surrounding
        // whitespace are accepted.
        for (seconds, date, time) in [
            ("0", "Jan  1 1970", "00:00:00"),
            ("1700000000", "Nov 14 2023", "22:13:20"),
            (" +5 ", "Jan  1 1970", "00:00:05"),
            ("-1", "Dec 31 1969", "23:59:59"),
        ] {
            let output = run_timestamp(&[], Some(seconds));
            assert_eq!(timestamp(&output), pinned(date, time), "{seconds:?}");
        }
    }

    #[test]
    fn source_date_epoch_flag_overrides_the_variable() {
        for variable in ["1700000000", "malformed"] {
            let output = run_timestamp(&["--source-date-epoch=0"], Some(variable));
            assert_eq!(
                timestamp(&output),
                pinned("Jan  1 1970", "00:00:00"),
                "{variable}"
            );
        }
    }

    #[test]
    fn malformed_source_date_epoch_flag_is_an_argument_error() {
        for (seconds, reason) in [
            ("soon", "invalid digit found in string"),
            ("1.5", "invalid digit found in string"),
            ("", "cannot parse integer from empty string"),
            ("99999999999999", "is not in"),
            ("9223372036854775808", "number too large"),
        ] {
            for variable in [None, Some("0")] {
                let output = run_timestamp(&["--source-date-epoch", seconds], variable);
                let stderr = without_styles(&String::from_utf8_lossy(&output.stderr));
                assert_eq!(output.status.code(), Some(2), "{seconds:?}: {output:?}");
                assert!(output.stdout.is_empty(), "{seconds:?}: {output:?}");
                assert!(
                    stderr.contains("invalid value") && stderr.contains(reason),
                    "{seconds:?}: {stderr}"
                );
                assert!(
                    stderr.contains("--source-date-epoch <SECONDS>"),
                    "{seconds:?}: {stderr}"
                );
            }
        }
    }

    #[test]
    fn malformed_source_date_epoch_variable_is_ignored() {
        // Values that are not integers, or that no date can represent, leave
        // the local time, as before the CLI read the variable.
        for variable in ["soon", "1.5", "", "99999999999999", "9223372036854775808"] {
            let output = run_timestamp(&[], Some(variable));
            let stamp = timestamp(&output);
            assert!(is_timestamp_spelling(&stamp), "{variable:?}: {stamp:?}");
            assert_ne!(stamp.0, "\"Jan  1 1970\"", "{variable:?}");
        }
    }

    #[test]
    fn date_and_time_default_to_the_local_time() {
        let stamp = timestamp(&run_timestamp(&[], None));
        assert!(is_timestamp_spelling(&stamp), "{stamp:?}");
    }

    #[test]
    fn help_documents_the_source_date_epoch_flag_and_variable() {
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .arg("--help")
            .env_remove("SOURCE_DATE_EPOCH")
            .output()
            .expect("the bcc-rust test binary must run");
        let stdout = without_styles(&String::from_utf8_lossy(&output.stdout));
        assert!(output.status.success(), "{output:?}");
        for expected in [
            "--source-date-epoch <SECONDS>",
            "[env: SOURCE_DATE_EPOCH=]",
            "The flag overrides `SOURCE_DATE_EPOCH`",
        ] {
            assert!(stdout.contains(expected), "{expected}: {stdout}");
        }
    }
}
