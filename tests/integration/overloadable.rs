//! Clang overload selection and address symbols, including real resource-header
//! arithmetic promotions and unavailable fallbacks without the host C library.
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
    fn overload_addresses_match_clang_symbols_and_reject_ambiguity() {
        let clang = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(if cfg!(windows) { "clang.exe" } else { "clang" });
        for target in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"] {
            let (int_symbol, double_symbol) = if target.ends_with("windows-msvc") {
                ("\"?o@@$$J0YAHH@Z\"", "\"?o@@$$J0YAHN@Z\"")
            } else {
                ("_Z1oi", "_Z1od")
            };
            for case in ["", "AMBIGUOUS_ADDRESS", "AMBIGUOUS_DECAY"] {
                for compiler in [clang.as_path(), Path::new(env!("CARGO_BIN_EXE_bcc-rust"))] {
                    let mut command = Command::new(compiler);
                    _ = command.args([&format!("--target={target}"), "-std=gnu17"]);
                    if !case.is_empty() {
                        _ = command.arg(format!("-D{case}"));
                    }
                    if compiler == clang {
                        _ = command.args(["-S", "-emit-llvm", "-O0", "-o", "-"]);
                    } else {
                        _ = command.args(["--emit=llvm", "--opt=none"]);
                    }
                    let output = command
                        .arg("tests/fixtures/overloadable/address.c")
                        .env("NO_COLOR", "1")
                        .env_remove("CPATH")
                        .env_remove("C_INCLUDE_PATH")
                        .output()
                        .unwrap();
                    let diagnostics = String::from_utf8_lossy(&output.stderr);
                    if !case.is_empty() {
                        assert!(!output.status.success(), "{command:?}");
                        assert!(diagnostics.contains("error:"), "{command:?}\n{diagnostics}");
                        assert!(output.stdout.is_empty(), "{command:?}");
                        continue;
                    }
                    assert!(output.status.success(), "{command:?}\n{diagnostics}");
                    let llvm = String::from_utf8_lossy(&output.stdout);
                    for (global, symbol) in [
                        ("global_int", int_symbol),
                        ("global_double", double_symbol),
                        ("global_decay_double", double_symbol),
                        ("static_int", int_symbol),
                        ("static_double", double_symbol),
                    ] {
                        let line = llvm
                            .lines()
                            .find(|line| {
                                line.starts_with('@')
                                    && line.contains(global)
                                    && line.contains(" = ")
                            })
                            .unwrap();
                        assert!(
                            line.contains(&format!("ptr @{symbol}"))
                                && !line.contains("always_inline"),
                            "{command:?}\n{line}"
                        );
                    }
                    for (suffix, symbol, other) in [
                        ("int", int_symbol, double_symbol),
                        ("double", double_symbol, int_symbol),
                    ] {
                        for form in ["automatic", "assignment", "cast", "argument"] {
                            let header = format!("@{form}_{suffix}(");
                            let body = llvm
                                .lines()
                                .skip_while(|line| {
                                    !line.starts_with("define ") || !line.contains(&header)
                                })
                                .skip(1)
                                .take_while(|line| *line != "}")
                                .collect::<Vec<_>>()
                                .join("\n");
                            assert!(
                                body.contains(&format!("@{symbol}")),
                                "{command:?}\n{header}\n{body}"
                            );
                            assert!(
                                !body.contains(&format!("@{other}"))
                                    && !body.contains("always_inline"),
                                "{command:?}\n{header}\n{body}"
                            );
                        }
                    }
                }
            }
        }
    }

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

    /// Under the GCC identity the `_FloatN` types are distinct, and the
    /// resource `<tgmath.h>` sends each to the functions of the standard
    /// type with its format, as GCC's `__builtin_tgmath` reaches the
    /// `_FloatN` functions with the same values. Without the types the
    /// fixture's names are unknown.
    #[test]
    fn tgmath_resource_takes_floatn_arguments_under_the_gcc_identity() {
        for target in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "x86_64-w64-windows-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            for standard in ["c11", "gnu17", "c23"] {
                for (options, succeeds) in [
                    (&["--compiler-identity=gcc"][..], true),
                    (&["-ffloatn-types"], true),
                    (&["--compiler-identity=gcc", "-fno-floatn-types"], false),
                    (
                        &["--compiler-identity=gcc", "-DNEXTTOWARD_FIRST=_Float32"],
                        false,
                    ),
                    (
                        &["--compiler-identity=gcc", "-DNEXTTOWARD_FIRST=_Float64"],
                        false,
                    ),
                    (
                        &["--compiler-identity=gcc", "-DNEXTTOWARD_FIRST=_Float32x"],
                        false,
                    ),
                    (
                        &["--compiler-identity=gcc", "-DNEXTTOWARD_FIRST=double"],
                        true,
                    ),
                ] {
                    let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                        .args([
                            &format!("--target={target}"),
                            &format!("-std={standard}"),
                            "-nostdlibinc",
                            "-isystem",
                            "tests/fixtures/overloadable",
                        ])
                        .args(options)
                        .arg("tests/fixtures/overloadable/tgmath-floatn.c")
                        .env("NO_COLOR", "1")
                        .env_remove("CPATH")
                        .env_remove("C_INCLUDE_PATH")
                        .output()
                        .unwrap();
                    let diagnostics = String::from_utf8_lossy(&output.stderr);
                    assert_eq!(
                        output.status.success() && !diagnostics.contains("error:"),
                        succeeds,
                        "{target} {standard} {options:?}:\n{diagnostics}"
                    );
                }
            }
        }
    }

    /// `conj` of a real argument gives imaginary part -0 under the GCC
    /// identity, as GCC's and glibc's do, for the standard types, integers
    /// and the `_FloatN` types, and +0 otherwise, as the pinned Clang's
    /// header gives. The fixture expects the sign of the identity it sees.
    #[test]
    fn tgmath_conj_of_a_real_argument_follows_the_identity() {
        for options in [
            &[][..],
            &["-std=c11"],
            &["--compiler-identity=gcc"],
            &["--compiler-identity=gcc", "-std=c11", "-fno-floatn-types"],
            &["--compiler-identity=clang-cl", "-fms-extensions"],
            &["--compiler-identity=tcc", "-std=gnu99"],
            &["-ffloatn-types"],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(["-nostdlibinc", "-isystem", "tests/fixtures/overloadable"])
                .args(options)
                .args(["--interpret", "tests/fixtures/overloadable/tgmath-conj.c"])
                .env("NO_COLOR", "1")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{options:?}:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
