//! Control-flow edges, reverse post-order and dominator trees.

use std::fmt::Write as _;

use super::*;

/// The edges and dominator tree of the only function in `text`, checked by
/// `check` with block numbers.
fn with_tree(text: &str, check: impl FnOnce(&Body<'_>, &ControlFlowGraph<'_>, &DominatorTree<'_>)) {
    let arena = Bump::new();
    let module = parse(&arena, text);
    let (_, function) = module.functions().next().unwrap();
    let body = function.body.as_ref().unwrap();
    let scratch = Bump::new();
    let cfg = ControlFlowGraph::compute(body, &scratch);
    let tree = DominatorTree::compute(body, &cfg, &scratch);
    check(body, &cfg, &tree);
}

fn blocks(numbers: &[usize]) -> Vec<Block> {
    numbers.iter().map(|&number| Block::new(number)).collect()
}

fn idoms(body: &Body<'_>, tree: &DominatorTree<'_>) -> Vec<Option<u32>> {
    body.blocks()
        .map(|block| tree.idom(block).map(Block::as_u32))
        .collect()
}

#[test]
fn diamond() {
    with_tree(
        "\
function @f(i1) external {
block0(v0: i1):
    brif v0, block1, block2
block1:
    jump block3
block2:
    jump block3
block3:
    return
}
",
        |body, cfg, tree| {
            assert_eq!(cfg.block_count(), 4);
            assert_eq!(cfg.successors(Block::new(0)), blocks(&[1, 2]));
            assert_eq!(cfg.predecessors(Block::new(3)), blocks(&[1, 2]));
            assert_eq!(cfg.predecessors(Block::new(0)), []);
            assert_eq!(tree.reverse_post_order(), blocks(&[0, 2, 1, 3]));
            assert_eq!(idoms(body, tree), [None, Some(0), Some(0), Some(0)]);
            assert!(tree.dominates(Block::new(0), Block::new(3)));
            assert!(tree.dominates(Block::new(3), Block::new(3)));
            assert!(!tree.dominates(Block::new(1), Block::new(3)));
            assert!(!tree.dominates(Block::new(3), Block::new(0)));
        },
    );
}

#[test]
fn loop_with_exit() {
    with_tree(
        "\
function @f(i1) external {
block0(v0: i1):
    jump block1
block1:
    brif v0, block2, block3
block2:
    jump block1
block3:
    return
}
",
        |body, cfg, tree| {
            assert_eq!(cfg.predecessors(Block::new(1)), blocks(&[0, 2]));
            assert_eq!(idoms(body, tree), [None, Some(0), Some(1), Some(1)]);
            assert!(tree.dominates(Block::new(1), Block::new(2)));
            assert!(!tree.dominates(Block::new(2), Block::new(1)));
        },
    );
}

#[test]
fn irreducible_loop_is_dominated_by_its_entry_split() {
    // Block 1 and block 2 form a loop entered at both blocks, so neither
    // dominates the other.
    with_tree(
        "\
function @f(i1) external {
block0(v0: i1):
    brif v0, block1, block2
block1:
    brif v0, block2, block3
block2:
    jump block1
block3:
    return
}
",
        |body, _, tree| {
            assert_eq!(idoms(body, tree), [None, Some(0), Some(0), Some(1)]);
            assert!(!tree.dominates(Block::new(1), Block::new(2)));
            assert!(!tree.dominates(Block::new(2), Block::new(1)));
        },
    );
}

#[test]
fn unreachable_blocks_are_outside_the_tree() {
    with_tree(
        "\
function @f() external {
block0:
    return
block1:
    jump block2
block2:
    switch v0, block1, [1: block2, 2: block1]
block3:
    v0 = iconst.i32 0
    jump block2
}
",
        |body, cfg, tree| {
            assert_eq!(
                cfg.successors(Block::new(2)),
                blocks(&[1, 2]),
                "repeated targets are listed once"
            );
            assert_eq!(tree.reverse_post_order(), blocks(&[0]));
            assert_eq!(idoms(body, tree), [None; 4]);
            assert!(!tree.is_reachable(Block::new(3)));
            assert!(!tree.dominates(Block::new(3), Block::new(2)));
        },
    );
}

#[test]
fn deep_chains_need_no_native_recursion() {
    let mut text = String::from("function @f() external {\nblock0:\n    jump block1\n");
    let depth = 50_000;
    for block in 1..depth {
        writeln!(text, "block{block}:\n    jump block{}", block + 1).unwrap();
    }
    writeln!(text, "block{depth}:\n    return\n}}").unwrap();
    with_tree(&text, |body, _, tree| {
        assert_eq!(tree.reverse_post_order().len(), body.block_count());
        assert!(tree.dominates(Block::new(1), Block::new(depth)));
        assert_eq!(tree.idom(Block::new(depth)), Some(Block::new(depth - 1)));
    });
}
