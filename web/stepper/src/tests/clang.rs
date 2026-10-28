//! The lesson modules clang built (`web/wasm/`, see its `generate.sh`): each
//! loads, maps its code to C lines through DWARF, and runs to the value C
//! gives it, or, for the lessons whose C has undefined behavior, to what
//! WebAssembly's fully defined semantics make of clang's translation.

use std::rc::Rc;

use crate::{
    Command,
    Program,
    Session,
    Status,
    Stop,
    Value,
};

fn lesson(name: &str) -> Rc<Program> {
    let path = format!("{}/../wasm/{name}.wasm", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    Rc::new(Program::load(&bytes, None).unwrap_or_else(|error| panic!("{name}: {error}")))
}

fn result(name: &str) -> Value {
    let mut session = Session::new(lesson(name)).unwrap();
    assert_eq!(
        session.run(Command::Continue, 1_000_000),
        Stop::Ended,
        "{name}"
    );
    match &session.machine().status {
        | Status::Finished(values) if values.len() == 1 => values[0],
        | other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn every_lesson_runs_to_its_result() {
    assert_eq!(result("objects"), Value::I32(15));
    assert_eq!(result("pointers"), Value::I32(17));
    assert_eq!(result("recursion"), Value::I32(24));
    assert_eq!(result("promotion"), Value::I32(255));
    // Signed overflow is undefined in C; clang's i32.add wraps.
    assert_eq!(result("overflow"), Value::I32(i32::MIN + 1));
    // Reading past the array or an uninitialized object is undefined in C;
    // in WebAssembly it reads whatever linear memory holds.
    for name in ["bounds", "indeterminate"] {
        let mut session = Session::new(lesson(name)).unwrap();
        session.run(Command::Continue, 1_000_000);
        assert!(
            matches!(session.machine().status, Status::Finished(_)),
            "{name}: {:?}",
            session.machine().status
        );
    }
}

#[test]
fn dwarf_maps_instructions_to_c_lines() {
    let program = lesson("recursion");
    assert!(!program.lines.is_empty());
    let factorial = program
        .module
        .export("factorial", crate::ExternKind::Func)
        .unwrap();
    let body = program.module.body(factorial).unwrap();
    let lines: Vec<u32> = (0..body.code.len() as u32)
        .filter_map(|pc| program.line_of(factorial, pc))
        .collect();
    // factorial spans lines 2-8 of recursion.c.
    assert!(lines.iter().all(|line| (2..=8).contains(line)), "{lines:?}");
    for line in [2, 3, 4, 6, 7, 8] {
        assert!(lines.contains(&line), "line {line} in {lines:?}");
    }
    // Stepping by line visits the source in order.
    let mut session = Session::new(Rc::clone(&program)).unwrap();
    let mut visited = vec![session.next_line()];
    while session.run(Command::NextLine, 10_000) == Stop::Done {
        visited.push(session.next_line());
    }
    let visited: Vec<u32> = visited.into_iter().flatten().collect();
    assert_eq!(&visited[..2], [10, 11], "{visited:?}");
    assert_eq!(
        program.module.func_name(session.program().entry.unwrap()),
        "__original_main",
        "clang's main(void), not its line-less main(argc, argv) wrapper"
    );
    assert!(visited.contains(&12), "{visited:?}");
}

#[test]
fn clangs_shadow_stack_is_found_and_tracked() {
    let program = lesson("recursion");
    let mut session = Session::new(Rc::clone(&program)).unwrap();
    let initial = session
        .machine()
        .stack_pointer_value()
        .expect("__stack_pointer");
    // Into factorial(4), past its prologue.
    session.run(Command::RunToLine(3), 10_000);
    let machine = session.machine();
    let frame = machine.frames.last().unwrap();
    assert_eq!(program.module.func_name(frame.func), "factorial");
    let now = machine.stack_pointer_value().unwrap();
    assert!(
        now < initial,
        "the shadow stack grows down: {now} < {initial}"
    );
    assert!(frame.stack_pointer.unwrap() > now, "factorial lowered it");
    assert!(program.wat.text.contains("call $factorial"));
    assert_eq!(program.unresolved, Vec::<String>::new());
}
