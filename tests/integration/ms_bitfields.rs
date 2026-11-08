//! `-mms-bitfields` and `-mno-ms-bitfields` in each personality, and TCC's
//! `#pragma comment(option, "...")`, against the record layouts GCC 14,
//! Clang 23 and TCC give. `tests/fixtures/programs/ms_bitfields.c` compiles
//! exactly when its sizes, alignments and offsets match theirs, and
//! `ms_bitfields_bits.c` prints the bits each field occupies.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
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

    use crate::native::run_native;

    const LINUX: &str = "--target=x86_64-linux-gnu";
    const MINGW: &str = "--target=x86_64-w64-mingw32";

    /// What GCC 14 prints for `ms_bitfields_bits.c` under `-mms-bitfields`.
    const GCC_BITS: &str = concat!(
        "struct A size 12 align 4 bits 000000ff000000ff0000000f\n",
        "struct B size 24 align 8 bits 000000000000001f000000ffffffffff0000000000000007\n",
        "struct C size 2 align 1 bits ffff\n",
        "struct D size 8 align 4 bits 0000000700000003\n",
        "struct E size 6 align 2 bits 01ff01ff01ff\n",
        "union U size 4 align 4 bits 000fffff\n",
        "struct F size 8 align 4 bits 000000037fffffff\n",
        "struct G size 24 align 8 bits 0000000000000001000000000000007f0000000000000001\n",
        "struct P size 6 align 1 bits ff000000ff0f\n",
        "struct Q size 8 align 2 bits 0003000fffff000f\n",
        "struct H size 4 align 4 bits 00ff0fff\n",
        "struct I size 12 align 4 bits 000000ff000000ff0000000f\n",
        "struct J size 16 align 8 bits 000000000000001f000000ff00070007\n",
        "offsets 8 1 8 4\n",
    );

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/programs")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn the_option_sets_the_layout_in_each_personality() {
        let records = fixture("ms_bitfields.c");
        for personality in ["--gcc-mode", "--clang-mode"] {
            for (options, microsoft) in [
                (&[LINUX][..], false),
                (&[LINUX, "-mms-bitfields"], true),
                (&[LINUX, "-mms-bitfields", "-mno-ms-bitfields"], false),
                (&[LINUX, "-mno-ms-bitfields", "-mms-bitfields"], true),
                (&[MINGW], true),
                (&[MINGW, "-mno-ms-bitfields"], false),
                (&[MINGW, "-mno-ms-bitfields", "-mms-bitfields"], true),
            ] {
                let mut args = vec![personality, "-fsyntax-only"];
                args.extend(options);
                if microsoft {
                    args.push("-DMS");
                }
                args.push(&records);
                succeeds(&bcc(&args), &args);
            }
        }
        // TCC's target is the host's, and TCC lays out a Windows record as
        // a Linux one, which bcc does not.
        if !cfg!(target_os = "linux") {
            eprintln!("skipping TCC: its target is the host's");
            return;
        }
        let object = temporary("records.o");
        let object = object.to_str().unwrap();
        for options in [&[][..], &["-mms-bitfields", "-DMS"]] {
            let mut args = vec!["--tcc-mode", "-c", &records, "-o", object];
            args.extend(options);
            succeeds(&bcc(&args), &args);
        }
        drop(std::fs::remove_file(object));
    }

    /// Clang lays out no record of an MSVC target in System V's layout, so
    /// `-mno-ms-bitfields` there makes every record an error, in clang and
    /// clang-cl alike; `-mms-bitfields` asks for the target's own layout.
    #[test]
    fn clang_refuses_the_system_v_layout_on_msvc() {
        let source = temporary("msvc.c");
        std::fs::write(&source, "struct s { char a : 4; int b : 8; } v;\n").unwrap();
        let path = source.to_str().unwrap();
        for args in [
            &[
                "--clang-mode",
                "--target=x86_64-pc-windows-msvc",
                "-fsyntax-only",
                "-mno-ms-bitfields",
                path,
            ][..],
            &[
                "--clang-cl-mode",
                "-fsyntax-only",
                "-mno-ms-bitfields",
                path,
            ],
        ] {
            let output = bcc(args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(
                text(&output.stderr).contains(
                    "error: Itanium-compatible layout for the Microsoft C++ ABI is not yet \
                     supported"
                ),
                "{args:?}: {}",
                text(&output.stderr)
            );
        }
        for args in [
            &[
                "--clang-mode",
                "--target=x86_64-pc-windows-msvc",
                "-fsyntax-only",
                "-mms-bitfields",
                path,
            ][..],
            &["--clang-cl-mode", "-fsyntax-only", "-mms-bitfields", path],
        ] {
            succeeds(&bcc(args), args);
        }
        std::fs::remove_file(source).unwrap();
    }

    /// The bits each field occupies, as GCC 14, Clang 23 and TCC print them;
    /// Clang ignores a union bit-field's alignment, and TCC's own Microsoft
    /// layout differs at a zero-width field and ignores `packed` (glibc's
    /// headers hide every attribute from a compiler that is not GCC).
    #[test]
    fn bits_match_each_compiler() {
        if !cfg!(all(target_arch = "x86_64", target_os = "linux")) {
            eprintln!("skipping: the fixture includes the host's C headers");
            return;
        }
        let program = fixture("ms_bitfields_bits.c");
        let clang = GCC_BITS.replace("union U size 4 align 4", "union U size 4 align 1");
        let tcc = GCC_BITS
            .replace(
                "struct D size 8 align 4 bits 0000000700000003",
                "struct D size 2 align 1 bits 0703",
            )
            .replace(
                "struct P size 6 align 1 bits ff000000ff0f",
                "struct P size 12 align 4 bits 000000ff000000ff0000000f",
            )
            .replace(
                "struct H size 4 align 4 bits 00ff0fff",
                "struct H size 12 align 4 bits 000000ff000000ff0000000f",
            );
        for (identity, expected) in [("gcc", GCC_BITS), ("clang", &clang), ("tcc", &tcc)] {
            let identity = format!("--compiler-identity={identity}");
            let args = [
                &identity,
                "--target=x86_64-unknown-linux-gnu",
                "-w",
                "-mms-bitfields",
                "--interpret",
                &program,
            ];
            let output = bcc(&args);
            succeeds(&output, &args);
            assert_eq!(text(&output.stdout), expected, "{identity}");
        }
    }

    /// TCC applies the option from the pragma on, to each record whose
    /// definition ends after it, even one whose body it is in.
    #[test]
    fn the_tcc_pragma_switches_the_layout_from_there_on() {
        let source = concat!(
            "#define LAYOUT(T, size) _Static_assert(sizeof(T) == (size), #T)\n",
            "struct before { char a : 4; int b : 8; char c; };\n",
            "#pragma comment(option, \"-mms-bitfields\")\n",
            "struct after { char a : 4; int b : 8; char c; };\n",
            "struct spans { char a : 4;\n",
            "#pragma comment(option, \"-mno-ms-bitfields\")\n",
            "  int b : 8; char c; };\n",
            "#define OPTION \"-mms-bitfields -fsigned-char\"\n",
            "_Pragma(\"comment(option, OPTION)\")\n",
            "struct late { char a : 4; int b : 8; char c; };\n",
            "LAYOUT(struct before, 4);\n",
            "LAYOUT(struct after, 12);\n",
            "LAYOUT(struct spans, 4);\n",
            "LAYOUT(struct late, 12);\n",
            "#pragma comment(lib, \"m\")\n",
            "#pragma comment(user, \"anything\")\n",
        );
        let output = tcc(source, &[]);
        succeeds(&output, &["--compiler-identity=tcc"]);
        assert_eq!(text(&output.stderr), "");
        // GCC knows no `comment` pragma, and the layout stays System V's.
        let gcc = run_source(
            &source.replace("12)", "4)"),
            &[
                "--compiler-identity=gcc",
                "--target=x86_64-unknown-linux-gnu",
                "-Wall",
            ],
        );
        succeeds(&gcc, &["--compiler-identity=gcc"]);
        assert!(
            text(&gcc.stderr).contains("ignoring '#pragma comment"),
            "{}",
            text(&gcc.stderr)
        );
    }

    /// Options bcc cannot follow from a pragma on are refused, a library it
    /// would not link is reported, and TCC's malformed pragma is an error;
    /// `-E` writes the pragma back without carrying it out.
    #[test]
    fn the_tcc_pragma_refuses_what_bcc_cannot_follow() {
        for (pragma, message) in [
            (
                "#pragma comment(option, \"-funsigned-char\")",
                "error: '-funsigned-char' is not supported in `#pragma comment(option)`: plain \
                 char keeps the signedness",
            ),
            (
                "#pragma comment(option, \"-mms-bitfields -fms-extensions\")",
                "error: '-fms-extensions' is not supported in `#pragma comment(option)`: bcc \
                 applies only -mms-bitfields",
            ),
            (
                "#pragma comment(option)",
                "error: malformed #pragma directive",
            ),
            (
                "#pragma comment(option, L\"-mms-bitfields\")",
                "error: malformed #pragma directive",
            ),
        ] {
            let output = tcc(&format!("{pragma}\nint x;\n"), &[]);
            assert_eq!(output.status.code(), Some(1), "{pragma}");
            assert!(
                text(&output.stderr).contains(message),
                "{pragma}: {}",
                text(&output.stderr)
            );
        }
        let library = tcc("#pragma comment(lib, \"pthread\")\nint x;\n", &[]);
        succeeds(&library, &["--compiler-identity=tcc"]);
        assert!(
            text(&library.stderr).contains(
                "warning: '#pragma comment(lib, \"pthread\")' is ignored: bcc adds no library \
                 from a pragma; link with -lpthread"
            ),
            "{}",
            text(&library.stderr)
        );
        let pragma = "#pragma comment(option, \"-fbogus\")";
        let text_only = tcc(&format!("{pragma}\nint x;\n"), &["-E"]);
        succeeds(&text_only, &["-E"]);
        assert!(
            text(&text_only.stdout).contains(pragma),
            "{}",
            text(&text_only.stdout)
        );
    }

    fn tcc(source: &str, args: &[&str]) -> Output {
        run_source(
            source,
            &[
                &[
                    "--compiler-identity=tcc",
                    "--target=x86_64-unknown-linux-gnu",
                ][..],
                args,
            ]
            .concat(),
        )
    }

    /// Runs bcc with `args` on a file holding `source`.
    fn run_source(source: &str, args: &[&str]) -> Output {
        let path = temporary("pragma.c");
        std::fs::write(&path, source).unwrap();
        let output = bcc(&[args, &[path.to_str().unwrap()]].concat());
        std::fs::remove_file(path).unwrap();
        output
    }

    fn succeeds(output: &Output, args: &[&str]) {
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&output.stderr)
        );
    }

    fn bcc(args: &[&str]) -> Output {
        run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH"),
        )
        .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "bcc-ms-bitfields-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
