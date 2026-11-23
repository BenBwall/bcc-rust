use super::*;
use crate::demos::DEMOS;

#[test]
fn every_lesson_has_llvm_ir_generated_from_its_current_source() {
    for demo in &DEMOS {
        let stem = demo.name.trim_end_matches(".c");
        assert_eq!(
            precomputed_source(stem),
            Some(demo.source),
            "{stem}: run web/pipeline/generate.sh after changing a lesson"
        );
        for optimize in [false, true] {
            let llvm = precomputed_llvm(stem, optimize).expect("LLVM IR is bundled");
            assert!(
                llvm.contains("define "),
                "{stem} at -O{}",
                u8::from(optimize) * 2
            );
        }
    }
}

#[test]
fn links_token_and_syntax_lines_to_source_lines() {
    let tokens = Compilation {
        tokens: Some(
            "<input>:12:5: keyword `for`\n<input>:3:1: punctuator `}`\nstddef.h:3:1: keyword \
             `typedef`"
                .into(),
        ),
        ..Compilation::default()
    };
    let exact = links(Stage::Tokens, &tokens, "", Unfolded::default());
    assert!(exact.is_exact());
    assert_eq!(exact.outputs_for(12), [1]);
    assert_eq!(exact.sources_for(2), [3]);
    assert_eq!(exact.outputs_for(3), [2], "a header's token links nowhere");
    let tree = Compilation {
        syntax_tree: Some(
            "function-definition main @7:1\n    function variadic=false\n  body: compound @7:16"
                .into(),
        ),
        ..Compilation::default()
    };
    let exact = links(Stage::SyntaxTree, &tree, "", Unfolded::default());
    assert_eq!(exact.outputs_for(7), [1, 3]);
    assert_eq!(exact.sources_for(2), Vec::<usize>::new());
    let semantic = Compilation {
        semantic_types: Some(
            "scope 0 file
  function main type=\"int (void)\" linkage=external @1:5
  scope 1 function
    object total type=int duration=automatic size=4 align=4 @2:9
    object __func__ type=\"const char [5]\" duration=static size=5 align=1 @1:5
expression 0 type=int category=rvalue value=0 ice constant=arithmetic @2:17"
                .into(),
        ),
        ..Compilation::default()
    };
    let exact = links(Stage::SemanticTypes, &semantic, "", Unfolded::default());
    assert_eq!(exact.outputs_for(2), [4, 6]);
    // A function's `__func__` is placed at the function's name.
    assert_eq!(exact.outputs_for(1), [2, 5]);
    assert_eq!(
        exact.sources_for(3),
        Vec::<usize>::new(),
        "a scope line has no place of its own"
    );
}

#[cfg(feature = "compiler")]
#[test]
fn every_lesson_shows_its_declarations_after_semantic_analysis() {
    // A lesson that includes <stdio.h> stops before semantic analysis in the
    // browser, which has no C library headers for x86-64.
    for demo in DEMOS
        .iter()
        .filter(|demo| !crate::wasm::is_command(demo.source))
    {
        let compilation = crate::explorer::compile(demo.source, false);
        let semantic = compilation
            .output(Stage::SemanticTypes)
            .expect("semantic analysis ran");
        assert!(
            semantic
                .lines()
                .any(|line| line.trim_start().starts_with("function main ")),
            "{}: {semantic}",
            demo.name
        );
        assert!(
            semantic
                .lines()
                .any(|line| line.trim_start().starts_with("expression ") && line.contains(" @")),
            "{}: expressions carry their source locations",
            demo.name
        );
    }
}

#[test]
fn links_the_other_stages_per_function() {
    let source =
        "int bump(int *value) {\n    return *value;\n}\n\nint main(void) {\n    return 0;\n}\n";
    let compilation = Compilation {
        ir: Some("header\nfunction @bump\n  a\n}\nfunction @main\n}\n".into()),
        llvm: Some(
            "; x\ndefine i32 @bump(ptr %0) {\n  ret i32 0\n}\n\ndefine i32 @main() {\n  ret i32 \
             0\n}\n"
                .into(),
        ),
        functions: vec![
            FunctionLines {
                name: "bump".into(),
                source: (1, 3),
                ir: Some((2, 4)),
                ..FunctionLines::default()
            },
            FunctionLines {
                name: "main".into(),
                source: (5, 7),
                ir: Some((5, 6)),
                ..FunctionLines::default()
            },
        ],
        ..Compilation::default()
    };
    let ir = links(Stage::Ir, &compilation, source, Unfolded::default());
    assert!(!ir.is_exact() && !ir.is_empty());
    assert_eq!(ir.outputs_for(2), [2, 3, 4]);
    assert_eq!(ir.sources_for(6), [5, 6, 7]);
    assert_eq!(
        ir.outputs_for(4),
        Vec::<usize>::new(),
        "a blank line links nowhere"
    );
    let llvm = links(Stage::Llvm, &compilation, source, Unfolded::default());
    assert_eq!(llvm.outputs_for(1), [2, 3, 4]);
    assert_eq!(llvm.sources_for(7), [5, 6, 7]);
    // Before the live compilation arrives, LLVM IR finds the definitions in
    // the source.
    let precomputed = Compilation {
        llvm: compilation.llvm.clone(),
        ..Compilation::default()
    };
    assert_eq!(
        links(Stage::Llvm, &precomputed, source, Unfolded::default()).outputs_for(6),
        [6, 7, 8]
    );
    assert!(links(Stage::Asm, &compilation, source, Unfolded::default()).is_empty());
}

/// Where each line came from, given its source line: 0 for a line that is not
/// code.
fn origins(lines: &[usize]) -> Vec<LineOrigin> {
    lines
        .iter()
        .map(|&line| LineOrigin {
            line,
            call: 0,
            code: line != 0,
        })
        .collect()
}

/// Every back-end stage reads the same table the same way, whatever its
/// function's last line is: a closing brace or directive stands for the
/// function, and the machine code's last line is an instruction.
#[test]
fn back_end_stages_link_line_by_line() {
    let source = "int f(int x) {\n    return x * x;\n}\n";
    let asm = "\t.text\nf:\n\t.cfi_startproc\n\timull\t%edi, %edi\n\tmovl\t%edi, %eax\n\tretq\n\t.\
               size\tf, .-f\n";
    let code = |line, call| LineOrigin {
        line,
        call,
        code: true,
    };
    let none = LineOrigin::default();
    let compilation = Compilation {
        asm: Some(asm.to_owned()),
        asm_lines: vec![none, none, none, code(2, 0), code(0, 0), code(2, 0), none],
        machine_code: Some("<f>:\n  0  0f af ff\n  3  89 f8\n  5  c3\n".to_owned()),
        machine_code_lines: vec![none, code(2, 0), code(0, 0), code(2, 0)],
        functions: vec![FunctionLines {
            name: "f".into(),
            source: (1, 3),
            asm: Some((2, 7)),
            machine_code: Some((1, 4)),
            ..FunctionLines::default()
        }],
        ..Compilation::default()
    };
    let asm = links(Stage::Asm, &compilation, source, Unfolded::default());
    assert!(asm.is_exact() && asm.has_origins());
    assert_eq!(asm.outputs_for(2), [4, 6]);
    assert_eq!(asm.sources_for(2), [1, 2, 3], "the label stands for the function");
    assert_eq!(asm.sources_for(7), [1, 2, 3], "and so does the end directive");
    assert_eq!(asm.sources_for(4), [2]);
    assert_eq!(asm.sources_for(5), Vec::<usize>::new());
    assert_eq!(asm.unclaimed().collect::<Vec<_>>(), [5]);
    let machine = links(Stage::MachineCode, &compilation, source, Unfolded::default());
    assert_eq!(machine.sources_for(4), [2], "the last instruction keeps its line");
    assert_eq!(machine.sources_for(1), [1, 2, 3]);
    assert_eq!(machine.outputs_for(2), [2, 4]);
}

/// An inlined copy links to its own line and to the call it was made for.
#[test]
fn inlined_copies_link_to_their_line_and_their_call() {
    let source = "int s(int n) {\n    return n * n;\n}\nint f(int x) {\n    return s(x);\n}\n";
    let code = |line, call| LineOrigin {
        line,
        call,
        code: true,
    };
    let none = LineOrigin::default();
    let compilation = Compilation {
        ir: Some(
            "function @s(i32) -> i32 {\nblock0(v0: i32):\n    v1 = imul v0, v0\n    return \
             v1\n}\nfunction @f(i32) -> i32 {\nblock0(v0: i32):\n    v1 = imul v0, v0\n    return \
             v1\n}\n"
                .to_owned(),
        ),
        ir_lines: vec![none, none, code(2, 0), code(2, 0), none, none, none, code(2, 5), code(5, 0), none],
        functions: vec![
            FunctionLines {
                name: "s".into(),
                source: (1, 3),
                ir: Some((1, 5)),
                ..FunctionLines::default()
            },
            FunctionLines {
                name: "f".into(),
                source: (4, 6),
                ir: Some((6, 10)),
                ..FunctionLines::default()
            },
        ],
        ..Compilation::default()
    };
    let ir = links(Stage::Ir, &compilation, source, Unfolded::default());
    assert_eq!(ir.outputs_for(2), [3, 4, 8], "the line, in place and inlined");
    assert_eq!(ir.outputs_for(5), [8, 9], "the call, with the copy made for it");
    assert_eq!(ir.sources_for(8), [2], "the copy's own line");
    assert_eq!(ir.origin(8).call, 5);
    assert_eq!(ir.first_output_for(5), Some(9), "scroll to the call's own code");
}

/// `f` in bcc IR, opening with its target lines.
fn targeted() -> Compilation {
    let none = LineOrigin::default();
    let code = LineOrigin {
        line: 2,
        call: 0,
        code: true,
    };
    Compilation {
        ir: Some(
            "target triple = \"x86_64-unknown-linux-gnu\"\ntarget datalayout = \
             \"e-S128\"\n\nfunction @f(i32) -> i32 {\nblock0(v0: i32):\n    v1 = imul v0, v0\n    \
             return v1\n}\n"
                .into(),
        ),
        ir_lines: vec![none, none, none, none, none, code, code, none],
        functions: vec![FunctionLines {
            name: "f".into(),
            source: (1, 3),
            ir: Some((4, 8)),
            ..FunctionLines::default()
        }],
        ..Compilation::default()
    }
}

const TARGETED_SOURCE: &str = "int f(int x) {\n    return x * x;\n}\n";

/// The IR's target lines fold into one line that links nowhere, and every
/// link below them moves up with the text.
#[test]
fn folded_target_lines_keep_the_links_below_them() {
    let compilation = targeted();
    let (text, _) = displayed(Stage::Ir, &compilation, Unfolded::default()).unwrap();
    assert!(target::is_summary(text.lines().next().unwrap()), "{text}");
    assert_eq!(text.lines().nth(2), Some("function @f(i32) -> i32 {"));
    let folded = links(
        Stage::Ir,
        &compilation,
        TARGETED_SOURCE,
        Unfolded::default(),
    );
    assert_eq!(folded.outputs_for(2), [5, 6]);
    assert_eq!(
        folded.sources_for(3),
        [1, 2, 3],
        "the header stands for the function"
    );
    assert_eq!(
        folded.sources_for(1),
        Vec::<usize>::new(),
        "no source line made the target"
    );
    let shown = Unfolded {
        target: true,
        ..Unfolded::default()
    };
    let (text, _) = displayed(Stage::Ir, &compilation, shown).unwrap();
    assert_eq!(Some(text.as_str()), compilation.ir.as_deref());
    assert_eq!(
        links(Stage::Ir, &compilation, TARGETED_SOURCE, shown).outputs_for(2),
        [6, 7]
    );
}

/// A view that changes under the reader opens at the same code: a place read
/// in one view (a source line and its row below the first line in view) puts
/// that source line's output at the same row in the other.
#[test]
fn a_place_in_one_view_opens_the_same_code_in_another() {
    let compilation = targeted();
    let shown = Unfolded {
        target: true,
        ..Unfolded::default()
    };
    let full = links(Stage::Ir, &compilation, TARGETED_SOURCE, shown);
    let folded = links(
        Stage::Ir,
        &compilation,
        TARGETED_SOURCE,
        Unfolded::default(),
    );
    let function = compilation.function_around(2);
    assert_eq!(function, Some((1, 3)));
    // `v1 = imul` first in view: line 6 here, line 5 there.
    let place = full.place(6, 6).unwrap();
    assert_eq!(
        place,
        Place {
            source: 2,
            row:    0,
        }
    );
    assert_eq!(folded.top_for(place, function), 5);
    // A label first in view has no source line: the line below it places it,
    // one row down.
    let place = full.place(5, 5).unwrap();
    assert_eq!(
        place,
        Place {
            source: 2,
            row:    1,
        }
    );
    assert_eq!(folded.top_for(place, function), 4);
    // The line under the pointer keeps its row.
    let place = full.place(4, 7).unwrap();
    assert_eq!(
        place,
        Place {
            source: 2,
            row:    3,
        }
    );
    assert_eq!(folded.top_for(place, function), 2);
    // At the start, the other view opens at its start.
    assert_eq!(full.place(1, 1), Some(Place::default()));
    assert_eq!(folded.top_for(Place::default(), function), 1);
    // Nothing near links: no place.
    assert_eq!(Links::default().place(3, 3), None);
}

/// A source line with no output in the other view (a declaration, a line the
/// optimizer removed) opens at the nearest line of its function that has one.
#[test]
fn a_place_without_code_falls_to_the_next_line_of_its_function() {
    let source = "int f(int x) {\n    int y = x;\n    return x;\n}\n";
    let none = LineOrigin::default();
    let code = LineOrigin {
        line: 3,
        call: 0,
        code: true,
    };
    let compilation = Compilation {
        ir: Some(
            "function @f(i32) -> i32 {\nblock0(v0: i32):\n    v1 = copy v0\n    return v1\n}\n"
                .into(),
        ),
        ir_lines: vec![none, none, code, code, none],
        functions: vec![FunctionLines {
            name: "f".into(),
            source: (1, 4),
            ir: Some((1, 5)),
            ..FunctionLines::default()
        }],
        ..Compilation::default()
    };
    let ir = links(Stage::Ir, &compilation, source, Unfolded::default());
    assert_eq!(ir.outputs_for(2), Vec::<usize>::new());
    let place = Place {
        source: 2,
        row:    1,
    };
    assert_eq!(ir.top_for(place, compilation.function_around(2)), 2);
    // The run of linked lines to bring into view for line 3.
    assert_eq!(ir.run_for(3), Some((3, 4)));
    assert_eq!(ir.run_for(2), None);
}

/// Each lesson's recorded lines match its LLVM IR, name lines of its source,
/// and mark exactly the instructions as code.
#[test]
fn precomputed_llvm_lines_match_their_ir() {
    for lesson in &DEMOS {
        let stem = lesson.name.trim_end_matches(".c");
        let lines = lesson.source.lines().count();
        for optimize in [false, true] {
            let Some(llvm) = precomputed_llvm(stem, optimize) else {
                continue;
            };
            let origins = precomputed_llvm_lines(stem, optimize);
            assert_eq!(origins.len(), llvm.lines().count(), "{stem}");
            for (text, origin) in llvm.lines().zip(&origins) {
                let instruction = text.starts_with("  ") && !text.trim_start().starts_with(';');
                assert_eq!(origin.code, instruction, "{stem}: {text}");
                assert!(origin.line <= lines && origin.call <= lines, "{stem}: {text}");
            }
            assert!(origins.iter().any(|origin| origin.line != 0), "{stem}");
        }
    }
}

#[test]
fn links_ir_instructions_to_their_lines() {
    let source = "int square(int x) {\n    int y = x * x;\n    return y;\n}\n";
    let function = |ir| FunctionLines {
        name: "square".into(),
        source: (1, 4),
        ir: Some(ir),
        ..FunctionLines::default()
    };
    let compilation = Compilation {
        ir:
            Some(
                "target\n\nfunction @square(i32) -> i32 {\nblock0(v0: i32):\n    v1 = imul v0, \
                 v0\n    return v1\n}\n"
                    .into(),
            ),
        ir_lines: origins(&[0, 0, 0, 0, 2, 3, 0]),
        functions: vec![function((3, 7))],
        ..Compilation::default()
    };
    let ir = links(Stage::Ir, &compilation, source, Unfolded::default());
    assert!(ir.is_exact());
    assert_eq!(
        ir.outputs_for(2),
        [5],
        "the multiply, not the function's ends"
    );
    assert_eq!(
        ir.outputs_for(1),
        [3, 7],
        "the function's first line, its header and closing brace"
    );
    assert_eq!(
        ir.first_output_for(2),
        Some(5),
        "scroll to the instruction, not the header"
    );
    assert_eq!(ir.first_output_for(4), Some(3));
    assert_eq!(ir.outputs_for(3), [6]);
    assert_eq!(ir.sources_for(5), [2]);
    assert_eq!(
        ir.sources_for(3),
        [1, 2, 3, 4],
        "the header stands for the function"
    );
    assert_eq!(
        ir.sources_for(4),
        Vec::<usize>::new(),
        "a block label links nowhere"
    );
    // A body the optimizer rebuilt has no lines left: it links per function.
    let rebuilt = Compilation {
        ir_lines: origins(&[0; 7]),
        ..compilation
    };
    let rebuilt = links(Stage::Ir, &rebuilt, source, Unfolded::default());
    assert_eq!(rebuilt.sources_for(5), [1, 2, 3, 4]);
    assert!(
        !rebuilt.is_exact(),
        "the note says the stage links per function"
    );
}

#[test]
fn the_optimizer_comparison_names_what_changed() {
    let plain = Compilation {
        ir: Some(
            "function @bump(ptr) -> i32 {\nblock0(v0: ptr):\n    v1 = load.i32 v0\n    return \
             v1\n}\nfunction @main() -> i32 {\n    slot0 = stack_slot 4, align 4\nblock0:\n    v0 \
             = stack_addr slot0\n    v1 = call @bump(v0)\n    jump block1\nblock1:\n    return \
             v1\n}\n"
                .into(),
        ),
        functions: vec![FunctionLines {
            name: "bump".into(),
            ..FunctionLines::default()
        }],
        ..Compilation::default()
    };
    let optimized = Compilation {
        ir: Some(
            "function @bump(ptr) -> i32 {\nblock0(v0: ptr):\n    v1 = load.i32 v0\n    return \
             v1\n}\nfunction @main() -> i32 {\nblock0:\n    v0 = iconst.i32 2\n    return v0\n}\n"
                .into(),
        ),
        ..Compilation::default()
    };
    let changes = optimizer_changes(Stage::Ir, &plain, &optimized);
    let text = changes.join("\n");
    assert!(text.contains("Instructions: 6 at -O0, 4 at -O2."), "{text}");
    assert!(text.contains("Basic blocks: 3 at -O0, 2 at -O2."), "{text}");
    assert!(text.contains("Stack slots: 1 at -O0, 0 at -O2."), "{text}");
    assert!(text.contains("bump is no longer called"), "{text}");
    assert!(!text.contains("loads and stores"), "{text}");
    // A count that grows gets no reason: the reasons explain savings.
    let grown = optimizer_changes(Stage::Ir, &optimized, &plain).join("\n");
    assert!(
        grown.contains("Basic blocks: 2 at -O0, 3 at -O2.\n"),
        "{grown}"
    );
    let same = optimizer_changes(Stage::Ir, &optimized, &optimized);
    assert_eq!(same.len(), 1);
    assert!(same[0].contains("nothing to improve"));
    assert_eq!(
        optimizer_changes(Stage::Tokens, &plain, &optimized),
        Vec::<String>::new()
    );
}

#[test]
fn every_lesson_counts_each_basic_block_once() {
    // Each basic block ends in exactly one terminator, so counting the
    // terminators checks the count of block labels.
    fn terminators(text: &str, indent: &str, names: &[&str]) -> usize {
        text.lines()
            .filter_map(|line| line.strip_prefix(indent))
            .filter(|instruction| {
                let operation = instruction
                    .split_once("= ")
                    .map_or(*instruction, |(_, operation)| operation);
                operation
                    .split([' ', '.'])
                    .next()
                    .is_some_and(|name| names.contains(&name))
            })
            .count()
    }
    let ir = [
        "jump",
        "brif",
        "switch",
        "return",
        "indirect_br",
        "unreachable",
    ];
    let llvm = ["br", "switch", "ret", "indirectbr", "unreachable", "resume"];
    for demo in &DEMOS {
        let stem = demo.name.trim_end_matches(".c");
        for optimize in [false, true] {
            let compilation = crate::explorer::compile(demo.source, optimize);
            let listing = compilation.output(Stage::Ir).expect("bcc IR");
            assert_eq!(
                Census::of(Stage::Ir, listing).blocks,
                terminators(listing, "    ", &ir),
                "{stem} bcc IR at -O{}",
                u8::from(optimize) * 2
            );
            let listing = precomputed_llvm(stem, optimize).expect("LLVM IR is bundled");
            assert_eq!(
                Census::of(Stage::Llvm, listing).blocks,
                terminators(listing, "  ", &llvm),
                "{stem} LLVM IR at -O{}",
                u8::from(optimize) * 2
            );
        }
    }
    // A block that others branch to carries a comment after its label.
    let callbacks = Compilation {
        llvm: precomputed_llvm("callbacks", false).map(Into::into),
        ..Compilation::default()
    };
    assert_eq!(
        example(Stage::Llvm, &callbacks, "").expect("an example"),
        "Here the LLVM IR is 27 instructions in 8 basic blocks, with 1 stack slot and 3 loads or \
         stores."
    );
}

#[test]
fn the_page_refuses_programs_larger_than_the_compiler_takes() {
    assert_eq!(too_large(&"x".repeat(MAX_COMPILE_BYTES)), None);
    let refusal = too_large(&"x".repeat(MAX_COMPILE_BYTES + 1)).expect("one byte too many");
    assert!(refusal.starts_with(TOO_LARGE), "{refusal}");
    assert!(
        refusal.contains("65 KiB") && refusal.contains("at most 64 KiB"),
        "{refusal}"
    );
}

#[test]
#[cfg(feature = "compiler")]
fn the_page_limit_is_the_in_browser_compilers() {
    assert_eq!(MAX_COMPILE_BYTES, bcc_rust::explorer::WASM_MAX_SOURCE_BYTES);
}

#[test]
fn a_trap_is_reported_by_its_cause() {
    use crate::engine::{
        INTERNAL_ERROR,
        OUT_OF_MEMORY,
        TOO_DEEP,
        trap_reason,
    };
    // A panic's message comes first; the trap after it says nothing more.
    assert_eq!(
        trap_reason(
            "Uncaught RuntimeError: unreachable",
            Some("index out of bounds")
        ),
        format!("{INTERNAL_ERROR}index out of bounds")
    );
    // Rust reaches `unreachable` without a panic when an allocation fails.
    assert_eq!(
        trap_reason("Uncaught RuntimeError: unreachable", None),
        OUT_OF_MEMORY
    );
    assert_eq!(
        trap_reason(
            "Uncaught RangeError: Maximum call stack size exceeded",
            None
        ),
        TOO_DEEP
    );
    assert_eq!(trap_reason("Something else", None), "Something else");
}

#[test]
fn a_functions_ends_stand_for_its_lines_only_without_lines_of_their_own() {
    // `square` (source 1-4) keeps an instruction for line 2; `rebuilt`
    // (source 6-9) has only its header, a body without lines, and its brace.
    let links = Links::exact(
        vec![
            None,
            Some((1, 4)),
            Some((2, 2)),
            Some((1, 4)),
            Some((6, 9)),
            Some((6, 9)),
            Some((6, 9)),
        ],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(links.outputs_for(2), [2]);
    assert_eq!(
        links.outputs_for(3),
        Vec::<usize>::new(),
        "a line with nothing of its own is not its function"
    );
    assert_eq!(links.outputs_for(4), [1, 3]);
    assert_eq!(links.outputs_for(7), [4, 5, 6]);
}

#[test]
fn the_stages_read_as_one_path_from_source_to_machine_code() {
    let mut stage = Stage::Lines;
    let mut visited = vec![stage];
    while let Some(next) = stage.next() {
        stage = next;
        visited.push(stage);
    }
    assert_eq!(visited, Stage::ALL);
    for stage in Stage::ALL {
        let handoff = stage.handoff();
        assert!(
            handoff.ends_with('.') && handoff.len() <= 150,
            "{}: {handoff}",
            stage.label()
        );
    }
}

#[test]
fn every_stage_gives_an_example_from_the_program_on_screen() {
    for demo in &DEMOS {
        for optimize in [false, true] {
            let compilation = crate::explorer::compile(demo.source, optimize);
            for stage in Stage::ALL {
                if compilation.output(stage).is_none() {
                    continue;
                }
                let example = example(stage, &compilation, demo.source)
                    .unwrap_or_else(|| panic!("{} {stage:?}: no example", demo.name));
                assert!(
                    example.ends_with('.') && !example.contains("``"),
                    "{} {stage:?}: {example}",
                    demo.name
                );
            }
        }
    }
    let path = crate::demos::find("path").expect("the path lesson");
    let compilation = crate::explorer::compile(path.source, false);
    let example = |stage| example(stage, &compilation, path.source).expect("an example");
    assert_eq!(
        example(Stage::Lines),
        "No line ends in a backslash, so each of the 10 physical lines stays a logical line of \
         its own."
    );
    assert_eq!(
        example(Stage::PpTokens),
        "Line 3, `return n * n;`, is cut into 5 preprocessing tokens: `return`, `n`, `*`, `n` and \
         `;`; `return` is still just an identifier."
    );
    assert_eq!(
        example(Stage::Preprocessor),
        "This program has no directives and no macros, so phase 4 passes its tokens on unchanged."
    );
    assert_eq!(
        example(Stage::Tokens),
        "`int` on line 2 is now a keyword, and `3` on line 7 an integer constant of type int: \
         this file makes 38 tokens in all."
    );
    assert_eq!(
        example(Stage::SyntaxTree),
        "On line 3, `*` is a `binary *` node with its left operand, `n`, and its right operand, \
         `n`, as its children."
    );
    assert_eq!(
        example(Stage::IrAbi),
        "Nothing here needs rewriting: every argument and result fits a register, so the \
         ABI-lowered IR is the same as the IR."
    );
    let constants = crate::demos::find("constants").expect("the constants lesson");
    let compilation = crate::explorer::compile(constants.source, false);
    let preprocessor =
        crate::pipeline::example(Stage::Preprocessor, &compilation, constants.source)
            .expect("an example");
    assert!(
        preprocessor.starts_with("#include pasted in <stdio.h> (")
            && preprocessor.contains("#define made the macros `LIMIT` and `LABEL`")
            && preprocessor.contains("`LIMIT` becoming `4`"),
        "{preprocessor}"
    );
}

/// A call through a register is a call through a pointer to a function, not
/// a call to a function named `*%r13`.
#[test]
fn the_assembly_example_counts_calls_through_pointers() {
    let callbacks = crate::demos::find("callbacks").expect("the callbacks lesson");
    let compilation = crate::explorer::compile(callbacks.source, true);
    assert_eq!(
        example(Stage::Asm, &compilation, callbacks.source).as_deref(),
        Some(
            "Here bcc selected 76 x86-64 instructions, with calls to `printf` and 3 calls through \
             a function pointer."
        )
    );
}
