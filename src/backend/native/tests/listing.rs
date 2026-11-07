//! What the emitter records about its listing: following the IR through the
//! back end changes no line, and each line of code names what it is for,
//! through selection, allocation's spill code and rewrites of the blocks.

use super::{
    super::{
        InstSource,
        ListingLine,
        emit,
        emit_assembly,
        emit_assembly_listing,
    },
    spill_everything,
};
use crate::{
    ir::{
        Entity,
        parse_module,
    },
    target::{
        CodeOptions,
        Target,
    },
    util::bump::Bump,
};

/// A loop whose test the back end copies over its back edge, a value live
/// across a call, and an addition the peephole combines with its copy.
const PROGRAM: &str = "abi lowered
function @g() -> i64 external
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = iconst.i64 0
    jump block1(v2, v0)
block1(v3: i64, v4: i64):
    v5 = icmp.i64 slt v3, v1
    brif v5, block2, block3
block2:
    v6 = call @g()
    v7 = iadd.i64 v4, v6
    v8 = iconst.i64 1
    v9 = iadd.i64 v3, v8
    jump block1(v9, v7)
block3:
    v10 = iadd.i64 v4, v0
    return v10
}
";

/// Whether a line of the listing is an instruction.
fn is_instruction(line: &str) -> bool {
    let line = line.trim();
    !(line.is_empty() || line.ends_with(':') || line.starts_with('.'))
}

/// Checks that `listed` says what each instruction of `text` is for, and
/// returns how many came from the IR.
fn check_listing(text: &str, listed: &[ListingLine], inst_count: usize) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    for (number, line) in lines.iter().enumerate() {
        let in_function = text.lines().take(number).any(|line| line == "f:")
            && !text.lines().take(number).any(|line| line.contains(".size"));
        if in_function && is_instruction(line) {
            assert_eq!(
                listed
                    .iter()
                    .filter(|entry| entry.line as usize == number)
                    .count(),
                1,
                "line {number}, {line:?}\n{text}"
            );
        }
    }
    listed
        .iter()
        .filter(|entry| is_instruction(lines[entry.line as usize]))
        .filter(|entry| match entry.source {
            | InstSource::Ir(inst) => {
                assert!(inst.index() < inst_count, "{entry:?}");
                true
            },
            | InstSource::Allocator | InstSource::Entry => false,
        })
        .count()
}

#[test]
fn recording_the_listing_changes_no_line() {
    let arena = Bump::new();
    let module = parse_module(&arena, PROGRAM).unwrap();
    let crate::ir::Symbol::Function(id) = module.symbol("f").unwrap() else {
        panic!("f is a function");
    };
    let inst_count = module.function(id).body.as_ref().unwrap().inst_count();

    let mut plain = String::new();
    emit_assembly(&module, Target::LinuxGnu, &mut plain).unwrap();
    let mut recorded = String::new();
    let mut listed = Vec::new();
    emit_assembly_listing(
        &module,
        Target::LinuxGnu,
        CodeOptions::default(),
        &mut recorded,
        &mut |line| listed.push(line),
    )
    .unwrap();
    pretty_assertions::assert_eq!(recorded, plain);
    assert!(
        check_listing(&recorded, &listed, inst_count) > 0,
        "{recorded}"
    );
}

/// With every value spilled, each instruction's spill code is that
/// instruction's, so nearly every line still names the IR.
#[test]
fn spill_code_serves_its_instruction() {
    let arena = Bump::new();
    let module = parse_module(&arena, PROGRAM).unwrap();
    let crate::ir::Symbol::Function(id) = module.symbol("f").unwrap() else {
        panic!("f is a function");
    };
    let inst_count = module.function(id).body.as_ref().unwrap().inst_count();

    let mut plain = String::new();
    emit(
        &module,
        Target::LinuxGnu,
        CodeOptions::default(),
        &mut plain,
        spill_everything,
        None,
    )
    .unwrap();
    let mut recorded = String::new();
    let mut listed = Vec::new();
    emit(
        &module,
        Target::LinuxGnu,
        CodeOptions::default(),
        &mut recorded,
        spill_everything,
        Some(&mut |line| listed.push(line)),
    )
    .unwrap();
    pretty_assertions::assert_eq!(recorded, plain);
    assert!(
        check_listing(&recorded, &listed, inst_count) > 0,
        "{recorded}"
    );
}
