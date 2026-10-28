//! The engine Worker's WebAssembly session: load a module into the stepper
//! (`web/stepper/`), run the page's commands, and present each state as a
//! [`WasmSnapshot`].

use std::rc::Rc;

use wasm_stepper::{
    Command,
    Effect,
    Instr,
    LabelKind,
    LineMap,
    Machine,
    Program,
    Row,
    Session,
    Status,
    Stop,
    Stream,
    Value,
};

use super::{
    Access,
    BlockLabel,
    CallFrame,
    MemoryView,
    MemoryWindow,
    Operand,
    Output,
    Place,
    Slot,
    StackBand,
    WasmControl,
    WasmModule,
    WasmProgram,
    WasmRun,
    WasmSnapshot,
    WatLine,
    narration,
};

/// The most instructions one request runs before it pauses, so a runaway
/// loop never outlasts the page's request timeout.
const BUDGET: u64 = 2_000_000;
/// Bytes shown per memory window.
const WINDOW: u32 = 256;

#[derive(Default)]
pub struct WasmWorker {
    session: Option<Session>,
}

impl WasmWorker {
    /// Load `module`, replacing any previous program.
    ///
    /// # Errors
    /// A malformed or invalid module, or one the stepper cannot instantiate.
    pub fn load(&mut self, module: WasmModule) -> Result<WasmProgram, String> {
        self.session = None;
        let lines = module.lines.map(|lines| {
            LineMap::new(
                lines
                    .into_iter()
                    .map(|line| Row {
                        offset: line.offset,
                        line:   line.line,
                        column: line.column,
                    })
                    .collect(),
            )
        });
        let program = Rc::new(Program::load(&module.bytes, lines)?);
        let session = Session::new(Rc::clone(&program)).map_err(|error| error.to_string())?;
        let lines = program
            .wat
            .lines
            .iter()
            .map(|line| WatLine {
                offset: line.instr.map(|(_, _, offset)| offset),
                source: line
                    .instr
                    .and_then(|(func, pc, _)| program.line_of(func, pc)),
            })
            .collect();
        let result = WasmProgram {
            wat: program.wat.text.clone(),
            lines,
            producer: module.producer,
            command: module.command,
            has_lines: !program.lines.is_empty(),
            size: module.bytes.len(),
            unresolved: program.unresolved.clone(),
            snapshot: snapshot(&session, None),
        };
        self.session = Some(session);
        Ok(result)
    }

    /// Run one of the page's commands.
    ///
    /// # Errors
    /// When no program has been loaded.
    pub fn control(&mut self, control: WasmControl) -> Result<WasmSnapshot, String> {
        let session = self
            .session
            .as_mut()
            .ok_or("No WebAssembly program is loaded.")?;
        let stop = match control {
            | WasmControl::Run(run) => Some(session.run(command(run), BUDGET)),
            | WasmControl::GoTo(step) => {
                session.go_to(step);
                None
            },
            | WasmControl::Reset => {
                session.reset();
                None
            },
            | WasmControl::Breakpoints(offsets) => {
                session.set_breakpoints(offsets);
                None
            },
        };
        let stopped = match stop {
            | Some(Stop::Breakpoint) => Some("breakpoint".to_owned()),
            | Some(Stop::Budget) => Some("budget".to_owned()),
            | _ => None,
        };
        Ok(snapshot(session, stopped))
    }
}

fn command(run: WasmRun) -> Command {
    match run {
        | WasmRun::Step => Command::Step,
        | WasmRun::StepOver => Command::StepOver,
        | WasmRun::StepOut => Command::StepOut,
        | WasmRun::NextLine => Command::NextLine,
        | WasmRun::RunToLine(line) => Command::RunToLine(line),
        | WasmRun::Continue => Command::Continue,
    }
}

/// The page's view of the session's current state.
fn snapshot(session: &Session, stopped: Option<String>) -> WasmSnapshot {
    let program = session.program();
    let machine = session.machine();
    let place = |func: u32, pc: u32| {
        Some(Place {
            offset:   program.offset_of(func, pc)?,
            wat_line: program.wat.line_of(func, pc)?,
            source:   program.line_of(func, pc).map(|line| line as usize),
        })
    };
    let status = match &machine.status {
        | Status::Running if machine.steps >= wasm_stepper::MAX_STEPS => "limit",
        | Status::Running => "running",
        | Status::Finished(_) => "finished",
        | Status::Exited(_) => "exited",
        | Status::Trapped(_) => "trapped",
    };
    let results = match &machine.status {
        | Status::Finished(values) => values.iter().map(|value| operand(*value, false)).collect(),
        | _ => Vec::new(),
    };
    WasmSnapshot {
        step: machine.steps,
        status: status.to_owned(),
        stopped,
        next: machine.next().and_then(|(func, pc)| place(func, pc)),
        last: machine
            .event
            .pc
            .and_then(|pc| place(machine.event.func, pc)),
        narration: narration::narrate(session),
        stack: stack(machine).0,
        below: stack(machine).1,
        locals: locals(session),
        frames: frames(session),
        labels: labels(session),
        globals: globals(session),
        memory: memory(session),
        output: machine
            .host
            .output
            .iter()
            .map(|(stream, bytes)| Output {
                stream: match stream {
                    | Stream::Stdout => "stdout",
                    | Stream::Stderr => "stderr",
                }
                .to_owned(),
                text:   String::from_utf8_lossy(bytes).into_owned(),
            })
            .collect(),
        results,
    }
}

pub fn operand(value: Value, fresh: bool) -> Operand {
    Operand {
        ty: value.ty().name().to_owned(),
        text: value.to_string(),
        fresh,
    }
}

/// The current call's operands and how many belong to its callers.
fn stack(machine: &Machine) -> (Vec<Operand>, usize) {
    let base = machine
        .frames
        .last()
        .map_or(machine.stack.len(), |frame| frame.stack_base);
    let fresh = if machine.event.depth == machine.frames.len() {
        machine.event.pushed.len()
    } else {
        0
    };
    let fresh_from = machine.stack.len().saturating_sub(fresh);
    let operands = machine.stack[base..]
        .iter()
        .enumerate()
        .map(|(index, value)| operand(*value, base + index >= fresh_from))
        .collect();
    (operands, base)
}

fn locals(session: &Session) -> Vec<Slot> {
    let program = session.program();
    let machine = session.machine();
    let Some(frame) = machine.frames.last() else {
        return Vec::new();
    };
    let params = program
        .module
        .func_type(frame.func)
        .map_or(0, |ty| ty.params.len());
    let same_call = machine.event.func == frame.func && machine.event.depth == machine.frames.len();
    frame
        .locals
        .iter()
        .zip(0..)
        .map(|(value, index)| {
            let touched = match machine.event.effect {
                | Effect::LocalRead(local) if same_call && local == index => "read",
                | Effect::LocalWrite(local) if same_call && local == index => "write",
                | _ => "",
            };
            Slot {
                index,
                name: program
                    .module
                    .names
                    .local(frame.func, index)
                    .map(str::to_owned),
                ty: value.ty().name().to_owned(),
                text: value.to_string(),
                role: if (index as usize) < params {
                    "param"
                } else {
                    "local"
                }
                .to_owned(),
                touched: touched.to_owned(),
                note: None,
            }
        })
        .collect()
}

fn frames(session: &Session) -> Vec<CallFrame> {
    let program = session.program();
    let machine = session.machine();
    let count = machine.frames.len();
    machine
        .frames
        .iter()
        .enumerate()
        .rev()
        .map(|(index, frame)| {
            // A caller waits on the call before its next instruction.
            let pc = if index + 1 == count {
                frame.pc
            } else {
                frame.pc.saturating_sub(1)
            };
            CallFrame {
                name:          program.module.func_name(frame.func),
                wat_line:      program.wat.line_of(frame.func, pc).unwrap_or(0),
                source:        program.line_of(frame.func, pc).map(|line| line as usize),
                stack_pointer: frame.stack_pointer,
            }
        })
        .collect()
}

fn labels(session: &Session) -> Vec<BlockLabel> {
    let program = session.program();
    let machine = session.machine();
    let Some(frame) = machine.frames.last() else {
        return Vec::new();
    };
    machine.labels[frame.label_base..]
        .iter()
        .enumerate()
        .rev()
        .map(|(name, label)| BlockLabel {
            kind: label.kind.name().to_owned(),
            name,
            start: if label.kind == LabelKind::Function {
                program.wat.header_of(frame.func).unwrap_or(0)
            } else {
                program.wat.line_of(frame.func, label.start).unwrap_or(0)
            },
            end: program.wat.line_of(frame.func, label.end).unwrap_or(0),
            arity: label.arity,
        })
        .collect()
}

fn globals(session: &Session) -> Vec<Slot> {
    let program = session.program();
    let machine = session.machine();
    machine
        .globals
        .iter()
        .zip(0..)
        .map(|(value, index)| {
            let mutable = program
                .module
                .globals
                .get(index as usize)
                .is_some_and(|global| global.ty.mutable);
            let touched = match machine.event.effect {
                | Effect::GlobalRead(global) if global == index => "read",
                | Effect::GlobalWrite(global) if global == index => "write",
                | _ => "",
            };
            let name = program.module.global_name(index).map(str::to_owned);
            Slot {
                index,
                note: name.as_deref().and_then(global_note).map(str::to_owned),
                name,
                ty: value.ty().name().to_owned(),
                text: value.to_string(),
                role: if mutable { "mut" } else { "const" }.to_owned(),
                touched: touched.to_owned(),
            }
        })
        .collect()
}

/// What the linker's globals are for.
fn global_note(name: &str) -> Option<&'static str> {
    Some(match name {
        | "__stack_pointer" => "C's shadow stack pointer",
        | "__stack_low" => "lowest shadow stack address",
        | "__stack_high" => "the shadow stack's top",
        | "__data_end" => "end of static data",
        | "__heap_base" => "where a heap would start",
        | "__heap_end" => "end of the heap",
        | "__memory_base" | "__table_base" => "base for position-independent code",
        | "__tls_base" => "thread-local storage",
        | "__global_base" => "where static data starts",
        | "__dso_handle" => "this module's handle",
        | "__rodata_start" | "__rodata_end" => "read-only data bounds",
        | _ => return None,
    })
}

/// The shadow stack pointer's initial value: the stack's top.
pub fn stack_top(program: &Program, machine: &Machine) -> Option<u32> {
    let index = machine.stack_pointer? as usize;
    match program.module.globals.get(index)?.init.code.first()? {
        | Instr::I32Const(value) => Some(value.cast_unsigned()),
        | _ => None,
    }
}

#[expect(clippy::too_many_lines, reason = "One block per memory window.")]
fn memory(session: &Session) -> MemoryView {
    let program = session.program();
    let machine = session.machine();
    let Some(memory) = &machine.memory else {
        return MemoryView::default();
    };
    let size = u32::try_from(memory.bytes.len()).unwrap_or(u32::MAX);
    let window = |key: &str, title: &str, start: u32, end: u32, note: String| {
        let start = start.min(size);
        let end = end.clamp(start, size);
        MemoryWindow {
            key: key.to_owned(),
            title: title.to_owned(),
            start,
            bytes: memory.bytes[start as usize..end as usize].to_vec(),
            note,
        }
    };
    let mut windows = Vec::new();
    let stack_pointer = machine.stack_pointer_value();
    let stack_top = stack_top(program, machine);
    if let (Some(pointer), Some(top)) = (stack_pointer, stack_top) {
        let used = top.saturating_sub(pointer);
        // A little below the pointer too: a leaf function's frame lives
        // there without moving it.
        let start = (pointer.saturating_sub(32) & !15).min(top.saturating_sub(64) & !15);
        let note = if used == 0 {
            "The shadow stack is empty: no C function has a frame in memory yet.".to_owned()
        } else {
            format!(
                "{used} bytes in use, from __stack_pointer ({pointer}) up to the top ({top}). It \
                 grows down."
            )
        };
        windows.push(window(
            "stack",
            "Shadow stack",
            start,
            top.min(start + WINDOW),
            note,
        ));
    }
    let data: Vec<(u32, u32)> = program
        .module
        .data
        .iter()
        .filter_map(|data| match &data.mode {
            | wasm_stepper::SegmentMode::Active { offset, .. } => match offset.code.first() {
                | Some(Instr::I32Const(at)) => Some((
                    at.cast_unsigned(),
                    at.cast_unsigned()
                        .saturating_add(u32::try_from(data.bytes.len()).unwrap_or(u32::MAX)),
                )),
                | _ => None,
            },
            | _ => None,
        })
        .collect();
    if let (Some(start), Some(end)) = (
        data.iter().map(|(start, _)| *start).min(),
        data.iter().map(|(_, end)| *end).max(),
    ) {
        windows.push(window(
            "data",
            "Static data",
            start & !15,
            end.min((start & !15) + WINDOW),
            format!(
                "{} bytes of data segments, copied in when the module was instantiated: C's \
                 string literals and initialized globals.",
                end - start
            ),
        ));
    }
    let access = match machine.event.effect {
        | Effect::Load { address, size } => Some(Access {
            address,
            size: u64::from(size),
            write: false,
        }),
        | Effect::Store { address, size } => Some(Access {
            address,
            size: u64::from(size),
            write: true,
        }),
        | Effect::Bulk {
            destination,
            length,
            ..
        } => Some(Access {
            address: destination,
            size:    length,
            write:   true,
        }),
        | _ => None,
    };
    if let Some(access) = access.filter(|access| access.address < u64::from(size)) {
        let start = (u32::try_from(access.address).unwrap_or(u32::MAX) & !15).saturating_sub(16);
        windows.push(window(
            "access",
            "Last access",
            start,
            start + 64,
            format!(
                "The {} bytes at {} that the last instruction {}.",
                access.size,
                access.address,
                if access.write { "wrote" } else { "read" }
            ),
        ));
    }
    let mut bands = Vec::new();
    if let Some(pointer) = stack_pointer {
        let mut below = pointer;
        for frame in machine.frames.iter().rev() {
            if let Some(entry) = frame.stack_pointer {
                if entry > below {
                    bands.push(StackBand {
                        name:  program.module.func_name(frame.func),
                        start: below,
                        end:   entry,
                    });
                }
                below = below.max(entry);
            }
        }
    }
    MemoryView {
        pages: memory.pages(),
        stack_pointer,
        stack_top,
        windows,
        access,
        bands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        demos::DEMOS,
        wasm::{
            Precompiled,
            WasmBackEnd,
        },
    };

    const CATEGORIES: [&str; 7] = [
        "evaluate", "convert", "memory", "lifetime", "control", "call", "fault",
    ];

    /// Every snapshot of `source`'s module, stepping one instruction at a
    /// time to the end.
    fn trace(source: &str) -> (WasmProgram, Vec<WasmSnapshot>) {
        let mut worker = WasmWorker::default();
        let program = worker.load(Precompiled.compile(source).unwrap()).unwrap();
        let mut snapshots = vec![program.snapshot.clone()];
        while snapshots.last().unwrap().can_step() {
            snapshots.push(worker.control(WasmControl::Run(WasmRun::Step)).unwrap());
            assert!(snapshots.len() < 20_000, "runaway");
        }
        (program, snapshots)
    }

    #[test]
    fn every_lesson_steps_to_its_end_with_a_narrated_placed_state() {
        for demo in &DEMOS {
            let (program, snapshots) = trace(demo.source);
            let lines = program.wat.lines().count();
            assert_eq!(program.lines.len(), lines, "{}", demo.name);
            assert!(program.has_lines, "{}: DWARF line table", demo.name);
            let source_lines = demo.source.lines().count();
            for snapshot in &snapshots {
                let narration = &snapshot.narration;
                let at = format!("{} step {}", demo.name, snapshot.step);
                assert!(!narration.title.is_empty(), "{at}");
                assert!(
                    !narration.explanation.is_empty(),
                    "{at}: {}",
                    narration.title
                );
                assert!(CATEGORIES.contains(&narration.category.as_str()), "{at}");
                assert!(
                    !narration.concepts.is_empty() || narration.title == "Do nothing",
                    "{at}: {}",
                    narration.title
                );
                for place in snapshot.next.iter().chain(&snapshot.last) {
                    assert!((1..=lines).contains(&place.wat_line), "{at}");
                    assert!(
                        place
                            .source
                            .is_none_or(|line| (1..=source_lines).contains(&line)),
                        "{at}"
                    );
                }
                assert_eq!(
                    snapshot.narration.title.matches('`').count() % 2,
                    0,
                    "{at}: unbalanced code span in {}",
                    snapshot.narration.title
                );
            }
            let last = snapshots.last().unwrap();
            assert_eq!(last.status, "finished", "{}", demo.name);
            assert_eq!(last.results.len(), 1, "{}", demo.name);
            assert!(
                last.narration
                    .explanation
                    .contains("the program is finished"),
                "{}",
                demo.name
            );
        }
    }

    #[test]
    fn the_recursion_lesson_narrates_frames_calls_and_returns() {
        let recursion = DEMOS
            .iter()
            .find(|demo| demo.name == "recursion.c")
            .unwrap();
        let (_, snapshots) = trace(recursion.source);
        let titles: Vec<&str> = snapshots
            .iter()
            .map(|s| s.narration.title.as_str())
            .collect();
        for expected in [
            "The module is instantiated; `$__original_main` is about to run",
            "Read the shadow stack pointer: `65536`",
            "Reserve 16 bytes of shadow stack",
            "Call `$factorial` with `4`",
            "`$factorial` returns `1` to `$factorial`",
            "`$factorial` returns `24` to `$__original_main`",
            "Release the shadow stack frame",
            "`$__original_main` returns `24` to the host",
        ] {
            assert!(
                titles.contains(&expected),
                "{expected:?} not in {titles:#?}"
            );
        }
        // Four nested calls of factorial, each with its own frame and band.
        let deepest = snapshots.iter().map(|s| s.frames.len()).max().unwrap();
        assert_eq!(deepest, 5, "main and factorial(4..1)");
        let banded = snapshots
            .iter()
            .map(|s| s.memory.bands.len())
            .max()
            .unwrap();
        assert_eq!(banded, 5);
        // A load inside factorial's frame says so.
        assert!(snapshots.iter().any(|s| {
            s.narration
                .explanation
                .contains("into this call's shadow stack frame")
        }));
        // Branches name their label and say how depth counts.
        assert!(
            snapshots
                .iter()
                .any(|s| s.narration.title.contains("block @2"))
        );
    }

    #[test]
    fn overflow_wraps_where_c_is_undefined() {
        let overflow = DEMOS.iter().find(|demo| demo.name == "overflow.c").unwrap();
        let (_, snapshots) = trace(overflow.source);
        let wrapped = snapshots
            .iter()
            .find(|s| {
                s.narration
                    .explanation
                    .contains("wraps around, keeping the low bits")
            })
            .expect("the overflowing add is narrated");
        assert_eq!(
            wrapped.narration.title,
            "`2147483647` + `1` = `-2147483648`"
        );
        assert_eq!(snapshots.last().unwrap().results[0].text, "-2147483647");
    }

    #[test]
    fn going_back_and_breakpoints_answer_with_the_right_state() {
        let mut worker = WasmWorker::default();
        let demo = &DEMOS[0];
        let program = worker
            .load(Precompiled.compile(demo.source).unwrap())
            .unwrap();
        let mut forward = vec![program.snapshot.clone()];
        for _ in 0..40 {
            forward.push(worker.control(WasmControl::Run(WasmRun::Step)).unwrap());
        }
        let back = worker.control(WasmControl::GoTo(17)).unwrap();
        assert_eq!(back, forward[17]);
        let reset = worker.control(WasmControl::Reset).unwrap();
        assert_eq!(reset, forward[0]);
        // A breakpoint on bump's first instruction stops each call.
        let line = program
            .wat
            .lines()
            .position(|line| line.starts_with("  (func $bump"))
            .unwrap();
        let first = program.lines[line + 2]
            .offset
            .expect("bump's first instruction");
        worker
            .control(WasmControl::Breakpoints(vec![first]))
            .unwrap();
        let mut hits = 0;
        loop {
            let snapshot = worker.control(WasmControl::Run(WasmRun::Continue)).unwrap();
            if snapshot.stopped.as_deref() == Some("breakpoint") {
                hits += 1;
                assert_eq!(snapshot.next.unwrap().offset, first);
                assert_eq!(snapshot.frames[0].name, "bump");
            } else {
                assert_eq!(snapshot.status, "finished");
                break;
            }
        }
        assert_eq!(hits, 3, "bump runs once per element");
    }

    #[test]
    fn the_memory_view_shows_the_stack_and_marks_accesses() {
        let mut worker = WasmWorker::default();
        let demo = DEMOS.iter().find(|demo| demo.name == "pointers.c").unwrap();
        worker
            .load(Precompiled.compile(demo.source).unwrap())
            .unwrap();
        let mut stored = None;
        for _ in 0..200 {
            let snapshot = worker.control(WasmControl::Run(WasmRun::Step)).unwrap();
            if snapshot.memory.access.is_some_and(|access| access.write) {
                stored = Some(snapshot);
                break;
            }
        }
        let snapshot = stored.expect("main stores into its frame");
        let memory = &snapshot.memory;
        let access = memory.access.unwrap();
        let pointer = memory.stack_pointer.unwrap();
        assert!(
            access.address >= u64::from(pointer),
            "a store into the frame"
        );
        let stack = memory.windows.iter().find(|w| w.key == "stack").unwrap();
        assert!((stack.start..stack.start + 256).contains(&pointer));
        assert!(stack.bytes.len() <= 256);
        assert!(memory.windows.iter().any(|w| w.key == "access"));
        assert_eq!(memory.bands[0].start, pointer);
    }

    #[test]
    fn a_trap_stops_the_program_and_is_explained() {
        // (module (func (export "main") unreachable))
        let bytes = [
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
            0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type: [] -> []
            0x03, 0x02, 0x01, 0x00, // function 0 has type 0
            0x07, 0x08, 0x01, 0x04, b'm', b'a', b'i', b'n', 0x00, 0x00, // export "main"
            0x0A, 0x05, 0x01, 0x03, 0x00, 0x00, 0x0B, // code: unreachable
        ];
        let mut worker = WasmWorker::default();
        let program = worker
            .load(WasmModule {
                bytes: bytes.to_vec(),
                ..WasmModule::default()
            })
            .unwrap();
        assert!(!program.has_lines);
        assert!(program.wat.contains("    unreachable"), "{}", program.wat);
        let trapped = worker.control(WasmControl::Run(WasmRun::Step)).unwrap();
        assert_eq!(trapped.status, "trapped");
        assert!(!trapped.can_step());
        assert_eq!(trapped.narration.category, "fault");
        assert_eq!(trapped.narration.title, "Trap: unreachable executed");
        assert!(
            trapped
                .narration
                .explanation
                .contains("stops the program immediately")
        );
        // Going back still works after a trap.
        assert_eq!(
            worker.control(WasmControl::GoTo(0)).unwrap(),
            program.snapshot
        );
        // A malformed module is an error, not a panic.
        let error = worker
            .load(WasmModule {
                bytes: b"\0asm\x01\0\0\0\x01".to_vec(),
                ..WasmModule::default()
            })
            .unwrap_err();
        assert!(error.contains("unexpected end of the module"), "{error}");
    }

    /// The precompiled modules must come from the lessons' current source:
    /// every function a lesson defines is in its module, and every C line
    /// the module's DWARF names holds code. `web/wasm/generate.sh --check`
    /// compares the bytes exactly, given a wasm32 clang.
    #[test]
    fn the_precompiled_modules_match_the_lessons() {
        for demo in &DEMOS {
            let mut worker = WasmWorker::default();
            let program = worker
                .load(Precompiled.compile(demo.source).unwrap())
                .unwrap();
            let lines: Vec<&str> = demo.source.lines().collect();
            for line in program.lines.iter().filter_map(|line| line.source) {
                let text = lines[line as usize - 1].trim();
                assert!(
                    !text.is_empty() && !text.starts_with("//"),
                    "{}: the module maps code to line {line}, {text:?}; regenerate web/wasm",
                    demo.name
                );
            }
            for function in demo
                .source
                .lines()
                .filter_map(|line| line.strip_prefix("int "))
                .filter_map(|rest| rest.split_once('(').map(|(name, _)| name))
            {
                let name = if function == "main" {
                    "__original_main"
                } else {
                    function
                };
                assert!(
                    program.wat.contains(&format!("(func ${name} ")),
                    "{}: {name} is missing; regenerate web/wasm",
                    demo.name
                );
            }
        }
    }
}
