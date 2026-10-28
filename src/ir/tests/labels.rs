//! Labels, block addresses, indirect branches and label relocations: the
//! textual form round-trips, the builder makes the same body, and the
//! verifier keeps LLVM's `blockaddress`/`indirectbr` rules.

use super::*;

/// A dispatch through a static table of label addresses and a table of
/// label differences, with an indirect branch passing block arguments.
pub(crate) const DISPATCH: &str =
    "\
global @table internal constant size 16, align 8 = bytes \"00000000000000000000000000000000\" \
     relocs [0: blockaddress(@dispatch, label0), 8: blockaddress(@dispatch, label1)]
global @offsets internal constant size 8, align 4 = bytes \"0000000000000000\" relocs [0: \
     labeldiff.i32(@dispatch, label0, label0), 4: labeldiff.i32(@dispatch, label1, label0) + 0]

function @dispatch(i64) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
block0(v0: i64):
    v1 = global_addr @table
    v2 = iconst.i64 8
    v3 = imul.i64 v0, v2
    v4 = ptr_add v1, v3
    v5 = load.ptr v4, align 8
    v6 = iconst.i32 7
    indirect_br v5, [block1(v6), block2]
block1(v7: i32):
    return v7
block2:
    v8 = block_addr label0
    v9 = iconst.i32 9
    indirect_br v8, [block1(v9), block2]
}
";

#[test]
fn labels_and_indirect_branches_round_trip() {
    // The `+ 0` addend is not printed back.
    let printed = DISPATCH.replace(" + 0]", "]");
    assert_round_trip(&printed);
    assert!(
        verify_text(&printed).is_empty(),
        "{:?}",
        verify_text(&printed)
    );
}

#[test]
fn parsed_labels_name_their_blocks() {
    let arena = Bump::new();
    let module = parse(&arena, DISPATCH);
    let Some(Symbol::Function(func)) = module.symbol("dispatch") else {
        panic!("@dispatch is a function");
    };
    let body = module.function(func).body.as_ref().unwrap();
    let labels: Vec<_> = body.labels().map(|(_, block)| block.index()).collect();
    assert_eq!(labels, [1, 2]);
    assert!(body.has_indirect_control());
    let Some(Symbol::Global(offsets)) = module.symbol("offsets") else {
        panic!("@offsets is a global");
    };
    let Some(GlobalInit::Bytes { relocations, .. }) = module.global(offsets).init else {
        panic!("@offsets has bytes");
    };
    assert_eq!(relocations[1].size(8), 4);
    assert_eq!(
        relocations[1].target,
        RelocationTarget::LabelDifference {
            func,
            label: BlockLabel::new(1),
            base: BlockLabel::new(0),
            width: Type::I32,
        }
    );
}

#[test]
fn the_builder_makes_the_parsed_body() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let signature = module.intern_signature(&[Type::Ptr], Some(Type::I32), false);
    let func = module.declare_function("f", signature, Linkage::External);
    let mut builder = FunctionBuilder::new(&mut module, func);
    let entry = builder.create_entry_block();
    let first = builder.create_block();
    let second = builder.create_block();
    let label = builder.create_label(first);
    _ = builder.create_label(second);
    builder.switch_to_block(entry);
    let addr = builder.body().block_params(entry)[0];
    builder.indirect_br(addr, &[(first, &[]), (second, &[])]);
    builder.switch_to_block(first);
    let own = builder.block_addr(label);
    let seven = builder.iconst(Type::I32, 7);
    builder.indirect_br(own, &[(second, &[])]);
    builder.switch_to_block(second);
    let _ = seven;
    builder.ret(Some(seven));
    builder.finish();
    let text = module.to_string();
    assert_eq!(
        text,
        "\
function @f(ptr) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
block0(v0: ptr):
    indirect_br v0, [block1, block2]
block1:
    v1 = block_addr label0
    v2 = iconst.i32 7
    indirect_br v1, [block2]
block2:
    return v2
}
"
    );
    // block1 does not dominate block2, which block0 also reaches.
    assert_eq!(
        verify_text(&text),
        ["@f, inst4: the use of v2 is not dominated by its definition"]
    );
}

#[test]
fn label_and_indirect_branch_errors() {
    let reject = |text: &str, expected: &[&str]| {
        assert_eq!(verify_text(text), expected, "for\n{text}");
    };
    reject(
        "function @f() external {\n    label0 = block_label block0\nblock0:\n    return\n}\n",
        &["@f, block0: a label names the entry block"],
    );
    reject(
        "function @f() external {\n    label0 = block_label block1\n    label1 = block_label \
         block1\nblock0:\n    jump block1\nblock1:\n    return\n}\n",
        &["@f, block1: two labels name the block"],
    );
    reject(
        "function @f(ptr) external {\nblock0(v0: ptr):\n    indirect_br v0, [block1]\nblock1:\n    \
         return\n}\n",
        &["@f, inst0: indirect_br lists block1, which no label names"],
    );
    reject(
        "function @f(ptr) external {\n    label0 = block_label block1\nblock0(v0: ptr):\n    \
         indirect_br v0, [block1, block1]\nblock1:\n    return\n}\n",
        &["@f, inst0: indirect_br lists block1 twice"],
    );
    reject(
        "function @f(i64) external {\n    label0 = block_label block1\nblock0(v0: i64):\n    \
         indirect_br v0, [block1]\nblock1:\n    return\n}\n",
        &["@f, inst0: operand 0 is i64, expected ptr"],
    );
    reject(
        "function @f(ptr) external {\n    label0 = block_label block1\nblock0(v0: ptr):\n    \
         indirect_br v0, [block1]\nblock1(v1: i32):\n    return\n}\n",
        &["@f, inst0: the edge to block1 passes 0 arguments for 1 parameters"],
    );
}

#[test]
fn label_relocation_errors() {
    let function = "function @f() external {\n    label0 = block_label block1\nblock0:\n    jump \
                    block1\nblock1:\n    return\n}\n";
    let reject = |global: &str, expected: &[&str]| {
        let text = format!("{global}\n{function}");
        assert_eq!(verify_text(&text), expected, "for\n{text}");
    };
    reject(
        "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: \
         blockaddress(@f, label1)]",
        &["@g: label 1 does not exist"],
    );
    reject(
        "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: \
         labeldiff.i32(@f, label0, label2)]",
        &["@g: label 2 does not exist"],
    );
    reject(
        "global @g internal size 4, align 4 = bytes \"00000000\" relocs [0: labeldiff.i64(@f, \
         label0, label0)]",
        &["@g: the relocation at offset 0 does not fit in the global"],
    );
    reject(
        "global @g internal size 4, align 4 = bytes \"00000000\" relocs [0: labeldiff.f32(@f, \
         label0, label0)]",
        &["@g: a label difference cannot have type f32"],
    );
    // A declared function has no labels.
    let text = "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: \
                blockaddress(@h, label0)]\nfunction @h() external\n";
    assert_eq!(verify_text(text), ["@g: label 0 does not exist"]);
}

#[test]
fn label_parse_errors() {
    let arena = Bump::new();
    for (text, expected) in [
        (
            "function @f() external {\nblock0:\n    v0 = block_addr label0\n    return\n}\n",
            "3:21: `label0` is never defined",
        ),
        (
            "function @f() external {\n    label0 = block_label block7\nblock0:\n    return\n}\n",
            "2:26: `block7` is never defined",
        ),
        (
            "global @g internal size 8, align 8 = bytes \"0000000000000000\" relocs [0: \
             blockaddress(@g, label0)]\n",
            "1:87: `@g` is not a declared function",
        ),
    ] {
        let error = parse_module(&arena, text)
            .err()
            .map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some(expected), "for\n{text}");
    }
}
