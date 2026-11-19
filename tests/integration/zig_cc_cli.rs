//! The zig cc personality (`--zig-cc-mode`, or an executable named `zig-cc`,
//! `zigcc`, or `zig` given `cc` first): zig 0.17.1's target queries, `-mcpu`,
//! optimization modes and their defaults, its own driver errors, and Clang
//! 22.1.8's identity. Every expectation was measured with zig 0.17.1;
//! `zig_cc_parity_with_a_real_zig` repeats the comparison when `BCC_ZIG`
//! names a `zig` binary. `docs/design/driver-modes.md` ("zig cc") lists what
//! differs.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary directories, command inputs and captured output."
)]

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        path::{
            Path,
            PathBuf,
        },
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
                "bcc-zig-cc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        /// Runs `program` in this directory with `args`.
        fn run(&self, program: impl AsRef<OsStr>, args: &[&str]) -> Output {
            Command::new(program)
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH")
                .output()
                .unwrap()
        }

        /// Runs `bcc --zig-cc-mode` with `args`.
        fn zig(&self, args: &[&str]) -> Output {
            let mut all = vec!["--zig-cc-mode"];
            all.extend_from_slice(args);
            self.run(env!("CARGO_BIN_EXE_bcc-rust"), &all)
        }

        /// The macros `bcc --zig-cc-mode ARGS -dM -E empty.c` defines.
        fn macros(&self, args: &[&str]) -> String {
            self.write("empty.c", "");
            let mut all = args.to_vec();
            all.extend_from_slice(&["-dM", "-E", "empty.c"]);
            let output = self.zig(&all);
            succeeds(&output);
            stdout(&output)
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

    fn succeeds(output: &Output) {
        assert!(
            output.status.success(),
            "status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            stdout(output),
            stderr(output)
        );
    }

    /// Asserts that the run failed with status 1 and wrote exactly `message`
    /// to stderr and nothing to stdout.
    fn fails_with(output: &Output, message: &str) {
        assert_eq!(output.status.code(), Some(1), "stderr:\n{}", stderr(output));
        assert_eq!(stderr(output), message);
        assert_eq!(stdout(output), "");
    }

    fn defines(macros: &str, definition: &str) -> bool {
        macros.lines().any(|line| line == definition)
    }

    fn names(macros: &str, name: &str) -> bool {
        macros.lines().any(|line| {
            line.strip_prefix("#define ")
                .and_then(|rest| rest.split([' ', '(']).next())
                == Some(name)
        })
    }

    #[test]
    fn version_banner_names_zigs_target_and_bcc() {
        let directory = Directory::new();
        for (args, triple) in [
            (
                &["-target", "x86_64-linux-gnu"][..],
                "x86_64-unknown-linux5.10.0-gnu2.31.0",
            ),
            (
                &["-target", "x86_64-linux-gnu.2.17"],
                "x86_64-unknown-linux5.10.0-gnu2.17.0",
            ),
            (
                &["-target", "x86_64-linux.6.1...6.5-gnu.2.28"],
                "x86_64-unknown-linux6.1.0-gnu2.28.0",
            ),
            (
                &["-target", "x86_64-linux"],
                "x86_64-unknown-linux5.10.0-musl",
            ),
            (&["--target=x86_64-windows"], "x86_64-unknown-windows-gnu"),
            (
                &["-target", "x86_64-windows-msvc"],
                "x86_64-unknown-windows-msvc",
            ),
            (&["-target", "wasm32-wasi"], "wasm32-unknown-wasip1-unknown"),
            (
                &["-target", "wasm32-freestanding"],
                "wasm32-unknown-unknown-unknown",
            ),
        ] {
            let mut all = args.to_vec();
            all.push("--version");
            let output = directory.zig(&all);
            succeeds(&output);
            let text = stdout(&output);
            let mut lines = text.lines();
            assert_eq!(
                lines.next(),
                Some("clang version 22.1.8 (bcc 0.1.0, a zig cc-compatible C compiler; not zig)"),
                "{args:?}"
            );
            assert_eq!(lines.next(), Some(format!("Target: {triple}").as_str()));
            assert_eq!(lines.next(), Some("Thread model: posix"));
            assert!(
                lines
                    .next()
                    .is_some_and(|line| line.starts_with("InstalledDir: "))
            );
        }
        // Without a target, zig names the running kernel and glibc.
        let output = directory.zig(&["--version"]);
        succeeds(&output);
        assert!(
            stdout(&output)
                .lines()
                .nth(1)
                .is_some_and(|line| line.starts_with("Target: x86_64-unknown-linux")),
            "{}",
            stdout(&output)
        );
        let output = directory.zig(&["-dumpversion"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "22.1.8\n");
    }

    /// zig hands `-dumpmachine` to Clang's own driver, which normalizes the
    /// query as a triple and has no target of its own but musl's.
    #[test]
    fn dumpmachine_is_clangs_reading_of_the_query() {
        let directory = Directory::new();
        for (args, machine) in [
            (&[][..], "x86_64-unknown-linux-musl"),
            (
                &["-target", "x86_64-windows"],
                "x86_64-unknown-windows-msvc",
            ),
            (
                &["-target", "x86_64-linux-musl"],
                "x86_64-unknown-linux-musl",
            ),
            (&["-target", "wasm32-freestanding"], "wasm32-freestanding"),
        ] {
            let mut all = args.to_vec();
            all.push("-dumpmachine");
            let output = directory.zig(&all);
            succeeds(&output);
            assert_eq!(stdout(&output), format!("{machine}\n"), "{args:?}");
        }
        let output = directory.zig(&["-target", "x86_64-linux-gnu.2.17", "-dumpmachine"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "x86_64-unknown-linux-gnu.2.17\n");
        assert_eq!(
            stderr(&output),
            "zig: error: version '.2.17' in target triple 'x86_64-unknown-linux-gnu.2.17' is \
             invalid\n"
        );
    }

    #[test]
    fn target_queries_fail_in_zigs_words() {
        let directory = Directory::new();
        directory.write("e.c", "int main(void) { return 0; }\n");
        for (query, error) in [
            ("x86_64-unknown-linux-gnu", "UnknownOperatingSystem"),
            ("x86_64", "MissingOperatingSystem"),
            ("x86_64-linux-bogus", "UnknownApplicationBinaryInterface"),
            ("x86_64-linux-gnu.abc", "InvalidAbiVersion"),
            ("x86_64-linux.6-gnu", "InvalidOperatingSystemVersion"),
            ("x86_64-linux-gnu-extra", "UnexpectedExtraField"),
            ("wasm32-wasip1", "UnknownOperatingSystem"),
        ] {
            fails_with(
                &directory.zig(&["-target", query, "-c", "e.c"]),
                &format!("error: unable to parse target query '{query}': {error}\n"),
            );
        }
        let output = directory.zig(&["-target", "bogus-linux-gnu", "-c", "e.c"]);
        assert_eq!(output.status.code(), Some(1));
        let text = stderr(&output);
        assert!(
            text.starts_with("info: available architectures:\n aarch64\n"),
            "{text}"
        );
        assert!(
            text.ends_with(
                " x86_64\n xcore\n xtensa\n xtensaeb\n native\n\nerror: unknown architecture: \
                 'bogus'\n"
            ),
            "{text}"
        );
        fails_with(
            &directory.zig(&["-target"]),
            "error: expected parameter after \"-target\"\n",
        );
        fails_with(
            &directory.zig(&["--target", "x86_64-linux-musl", "-c", "e.c"]),
            "error: unknown clang option: \"--target\"\n",
        );
        // A target zig has and bcc lacks.
        fails_with(
            &directory.zig(&["-target", "aarch64-linux-gnu", "-c", "e.c"]),
            "error: unable to compile for target 'aarch64-linux-gnu': bcc compiles for \
             x86_64-linux-gnu, x86_64-linux-musl, x86_64-windows-gnu, x86_64-windows-msvc, \
             wasm32-wasi and wasm32-freestanding\n",
        );
    }

    /// `-mcpu=`, `-march=` and `-mtune=` all name one of zig's processors,
    /// with zig's spelling of LLVM's names.
    #[test]
    fn processors_take_zigs_names() {
        let directory = Directory::new();
        directory.write("e.c", "int main(void) { return 0; }\n");
        let macros = directory.macros(&["-target", "x86_64-linux-gnu", "-mcpu=x86_64_v3"]);
        assert!(defines(&macros, "#define __AVX2__ 1"), "{macros}");
        let macros = directory.macros(&["-target", "x86_64-linux-gnu", "-mcpu=x86_64+avx2"]);
        assert!(defines(&macros, "#define __AVX2__ 1"), "{macros}");
        let macros = directory.macros(&["-target", "x86_64-linux-gnu"]);
        assert!(!names(&macros, "__AVX__"), "{macros}");
        for option in ["-march=x86-64-v3", "-mtune=x86-64", "-mcpu=x86-64"] {
            let output = directory.zig(&[option, "-c", "e.c"]);
            assert_eq!(output.status.code(), Some(1));
            let text = stderr(&output);
            assert!(
                text.starts_with("info: available CPUs for architecture 'x86_64':\n alderlake\n"),
                "{text}"
            );
            assert!(
                text.ends_with(" znver5\n\nerror: unknown CPU: 'x86'\n"),
                "{text}"
            );
        }
        let output = directory.zig(&["-mcpu=x86_64+bogus", "-c", "e.c"]);
        assert_eq!(output.status.code(), Some(1));
        let text = stderr(&output);
        assert!(
            text.starts_with(
                "info: available CPU features for architecture 'x86_64':\n 16bit_mode: 16-bit \
                 mode (i8086)\n"
            ),
            "{text}"
        );
        assert!(
            text.ends_with("\n\nerror: unknown CPU feature: 'bogus'\n"),
            "{text}"
        );
    }

    /// zig's optimization modes and what each gives Clang, with Clang
    /// 22.1.8's identity in every mode.
    #[test]
    fn modes_and_targets_define_zigs_macros() {
        let directory = Directory::new();
        let gnu = ["-target", "x86_64-linux-gnu"];
        let debug = directory.macros(&gnu);
        for definition in [
            "#define __clang_major__ 22",
            "#define __clang_minor__ 1",
            "#define __clang_patchlevel__ 8",
            "#define __clang_version__ \"22.1.8 \"",
            "#define __VERSION__ \"Clang 22.1.8\"",
            "#define __GLIBC_MINOR__ 31",
            "#define __SSP_STRONG__ 2",
            "#define __PIC__ 2",
            "#define __NO_INLINE__ 1",
            "#define __bcc__ 1",
        ] {
            assert!(defines(&debug, definition), "{definition}\n{debug}");
        }
        // Clang 23 added these, and zig's stays a release behind.
        for name in [
            "NDEBUG",
            "__PIE__",
            "__pie__",
            "__OPTIMIZE__",
            "__SIG_ATOMIC_MIN__",
            "__SIG_ATOMIC_TYPE__",
            "__WCHAR_MIN__",
            "__WINT_MIN__",
        ] {
            assert!(!names(&debug, name), "{name}\n{debug}");
        }
        for level in ["-O1", "-O2", "-O3", "-O4", "-Ofast"] {
            let macros = directory.macros(&[gnu[0], gnu[1], level]);
            assert!(defines(&macros, "#define NDEBUG 1"), "{level}");
            assert!(defines(&macros, "#define __OPTIMIZE__ 1"), "{level}");
            assert!(!names(&macros, "__OPTIMIZE_SIZE__"), "{level}");
            assert!(!names(&macros, "__FAST_MATH__"), "{level}");
            assert!(!names(&macros, "__SSP_STRONG__"), "{level}");
        }
        for level in ["-Os", "-Oz"] {
            let macros = directory.macros(&[gnu[0], gnu[1], level]);
            assert!(defines(&macros, "#define NDEBUG 1"), "{level}");
            assert!(defines(&macros, "#define __OPTIMIZE_SIZE__ 1"), "{level}");
        }
        // The last `-O` chooses the mode, and the command line's own
        // options follow zig's.
        let macros = directory.macros(&[gnu[0], gnu[1], "-O3", "-Og"]);
        assert!(!names(&macros, "NDEBUG"));
        let macros = directory.macros(&[gnu[0], gnu[1], "-O2", "-UNDEBUG"]);
        assert!(!names(&macros, "NDEBUG"));
        let macros = directory.macros(&[gnu[0], gnu[1], "-O2", "-fstack-protector-all"]);
        assert!(defines(&macros, "#define __SSP_STRONG__ 2"));
        let macros = directory.macros(&[gnu[0], gnu[1], "-fno-stack-protector"]);
        assert!(!names(&macros, "__SSP_STRONG__"));
        // Clang reads zig's `-fPIE -fPIC` as PIC alone.
        let macros = directory.macros(&[gnu[0], gnu[1], "-fPIE"]);
        assert!(defines(&macros, "#define __PIC__ 2") && !names(&macros, "__PIE__"));
        let macros = directory.macros(&["-target", "x86_64-linux-gnu.2.17"]);
        assert!(defines(&macros, "#define __GLIBC_MINOR__ 17"));
        // Without the C library zig neither links glibc nor protects the
        // stack, and makes no position-independent code.
        let macros = directory.macros(&[gnu[0], gnu[1], "-nostdlib"]);
        for name in ["__GLIBC_MINOR__", "__SSP_STRONG__", "__PIC__"] {
            assert!(!names(&macros, name), "{name}");
        }
        let musl = directory.macros(&["-target", "x86_64-linux-musl"]);
        assert!(!names(&musl, "__PIC__") && defines(&musl, "#define __SSP_STRONG__ 2"));
        let musl = directory.macros(&["-target", "x86_64-linux-musl", "-fPIC"]);
        assert!(defines(&musl, "#define __PIC__ 2"));
        let mingw = directory.macros(&["-target", "x86_64-windows-gnu"]);
        for definition in [
            "#define _WIN32_WINNT 0x0a00",
            "#define __MSVCRT_VERSION__ 0xE00",
            "#define __CRT__NO_INLINE 1",
        ] {
            assert!(defines(&mingw, definition), "{definition}");
        }
        let win7 = directory.macros(&["-target", "x86_64-windows.win7-gnu"]);
        assert!(defines(&win7, "#define _WIN32_WINNT 0x0601"));
        let freestanding = directory.macros(&["-target", "wasm32-freestanding"]);
        assert!(defines(&freestanding, "#define __STDC_HOSTED__ 0"));
    }

    /// `UBSan`'s checks make `-O1` to `-Ofast` ReleaseSafe, as zig 0.17.1
    /// does after its `cc` loop: no `NDEBUG`, `_FORTIFY_SOURCE=2` and the
    /// stack protector. ReleaseSmall stays itself.
    #[test]
    fn checks_make_release_fast_safe() {
        let directory = Directory::new();
        for level in ["-O1", "-O2", "-O3", "-O4", "-Ofast"] {
            for checks in [
                &["-fsanitize=undefined"][..],
                &["-fsanitize-trap=undefined"],
                &["-fno-sanitize=undefined", "-fsanitize=undefined"],
            ] {
                let mut args = vec!["-target", "x86_64-linux-gnu", level];
                args.extend_from_slice(checks);
                let macros = directory.macros(&args);
                for definition in [
                    "#define _FORTIFY_SOURCE 2",
                    "#define __SSP_STRONG__ 2",
                    "#define __OPTIMIZE__ 1",
                ] {
                    assert!(defines(&macros, definition), "{args:?}: {definition}");
                }
                assert!(!names(&macros, "NDEBUG"), "{args:?}");
                assert!(!names(&macros, "__OPTIMIZE_SIZE__"), "{args:?}");
            }
            for checks in [
                &["-fsanitize=undefined", "-fno-sanitize=undefined"][..],
                &["-fno-sanitize-trap=undefined"],
            ] {
                let mut args = vec!["-target", "x86_64-linux-gnu", level];
                args.extend_from_slice(checks);
                let macros = directory.macros(&args);
                assert!(defines(&macros, "#define NDEBUG 1"), "{args:?}");
                assert!(!names(&macros, "_FORTIFY_SOURCE"), "{args:?}");
                assert!(!names(&macros, "__SSP_STRONG__"), "{args:?}");
            }
        }
        // The order does not matter: zig decides once it has read them all.
        let macros =
            directory.macros(&["-target", "x86_64-linux-gnu", "-fsanitize=undefined", "-O2"]);
        assert!(defines(&macros, "#define _FORTIFY_SOURCE 2"));
        for level in ["-Os", "-Oz"] {
            for checks in ["-fsanitize=undefined", "-fsanitize-trap=undefined"] {
                let macros = directory.macros(&["-target", "x86_64-linux-gnu", level, checks]);
                assert!(defines(&macros, "#define NDEBUG 1"), "{level} {checks}");
                assert!(!names(&macros, "_FORTIFY_SOURCE"), "{level} {checks}");
                assert!(!names(&macros, "__SSP_STRONG__"), "{level} {checks}");
            }
        }
        // Without the C library zig protects no stack.
        let macros = directory.macros(&[
            "-target",
            "x86_64-linux-gnu",
            "-O2",
            "-fsanitize-trap=undefined",
            "-nostdlib",
        ]);
        assert!(defines(&macros, "#define _FORTIFY_SOURCE 2") && !names(&macros, "__SSP_STRONG__"));
    }

    /// zig's ReleaseSmall strips the debug information and the symbols
    /// unless a `-g` option keeps them; the other modes write DWARF 4.
    #[cfg(target_os = "linux")]
    #[test]
    fn release_small_strips_unless_asked_for_debug_information() {
        let directory = Directory::new();
        directory.write(
            "m.c",
            "int triple(int a) { return a * 3; }
int main(void) { return triple(0); }
",
        );
        let has = |name: &str, section: &[u8]| {
            let bytes = std::fs::read(directory.0.join(name)).unwrap();
            bytes.windows(section.len()).any(|window| window == section)
        };
        for (args, debug_info, symbols) in [
            (&["-Os"][..], false, false),
            (&["-Oz"], false, false),
            (&["-Os", "-g"], true, true),
            (&["-g", "-Oz"], true, true),
            (&["-Os", "-gdwarf32"], true, true),
            (&["-O2"], true, true),
            (&["-O2", "-fsanitize-trap=undefined"], true, true),
            (&["-O0"], true, true),
            (&["-O2", "-s"], false, false),
        ] {
            let mut all = vec!["-target", "x86_64-linux-gnu"];
            all.extend_from_slice(args);
            let mut object = all.clone();
            object.extend_from_slice(&["-c", "m.c", "-o", "m.o"]);
            succeeds(&directory.zig(&object));
            assert_eq!(has("m.o", b".debug_info"), debug_info, "{args:?}");
            all.extend_from_slice(&["m.c", "-o", "m"]);
            succeeds(&directory.zig(&all));
            assert_eq!(has("m", b".debug_info"), debug_info, "{args:?}");
            assert_eq!(has("m", b".symtab"), symbols, "{args:?}");
        }
    }

    /// `-fPIE` makes position-independent code where zig would not
    /// otherwise (musl), and zig refuses it with `-fno-PIC` or `-shared`.
    #[test]
    fn position_independent_executables_need_position_independent_code() {
        let directory = Directory::new();
        let musl = ["-target", "x86_64-linux-musl"];
        for pie in ["-fPIE", "-fpie"] {
            let macros = directory.macros(&[musl[0], musl[1], pie]);
            assert!(defines(&macros, "#define __PIC__ 2"), "{pie}");
            assert!(!names(&macros, "__PIE__"), "{pie}");
        }
        let macros = directory.macros(&[musl[0], musl[1], "-fPIE", "-fno-PIE"]);
        assert!(!names(&macros, "__PIC__"));
        directory.write(
            "e.c",
            "static int x;
int *f(void) { return &x; }
",
        );
        fails_with(
            &directory.zig(&[musl[0], musl[1], "-fPIE", "-fno-PIC", "-c", "e.c"]),
            "error: unable to create module \"e\": making a Position Independent Executable \
             requires enabling Position Independent Code\n",
        );
        // glibc's own requirement is reported first.
        fails_with(
            &directory.zig(&[
                "-target",
                "x86_64-linux-gnu",
                "-fPIE",
                "-fno-PIC",
                "-c",
                "e.c",
            ]),
            "error: unable to create module \"e\": the selected target requires position \
             independent code\n",
        );
        fails_with(
            &directory.zig(&[musl[0], musl[1], "-shared", "-fPIE", "e.c", "-o", "e.so"]),
            "error: dynamic libraries cannot be position independent executables\n",
        );
        succeeds(&directory.zig(&[musl[0], musl[1], "-shared", "-fPIE", "-c", "e.c"]));
    }

    /// bcc's `<stdint.h>` spells `WCHAR_MIN` and `WINT_MIN` without the
    /// macros Clang 22 lacks.
    #[test]
    fn freestanding_limits_need_no_clang_23_macros() {
        let directory = Directory::new();
        directory.write(
            "limits.c",
            "#include <stdint.h>\n_Static_assert(WCHAR_MIN == -2147483647 - 1, \
             \"wchar\");\n_Static_assert(WINT_MIN == 0u, \
             \"wint\");\n_Static_assert(SIG_ATOMIC_MIN == -2147483647 - 1, \"sig\");\n",
        );
        succeeds(&directory.zig(&[
            "-target",
            "x86_64-linux-gnu",
            "-ffreestanding",
            "-fsyntax-only",
            "limits.c",
        ]));
    }

    #[test]
    fn driver_errors_are_zigs() {
        let directory = Directory::new();
        directory.write("e.c", "int main(void) { return 0; }\n");
        // zig stops at the first option Clang does not know.
        fails_with(
            &directory.zig(&["-fbogus-option", "-fother-bogus", "-c", "e.c"]),
            "error: unknown clang option: \"-fbogus-option\"\n",
        );
        fails_with(
            &directory.zig(&["-x", "bogus", "e.c"]),
            "error: language not recognized: \"bogus\"\n",
        );
        fails_with(
            &directory.zig(&["-flto=bogus", "-c", "e.c"]),
            "error: invalid -flto mode \"bogus\"; must be \"auto\", \"full\", \"thin\", or \
             \"jobserver\"\n",
        );
        fails_with(
            &directory.zig(&["-fno-PIC", "-c", "e.c", "-target", "x86_64-linux-gnu"]),
            "error: unable to create module \"e\": the selected target requires position \
             independent code\n",
        );
        fails_with(
            &directory.zig(&[
                "-target",
                "x86_64-linux-musl",
                "-fno-PIC",
                "-shared",
                "-c",
                "e.c",
            ]),
            "error: unable to create module \"e\": dynamic linking requires enabling Position \
             Independent Code\n",
        );
        fails_with(
            &directory.zig(&["-target", "x86_64-linux-gnu.2.17", "-static", "e.c"]),
            "error: libc of the specified target requires dynamic linking\n",
        );
        fails_with(&directory.zig(&[]), "zig: error: no input files\n");
        // zig's linker answers its version as zig reads it, in words Meson
        // takes for zig's linker.
        let output = directory.zig(&["-Wl,--version"]);
        succeeds(&output);
        assert_eq!(stdout(&output), "zig ld 0.17.1 (bcc 0.1.0; not zig)\n");
        assert_eq!(stderr(&output), "");
        let full = |name: &str| directory.0.join(name).display().to_string();
        fails_with(
            &directory.zig(&["missing.c", "other.c"]),
            &format!(
                "missing.c:1:1: error: checking cache failed: file_open FileNotFound \
                 {}\nother.c:1:1: error: checking cache failed: file_open FileNotFound {}\n",
                full("missing.c"),
                full("other.c")
            ),
        );
        fails_with(
            &directory.zig(&["e.c", "missing.o"]),
            "error: failed to open object missing.o: FileNotFound\n",
        );
        fails_with(
            &directory.zig(&["@missing.rsp", "e.c"]),
            "error: unable to read response file \"missing.rsp\": FileNotFound\n",
        );
        directory.write("outer.rsp", "-c @nested.rsp\n");
        fails_with(
            &directory.zig(&["@outer.rsp", "e.c"]),
            "error: unable to read response file \"nested.rsp\": FileNotFound\n",
        );
        assert!(!directory.0.join("a.out").exists());
    }

    /// The executable names: `zig cc`, as build systems are given it, and
    /// `zig-cc`.
    #[cfg(unix)]
    #[test]
    fn zig_and_zig_cc_executables_select_the_personality() {
        let directory = Directory::new();
        let zig = directory.0.join("zig");
        let zig_cc = directory.0.join("zig-cc");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &zig).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_bcc-rust"), &zig_cc).unwrap();
        for (program, args) in [
            (&zig, &["cc", "-dumpversion"][..]),
            (&zig_cc, &["-dumpversion"]),
        ] {
            let output = directory.run(program, args);
            succeeds(&output);
            assert_eq!(stdout(&output), "22.1.8\n");
        }
        fails_with(
            &directory.run(&zig, &["build"]),
            "error: 'zig build' is not supported: bcc implements only 'zig cc'\n",
        );
        fails_with(
            &directory.run(&zig, &["version"]),
            "error: 'zig version' is not supported: bcc implements only 'zig cc'\n",
        );
        fails_with(
            &directory.run(&zig, &["frobnicate"]),
            "error: unknown command: frobnicate\n",
        );
        fails_with(
            &directory.run(&zig, &[]),
            "error: expected command argument\n",
        );
    }

    /// The type of the ELF file at `path`: 2 for an executable at a fixed
    /// address, 3 for a position-independent one.
    #[cfg(target_os = "linux")]
    fn elf_type(path: &Path) -> u16 {
        let bytes = std::fs::read(path).unwrap();
        u16::from_le_bytes([bytes[16], bytes[17]])
    }

    /// zig's Debug mode stops at the first undefined behavior `UBSan` finds,
    /// and its release modes do not check; executables are not
    /// position-independent, and carry DWARF unless stripped.
    #[cfg(target_os = "linux")]
    #[test]
    fn debug_mode_checks_undefined_behavior_and_links_fixed_executables() {
        let directory = Directory::new();
        directory.write(
            "ub.c",
            "#include <limits.h>\n#include <stdio.h>\nint main(int argc, char **argv) {\n  int x \
             = INT_MAX;\n  x += argc;\n  printf(\"%d\\n\", x);\n  return 0;\n}\n",
        );
        succeeds(&directory.zig(&["-target", "x86_64-linux-gnu", "ub.c", "-o", "debug"]));
        let run = directory.run(directory.0.join("debug"), &[]);
        assert!(!run.status.success());
        assert_eq!(stdout(&run), "");
        assert!(
            stderr(&run).contains(
                "ub.c:5:5: runtime error: signed integer overflow: 2147483647 + 1 cannot be \
                 represented in type 'int'"
            ),
            "{}",
            stderr(&run)
        );
        assert_eq!(elf_type(&directory.0.join("debug")), 2);
        succeeds(&directory.zig(&["-O2", "ub.c", "-o", "release"]));
        let run = directory.run(directory.0.join("release"), &[]);
        succeeds(&run);
        assert_eq!(stdout(&run), "-2147483648\n");
        succeeds(&directory.zig(&["-fsanitize-trap=undefined", "ub.c", "-o", "trap"]));
        let run = directory.run(directory.0.join("trap"), &[]);
        assert!(!run.status.success() && stdout(&run).is_empty());
        let debug_info = |name: &str| {
            let bytes = std::fs::read(directory.0.join(name)).unwrap();
            bytes
                .windows(b".debug_info".len())
                .any(|window| window == b".debug_info")
        };
        assert!(debug_info("release"));
        succeeds(&directory.zig(&["-s", "ub.c", "-o", "stripped"]));
        assert!(!debug_info("stripped"));
        succeeds(&directory.zig(&["-fPIE", "ub.c", "-o", "pie"]));
        assert_eq!(elf_type(&directory.0.join("pie")), 3);
    }

    /// Compares bcc with the `zig` that `BCC_ZIG` names, byte for byte after
    /// each compiler's install directory and bcc's banner note are replaced:
    /// stdout, stderr and the exit status, and what the programs built print.
    /// The test passes without one.
    #[test]
    fn zig_cc_parity_with_a_real_zig() {
        let Some(zig) = std::env::var_os("BCC_ZIG") else {
            eprintln!("skipping: BCC_ZIG names no zig binary");
            return;
        };
        let cases: &[&[&str]] = &[
            &["--version"],
            &["-target", "x86_64-linux-gnu.2.17", "--version"],
            &["-target", "x86_64-linux-musl", "--version"],
            &["-target", "x86_64-windows-msvc", "--version"],
            &["-target", "wasm32-wasi", "--version"],
            &["-dumpversion"],
            &["-dumpmachine"],
            &["-target", "x86_64-windows", "-dumpmachine"],
            &["-target", "x86_64-linux-gnu.2.17", "-dumpmachine"],
            &["-target", "x86_64-unknown-linux-gnu", "-c", "e.c"],
            &["-target", "bogus-linux-gnu", "-c", "e.c"],
            &["-target", "x86_64", "-c", "e.c"],
            &["-target", "x86_64-linux-gnu.abc", "-c", "e.c"],
            &["-target"],
            &["-mcpu=bogus", "-c", "e.c"],
            &["-march=x86-64-v3", "-c", "e.c"],
            &["-mcpu=x86_64+bogus", "-c", "e.c"],
            &["-fbogus-option", "-c", "e.c"],
            &["-x", "bogus", "e.c"],
            &["-flto=bogus", "-c", "e.c"],
            &["-c", "missing.c"],
            &["e.c", "missing.o"],
            &[],
            &["-fno-PIC", "-c", "e.c"],
            &[
                "-target",
                "x86_64-linux-gnu.2.17",
                "-static",
                "e.c",
                "-o",
                "out",
            ],
            &["-std=bogus", "-c", "e.c"],
            &["-target", "x86_64-linux-gnu", "-c", "undeclared.c"],
            &["hello.c", "-o", "out"],
            &["-O2", "hello.c", "-o", "out"],
            &["-Os", "hello.c", "-o", "out"],
            &["-O2", "-fsanitize=undefined", "assert.c", "-o", "out"],
            &["-O3", "-fsanitize-trap=undefined", "assert.c", "-o", "out"],
            &["-Os", "-fsanitize=undefined", "assert.c", "-o", "out"],
            &[
                "-target",
                "x86_64-linux-musl",
                "-fPIE",
                "-fno-PIC",
                "-c",
                "e.c",
            ],
            &[
                "-target",
                "x86_64-linux-musl",
                "-shared",
                "-fPIE",
                "e.c",
                "-o",
                "e.so",
            ],
            &["@missing.rsp", "e.c"],
        ];
        let fixtures = [
            ("e.c", "int main(void) { return 0; }\n"),
            ("undeclared.c", "int f(void) { return y; }\n"),
            (
                "hello.c",
                "#include <stdio.h>\nint main(void) { printf(\"hi\\n\"); return 0; }\n",
            ),
            // Prints only when `NDEBUG` takes the assertion away.
            (
                "assert.c",
                "#include <assert.h>\n#include <stdio.h>\nint main(int argc, char **argv) {\n  \
                 (void)argv;\n  assert(argc == 42);\n  puts(\"asserts off\");\n  return 0;\n}\n",
            ),
        ];
        let normalize = |text: &[u8], program: &Path, directory: &Path| {
            let installed = std::fs::canonicalize(program)
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let text = String::from_utf8_lossy(text)
                .replace(&directory.display().to_string(), "<cwd>")
                .replace(&installed.display().to_string(), "<installed>");
            match text.split_once(" (bcc ") {
                | Some((before, after)) if before.starts_with("clang version") =>
                    format!("{before}{}", &after[after.find(")\n").unwrap_or(0) + 1..]),
                | _ => text,
            }
        };
        let mut failures = Vec::new();
        for args in cases {
            let mut outcomes = Vec::new();
            for real in [true, false] {
                let directory = Directory::new();
                for (name, text) in fixtures {
                    directory.write(name, text);
                }
                let (program, mut all): (PathBuf, Vec<&str>) = if real {
                    (PathBuf::from(&zig), vec!["cc"])
                } else {
                    (
                        PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust")),
                        vec!["--zig-cc-mode"],
                    )
                };
                all.extend_from_slice(args);
                let output = directory.run(&program, &all);
                let mut ran = String::new();
                if output.status.success() && directory.0.join("out").exists() {
                    let run = directory.run(directory.0.join("out"), &[]);
                    ran = format!("{:?} {}", run.status.code(), stdout(&run));
                }
                outcomes.push((
                    output.status.code(),
                    normalize(&output.stdout, &program, &directory.0),
                    normalize(&output.stderr, &program, &directory.0),
                    ran,
                ));
            }
            if outcomes[0] != outcomes[1] {
                failures.push(format!(
                    "zig cc {}\nzig: {:?}\nbcc: {:?}",
                    args.join(" "),
                    outcomes[0],
                    outcomes[1]
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
