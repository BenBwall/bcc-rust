//! File-scope GNU basic assembly through both back ends: LLVM module asm and
//! the native listing keep every block, in source order and ahead of the
//! code as Clang places it; the paths that cannot carry it refuse it.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        env,
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

    use crate::native::run_native;

    const LINUX: &str = "--target=x86_64-unknown-linux-gnu";

    /// `answer` exists only in the first block; the last block follows the
    /// code in the source.
    const PROGRAM: &str = "__asm__(\".globl answer\\nanswer:\\n\\tmovl $42, %eax\\n\\tret\");
int answer(void);
int main(void) { return answer(); }
__asm__(\"# last \\\"block\\\"\");
";

    #[test]
    fn llvm_module_asm_keeps_every_block_in_source_order() {
        let output = bcc(PROGRAM, &[LINUX, "--emit=llvm"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let llvm = text(&output.stdout);
        let Some((_, block)) = llvm.split_once("module asm\n") else {
            panic!("no module asm:\n{llvm}");
        };
        assert!(
            block.starts_with(
                "    \".globl answer\"\n    \"answer:\"\n    \"\\09movl $42, %eax\"\n    \
                 \"\\09ret\"\n    \"# last \\22block\\22\"\n"
            ),
            "{llvm}"
        );
    }

    #[test]
    fn native_listing_places_blocks_verbatim_before_the_code() {
        let output = bcc(PROGRAM, &[LINUX, "--backend=native", "--emit=asm"]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let listing = text(&output.stdout);
        let expected = "\t.text\n# Start of file scope inline assembly\n.globl \
                        answer\nanswer:\n\tmovl $42, %eax\n\tret\n# last \"block\"\n# End of file \
                        scope inline assembly\n\t.att_syntax\n";
        assert!(listing.starts_with(expected), "{listing}");
        assert!(listing.contains("main:"), "{listing}");
    }

    /// The symbol the first block defines links and runs through each back
    /// end on the host, as Clang's output does. On Windows the native back
    /// end's default, direct COFF objects, gives way to the assembler.
    #[test]
    fn both_back_ends_link_symbols_defined_in_file_scope_asm() {
        let target = if cfg!(all(windows, target_env = "gnu")) {
            "--target=x86_64-w64-windows-gnu"
        } else if cfg!(all(windows, target_env = "msvc")) {
            "--target=x86_64-pc-windows-msvc"
        } else if cfg!(all(target_os = "linux", target_env = "musl")) {
            "--target=x86_64-unknown-linux-musl"
        } else if cfg!(target_os = "linux") {
            LINUX
        } else {
            eprintln!("skipping: the host is not a supported target");
            return;
        };
        if !cfg!(target_arch = "x86_64") {
            eprintln!("skipping: the program's assembly is for x86-64 hosts");
            return;
        }
        for backend in ["--backend=llvm", "--backend=native"] {
            let executable = temporary("asm-answer.exe");
            let built = bcc(
                PROGRAM,
                &[target, backend, "-o", executable.to_str().unwrap()],
            );
            assert!(built.status.success(), "{backend}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(42), "{backend}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// The assembler sees the text: its `.error` reaches the diagnostics.
    /// Both back ends fail the build.
    #[test]
    fn assembler_errors_in_file_scope_asm_are_reported() {
        let source = "__asm__(\".error \\\"file scope boom\\\"\");\nint x;\n";
        for backend in ["--backend=llvm", "--backend=native"] {
            let object = temporary("asm-error.o");
            let built = bcc(
                source,
                &[LINUX, backend, "-c", "-o", object.to_str().unwrap()],
            );
            assert!(
                text(&built.stderr).contains("file scope boom"),
                "{backend}: {}",
                text(&built.stderr)
            );
            assert!(
                !built.status.success(),
                "{backend}: {}",
                text(&built.stderr)
            );
            if object.exists() {
                std::fs::remove_file(object).unwrap();
            }
        }
    }

    /// Direct COFF objects are the native back end's Windows default, but
    /// only the assembler reads file-scope assembly: such a module is
    /// assembled as `--no-native-object` asks, unless `--native-object` comes
    /// last.
    #[test]
    fn windows_native_objects_assemble_file_scope_asm_by_default() {
        for target in [
            "--target=x86_64-pc-windows-msvc",
            "--target=x86_64-w64-windows-gnu",
        ] {
            let compile = |flags: &[&str]| {
                let object = temporary("asm-windows.o");
                let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
                _ = command
                    .args([target, "--backend=native", "-c", "-o"])
                    .arg(&object)
                    .args(flags)
                    .args(["--input", PROGRAM])
                    .env("NO_COLOR", "1");
                if target.ends_with("windows-gnu") {
                    // bcc's toolchain lookup asks for a MinGW root before clang
                    // assembles even an object, on every host; this header-free
                    // source needs none.
                    _ = command.env("MINGW_ROOT", env::temp_dir());
                }
                let output = command.output().unwrap();
                let bytes = std::fs::read(&object).ok();
                if bytes.is_some() {
                    std::fs::remove_file(object).unwrap();
                }
                (output, bytes)
            };
            let (built, default) = compile(&[]);
            assert!(built.status.success(), "{target}: {}", text(&built.stderr));
            let default = default.unwrap();
            assert!(default.starts_with(&[0x64, 0x86]), "{target}: not COFF");
            assert!(default.windows(6).any(|s| s == b"answer"), "{target}");
            for flags in [
                &["--no-native-object"][..],
                &["--native-object", "--no-native-object"][..],
            ] {
                let (built, assembled) = compile(flags);
                assert!(
                    built.status.success(),
                    "{target} {flags:?}: {}",
                    text(&built.stderr)
                );
                assert!(
                    assembled.as_deref() == Some(&default[..]),
                    "{target} {flags:?}"
                );
            }
            let (refused, object) = compile(&["--no-native-object", "--native-object"]);
            assert_eq!(refused.status.code(), Some(1), "{target}");
            assert!(
                text(&refused.stderr).contains("--native-object cannot encode file-scope assembly"),
                "{target}: {}",
                text(&refused.stderr)
            );
            assert!(object.is_none(), "{target}");
        }
    }

    #[test]
    fn direct_objects_and_the_interpreter_refuse_file_scope_asm() {
        for target in [
            LINUX,
            "--target=x86_64-pc-windows-msvc",
            "--target=x86_64-w64-windows-gnu",
        ] {
            let object = temporary("asm-direct.o");
            let refused = bcc(
                PROGRAM,
                &[
                    target,
                    "--backend=native",
                    "--native-object",
                    "-c",
                    "-o",
                    object.to_str().unwrap(),
                ],
            );
            assert_eq!(
                refused.status.code(),
                Some(1),
                "{target}: {}",
                text(&refused.stderr)
            );
            assert!(
                text(&refused.stderr).contains("--native-object cannot encode file-scope assembly"),
                "{target}: {}",
                text(&refused.stderr)
            );
            assert!(!object.exists(), "{target}");
        }
        let interpreted = bcc(PROGRAM, &[LINUX, "--interpret"]);
        assert_eq!(interpreted.status.code(), Some(70));
        assert!(
            text(&interpreted.stderr).contains("the module has file-scope assembly"),
            "{}",
            text(&interpreted.stderr)
        );
    }

    /// Forms Clang rejects get Clang's diagnostic, not the note that calls
    /// unsupported code valid C, and produce no output.
    #[test]
    fn file_scope_asm_clang_rejects_is_diagnosed_as_invalid() {
        for (source, message) in [
            (
                "__asm__ volatile(\"nop\");",
                "error: meaningless 'volatile' on asm outside function",
            ),
            (
                "__asm__ inline(\"nop\");",
                "error: meaningless 'inline' on asm outside function",
            ),
            (
                "__asm__(\"nop\" : );",
                "error: file-scope asm cannot have operands, clobbers or labels",
            ),
            (
                "__asm__(L\"nop\");",
                "error: cannot use wide string literal in 'asm'",
            ),
            (
                "__asm__(u8\"nop\");",
                "error: cannot use unicode string literal in 'asm'",
            ),
        ] {
            let source = format!("{source}\nint x;\n");
            let refused = bcc(&source, &[LINUX, "-std=gnu11", "--emit=llvm"]);
            let stderr = text(&refused.stderr);
            assert_eq!(refused.status.code(), Some(1), "{source}: {stderr}");
            assert!(refused.stdout.is_empty(), "{source}");
            assert!(stderr.contains(message), "{source}: {stderr}");
            assert!(!stderr.contains("valid C"), "{source}: {stderr}");
        }
    }

    /// Clang fails the build when its assembler reports an error in module
    /// asm and leaves no object or executable; so must the LLVM back end.
    #[test]
    fn llvm_assembler_errors_in_file_scope_asm_fail_the_build() {
        let source = "__asm__(\".error \\\"file scope boom\\\"\");\nint main(void) { return 0; }\n";
        for flags in [&["-c"][..], &[][..]] {
            let output = temporary("asm-error-llvm.out");
            let built = bcc(
                source,
                &[
                    &[LINUX, "--backend=llvm", "-o", output.to_str().unwrap()][..],
                    flags,
                ]
                .concat(),
            );
            let stderr = text(&built.stderr);
            assert!(!built.status.success(), "{flags:?}: {stderr}");
            assert!(stderr.contains("file scope boom"), "{flags:?}: {stderr}");
            assert!(!output.exists(), "{flags:?}: the failed build left output");
        }
    }

    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        env::temp_dir().join(format!(
            "bcc-file-asm-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
