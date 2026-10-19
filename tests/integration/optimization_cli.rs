//! Optimization flags and GNU optimization macros, compared with pinned Clang.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "CLI oracle tests own process arguments and output."
)]
mod tests {

    use std::{
        collections::BTreeSet,
        process::Command,
    };

    fn check_macros(flags: &[&str], expected: &BTreeSet<String>) {
        let mut source = String::new();
        for name in ["__OPTIMIZE__", "__OPTIMIZE_SIZE__"] {
            if expected.contains(&format!("#define {name} 1")) {
                source.push_str(&format!(
                    "#if !defined({name}) || {name} != 1\n#error {name}\n#endif\n"
                ));
            } else {
                source.push_str(&format!("#ifdef {name}\n#error {name}\n#endif\n"));
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", &source])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn optimization_macro_set_matches_clang_at_every_level() {
        let clang = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        for flags in [
            vec![],
            vec!["-O0"],
            vec!["-O1"],
            vec!["-O2"],
            vec!["-O3"],
            vec!["-Os"],
            vec!["-Oz"],
            vec!["-Og"],
            vec!["-O"],
            vec!["-Os", "-O0"],
            vec!["-O0", "-O2"],
            vec!["-Os", "-O3"],
            vec!["-O2", "-U__OPTIMIZE__"],
            vec!["-O0", "-D__OPTIMIZE__=1"],
        ] {
            let output = Command::new(&clang)
                .args(&flags)
                .args([
                    "-dM",
                    "-E",
                    "-x",
                    "c",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let expected = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .filter(|line| line.starts_with("#define __OPTIMIZE"))
                .map(str::to_owned)
                .collect();
            check_macros(&flags, &expected);
        }
    }

    #[test]
    fn existing_optimizer_controls_define_macros_without_requesting_a_backend() {
        let optimized = BTreeSet::from(["#define __OPTIMIZE__ 1".to_owned()]);
        for flags in [
            vec!["--opt=bcc"],
            vec!["--llvm-opt=O1"],
            vec!["--llvm-opt=O2"],
            vec!["--llvm-opt=O3"],
            vec!["-O0", "--opt=bcc"],
            vec!["--opt", "bcc"],
        ] {
            check_macros(&flags, &optimized);
        }
        for flags in [
            vec!["--opt=none"],
            vec!["--llvm-opt=O0"],
            vec!["--llvm-opt=none"],
            vec!["-O2", "--opt=none", "--llvm-opt=none"],
        ] {
            check_macros(&flags, &BTreeSet::new());
        }
    }

    #[test]
    fn optimization_flags_select_working_codegen_pipelines() {
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os", "-Oz", "-Og"] {
            for backend in ["llvm", "native"] {
                let emit = if backend == "llvm" {
                    "--emit=llvm"
                } else {
                    "--emit=asm"
                };
                let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                    .args([
                        level,
                        "--backend",
                        backend,
                        emit,
                        "--input",
                        "int main(void) { return 0; }",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{level} {backend}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["-Oinvalid", "--input", "int x;"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}
