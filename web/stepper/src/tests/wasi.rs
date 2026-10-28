//! The WASI preview 1 host: output, exit, arguments, the simulated clock,
//! and imports the stepper does not provide.

use super::{
    Builder,
    program,
};
use crate::{
    Command,
    Effect,
    Program,
    Session,
    Status,
    Stream,
    ValType::{
        I32,
        I64,
    },
    Value,
};

const WASI: &str = "wasi_snapshot_preview1";

/// A program that writes "hello\n" to `fd` with `fd_write`, then returns
/// `fd_write`'s errno.
fn hello(fd: i32) -> Builder {
    let mut module = Builder::new();
    module.import(WASI, "fd_write", &[I32, I32, I32, I32], &[I32]);
    module.memory(1, None).data(Some(16), b"hello\n");
    // One iovec at 0: { base 16, length 6 }; the count is written to 8.
    module.func(
        "_start",
        &[],
        &[I32],
        &[],
        &format!(
            "i32.const 0 i32.const 16 i32.store i32.const 4 i32.const 6 i32.store
             i32.const {fd} i32.const 0 i32.const 1 i32.const 8 call fd_write"
        ),
    );
    module
}

fn run(module: &Builder) -> Session {
    let mut session = Session::new(program(module)).unwrap();
    session.run(Command::Continue, 100_000);
    session
}

#[test]
fn fd_write_appends_to_standard_output_and_error() {
    let session = run(&hello(1));
    let machine = session.machine();
    assert_eq!(machine.status, Status::Finished(vec![Value::I32(0)]));
    assert_eq!(machine.host.output, [(Stream::Stdout, b"hello\n".to_vec())]);
    assert_eq!(
        &machine.memory.as_ref().unwrap().bytes[8..12],
        6_u32.to_le_bytes()
    );
    let session = run(&hello(2));
    assert_eq!(
        session.machine().host.output,
        [(Stream::Stderr, b"hello\n".to_vec())]
    );
    // Other descriptors are not open: EBADF.
    let session = run(&hello(5));
    assert_eq!(
        session.machine().status,
        Status::Finished(vec![Value::I32(8)])
    );
    assert_eq!(session.machine().host.output, []);
}

#[test]
fn a_host_call_is_one_step_with_a_note() {
    let mut session = Session::new(program(&hello(1))).unwrap();
    while !matches!(session.machine().event.effect, Effect::Host { .. }) {
        session.run(Command::Step, 1);
    }
    let Effect::Host { note, .. } = &session.machine().event.effect else {
        unreachable!();
    };
    assert_eq!(note, "wrote 6 bytes to standard output");
    assert_eq!(session.machine().event.popped.len(), 4);
    assert_eq!(session.machine().event.pushed, [Value::I32(0)]);
}

#[test]
fn a_bad_iovec_traps_like_any_access() {
    let mut module = Builder::new();
    module.import(WASI, "fd_write", &[I32, I32, I32, I32], &[I32]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[I32],
        &[],
        "i32.const 1 i32.const 65534 i32.const 1 i32.const 8 call fd_write",
    );
    let session = run(&module);
    assert!(matches!(
        &session.machine().status,
        Status::Trapped(trap) if trap.kind == crate::TrapKind::MemoryOutOfBounds
    ));
}

#[test]
fn proc_exit_ends_the_program_with_its_status() {
    let mut module = Builder::new();
    module.import(WASI, "proc_exit", &[I32], &[]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[],
        &[],
        "i32.const 3 call proc_exit unreachable",
    );
    assert_eq!(run(&module).machine().status, Status::Exited(3));
}

#[test]
fn arguments_environment_and_clock_are_fixed() {
    let mut module = Builder::new();
    module.import(WASI, "args_sizes_get", &[I32, I32], &[I32]);
    module.import(WASI, "args_get", &[I32, I32], &[I32]);
    module.import(WASI, "environ_sizes_get", &[I32, I32], &[I32]);
    module.import(WASI, "clock_time_get", &[I32, I64, I32], &[I32]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[],
        &[],
        "i32.const 0 i32.const 4 call args_sizes_get drop
         i32.const 8 i32.const 16 call args_get drop
         i32.const 32 i32.const 36 call environ_sizes_get drop
         i32.const 0 i64.const 1 i32.const 40 call clock_time_get drop",
    );
    let session = run(&module);
    let memory = &session.machine().memory.as_ref().unwrap().bytes;
    let word = |at: usize| u32::from_le_bytes(memory[at..at + 4].try_into().unwrap());
    assert_eq!((word(0), word(4)), (1, 8), "one argument, \"program\\0\"");
    assert_eq!(word(8), 16, "argv[0] points at the buffer");
    assert_eq!(&memory[16..24], b"program\0");
    assert_eq!((word(32), word(36)), (0, 0), "no environment");
    // The clock reads one microsecond per instruction executed before it.
    let time = u64::from_le_bytes(memory[40..48].try_into().unwrap());
    assert_eq!(time, 15 * 1_000);
}

#[test]
fn unresolved_imports_trap_only_when_called() {
    let mut module = Builder::new();
    module.import("env", "puts", &[I32], &[I32]);
    module.import(WASI, "fd_write", &[I32], &[]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[I32],
        &[],
        "i32.const 0 if i32 i32.const 0 call puts else i32.const 1 end",
    );
    let program = program(&module);
    assert_eq!(
        program.unresolved,
        ["env.puts", "wasi_snapshot_preview1.fd_write"],
        "a wrongly typed WASI import is unresolved too"
    );
    let session = run(&module);
    assert_eq!(
        session.machine().status,
        Status::Finished(vec![Value::I32(1)])
    );
    let mut module = Builder::new();
    module.import("env", "puts", &[I32], &[I32]);
    module.func("_start", &[], &[I32], &[], "i32.const 0 call puts");
    let session = run(&module);
    let Status::Trapped(trap) = &session.machine().status else {
        panic!("{:?}", session.machine().status);
    };
    assert_eq!(trap.kind, crate::TrapKind::Unresolved);
    assert!(trap.message.contains("`env.puts`"), "{}", trap.message);
}

#[test]
fn unimplemented_wasi_functions_answer_enosys() {
    let mut module = Builder::new();
    module.import(WASI, "path_open", &[I32, I32, I32], &[I32]);
    module.memory(1, None);
    module.func(
        "_start",
        &[],
        &[I32],
        &[],
        "i32.const 0 i32.const 0 i32.const 0 call path_open",
    );
    let program = program(&module);
    assert_eq!(program.unresolved, Vec::<String>::new());
    let mut session = Session::new(program).unwrap();
    session.run(Command::Continue, 100);
    assert_eq!(
        session.machine().status,
        Status::Finished(vec![Value::I32(52)])
    );
    // One that returns nothing cannot answer an error number.
    let mut module = Builder::new();
    module.import(WASI, "proc_raise_later", &[I32], &[]);
    module.func("_start", &[], &[], &[], "");
    assert_eq!(
        program_of(&module).unresolved,
        ["wasi_snapshot_preview1.proc_raise_later"]
    );
}

fn program_of(module: &Builder) -> Program {
    Program::load(&module.build(), None).unwrap()
}
