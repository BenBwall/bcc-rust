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
        fmt::Write as _,
        process::Command,
    };

    fn check_macros(flags: &[&str], expected: &BTreeSet<String>) {
        let mut source = String::new();
        for name in ["__OPTIMIZE__", "__OPTIMIZE_SIZE__", "__NO_INLINE__"] {
            if expected.contains(&format!("#define {name} 1")) {
                write!(
                    source,
                    "#if !defined({name}) || {name} != 1\n#error {name}\n#endif\n"
                )
                .unwrap();
            } else {
                write!(source, "#ifdef {name}\n#error {name}\n#endif\n").unwrap();
            }
        }
        source.push_str("int macro_probe;\n");
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", &source])
            .output()
            .unwrap();
        assert!(
            output.status.success() && !String::from_utf8_lossy(&output.stderr).contains("error:"),
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
            vec!["-O4"],
            vec!["-O5"],
            vec!["-O99"],
            vec!["-O-1"],
            vec!["-O01"],
            vec!["-O02"],
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
                .filter(|line| {
                    line.starts_with("#define __OPTIMIZE")
                        || line.starts_with("#define __NO_INLINE__")
                })
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
            check_macros(
                &flags,
                &BTreeSet::from(["#define __NO_INLINE__ 1".to_owned()]),
            );
        }
    }

    #[test]
    fn extended_optimization_warnings_match_clang() {
        for (flag, warning) in [
            ("-O4", "warning: -O4 is equivalent to -O3 [-Wdeprecated]"),
            (
                "-O5",
                "warning: optimization level '-O5' is not supported; using '-O3' instead",
            ),
            (
                "-O99",
                "warning: optimization level '-O99' is not supported; using '-O3' instead",
            ),
            (
                "-O-1",
                "warning: optimization level '-O-1' is not supported; using '-O3' instead",
            ),
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([flag, "--input", "int x;"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), warning);
        }
        for flag in ["-O01", "-O02"] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([flag, "--input", "int x;"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(output.stderr.is_empty(), "{output:?}");
        }
    }

    /// A read of an uninitialized local is a fixed value, so masking it clears
    /// bits and testing them decides nothing else, as GCC and Clang read such
    /// a program (gcc.c-torture/execute/pr43629.c), on every back end and with
    /// the local kept as a value, as a field, across a loop and in a `switch`.
    #[test]
    fn an_uninitialized_read_stays_a_value() {
        const COMMON: &str = "int flag;\nvoid abort(void);\nstruct s { int a; char b; };\n";
        for (name, body) in [
            (
                "scalar",
                "int x; if (flag) x = -1; else x &= 0xff; if (x & ~0xff) abort();",
            ),
            (
                "field",
                "struct s v; if (flag) v.a = -1; else v.a &= 0xff; if (v.a & ~0xff) abort();",
            ),
            (
                "loop",
                "int x; for (int i = 0; i < 3; i++) { if (flag) x = -1; else x &= 15; if (x & \
                 ~15) abort(); x = 0; }",
            ),
            (
                "switch",
                "unsigned x; switch (flag) { case 1: x = ~0u; break; default: x >>= 28; } if \
                 (!flag && x > 15) abort();",
            ),
            (
                "types",
                "long l; l &= 1; __int128 w; w &= 1; double d; d = d * 1.0; int *p; p = p ? p : \
                 0; if (flag && (l || w || d > 0 || p)) abort();",
            ),
        ] {
            let source = format!("{COMMON}int main(void) {{ {body} return 0; }}\n");
            for level in ["-O0", "-O1", "-O2"] {
                let interpreted = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                    .args([level, "-w", "--interpret", "--input", &source])
                    .output()
                    .unwrap();
                // Unoptimized, a field stays in memory and a merge keeps its
                // `poison`, which the interpreter reports reading.
                if level != "-O0" {
                    assert_eq!(
                        interpreted.status.code(),
                        Some(0),
                        "{name} {level} {interpreted:?}"
                    );
                }
                if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                    continue;
                }
                for backend in ["llvm", "native"] {
                    let path = std::env::temp_dir().join(format!(
                        "uninit-{}-{name}-{level}-{backend}",
                        std::process::id()
                    ));
                    let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                        .args([level, "-w", "--backend", backend, "-o"])
                        .arg(&path)
                        .args(["--input", &source])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{name} {level} {backend}: {output:?}"
                    );
                    let run = Command::new(&path).output().unwrap();
                    assert_eq!(
                        run.status.code(),
                        Some(0),
                        "{name} {level} {backend}: {run:?}"
                    );
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
    }

    #[test]
    fn optimization_flags_select_working_codegen_pipelines() {
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os", "-Oz", "-Og"] {
            for backend in ["llvm", "native"] {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("target")
                    .join(format!(
                        "optimization-{}-{level}-{backend}.o",
                        std::process::id()
                    ));
                let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                    .args([
                        level,
                        "--backend",
                        backend,
                        "-c",
                        "-o",
                        path.to_str().unwrap(),
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
                assert!(std::fs::metadata(&path).unwrap().len() > 0);
                std::fs::remove_file(path).unwrap();
            }
        }
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["-Oinvalid", "--input", "int x;"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("invalid integral value 'invalid' in '-Oinvalid'"));
        assert!(!stderr.contains("--optimization"));
    }
}
