//! The execution environment, header search order, resource-header chaining
//! and compiler identity, exercised through the CLI.

#![expect(
    unused_crate_dependencies,
    reason = "CLI integration tests exercise the compiled binary."
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
        path::{
            Path,
            PathBuf,
        },
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

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn clean(output: &Output) {
        let text = stderr(output);
        assert!(
            output.status.success() && !text.contains("error:") && !text.contains("warning:"),
            "{text}"
        );
    }

    /// The identifiers in a `--tokens` dump, in order.
    fn identifiers(output: &Output) -> Vec<String> {
        stderr(output)
            .lines()
            .filter_map(|line| line.split_once("identifier `"))
            .map(|(_, rest)| rest.trim_end_matches('`').to_owned())
            .collect()
    }

    /// The directories a missing-header diagnostic lists, in order.
    fn searched(output: &Output) -> Vec<String> {
        stderr(output)
            .lines()
            .skip_while(|line| !line.contains("searched these directories:"))
            .skip(1)
            .take_while(|line| !line.contains("= help"))
            .map(|line| line.trim().to_owned())
            .collect()
    }

    /// A fresh directory under `target/` for one test's header trees.
    fn scratch(name: &str) -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/hosted-cli")
            .join(name);
        drop(fs::remove_dir_all(&directory));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn arg(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    #[test]
    fn execution_environment_is_hosted_unless_freestanding_and_the_last_flag_wins() {
        let source =
            "#if __STDC_HOSTED__ == 1\nhosted\n#elif __STDC_HOSTED__ == 0\nfreestanding\n#endif\n";
        for (flags, expected) in [
            (&[][..], "hosted"),
            (&["-ffreestanding"][..], "freestanding"),
            (&["-fhosted"][..], "hosted"),
            (&["-ffreestanding", "-fhosted"][..], "hosted"),
            (&["-fhosted", "-ffreestanding"][..], "freestanding"),
            (
                &["-std=c89", "-pedantic-errors", "-ffreestanding"][..],
                "freestanding",
            ),
        ] {
            let mut args = flags.to_vec();
            args.extend(["--tokens", "--input", source]);
            let output = bcc(&args);
            clean(&output);
            assert_eq!(identifiers(&output), [expected], "{flags:?}");
        }
    }

    #[test]
    fn main_signature_is_checked_only_in_a_hosted_environment() {
        let source = "void main(void) {}\n";
        let hosted = bcc(&["--input", source]);
        assert!(
            stderr(&hosted).contains("main has a nonportable signature"),
            "{hosted:?}"
        );
        clean(&bcc(&["-ffreestanding", "--input", source]));
    }

    #[test]
    fn resource_headers_chain_to_a_glibc_like_library_only_when_hosted() {
        let sysroot = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hosted/sysroot");
        let probe = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/hosted/probe.c");
        for mode in [
            "-std=gnu99",
            "-std=gnu17",
            "-std=c99",
            "-std=c11",
            "-std=c23",
        ] {
            for environment in ["-fhosted", "-ffreestanding"] {
                let output = bcc(&[mode, environment, "--sysroot", sysroot, probe]);
                clean(&output);
            }
        }
        // Without the library the hosted resource headers stand alone.
        let output = bcc(&[
            "--input",
            "#include <limits.h>\n#include <stdint.h>\n_Static_assert(MB_LEN_MAX == 4, \
             \"\");\n_Static_assert(sizeof(int64_t) == 8, \"\");\n",
        ]);
        clean(&output);
    }

    #[test]
    fn headers_from_system_directories_withhold_warnings_and_extensions_but_not_errors() {
        let root = scratch("system-headers");
        // One header per search group, each warning about an undefined
        // name and using `long long`, a C99 extension in C89.
        let groups = [
            ("quote", "-iquote", false),
            ("user", "-I", false),
            ("cpath", "CPATH", false),
            ("system", "-isystem", true),
            ("c-include-path", "C_INCLUDE_PATH", true),
            ("sysroot/usr/include", "--sysroot", true),
            ("after", "-idirafter", true),
        ];
        let mut source = String::new();
        for (directory, _, _) in groups {
            let name = directory.replace(['/', '-'], "_");
            write(
                &root.join(directory).join(format!("{name}.h")),
                &format!("#if UNDEFINED_IN_{name}\n#endif\nlong long {name}_value;\n"),
            );
            writeln!(source, "#include \"{name}.h\"").unwrap();
        }
        let mut args = vec!["-std=c89".to_owned(), "-pedantic-errors".to_owned()];
        for (directory, flag, _) in groups {
            match flag {
                | "CPATH" | "C_INCLUDE_PATH" => {},
                | "--sysroot" =>
                    args.extend([flag.to_owned(), arg(&root.join("sysroot")).to_owned()]),
                | _ => args.extend([flag.to_owned(), arg(&root.join(directory)).to_owned()]),
            }
        }
        args.extend(["--input".to_owned(), source]);
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(&args)
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .env("CPATH", root.join("cpath"))
            .env("C_INCLUDE_PATH", root.join("c-include-path"))
            .output()
            .unwrap();
        let text = stderr(&output);
        for (directory, flag, system) in groups {
            let name = directory.replace(['/', '-'], "_");
            assert_eq!(
                !text.contains(&format!("UNDEFINED_IN_{name}")),
                system,
                "{flag}: {text}"
            );
            assert_eq!(
                !text.contains(&format!("{name}.h:3:6")),
                system,
                "{flag}: {text}"
            );
        }
        // Errors are never withheld.
        write(&root.join("system").join("broken.h"), "int broken = ;\n");
        let output = bcc(&[
            "-isystem",
            arg(&root.join("system")),
            "--input",
            "#include <broken.h>\n",
        ]);
        assert!(stderr(&output).contains("error:"), "{output:?}");
    }

    #[test]
    fn float_h_chains_only_for_windows_runtimes_and_keeps_the_targets_values() {
        // MinGW-w64's <float.h> adds Windows definitions and would chain to
        // GCC's own unless that header's guard, `_FLOAT_H___`, is defined.
        let library = scratch("mingw-float").join("include");
        write(
            &library.join("float.h"),
            "#ifndef _FLOAT_H___\n#include_next <float.h>\n#endif\n#define FLT_DIG 99\n#define \
             _MCW_EM 0x0008001F\n",
        );
        let check = "#include <float.h>\n_Static_assert(FLT_DIG == 6, \"\");\n#ifdef \
                     _MCW_EM\nwindows_additions\n#endif\n";
        for (prefix, expected) in [
            ("#define __MINGW32__ 1\n", &["windows_additions"][..]),
            ("", &[][..]),
        ] {
            let source = format!("{prefix}{check}");
            for flags in [&["-fhosted"][..], &["-ffreestanding"][..]] {
                let mut args = flags.to_vec();
                args.extend(["-idirafter", arg(&library), "--tokens", "--input", &source]);
                let output = bcc(&args);
                clean(&output);
                let hosted = flags == ["-fhosted"];
                assert_eq!(
                    identifiers(&output),
                    if hosted { expected } else { &[] },
                    "{prefix:?} {flags:?}"
                );
            }
        }
        // `-fms-extensions` claims MSVC, whose runtime chains the same way.
        let output = bcc(&[
            "-fms-extensions",
            "-idirafter",
            arg(&library),
            "--tokens",
            "--input",
            check,
        ]);
        clean(&output);
        assert_eq!(identifiers(&output), ["windows_additions"]);
    }

    #[test]
    fn mm_malloc_h_allocates_through_each_runtimes_aligned_allocator() {
        // Strict modes reject a call to an undeclared function, so each
        // allocator the header calls must be declared for its target.
        let use_ =
            "void *f(void) {\n  void *p = _mm_malloc(64, 16);\n  _mm_free(p);\n  return p;\n}\n";
        let root = scratch("mm-malloc");
        let stdlib = "#include <stddef.h>\nvoid *malloc(size_t);\nvoid free(void *);\n";
        // A POSIX library declares posix_memalign only on request; the header
        // declares it itself, as Clang's does.
        let posix = root.join("posix");
        write(&posix.join("stdlib.h"), stdlib);
        // MinGW-w64's <malloc.h> declares its aligned allocator and then
        // includes <mm_malloc.h>, which includes <malloc.h> again.
        let mingw = root.join("mingw");
        write(&mingw.join("stdlib.h"), stdlib);
        write(
            &mingw.join("malloc.h"),
            "#ifndef _MALLOC_H_\n#define _MALLOC_H_\n#include <stddef.h>\nvoid \
             *__mingw_aligned_malloc(size_t, size_t);\nvoid __mingw_aligned_free(void \
             *);\n#include <mm_malloc.h>\n#endif\n",
        );
        // The MSVC runtime's <malloc.h> declares `_aligned_malloc`, and may
        // define both names as macros, which the header then leaves alone.
        let msvc = root.join("msvc");
        write(&msvc.join("stdlib.h"), stdlib);
        write(
            &msvc.join("malloc.h"),
            "#include <stddef.h>\nvoid *_aligned_malloc(size_t, size_t);\nvoid _aligned_free(void \
             *);\n#ifdef MM_MACROS\n#define _mm_malloc(a, b) _aligned_malloc(a, b)\n#define \
             _mm_free(a) _aligned_free(a)\n#endif\n",
        );
        for standard in ["-std=c99", "-std=c17", "-std=gnu17"] {
            for (flags, library, include) in [
                (
                    &["--target=x86_64-unknown-linux-gnu"][..],
                    &posix,
                    "mm_malloc.h",
                ),
                (&["--target=x86_64-w64-windows-gnu"][..], &mingw, "malloc.h"),
                (
                    &["--target=x86_64-w64-windows-gnu"][..],
                    &mingw,
                    "mm_malloc.h",
                ),
                (
                    &["--target=x86_64-pc-windows-msvc", "-fms-extensions"][..],
                    &msvc,
                    "mm_malloc.h",
                ),
            ] {
                for prefix in ["", "#define MM_MACROS\n"] {
                    let source = format!("{prefix}#include <{include}>\n{use_}");
                    let mut args = vec![standard];
                    args.extend_from_slice(flags);
                    args.extend(["-idirafter", arg(library), "--input", &source]);
                    clean(&bcc(&args));
                }
            }
        }
    }

    /// One directory per search group. Each holds `chain.h`, which names
    /// its group and continues with `#include_next`, so a lookup records
    /// every place it visits in order; the last group ends the chain.
    struct SearchTree {
        root:   PathBuf,
        quote:  PathBuf,
        user:   PathBuf,
        cpath:  PathBuf,
        system: PathBuf,
        c_path: PathBuf,
        after:  PathBuf,
    }

    impl SearchTree {
        fn new(name: &str) -> Self {
            let root = scratch(name);
            let tree = Self {
                quote: root.join("quote"),
                user: root.join("user"),
                cpath: root.join("cpath"),
                system: root.join("system"),
                c_path: root.join("c-path"),
                after: root.join("after"),
                root,
            };
            for (directory, name) in [
                (tree.quote.clone(), "quote_dir"),
                (tree.user.clone(), "user_dir"),
                (tree.cpath.clone(), "cpath_dir"),
                (tree.system.clone(), "isystem_dir"),
                (tree.c_path.clone(), "c_include_path_dir"),
                (
                    tree.sysroot().join("usr/local/include"),
                    "usr_local_include",
                ),
                (tree.sysroot().join("usr/include"), "usr_include"),
            ] {
                write(
                    &directory.join("chain.h"),
                    &format!("{name}\n#include_next <chain.h>\n"),
                );
            }
            write(&tree.after.join("chain.h"), "idirafter_dir\n");
            // Resource header names in a group before the resource directory
            // and in the C library after it.
            write(&tree.c_path.join("stddef.h"), "c_include_path_stddef\n");
            write(
                &tree.sysroot().join("usr/include/stdbool.h"),
                "libc_stdbool\n",
            );
            tree
        }

        fn sysroot(&self) -> PathBuf {
            self.root.join("sysroot")
        }

        fn run(&self, flags: &[&str], source: &str) -> Output {
            let sysroot = self.sysroot();
            let mut args = vec![
                "-iquote",
                arg(&self.quote),
                "-isystem",
                arg(&self.system),
                "-I",
                arg(&self.user),
                "-idirafter",
                arg(&self.after),
                "--sysroot",
                arg(&sysroot),
            ];
            args.extend_from_slice(flags);
            args.extend(["--tokens", "--input", source]);
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(&args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env("CPATH", &self.cpath)
                .env("C_INCLUDE_PATH", &self.c_path)
                .output()
                .unwrap()
        }
    }

    #[test]
    fn header_search_follows_clang_with_the_resource_directory_before_the_library() {
        let tree = SearchTree::new("order");
        let output = tree.run(&[], "#include \"chain.h\"\n");
        clean(&output);
        assert_eq!(
            identifiers(&output),
            [
                "quote_dir",
                "user_dir",
                "cpath_dir",
                "isystem_dir",
                "c_include_path_dir",
                "usr_local_include",
                "usr_include",
                "idirafter_dir",
            ]
        );
        // `<…>` skips the `-iquote` directory.
        let output = tree.run(&[], "#include <chain.h>\n");
        clean(&output);
        assert_eq!(identifiers(&output)[0], "user_dir");
        // Command-line and environment groups precede the resource
        // directory, which precedes the C library.
        let output = tree.run(&[], "#include <stddef.h>\n#include <stdbool.h>\nbool\n");
        clean(&output);
        let text = stderr(&output);
        assert_eq!(identifiers(&output), ["c_include_path_stddef"], "{text}");
        assert!(text.contains("<built-in>/stdbool.h"), "{text}");
    }

    #[test]
    fn nostdinc_options_remove_the_resource_and_library_directories() {
        let tree = SearchTree::new("nostdinc");
        for (flag, resource, library) in [
            ("-nostdinc", false, false),
            ("--nostdinc", false, false),
            ("-nobuiltininc", false, true),
            ("-nostdlibinc", true, false),
        ] {
            let output = tree.run(&[flag], "#include <chain.h>\n");
            clean(&output);
            let visited = identifiers(&output);
            assert_eq!(
                visited.iter().any(|name| name.starts_with("usr_")),
                library,
                "{flag}: {visited:?}"
            );
            // `-idirafter` is the user's and stays.
            assert_eq!(visited.last().unwrap(), "idirafter_dir", "{flag}");
            let output = tree.run(&[flag], "#include <stdbool.h>\nbool\n");
            let text = stderr(&output);
            assert_eq!(text.contains("<built-in>/stdbool.h"), resource, "{flag}");
            assert_eq!(
                identifiers(&output).first().map(String::as_str) == Some("libc_stdbool"),
                !resource && library,
                "{flag}: {text}"
            );
        }
    }

    #[test]
    fn search_options_accept_gcc_and_clang_spellings() {
        let root = scratch("spellings");
        let probe = root.join("probe");
        write(&probe.join("probe.h"), "found_probe\n");
        let joined_i = format!("-I{}", arg(&probe));
        let joined_isystem = format!("-isystem{}", arg(&probe));
        let joined_iquote = format!("-iquote{}", arg(&probe));
        let joined_idirafter = format!("-idirafter{}", arg(&probe));
        let equals = format!("--isystem={}", arg(&probe));
        for (args, include) in [
            (vec![joined_i.as_str()], "<probe.h>"),
            (vec![joined_isystem.as_str()], "<probe.h>"),
            (vec![joined_iquote.as_str()], "\"probe.h\""),
            (vec!["-iquote", arg(&probe)], "\"probe.h\""),
            (vec![joined_idirafter.as_str()], "<probe.h>"),
            (vec!["--include-directory", arg(&probe)], "<probe.h>"),
            (vec!["-idirafter", arg(&probe)], "<probe.h>"),
            (vec![equals.as_str()], "<probe.h>"),
        ] {
            let source = format!("#include {include}\n");
            let mut full = args.clone();
            full.extend(["--tokens", "--input", &source]);
            let output = bcc(&full);
            clean(&output);
            assert_eq!(identifiers(&output), ["found_probe"], "{args:?}");
        }
        // A missing header lists every place in order, the library last.
        let sysroot = format!("--sysroot={}", arg(&root));
        let output = bcc(&[
            &sysroot,
            "-I",
            arg(&probe),
            "--input",
            "#include <absent.h>\n",
        ]);
        assert_eq!(
            searched(&output),
            [
                probe.display().to_string(),
                "<built-in>".to_owned(),
                root.join("usr")
                    .join("local")
                    .join("include")
                    .display()
                    .to_string(),
                root.join("usr").join("include").display().to_string(),
            ],
            "{}",
            stderr(&output)
        );
    }

    /// The GNU identity and inline-semantics macros Clang predefines.
    const GNU_IDENTITY: [&str; 5] = [
        "__GNUC__",
        "__GNUC_MINOR__",
        "__GNUC_PATCHLEVEL__",
        "__GNUC_GNU_INLINE__",
        "__GNUC_STDC_INLINE__",
    ];

    /// `(name, value)` for each defined name of `GNU_IDENTITY`, as bcc
    /// predefines them in `mode`.
    fn bcc_gnu_identity(mode: &str) -> Vec<(String, String)> {
        let mut source = String::new();
        for name in GNU_IDENTITY {
            writeln!(source, "#ifdef {name}\nfound{name} {name}\n#endif").unwrap();
        }
        let output = bcc(&[mode, "--tokens", "--input", &source]);
        clean(&output);
        let text = stderr(&output);
        let mut lines = text.lines();
        let mut defined = Vec::new();
        while let Some(line) = lines.next() {
            if let Some((_, name)) = line.split_once("identifier `found") {
                let value = lines.next().unwrap();
                let value = value.split('`').nth(1).unwrap();
                defined.push((name.trim_end_matches('`').to_owned(), value.to_owned()));
            }
        }
        defined
    }

    fn clang() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) {
            "target/llvm/bin/clang.exe"
        } else {
            "target/llvm/bin/clang"
        })
    }

    #[test]
    fn gnu_modes_claim_the_gnu_identity_clang_claims_and_strict_modes_none() {
        for mode in [
            "-std=gnu89",
            "-std=gnu99",
            "-std=gnu11",
            "-std=gnu17",
            "-std=gnu23",
        ] {
            let dump = Command::new(clang())
                .args([
                    "--target=x86_64-unknown-linux-gnu",
                    mode,
                    "-dM",
                    "-E",
                    "-x",
                    "c",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                ])
                .output()
                .unwrap();
            assert!(dump.status.success());
            let mut expected: Vec<_> = String::from_utf8_lossy(&dump.stdout)
                .lines()
                .filter_map(|line| {
                    let mut words = line.split_whitespace().skip(1);
                    let name = words.next()?;
                    let value = words.next()?;
                    GNU_IDENTITY
                        .contains(&name)
                        .then(|| (name.to_owned(), value.to_owned()))
                })
                .collect();
            let mut actual = bcc_gnu_identity(mode);
            expected.sort();
            actual.sort();
            assert_eq!(actual, expected, "{mode}");
        }
        // The user chose GNU identity for GNU modes only, unlike Clang.
        for mode in ["-std=c89", "-std=c99", "-std=c17", "-std=c23"] {
            assert!(bcc_gnu_identity(mode).is_empty(), "{mode}");
        }
    }

    #[test]
    fn msvc_identity_needs_the_umbrella_flag_and_clang_is_never_claimed() {
        let source = "#ifdef _MSC_VER\nms _MSC_VER _MSC_FULL_VER _MSC_BUILD \
                      _MSC_EXTENSIONS\n#endif\n#ifdef __clang__\nclang\n#endif\n#if __bcc__ == 1 \
                      && defined __bcc_version__\nbcc __bcc_major__ __bcc_minor__ \
                      __bcc_patchlevel__\n#endif\n";
        let constants = |output: &Output| -> Vec<String> {
            stderr(output)
                .lines()
                .filter_map(|line| line.split('`').nth(1))
                .map(str::to_owned)
                .collect()
        };
        let version = [
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
        ];
        for mode in ["-std=c99", "-std=gnu17"] {
            let output = bcc(&[mode, "-fms-extensions", "--tokens", "--input", source]);
            clean(&output);
            let mut expected = vec!["ms", "1933", "193300000", "1", "1", "bcc"];
            expected.extend(version);
            assert_eq!(constants(&output), expected, "{mode}");
            for flags in [
                &["-fms-declspec"][..],
                &["-fms-extensions", "-fno-ms-extensions"][..],
                &[][..],
            ] {
                let mut args = vec![mode];
                args.extend_from_slice(flags);
                args.extend(["--tokens", "--input", source]);
                let output = bcc(&args);
                clean(&output);
                let mut expected = vec!["bcc"];
                expected.extend(version);
                assert_eq!(constants(&output), expected, "{mode} {flags:?}");
            }
        }
    }
}
