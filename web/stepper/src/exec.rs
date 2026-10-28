//! Execution (WebAssembly 2.0 §4): a module instance and a machine that runs
//! one instruction per [`Machine::step`].
//!
//! The state is what the spec's abstract machine keeps, made explicit for
//! display: an operand stack shared by all calls, a stack of call frames
//! (each with its locals and where its operands start), and a stack of
//! labels, one per entered `block`, `loop` or `if` plus one per call for the
//! function body. Every step also records an [`Event`]: which operands it
//! popped and pushed, and what else it touched, so the page can explain it.
//!
//! Reading order: [`Machine::step`], then `execute` and its helpers for
//! control flow (`enter`, `branch`, `call`, `finish_call`); `numeric.rs`
//! holds the arithmetic, `memory.rs` linear memory, and `instantiate.rs`
//! builds the instance.

mod instantiate;
mod memory;
pub mod numeric;
mod tables;

use std::rc::Rc;

pub use instantiate::{
    InstantiateError,
    entry_point,
};
pub use memory::Memory;

use crate::{
    instr::Instr,
    module::{
        BlockType,
        FuncKind,
        Module,
    },
    value::Value,
    wasi::{
        Host,
        HostOutcome,
        WasiFunction,
    },
};

/// The deepest call stack a program may build.
pub const MAX_FRAMES: usize = 4096;
/// The most operands the stack may hold.
pub const MAX_OPERANDS: usize = 1 << 16;

#[derive(Clone, Debug)]
pub struct Machine {
    pub module:        Rc<Module>,
    pub memory:        Option<Memory>,
    pub globals:       Vec<Value>,
    pub tables:        Vec<tables::Table>,
    /// Each element segment's references; a dropped segment is empty.
    pub elements:      Vec<Vec<Value>>,
    /// Which data segments have been dropped.
    pub dropped_data:  Vec<bool>,
    pub stack:         Vec<Value>,
    pub frames:        Vec<Frame>,
    pub labels:        Vec<Label>,
    pub status:        Status,
    /// Instructions executed.
    pub steps:         u64,
    pub host:          Host,
    /// The WASI function each imported function resolved to.
    pub resolved:      Vec<Option<WasiFunction>>,
    /// Functions the host still has to call: the start function, then the
    /// entry point. The last is called first.
    pending:           Vec<u32>,
    /// Arguments `call_with` put on the stack for the next start.
    queued_args:       usize,
    /// The global holding the C shadow stack's pointer, if the module names
    /// one (`__stack_pointer`).
    pub stack_pointer: Option<u32>,
    /// What the last step did.
    pub event:         Event,
}

/// One call in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub func:          u32,
    /// The index of the next instruction to execute.
    pub pc:            u32,
    pub locals:        Vec<Value>,
    /// Where this call's operands start on the operand stack.
    pub stack_base:    usize,
    /// Where its labels start on the label stack; that label is the
    /// function body's.
    pub label_base:    usize,
    pub arity:         usize,
    /// The shadow stack pointer when the call began.
    pub stack_pointer: Option<u32>,
}

/// An entered structured instruction, or a function body (§4.2.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub kind:   LabelKind,
    /// The operand stack's height below the block's parameters.
    pub height: usize,
    /// How many values a branch to it carries: a loop's parameters, a
    /// block's results.
    pub arity:  usize,
    /// The index of the instruction that opened it.
    pub start:  u32,
    /// The index of its `end`.
    pub end:    u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Function,
    Block,
    Loop,
    If,
    Else,
}

impl LabelKind {
    pub fn name(self) -> &'static str {
        match self {
            | Self::Function => "function",
            | Self::Block => "block",
            | Self::Loop => "loop",
            | Self::If => "if",
            | Self::Else => "else",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Running,
    /// The entry point returned these values.
    Finished(Vec<Value>),
    /// The program called WASI's `proc_exit`.
    Exited(i32),
    Trapped(Trap),
}

/// A trap (§4.4): execution stops and cannot continue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    pub kind:    TrapKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    Unreachable,
    DivideByZero,
    IntegerOverflow,
    InvalidConversion,
    MemoryOutOfBounds,
    TableOutOfBounds,
    UndefinedElement,
    UninitializedElement,
    IndirectCallTypeMismatch,
    CallStackExhausted,
    /// An import the stepper does not provide was called.
    Unresolved,
    /// A host function failed, as for bad pointers.
    Host,
    /// The stepper itself failed; validation should rule this out.
    Internal,
}

impl Trap {
    pub fn new(kind: TrapKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn internal(what: &str) -> Self {
        Self::new(TrapKind::Internal, format!("stepper error: {what}"))
    }
}

/// What one step did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub func:   u32,
    /// The executed instruction's index; `None` for the host starting a
    /// function.
    pub pc:     Option<u32>,
    /// The call depth it ran at (1 is the entry point).
    pub depth:  usize,
    /// Operands taken from the stack, in stack order (deepest first).
    pub popped: Vec<Value>,
    /// Operands left on the stack, in stack order.
    pub pushed: Vec<Value>,
    pub effect: Effect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Effect {
    #[default]
    None,
    /// The host started `func` (the start function or the entry point).
    Start {
        func: u32,
    },
    LocalRead(u32),
    LocalWrite(u32),
    GlobalRead(u32),
    GlobalWrite(u32),
    Load {
        address: u64,
        size:    u32,
    },
    Store {
        address: u64,
        size:    u32,
    },
    /// A `block`, `loop` or `if` began; `taken` is an `if`'s condition.
    Enter {
        kind:  LabelKind,
        taken: bool,
    },
    /// An `end` closed a block.
    Exit {
        kind: LabelKind,
    },
    /// The then-branch ended at `else`, which jumps to the `end`.
    SkipElse,
    /// A branch. `taken` is false for a `br_if` that fell through; `index`
    /// is the `br_table` operand.
    Branch {
        depth:     u32,
        taken:     bool,
        kind:      LabelKind,
        discarded: usize,
        index:     Option<u32>,
    },
    Call {
        callee: u32,
    },
    /// A call to a host function, and what it did.
    Host {
        callee: u32,
        note:   String,
    },
    /// The function returned to its caller (or to the host).
    Return {
        func:      u32,
        discarded: usize,
    },
    Grow {
        previous: i32,
        delta:    u32,
    },
    Bulk {
        destination: u64,
        source:      Option<u64>,
        length:      u64,
    },
    Table {
        index: u32,
    },
    Trap,
}

impl Machine {
    /// Execute the next instruction, or start the next function the host
    /// calls. Does nothing once the program has stopped.
    pub fn step(&mut self) {
        if self.status != Status::Running {
            return;
        }
        self.event = Event {
            depth: self.frames.len(),
            ..Event::default()
        };
        let result = if self.frames.is_empty() {
            self.start_pending()
        } else {
            self.execute_next()
        };
        self.steps += 1;
        if let Err(trap) = result {
            self.event.effect = Effect::Trap;
            self.status = Status::Trapped(trap);
        }
    }

    /// Queue a call of `func` with `args` (for tests and embedders that call
    /// a function other than the entry point); the next step starts it.
    pub fn call_with(&mut self, func: u32, args: &[Value]) {
        self.pending.push(func);
        self.stack.extend_from_slice(args);
        self.status = Status::Running;
        self.queued_args = args.len();
    }

    /// The next instruction's function and index, if one runs next.
    pub fn next(&self) -> Option<(u32, u32)> {
        if self.status != Status::Running {
            return None;
        }
        self.frames.last().map(|frame| (frame.func, frame.pc))
    }

    fn start_pending(&mut self) -> Result<(), Trap> {
        let Some(func) = self.pending.pop() else {
            self.status = Status::Finished(Vec::new());
            return Ok(());
        };
        self.event.func = func;
        self.event.effect = Effect::Start { func };
        let ty = self
            .module
            .func_type(func)
            .ok_or_else(|| Trap::internal("unknown entry"))?;
        // An entry point with parameters (C's `main(argc, argv)` wrapper) is
        // called with zeros, unless `call_with` gave arguments.
        if std::mem::take(&mut self.queued_args) != ty.params.len() {
            for &param in &ty.params.clone() {
                self.stack.push(Value::default_of(param));
            }
        }
        let result = self.invoke(func);
        self.event.popped.clear();
        result
    }

    fn execute_next(&mut self) -> Result<(), Trap> {
        let module = Rc::clone(&self.module);
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| Trap::internal("no frame"))?;
        let (func, pc) = (frame.func, frame.pc);
        let body = module.body(func).ok_or_else(|| Trap::internal("no body"))?;
        let instr = body
            .code
            .get(pc as usize)
            .ok_or_else(|| Trap::internal("ran past the end of a function"))?;
        frame.pc += 1;
        self.event.func = func;
        self.event.pc = Some(pc);
        let result = self.execute(instr, pc);
        self.event.popped.reverse();
        result
    }

    #[expect(clippy::too_many_lines, reason = "One arm per instruction family.")]
    fn execute(&mut self, instr: &Instr, pc: u32) -> Result<(), Trap> {
        match instr {
            | Instr::Unreachable =>
                return Err(Trap::new(
                    TrapKind::Unreachable,
                    "unreachable executed: the program reached code that should never run",
                )),
            | Instr::Nop => {},
            | Instr::Block { ty, end } => self.enter(LabelKind::Block, *ty, pc, *end, true)?,
            | Instr::Loop { ty, end } => self.enter(LabelKind::Loop, *ty, pc, *end, true)?,
            | Instr::If { ty, else_, end } => {
                let condition = self.pop_i32()? != 0;
                self.enter(LabelKind::If, *ty, pc, *end, condition)?;
                if !condition {
                    // Into the else branch, or to the end when there is none.
                    let target = if *else_ == *end { *end } else { *else_ + 1 };
                    if let Some(label) = self.labels.last_mut()
                        && *else_ != *end
                    {
                        label.kind = LabelKind::Else;
                    }
                    self.frame_mut()?.pc = target;
                }
            },
            | Instr::Else { end } => {
                self.frame_mut()?.pc = *end;
                self.event.effect = Effect::SkipElse;
            },
            | Instr::End => {
                let base = self.frame()?.label_base;
                if self.labels.len() > base + 1 {
                    let label = self.labels.pop().ok_or_else(|| Trap::internal("labels"))?;
                    self.event.effect = Effect::Exit { kind: label.kind };
                } else {
                    self.finish_call()?;
                }
            },
            | Instr::Br(depth) => self.branch(*depth, None)?,
            | Instr::BrIf(depth) =>
                if self.pop_i32()? == 0 {
                    let kind = self.label(*depth)?.kind;
                    self.event.effect = Effect::Branch {
                        depth: *depth,
                        taken: false,
                        kind,
                        discarded: 0,
                        index: None,
                    };
                } else {
                    self.branch(*depth, None)?;
                },
            | Instr::BrTable { targets, default } => {
                let index = self.pop_i32()? as u32;
                let depth = targets.get(index as usize).copied().unwrap_or(*default);
                self.branch(depth, Some(index))?;
            },
            | Instr::Return => {
                let depth = self.labels.len() - 1 - self.frame()?.label_base;
                self.branch(u32::try_from(depth).unwrap_or(u32::MAX), None)?;
            },
            | Instr::Call(func) => self.call(*func)?,
            | Instr::CallIndirect { ty, table } => {
                let index = self.pop_i32()? as u32;
                let func = self.indirect(*table, index, *ty)?;
                self.call(func)?;
            },
            | Instr::Drop => {
                self.pop()?;
            },
            | Instr::Select(_) => {
                let condition = self.pop_i32()?;
                let second = self.pop()?;
                let first = self.pop()?;
                self.push(if condition != 0 { first } else { second })?;
            },
            | Instr::LocalGet(index) => {
                let value = *self.local(*index)?;
                self.event.effect = Effect::LocalRead(*index);
                self.push(value)?;
            },
            | Instr::LocalSet(index) => {
                let value = self.pop()?;
                *self.local(*index)? = value;
                self.event.effect = Effect::LocalWrite(*index);
            },
            | Instr::LocalTee(index) => {
                let value = self.pop()?;
                *self.local(*index)? = value;
                self.event.effect = Effect::LocalWrite(*index);
                self.push(value)?;
            },
            | Instr::GlobalGet(index) => {
                let value = *self
                    .globals
                    .get(*index as usize)
                    .ok_or_else(|| Trap::internal("global"))?;
                self.event.effect = Effect::GlobalRead(*index);
                self.push(value)?;
            },
            | Instr::GlobalSet(index) => {
                let value = self.pop()?;
                *self
                    .globals
                    .get_mut(*index as usize)
                    .ok_or_else(|| Trap::internal("global"))? = value;
                self.event.effect = Effect::GlobalWrite(*index);
            },
            | Instr::Load(load, memarg) => {
                let base = self.pop_i32()?;
                let (address, value) = self.memory()?.load(*load, base, memarg.offset)?;
                self.event.effect = Effect::Load {
                    address,
                    size: load.size(),
                };
                self.push(value)?;
            },
            | Instr::Store(store, memarg) => {
                let value = self.pop()?;
                let base = self.pop_i32()?;
                let address = self
                    .memory_mut()?
                    .store(*store, base, memarg.offset, value)?;
                self.event.effect = Effect::Store {
                    address,
                    size: store.size(),
                };
            },
            | Instr::MemorySize => {
                let pages = self.memory()?.pages();
                self.push(Value::I32(pages as i32))?;
            },
            | Instr::MemoryGrow => {
                let delta = self.pop_i32()? as u32;
                let previous = self.memory_mut()?.grow(delta);
                self.event.effect = Effect::Grow { previous, delta };
                self.push(Value::I32(previous))?;
            },
            | Instr::MemoryInit(segment) => self.memory_init(*segment)?,
            | Instr::DataDrop(segment) => {
                if let Some(dropped) = self.dropped_data.get_mut(*segment as usize) {
                    *dropped = true;
                }
            },
            | Instr::MemoryCopy => {
                let length = self.pop_i32()? as u32;
                let source = self.pop_i32()? as u32;
                let destination = self.pop_i32()? as u32;
                self.memory_mut()?.copy(destination, source, length)?;
                self.event.effect = Effect::Bulk {
                    destination: u64::from(destination),
                    source:      Some(u64::from(source)),
                    length:      u64::from(length),
                };
            },
            | Instr::MemoryFill => {
                let length = self.pop_i32()? as u32;
                let value = self.pop_i32()? as u8;
                let destination = self.pop_i32()? as u32;
                self.memory_mut()?.fill(destination, value, length)?;
                self.event.effect = Effect::Bulk {
                    destination: u64::from(destination),
                    source:      None,
                    length:      u64::from(length),
                };
            },
            | Instr::TableGet(_)
            | Instr::TableSet(_)
            | Instr::TableInit { .. }
            | Instr::ElemDrop(_)
            | Instr::TableCopy { .. }
            | Instr::TableGrow(_)
            | Instr::TableSize(_)
            | Instr::TableFill(_) => self.table_instruction(instr)?,
            | Instr::I32Const(value) => self.push(Value::I32(*value))?,
            | Instr::I64Const(value) => self.push(Value::I64(*value))?,
            | Instr::F32Const(bits) => self.push(Value::F32(*bits))?,
            | Instr::F64Const(bits) => self.push(Value::F64(*bits))?,
            | Instr::RefNull(ty) => self.push(Value::default_of(*ty))?,
            | Instr::RefIsNull => {
                let null = matches!(self.pop()?, Value::FuncRef(None) | Value::ExternRef(None));
                self.push(Value::I32(i32::from(null)))?;
            },
            | Instr::RefFunc(func) => self.push(Value::FuncRef(Some(*func)))?,
            | Instr::Numeric(op) => {
                let (operands, _) = crate::instr::numeric_signature(*op)
                    .ok_or_else(|| Trap::internal("unknown opcode"))?;
                let mut args = [Value::I32(0); 2];
                for slot in args[..operands.len()].iter_mut().rev() {
                    *slot = self.pop()?;
                }
                let result = numeric::apply(*op, &args[..operands.len()])?;
                self.push(result)?;
            },
            | Instr::TruncSat(op) => {
                let operand = self.pop()?;
                self.push(numeric::trunc_sat(*op, operand)?)?;
            },
        }
        Ok(())
    }

    /// Enter a `block`, `loop` or `if` opened at `start` and closed at
    /// `end`.
    fn enter(
        &mut self,
        kind: LabelKind,
        ty: BlockType,
        start: u32,
        end: u32,
        taken: bool,
    ) -> Result<(), Trap> {
        let (params, results) = match ty {
            | BlockType::Empty => (0, 0),
            | BlockType::Value(_) => (0, 1),
            | BlockType::Type(index) => {
                let ty = self
                    .module
                    .types
                    .get(index as usize)
                    .ok_or_else(|| Trap::internal("block type"))?;
                (ty.params.len(), ty.results.len())
            },
        };
        let height = self
            .stack
            .len()
            .checked_sub(params)
            .ok_or_else(|| Trap::internal("block parameters"))?;
        self.labels.push(Label {
            kind,
            height,
            arity: if kind == LabelKind::Loop {
                params
            } else {
                results
            },
            start,
            end,
        });
        self.event.effect = Effect::Enter { kind, taken };
        Ok(())
    }

    /// The label `depth` levels out from the innermost one.
    fn label(&self, depth: u32) -> Result<Label, Trap> {
        let base = self.frame()?.label_base;
        self.labels
            .len()
            .checked_sub(1 + depth as usize)
            .filter(|&index| index >= base)
            .map(|index| self.labels[index])
            .ok_or_else(|| Trap::internal("branch target"))
    }

    /// Branch to the label `depth` out (§4.4.8): keep the values it carries,
    /// discard the operands above its height, and continue after its `end`
    /// (or at the top of a loop). Branching to the function's own label
    /// returns.
    fn branch(&mut self, depth: u32, index: Option<u32>) -> Result<(), Trap> {
        let label = self.label(depth)?;
        let position = self.labels.len() - 1 - depth as usize;
        if label.kind == LabelKind::Function {
            // A branch to the function's own label is a return, and records
            // one.
            return self.finish_call();
        }
        let discarded = self.unwind(label.height, label.arity)?;
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| Trap::internal("frame"))?;
        if label.kind == LabelKind::Loop {
            self.labels.truncate(position + 1);
            frame.pc = label.start + 1;
        } else {
            self.labels.truncate(position);
            frame.pc = label.end + 1;
        }
        self.event.effect = Effect::Branch {
            depth,
            taken: true,
            kind: label.kind,
            discarded,
            index,
        };
        Ok(())
    }

    /// Keep the top `arity` operands and drop everything above `height`
    /// below them; returns how many were dropped.
    fn unwind(&mut self, height: usize, arity: usize) -> Result<usize, Trap> {
        let top = self
            .stack
            .len()
            .checked_sub(arity)
            .filter(|&top| top >= height)
            .ok_or_else(|| Trap::internal("branch operands"))?;
        let kept = self.stack.split_off(top);
        let discarded = top - height;
        self.stack.truncate(height);
        self.stack.extend(kept);
        Ok(discarded)
    }

    /// Call `func` with the arguments on top of the stack (§4.4.10).
    fn call(&mut self, func: u32) -> Result<(), Trap> {
        let module = Rc::clone(&self.module);
        let entry = module
            .funcs
            .get(func as usize)
            .ok_or_else(|| Trap::internal("callee"))?;
        if let FuncKind::Import { module: from, name } = &entry.kind {
            return self.call_host(func, from, name);
        }
        self.event.effect = Effect::Call { callee: func };
        self.invoke(func)
    }

    /// Push a frame for the defined function `func`, taking its arguments.
    fn invoke(&mut self, func: u32) -> Result<(), Trap> {
        let module = Rc::clone(&self.module);
        let ty = module
            .func_type(func)
            .ok_or_else(|| Trap::internal("callee type"))?;
        let Some(body) = module.body(func) else {
            let (from, name) = match &module.funcs[func as usize].kind {
                | FuncKind::Import { module, name } => (module.clone(), name.clone()),
                | FuncKind::Defined(_) => unreachable!("a body was found"),
            };
            return self.call_host(func, &from, &name);
        };
        if self.frames.len() >= MAX_FRAMES {
            return Err(Trap::new(
                TrapKind::CallStackExhausted,
                format!(
                    "call stack exhausted: more than {MAX_FRAMES} calls were active at once \
                     (runaway recursion?)"
                ),
            ));
        }
        let mut locals = self.pop_n(ty.params.len())?;
        locals.extend(body.locals.iter().map(|&ty| Value::default_of(ty)));
        let stack_pointer = self.stack_pointer_value();
        self.frames.push(Frame {
            func,
            pc: 0,
            locals,
            stack_base: self.stack.len(),
            label_base: self.labels.len(),
            arity: ty.results.len(),
            stack_pointer,
        });
        self.labels.push(Label {
            kind:   LabelKind::Function,
            height: self.stack.len(),
            arity:  ty.results.len(),
            start:  0,
            end:    u32::try_from(body.code.len().saturating_sub(1)).unwrap_or(u32::MAX),
        });
        Ok(())
    }

    /// Return from the innermost call: keep its results, drop its other
    /// operands, labels and frame.
    fn finish_call(&mut self) -> Result<(), Trap> {
        let frame = self.frames.pop().ok_or_else(|| Trap::internal("return"))?;
        let discarded = self.unwind(frame.stack_base, frame.arity)?;
        self.labels.truncate(frame.label_base);
        self.event.effect = Effect::Return {
            func: frame.func,
            discarded,
        };
        let results = self.stack.len() - frame.arity;
        self.event.pushed.extend_from_slice(&self.stack[results..]);
        if self.frames.is_empty() && self.pending.is_empty() {
            let results = std::mem::take(&mut self.stack);
            self.status = Status::Finished(results);
        } else if self.frames.is_empty() {
            // The start function's results (it has none) are not the
            // program's.
            self.stack.clear();
        }
        Ok(())
    }

    fn call_host(&mut self, func: u32, from: &str, name: &str) -> Result<(), Trap> {
        let ty = self
            .module
            .func_type(func)
            .cloned()
            .ok_or_else(|| Trap::internal("host type"))?;
        let Some(function) = self.resolved.get(func as usize).copied().flatten() else {
            return Err(Trap::new(
                TrapKind::Unresolved,
                format!(
                    "the program called `{from}.{name}`, an import the stepper does not provide"
                ),
            ));
        };
        let args = self.pop_n(ty.params.len())?;
        let outcome = self
            .host
            .call(function, &args, self.memory.as_mut(), self.steps)?;
        match outcome {
            | HostOutcome::Return { results, note } => {
                for value in results {
                    self.push(value)?;
                }
                self.event.effect = Effect::Host { callee: func, note };
            },
            | HostOutcome::Exit(code) => {
                self.event.effect = Effect::Host {
                    callee: func,
                    note:   format!("the program exited with status {code}"),
                };
                self.status = Status::Exited(code);
            },
        }
        Ok(())
    }

    /// The function `call_indirect` calls through `table[index]`, checked
    /// against type `ty`.
    fn indirect(&self, table: u32, index: u32, ty: u32) -> Result<u32, Trap> {
        let table = self
            .tables
            .get(table as usize)
            .ok_or_else(|| Trap::internal("table"))?;
        let element = table.elements.get(index as usize).ok_or_else(|| {
            Trap::new(
                TrapKind::UndefinedElement,
                format!(
                    "undefined element: call_indirect used index {index}, but the table has {} \
                     entries",
                    table.elements.len()
                ),
            )
        })?;
        let Value::FuncRef(Some(func)) = *element else {
            return Err(Trap::new(
                TrapKind::UninitializedElement,
                format!("uninitialized element: table entry {index} holds no function"),
            ));
        };
        let expected = self.module.types.get(ty as usize);
        if self.module.func_type(func) != expected {
            return Err(Trap::new(
                TrapKind::IndirectCallTypeMismatch,
                format!(
                    "indirect call type mismatch: table entry {index} is {}, whose type differs \
                     from the call's",
                    self.module.func_name(func)
                ),
            ));
        }
        Ok(func)
    }

    fn memory_init(&mut self, segment: u32) -> Result<(), Trap> {
        let length = self.pop_i32()? as u32;
        let source = self.pop_i32()? as u32;
        let destination = self.pop_i32()? as u32;
        let module = Rc::clone(&self.module);
        let bytes = if self.dropped_data.get(segment as usize) == Some(&true) {
            &[][..]
        } else {
            module
                .data
                .get(segment as usize)
                .map_or(&[][..], |data| data.bytes.as_slice())
        };
        let end = u64::from(source) + u64::from(length);
        if end > bytes.len() as u64 {
            return Err(Trap::new(
                TrapKind::MemoryOutOfBounds,
                format!(
                    "out of bounds memory access: memory.init read bytes {source}..{end} of a \
                     {}-byte data segment",
                    bytes.len()
                ),
            ));
        }
        self.memory_mut()?
            .write(destination, &bytes[source as usize..end as usize])?;
        self.event.effect = Effect::Bulk {
            destination: u64::from(destination),
            source:      None,
            length:      u64::from(length),
        };
        Ok(())
    }

    fn frame(&self) -> Result<&Frame, Trap> {
        self.frames.last().ok_or_else(|| Trap::internal("no frame"))
    }

    fn frame_mut(&mut self) -> Result<&mut Frame, Trap> {
        self.frames
            .last_mut()
            .ok_or_else(|| Trap::internal("no frame"))
    }

    fn local(&mut self, index: u32) -> Result<&mut Value, Trap> {
        self.frame_mut()?
            .locals
            .get_mut(index as usize)
            .ok_or_else(|| Trap::internal("local"))
    }

    fn memory(&self) -> Result<&Memory, Trap> {
        self.memory
            .as_ref()
            .ok_or_else(|| Trap::internal("no memory"))
    }

    fn memory_mut(&mut self) -> Result<&mut Memory, Trap> {
        self.memory
            .as_mut()
            .ok_or_else(|| Trap::internal("no memory"))
    }

    /// What keeping a copy of this machine costs, in bytes, beside
    /// `previous` (whose unchanged linear memory it shares).
    pub fn retained_bytes(&self, previous: Option<&Self>) -> usize {
        let value = size_of::<Value>();
        let memory = match (
            &self.memory,
            previous.and_then(|previous| previous.memory.as_ref()),
        ) {
            | (Some(memory), Some(shared)) if memory.shares(shared) => 0,
            | (Some(memory), _) => memory.bytes.len(),
            | (None, _) => 0,
        };
        let tables: usize = self
            .tables
            .iter()
            .map(|table| table.elements.len() * value)
            .sum();
        let elements: usize = self
            .elements
            .iter()
            .map(|segment| segment.len() * value)
            .sum();
        let frames: usize = self
            .frames
            .iter()
            .map(|frame| size_of::<Frame>() + frame.locals.len() * value)
            .sum();
        memory
            + tables
            + elements
            + frames
            + (self.stack.len() + self.globals.len()) * value
            + self.labels.len() * size_of::<Label>()
            + self.host.retained_bytes()
    }

    /// The shadow stack pointer's current value.
    pub fn stack_pointer_value(&self) -> Option<u32> {
        match self.globals.get(self.stack_pointer? as usize)? {
            | Value::I32(value) => Some(*value as u32),
            | _ => None,
        }
    }

    fn push(&mut self, value: Value) -> Result<(), Trap> {
        if self.stack.len() >= MAX_OPERANDS {
            return Err(Trap::new(
                TrapKind::CallStackExhausted,
                "the operand stack holds too many values",
            ));
        }
        self.stack.push(value);
        self.event.pushed.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Value, Trap> {
        let value = self
            .stack
            .pop()
            .ok_or_else(|| Trap::internal("operand stack underflow"))?;
        // A value pushed and popped in the same step is neither.
        if self.event.pushed.last() == Some(&value) {
            self.event.pushed.pop();
        } else {
            self.event.popped.push(value);
        }
        Ok(value)
    }

    fn pop_i32(&mut self) -> Result<i32, Trap> {
        match self.pop()? {
            | Value::I32(value) => Ok(value),
            | _ => Err(Trap::internal("expected an i32")),
        }
    }

    /// Pop `count` operands, returned in stack order.
    fn pop_n(&mut self, count: usize) -> Result<Vec<Value>, Trap> {
        let top = self
            .stack
            .len()
            .checked_sub(count)
            .ok_or_else(|| Trap::internal("missing arguments"))?;
        let values = self.stack.split_off(top);
        self.event.popped.extend(values.iter().rev());
        Ok(values)
    }
}
