#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "CLI target regression tests."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own subprocess buffers."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::Command,
    };

    #[test]
    fn x86_baseline_feature_selection_matches_clang() {
        let clang = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        for triple in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            for mode in ["gnu17", "c17"] {
                for binary in [&clang, &PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust"))] {
                    let mut command = Command::new(binary);
                    _ = command.args([format!("--target={triple}"), format!("-std={mode}")]);
                    if binary == &clang {
                        // Only Clang's resource headers: for musl and MinGW,
                        // Clang searches the C library before them, and a
                        // Linux host has glibc there. The fixture sysroot
                        // stands in for that library on every host.
                        _ = command.args([
                            "--sysroot=tests/fixtures/hosted/sysroot",
                            "-nostdlibinc",
                            "-fsyntax-only",
                        ]);
                    }
                    let output = command
                        .arg("tests/fixtures/targets/x86-baseline-macros.c")
                        .output()
                        .unwrap();
                    let diagnostics = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        output.status.success() && !diagnostics.contains("error:"),
                        "{triple} {mode}: {diagnostics}"
                    );
                }
            }
        }
    }
}
