#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the compiled CLI."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Test harnesses own process output and file paths outside compilation."
)]
mod tests {
    use std::{
        env,
        fs,
        path::PathBuf,
        process::Command,
    };

    #[test]
    fn semantic_inspection_matches_snapshot() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic");
        for (input, snapshot) in [
            ("types.c", "types.stderr"),
            ("expressions.c", "expressions.stderr"),
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["-std=c99", "--semantic-types", input])
                .current_dir(&directory)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .output()
                .unwrap();
            assert!(output.status.success(), "semantic inspector process");
            assert!(
                output.stdout.is_empty(),
                "inspection uses existing stderr channel"
            );
            let expected = directory.join(snapshot);
            if env::var_os("BLESS").is_some_and(|v| v == "1") {
                fs::write(&expected, &output.stderr).unwrap();
            }
            assert_eq!(output.stderr, fs::read(expected).unwrap());
            assert!(
                !String::from_utf8_lossy(&output.stderr).contains("error:"),
                "positive declarations and expressions"
            );
        }
    }

    #[test]
    fn semantic_default_and_syntax_only_modes_keep_separate_boundaries() {
        for (flags, diagnosed) in [
            (&[][..], true),
            (&["--syntax-tree"][..], false),
            (&["--raw-syntax"][..], false),
            (&["--tokens"][..], false),
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(flags)
                .args(["--input", "int x; double x;\n"])
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .output()
                .unwrap();
            assert!(output.status.success(), "inspection process");
            assert_eq!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("redeclaration has an incompatible type"),
                diagnosed
            );
        }
    }
}
