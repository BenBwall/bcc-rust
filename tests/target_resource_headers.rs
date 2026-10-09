//! Target resource headers, independent of the host's C library.
#![expect(
    unused_crate_dependencies,
    reason = "Integration tests run compiler processes."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own process arguments and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        process::Command,
    };

    #[test]
    fn resource_headers_match_clang_across_targets_and_modes() {
        let clang = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(if cfg!(windows) { "clang.exe" } else { "clang" });
        let bcc = Path::new(env!("CARGO_BIN_EXE_bcc-rust"));
        for triple in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            for standard in ["c11", "gnu17"] {
                for ms_extensions in [false, true] {
                    for environment in ["-ffreestanding", "-fhosted"] {
                        for binary in [clang.as_path(), bcc] {
                            let mut command = Command::new(binary);
                            _ = command.args([
                                &format!("--target={triple}"),
                                &format!("-std={standard}"),
                                environment,
                                "-nostdlibinc",
                            ]);
                            _ = command
                                .env("NO_COLOR", "1")
                                .env_remove("CLICOLOR_FORCE")
                                .env_remove("CPATH")
                                .env_remove("C_INCLUDE_PATH");
                            if binary == clang {
                                _ = command
                                    .args(["-fsyntax-only", "-fms-compatibility-version=19.33"]);
                            }
                            if ms_extensions {
                                _ = command.arg("-fms-extensions");
                            }
                            let output = command
                                .arg("tests/fixtures/targets/conformance.c")
                                .output()
                                .unwrap();
                            let diagnostics = String::from_utf8_lossy(&output.stderr);
                            assert!(
                                output.status.success() && !diagnostics.contains("error:"),
                                "{} {triple} {standard} {environment} ms={ms_extensions}: \
                                 {diagnostics}",
                                binary.display()
                            );
                        }
                    }
                }
            }
        }
    }
}
