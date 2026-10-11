//! The loop forest: headers, membership and nesting.

use super::*;
use crate::{
    ir::{
        Block,
        ControlFlowGraph,
        DominatorTree,
        Entity,
    },
    optimizer::loops::LoopForest,
};

/// Each loop of the only function in `text`, innermost first, as its header,
/// its blocks and its parent's header, all by number.
fn loops_of(text: &str) -> Vec<(usize, Vec<usize>, Option<usize>)> {
    let arena = Bump::new();
    let module = parse(&arena, text);
    assert_verifies(&module, "the input");
    let (_, function) = module.functions().next().unwrap();
    let body = function.body.as_ref().unwrap();
    let scratch = Bump::new();
    let cfg = ControlFlowGraph::compute(body, &scratch);
    let tree = DominatorTree::compute(body, &cfg, &scratch);
    let forest = LoopForest::compute(&cfg, &tree, &scratch);
    forest
        .loops()
        .map(|lp| {
            let blocks = (0..body.block_count())
                .filter(|&index| forest.contains(lp, Block::new(index)))
                .collect();
            let header = forest.header(lp).index();
            assert_eq!(forest.innermost(forest.header(lp)), Some(lp));
            let parent = forest
                .parent(lp)
                .map(|parent| forest.header(parent).index());
            (header, blocks, parent)
        })
        .collect()
}

#[test]
fn nested_loops_are_found_inner_first() {
    let text = "\
function @f(i1, i1) external {
block0(v0: i1, v1: i1):
    jump block1
block1:
    brif v0, block2, block5
block2:
    jump block3
block3:
    brif v1, block3, block4
block4:
    jump block1
block5:
    return
}
";
    assert_eq!(
        loops_of(text),
        [(3, vec![3], Some(1)), (1, vec![1, 2, 3, 4], None)]
    );
}

#[test]
fn back_edges_to_one_header_form_one_loop() {
    let text = "\
function @f(i1, i1) external {
block0(v0: i1, v1: i1):
    jump block1
block1:
    brif v0, block2, block4
block2:
    brif v1, block1, block3
block3:
    jump block1
block4:
    return
}
";
    assert_eq!(loops_of(text), [(1, vec![1, 2, 3], None)]);
}

#[test]
fn sibling_loops_are_separate_and_self_loops_count() {
    let text = "\
function @f(i1, i1) external {
block0(v0: i1, v1: i1):
    jump block1
block1:
    brif v0, block1, block2
block2:
    jump block3
block3:
    brif v1, block4, block5
block4:
    jump block3
block5:
    return
}
";
    assert_eq!(loops_of(text), [(3, vec![3, 4], None), (1, vec![1], None)]);
}

#[test]
fn irreducible_cycles_and_unreachable_loops_are_not_loops() {
    // block1 and block2 form a cycle entered at both; block3 loops on itself
    // but cannot be reached.
    let text = "\
function @f(i1, i1) external {
block0(v0: i1, v1: i1):
    brif v0, block1, block2
block1:
    brif v1, block2, block4
block2:
    jump block1
block3:
    jump block3
block4:
    return
}
";
    assert_eq!(loops_of(text), []);
}
