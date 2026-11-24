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
            vec!["-fno-inline"],
            vec!["-O2", "-fno-inline"],
            vec!["-Os", "-fno-inline"],
            vec!["-O2", "-fno-inline", "-finline"],
            vec!["-O2", "-finline", "-fno-inline"],
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

    /// clang-cl reads `/O`'s letters one at a time, and only the last `1`,
    /// `2`, `x` or `d` on the command line sets a level, so `/O2s` defines
    /// `__OPTIMIZE_SIZE__` and `/Os2` does not; `-O` is the same option,
    /// `/Ob0` defines `__NO_INLINE__`, and an argument no letter of which
    /// does anything (`/O3`) is unused. The macros, the warnings and the
    /// status match the pinned clang-cl's. `cl` reads `/O2s` as `/O2 /Os`
    /// and defines none of these GNU macros (Microsoft Learn, "/O options
    /// (Optimize code)" and "Predefined macros").
    #[test]
    fn clang_cl_optimization_letters_match_clang_cl() {
        let clang = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        });
        let directory = std::env::temp_dir().join(format!("bcc-cl-o-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("a.c"),
            "int x;\n#if defined __OPTIMIZE__ || defined __OPTIMIZE_SIZE__ || defined \
             __NO_INLINE__\nint gnu;\n#endif\n",
        )
        .unwrap();
        let run = |command: &mut Command, options: &[&str]| {
            let output = command
                .current_dir(&directory)
                .args(["/nologo", "/E", "/clang:-dM"])
                .args(options)
                .arg("a.c")
                .output()
                .unwrap();
            let macros: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| {
                    ["__OPTIMIZE__", "__OPTIMIZE_SIZE__", "__NO_INLINE__"]
                        .iter()
                        .any(|name| line.starts_with(&format!("#define {name} ")))
                })
                .map(str::to_owned)
                .collect();
            let stderr: Vec<String> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .map(|line| {
                    line.split_once(": ")
                        .map_or(line, |(_, rest)| rest)
                        .to_owned()
                })
                .collect();
            (output.status.code(), macros, stderr)
        };
        for options in [
            &["/O2s"][..],
            &["-O2s"],
            &["/Os2"],
            &["/Oxs"],
            &["-Oxs"],
            &["/O2b2"],
            &["-O2b2"],
            &["/O1", "/O2"],
            &["/O2", "/O1"],
            &["/O12"],
            &["/O2", "/Ob0"],
            &["/Ob0", "/Od"],
            &["/Obx"],
            &["/Ob4"],
            &["/O3"],
            &["-O3"],
            &["-Ofast"],
            &["/Og-"],
            &["/WX", "/O3"],
            &["/O2s", "/clang:-O0"],
        ] {
            let expected = run(Command::new(&clang).arg("--driver-mode=cl"), options);
            let actual = run(
                Command::new(env!("CARGO_BIN_EXE_bcc-rust")).arg("--clang-cl-mode"),
                options,
            );
            assert_eq!(actual, expected, "{options:?}");
        }
        let cl = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .current_dir(&directory)
            .args(["--msvc-mode", "/nologo", "/EP", "/O2s", "a.c"])
            .output()
            .unwrap();
        assert!(cl.status.success(), "{cl:?}");
        assert!(
            !String::from_utf8_lossy(&cl.stderr).contains("D9002"),
            "{cl:?}"
        );
        assert!(
            !String::from_utf8_lossy(&cl.stdout).contains("gnu"),
            "{cl:?}"
        );
        drop(std::fs::remove_dir_all(&directory));
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

    /// After `fold` decides the inlined `never ()`, nothing reaches the
    /// branch on `a`, but it stays until `simplify-cfg` removes unreachable
    /// blocks. Merging the `b < 0` test into it as into the arm's only
    /// predecessor removed the arm the `c` branch still leads to, and the
    /// compiler panicked (reduced from lz4 1.10.0's `lib/lz4.c`). Every
    /// level, back end and the interpreter print what GCC prints.
    #[test]
    fn a_decided_branch_keeps_the_arm_live_code_reaches() {
        // Declared rather than included, as a Windows host has no C headers.
        const SOURCE: &str = "int printf(const char *, ...);
int puts(const char *);
static int never(void) { return 0; }
int pick(int a, int b, int c) {
    if (never()) {
        if (a) goto test;
        goto done;
    }
    if (c) return 3;
test:
    if (b < 0) return 7;
done:
    return b;
}
int main(void) {
    for (int a = 0; a < 2; a++)
        for (int b = -1; b < 2; b++)
            for (int c = 0; c < 2; c++) printf(\"%d \", pick(a, b, c));
    puts(\"\");
    return 0;
}
";
        const EXPECTED: &str = "7 3 0 3 1 3 7 3 0 3 1 3 \n";
        let bcc = |flags: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(flags)
                .args(["--input", SOURCE])
                .output()
                .unwrap()
        };
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os"] {
            let output = bcc(&[level, "--interpret"]);
            assert!(output.status.success(), "{level}: {output:?}");
            assert_eq!(String::from_utf8_lossy(&output.stdout), EXPECTED, "{level}");
        }
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return;
        }
        let path = std::env::temp_dir().join(format!("decided-branch-{}", std::process::id()));
        let run = |what: &str| {
            let run = Command::new(&path).output().unwrap();
            assert!(run.status.success(), "{what}: {run:?}");
            assert_eq!(String::from_utf8_lossy(&run.stdout), EXPECTED, "{what}");
        };
        for level in ["-O0", "-O2"] {
            for backend in ["llvm", "native"] {
                let output = bcc(&[level, "--backend", backend, "-o", path.to_str().unwrap()]);
                assert!(output.status.success(), "{level} {backend}: {output:?}");
                run(&format!("{level} {backend}"));
            }
        }
        let source = path.with_extension("c");
        std::fs::write(&source, SOURCE).unwrap();
        let gcc = Command::new("gcc")
            .args(["-O2", "-o"])
            .arg(&path)
            .arg(&source)
            .output();
        std::fs::remove_file(&source).unwrap();
        match gcc {
            | Ok(output) if output.status.success() => run("gcc -O2"),
            | _ => eprintln!("skipping the GCC comparison: gcc did not build the program"),
        }
        std::fs::remove_file(path).unwrap();
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
