//! A steppable WebAssembly interpreter for the bcc site's WebAssembly view.
//!
//! A [`Program`] is a decoded, validated module with its text-format
//! listing and an optional map from instructions to C source lines. A
//! [`Session`] runs it one instruction at a time and answers the questions
//! a debugger's buttons ask: step, step over a call, step out of one, run
//! to the next source line, to a given line or a breakpoint, or to the end;
//! go back to any earlier step; reset. After each step the [`Machine`] holds
//! everything the page shows (operand stack, locals, labels, frames,
//! globals, linear memory) and an [`Event`] saying what the instruction did.
//!
//! Execution is deterministic, so going back re-executes from the nearest
//! checkpoint: the session keeps a copy of the machine every few thousand
//! steps (fewer, further apart, as a run grows).
//!
//! Example: `(func (result i32) i32.const 2 i32.const 3 i32.add)` takes four
//! steps: two pushes, the add (popping 2 and 3, pushing 5), and the `end`
//! that returns 5 to the host, which finishes the program.
//!
//! Reading order: [`Session::run`] here, then [`Machine::step`] in
//! `exec.rs`.
//!
//! Files:
//! - The binary format: `decode.rs` (with `decode/` for its reader, code and
//!   name sections), `module.rs` (the decoded structure), `instr.rs`.
//! - Checking: `validate.rs`.
//! - Running: `exec.rs` (with `exec/` for instantiation, numerics, memory and
//!   tables), `value.rs`, and the WASI host in `wasi.rs`.
//! - Presenting: `wat.rs` (the listing) and `lines.rs` (source lines, including
//!   DWARF's).
//!
//! The citations name sections of the WebAssembly 2.0 core specification
//! (W3C Recommendation, <https://www.w3.org/TR/wasm-core-2/>), whose
//! numbering these modules follow.

// The format
mod decode;
mod instr;
mod module;
// Checking and running
mod exec;
mod validate;
mod value;
mod wasi;
// Presenting
mod lines;
mod wat;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    rc::Rc,
};

pub use decode::DecodeError;
pub use exec::{
    Effect,
    Event,
    Frame,
    InstantiateError,
    Label,
    LabelKind,
    MAX_FRAMES,
    Machine,
    Memory,
    Status,
    Trap,
    TrapKind,
};
pub use instr::{
    Instr,
    Load,
    MemArg,
    Store,
    numeric_name,
    trunc_sat_name,
};
pub use lines::{
    LineMap,
    Row,
};
pub use module::{
    BlockType,
    Body,
    ExternKind,
    FuncKind,
    FuncType,
    ImportKind,
    Module,
    SegmentMode,
    ValType,
};
pub use value::Value;
pub use wasi::{
    Host,
    Stream,
};
pub use wat::{
    Wat,
    WatLine,
};

/// The most instructions one session executes.
pub const MAX_STEPS: u64 = 50_000_000;

/// How far one run request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// One instruction.
    Step,
    /// One instruction; a call runs to its return.
    StepOver,
    /// Until the current function returns.
    StepOut,
    /// Until the next instruction is on another source line, in this call
    /// or its caller (calls run through).
    NextLine,
    /// Until execution enters `line`.
    RunToLine(u32),
    /// Until the program stops or reaches a breakpoint.
    Continue,
}

/// Why a run request stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The command did what it asked.
    Done,
    /// The next instruction has a breakpoint.
    Breakpoint,
    /// The program finished, exited or trapped.
    Ended,
    /// The request's instruction budget ran out first.
    Budget,
    /// The session reached [`MAX_STEPS`].
    Limit,
}

/// A program and its execution.
pub struct Session {
    program:     Rc<Program>,
    machine:     Machine,
    /// Copies of the machine at steps 0, `interval`, `2 * interval`, …
    checkpoints: Vec<Machine>,
    interval:    u64,
    /// Module offsets of instructions to stop before.
    breakpoints: BTreeSet<u32>,
}

/// Checkpoints kept before they are thinned.
const CHECKPOINTS: usize = 48;

impl Session {
    /// Run `command`, executing at most `budget` instructions.
    pub fn run(&mut self, command: Command, budget: u64) -> Stop {
        let depth = self.machine.frames.len();
        let start_line = self.next_line();
        let over_call = command == Command::StepOver
            && self
                .next_instr()
                .is_some_and(|instr| matches!(instr, Instr::Call(_) | Instr::CallIndirect { .. }));
        let mut previous_line = start_line;
        let mut executed = 0;
        loop {
            if self.machine.status != Status::Running {
                return Stop::Ended;
            }
            if executed > 0 && self.at_breakpoint() {
                return Stop::Breakpoint;
            }
            if self.machine.steps >= MAX_STEPS {
                return Stop::Limit;
            }
            if executed >= budget {
                return Stop::Budget;
            }
            self.step();
            executed += 1;
            if self.machine.status != Status::Running {
                return Stop::Ended;
            }
            let frames = self.machine.frames.len();
            let line = self.next_line();
            let done = match command {
                | Command::Step => true,
                | Command::StepOver => !over_call || frames <= depth,
                | Command::StepOut => frames < depth,
                // From code with no line (clang's `main` wrapper), the first
                // line reached at any depth.
                | Command::NextLine =>
                    line.is_some()
                        && (start_line.is_none()
                            || (frames <= depth && (line != start_line || frames < depth))),
                | Command::RunToLine(target) => line == Some(target) && previous_line != line,
                | Command::Continue => false,
            };
            if line.is_some() {
                previous_line = line;
            }
            if done {
                return Stop::Done;
            }
        }
    }

    /// Execute one instruction, keeping a checkpoint when one is due.
    fn step(&mut self) {
        self.machine.step();
        if self.machine.steps.is_multiple_of(self.interval)
            && self
                .checkpoints
                .last()
                .is_none_or(|last| last.steps < self.machine.steps)
        {
            self.checkpoints.push(self.machine.clone());
            if self.checkpoints.len() > CHECKPOINTS {
                // Keep every other one, at twice the spacing.
                self.interval *= 2;
                let interval = self.interval;
                self.checkpoints
                    .retain(|checkpoint| checkpoint.steps.is_multiple_of(interval));
            }
        }
    }
}

/// A module ready to run: decoded, validated, listed, and mapped to source
/// lines when a map is known.
#[derive(Debug)]
pub struct Program {
    pub module:     Rc<Module>,
    pub lines:      LineMap,
    pub wat:        Wat,
    /// The function the host calls, if the module has an entry point.
    pub entry:      Option<u32>,
    /// Imported functions the stepper does not provide, as `module.name`.
    pub unresolved: Vec<String>,
}

impl Program {
    /// Load the module in `bytes`. With no `lines`, its DWARF line table is
    /// read, if it has one.
    ///
    /// # Errors
    /// When the module is malformed or invalid.
    pub fn load(bytes: &[u8], lines: Option<LineMap>) -> Result<Self, String> {
        let module = decode::decode(bytes).map_err(|error| error.to_string())?;
        validate::validate(&module)?;
        let lines = lines
            .or_else(|| LineMap::from_dwarf(&module))
            .unwrap_or_default();
        let wat = wat::print(&module);
        let entry = exec::entry_point(&module);
        let unresolved = module
            .imports
            .iter()
            .filter(|import| match import.kind {
                | ImportKind::Func(ty) =>
                    !wasi::provides(&import.module, &import.name, module.types.get(ty as usize)),
                | _ => false,
            })
            .map(|import| format!("{}.{}", import.module, import.name))
            .collect();
        Ok(Self {
            module: Rc::new(module),
            lines,
            wat,
            entry,
            unresolved,
        })
    }

    /// The source line of instruction `pc` of `func`.
    pub fn line_of(&self, func: u32, pc: u32) -> Option<u32> {
        let offset = *self.module.body(func)?.offsets.get(pc as usize)?;
        self.lines.line_at(offset)
    }

    /// The module offset of instruction `pc` of `func`.
    pub fn offset_of(&self, func: u32, pc: u32) -> Option<u32> {
        self.module.body(func)?.offsets.get(pc as usize).copied()
    }
}

impl Session {
    /// Instantiate `program`, ready before its first instruction.
    ///
    /// # Errors
    /// When the module cannot be instantiated (see [`Machine::instantiate`]).
    pub fn new(program: Rc<Program>) -> Result<Self, InstantiateError> {
        let mut machine =
            Machine::instantiate(Rc::clone(&program.module), program.entry, Host::default())?;
        if program.entry.is_none() && program.module.start.is_none() {
            return Err(InstantiateError(
                "the module exports no entry point: no `_start` or `main` function".into(),
            ));
        }
        // Step 0 starts the first function, so the page opens on its first
        // instruction.
        machine.step();
        machine.steps = 0;
        machine.event = Event {
            depth: machine.frames.len(),
            ..machine.event.clone()
        };
        Ok(Self {
            program,
            checkpoints: vec![machine.clone()],
            machine,
            interval: 1024,
            breakpoints: BTreeSet::new(),
        })
    }

    pub fn program(&self) -> &Rc<Program> {
        &self.program
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// Return to the state before the first instruction.
    pub fn reset(&mut self) {
        self.machine = self.checkpoints[0].clone();
    }

    /// Show the state after `steps` instructions, re-executing from the
    /// nearest checkpoint (or the end, if the program stops first).
    pub fn go_to(&mut self, steps: u64) {
        let steps = steps.min(MAX_STEPS);
        let index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.steps <= steps)
            .saturating_sub(1);
        // Restore a checkpoint to go back, or to skip ahead of the machine.
        if steps < self.machine.steps || self.checkpoints[index].steps > self.machine.steps {
            self.machine = self.checkpoints[index].clone();
        }
        while self.machine.steps < steps && self.machine.status == Status::Running {
            self.step();
        }
    }

    pub fn set_breakpoints(&mut self, offsets: impl IntoIterator<Item = u32>) {
        self.breakpoints = offsets.into_iter().collect();
    }

    pub fn breakpoints(&self) -> &BTreeSet<u32> {
        &self.breakpoints
    }

    /// The instruction that runs next.
    pub fn next_instr(&self) -> Option<&Instr> {
        let (func, pc) = self.machine.next()?;
        self.program.module.body(func)?.code.get(pc as usize)
    }

    /// The source line of the instruction that runs next.
    pub fn next_line(&self) -> Option<u32> {
        let (func, pc) = self.machine.next()?;
        self.program.line_of(func, pc)
    }

    fn at_breakpoint(&self) -> bool {
        self.machine
            .next()
            .and_then(|(func, pc)| self.program.offset_of(func, pc))
            .is_some_and(|offset| self.breakpoints.contains(&offset))
    }
}
