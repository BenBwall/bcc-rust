//! Debug locations through the passes: they stay with their instructions,
//! follow an inlined copy to its call, and leave an instruction moved to
//! another block; the text without them is what the same input without them
//! gives, so no pass decides anything from one.

use super::*;
use crate::ir::Entity;

const TABLES: &str = "\
debug file file0 @a.c
debug unit file0 language c11 level 2
debug scope scope0 @twice name @twice file file0 line 1 scope_line 2
debug scope scope1 @main name @main file file0 line 10 scope_line 11
debug scope scope2 @loop name @loop file file0 line 20 scope_line 21
debug loc loc0 file0 line 3 column 3 scope scope0
debug loc loc1 file0 line 4 column 3 scope scope0
debug loc loc2 file0 line 12 column 3 scope scope1
debug loc loc3 file0 line 13 column 3 scope scope1
debug loc loc4 file0 line 22 column 3 scope scope2
debug loc loc5 file0 line 23 column 3 scope scope2
debug loc loc6 file0 line 24 column 3 scope scope2
";

const FUNCTIONS: &str = "\
function @twice(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 nsw v0, v0 dbg loc0
    return v1 dbg loc1
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @twice(v0) dbg loc2
    v2 = iadd.i32 v1, v0 dbg loc3
    return v2 dbg loc3
}

function @loop(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0 dbg loc4
    jump block1(v2, v2) dbg loc4
block1(v3: i32, v4: i32):
    v5 = imul.i32 nsw v0, v1 dbg loc5
    v6 = iadd.i32 v4, v5 dbg loc5
    v7 = iadd.i32 v3, v1 dbg loc5
    v8 = icmp.i32 slt v7, v0 dbg loc5
    brif v8, block1(v7, v6), block2 dbg loc5
block2:
    return v4 dbg loc6
}
";

fn located() -> String {
    format!("{TABLES}\n{FUNCTIONS}")
}

fn optimized<'ir>(arena: &'ir Bump, text: &str, options: &OptimizerOptions<'_>) -> Module<'ir> {
    let scratch = Bump::new();
    let mut module = parse(arena, text);
    assert_verifies(&module, "the input");
    _ = optimize(&mut module, options, &scratch);
    assert_verifies(&module, "the output");
    module
}

fn body_named<'a, 'ir>(module: &'a Module<'ir>, name: &str) -> &'a crate::ir::Body<'ir> {
    let (_, function) = module
        .functions()
        .find(|(_, function)| function.name == name)
        .unwrap();
    function.body.as_ref().unwrap()
}

#[test]
fn an_inlined_copy_keeps_its_line_and_names_the_call() {
    let arena = Bump::new();
    let module = optimized(&arena, &located(), &only(&[Pass::Inline]));
    let debug = module.debug_info();
    let body = body_named(&module, "main");
    let inlined: Vec<_> = body
        .blocks()
        .flat_map(|block| body.block_insts(block).iter().copied())
        .filter_map(|inst| body.inst_location(inst))
        .filter_map(|location| debug.location_data(location))
        .filter(|location| location.inlined_at.is_some())
        .collect();
    assert!(
        inlined.iter().any(|location| location.line == 3),
        "{inlined:?}"
    );
    assert!(
        inlined.iter().any(|location| location.line == 4),
        "{inlined:?}"
    );
    for location in inlined {
        let call = debug.location_data(location.inlined_at.unwrap()).unwrap();
        assert_eq!(call.line, 12);
        assert_eq!(debug.scope(location.scope).unwrap().name, "twice");
    }
}

#[test]
fn inlining_twice_chains_the_calls() {
    let text = "\
debug file file0 @a.c
debug unit file0 language c11 level 2
debug scope scope0 @leaf name @leaf file file0 line 1 scope_line 1
debug scope scope1 @middle name @middle file file0 line 2 scope_line 2
debug scope scope2 @main name @main file file0 line 3 scope_line 3
debug loc loc0 file0 line 1 column 20 scope scope0
debug loc loc1 file0 line 2 column 20 scope scope1
debug loc loc2 file0 line 3 column 20 scope scope2

function @leaf(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 v0, v0 dbg loc0
    return v1 dbg loc0
}

function @middle(i32) -> i32 internal {
block0(v0: i32):
    v1 = call @leaf(v0) dbg loc1
    return v1 dbg loc1
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @middle(v0) dbg loc2
    return v1 dbg loc2
}
";
    let arena = Bump::new();
    let module = optimized(&arena, text, &only(&[Pass::Inline]));
    let debug = module.debug_info();
    let body = body_named(&module, "main");
    let leaf = body
        .blocks()
        .flat_map(|block| body.block_insts(block).iter().copied())
        .filter_map(|inst| body.inst_location(inst))
        .filter_map(|location| debug.location_data(location))
        .find(|location| location.line == 1)
        .unwrap();
    let middle = debug.location_data(leaf.inlined_at.unwrap()).unwrap();
    assert_eq!(middle.line, 2);
    let call = debug.location_data(middle.inlined_at.unwrap()).unwrap();
    assert_eq!((call.line, call.inlined_at), (3, None));
}

#[test]
fn an_instruction_moved_out_of_its_loop_loses_its_line() {
    let arena = Bump::new();
    let module = optimized(&arena, &located(), &only(&[Pass::Licm]));
    let body = body_named(&module, "loop");
    let entry = body.entry_block().unwrap();
    let multiply = body
        .block_insts(entry)
        .iter()
        .copied()
        .find(|&inst| body.inst(inst).opcode() == crate::ir::Opcode::Imul)
        .expect("the invariant multiplication left the loop");
    assert_eq!(body.inst_location(multiply), None);
    // What stayed keeps its own, the terminators too.
    let last = body.blocks().next_back().unwrap();
    let line = body
        .terminator(last)
        .and_then(|inst| body.inst_location(inst))
        .and_then(|location| module.debug_info().location_data(location))
        .map(|location| location.line);
    assert_eq!(line, Some(24));
}

#[test]
fn locations_change_nothing_a_pass_decides() {
    for options in [
        OptimizerOptions {
            verify_each: true,
            ..OptimizerOptions::default()
        },
        only(&[
            Pass::Inline,
            Pass::Sroa,
            Pass::Promote,
            Pass::Fold,
            Pass::Combine,
            Pass::SimplifyCfg,
            Pass::Sccp,
            Pass::KnownBits,
            Pass::Gvn,
            Pass::Licm,
            Pass::Dce,
        ]),
    ] {
        let (with, with_report) = optimize_text(&located(), &options);
        let (without, without_report) = optimize_text(FUNCTIONS_PLAIN, &options);
        pretty_assertions::assert_eq!(with, without);
        assert_eq!(
            with_report.transformations(),
            without_report.transformations()
        );
    }
}

#[test]
fn every_surviving_location_belongs_to_its_function() {
    let arena = Bump::new();
    let options = OptimizerOptions {
        verify_each: true,
        ..OptimizerOptions::default()
    };
    let module = optimized(&arena, &located(), &options);
    let debug = module.debug_info();
    for (func, function) in module.functions() {
        let body = function.body.as_ref().unwrap();
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                if let Some(location) = body.inst_location(inst) {
                    assert_eq!(debug.root_function(location), Some(func));
                    assert!(location.index() < debug.location_count());
                }
            }
        }
    }
}

const FUNCTIONS_PLAIN: &str = "\
function @twice(i32) -> i32 internal {
block0(v0: i32):
    v1 = iadd.i32 nsw v0, v0
    return v1
}

function @main(i32) -> i32 external {
block0(v0: i32):
    v1 = call @twice(v0)
    v2 = iadd.i32 v1, v0
    return v2
}

function @loop(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iconst.i32 0
    jump block1(v2, v2)
block1(v3: i32, v4: i32):
    v5 = imul.i32 nsw v0, v1
    v6 = iadd.i32 v4, v5
    v7 = iadd.i32 v3, v1
    v8 = icmp.i32 slt v7, v0
    brif v8, block1(v7, v6), block2
block2:
    return v4
}
";
