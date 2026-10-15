//! Block addresses and indirect branches: label tables in globals, label
//! differences added back to a label, and the traps for every other target.

use super::*;

/// `select(k)` jumps through a table of label addresses; `offset(k)` adds a
/// label difference from a second table to the address of label 0.
const DISPATCH: &str = "\
global @table internal constant size 24, align 8 = bytes \
                        \"000000000000000000000000000000000000000000000000\" relocs [0: \
                        blockaddress(@select, label0), 8: blockaddress(@select, label1), 16: \
                        blockaddress(@select, label2)]
global @offsets internal constant size 12, align 4 = bytes \"000000000000000000000000\" relocs [0: \
                        labeldiff.i32(@offset, label0, label0), 4: labeldiff.i32(@offset, label1, \
                        label0), 8: labeldiff.i32(@offset, label2, label0)]

function @select(i64) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i64):
    v1 = global_addr @table
    v2 = iconst.i64 8
    v3 = imul.i64 v0, v2
    v4 = ptr_add v1, v3
    v5 = load.ptr v4, align 8
    v6 = iconst.i32 100
    indirect_br v5, [block1(v6), block2, block3]
block1(v7: i32):
    return v7
block2:
    v8 = iconst.i32 200
    return v8
block3:
    v9 = block_addr label0
    v10 = iconst.i32 300
    indirect_br v9, [block1(v10), block2, block3]
}

function @offset(i64) -> i32 external {
    label0 = block_label block1
    label1 = block_label block2
    label2 = block_label block3
block0(v0: i64):
    v1 = global_addr @offsets
    v2 = iconst.i64 4
    v3 = imul.i64 v0, v2
    v4 = ptr_add v1, v3
    v5 = load.i32 v4, align 4
    v6 = sext.i64 v5
    v7 = block_addr label0
    v8 = ptr_add inbounds v7, v6
    indirect_br v8, [block1, block2, block3]
block1:
    v9 = iconst.i32 10
    return v9
block2:
    v10 = iconst.i32 20
    return v10
block3:
    v11 = iconst.i32 30
    return v11
}
";

#[test]
fn label_tables_select_blocks_with_their_arguments() {
    assert_eq!(returned_with(DISPATCH, "select", &[int(0)]), int(100));
    assert_eq!(returned_with(DISPATCH, "select", &[int(1)]), int(200));
    // Label 2 branches again, to label 0 with another argument.
    assert_eq!(returned_with(DISPATCH, "select", &[int(2)]), int(300));
}

#[test]
fn label_differences_add_back_to_labels() {
    for (index, expected) in [(0, 10), (1, 20), (2, 30)] {
        assert_eq!(
            returned_with(DISPATCH, "offset", &[int(index)]),
            int(expected)
        );
    }
}

/// The trap of running `@f` whose entry block computes `v1` with `setup`
/// and branches to it.
fn indirect_trap(setup: &str) -> UbKind {
    let text = format!(
        "\
function @other() external {{
    label0 = block_label block1
block0:
    jump block1
block1:
    return
}}

function @f() -> i32 external {{
    label0 = block_label block1
    label1 = block_label block2
block0:
    v0 = iconst.i32 0
{setup}
    indirect_br v1, [block1]
block1:
    return v0
block2:
    return v0
}}
"
    );
    undefined(&text, "f")
}

#[test]
fn indirect_branches_to_other_addresses_are_undefined() {
    // Null, a label that is not listed, a function and past the last label.
    for setup in [
        "    v1 = null",
        "    v1 = block_addr label1",
        "    v1 = func_addr @other",
        "    v2 = block_addr label1\n    v3 = iconst.i64 1\n    v1 = ptr_add v2, v3",
    ] {
        assert_eq!(indirect_trap(setup), UbKind::IndirectBranch, "{setup}");
    }
    assert_eq!(indirect_trap("    v1 = poison.ptr"), UbKind::BranchOnPoison);
}

#[test]
fn labels_of_another_function_are_not_targets() {
    let text = "\
global @foreign internal constant size 8, align 8 = bytes \"0000000000000000\" relocs [0: \
                blockaddress(@other, label0)]

function @other() external {
    label0 = block_label block1
block0:
    jump block1
block1:
    return
}

function @f() -> i32 external {
    label0 = block_label block1
block0:
    v0 = global_addr @foreign
    v1 = load.ptr v0, align 8
    indirect_br v1, [block1]
block1:
    v2 = iconst.i32 1
    return v2
}
";
    assert_eq!(undefined(text, "f"), UbKind::IndirectBranch);
}

#[test]
fn label_addresses_hold_no_data_and_compare_by_address() {
    let text = "\
function @read() -> i8 external {
    label0 = block_label block1
block0:
    v0 = block_addr label0
    v1 = load.i8 v0, align 1
    return v1
block1:
    v2 = iconst.i8 0
    return v2
}

function @same() -> i1 external {
    label0 = block_label block1
block0:
    v0 = block_addr label0
    v1 = block_addr label0
    v2 = icmp.ptr eq v0, v1
    return v2
block1:
    v3 = iconst.i1 0
    return v3
}
";
    assert_eq!(undefined(text, "read"), UbKind::NotData);
    assert_eq!(returned(text, "same"), int(1));
}
