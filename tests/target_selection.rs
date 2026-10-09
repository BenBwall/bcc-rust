#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Integration tests exercise the CLI."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Test processes and captured output use standard owned buffers."
)]

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        process::Command,
    };

    const TRIPLES: [&str; 4] = [
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "x86_64-w64-windows-gnu",
        "x86_64-pc-windows-msvc",
    ];

    fn clang() -> std::path::PathBuf {
        let binary = if cfg!(windows) { "clang.exe" } else { "clang" };
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(binary)
    }

    #[test]
    fn every_target_passes_the_same_clang_and_bcc_c11_probe() {
        let probe = "tests/fixtures/targets/conformance.c";
        for triple in TRIPLES {
            let reference = Command::new(clang())
                .args([
                    &format!("--target={triple}"),
                    "-std=c11",
                    "-ffreestanding",
                    "-fsyntax-only",
                    probe,
                ])
                .output()
                .unwrap();
            assert!(
                reference.status.success(),
                "{triple}: {}",
                String::from_utf8_lossy(&reference.stderr)
            );
            let actual = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--target", triple, "-std=c11", probe])
                .output()
                .unwrap();
            let diagnostics = String::from_utf8_lossy(&actual.stderr);
            assert!(
                actual.status.success() && !diagnostics.contains("error:"),
                "{triple}: {diagnostics}"
            );
        }
    }

    #[test]
    fn mingw_calling_conventions_and_declspec_attributes_match_clang() {
        let probe = "tests/fixtures/targets/mingw-callconv.c";
        for standard in ["c17", "gnu17", "c23"] {
            for binary in [clang(), env!("CARGO_BIN_EXE_bcc-rust").into()] {
                let mut command = Command::new(&binary);
                _ = command.args([
                    "--target=x86_64-w64-windows-gnu",
                    &format!("-std={standard}"),
                ]);
                if binary == clang() {
                    _ = command.arg("-fsyntax-only");
                }
                let output = command.arg(probe).output().unwrap();
                let text = String::from_utf8_lossy(&output.stderr);
                assert!(
                    output.status.success() && !text.contains("error:"),
                    "{standard}: {text}"
                );
            }
        }
    }

    #[test]
    fn every_target_passes_the_int128_clang_and_bcc_probe() {
        let probe = "tests/fixtures/targets/int128.c";
        for triple in TRIPLES {
            for binary in [clang(), env!("CARGO_BIN_EXE_bcc-rust").into()] {
                let mut command = Command::new(&binary);
                _ = command.args([&format!("--target={triple}"), "-std=c11"]);
                if binary == clang() {
                    _ = command.arg("-fsyntax-only");
                }
                let output = command.arg(probe).output().unwrap();
                let diagnostics = String::from_utf8_lossy(&output.stderr);
                assert!(
                    output.status.success() && !diagnostics.contains("error:"),
                    "{triple}: {diagnostics}"
                );
            }
        }
    }

    #[test]
    fn aliases_and_equals_form_select_the_windows_abi() {
        for triple in [
            "x86_64-w64-windows-gnu",
            "x86_64-w64-mingw32",
            "x86_64-pc-windows-gnu",
        ] {
            let actual = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([
                    &format!("--target={triple}"),
                    "--semantic-types",
                    "--input",
                    "long value;\n",
                ])
                .output()
                .unwrap();
            let diagnostics = String::from_utf8_lossy(&actual.stderr);
            assert!(
                actual.status.success() && !diagnostics.contains("error:"),
                "{diagnostics}"
            );
            assert!(
                diagnostics.contains("target x86_64-w64-windows-gnu"),
                "{diagnostics}"
            );
            assert!(diagnostics.contains("size=4 align=4"), "{diagnostics}");
        }
    }

    #[test]
    fn unknown_and_missing_target_values_are_cli_errors() {
        for args in [
            vec!["--target", "aarch64-unknown-linux-gnu", "--input", ""],
            vec!["--target"],
        ] {
            let actual = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .output()
                .unwrap();
            assert!(!actual.status.success());
            let diagnostics = String::from_utf8_lossy(&actual.stderr);
            assert!(diagnostics.contains("target"), "{diagnostics}");
        }
    }

    #[test]
    fn non_bmp_wide_characters_follow_the_target_encoding() {
        for triple in TRIPLES {
            let source = "int f(void) { return L'\\U0001f600'; }\n";
            let windows = triple.contains("windows");
            let reference = Command::new(clang())
                .args([
                    &format!("--target={triple}"),
                    "-std=c11",
                    "-fsyntax-only",
                    "-x",
                    "c",
                    "-",
                ])
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut reference = reference;
            reference
                .stdin
                .take()
                .unwrap()
                .write_all(source.as_bytes())
                .unwrap();
            assert_eq!(
                reference.wait_with_output().unwrap().status.success(),
                !windows
            );
            let actual = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["--target", triple, "--input", source])
                .output()
                .unwrap();
            let diagnostics = String::from_utf8_lossy(&actual.stderr);
            assert_eq!(
                diagnostics.contains("error:"),
                windows,
                "{triple}: {diagnostics}"
            );
            if windows {
                assert!(diagnostics.contains("C99 §6.4.4.4p11"), "{diagnostics}");
            }
        }
    }
}
