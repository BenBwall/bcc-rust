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
    use std::{
        env,
        fs,
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
    };

    fn fixture_directory() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics")
    }

    fn fixtures() -> Vec<PathBuf> {
        let mut fixtures: Vec<_> = fs::read_dir(fixture_directory())
            .expect("diagnostic fixtures must be readable")
            .map(|entry| entry.expect("fixture entry must be readable").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
            .collect();
        fixtures.sort();
        assert!(
            !fixtures.is_empty(),
            "the diagnostic corpus must not be empty"
        );
        fixtures
    }

    fn run(fixture: &Path) -> Output {
        let flags = fs::read_to_string(fixture.with_extension("args")).unwrap_or_default();
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags.split_whitespace())
            .arg(fixture.file_name().expect("fixture must have a file name"))
            .current_dir(fixture_directory())
            .env("NO_COLOR", "1")
            .env("RUST_BACKTRACE", "0")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .expect("the bcc-rust test binary must run")
    }

    #[test]
    fn diagnostics_match_golden_files() {
        let bless = env::var_os("BLESS").is_some_and(|value| value == "1");
        let mut failures = Vec::new();
        for fixture in fixtures() {
            let output = run(&fixture);
            assert!(
                output.stdout.is_empty(),
                "{}: {output:?}",
                fixture.display()
            );
            assert!(
                !output.stderr.is_empty(),
                "{} produced no diagnostic",
                fixture.display()
            );
            let expected_path = fixture.with_extension("stderr");
            if bless {
                fs::write(&expected_path, &output.stderr).expect("golden file must be writable");
            } else {
                let expected = fs::read(&expected_path).unwrap_or_else(|error| {
                    panic!(
                        "{}: {error}; run with BLESS=1 to create it",
                        expected_path.display()
                    )
                });
                if output.stderr != expected {
                    failures.push(format!(
                        "{}: stderr differs; actual:\n{}\nexpected:\n{}",
                        fixture.display(),
                        String::from_utf8_lossy(&output.stderr),
                        String::from_utf8_lossy(&expected)
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }

    #[test]
    fn diagnostics_never_leak_internal_representations() {
        let mut failures = Vec::new();
        for fixture in fixtures() {
            let output = run(&fixture);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                failures.push(format!("{}: process failed: {stderr}", fixture.display()));
            }
            for forbidden in [
                "StringCacheId",
                "SourceVector",
                "VectorSlice",
                "Some(",
                "Keyword(",
                "Operator(",
                "Integer(",
                "LongDouble",
                "panicked",
            ] {
                if stderr.contains(forbidden) {
                    failures.push(format!(
                        "{} contains {forbidden}:\n{stderr}",
                        fixture.display()
                    ));
                }
            }
            if output.stderr.contains(&0) {
                failures.push(format!("{} contains a raw NUL byte", fixture.display()));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
