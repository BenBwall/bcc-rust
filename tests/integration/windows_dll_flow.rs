//! DLL storage follows generated references, including Clang's -O0 control
//! flow choices, on both Windows ABIs.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own fixtures and captured process output."
)]

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        process::Command,
    };

    #[test]
    fn generated_reference_import_state_matches_live_clang() {
        let root = std::env::temp_dir().join(format!("bcc-dll-flow-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("flow.c");
        for target in ["x86_64-pc-windows-msvc", "x86_64-w64-windows-gnu"] {
            for (body, expected) in [
                ("if (0) { x; L: return 0; } goto L;", false),
                ("for (;;) {} return x;", true),
                ("while (1) {} return x;", true),
                ("while (1.0) {} return x;", true),
                ("for (;;) { sizeof(({ break; 0; })); } return x;", true),
                ("for (;;) { if (0) break; } return x;", true),
                ("for (;;) { if (n) break; } return x;", false),
                ("while (1) { while (0) { break; } } return x;", true),
                (
                    "while (1) { switch (n) { case 1: break; } } return x;",
                    true,
                ),
                ("for (;;) { return 0; L: break; } return x;", false),
                ("while (0) return x; return 0;", false),
                ("for (;1;) {} return x;", false),
                ("do {} while (1); return x;", false),
                ("if (0.0) return x; return 0;", false),
                ("return 0.0 && x;", false),
                ("return -0.0 ? x : 0;", false),
                ("if ((int)0.0) return x; return 0;", true),
                ("if (0.0 == 0.0) return 0; return x;", true),
            ] {
                fs::write(
                    &source,
                    format!(
                        "__declspec(dllimport) extern int x; int g(void) {{ return x; }} extern \
                         int x; int h(int n) {{ {body} }}"
                    ),
                )
                .unwrap();
                let clang = imported(&source, target, false);
                assert_eq!(clang, expected, "Clang {target}: {body}");
                assert_eq!(
                    imported(&source, target, true),
                    clang,
                    "BCC {target}: {body}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn imported(source: &Path, target: &str, bcc: bool) -> bool {
        let compiler = if bcc {
            Path::new(env!("CARGO_BIN_EXE_bcc-rust")).to_owned()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/llvm/bin")
                .join(if cfg!(windows) { "clang.exe" } else { "clang" })
        };
        let mut command = Command::new(compiler);
        _ = command.args([
            &format!("--target={target}"),
            "-std=gnu17",
            "-fms-extensions",
        ]);
        if bcc {
            _ = command.args(["--emit=llvm", "--opt=none", "--llvm-opt=O0"]);
        } else {
            _ = command.args(["-O0", "-S", "-emit-llvm", "-o", "-"]);
        }
        let output = command
            .arg(source)
            .env("NO_COLOR", "1")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let llvm = String::from_utf8(output.stdout).unwrap();
        llvm.lines()
            .find(|line| line.starts_with("@x =") || line.starts_with("@\"x\" ="))
            .unwrap_or_else(|| panic!("{command:?}: no x declaration in:\n{llvm}"))
            .contains("dllimport")
    }
}
