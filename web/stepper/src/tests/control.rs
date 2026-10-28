//! Structured control flow, calls and the parametric instructions.

use super::{
    Builder,
    call,
    eval_i32,
    single,
    trap,
};
use crate::{
    TrapKind,
    ValType::{
        F64,
        I32,
        I64,
    },
    Value,
};

#[test]
fn blocks_produce_their_results() {
    assert_eq!(eval_i32("block i32 i32.const 7 end"), 7);
    assert_eq!(
        eval_i32("block i32 i32.const 1 i32.const 2 i32.add end i32.const 10 i32.mul"),
        30
    );
    // A branch out of nested blocks carries the label's result.
    assert_eq!(
        eval_i32("block i32 block i32 i32.const 3 br 1 end drop i32.const 9 end"),
        3
    );
}

#[test]
fn br_if_branches_only_on_nonzero() {
    let code = "block i32 i32.const 1 local.get 0 br_if 0 drop i32.const 2 end";
    let module = single(&[I32], &[I32], code);
    assert_eq!(
        call(&module, "f", &[Value::I32(5)]).unwrap(),
        [Value::I32(1)]
    );
    assert_eq!(
        call(&module, "f", &[Value::I32(0)]).unwrap(),
        [Value::I32(2)]
    );
}

#[test]
fn loops_branch_back_to_their_start() {
    // Sum 1..=n with a loop and a counter in a local.
    let mut module = Builder::new();
    module.func(
        "sum",
        &[I32],
        &[I32],
        &[I32],
        "loop
           local.get 1 local.get 0 i32.add local.set 1
           local.get 0 i32.const 1 i32.sub local.tee 0
           br_if 0
         end
         local.get 1",
    );
    assert_eq!(
        call(&module, "sum", &[Value::I32(10)]).unwrap(),
        [Value::I32(55)]
    );
}

#[test]
fn if_else_takes_one_branch() {
    let module = single(
        &[I32],
        &[I32],
        "local.get 0 if i32 i32.const 10 else i32.const 20 end",
    );
    assert_eq!(
        call(&module, "f", &[Value::I32(1)]).unwrap(),
        [Value::I32(10)]
    );
    assert_eq!(
        call(&module, "f", &[Value::I32(0)]).unwrap(),
        [Value::I32(20)]
    );
    // Without an else, a false condition skips to the end.
    let module = single(
        &[I32],
        &[I32],
        "local.get 0 if i32.const 6 return end i32.const 5",
    );
    assert_eq!(
        call(&module, "f", &[Value::I32(0)]).unwrap(),
        [Value::I32(5)]
    );
    assert_eq!(
        call(&module, "f", &[Value::I32(1)]).unwrap(),
        [Value::I32(6)]
    );
}

#[test]
fn br_table_selects_by_index_with_a_default() {
    let code = "block block block
                  local.get 0 br_table 0 1 2
                end i32.const 100 return
              end i32.const 101 return
            end i32.const 102";
    let module = single(&[I32], &[I32], code);
    for (index, expected) in [(0, 100), (1, 101), (2, 102), (7, 102), (-1, 102)] {
        assert_eq!(
            call(&module, "f", &[Value::I32(index)]).unwrap(),
            [Value::I32(expected)],
            "index {index}"
        );
    }
}

#[test]
fn multi_value_blocks_take_parameters() {
    let mut module = Builder::new();
    let ty = module.ty(&[I32, I32], &[I32]);
    module.func(
        "f",
        &[],
        &[I32],
        &[],
        &format!("i32.const 6 i32.const 7 block type={ty} i32.mul end"),
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I32(42)]);
    // A branch to a loop carries the loop's parameters.
    let mut fixed = Builder::new();
    let ty = fixed.ty(&[I32], &[I32]);
    fixed.func(
        "count",
        &[],
        &[I32],
        &[I32],
        &format!(
            "i32.const 0 loop type={ty} i32.const 1 i32.add local.tee 0 local.get 0 i32.const 5 \
             i32.lt_s br_if 0 end"
        ),
    );
    assert_eq!(call(&fixed, "count", &[]).unwrap(), [Value::I32(5)]);
}

#[test]
fn calls_pass_arguments_and_return_results() {
    let mut module = Builder::new();
    module.func(
        "square",
        &[I64],
        &[I64],
        &[],
        "local.get 0 local.get 0 i64.mul",
    );
    module.func(
        "f",
        &[I64],
        &[I64],
        &[],
        "local.get 0 call square i64.const 1 i64.add",
    );
    assert_eq!(
        call(&module, "f", &[Value::I64(9)]).unwrap(),
        [Value::I64(82)]
    );
}

#[test]
fn recursion_builds_one_frame_per_call() {
    let mut module = Builder::new();
    module.func(
        "fact",
        &[I32],
        &[I32],
        &[],
        "local.get 0 i32.const 1 i32.le_s
         if i32 i32.const 1
         else local.get 0 local.get 0 i32.const 1 i32.sub call fact i32.mul end",
    );
    assert_eq!(
        call(&module, "fact", &[Value::I32(10)]).unwrap(),
        [Value::I32(3_628_800)]
    );
}

#[test]
fn return_unwinds_blocks_and_discards_operands() {
    let module = single(
        &[],
        &[I32],
        "i32.const 1 i32.const 2 block block i32.const 3 return end end unreachable",
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::I32(3)]);
}

#[test]
fn runaway_recursion_exhausts_the_call_stack() {
    let mut module = Builder::new();
    module.func("f", &[], &[], &[], "call f");
    assert_eq!(
        call(&module, "f", &[]).unwrap_err().kind,
        TrapKind::CallStackExhausted
    );
}

#[test]
fn call_indirect_checks_the_table_entry() {
    let mut module = Builder::new();
    let double = module.func(
        "double",
        &[I32],
        &[I32],
        &[],
        "local.get 0 i32.const 2 i32.mul",
    );
    let wide = module.func("wide", &[I64], &[I64], &[], "local.get 0");
    let ty = module.ty(&[I32], &[I32]);
    module.func(
        "dispatch",
        &[I32],
        &[I32],
        &[],
        &format!("i32.const 21 local.get 0 call_indirect {ty}"),
    );
    module.table(3).elements(0, &[double, wide]);
    let dispatch = |index| call(&module, "dispatch", &[Value::I32(index)]);
    assert_eq!(dispatch(0).unwrap(), [Value::I32(42)]);
    assert_eq!(
        dispatch(1).unwrap_err().kind,
        TrapKind::IndirectCallTypeMismatch
    );
    assert_eq!(
        dispatch(2).unwrap_err().kind,
        TrapKind::UninitializedElement
    );
    assert_eq!(dispatch(3).unwrap_err().kind, TrapKind::UndefinedElement);
}

#[test]
fn select_and_drop() {
    assert_eq!(eval_i32("i32.const 1 i32.const 2 i32.const 0 select"), 2);
    assert_eq!(eval_i32("i32.const 1 i32.const 2 i32.const 9 select"), 1);
    assert_eq!(eval_i32("i32.const 1 i32.const 2 drop"), 1);
    let module = single(
        &[],
        &[F64],
        "f64.const 1.5 f64.const 2.5 i32.const 1 select",
    );
    assert_eq!(call(&module, "f", &[]).unwrap(), [Value::f64(1.5)]);
}

#[test]
fn unreachable_traps() {
    assert_eq!(trap(I32, "unreachable"), TrapKind::Unreachable);
}

#[test]
fn the_start_function_runs_before_the_entry_point() {
    let mut module = Builder::new();
    let counter = module.global("counter", I32, true, "i32.const 1");
    let init = module.func(
        "init",
        &[],
        &[],
        &[],
        &format!("global.get {counter} i32.const 40 i32.add global.set {counter}"),
    );
    module.func(
        "main",
        &[],
        &[I32],
        &[],
        &format!("global.get {counter} i32.const 1 i32.add"),
    );
    module.start(init);
    let program = super::program(&module);
    let mut session = crate::Session::new(program).unwrap();
    assert_eq!(
        session.run(crate::Command::Continue, 1000),
        crate::Stop::Ended
    );
    assert_eq!(
        session.machine().status,
        crate::Status::Finished(vec![Value::I32(42)])
    );
}
