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
        assert!(stderr.contains("at 1:"), "{stderr}");
        assert!(!stderr.contains("at 0:[SourceVector"), "{stderr}");
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
            stderr.contains(
                "context: code=Syntax frame=Declaration expected=DeclarationContinuation"
            ),
            "{stderr}"
        );
        assert!(stderr.contains("discarded input:"), "{stderr}");
        assert!(
            stderr.contains("recovery: owner=Declaration discarded-tokens=2"),
            "{stderr}"
        );
        assert!(stderr.contains("note: parsing resumes here"), "{stderr}");
        assert!(stderr.contains("declarator after"), "{stderr}");
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
            default_stderr.contains("`const` keyword specified twice"),
            "{default_stderr}"
        );
        assert!(
            !suppressed_stderr.contains("`const` keyword specified twice"),
            "{suppressed_stderr}"
        );
        assert!(
            suppressed_stderr.contains("declarator value"),
            "{suppressed_stderr}"
        );
    }
}
