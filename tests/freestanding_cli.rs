#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "CLI integration tests have no public API."
)]

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own command output and generated sources outside compilation."
)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        path::PathBuf,
        process::{
            Command,
            Output,
        },
    };

    fn bcc(args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap()
    }

    fn clean(output: &Output) {
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && !text.contains("error:") && !text.contains("warning:"),
            "{text}"
        );
    }

    fn clang() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }

    #[test]
    fn every_resource_header_and_value_passes_the_shared_probe() {
        let source = include_str!("fixtures/freestanding/conformance.c");
        clean(&bcc(&["-std=c11", "-pedantic-errors", "--input", source]));
        clean(&bcc(&["-std=gnu17", "--input", source]));
        // Clang's resource headers also implement the freestanding execution
        // model.
        let output = Command::new(clang())
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "-std=c11",
                "-ffreestanding",
                "-pedantic-errors",
                "-fsyntax-only",
                "tests/fixtures/freestanding/conformance.c",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(clang())
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "-std=gnu17",
                "-ffreestanding",
                "-fsyntax-only",
                "tests/fixtures/freestanding/conformance.c",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn every_target_macro_expands_in_every_language_mode() {
        let definitions = include_str!("fixtures/freestanding/target-macros.h");
        let mut source = String::from("#define S0(x) #x\n#define S(x) S0(x)\n");
        let mut count = 0;
        for definition in definitions.lines() {
            let name = definition.split_whitespace().nth(1).unwrap();
            let base = name.split('(').next().unwrap();
            write!(source, "#ifndef {base}\n#error missing {base}\n#endif\n").unwrap();
            writeln!(
                source,
                "S({});",
                if name.contains('(') {
                    name.replace("(c)", "(123)")
                } else {
                    name.to_owned()
                }
            )
            .unwrap();
            count += 1;
        }
        let probe =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/freestanding-macro-probe.c");
        fs::write(&probe, &source).unwrap();
        let expanded = Command::new(clang())
            .args(["--target=x86_64-unknown-linux-gnu", "-E", "-P"])
            .arg(&probe)
            .output()
            .unwrap();
        assert!(expanded.status.success());
        let normalize = |text: &str| {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        let expected: Vec<_> = String::from_utf8_lossy(&expanded.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| normalize(line.trim_end_matches(';')))
            .collect();
        let dump = Command::new(clang())
            .args([
                "--target=x86_64-unknown-linux-gnu",
                "-dM",
                "-E",
                "-x",
                "c",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ])
            .output()
            .unwrap();
        assert!(dump.status.success());
        let dump = String::from_utf8_lossy(&dump.stdout);
        for line in definitions.lines() {
            assert!(
                dump.lines()
                    .any(|actual| actual.trim_end() == line.trim_end()),
                "{line}"
            );
        }
        source.push_str(
            "#if defined(__clang__) || defined(_MSC_VER)
#error compiler identity macros must              be absent
#endif
#if !defined(__GNUC__) || __GNUC__ != 4 || __GNUC_MINOR__ != 2 || __GNUC_PATCHLEVEL__ != 1
#error              GNU identity must match Clang in every mode
#endif
#if !defined(__linux__) ||              defined(_WIN32)
#error default target must identify Linux
#endif
",
        );
        for mode in [
            "c89",
            "iso9899:199409",
            "c99",
            "c11",
            "c17",
            "c23",
            "c2y",
            "gnu17",
        ] {
            let output = bcc(&[&format!("-std={mode}"), "--tokens", "--input", &source]);
            clean(&output);
            let text = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                text.lines()
                    .filter(|line| line.contains("string literal"))
                    .count(),
                count
            );
            let actual: Vec<_> = text
                .lines()
                .filter_map(|line| {
                    line.split_once("string literal ")
                        .map(|(_, literal)| normalize(literal))
                })
                .collect();
            assert_eq!(actual, expected, "{mode}");
        }
    }

    #[test]
    fn user_headers_override_resources_and_provenance_is_stable() {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/freestanding-header-precedence");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("stddef.h"), "int user_header;\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(["--tokens", "--input", "#include <stddef.h>\n"])
            .env_remove("CPATH")
            .env("C_INCLUDE_PATH", &directory)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        clean(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("user_header"));
        fs::write(
            directory.join("limits.h"),
            "#include_next <limits.h>\nint resource_limit[CHAR_BIT == 8 ? 1 : -1];\n",
        )
        .unwrap();
        clean(&bcc(&[
            "--isystem",
            directory.to_str().unwrap(),
            "--input",
            "#include <limits.h>\n",
        ]));
        for (option, include) in [("--isystem", "<stddef.h>"), ("--iquote", "\"stddef.h\"")] {
            let source = format!("#include {include}\n");
            let output = bcc(&[
                option,
                directory.to_str().unwrap(),
                "--tokens",
                "--input",
                &source,
            ]);
            clean(&output);
            assert!(String::from_utf8_lossy(&output.stderr).contains("user_header"));
        }
        let output = bcc(&["--tokens", "--input", "#include <stddef.h>\n"]);
        clean(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("<built-in>/stddef.h:"));
        let output = bcc(&["--input", "#include <absent-resource.h>\n"]);
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("searched these directories:\n            <built-in>")
        );
    }

    #[test]
    fn stddef_h_follows_clang_for_msvc() {
        // Like Clang's, the header defines vcruntime.h's `_WCHAR_T_DEFINED`
        // guard under `_MSC_EXTENSIONS` and makes `max_align_t` a `double`
        // under `_MSC_VER`.
        let source = "#include <stddef.h>\n#ifndef _WCHAR_T_DEFINED\n#error no \
                      guard\n#endif\n_Static_assert(sizeof(max_align_t) == 8 && \
                      _Alignof(max_align_t) == 8, \"max_align_t\");\nwchar_t w = L'x';\n";
        clean(&bcc(&[
            "--target=x86_64-pc-windows-msvc",
            "-fms-extensions",
            "-std=c11",
            "-pedantic",
            "--input",
            source,
        ]));
        let probe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/msvc-stddef-probe.c");
        fs::write(&probe, source).unwrap();
        let output = Command::new(clang())
            .args([
                "--target=x86_64-pc-windows-msvc",
                "-fms-extensions",
                "-std=c11",
                "-ffreestanding",
                "-fsyntax-only",
            ])
            .arg(&probe)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Other targets keep the GCC-compatible record.
        clean(&bcc(&[
            "-std=c11",
            "--input",
            "#include <stddef.h>\n_Static_assert(sizeof(max_align_t) == 32, \"\");\n#ifdef \
             _WCHAR_T_DEFINED\n#error guard\n#endif\n",
        ]));
    }

    #[test]
    fn modern_header_macros_follow_clang_mode_gates() {
        for (mode, modern_bool, alignment) in [
            ("c89", false, false),
            ("c99", false, true),
            ("c11", false, true),
            ("c17", false, true),
            ("c23", true, false),
            ("c2y", true, false),
        ] {
            let source = format!(
                "#include <stdbool.h>\n#include <stdalign.h>\n#include <stdnoreturn.h>\n#if \
                 defined(bool) != {} || defined(alignas) != {} || !defined(noreturn)\n#error \
                 wrong mode gates\n#endif\nint value;\n",
                u8::from(!modern_bool),
                u8::from(alignment)
            );
            clean(&bcc(&[&format!("-std={mode}"), "--input", &source]));
        }
        clean(&bcc(&[
            "-std=c23",
            "-pedantic-errors",
            "--input",
            "#include <stdbool.h>\nbool value = true;\n",
        ]));
    }

    #[test]
    fn varargs_have_real_types_and_diagnose_invalid_operands() {
        clean(&bcc(&[
            "-std=c99",
            "-pedantic-errors",
            "--input",
            "int f(int n, ...) { __builtin_va_list ap; __builtin_va_start(ap,n); int \
             value=__builtin_va_arg(ap,int); __builtin_va_end(ap); return value; }\n",
        ]));
        for (source, message) in [
            (
                "#include <stdarg.h>\nvoid f(void) { va_list ap; va_start(ap, ap); }\n",
                "va_start requires a variadic function",
            ),
            (
                "#include <stdarg.h>\nvoid f(int n, ...) { va_list ap; va_arg(ap, void); }\n",
                "va_arg requires a complete object type",
            ),
            (
                "#include <stdarg.h>\nvoid f(void) { va_end(1); }\n",
                "varargs builtin requires a modifiable va_list operand",
            ),
            (
                "#include <stdarg.h>\nvoid f(int n, ...) { va_list ap; int *p=va_arg(ap,int); }\n",
                "initializer",
            ),
            (
                "#include <stddef.h>\nstruct S {int bit:3;}; int x=offsetof(struct S,bit);\n",
                "offsetof requires a valid non-bit-field member path",
            ),
        ] {
            let output = bcc(&["--input", source]);
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(message),
                "{output:?}"
            );
        }
    }
}
