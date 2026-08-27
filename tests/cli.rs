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
}
