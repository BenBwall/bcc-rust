//! Linear memory: loads and stores of every width, bounds, growth, data
//! segments and the bulk memory instructions.

use super::{
    Builder,
    call,
    eval_i32,
    eval_i64,
    trap,
};
use crate::{
    TrapKind,
    ValType::{
        F32,
        I32,
        I64,
    },
    Value,
};

#[test]
fn stores_are_little_endian_and_loads_extend() {
    assert_eq!(
        eval_i32("i32.const 8 i32.const 0x11223344 i32.store i32.const 8 i32.load8_u"),
        0x44
    );
    assert_eq!(
        eval_i32("i32.const 8 i32.const 0x11223344 i32.store i32.const 11 i32.load8_u"),
        0x11
    );
    assert_eq!(
        eval_i32("i32.const 0 i32.const 0xff i32.store8 i32.const 0 i32.load8_s"),
        -1
    );
    assert_eq!(
        eval_i32("i32.const 0 i32.const 0x8001 i32.store16 i32.const 0 i32.load16_s"),
        -32767
    );
    assert_eq!(
        eval_i32("i32.const 0 i32.const 0x8001 i32.store16 i32.const 0 i32.load16_u"),
        0x8001
    );
    assert_eq!(
        eval_i64("i32.const 0 i64.const -2 i64.store i32.const 0 i64.load32_u"),
        0xFFFF_FFFE
    );
    assert_eq!(
        eval_i64("i32.const 0 i64.const -2 i64.store i32.const 4 i64.load32_s"),
        -1
    );
    assert_eq!(
        eval_i64("i32.const 0 i64.const 0x1234 i64.store16 i32.const 0 i64.load"),
        0x1234
    );
    // The constant offset adds to the address operand.
    assert_eq!(
        eval_i32("i32.const 4 i32.const 77 i32.store offset=12 i32.const 16 i32.load"),
        77
    );
    // Unaligned access is allowed; alignment is only a hint.
    assert_eq!(
        eval_i32("i32.const 1 i32.const 0x01020304 i32.store align=1 i32.const 1 i32.load align=1"),
        0x0102_0304
    );
}

#[test]
fn floats_round_trip_through_memory_bit_for_bit() {
    let mut module = Builder::new();
    module.memory(1, None);
    module.func(
        "f",
        &[],
        &[I32],
        &[],
        "i32.const 0 f32.const bits:0x7fa00001 f32.store i32.const 0 i32.load",
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I32(0x7FA0_0001)]);
    let mut module = Builder::new();
    module.memory(1, None);
    module.func(
        "g",
        &[],
        &[F32],
        &[],
        "i32.const 0 i32.const 0x3fc00000 i32.store i32.const 0 f32.load",
    );
    assert_eq!(call(&module, "g", &[]).unwrap(), [Value::f32(1.5)]);
}

#[test]
fn accesses_past_the_end_trap() {
    // One page is 65536 bytes: the last i32 starts at 65532.
    assert_eq!(eval_i32("i32.const 65532 i32.load"), 0);
    assert_eq!(
        trap(I32, "i32.const 65533 i32.load"),
        TrapKind::MemoryOutOfBounds
    );
    assert_eq!(
        trap(I32, "i32.const 65535 i32.load offset=1"),
        TrapKind::MemoryOutOfBounds
    );
    // The address is unsigned: -1 is 4 GiB - 1, and the offset does not wrap.
    assert_eq!(
        trap(I32, "i32.const -1 i32.load8_u"),
        TrapKind::MemoryOutOfBounds
    );
    assert_eq!(
        trap(
            I32,
            "i32.const 1 i32.const 0 i32.store offset=4294967295 i32.const 0"
        ),
        TrapKind::MemoryOutOfBounds
    );
}

#[test]
fn memory_grows_by_pages_up_to_its_maximum() {
    let mut module = Builder::new();
    module.memory(1, Some(3));
    module.func(
        "f",
        &[],
        &[I32],
        &[],
        // 1 -> 3 pages succeeds (returns 1), 3 -> 4 fails (returns -1).
        "i32.const 2 memory.grow i32.const 1 memory.grow i32.add memory.size i32.const 100 \
         i32.mul i32.add",
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I32(300)]);
    // New pages are zeroed and addressable.
    let mut module = Builder::new();
    module.memory(1, None);
    module.func(
        "g",
        &[],
        &[I32],
        &[],
        "i32.const 1 memory.grow drop i32.const 70000 i32.load",
    );
    assert_eq!(call(&module, "g", &[]).unwrap(), [Value::I32(0)]);
    // The stepper caps memory at 16 MiB.
    let mut module = Builder::new();
    module.memory(1, None);
    module.func("h", &[], &[I32], &[], "i32.const 1000 memory.grow");
    assert_eq!(call(&module, "h", &[]).unwrap(), [Value::I32(-1)]);
}

#[test]
fn active_data_segments_are_copied_at_instantiation() {
    let mut module = Builder::new();
    module.memory(1, None).data(Some(100), b"Hi!\0");
    module.func("f", &[], &[I32], &[], "i32.const 101 i32.load8_u");
    assert_eq!(
        call(&module, "f", &[]).unwrap(),
        [Value::I32(i32::from(b'i'))]
    );
}

#[test]
fn a_data_segment_that_does_not_fit_fails_instantiation() {
    let mut module = Builder::new();
    module.memory(1, None).data(Some(65534), b"abc");
    module.func("main", &[], &[], &[], "");
    let program = super::program(&module);
    let error = crate::Session::new(program)
        .err()
        .expect("instantiation fails");
    assert!(error.0.contains("does not fit"), "{error}");
}

#[test]
fn bulk_memory_copies_fills_and_initializes() {
    let mut module = Builder::new();
    module
        .memory(1, None)
        .data(Some(0), b"abcdef")
        .data(None, b"XYZ");
    module.func(
        "copy",
        &[],
        &[I32],
        &[],
        // Overlapping copy of "abcde" one byte up: "aabcde".
        "i32.const 1 i32.const 0 i32.const 5 memory.copy i32.const 2 i32.load8_u",
    );
    module.func(
        "fill",
        &[],
        &[I32],
        &[],
        "i32.const 10 i32.const 0x2a i32.const 4 memory.fill i32.const 10 i32.load",
    );
    module.func(
        "init",
        &[],
        &[I32],
        &[],
        "i32.const 20 i32.const 1 i32.const 2 memory.init 1 i32.const 20 i32.load16_u",
    );
    module.func(
        "dropped",
        &[],
        &[],
        &[],
        "data.drop 1 i32.const 20 i32.const 0 i32.const 1 memory.init 1",
    );
    module.func(
        "copy_out",
        &[],
        &[],
        &[],
        "i32.const 65535 i32.const 0 i32.const 2 memory.copy",
    );
    assert_eq!(
        call(&module, "copy", &[]).unwrap(),
        [Value::I32(i32::from(b'b'))]
    );
    assert_eq!(
        call(&module, "fill", &[]).unwrap(),
        [Value::I32(0x2A2A_2A2A)]
    );
    assert_eq!(
        call(&module, "init", &[]).unwrap(),
        [Value::I32(i32::from(b'Y') | i32::from(b'Z') << 8)]
    );
    assert_eq!(
        call(&module, "dropped", &[]).unwrap_err().kind,
        TrapKind::MemoryOutOfBounds
    );
    assert_eq!(
        call(&module, "copy_out", &[]).unwrap_err().kind,
        TrapKind::MemoryOutOfBounds
    );
}

#[test]
fn tables_hold_references() {
    let mut module = Builder::new();
    let target = module.func("target", &[], &[I32], &[], "i32.const 7");
    module.table(2);
    module.func(
        "f",
        &[],
        &[I32],
        &[],
        &format!(
            "i32.const 1 ref.func {target} table.set
             i32.const 0 table.get ref.is_null
             i32.const 1 table.get ref.is_null i32.const 10 i32.mul i32.add
             ref.null func i32.const 3 table.grow i32.const 100 i32.mul i32.add
             table.size i32.const 1000 i32.mul i32.add"
        ),
    );
    // null at 0 (1), func at 1 (0), grow from 2 (200), size 5 (5000).
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I32(5201)]);
    let mut module = Builder::new();
    module.table(1);
    module.func("g", &[], &[I32], &[], "i32.const 1 table.get ref.is_null");
    assert_eq!(
        call(&module, "g", &[]).unwrap_err().kind,
        TrapKind::TableOutOfBounds
    );
}

#[test]
fn i64_values_store_in_eight_bytes() {
    let mut module = Builder::new();
    module.memory(1, None);
    module.func(
        "f",
        &[],
        &[I64],
        &[],
        "i32.const 0 i64.const 0x0102030405060708 i64.store i32.const 7 i64.load8_u",
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I64(1)]);
}
