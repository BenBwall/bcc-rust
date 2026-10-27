//! The explorer's outputs are the command line's: each view equals what
//! `bcc-rust -nostdlibinc --input SOURCE` prints for the matching flag, and the
//! machine code equals the text of the object `--native-object` writes.

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

    /// Runs the command line on `source`. `-nostdlibinc` leaves out the
    /// host's C library headers, which the command line searches when
    /// compiling for the host and the explorer never reads.
    fn bcc(arguments: &[&str], source: &str) -> (String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(arguments)
            .args(["-nostdlibinc", "--source-date-epoch=0", "--input", source])
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
                        output.preprocessed.as_deref(),
                        Some(&*with(&["-E"]).0),
                        "{name}"
                    );
                    assert_eq!(
                        output.syntax_tree.as_deref(),
                        Some(&*with(&["--syntax-tree"]).1),
                        "{name}"
                    );
                    let located = compile(
                        &source,
                        &ExplorerOptions {
                            syntax_locations: true,
                            ..options(optimize)
                        },
                    );
                    assert_eq!(
                        located.syntax_tree.as_deref(),
                        Some(&*with(&["--syntax-tree", "--syntax-locations"]).1),
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

    /// One program that exercises each front-end phase.
    const FRONT_END: &str =
        "#include <limits.h>\n#define SQUARE(x) ((x) * (x))\n#define STR(x) #x\n#define CAT(a, b) \
         a ## b\n#define DEBUG 0\nint long_name_\\\nsplit = 3; /* a comment */\n#if DEBUG\nint \
         debug_only;\n#else\nint CAT(re, lease) = SQUARE(2 + 1);\n#endif\nconst char *s = STR(a  \
         +  b) \"!\"; // trailing\nint max = INT_MAX;\n??=define T 1\n";

    #[test]
    fn logical_lines_mark_splices_and_trigraphs() {
        let output = compile(FRONT_END, &options(false));
        let lines = output.logical_lines.expect("phases 1-2 always run");
        // Fifteen physical lines, two of them spliced into one.
        assert_eq!(lines.len(), 14, "{lines:#?}");
        let joined = &lines[5];
        assert_eq!(joined.text, "int long_name_split = 3; /* a comment */");
        assert_eq!((joined.source.start, joined.source.end), (6, 7));
        assert_eq!(joined.changes.len(), 1, "{joined:#?}");
        let splice = &joined.changes[0];
        assert_eq!(
            (
                splice.kind.as_str(),
                splice.column,
                splice.line,
                splice.source_column
            ),
            ("splice", 15, 6, 15)
        );
        assert_eq!(
            (splice.spelling.as_str(), splice.replacement.as_str()),
            ("\\\n", "")
        );
        // Later lines keep their physical numbers.
        assert_eq!(
            (lines[6].source.start, lines[6].text.as_str()),
            (8, "#if DEBUG")
        );
        let trigraph = &lines[13];
        assert_eq!(trigraph.text, "#define T 1");
        assert_eq!(
            (
                trigraph.changes[0].kind.as_str(),
                trigraph.changes[0].spelling.as_str(),
                trigraph.changes[0].replacement.as_str()
            ),
            ("trigraph", "??=", "#")
        );
        // Without trigraphs (a GNU mode), `??=` stays.
        let gnu = compile(
            FRONT_END,
            &ExplorerOptions {
                std: "gnu17".to_owned(),
                ..options(false)
            },
        );
        let unchanged = &gnu.logical_lines.unwrap()[13];
        assert_eq!(unchanged.text, "??=define T 1", "{unchanged:#?}");
        let crlf = compile("int a; /* é */\r\nint b;\r\n", &options(false));
        let lines = crlf.logical_lines.unwrap();
        let kinds: Vec<&str> = lines[0]
            .changes
            .iter()
            .map(|change| change.kind.as_str())
            .collect();
        assert_eq!(kinds, ["extended", "crlf"]);
        assert_eq!(
            (lines[0].changes[0].column, lines[0].changes[1].column),
            (11, 15)
        );
        assert_eq!(lines[1].source.start, 2);
    }

    #[test]
    fn pp_tokens_are_the_lexers_decomposition() {
        let output = compile(FRONT_END, &options(false));
        let tokens = output.pp_tokens.expect("phase 3 always runs");
        let find = |spelling: &str| {
            tokens
                .iter()
                .find(|token| token.spelling == spelling)
                .unwrap_or_else(|| panic!("{spelling}: {tokens:#?}"))
        };
        let spliced = find("long_name_split");
        assert_eq!(spliced.kind, "identifier");
        assert_eq!(
            (spliced.line, spliced.end_line, spliced.end_column),
            (6, 7, 6)
        );
        assert!(spliced.leading_space && !spliced.line_start);
        let comment = tokens
            .iter()
            .find(|token| token.comment)
            .expect("a comment");
        assert_eq!(
            (comment.kind.as_str(), comment.spelling.as_str()),
            ("whitespace", " ")
        );
        // The run of white space starts before the comment.
        assert_eq!((comment.line, comment.column), (7, 11));
        let hash = find("#");
        assert!(hash.line_start && hash.kind == "punctuator");
        assert_eq!(find("3").kind, "pp-number");
        assert_eq!(find("\"!\"").kind, "string-literal");
        let header: Vec<&str> = tokens
            .iter()
            .filter(|token| token.header_name)
            .map(|token| token.spelling.as_str())
            .collect();
        assert_eq!(header, ["<", "limits", ".", "h", ">"]);
        assert!(tokens.iter().any(|token| token.kind == "newline"));
        let universal = compile("int caf\\u00e9 = 1;\n", &options(false))
            .pp_tokens
            .unwrap();
        let name = universal
            .iter()
            .find(|token| token.spelling.starts_with("caf"))
            .unwrap();
        assert_eq!(name.canonical.as_deref(), Some("café"));
    }

    #[test]
    fn the_preprocessing_trace_follows_each_expansion() {
        let output = compile(FRONT_END, &options(false));
        let trace = output.preprocessing.expect("phase 4 always runs");
        let input: Vec<_> = trace
            .expansions
            .iter()
            .filter(|expansion| expansion.file == "<input>")
            .collect();
        let shown: Vec<(&str, &str)> = input
            .iter()
            .map(|expansion| (expansion.invocation.as_str(), expansion.result.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("DEBUG", ""),
                ("CAT(re, lease)", "release"),
                ("SQUARE(2 + 1)", "((2 + 1) * (2 + 1))"),
                ("STR(a + b)", "\"a + b\""),
                ("INT_MAX", "2147483647"),
            ],
            "{input:#?}"
        );
        let square = &input[2].steps[0];
        assert_eq!(
            (square.kind.as_str(), square.macro_name.as_str()),
            ("replace", "SQUARE")
        );
        assert_eq!(
            (
                square.arguments[0].parameter.as_str(),
                square.arguments[0].argument.as_str()
            ),
            ("x", "2 + 1")
        );
        assert!(input[1].steps.iter().any(|step| step.kind == "paste"
            && step.operands == ["re", "lease"]
            && step.result == "release"));
        assert!(input[3].steps.iter().any(|step| step.kind == "stringify"
            && step.macro_name == "STR"
            && step.result == "\"a + b\""));
        assert!(
            input[4].steps.iter().any(|step| step.depth == 1),
            "{:#?}",
            input[4]
        );
        let conditions: Vec<(u32, &str, bool)> = trace
            .conditionals
            .iter()
            .filter(|conditional| conditional.file == "<input>")
            .map(|conditional| {
                (
                    conditional.line,
                    conditional.directive.as_str(),
                    conditional.taken,
                )
            })
            .collect();
        assert_eq!(conditions, [(8, "#if DEBUG", false), (10, "#else", true)]);
        let skipped: Vec<(u32, u32)> = trace
            .skipped
            .iter()
            .filter(|skipped| skipped.file == "<input>")
            .map(|skipped| (skipped.lines.start, skipped.lines.end))
            .collect();
        assert_eq!(skipped, [(9, 9)]);
        let include = &trace.includes[0];
        assert_eq!(
            (include.header.as_str(), include.angled, include.line),
            ("limits.h", true, 1)
        );
        assert!(
            include.found.is_some() && include.system && include.lines > 10,
            "{include:#?}"
        );
        let defined: Vec<(&str, &str)> = trace
            .definitions
            .iter()
            .filter(|definition| definition.file == "<input>")
            .map(|definition| (definition.name.as_str(), definition.kind.as_str()))
            .collect();
        assert_eq!(
            defined,
            [
                ("SQUARE", "function"),
                ("STR", "function"),
                ("CAT", "function"),
                ("DEBUG", "object"),
                ("T", "object")
            ]
        );
        let text = output.preprocessed.unwrap();
        assert!(
            text.contains("int release = ((2 + 1) * (2 + 1));"),
            "{text}"
        );
        assert!(text.contains("# 1 \"<input>\""), "{text}");
    }

    #[test]
    fn each_skipped_group_is_traced_with_its_own_lines() {
        let source = "#if 0\na\n#elif 0\nb\n#elif 1\nc\n#else\nd\n#endif\n";
        let trace = compile(source, &options(false)).preprocessing.unwrap();
        let conditions: Vec<(u32, bool)> = trace
            .conditionals
            .iter()
            .map(|conditional| (conditional.line, conditional.taken))
            .collect();
        assert_eq!(conditions, [(1, false), (3, false), (5, true), (7, false)]);
        let skipped: Vec<(u32, u32)> = trace
            .skipped
            .iter()
            .map(|skipped| (skipped.lines.start, skipped.lines.end))
            .collect();
        assert_eq!(skipped, [(2, 2), (4, 4), (8, 8)]);
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
    fn line_maps_find_functions_by_their_assembler_names() {
        let output = compile(
            "int f(void) __asm__(\"g\");\nint f(void) {\n    return 0;\n}\n",
            &ExplorerOptions::default(),
        );
        assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
        let [mapping] = &*output.line_map else {
            panic!("{:#?}", output.line_map);
        };
        assert_eq!(mapping.function, "f");
        assert_eq!((mapping.source.start, mapping.source.end), (2, 4));
        for (text, range, header) in [
            (&output.ir, mapping.ir, "function @g("),
            (&output.ir_abi, mapping.ir_abi, "function @g("),
            (&output.asm, mapping.asm, "g:"),
        ] {
            let range = range.unwrap_or_else(|| panic!("{mapping:#?}"));
            let first = text
                .as_deref()
                .unwrap()
                .lines()
                .nth(range.start as usize - 1);
            assert!(
                first.is_some_and(|line| line.trim_start().starts_with(header)),
                "{first:?}"
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
