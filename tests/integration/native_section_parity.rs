//! C section attributes against pinned Clang's ELF headers on both native
//! paths.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Integration tests own process output and temporary paths."
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
    };

    use crate::native::run_native;

    /// Each input is compiled in isolation: Clang can otherwise reuse a
    /// previously declared NOBITS section and silently drop nonzero bytes.
    #[test]
    fn c_section_attribute_parity_table() {
        if !cfg!(target_os = "linux") {
            eprintln!("ELF section parity requires the host readelf utility");
            return;
        }
        let directory = temporary();
        std::fs::create_dir_all(&directory).unwrap();
        // Exact writable `.rodata` has a separate pre-existing conflict
        // with the assembler's reserved readonly section. These rows cover
        // the reviewed dotted family and its non-family boundary spelling.
        let sections = [
            ".bss",
            ".bss.z",
            ".bssx",
            ".sbss",
            ".sbss.z",
            ".sbssx",
            ".gnu.linkonce.b.z",
            ".llvm.linkonce.b.z",
            ".tbss",
            ".tbss.z",
            ".tdata",
            ".tdata.z",
            ".note",
            ".note.mine",
            ".notefoo",
            ".ldata",
            ".ldata.z",
            ".ldatax",
            ".lbss",
            ".lbss.z",
            ".lbssx",
            ".lrodata.z",
            ".rodatax",
            ".rodata.z",
            "custom",
            ".init_array",
            ".init_array.101",
            ".init_arrayx",
            ".fini_array",
            ".fini_array.101",
            ".preinit_array",
            ".preinit_array.101",
        ];
        for section in sections {
            for declaration in [
                "int x;",
                "int x = 0;",
                "int x = 5;",
                "const int x = 0;",
                "int f(void) { return 5; }",
                "int *x = &other;",
                "double x = -0.0;",
                "int x[4] = {0, 0, 0, 1};",
                "char x[] = \"hi\";",
                "__attribute__((selectany)) int x = 0;",
                "__attribute__((selectany)) const int x = 0;",
                "__attribute__((selectany)) int x = 5;",
            ] {
                // Array rows explicitly account for the assembler's mandatory
                // W; every other row requires exact Clang -c flags.
                let array = [".init_array", ".fini_array", ".preinit_array"]
                    .iter()
                    .any(|stem| section == *stem || section.starts_with(&format!("{stem}.")));
                let assembler_write =
                    array && (declaration.contains("const") || declaration.contains("int f("));
                check_case(&directory, section, declaration, assembler_write);
            }
        }
        // BSS fixups include addresses with nonzero and zero addends; a
        // numerical zero or overwritten nonzero initializer stays accepted.
        for declaration in [
            "int *x = &other + 1;",
            "int *x = 0;",
            "double x = 0.0;",
            "int x[2] = {[0] = 5, [0] = 0};",
            "__attribute__((selectany)) int *x = &other;",
        ] {
            check_case(&directory, ".sbss.z", declaration, false);
        }
        // A redeclaration and an inherited section still diagnose at
        // the function definition, rather than failing in the assembler.
        for source in [
            "__attribute__((section(\".bss.z\"))) int f(void);\nint f(void) { return 1; }",
            "int f(void);\n__attribute__((section(\".sbss.z\"))) int f(void);\nint f(void) { \
             return 1; }",
        ] {
            let path = directory.join("input.c");
            std::fs::write(&path, source).unwrap();
            let result = bcc(&path, &directory.join("refused.o"), false, false);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{source}: {result:?}");
            assert!(stderr.contains("cannot have non-zero bytes"), "{stderr}");
            assert!(
                stderr.contains("input.c:1:")
                    || stderr.contains("input.c:2:")
                    || stderr.contains("input.c:3:"),
                "{stderr}"
            );
        }
        // Unused static and C99 inline-only definitions are omitted before
        // object emission, so Clang does not reject their section contents.
        for declaration in [
            "static int f(void) { return 1; }",
            "inline int f(void) { return 1; }",
            "static inline int f(void) { return 1; }",
        ] {
            let path = directory.join("input.c");
            std::fs::write(
                &path,
                format!("__attribute__((section(\".bss.z\"))) {declaration}\n"),
            )
            .unwrap();
            let reference = run_native(
                Command::new(clang())
                    .args(["--target=x86_64-unknown-linux-gnu", "-c"])
                    .arg(&path)
                    .arg("-o")
                    .arg(directory.join("clang.o")),
            )
            .unwrap();
            assert!(reference.status.success(), "{declaration}: {reference:?}");
            for direct in [false, true] {
                let result = bcc(&path, &directory.join("omitted.o"), direct, false);
                assert!(result.status.success(), "{declaration}: {result:?}");
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    fn check_case(directory: &Path, section: &str, declaration: &str, assembler_write: bool) {
        let source = directory.join("input.c");
        let reference = directory.join("clang.o");
        std::fs::write(
            &source,
            format!("extern int other;\n__attribute__((section(\"{section}\"))) {declaration}\n"),
        )
        .unwrap();
        let clang = run_native(
            Command::new(clang())
                .args(["--target=x86_64-unknown-linux-gnu", "-fms-extensions", "-c"])
                .arg(&source)
                .arg("-o")
                .arg(&reference),
        )
        .unwrap();
        let expected = clang
            .status
            .success()
            .then(|| section_header(&reference, section));
        let diagnostic = String::from_utf8_lossy(&clang.stderr);
        for optimize in [false, true] {
            let mut text_header = None;
            for direct in [false, true] {
                let output = directory.join(if direct { "direct.o" } else { "text.o" });
                if output.exists() {
                    std::fs::remove_file(&output).unwrap();
                }
                let result = bcc(&source, &output, direct, optimize);
                assert_eq!(
                    result.status.success(),
                    clang.status.success(),
                    "{section}: {declaration}; direct={direct} opt={optimize}\nclang: \
                     {diagnostic}\nbcc: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                if let Some((kind, flags)) = &expected {
                    let actual = section_header(&output, section);
                    let mut expected_flags = flags.clone();
                    if assembler_write && !expected_flags.contains(&'W') {
                        // Clang -S followed by its assembler has this same
                        // difference from Clang -c. No section directive can
                        // clear the assembler's name-default SHF_WRITE.
                        expected_flags.push('W');
                        expected_flags.sort_unstable();
                    }
                    assert_eq!(
                        actual,
                        (kind.clone(), expected_flags),
                        "{section}: {declaration}; direct={direct} opt={optimize}"
                    );
                    if direct {
                        assert_eq!(text_header, Some(actual), "native paths disagree");
                    } else {
                        text_header = Some(actual);
                    }
                } else {
                    assert!(!output.exists(), "refusal must not create an object");
                    let message = if diagnostic.contains("cannot have fixups") {
                        "cannot have fixups"
                    } else {
                        "cannot have non-zero bytes"
                    };
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    assert!(
                        stderr.contains(&format!("BSS section '{section}' {message}")),
                        "{stderr}"
                    );
                    if declaration.contains("int f(") {
                        assert!(stderr.contains("input.c:2:"), "{stderr}");
                    }
                }
            }
        }
    }

    fn bcc(source: &Path, object: &Path, direct: bool, optimize: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bcc-rust"));
        _ = command.args([
            "--backend=native",
            "--target=x86_64-unknown-linux-gnu",
            "-fms-extensions",
            "-c",
        ]);
        if direct {
            _ = command.arg("--native-object");
        }
        if optimize {
            _ = command.arg("--opt=bcc");
        }
        run_native(command.arg(source).arg("-o").arg(object)).unwrap()
    }

    fn section_header(object: &Path, name: &str) -> (String, Vec<char>) {
        let output = run_native(Command::new("readelf").arg("-SW").arg(object)).unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let fields = stdout
            .lines()
            .find_map(|line| {
                let (_, rest) = line.split_once(']')?;
                let fields = rest.split_whitespace().collect::<Vec<_>>();
                (fields.first().copied() == Some(name)).then_some(fields)
            })
            .unwrap_or_else(|| panic!("missing {name}: {stdout}"));
        let mut flags = fields[6].chars().collect::<Vec<_>>();
        flags.sort_unstable();
        (fields[1].to_owned(), flags)
    }

    fn clang() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/llvm/bin/clang")
    }

    fn temporary() -> PathBuf {
        std::env::temp_dir().join(format!("bcc-native-section-parity-{}", std::process::id()))
    }
}
