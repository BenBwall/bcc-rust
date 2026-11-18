//! The probes build systems run to identify the compiler, against each
//! driver personality: Meson's `-Wl,--version` linker check, `CMake`'s `-v`
//! link line, extension default and `check_c_compiler_flag`, Autoconf's
//! `AC_SEARCH_LIBS([sqrt], [m])`, and libtool's `-fno-rtti` check and
//! `-print-search-dirs`. Each test runs the probe as the tool does and
//! checks what the tool concludes from it, which was compared with GCC 13,
//! GCC 14's cc1, Clang 23 and TCC 0.9.27 (`docs/design/driver-modes.md`,
//! "Build systems"). `scripts/build_system_probes.py` runs `CMake`, Autoconf,
//! libtool and Meson themselves.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    /// A fresh working directory, removed afterwards.
    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-build-probes-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        /// Runs bcc in this directory with `mode` first, unless it is empty.
        fn bcc(&self, mode: &str, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args((!mode.is_empty()).then_some(mode))
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .env_remove("CL")
                .env_remove("_CL_")
                .output()
                .unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// Whether the host links programs for itself, which the probes that
    /// run the linker need.
    const LINKS: bool = cfg!(all(target_os = "linux", target_env = "gnu"));

    /// Meson names the linker from what `CC -Wl,--version` prints, which
    /// both compilers' drivers answer by running the linker: a linker
    /// option is an input that links (GCC counts `-l`, `-Wl,` and
    /// `-Xlinker`, Clang every linker input). bcc links with LLD, whose
    /// first line Meson reads as `ld.lld`. Without the link, a linker input
    /// alone leaves nothing to do: GCC succeeds silently, Clang after its
    /// warning; GCC still has no input in `-z now`.
    #[test]
    fn meson_linker_probe_runs_the_linker() {
        if !LINKS {
            return;
        }
        let directory = Directory::new();
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-Wl,--version"]);
            assert_eq!(output.status.code(), Some(0), "{mode}: {}", stderr(&output));
            let text = stdout(&output);
            let first = text.lines().next().unwrap_or_default();
            assert!(
                first.starts_with("LLD ") && first.contains("compatible with GNU linkers"),
                "{mode}: {text}"
            );
            assert!(!directory.0.join("a.out").exists(), "{mode}");
        }
        let gcc = directory.bcc("--gcc-mode", &["-c", "-lm"]);
        assert_eq!((gcc.status.code(), stderr(&gcc).as_str()), (Some(0), ""));
        let clang = directory.bcc("--clang-mode", &["-c", "-lm"]);
        assert_eq!(
            (clang.status.code(), stderr(&clang).as_str()),
            (
                Some(0),
                "clang: warning: -lm: 'linker' input unused [-Wunused-command-line-argument]\n"
            )
        );
        let gcc = directory.bcc("--gcc-mode", &["-z", "now"]);
        assert_eq!(
            (gcc.status.code(), stderr(&gcc).as_str()),
            (
                Some(1),
                "gcc: fatal error: no input files\ncompilation terminated.\n"
            )
        );
        // Clang counts `-z` as a linker input, so it links and finds no
        // `main`.
        let clang = directory.bcc("--clang-mode", &["-z", "now"]);
        assert_eq!(clang.status.code(), Some(1));
        assert!(
            stderr(&clang).contains("undefined symbol: main"),
            "{}",
            stderr(&clang)
        );
    }

    /// `CMake` links its ABI probe with `-v -Wl,-v` and reads the linker's
    /// command line from what the driver prints: the libraries and
    /// directories every link gets (`CMAKE_C_IMPLICIT_LINK_LIBRARIES`,
    /// `gcc;gcc_s;c;gcc;gcc_s` with GCC and Clang alike) and the linker,
    /// which it then asks for its version. `libm` is not one of them.
    #[test]
    fn cmake_reads_the_linker_line_from_a_verbose_link() {
        if !LINKS {
            return;
        }
        let directory = Directory::new();
        directory.write("abi.c", "int main(void) { return 0; }\n");
        for mode in ["--gcc-mode", "--clang-mode"] {
            let output = directory.bcc(mode, &["-v", "-Wl,-v", "abi.c", "-o", "abi"]);
            assert_eq!(output.status.code(), Some(0), "{mode}: {}", stderr(&output));
            let text = stderr(&output);
            let linker = text
                .lines()
                .find(|line| {
                    line.split_whitespace()
                        .next()
                        .is_some_and(|program| program.trim_matches('"').ends_with("/ld.lld"))
                })
                .unwrap_or_else(|| panic!("{mode}: no linker line in\n{text}"));
            let words: Vec<&str> = linker
                .split_whitespace()
                .map(|word| word.trim_matches('"'))
                .collect();
            for library in ["-lgcc", "-lgcc_s", "-lc"] {
                assert!(words.contains(&library), "{mode}: {linker}");
            }
            assert!(!words.contains(&"-lm"), "{mode}: {linker}");
            assert!(
                words.iter().any(|word| word.starts_with("-L")),
                "{mode}: {linker}"
            );
            // GCC's `-v` quotes no word, Clang's every word.
            assert_eq!(
                linker.starts_with(" \""),
                mode == "--clang-mode",
                "{linker}"
            );
            // `-Wl,-v` makes the linker print its version.
            assert!(
                stdout(&output).starts_with("LLD "),
                "{mode}: {}",
                stdout(&output)
            );
        }
    }

    /// Autoconf's `AC_SEARCH_LIBS([sqrt], [m])` links this program without
    /// `-lm` first: GCC's, Clang's and TCC's drivers link only what `-lm`
    /// asks for, so the search settles on `-lm`, which the project then
    /// records (`LIBS`, its `.pc` and `.la` files). bcc's own driver links
    /// `libm` whatever the command line says.
    #[test]
    fn autoconf_finds_sqrt_in_the_math_library() {
        if !LINKS {
            return;
        }
        let directory = Directory::new();
        directory.write(
            "conftest.c",
            "char sqrt (void);\nint main (void) { return sqrt (); }\n",
        );
        for mode in ["--gcc-mode", "--clang-mode", "--tcc-mode"] {
            let without = directory.bcc(mode, &["-o", "conftest", "conftest.c"]);
            assert_eq!(without.status.code(), Some(1), "{mode}");
            assert!(
                stderr(&without).contains("sqrt"),
                "{mode}: {}",
                stderr(&without)
            );
            let with = directory.bcc(mode, &["-o", "conftest", "conftest.c", "-lm"]);
            assert_eq!(with.status.code(), Some(0), "{mode}: {}", stderr(&with));
        }
        let own = directory.bcc("", &["-o", "conftest", "conftest.c"]);
        assert_eq!(own.status.code(), Some(0), "{}", stderr(&own));
    }

    /// `CMake`'s `check_c_compiler_flag` fails a flag the compiler warns
    /// about (`unknown warning option`): Clang warns about an unknown
    /// `-Wno-` option as about any unknown warning option, while GCC
    /// ignores an unknown `-Wno-` option unless another diagnostic follows.
    #[test]
    fn cmake_flag_check_sees_unknown_negated_warnings_as_each_compiler_does() {
        let directory = Directory::new();
        directory.write("src.c", "int main(void) { return 0; }\n");
        let clang = directory.bcc("--clang-mode", &["-Wno-bogus-warning", "-c", "src.c"]);
        assert_eq!(
            (clang.status.code(), stderr(&clang).as_str()),
            (
                Some(0),
                "warning: unknown warning option '-Wno-bogus-warning' [-Wunknown-warning-option]\n"
            )
        );
        let known = directory.bcc("--clang-mode", &["-Wno-shadow", "-c", "src.c"]);
        assert_eq!(
            (known.status.code(), stderr(&known).as_str()),
            (Some(0), "")
        );
        let gcc = directory.bcc("--gcc-mode", &["-Wno-bogus-warning", "-c", "src.c"]);
        assert_eq!((gcc.status.code(), stderr(&gcc).as_str()), (Some(0), ""));
    }

    /// libtool compiles with `-fno-rtti -fno-exceptions` and keeps them
    /// (`no_builtin_flag`) only if the compiler says nothing: GCC's C
    /// compiler warns about C++'s `-fno-rtti` for each C source, which `-w`
    /// silences and `-Werror` makes an error; Clang accepts it silently.
    #[test]
    fn libtool_rtti_check_sees_gcc_warn_about_cplusplus_options() {
        let directory = Directory::new();
        directory.write("conftest.c", "int x;\n");
        directory.write("other.c", "int y;\n");
        let warning = "cc1: warning: command-line option '-fno-rtti' is valid for C++/D/ObjC++ \
                       but not for C\n";
        let gcc = directory.bcc(
            "--gcc-mode",
            &[
                "-fno-rtti",
                "-fno-exceptions",
                "-c",
                "conftest.c",
                "other.c",
            ],
        );
        assert_eq!(gcc.status.code(), Some(0));
        assert_eq!(stderr(&gcc), warning.repeat(2));
        let quiet = directory.bcc("--gcc-mode", &["-fno-rtti", "-w", "-c", "conftest.c"]);
        assert_eq!(
            (quiet.status.code(), stderr(&quiet).as_str()),
            (Some(0), "")
        );
        let error = directory.bcc("--gcc-mode", &["-frtti", "-Werror", "-c", "conftest.c"]);
        assert_eq!(
            (error.status.code(), stderr(&error).as_str()),
            (
                Some(1),
                "cc1: error: command-line option '-frtti' is valid for C++/D/ObjC++ but not for C \
                 [-Werror]\ncc1: all warnings being treated as errors\n"
            )
        );
        let clang = directory.bcc(
            "--clang-mode",
            &["-fno-rtti", "-fno-exceptions", "-c", "conftest.c"],
        );
        assert_eq!(
            (clang.status.code(), stderr(&clang).as_str()),
            (Some(0), "")
        );
    }

    /// libtool reads the `libraries:` line of `-print-search-dirs` as the
    /// directories the linker searches by itself, whose libraries need no
    /// run path. It names the directories the bundled clang links from, in
    /// both personalities, not a directory the link does not search.
    #[test]
    fn libtool_search_path_is_the_linkers() {
        if !LINKS {
            return;
        }
        let directory = Directory::new();
        let libraries = |mode: &str| {
            let output = directory.bcc(mode, &["-print-search-dirs"]);
            assert_eq!(output.status.code(), Some(0), "{mode}");
            stdout(&output)
                .lines()
                .find_map(|line| line.strip_prefix("libraries: =").map(str::to_owned))
                .unwrap_or_else(|| panic!("{mode}: {}", stdout(&output)))
        };
        let clang = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("llvm")
            .join("bin")
            .join("clang");
        let bundled = Command::new(clang)
            .args(["--target=x86_64-unknown-linux-gnu", "-print-search-dirs"])
            .output()
            .unwrap();
        let bundled = stdout(&bundled)
            .lines()
            .find_map(|line| line.strip_prefix("libraries: =").map(str::to_owned))
            .unwrap();
        assert_eq!(libraries("--gcc-mode"), bundled);
        assert_eq!(libraries("--clang-mode"), bundled);
    }

    /// `CMake` decides a compiler's default extensions from
    /// `__STRICT_ANSI__`, which neither `cl` nor clang-cl defines in any
    /// mode, while GCC and Clang define it for a strict `-std=`.
    #[test]
    fn cmake_extension_default_reads_strict_ansi_as_each_compiler_defines_it() {
        let directory = Directory::new();
        directory.write(
            "id.c",
            "#ifdef __STRICT_ANSI__\nstrict\n#else\nextended\n#endif\n",
        );
        let preprocessed = |mode: &str, args: &[&str]| {
            let output = directory.bcc(mode, args);
            assert_eq!(output.status.code(), Some(0), "{mode}: {}", stderr(&output));
            stdout(&output)
        };
        for mode in ["--msvc-mode", "--clang-cl-mode"] {
            for standard in [&[][..], &["/std:c11"], &["/std:c17"]] {
                let args = [&["/nologo", "/E"][..], standard, &["id.c"]].concat();
                let text = preprocessed(mode, &args);
                assert!(text.contains("extended"), "{mode} {standard:?}: {text}");
            }
        }
        for mode in ["--gcc-mode", "--clang-mode"] {
            assert!(preprocessed(mode, &["-E", "-std=c11", "id.c"]).contains("strict"));
            assert!(preprocessed(mode, &["-E", "id.c"]).contains("extended"));
        }
    }

    /// Meson recognizes `cl` by `Microsoft` in what `cl /?` prints and
    /// reads its version and target from the first line that has a version
    /// (`19.33.0`, `x64`), which `cl` prints before anything it says about
    /// the command line, unless `/nologo` stands before `/link`.
    #[test]
    fn meson_reads_cl_from_its_banner() {
        let directory = Directory::new();
        let help = directory.bcc("--msvc-mode", &["/?"]);
        assert_eq!(help.status.code(), Some(0));
        let banner = stderr(&help);
        assert!(
            banner.starts_with(&format!(
                "bcc {}, compatible with Microsoft (R) C/C++ Optimizing Compiler Version 19.33.0 \
                 for x64;",
                env!("CARGO_PKG_VERSION")
            )),
            "{banner}"
        );
        assert!(stdout(&help).starts_with("usage: cl "), "{}", stdout(&help));
        let version = directory.bcc("--msvc-mode", &["--version"]);
        assert_eq!(version.status.code(), Some(2));
        let lines: Vec<String> = stderr(&version).lines().map(str::to_owned).collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(format!("{}\n", lines[0]), banner);
        assert_eq!(
            lines[1..],
            [
                "cl : Command line warning D9002 : ignoring unknown option '--version'",
                "cl : Command line error D8003 : missing source filename"
            ]
        );
        let quiet = directory.bcc("--msvc-mode", &["--version", "/nologo"]);
        assert!(!stderr(&quiet).contains("Microsoft"), "{}", stderr(&quiet));
        let clang_cl = directory.bcc("--clang-cl-mode", &["/?"]);
        assert_eq!(
            (clang_cl.status.code(), stderr(&clang_cl).as_str()),
            (Some(0), "")
        );
    }

    /// `CMake`'s `check_c_compiler_flag(/bogus)` with an MSVC compiler fails
    /// on `cl`'s warning D9002. `cl` reads every argument that starts with
    /// `/` as an option; off Windows the `cl` personality reads one as an
    /// option when it names no file and no directory, while clang-cl reads
    /// any argument that names no option as an input, which is then
    /// missing.
    #[test]
    fn cmake_flag_check_sees_cl_warn_about_an_unknown_option() {
        if cfg!(windows) {
            return;
        }
        let directory = Directory::new();
        directory.write("src.c", "int x;\n");
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", "/bogus-flag", "src.c"]);
        assert_eq!(cl.status.code(), Some(0), "{}", stderr(&cl));
        assert_eq!(
            stderr(&cl),
            "cl : Command line warning D9002 : ignoring unknown option '/bogus-flag'\n"
        );
        let clang_cl = directory.bcc("--clang-cl-mode", &["/c", "/bogus-flag", "src.c"]);
        assert_eq!(clang_cl.status.code(), Some(1));
        assert_eq!(
            stderr(&clang_cl),
            "clang-cl: error: no such file or directory: '/bogus-flag'\n"
        );
        // A file inside a directory is a source, as before.
        let absolute = directory.0.join("src.c");
        let absolute = absolute.to_str().unwrap();
        let cl = directory.bcc("--msvc-mode", &["/nologo", "/c", absolute]);
        assert_eq!(cl.status.code(), Some(0), "{}", stderr(&cl));
        assert!(directory.0.join("src.obj").exists());
    }
}
