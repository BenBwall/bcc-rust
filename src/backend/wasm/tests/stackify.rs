//! The stackifier's structure, executed: every graph, reducible or not,
//! must visit the same blocks in the same order as a walk of the graph
//! itself that makes the same choices at each branch.

use crate::{
    backend::wasm::{
        stackify::{
            Emitter,
            Exit,
            Graph,
            Target,
            stackify,
        },
        u32_of,
    },
    util::bump::Bump,
};

/// What the emitter receives, recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Block,
    Loop,
    If(u32),
    Else,
    End,
    Body(u32),
    Edge(u32, u32, Target),
    Leave(u32),
    Select(u32, Vec<u32>),
    Dispatch(u32),
}

#[derive(Default)]
struct Recorder {
    ops: Vec<Op>,
}

impl Emitter for Recorder {
    fn block(&mut self) {
        self.ops.push(Op::Block);
    }

    fn r#loop(&mut self) {
        self.ops.push(Op::Loop);
    }

    fn r#if(&mut self, from: u32) {
        self.ops.push(Op::If(from));
    }

    fn r#else(&mut self) {
        self.ops.push(Op::Else);
    }

    fn end(&mut self) {
        self.ops.push(Op::End);
    }

    fn body(&mut self, block: u32) {
        self.ops.push(Op::Body(block));
    }

    fn edge(&mut self, from: u32, edge: u32, target: Target) {
        self.ops.push(Op::Edge(from, edge, target));
    }

    fn leave(&mut self, from: u32) {
        self.ops.push(Op::Leave(from));
    }

    fn select(&mut self, from: u32, depths: &[u32]) {
        self.ops.push(Op::Select(from, depths.to_vec()));
    }

    fn dispatch(&mut self, count: u32) {
        self.ops.push(Op::Dispatch(count));
    }
}

/// A graph in owned vectors.
struct TestGraph {
    exits: Vec<Exit>,
    edges: Vec<Vec<u32>>,
}

impl TestGraph {
    fn stackify(&self) -> (Vec<Op>, bool) {
        let mut starts = vec![0_u32];
        let mut targets = Vec::new();
        for edges in &self.edges {
            targets.extend_from_slice(edges);
            starts.push(u32_of(targets.len()));
        }
        let graph = Graph {
            exits:   &self.exits,
            starts:  &starts,
            targets: &targets,
        };
        let mut recorder = Recorder::default();
        let arena = Bump::new();
        let dispatched = stackify(&graph, &mut recorder, &arena);
        (recorder.ops, dispatched)
    }

    /// The blocks a walk visits, taking edge `choices[i] % count` at the
    /// `i`th branching visit, for at most `steps` visits.
    fn walk(&self, choices: &[u32], steps: usize) -> Vec<u32> {
        let mut visited = Vec::new();
        let mut block = 0;
        let mut choice = 0;
        while visited.len() < steps {
            visited.push(block);
            let edges = &self.edges[block as usize];
            if edges.is_empty() {
                break;
            }
            let pick = if edges.len() == 1 {
                0
            } else {
                choice += 1;
                choices[(choice - 1) % choices.len()] as usize % edges.len()
            };
            block = edges[pick];
        }
        visited
    }
}

/// Runs the structured program, making the walk's choices.
fn execute(ops: &[Op], graph: &TestGraph, choices: &[u32], steps: usize) -> Vec<u32> {
    // The matching `end` of every construct, and the `else` of an `if`.
    let mut ends = vec![usize::MAX; ops.len()];
    let mut elses = vec![usize::MAX; ops.len()];
    let mut open = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        match op {
            | Op::Block | Op::Loop | Op::If(_) => open.push(index),
            | Op::Else => elses[*open.last().expect("else inside if")] = index,
            | Op::End => ends[open.pop().expect("balanced end")] = index,
            | _ => {},
        }
    }
    assert!(open.is_empty(), "unbalanced structure: {ops:?}");
    let mut visited = Vec::new();
    let mut frames: Vec<usize> = Vec::new();
    let mut choice = 0;
    let mut label = 0_u32;
    let pick = |count: usize, choice: &mut usize| -> usize {
        if count == 1 {
            return 0;
        }
        *choice += 1;
        choices[(*choice - 1) % choices.len()] as usize % count
    };
    let mut pc = 0;
    let branch = |depth: u32, frames: &mut Vec<usize>, pc: &mut usize| {
        let at = frames.len() - 1 - depth as usize;
        let start = frames[at];
        if matches!(ops[start], Op::Loop) {
            frames.truncate(at + 1);
            *pc = start + 1;
        } else {
            frames.truncate(at);
            *pc = ends[start] + 1;
        }
    };
    while pc < ops.len() && visited.len() < steps {
        match &ops[pc] {
            | Op::Block | Op::Loop => {
                frames.push(pc);
                pc += 1;
            },
            | Op::If(from) => {
                let edges = graph.edges[*from as usize].len();
                assert_eq!(edges, 2);
                let taken = pick(2, &mut choice);
                frames.push(pc);
                pc = if taken == 0 { pc + 1 } else { elses[pc] + 1 };
            },
            | Op::Else => {
                let start = frames.pop().expect("an else's if is open");
                pc = ends[start] + 1;
            },
            | Op::End => {
                _ = frames.pop();
                pc += 1;
            },
            | Op::Body(block) => {
                visited.push(*block);
                pc += 1;
            },
            | Op::Edge(_, _, target) => match *target {
                | Target::Br(depth) => branch(depth, &mut frames, &mut pc),
                | Target::Inline => pc += 1,
                | Target::Dispatch { index, depth } => {
                    label = index;
                    branch(depth, &mut frames, &mut pc);
                },
            },
            | Op::Leave(_) => break,
            | Op::Select(from, depths) => {
                let taken = pick(graph.edges[*from as usize].len(), &mut choice);
                branch(depths[taken], &mut frames, &mut pc);
            },
            | Op::Dispatch(count) => {
                assert!(label < *count);
                branch(label, &mut frames, &mut pc);
            },
        }
    }
    visited
}

/// A small deterministic generator.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

fn random_graph(random: &mut Random, blocks: u32) -> TestGraph {
    let mut exits = Vec::new();
    let mut edges = Vec::new();
    for _ in 0..blocks {
        let (exit, count) = match random.next() % 8 {
            | 0 => (Exit::Leave, 0),
            | 1 | 2 => (Exit::Jump, 1),
            | 3..=5 => (Exit::Branch, 2),
            | _ => (Exit::Select, 2 + random.next() % 3),
        };
        exits.push(exit);
        edges.push((0..count).map(|_| random.next() % blocks).collect());
    }
    TestGraph { exits, edges }
}

#[test]
fn random_graphs_visit_the_same_blocks_as_their_walks() {
    let mut random = Random(0x5EED);
    let mut dispatched = 0;
    for case in 0..3000 {
        let blocks = 1 + random.next() % 12;
        let graph = random_graph(&mut random, blocks);
        let (ops, irreducible) = graph.stackify();
        dispatched += usize::from(irreducible);
        for run in 0..4 {
            let choices: Vec<u32> = (0..64).map(|_| random.next()).collect();
            let expected = graph.walk(&choices, 200);
            let actual = execute(&ops, &graph, &choices, 200);
            assert_eq!(
                actual, expected,
                "case {case} run {run}: exits {:?} edges {:?}\n{ops:#?}",
                graph.exits, graph.edges
            );
        }
    }
    // The generator makes both kinds of graph.
    assert!(dispatched > 100, "only {dispatched} irreducible graphs");
}

#[test]
fn irreducible_graph_uses_the_dispatch_loop() {
    // 0 branches into the middle of the cycle 1 <-> 2 at both of its
    // nodes; neither dominates the other.
    let graph = TestGraph {
        exits: vec![Exit::Branch, Exit::Branch, Exit::Branch, Exit::Leave],
        edges: vec![vec![1, 2], vec![2, 3], vec![1, 3], vec![]],
    };
    let (ops, irreducible) = graph.stackify();
    assert!(irreducible);
    assert!(ops.contains(&Op::Dispatch(4)), "{ops:?}");
    for seed in 0..50 {
        let mut random = Random(seed);
        let choices: Vec<u32> = (0..32).map(|_| random.next()).collect();
        assert_eq!(
            execute(&ops, &graph, &choices, 100),
            graph.walk(&choices, 100)
        );
    }
}

#[test]
fn while_loop_is_a_loop_with_an_inline_exit() {
    // while (i < n) i++; return i;
    let graph = TestGraph {
        exits: vec![Exit::Jump, Exit::Branch, Exit::Jump, Exit::Leave],
        edges: vec![vec![1], vec![2, 3], vec![1], vec![]],
    };
    let (ops, irreducible) = graph.stackify();
    assert!(!irreducible);
    assert_eq!(
        ops,
        vec![
            Op::Body(0),
            Op::Edge(0, 0, Target::Inline),
            Op::Loop,
            Op::Body(1),
            Op::If(1),
            Op::Edge(1, 0, Target::Inline),
            Op::Body(2),
            Op::Edge(2, 0, Target::Br(1)),
            Op::Else,
            Op::Edge(1, 1, Target::Inline),
            Op::Body(3),
            Op::Leave(3),
            Op::End,
            Op::End,
        ]
    );
}

#[test]
fn diamond_merge_follows_a_block() {
    // if (c) a; else b; join;
    let graph = TestGraph {
        exits: vec![Exit::Branch, Exit::Jump, Exit::Jump, Exit::Leave],
        edges: vec![vec![1, 2], vec![3], vec![3], vec![]],
    };
    let (ops, irreducible) = graph.stackify();
    assert!(!irreducible);
    assert_eq!(
        ops,
        vec![
            Op::Block,
            Op::Body(0),
            Op::If(0),
            Op::Edge(0, 0, Target::Inline),
            Op::Body(1),
            Op::Edge(1, 0, Target::Br(1)),
            Op::Else,
            Op::Edge(0, 1, Target::Inline),
            Op::Body(2),
            Op::Edge(2, 0, Target::Br(1)),
            Op::End,
            Op::End,
            Op::Body(3),
            Op::Leave(3),
        ]
    );
}

#[test]
fn deep_chains_need_no_native_recursion() {
    // 20000 blocks in a row, each branching to the next or to the end.
    let count = 20_000_u32;
    let mut exits = vec![Exit::Branch; count as usize];
    let mut edges: Vec<Vec<u32>> = (0..count).map(|block| vec![block + 1, count]).collect();
    exits.push(Exit::Leave);
    edges.push(vec![]);
    edges[count as usize - 1] = vec![count, count];
    let graph = TestGraph { exits, edges };
    let (ops, irreducible) = graph.stackify();
    assert!(!irreducible);
    assert_eq!(
        ops.iter().filter(|op| matches!(op, Op::Body(_))).count(),
        count as usize + 1
    );
}
