//! Real Clang resource headers exercise arithmetic promotion overloads and
//! unavailable fallbacks independently of the host C library.
#![expect(unused_crate_dependencies, reason = "Tests run compiler processes.")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own process output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        process::Command,
    };

    #[test]
    fn tgmath_resource_selects_real_complex_and_integer_promotions_on_every_target() {
        let clang = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(if cfg!(windows) { "clang.exe" } else { "clang" });
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            for standard in ["c17", "gnu17", "c23"] {
                for compiler in [clang.as_path(), Path::new(env!("CARGO_BIN_EXE_bcc-rust"))] {
                    let mut command = Command::new(compiler);
                    _ = command.args([
                        &format!("--target={target}"),
                        &format!("-std={standard}"),
                        "-nostdlibinc",
                        "-isystem",
                        "tests/fixtures/overloadable",
                    ]);
                    if compiler == clang {
                        _ = command.arg("-fsyntax-only");
                    }
                    let output = command
                        .arg("tests/fixtures/overloadable/tgmath.c")
                        .env("NO_COLOR", "1")
                        .env_remove("CPATH")
                        .env_remove("C_INCLUDE_PATH")
                        .output()
                        .unwrap();
                    let diagnostics = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        output.status.success() && !diagnostics.contains("error:"),
                        "{} {target} {standard}:\n{diagnostics}",
                        compiler.display()
                    );
                }
            }
        }
    }
}
