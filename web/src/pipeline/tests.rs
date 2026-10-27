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
    let exact = links(Stage::Tokens, &tokens, "", false);
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
    let exact = links(Stage::SyntaxTree, &tree, "", false);
    assert_eq!(exact.outputs_for(7), [1, 3]);
    assert_eq!(exact.sources_for(2), Vec::<usize>::new());
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
    let ir = links(Stage::Ir, &compilation, source, false);
    assert!(!ir.is_exact() && !ir.is_empty());
    assert_eq!(ir.outputs_for(2), [2, 3, 4]);
    assert_eq!(ir.sources_for(6), [5, 6, 7]);
    assert_eq!(
        ir.outputs_for(4),
        Vec::<usize>::new(),
        "a blank line links nowhere"
    );
    let llvm = links(Stage::Llvm, &compilation, source, false);
    assert_eq!(llvm.outputs_for(1), [2, 3, 4]);
    assert_eq!(llvm.sources_for(7), [5, 6, 7]);
    // Before the live compilation arrives, LLVM IR finds the definitions in
    // the source.
    let precomputed = Compilation {
        llvm: compilation.llvm.clone(),
        ..Compilation::default()
    };
    assert_eq!(
        links(Stage::Llvm, &precomputed, source, false).outputs_for(6),
        [6, 7, 8]
    );
    assert!(links(Stage::Asm, &compilation, source, false).is_empty());
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
