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
        ExplorerLineMapping,
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
                    // The explorer shows phase 4's text, which `-E` writes
                    // after phases 5 and 6 have converted and joined string
                    // literals; without them the two are the same.
                    let preprocessed = with(&["-E"]).0;
                    if !preprocessed
                        .lines()
                        .any(|line| !line.starts_with('#') && line.contains(['"', '\'']))
                    {
                        assert_eq!(
                            output.preprocessed.as_deref(),
                            Some(&*preprocessed),
                            "{name}"
                        );
                    }
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
                    assert_eq!(
                        output.semantic_types.as_deref(),
                        Some(&*with(&["--semantic-types"]).1),
                        "{name}"
                    );
                    assert_eq!(
                        located.semantic_types.as_deref(),
                        Some(&*with(&["--semantic-types", "--syntax-locations"]).1),
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
        // A Windows path names a fixture with either separator.
        let object = temporary(&format!("{}.o", name.replace(['/', '\\', '.'], "-")));
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
        // Each IR line that names a source line lies in a function of the
        // input and names a line of that function.
        for (text, lines, range) in [
            (
                &output.ir,
                &output.ir_lines,
                (|mapping: &ExplorerLineMapping| mapping.ir) as fn(&_) -> _,
            ),
            (&output.ir_abi, &output.ir_abi_lines, |mapping| {
                mapping.ir_abi
            }),
        ] {
            assert_eq!(
                lines.len(),
                text.as_ref().unwrap().lines().count(),
                "{name}"
            );
            for (index, &line) in lines.iter().enumerate().filter(|&(_, &line)| line != 0) {
                let output_line = u32::try_from(index + 1).unwrap();
                let mapping = output
                    .line_map
                    .iter()
                    .find(|mapping| {
                        range(mapping).is_some_and(|ir| (ir.start..=ir.end).contains(&output_line))
                    })
                    .unwrap_or_else(|| panic!("{name}: IR line {output_line} is in no function"));
                assert!(
                    (mapping.source.start..=mapping.source.end).contains(&line),
                    "{name}: IR line {output_line} names line {line}, outside {mapping:?}"
                );
            }
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
        assert!(input[0].directive && !input[1].directive);
        let text = output.preprocessed.unwrap();
        assert!(
            text.contains("int release = ((2 + 1) * (2 + 1));"),
            "{text}"
        );
        // Phase 4 leaves adjacent string literals apart; phase 6 joins them.
        assert!(text.contains("const char *s = \"a + b\" \"!\";"), "{text}");
        assert!(text.contains("# 1 \"<input>\""), "{text}");
    }

    /// What phase 4 did with `source`.
    fn trace_of(source: &str) -> bcc_rust::explorer::ExplorerPreprocessing {
        compile(source, &options(false)).preprocessing.unwrap()
    }

    #[test]
    fn argument_prescans_are_steps_of_their_expansion() {
        let trace = trace_of("#define A(x) x+x\n#define B(x) A(x)\nint a = A(B(2));\n");
        let [expansion] = &trace.expansions[..] else {
            panic!("{:#?}", trace.expansions);
        };
        assert_eq!(expansion.invocation, "A(B(2))");
        assert_eq!(expansion.result, "2+2+2+2");
        let steps: Vec<(&str, u32, bool, String)> = expansion
            .steps
            .iter()
            .map(|step| {
                let arguments: Vec<String> = step
                    .arguments
                    .iter()
                    .map(|argument| format!("{}={}", argument.parameter, argument.argument))
                    .collect();
                (
                    step.macro_name.as_str(),
                    step.depth,
                    step.prescan,
                    arguments.join(","),
                )
            })
            .collect();
        // B is replaced while A's argument is prescanned (C99 §6.10.3.1p1),
        // and the A of B's replacement list inside that.
        assert_eq!(
            steps,
            [
                ("A", 0, false, "x=B(2)".to_owned()),
                ("B", 1, true, "x=2".to_owned()),
                ("A", 2, false, "x=2".to_owned()),
            ]
        );
    }

    #[test]
    fn predefined_macros_and_every_directive_are_traced() {
        let trace = trace_of("#pragma once\nint line = __LINE__;\n#if __LINE__ > 1\n#endif\n");
        let shown: Vec<(&str, &str, bool)> = trace
            .expansions
            .iter()
            .map(|expansion| {
                (
                    expansion.invocation.as_str(),
                    expansion.result.as_str(),
                    expansion.directive,
                )
            })
            .collect();
        assert_eq!(shown, [("__LINE__", "2", false), ("__LINE__", "", true)]);
        assert_eq!(trace.expansions[0].steps[0].kind, "builtin");
        assert_eq!(trace.expansions[1].steps[0].result, "3");
        // The target's predefined macros are directives of their own file.
        let directives: Vec<(u32, &str)> = trace
            .directives
            .iter()
            .filter(|directive| directive.file == "<input>")
            .map(|directive| (directive.line, directive.name.as_str()))
            .collect();
        assert_eq!(directives, [(1, "pragma"), (3, "if"), (4, "endif")]);
    }

    #[test]
    fn an_empty_replacement_is_not_a_directive() {
        let trace = trace_of("#define EMPTY\nint x EMPTY;\n");
        let expansion = &trace.expansions[0];
        assert_eq!(
            (expansion.result.as_str(), expansion.directive),
            ("", false)
        );
    }

    #[test]
    fn line_directives_leave_the_trace_physical() {
        let source = "int a;\n#line 1000 \"fake.c\"\n#if 0\nint b;\n#else\nint c;\n#endif\n";
        let trace = trace_of(source);
        let conditions: Vec<(&str, u32, &str, Option<&str>)> = trace
            .conditionals
            .iter()
            .map(|conditional| {
                (
                    conditional.file.as_str(),
                    conditional.line,
                    conditional.directive.as_str(),
                    conditional.presumed.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            conditions,
            [
                ("<input>", 3, "#if 0", Some("fake.c:1000")),
                ("<input>", 5, "#else", Some("fake.c:1002")),
            ]
        );
        let skipped: Vec<(&str, u32, u32)> = trace
            .skipped
            .iter()
            .map(|skipped| {
                (
                    skipped.file.as_str(),
                    skipped.lines.start,
                    skipped.lines.end,
                )
            })
            .collect();
        assert_eq!(skipped, [("<input>", 4, 4)]);
    }

    #[test]
    fn each_arm_after_a_processed_group_is_traced() {
        let trace = trace_of("#if 1\na\n#elif 0\nb\n#elif 0\nc\n#else\nd\n#endif\n");
        let conditions: Vec<(u32, bool, bool)> = trace
            .conditionals
            .iter()
            .map(|conditional| (conditional.line, conditional.taken, conditional.evaluated))
            .collect();
        assert_eq!(
            conditions,
            [
                (1, true, true),
                (3, false, false),
                (5, false, false),
                (7, false, false),
            ]
        );
        let skipped: Vec<(u32, u32)> = trace
            .skipped
            .iter()
            .map(|skipped| (skipped.lines.start, skipped.lines.end))
            .collect();
        assert_eq!(skipped, [(4, 4), (6, 6), (8, 8)]);
    }

    #[test]
    fn preprocessed_lines_say_which_include_brought_a_header_in() {
        let source = "int a;\n#include <stdint.h>\nint32_t x;\n";
        let output = compile(source, &options(false));
        let text = output.preprocessed.unwrap();
        let lines = output.preprocessed_lines.unwrap();
        assert_eq!(lines.len(), text.lines().count(), "{text}");
        let of = |wanted: &str| {
            let index = text
                .lines()
                .position(|line| line == wanted)
                .unwrap_or_else(|| panic!("{wanted}: {text}"));
            &lines[index]
        };
        let first = of("int a;");
        assert_eq!(
            (first.file.as_str(), first.line, first.include),
            ("<input>", Some(1), None)
        );
        assert_eq!(of("int32_t x;").line, Some(3));
        let header = text
            .lines()
            .position(|line| line.starts_with("typedef"))
            .map(|index| &lines[index])
            .expect("stdint.h declares types");
        assert!(
            header.file.ends_with("stdint.h") && header.line.is_some(),
            "{header:#?}"
        );
        assert_eq!(header.include, Some(2));
        // Every line from the header, line markers too, names the include.
        for (line, place) in text.lines().zip(&lines) {
            assert_eq!(
                place.include.is_some(),
                place.file != "<input>",
                "{line}: {place:#?}"
            );
        }
    }

    #[test]
    fn every_step_sits_under_the_replacement_it_happened_in() {
        let trace = trace_of(
            "#define CAT_(x,y) x##y\n#define CAT(x,y) CAT_(x,y)\n#define A(x) x+x\n#define B(x) \
             A(x)\n#define BUILTIN() __LINE__\nint a=A(B(BUILTIN()));\nint CAT(x, y)=2;\n",
        );
        let tree = |index: usize| -> Vec<(String, u32, Option<u32>)> {
            trace.expansions[index]
                .steps
                .iter()
                .map(|step| {
                    (
                        format!("{} {}", step.kind, step.macro_name),
                        step.depth,
                        step.parent,
                    )
                })
                .collect()
        };
        let owned = |steps: &[(&str, u32, Option<u32>)]| -> Vec<(String, u32, Option<u32>)> {
            steps
                .iter()
                .map(|&(step, depth, parent)| (step.to_owned(), depth, parent))
                .collect()
        };
        // `__LINE__` is replaced inside BUILTIN's replacement list, which
        // B's argument prescan reads inside A's.
        assert_eq!(
            tree(0),
            owned(&[
                ("replace A", 0, None),
                ("replace B", 1, Some(0)),
                ("replace BUILTIN", 2, Some(1)),
                ("builtin __LINE__", 3, Some(2)),
                ("replace A", 2, Some(1)),
            ])
        );
        assert_eq!(trace.expansions[0].result, "6+6+6+6");
        // The `##` is CAT_'s, not CAT's.
        assert_eq!(
            tree(1),
            owned(&[
                ("replace CAT", 0, None),
                ("replace CAT_", 1, Some(0)),
                ("paste CAT_", 2, Some(1)),
            ])
        );
    }

    #[test]
    fn header_lines_name_their_include_after_a_line_directive() {
        for source in [
            "#line 1000 \"fake.c\"\n#include <stdint.h>\nint32_t x = INT8_MAX;\n",
            // stdatomic.h includes further headers of its own.
            "#line 1000 \"fake.c\"\n#include <stdatomic.h>\natomic_int x;\n",
        ] {
            let output = compile(source, &options(false));
            let text = output.preprocessed.unwrap();
            let lines = output.preprocessed_lines.unwrap();
            let mut from_header = 0;
            for (line, place) in text.lines().zip(&lines) {
                if place.file != "<input>" {
                    from_header += 1;
                    assert_eq!(place.include, Some(2), "{line}: {place:#?}\n{text}");
                }
            }
            assert!(from_header > 0, "{text}");
        }
    }

    #[test]
    fn ir_lines_name_the_source_line_of_each_instruction() {
        let source = "int square(int x) {\n    int y = x * x;\n    return y;\n}\n";
        let output = compile(source, &options(false));
        let ir = output.ir.as_deref().expect("IR");
        let line_of = |needle: &str| {
            let index = ir
                .lines()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} in\n{ir}"));
            output.ir_lines[index]
        };
        assert_eq!(line_of("imul"), 2, "{ir}\n{:?}", output.ir_lines);
        assert_eq!(line_of("return"), 3, "{ir}\n{:?}", output.ir_lines);
        assert_eq!(line_of("function @square"), 0);
        assert!(output.ir_abi_lines.contains(&2));
    }

    #[test]
    fn a_long_line_maps_in_linear_time() {
        // One line of 288 KB: every token used to recount the line.
        let source = "int x;".repeat(48_000) + "\n";
        let started = std::time::Instant::now();
        let output = compile(&source, &options(false));
        let elapsed = started.elapsed();
        let tokens = output.pp_tokens.unwrap();
        let last = tokens
            .iter()
            .rev()
            .find(|token| token.kind == "punctuator")
            .unwrap();
        assert_eq!(
            (last.line, last.column, last.end_column),
            (1, 288_000, 288_001)
        );
        let lines = output.preprocessed_lines.unwrap();
        assert!(lines.iter().any(|line| line.line == Some(1)));
        // The whole compilation in a debug build takes a few seconds; the
        // quadratic mapping took minutes.
        assert!(elapsed.as_secs() < 30, "{elapsed:?}");
    }

    #[test]
    fn a_macro_that_multiplies_its_input_truncates_the_trace() {
        // 30,000 initializer elements from five lines: each produces trace
        // events, which stop at a fixed number instead of filling the
        // preprocessing arena (one fixed region in WebAssembly).
        let source = "#define A 0,0,0,\n#define B A A A A A A A A A A\n#define C B B B B B B B B \
                      B B\n#define D C C C C C C C C C C\n#define E D D D D D D D D D D\nint a[] \
                      = {E 0};\n";
        let output = compile(source, &options(false));
        assert!(
            output.diagnostics.iter().all(|d| d.severity != "error"),
            "{:#?}",
            output.diagnostics
        );
        let trace = output.preprocessing.unwrap();
        assert!(trace.truncated);
        assert_eq!(trace.expansions.len(), 1);
        // The output is whole: every element is there.
        assert_eq!(output.preprocessed.unwrap().matches('0').count(), 30_001);
    }

    #[test]
    fn deeply_nested_arguments_cost_the_trace_a_bounded_spelling() {
        // Each nested invocation spells what is left of its argument, which
        // without a cut grows with the square of the depth and filled an
        // arena in WebAssembly at 400 levels.
        for depth in [400, 2000] {
            let source = format!(
                "#define F(x) x\nint x={}1{};\n",
                "F(".repeat(depth),
                ")".repeat(depth)
            );
            let output = compile(&source, &options(false));
            assert!(
                output.diagnostics.iter().all(|d| d.severity != "error"),
                "{depth}: {:#?}",
                output.diagnostics
            );
            let text = output.preprocessed.unwrap();
            assert!(text.replace(' ', "").contains("intx=1;"), "{text}");
            let trace = output.preprocessing.unwrap();
            let steps = &trace.expansions[0].steps;
            let outer = &steps[0].arguments[0].argument;
            assert!(outer.len() < 200 && outer.ends_with(" …"), "{outer}");
            assert!(steps.len() > 100, "{depth}: {}", steps.len());
        }
    }

    #[test]
    fn an_expansion_open_when_the_trace_stops_is_partial() {
        let mut source = String::from(
            "#define A 0,\n#define B A A A A A A A A A A\n#define C B B B B B B B B B B\n#define \
             D C C C C C C C C C C\n#define P",
        );
        source.push_str(&" D".repeat(15));
        source.push_str(&" B".repeat(98));
        source.push_str(" A 0,\n#define Z 12345\nint a[]={P Z};\n");
        let trace = compile(&source, &options(false)).preprocessing.unwrap();
        assert!(trace.truncated);
        let last = trace.expansions.last().unwrap();
        assert!(last.partial, "{:?}", last.invocation);
        assert!(
            trace.expansions[..trace.expansions.len() - 1]
                .iter()
                .all(|expansion| !expansion.partial)
        );
    }

    #[test]
    fn a_trace_that_stops_as_an_expansion_begins_lists_that_one_as_partial() {
        // An empty replacement is one event, so the trace stops as an
        // invocation in the source begins rather than inside one.
        let source = format!("#define E\n{}", "E\n".repeat(60_000));
        let trace = trace_of(&source);
        assert!(trace.truncated);
        let [.., complete, last] = &trace.expansions[..] else {
            panic!("{} expansions", trace.expansions.len());
        };
        assert!(!complete.partial);
        assert!(last.partial && last.steps.is_empty());
        assert_eq!(
            (last.invocation.as_str(), last.line),
            ("E", complete.line + 1)
        );
        let partial = trace.expansions.iter().filter(|e| e.partial).count();
        assert_eq!(partial, 1);
    }

    #[test]
    fn an_argument_is_cut_only_past_its_limit() {
        let fits = "a".repeat(160);
        let over = "a".repeat(161);
        let wide = "é".repeat(81);
        let source = format!(
            "#define F(x) 0\nint a = F({fits});\nint b = F({over});\nint c = F(u8\"{wide}\");\n"
        );
        let trace = trace_of(&source);
        let arguments: Vec<&str> = trace
            .expansions
            .iter()
            .map(|expansion| expansion.steps[0].arguments[0].argument.as_str())
            .collect();
        assert_eq!(
            arguments,
            [
                fits.clone(),
                format!("{fits} …"),
                format!("u8\"{} …", "é".repeat(78)),
            ]
        );
    }

    #[test]
    fn tracing_changes_nothing_but_the_trace() {
        let blow_up = "#define A 0,0,0,\n#define B A A A A A A A A A A\n#define C B B B B B B B B \
                       B B\n#define D C C C C C C C C C C\n#define E D D D D D D D D D D\nint a[] \
                       = {E 0};\n";
        let nested = format!(
            "#define F(x) x\nint x={}1{};\n",
            "F(".repeat(400),
            ")".repeat(400)
        );
        // A header included once the trace has stopped is still a header
        // name to phase 3.
        let late_include =
            format!("#include <stddef.h>\n{blow_up}#include <stdint.h>\nint32_t z;\n");
        let mut sources: Vec<(String, String)> = [
            ("blow-up", blow_up),
            ("nested", &nested),
            ("late include", &late_include),
            ("not found", "#include \"missing.h\"\nint x;\n"),
            ("error", "int main(void) { return 1 +; }\n"),
        ]
        .iter()
        .map(|(name, source)| ((*name).to_owned(), (*source).to_owned()))
        .collect();
        sources.extend(programs());
        for (name, source) in sources {
            let traced = compile(&source, &options(false));
            let untraced = compile(
                &source,
                &ExplorerOptions {
                    trace: false,
                    ..options(false)
                },
            );
            assert!(traced.preprocessing.is_some() && untraced.preprocessing.is_none());
            // Every output but the trace, field by field.
            assert_eq!(
                ExplorerOutput {
                    preprocessing: None,
                    ..traced
                },
                untraced,
                "{name}"
            );
        }
        // For wasm32-wasip1 too, whose module, binary, text and source-map
        // positions, is the same either way.
        let lesson = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("web/demos/macros.c"),
        )
        .unwrap();
        let for_wasm = |trace| {
            compile(
                &lesson,
                &ExplorerOptions {
                    target: "wasm32-wasip1".into(),
                    trace,
                    ..options(false)
                },
            )
        };
        let (traced, untraced) = (for_wasm(true), for_wasm(false));
        let wasm = traced.wasm.as_ref().expect("a module");
        assert!(!wasm.binary.is_empty() && wasm.positions.iter().any(|p| p.line.is_some()));
        assert!(traced.preprocessing.is_some());
        assert_eq!(
            ExplorerOutput {
                preprocessing: None,
                ..traced
            },
            untraced
        );
        let output = compile(&late_include, &options(false));
        assert!(output.preprocessing.unwrap().truncated);
        let tokens = output.pp_tokens.unwrap();
        let header_name = |word: &str| {
            tokens
                .iter()
                .any(|token| token.header_name && token.spelling.contains(word))
        };
        assert!(header_name("stddef") && header_name("stdint"));
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
    fn line_maps_place_macro_expansions_at_their_invocations() {
        let source = "#define SWAP(a, b) do { \\\n    int t = (a); \\\n    (a) = (b); \\\n    (b) \
                      = t; \\\n} while (0)\n#define GETTER(name, value) int get_##name(void) { \
                      return value; }\nGETTER(seven, 7)\n#define STATIC static\n\nSTATIC int \
                      main2(void) {\n    int x = 1, y = 2;\n    SWAP(x, y);\n    return x + \
                      get_seven();\n}\nint main(void) { return main2(); }\n";
        let output = compile(source, &ExplorerOptions::default());
        assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
        let ranges: Vec<_> = output
            .line_map
            .iter()
            .map(|mapping| {
                (
                    mapping.function.as_str(),
                    mapping.source.start,
                    mapping.source.end,
                )
            })
            .collect();
        // Neither starts in a `#define`: `get_seven` is all on its
        // invocation's line, and `main2` starts at `STATIC`.
        assert_eq!(
            ranges,
            [("get_seven", 7, 7), ("main2", 10, 14), ("main", 15, 15)]
        );
    }

    /// Stepping `SWAP` in the macros lesson shows the line that invokes it,
    /// not the lines of its `#define`.
    #[test]
    fn wasm_positions_place_macro_expansions_at_their_invocations() {
        let lesson = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("web/demos/macros.c"),
        )
        .unwrap();
        let swap = 1 + lesson
            .lines()
            .position(|line| line.trim() == "SWAP(low, high);")
            .unwrap() as u32;
        let output = compile(
            &lesson,
            &ExplorerOptions {
                target: "wasm32-wasip1".into(),
                ..options(false)
            },
        );
        let wasm = output.wasm.as_ref().expect("a module");
        let main = wasm
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let source = main.source.unwrap();
        assert_eq!((source.start, source.end), (16, 28));
        let lines: Vec<u32> = wasm
            .positions
            .iter()
            .filter(|position| (main.start..main.end).contains(&position.offset))
            .filter_map(|position| position.line)
            .collect();
        assert!(lines.contains(&swap), "{lines:?}");
        assert!(
            lines
                .iter()
                .all(|line| (source.start..=source.end).contains(line)),
            "{lines:?}"
        );
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
