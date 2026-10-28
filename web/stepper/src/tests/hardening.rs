//! Inputs chosen to exhaust the stepper: malformed debug information, host
//! calls and imports that ask for huge allocations, and programs that would
//! make checkpoints grow without bound. Each must fail cleanly, within the
//! limits.

use std::rc::Rc;

use super::{
    Builder,
    builder::leb,
    program,
    raw,
};
use crate::{
    CHECKPOINT_BYTES,
    Command,
    Host,
    Machine,
    Program,
    RECENT_INTERVAL,
    Session,
    Status,
    TrapKind,
    ValType::I32,
    Value,
    retained_bytes,
};

const WASI: &str = "wasi_snapshot_preview1";

/// A `.debug_line` custom section holding one DWARF 5 unit whose header
/// ends with `tables` (the directory and file tables) and whose program is
/// `program`.
fn debug_line(tables: &[u8], program: &[u8]) -> Vec<u8> {
    // min_inst, max_ops, default_is_stmt, line_base -5, line_range 14,
    // opcode_base 13 and the standard opcodes' operand counts.
    let mut header = vec![1, 1, 1, 0xFB, 14, 13, 0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1];
    header.extend_from_slice(tables);
    let mut unit = vec![5, 0, 4, 0];
    unit.extend((header.len() as u32).to_le_bytes());
    unit.extend(header);
    unit.extend_from_slice(program);
    let mut custom = Vec::new();
    leb(&mut custom, 11);
    custom.extend(b".debug_line");
    custom.extend((unit.len() as u32).to_le_bytes());
    custom.extend(unit);
    custom
}

/// One directory, `d`, and one file, `a.c`, as DWARF 5 tables.
const TABLES: &[u8] = &[1, 1, 0x08, 1, b'd', 0, 1, 1, 0x08, 1, b'a', b'.', b'c', 0];

#[test]
fn an_entry_count_beyond_the_section_is_rejected_without_allocating() {
    // No entry fields, and 2^32 - 1 entries.
    let custom = debug_line(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F], &[]);
    let module = Program::load(&raw(&[(0, &custom)]), None).unwrap();
    assert!(module.lines.is_empty());
    // Fields, but more entries than bytes left.
    let custom = debug_line(&[1, 1, 0x08, 0xFF, 0xFF, 0x03], &[]);
    let module = Program::load(&raw(&[(0, &custom)]), None).unwrap();
    assert!(module.lines.is_empty());
}

#[test]
fn line_and_address_overflow_end_the_program() {
    let max = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    for opcode in [0x03, 0x02] {
        // set_file 0, set_address 0, copy (row at line 1), then advance
        // the line (or the address) by 2^63 - 1 twice.
        let mut program = vec![0x04, 0, 0x00, 5, 0x02, 0, 0, 0, 0, 0x01];
        for _ in 0..2 {
            program.push(opcode);
            program.extend(max);
        }
        program.push(0x01);
        let custom = debug_line(TABLES, &program);
        let module = Program::load(&raw(&[(0, &custom)]), None).unwrap();
        // The row before the overflow survives.
        assert_eq!(module.lines.line_at(module.module.code_base), Some(1));
    }
    // An address size of 0 is malformed. The unit starts after the name's
    // length (1 byte), the name (11) and the unit's length (4); the address
    // size follows the 2-byte version.
    let valid = debug_line(TABLES, &[0x04, 0, 0x01]);
    assert!(
        !Program::load(&raw(&[(0, &valid)]), None)
            .unwrap()
            .lines
            .is_empty()
    );
    let mut malformed = valid;
    assert_eq!(malformed[16..19], [5, 0, 4]);
    malformed[18] = 0;
    let module = Program::load(&raw(&[(0, &malformed)]), None).unwrap();
    assert!(module.lines.is_empty());
}

#[test]
fn random_get_checks_its_buffer_before_filling_it() {
    let mut module = Builder::new();
    module.import(WASI, "random_get", &[I32, I32], &[I32]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[I32],
        &[],
        "i32.const 0 i32.const 1073741824 call random_get",
    );
    let mut session = Session::new(program(&module)).unwrap();
    session.run(Command::Continue, 100);
    let Status::Trapped(trap) = &session.machine().status else {
        panic!("{:?}", session.machine().status);
    };
    assert_eq!(trap.kind, TrapKind::MemoryOutOfBounds);
}

#[test]
fn fd_write_keeps_at_most_the_output_limit() {
    // 1,024 iovecs, each the whole 1 MiB-minus-header of memory.
    let mut module = Builder::new();
    module.import(WASI, "fd_write", &[I32, I32, I32, I32], &[I32]);
    module.memory(17, None);
    module.func(
        "_start",
        &[],
        &[I32],
        &[I32],
        "loop
           local.get 0 i32.const 8 i32.mul i32.const 16 i32.add i32.const 65536 i32.store
           local.get 0 i32.const 8 i32.mul i32.const 20 i32.add i32.const 1048576 i32.store
           local.get 0 i32.const 1 i32.add local.tee 0 i32.const 1024 i32.lt_u br_if 0
         end
         i32.const 1 i32.const 16 i32.const 1024 i32.const 8 call fd_write",
    );
    let mut session = Session::new(program(&module)).unwrap();
    session.run(Command::Continue, 1_000_000);
    let machine = session.machine();
    assert_eq!(machine.status, Status::Finished(vec![Value::I32(0)]));
    let kept: usize = machine
        .host
        .output
        .iter()
        .map(|(_, bytes)| bytes.len())
        .sum();
    assert_eq!(kept, 1 << 20, "the limit");
    assert_eq!(machine.host.dropped, (1024 << 20) - (1 << 20));
    // fd_write reports everything as written: 1 GiB.
    let memory = &machine.memory.as_ref().unwrap().bytes;
    assert_eq!(&memory[8..12], (1_u32 << 30).to_le_bytes());
}

#[test]
fn an_imported_table_is_limited_like_a_defined_one() {
    let mut import = vec![1, 1, b'e', 1, b't', 1, 0x70, 0];
    leb(&mut import, 100_000_000);
    let program = Program::load(&raw(&[(2, &import)]), None).unwrap();
    let error = Machine::instantiate(Rc::clone(&program.module), None, Host::default())
        .expect_err("instantiation fails");
    assert!(error.0.contains("a table of 100000000 entries"), "{error}");
}

#[test]
fn checkpoints_stay_within_their_byte_budget() {
    // 16 MiB of memory, written on every iteration of an endless loop, so
    // each checkpoint after a store owns a whole copy.
    let mut module = Builder::new();
    module.memory(256, None);
    module.func(
        "_start",
        &[],
        &[],
        &[I32],
        "loop local.get 0 i32.const 1 i32.add local.tee 0 i32.const 4 i32.mul local.get 0 \
         i32.store br 0 end",
    );
    let mut session = Session::new(program(&module)).unwrap();
    for _ in 0..40 {
        session.run(Command::Continue, 10_000);
        assert!(
            retained_bytes(&session.checkpoints) <= CHECKPOINT_BYTES,
            "{} bytes after {} steps",
            retained_bytes(&session.checkpoints),
            session.machine().steps
        );
    }
    // Going back still lands on the same state as running forward.
    let mut forward = Session::new(Rc::clone(session.program())).unwrap();
    forward.run(Command::Continue, 123_457);
    session.go_to(123_457);
    assert_eq!(session.machine().stack, forward.machine().stack);
    assert_eq!(session.machine().frames, forward.machine().frames);
    assert_eq!(
        session.machine().memory.as_ref().unwrap().bytes,
        forward.machine().memory.as_ref().unwrap().bytes
    );
    // A loop that only reads memory shares it, so it keeps more checkpoints.
    let mut module = Builder::new();
    module.memory(256, None);
    module.func(
        "_start",
        &[],
        &[],
        &[],
        "loop i32.const 0 i32.load drop br 0 end",
    );
    let mut session = Session::new(program(&module)).unwrap();
    session.run(Command::Continue, 100_000);
    assert!(
        session.checkpoints.len() > 16,
        "{}",
        session.checkpoints.len()
    );
    assert!(retained_bytes(&session.checkpoints) < 2 * (16 << 20));
}

/// A program with big static arrays fits up to 64 MiB of memory; going
/// back in it replays exactly, and near where the run is it stays quick
/// though its budgeted checkpoints thin to the first. Past the cap it is
/// refused, in words a beginner can act on.
#[test]
fn memory_up_to_64_mib_runs_and_more_is_refused_plainly() {
    let mut module = Builder::new();
    module.memory(1024, None);
    // Write a word into every 64 KiB page, over and over.
    module.func(
        "_start",
        &[],
        &[],
        &[I32],
        "loop local.get 0 i32.const 65536 i32.add i32.const 67108863 i32.and local.tee 0 \
         local.get 0 i32.store br 0 end",
    );
    let mut session = Session::new(program(&module)).unwrap();
    session.run(Command::Continue, 300_000);
    assert_eq!(session.machine().status, Status::Running);
    assert_eq!(session.machine().memory.as_ref().unwrap().pages(), 1024);
    // The budgeted checkpoints have thinned to the first, but the recent
    // copies keep going back cheap: a step back, and a few thousand, each
    // re-execute at most two recent intervals, not 300,000 instructions.
    assert_eq!(session.checkpoints.len(), 1);
    for target in [299_999, 290_000, 270_123] {
        let mut forward = Session::new(Rc::clone(session.program())).unwrap();
        forward.run(Command::Continue, target);
        session.go_to(target);
        assert!(
            session.replayed <= 2 * RECENT_INTERVAL,
            "{} instructions re-executed to reach {target}",
            session.replayed
        );
        assert_eq!(session.machine().steps, target);
        assert_eq!(session.machine().stack, forward.machine().stack);
        assert_eq!(
            session.machine().memory.as_ref().unwrap().bytes,
            forward.machine().memory.as_ref().unwrap().bytes
        );
    }
    // Far back, before the recent copies, it replays from the first.
    session.go_to(31_234);
    assert_eq!(session.replayed, 31_234);

    let mut module = Builder::new();
    module.memory(1025, None);
    module.func("_start", &[], &[], &[], "");
    let error = Session::new(program(&module))
        .err()
        .expect("1025 pages are refused");
    assert!(
        error.0.contains("needs 65 MiB of memory")
            && error.0.contains("at most 64 MiB")
            && error.0.contains("Make its largest arrays smaller"),
        "{error}"
    );
}
