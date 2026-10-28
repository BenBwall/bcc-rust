//! Spec-style tests: each instruction family against the values the
//! WebAssembly specification requires, control flow and traps, memory
//! bounds, the stepping commands, the WASI host, decoding and validation,
//! the listing, and the clang-built lesson modules in `web/wasm/`.
//!
//! Modules are built with `builder.rs`'s encoder and assembler.

mod builder;
mod clang;
mod control;
mod format;
mod memory;
mod numeric;
mod stepping;
mod wasi;

use std::rc::Rc;

use builder::Builder;

use crate::{
    Host,
    Machine,
    Program,
    Status,
    Trap,
    Value,
};

/// Load `module`, panicking with the loader's message if it is rejected.
fn program(module: &Builder) -> Rc<Program> {
    Rc::new(Program::load(&module.build(), None).unwrap_or_else(|error| panic!("{error}")))
}

/// Call the exported function `name` with `args` and run it to the end.
fn call(module: &Builder, name: &str, args: &[Value]) -> Result<Vec<Value>, Trap> {
    let program = program(module);
    let mut machine = machine(&program);
    let func = program
        .module
        .export(name, crate::ExternKind::Func)
        .unwrap_or_else(|| panic!("no export {name}"));
    machine.call_with(func, args);
    finish(&mut machine)
}

fn machine(program: &Program) -> Machine {
    Machine::instantiate(Rc::clone(&program.module), None, Host::default())
        .unwrap_or_else(|error| panic!("{error}"))
}

fn finish(machine: &mut Machine) -> Result<Vec<Value>, Trap> {
    for _ in 0..1_000_000 {
        machine.step();
        match &machine.status {
            | Status::Running => {},
            | Status::Finished(values) => return Ok(values.clone()),
            | Status::Exited(code) => return Ok(vec![Value::I32(*code)]),
            | Status::Trapped(trap) => return Err(trap.clone()),
        }
    }
    panic!("the test program did not finish");
}

/// A module with one function, `f`, of the given signature and body.
fn single(params: &[crate::ValType], results: &[crate::ValType], code: &str) -> Builder {
    let mut module = Builder::new();
    module.memory(1, None);
    module.func("f", params, results, &[], code);
    module
}

/// Evaluate `code` as the body of a `[] -> [ty]` function.
fn eval(ty: crate::ValType, code: &str) -> Result<Value, Trap> {
    call(&single(&[], &[ty], code), "f", &[]).map(|values| values[0])
}

fn eval_i32(code: &str) -> i32 {
    match eval(crate::ValType::I32, code) {
        | Ok(Value::I32(value)) => value,
        | other => panic!("{code}: {other:?}"),
    }
}

fn eval_i64(code: &str) -> i64 {
    match eval(crate::ValType::I64, code) {
        | Ok(Value::I64(value)) => value,
        | other => panic!("{code}: {other:?}"),
    }
}

fn eval_f32(code: &str) -> u32 {
    match eval(crate::ValType::F32, code) {
        | Ok(Value::F32(bits)) => bits,
        | other => panic!("{code}: {other:?}"),
    }
}

fn eval_f64(code: &str) -> u64 {
    match eval(crate::ValType::F64, code) {
        | Ok(Value::F64(bits)) => bits,
        | other => panic!("{code}: {other:?}"),
    }
}

/// The trap `code` (a `[] -> [ty]` body) raises.
fn trap(ty: crate::ValType, code: &str) -> crate::TrapKind {
    match eval(ty, code) {
        | Err(trap) => trap.kind,
        | Ok(value) => panic!("{code}: expected a trap, got {value:?}"),
    }
}
