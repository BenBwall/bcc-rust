//! The session's commands, going back, breakpoints, and the events each
//! step records.

use std::rc::Rc;

use super::{
    Builder,
    program,
};
use crate::{
    Command,
    Effect,
    LabelKind,
    LineMap,
    Program,
    Row,
    Session,
    Status,
    Stop,
    ValType::I32,
    Value,
};

/// `fact(n)` by recursion and `main` calling `fact(3)`.
fn factorial() -> Builder {
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
    module.func("main", &[], &[I32], &[], "i32.const 3 call fact");
    module
}

fn session(module: &Builder) -> Session {
    Session::new(program(module)).unwrap()
}

/// What a test compares between two machines.
type State = (
    u64,
    Vec<Value>,
    Vec<crate::Frame>,
    Vec<crate::Label>,
    Vec<Value>,
    Status,
    crate::Event,
);

fn state(session: &Session) -> State {
    let machine = session.machine();
    (
        machine.steps,
        machine.stack.clone(),
        machine.frames.clone(),
        machine.labels.clone(),
        machine.globals.clone(),
        machine.status.clone(),
        machine.event.clone(),
    )
}

#[test]
fn a_session_opens_on_the_entry_points_first_instruction() {
    let session = session(&factorial());
    let machine = session.machine();
    assert_eq!(machine.steps, 0);
    let main = session
        .program()
        .module
        .export("main", crate::ExternKind::Func)
        .unwrap();
    assert_eq!(machine.next(), Some((main, 0)));
    assert_eq!(machine.event.effect, Effect::Start { func: main });
}

#[test]
fn each_step_records_what_it_popped_and_pushed() {
    let mut module = Builder::new();
    module.func(
        "main",
        &[],
        &[I32],
        &[I32],
        "i32.const 2 i32.const 3 i32.add local.tee 0 block br 0 end",
    );
    let mut session = session(&module);
    session.run(Command::Step, 1);
    assert_eq!(session.machine().event.pushed, [Value::I32(2)]);
    session.run(Command::Step, 1);
    session.run(Command::Step, 1);
    let event = &session.machine().event;
    assert_eq!(
        event.popped,
        [Value::I32(2), Value::I32(3)],
        "deepest first"
    );
    assert_eq!(event.pushed, [Value::I32(5)]);
    session.run(Command::Step, 1);
    assert_eq!(session.machine().event.effect, Effect::LocalWrite(0));
    assert_eq!(session.machine().frames[0].locals, [Value::I32(5)]);
    session.run(Command::Step, 1);
    assert_eq!(
        session.machine().event.effect,
        Effect::Enter {
            kind:  LabelKind::Block,
            taken: true,
        }
    );
    assert_eq!(
        session.machine().labels.len(),
        2,
        "the function's and the block's"
    );
    session.run(Command::Step, 1);
    assert!(matches!(
        session.machine().event.effect,
        Effect::Branch {
            depth: 0,
            taken: true,
            kind: LabelKind::Block,
            ..
        }
    ));
    assert_eq!(session.machine().labels.len(), 1);
    session.run(Command::Step, 1);
    assert!(matches!(
        session.machine().event.effect,
        Effect::Return { .. }
    ));
    assert_eq!(
        session.machine().status,
        Status::Finished(vec![Value::I32(5)])
    );
}

#[test]
fn step_over_runs_a_call_to_its_return() {
    let mut over = session(&factorial());
    // `i32.const 3`, then `call fact` is next.
    over.run(Command::Step, 1);
    assert_eq!(over.run(Command::StepOver, 10_000), Stop::Done);
    let machine = over.machine();
    assert_eq!(machine.frames.len(), 1);
    assert_eq!(machine.stack, [Value::I32(6)]);
    // Over a non-call, it is one step.
    let mut single = session(&factorial());
    single.run(Command::StepOver, 10_000);
    assert_eq!(single.machine().steps, 1);
}

#[test]
fn step_out_returns_to_the_caller() {
    let mut session = session(&factorial());
    session.run(Command::Step, 1);
    session.run(Command::Step, 1);
    assert_eq!(session.machine().frames.len(), 2, "inside fact(3)");
    // Into fact(2).
    while session.machine().frames.len() < 3 {
        session.run(Command::Step, 1);
    }
    assert_eq!(session.run(Command::StepOut, 10_000), Stop::Done);
    let machine = session.machine();
    assert_eq!(machine.frames.len(), 2, "back in fact(3)");
    assert_eq!(
        machine.stack.last(),
        Some(&Value::I32(2)),
        "fact(2) returned 2"
    );
}

#[test]
fn going_back_replays_to_the_same_state() {
    let mut module = Builder::new();
    // A loop long enough to need several checkpoints.
    module.func(
        "main",
        &[],
        &[I32],
        &[I32],
        "loop local.get 0 i32.const 1 i32.add local.tee 0 i32.const 3000 i32.lt_s br_if 0 end
         local.get 0",
    );
    let mut forward = session(&module);
    let mut states = Vec::new();
    while forward.machine().status == Status::Running {
        states.push(state(&forward));
        forward.run(Command::Step, 1);
    }
    states.push(state(&forward));
    assert!(states.len() > 18_000, "{}", states.len());
    let mut replayed = session(&module);
    replayed.run(Command::Continue, u64::MAX);
    let session = &mut replayed;
    for target in [17_000, 3, 0, 9_999, 10_000, 1, 18_001, 4_096] {
        session.go_to(target);
        assert_eq!(state(session), states[target as usize], "step {target}");
    }
    // Past the end, the program stays finished.
    session.go_to(1 << 40);
    assert_eq!(state(session), *states.last().unwrap());
}

#[test]
fn reset_returns_to_step_zero() {
    let mut session = session(&factorial());
    let start = state(&session);
    session.run(Command::Continue, u64::MAX);
    session.reset();
    assert_eq!(state(&session), start);
}

#[test]
fn breakpoints_stop_before_their_instruction() {
    let mut session = session(&factorial());
    let program = Rc::clone(session.program());
    let fact = program
        .module
        .export("fact", crate::ExternKind::Func)
        .unwrap();
    // The `i32.mul` after the recursive call.
    let body = program.module.body(fact).unwrap();
    let mul = body
        .code
        .iter()
        .position(|instr| *instr == crate::Instr::Numeric(0x6C))
        .unwrap() as u32;
    session.set_breakpoints([program.offset_of(fact, mul).unwrap()]);
    assert_eq!(session.run(Command::Continue, u64::MAX), Stop::Breakpoint);
    assert_eq!(session.machine().next(), Some((fact, mul)));
    // The innermost multiplication is fact(2)'s: 2 * fact(1).
    assert_eq!(
        session.machine().stack[session.machine().stack.len() - 2..],
        [Value::I32(2), Value::I32(1)]
    );
    // Continuing from a breakpoint runs past it, to the next hit.
    assert_eq!(session.run(Command::Continue, u64::MAX), Stop::Breakpoint);
    assert_eq!(session.run(Command::Continue, u64::MAX), Stop::Ended);
    assert_eq!(
        session.machine().status,
        Status::Finished(vec![Value::I32(6)])
    );
}

#[test]
fn a_budget_pauses_a_long_run() {
    let mut module = Builder::new();
    module.func("main", &[], &[], &[], "loop br 0 end");
    let mut session = session(&module);
    assert_eq!(session.run(Command::Continue, 500), Stop::Budget);
    assert_eq!(session.machine().steps, 500);
}

/// `main` with three instructions per "line": lines 10, 11, 10, 12.
fn lined() -> Rc<Program> {
    let mut module = Builder::new();
    module.func(
        "helper",
        &[],
        &[I32],
        &[],
        "i32.const 1 i32.const 2 i32.add",
    );
    module.func(
        "main",
        &[],
        &[I32],
        &[],
        "i32.const 4 drop nop call helper drop nop i32.const 5 drop nop i32.const 6",
    );
    let probe = Program::load(&module.build(), None).unwrap();
    let main = probe
        .module
        .export("main", crate::ExternKind::Func)
        .unwrap();
    let helper = probe
        .module
        .export("helper", crate::ExternKind::Func)
        .unwrap();
    let offset = |func, pc| probe.offset_of(func, pc).unwrap();
    let rows = vec![
        Row {
            offset: offset(helper, 0),
            line:   2,
            column: 0,
        },
        Row {
            offset: offset(main, 0),
            line:   10,
            column: 0,
        },
        Row {
            offset: offset(main, 3),
            line:   11,
            column: 0,
        },
        Row {
            offset: offset(main, 6),
            line:   10,
            column: 0,
        },
        Row {
            offset: offset(main, 9),
            line:   12,
            column: 0,
        },
    ];
    Rc::new(Program::load(&module.build(), Some(LineMap::new(rows))).unwrap())
}

#[test]
fn next_line_runs_to_the_next_source_line_over_calls() {
    let program = lined();
    let mut session = Session::new(Rc::clone(&program)).unwrap();
    assert_eq!(session.next_line(), Some(10));
    assert_eq!(session.run(Command::NextLine, 1000), Stop::Done);
    assert_eq!(session.next_line(), Some(11));
    assert_eq!(session.machine().steps, 3);
    // Line 11 calls helper (line 2); next line steps over it.
    assert_eq!(session.run(Command::NextLine, 1000), Stop::Done);
    assert_eq!(session.next_line(), Some(10));
    assert_eq!(session.machine().frames.len(), 1);
    assert_eq!(session.run(Command::NextLine, 1000), Stop::Done);
    assert_eq!(session.next_line(), Some(12));
}

#[test]
fn run_to_line_stops_where_execution_enters_it() {
    let program = lined();
    let mut session = Session::new(program).unwrap();
    assert_eq!(session.run(Command::RunToLine(12), 1000), Stop::Done);
    assert_eq!(session.next_line(), Some(12));
    // From line 10, running to line 10 waits until it is entered again.
    let mut session = Session::new(lined()).unwrap();
    assert_eq!(session.run(Command::RunToLine(10), 1000), Stop::Done);
    assert_eq!(
        session.machine().steps,
        6 + 4,
        "main's first six and helper's four"
    );
    // A line that never runs ends with the program.
    let mut session = Session::new(lined()).unwrap();
    assert_eq!(session.run(Command::RunToLine(99), 1000), Stop::Ended);
}

#[test]
fn line_maps_cover_from_each_row_to_the_next() {
    let map = LineMap::new(vec![
        Row {
            offset: 20,
            line:   0,
            column: 0,
        },
        Row {
            offset: 10,
            line:   3,
            column: 5,
        },
    ]);
    assert_eq!(map.line_at(9), None);
    assert_eq!(map.line_at(10), Some(3));
    assert_eq!(map.line_at(19), Some(3));
    assert_eq!(map.line_at(20), None, "line 0 is no line");
}
