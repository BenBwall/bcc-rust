//! The explorer's outputs are the command line's: each view equals what
//! `bcc-rust --input SOURCE` prints for the matching flag, and the machine
//! code equals the text of the object `--native-object` writes.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    clippy::cast_possible_truncation,
    reason = "Test harnesses own process output and file paths outside compilation, and read \
              64-bit ELF fields on 64-bit hosts."
)]
mod tests {
    use std::{
        path::{
            Path,
            PathBuf,
        },
        process::Command,
    };

    use bcc_rust::explorer::{
        ExplorerFile,
        ExplorerOptions,
        ExplorerOutput,
        compile,
        llvm_ir,
    };

    /// Programs every stage handles, from the repository and the site.
    fn programs() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut programs: Vec<_> = ["test-programs/ir-design-study", "web/demos"]
            .iter()
            .flat_map(|directory| std::fs::read_dir(root.join(directory)).unwrap())
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
            .map(|path| {
                (
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read_to_string(&path).unwrap(),
                )
            })
            .collect();
        programs.sort();
        for (name, source) in INLINE {
            programs.push(((*name).to_owned(), (*source).to_owned()));
        }
        programs
    }

    const INLINE: &[(&str, &str)] = &[
        (
            "arithmetic",
            "int add(int a, int b) { return a + b; }\nint main(void) { return add(1, 2); }\n",
        ),
        (
            "floating",
            "#include <float.h>\nlong double third = 1.0L / 3;\ndouble half(double x) { return x \
             * 0.5; }\nfloat tenth = 0.1f;\nint main(void) { return (int)(half(4.0) + third + \
             tenth + LDBL_EPSILON * 0); }\n",
        ),
        (
            "aggregates",
            "#include <stdint.h>\nstruct point { int64_t x, y; double weight; };\nstatic struct \
             point shift(struct point p, int64_t by) { p.x += by; p.y -= by; return p; }\nint \
             main(void) {\n    struct point p = { 1, 2, 0.5 };\n    p = shift(p, 3);\n    return \
             (int)(p.x - p.y);\n}\n",
        ),
        (
            "control",
            "int classify(int c) {\n    switch (c) {\n    case 'a': case 'e': return 1;\n    case \
             ' ': return 2;\n    default: return c > 0 ? 3 : 4;\n    }\n}\nconst char *text = \
             \"hello\";\nint main(void) {\n    int n = 0;\n    for (const char *p = text; *p; \
             ++p) n += classify(*p);\n    while (n > 10) n -= 3;\n    do n++; while (n < 5);\n    \
             return n;\n}\n",
        ),
    ];

    fn bcc(arguments: &[&str], source: &str) -> (String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(arguments)
            .args(["--source-date-epoch=0", "--input", source])
            .output()
            .unwrap();
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    }

    fn options(optimize: bool) -> ExplorerOptions {
        ExplorerOptions {
            optimize,
            ..ExplorerOptions::default()
        }
    }

    #[test]
    fn every_view_matches_the_command_line() {
        for (name, source) in programs() {
            for optimize in [false, true] {
                let output = compile(&source, &options(optimize));
                assert!(
                    output.diagnostics.iter().all(|d| d.severity != "error"),
                    "{name}: {:#?}",
                    output.diagnostics
                );
                let level: &[&str] = if optimize { &["-O2"] } else { &[] };
                let with = |flags: &[&str]| {
                    let mut arguments = vec!["--std=c17"];
                    arguments.extend_from_slice(level);
                    arguments.extend_from_slice(flags);
                    bcc(&arguments, &source)
                };
                // Both views print diagnostics among their text.
                if output.diagnostics.is_empty() {
                    assert_eq!(
                        output.tokens.as_deref(),
                        Some(&*with(&["--tokens"]).1),
                        "{name}"
                    );
                    assert_eq!(
                        output.syntax_tree.as_deref(),
                        Some(&*with(&["--syntax-tree"]).1),
                        "{name}"
                    );
                }
                for (view, flag) in [
                    (&output.ir, "--emit=ir"),
                    (&output.ir_abi, "--emit=ir-abi"),
                    (&output.asm, "--emit=asm"),
                ] {
                    assert_eq!(
                        view.as_deref(),
                        Some(&*with(&[flag, "--backend=native"]).0),
                        "{name} {flag} at -O{}",
                        if optimize { 2 } else { 0 }
                    );
                }
                assert_eq!(
                    llvm_ir(&source, &options(optimize)).unwrap(),
                    with(&["--emit=llvm"]).0,
                    "{name} --emit=llvm"
                );
                check_machine_code(&name, &source, level, &output);
                check_line_map(&name, &output);
            }
        }
    }

    /// Each function's bytes are its instructions' bytes and the bytes of
    /// its symbol in the object `--native-object` writes.
    fn check_machine_code(name: &str, source: &str, level: &[&str], output: &ExplorerOutput) {
        let functions = output.machine_code.as_ref().expect("machine code");
        assert!(!functions.is_empty(), "{name}");
        let object = temporary(&format!("{}.o", name.replace(['/', '.'], "-")));
        let mut arguments = vec![
            "--std=c17",
            "--backend=native",
            "--native-object",
            "-c",
            "-o",
            object.to_str().unwrap(),
        ];
        arguments.extend_from_slice(level);
        let (_, stderr) = bcc(&arguments, source);
        let elf = std::fs::read(&object).unwrap_or_else(|error| panic!("{name}: {error} {stderr}"));
        drop(std::fs::remove_file(&object));
        for function in functions {
            let joined: Vec<u8> = function
                .instructions
                .iter()
                .flat_map(|instruction| instruction.bytes.iter().copied())
                .collect();
            let last = function.instructions.last().unwrap();
            assert_eq!(
                last.offset as usize + last.bytes.len(),
                joined.len()
                    + function
                        .instructions
                        .windows(2)
                        .map(|pair| {
                            pair[1].offset as usize - pair[0].offset as usize - pair[0].bytes.len()
                        })
                        .sum::<usize>(),
                "{name}: {} instructions are contiguous apart from padding",
                function.name
            );
            assert!(function.bytes.starts_with(&function.instructions[0].bytes));
            assert_eq!(
                Some(&function.bytes[..]),
                symbol_bytes(&elf, &function.name),
                "{name}: {}",
                function.name
            );
        }
    }

    /// Every defined function maps to lines in each output that hold it.
    fn check_line_map(name: &str, output: &ExplorerOutput) {
        assert!(!output.line_map.is_empty(), "{name}");
        for mapping in &output.line_map {
            let asm = mapping.asm.unwrap_or_else(|| panic!("{name}: {mapping:?}"));
            let label = output
                .asm
                .as_ref()
                .unwrap()
                .lines()
                .nth(asm.start as usize - 1);
            assert_eq!(label, Some(&*format!("{}:", mapping.function)), "{name}");
            let ir = mapping.ir.unwrap_or_else(|| panic!("{name}: {mapping:?}"));
            let header = output
                .ir
                .as_ref()
                .unwrap()
                .lines()
                .nth(ir.start as usize - 1)
                .unwrap();
            assert!(
                header.starts_with(&format!("function @{}(", mapping.function)),
                "{name}"
            );
            assert!(mapping.source.start <= mapping.source.end);
        }
    }

    /// The bytes of the function symbol `name` in an ELF64 object.
    fn symbol_bytes<'a>(elf: &'a [u8], name: &str) -> Option<&'a [u8]> {
        let u16_at = |at: usize| u16::from_le_bytes(elf[at..at + 2].try_into().unwrap());
        let u32_at = |at: usize| u32::from_le_bytes(elf[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(elf[at..at + 8].try_into().unwrap());
        let sections = u64_at(0x28) as usize;
        let section = |index: usize| sections + index * 64;
        for index in 0..usize::from(u16_at(0x3C)) {
            let header = section(index);
            if u32_at(header + 4) != 2 {
                continue;
            }
            let (offset, size) = (
                u64_at(header + 0x18) as usize,
                u64_at(header + 0x20) as usize,
            );
            let strings = section(u32_at(header + 0x28) as usize);
            let strings = u64_at(strings + 0x18) as usize;
            for symbol in (offset..offset + size).step_by(24) {
                let start = strings + u32_at(symbol) as usize;
                let end = start + elf[start..].iter().position(|&byte| byte == 0)?;
                if &elf[start..end] != name.as_bytes() {
                    continue;
                }
                let text = section(usize::from(u16_at(symbol + 6)));
                let base = u64_at(text + 0x18) as usize + u64_at(symbol + 8) as usize;
                return elf.get(base..base + u64_at(symbol + 16) as usize);
            }
        }
        None
    }

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bcc-explorer-{}-{name}", std::process::id()))
    }

    #[test]
    fn preprocessor_listings_match_the_command_line() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs");
        for name in [
            "define.c",
            "hash-if.c",
            "paste.c",
            "stringify.c",
            "triple-paste.c",
            "x.c",
        ] {
            let source = std::fs::read_to_string(root.join(name)).unwrap();
            let output = compile(&source, &ExplorerOptions::default());
            let (_, stderr) = bcc(&["--std=c17", "--tokens"], &source);
            let mut listing = String::new();
            for line in stderr.lines().filter(|line| line.starts_with("<input>:")) {
                listing.push_str(line);
                listing.push('\n');
            }
            assert_eq!(output.tokens.as_deref(), Some(&*listing), "{name}");
        }
    }

    #[test]
    fn diagnostics_render_as_the_command_line_prints_them() {
        for source in [
            "int main(void) { return 1 +; }\n",
            "int f(void);\nint main(void) { int x = y; return f(x) }\n",
            "#include <missing.h>\nint x;\n",
            "#warning careful\nint unused(int a) { int a2 = a << 40; return a2; }\n",
        ] {
            let output = compile(source, &ExplorerOptions::default());
            let (_, stderr) = bcc(&["--std=c17"], source);
            let rendered: String = output
                .diagnostics
                .iter()
                .filter(|d| d.stage == "front-end")
                .map(|d| d.rendered.as_str())
                .collect();
            let summary = stderr.rfind("\n\n").map_or(0, |at| at + 2);
            assert_eq!(rendered, stderr[..summary], "{source}");
            for diagnostic in &output.diagnostics {
                assert_eq!(
                    diagnostic.file.as_deref(),
                    Some("<input>"),
                    "{diagnostic:?}"
                );
                let (line, column) = (diagnostic.line.unwrap(), diagnostic.column.unwrap());
                let (end_line, end_column) =
                    (diagnostic.end_line.unwrap(), diagnostic.end_column.unwrap());
                assert!((line, column) <= (end_line, end_column), "{diagnostic:?}");
                assert!(
                    diagnostic
                        .rendered
                        .contains(&format!("<input>:{line}:{column}")),
                    "{diagnostic:?}"
                );
            }
            let errors = output.diagnostics.iter().any(|d| d.severity == "error");
            assert_eq!(output.ir.is_none(), errors, "{source}");
            assert!(output.tokens.is_some() && output.syntax_tree.is_some());
        }
    }

    #[test]
    fn diagnostic_places_are_physical_despite_line_directives() {
        for (source, presumed) in [
            ("#line 100\nint x = y;\n", "<input>:100:9"),
            (
                "#line 100 \"generated.c\"\nint x = y;\n",
                "generated.c:100:9",
            ),
        ] {
            let output = compile(source, &ExplorerOptions::default());
            let [diagnostic] = &*output.diagnostics else {
                panic!("{:#?}", output.diagnostics);
            };
            assert_eq!(
                (
                    diagnostic.file.as_deref(),
                    diagnostic.line,
                    diagnostic.column,
                    diagnostic.end_line,
                    diagnostic.end_column
                ),
                (Some("<input>"), Some(2), Some(9), Some(2), Some(10)),
                "{diagnostic:#?}"
            );
            assert!(diagnostic.rendered.contains(presumed), "{diagnostic:#?}");
        }
    }

    #[test]
    fn line_maps_use_physical_lines_of_the_input() {
        for source in [
            "#line 100\nint f(void) {\n    return 0;\n}\n",
            "#line 100 \"generated.c\"\nint f(void) {\n    return 0;\n}\n",
        ] {
            let output = compile(source, &ExplorerOptions::default());
            assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
            let [mapping] = &*output.line_map else {
                panic!("{source:?}: {:#?}", output.line_map);
            };
            assert_eq!(
                (mapping.source.start, mapping.source.end),
                (2, 4),
                "{source:?}"
            );
        }
    }

    #[test]
    fn host_files_are_never_found_or_read() {
        let host = temporary("host.h");
        std::fs::write(&host, "int host_header;\n").unwrap();
        let absolute = host.display();
        // Relative names would be found beside `<input>`, in the working
        // directory, which holds `Cargo.toml` while tests run.
        let source = format!(
            "#if __has_include(\"{absolute}\") || __has_include(\"Cargo.toml\") || \
             __has_include(<Cargo.toml>) || __has_embed(\"Cargo.toml\")\n#error host file \
             found\n#endif\n#if __has_embed(\"data.bin\") != 1 || __has_embed(\"empty.bin\") != \
             2\n#error in-memory resource not found\n#endif\nconst unsigned char data[] = \
             {{\n#embed \"data.bin\"\n}};\n#include <stdint.h>\n#include \"{absolute}\"\n#embed \
             \"Cargo.toml\"\n"
        );
        let output = compile(
            &source,
            &ExplorerOptions {
                std: "c23".into(),
                files: vec![
                    ExplorerFile {
                        name:     "data.bin".into(),
                        contents: "AB".into(),
                    },
                    ExplorerFile {
                        name:     "empty.bin".into(),
                        contents: String::new(),
                    },
                ],
                ..ExplorerOptions::default()
            },
        );
        std::fs::remove_file(&host).unwrap();
        let lines: Vec<_> = output
            .diagnostics
            .iter()
            .map(|d| (d.severity.as_str(), d.line))
            .collect();
        // Only the host header's `#include` and the host resource's `#embed`
        // fail, each because nothing is found.
        assert_eq!(
            lines,
            [("error", Some(11)), ("error", Some(12))],
            "{:#?}",
            output.diagnostics
        );
        assert!(
            output
                .diagnostics
                .iter()
                .all(|d| d.message.starts_with("cannot find")),
            "{:#?}",
            output.diagnostics
        );
        assert!(!output.tokens.unwrap().contains("host_header"));
    }

    #[test]
    fn in_memory_headers_are_included_and_deep_nesting_is_a_diagnostic() {
        let options = ExplorerOptions {
            files: vec![ExplorerFile {
                name:     "twice.h".into(),
                contents: "#define TWICE(x) ((x) * 2)\nint twice(int);\n".into(),
            }],
            ..ExplorerOptions::default()
        };
        let output = compile(
            "#include \"twice.h\"\n#include <twice.h>\nint twice(int x) { return TWICE(x); }\n",
            &options,
        );
        assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
        assert!(output.asm.unwrap().contains("twice:"));

        let deep = format!("int x = {}1{};\n", "(".repeat(5000), ")".repeat(5000));
        let output = compile(
            &deep,
            &ExplorerOptions {
                max_nesting_depth: Some(100),
                ..ExplorerOptions::default()
            },
        );
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.severity == "error" && d.message.contains("nest")),
            "{:#?}",
            output.diagnostics
        );
        assert!(output.ir.is_none());

        let output = compile(
            "int x;",
            &ExplorerOptions {
                target: "x86_64-pc-windows-msvc".into(),
                ..ExplorerOptions::default()
            },
        );
        assert!(output.asm.is_some() && output.machine_code.is_none());
        assert_eq!(output.diagnostics[0].severity, "note");

        let output = compile(
            "int x;",
            &ExplorerOptions {
                std: "c99x".into(),
                ..ExplorerOptions::default()
            },
        );
        assert_eq!(output.diagnostics.len(), 1);
        assert!(output.tokens.is_none());
    }
}
