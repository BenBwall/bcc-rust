//! What each WebAssembly step did, in plain language for programmers who
//! know some C but not WebAssembly or compilers: the instruction with its
//! actual values, why the machine works that way, and how the C compiler
//! uses it (the shadow stack, structured control flow in place of `goto`).
//! Each step also names the ideas it relies on, with the section of the
//! WebAssembly 2.0 specification that defines them.

use std::fmt::Write;

use wasm_stepper::{
    Effect,
    Instr,
    LabelKind,
    Load,
    Machine,
    Session,
    Status,
    TrapKind,
    Value,
};

use super::{
    Concept,
    WasmNarration,
};

/// The narration for the session's last step.
pub fn narrate(session: &Session) -> WasmNarration {
    let machine = session.machine();
    let program = session.program();
    let event = &machine.event;
    let instruction = event
        .pc
        .and_then(|pc| program.wat.line_of(event.func, pc))
        .and_then(|line| program.wat.text.lines().nth(line - 1))
        .map(|text| {
            text.trim()
                .split("  ;;")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    let context = Context {
        session,
        machine,
        instruction: &instruction,
    };
    let mut narration = match (&machine.status, event.pc) {
        | (Status::Trapped(trap), _) if event.effect == Effect::Trap =>
            trapped(&trap.message, trap.kind),
        | (_, None) => context.start(),
        | (_, Some(pc)) => {
            let instr = program
                .module
                .body(event.func)
                .and_then(|body| body.code.get(pc as usize));
            match instr {
                | Some(instr) => context.instruction(instr, pc),
                | None => Step::new(
                    "control",
                    "Step",
                    "The stepper executed an instruction.",
                    "",
                ),
            }
        },
    }
    .finish(&instruction);
    if let Status::Finished(values) = &machine.status
        && machine.frames.is_empty()
        && event.pc.is_some()
    {
        let result = values
            .first()
            .map_or_else(|| "nothing".to_owned(), |value| format!("`{value}`"));
        _ = write!(
            narration.explanation,
            " That was the entry point, so the program is finished: it returned {result} to the \
             host, which is what a C program's exit status would be."
        );
    }
    if let Status::Exited(code) = machine.status {
        narration.title = format!("The program exited with status `{code}`");
    }
    narration
}

/// The narration after Start at main ran the code before `main`, `skipped`
/// instructions: the C library's startup, and any constructors the
/// program has.
pub fn skipped_startup(session: &Session, skipped: u64) -> WasmNarration {
    let program = session.program();
    let entry = program.entry.map_or_else(
        || "_start".to_owned(),
        |func| program.module.func_name(func),
    );
    Step::new(
        "call",
        "Start",
        "Skipped the code before `main`: `main` is about to run",
        &format!(
            "The host (here, this page) called `${entry}`, the entry point every WASI program \
             exports, and it ran {skipped} instruction{} before calling your `main`: the C \
             library's startup, such as asking the host for the program's arguments (WASI's \
             `args_sizes_get` and `args_get`) to pass as `argc` and `argv`, and any constructors \
             the program has. It will pass `main`'s result to `proc_exit` as the exit status. \
             Start at main ran them for you, so the first instruction of `main` is next. Back \
             steps through them; turn Start at main off to begin at `${entry}`.",
            plural(skipped)
        ),
    )
    .with(concepts::instantiation())
    .with(concepts::host())
    .finish("")
}

/// A narration in progress.
struct Step {
    category:    &'static str,
    label:       &'static str,
    title:       String,
    explanation: String,
    concepts:    Vec<Concept>,
}

impl Step {
    fn new(category: &'static str, label: &'static str, title: &str, explanation: &str) -> Self {
        Self {
            category,
            label,
            title: title.to_owned(),
            explanation: explanation.to_owned(),
            concepts: Vec::new(),
        }
    }

    fn with(mut self, concept: Concept) -> Self {
        self.concepts.push(concept);
        self
    }

    fn finish(self, instruction: &str) -> WasmNarration {
        WasmNarration {
            category:    self.category.to_owned(),
            label:       self.label.to_owned(),
            instruction: instruction.to_owned(),
            title:       self.title,
            explanation: self.explanation,
            concepts:    self.concepts,
        }
    }
}

struct Context<'s> {
    session:     &'s Session,
    machine:     &'s Machine,
    instruction: &'s str,
}

impl Context<'_> {
    fn start(&self) -> Step {
        let program = self.session.program();
        let Effect::Start { func } = self.machine.event.effect else {
            return Step::new("call", "Start", "Ready", "");
        };
        let name = program.module.func_name(func);
        let pages = self
            .machine
            .memory
            .as_ref()
            .map_or(0, wasm_stepper::Memory::pages);
        let segments = program.module.data.len();
        let memory = if pages == 0 {
            "It has no linear memory.".to_owned()
        } else {
            format!(
                "Its linear memory, {pages} page{} of 64 KiB, is allocated and zeroed{}, and its \
                 globals hold their initial values.",
                plural(u64::from(pages)),
                if segments == 0 {
                    String::new()
                } else {
                    format!(
                        ", its {segments} data segment{} copied in",
                        plural(segments as u64)
                    )
                }
            )
        };
        let start = if program.module.start == Some(func) {
            format!(
                "The host first calls `${name}`, the module's start function, which runs before \
                 anything else."
            )
        } else if name == "_start" {
            "The host (here, this page) now calls `$_start`, the entry point every WASI program \
             exports. It is not your code: the compiler's C library added it. It prepares the \
             program's arguments, calls `main`, and passes `main`'s result to WASI's `proc_exit` \
             as the program's exit status. Its first instruction is next; choose Next line to run \
             it up to the first line of your C."
                .to_owned()
        } else {
            format!(
                "The host (here, this page) now calls `${name}`, the function the C compiler made \
                 from `main`. Its first instruction is next."
            )
        };
        Step::new(
            "call",
            "Start",
            &format!("The module is instantiated; `${name}` is about to run"),
            &format!(
                "A WebAssembly module is a list of functions, plus the memory, globals and tables \
                 they share. To run it, the host instantiates it. {memory} {start} Every step \
                 from here executes exactly one instruction."
            ),
        )
        .with(concepts::instantiation())
        .with(concepts::stack())
    }

    #[expect(clippy::too_many_lines, reason = "One arm per instruction family.")]
    fn instruction(&self, instr: &Instr, pc: u32) -> Step {
        let event = &self.machine.event;
        let popped = &event.popped;
        let pushed = &event.pushed;
        let value = |values: &[Value], index: usize| {
            values
                .get(index)
                .map_or_else(|| "?".to_owned(), |value| format!("`{value}`"))
        };
        match instr {
            | Instr::I32Const(_) | Instr::I64Const(_) | Instr::F32Const(_) | Instr::F64Const(_) => {
                let ty = self.instruction.split('.').next().unwrap_or("i32");
                Step::new(
                    "evaluate",
                    "Constant",
                    &format!("Push the constant {}", value(pushed, 0)),
                    &format!(
                        "`{ty}.const` pushes a {} onto the operand stack. It is the simplest \
                         instruction: no operands, one result.{}",
                        type_description(ty),
                        self.constant_hint(pc)
                    ),
                )
                .with(concepts::stack())
            },
            | Instr::Numeric(op) => self.numeric(*op, popped, pushed, pc),
            | Instr::TruncSat(_) => Step::new(
                "evaluate",
                "Convert",
                &format!(
                    "Convert {} to {}, saturating",
                    value(popped, 0),
                    value(pushed, 0)
                ),
                &format!(
                    "`{}` truncates toward zero like C's cast, but never traps: a value out of \
                     range becomes the nearest integer that fits, and NaN becomes 0.",
                    self.instruction
                ),
            )
            .with(concepts::conversions()),
            | Instr::LocalGet(index) => {
                let local = self.local_name(*index);
                Step::new(
                    "convert",
                    "Local",
                    &format!("Read {local}: {}", value(pushed, 0)),
                    &format!(
                        "`local.get` copies {local} onto the operand stack. {}",
                        self.local_story(*index)
                    ),
                )
                .with(concepts::locals())
            },
            | Instr::LocalSet(index) | Instr::LocalTee(index) => {
                let local = self.local_name(*index);
                let tee = matches!(instr, Instr::LocalTee(_));
                Step::new(
                    "convert",
                    "Local",
                    &format!("Store {} in {local}", value(popped, 0)),
                    &format!(
                        "`{}` pops the top value and stores it in {local}{}. {}",
                        if tee { "local.tee" } else { "local.set" },
                        if tee {
                            ", then pushes it back: a store that keeps its value, like C's `x = y` \
                             used inside a larger expression"
                        } else {
                            ""
                        },
                        self.local_story(*index)
                    ),
                )
                .with(concepts::locals())
            },
            | Instr::GlobalGet(index) if Some(*index) == self.machine.stack_pointer => Step::new(
                "lifetime",
                "Shadow stack",
                &format!("Read the shadow stack pointer: {}", value(pushed, 0)),
                "`global.get $__stack_pointer` reads the C stack pointer. WebAssembly keeps its \
                 own call stack out of the program's reach, so nothing can take the address of a \
                 local. C needs addresses (for arrays, for `&x`, and at `-O0` for every local \
                 variable), so the compiler keeps a second stack in linear memory and this global \
                 points at its top. It grows downward.",
            )
            .with(concepts::shadow_stack()),
            | Instr::GlobalSet(index) if Some(*index) == self.machine.stack_pointer => {
                let new = match popped.first() {
                    | Some(Value::I32(value)) => value.cast_unsigned(),
                    | _ => 0,
                };
                let frame = self
                    .machine
                    .frames
                    .last()
                    .and_then(|frame| frame.stack_pointer);
                let (title, story) = match frame {
                    | Some(entry) if new < entry => (
                        format!("Reserve {} bytes of shadow stack", entry - new),
                        format!(
                            "The function's prologue lowers `$__stack_pointer` from {entry} to \
                             {new}: the {} bytes between are this call's frame in linear memory, \
                             where its C variables live. Watch the memory panel: loads and stores \
                             with an offset from this address are reads and writes of those \
                             variables.",
                            entry - new
                        ),
                    ),
                    | Some(entry) if new == entry => (
                        "Release the shadow stack frame".to_owned(),
                        format!(
                            "The epilogue restores `$__stack_pointer` to {entry}, its value when \
                             the call began. The frame's bytes are still in memory, but they now \
                             belong to no one: the next call will reuse them. That is why a C \
                             pointer to a returned function's local is dangling."
                        ),
                    ),
                    | _ => (
                        format!("Set the shadow stack pointer to `{new}`"),
                        "`global.set $__stack_pointer` moves the top of C's stack in linear \
                         memory."
                            .to_owned(),
                    ),
                };
                Step::new("lifetime", "Shadow stack", &title, &story).with(concepts::shadow_stack())
            },
            | Instr::GlobalGet(index) => Step::new(
                "convert",
                "Global",
                &format!(
                    "Read global {}: {}",
                    self.global_name(*index),
                    value(pushed, 0)
                ),
                "`global.get` pushes a global's value. Globals belong to the module rather than \
                 to a call, like C's file-scope variables, though a C compiler usually keeps \
                 those in linear memory so they have addresses.",
            )
            .with(concepts::globals()),
            | Instr::GlobalSet(index) => Step::new(
                "convert",
                "Global",
                &format!(
                    "Store {} in global {}",
                    value(popped, 0),
                    self.global_name(*index)
                ),
                "`global.set` pops a value into a mutable global. Validation rejects a module \
                 that writes an immutable one.",
            )
            .with(concepts::globals()),
            | Instr::Load(load, memarg) => {
                let (address, size) = match event.effect {
                    | Effect::Load { address, size } => (address, size),
                    | _ => (0, 0),
                };
                let base = value(popped, 0);
                let offset = if memarg.offset == 0 {
                    format!("address {base}")
                } else {
                    format!(
                        "{base} plus the constant offset {}, address `{address}`",
                        memarg.offset
                    )
                };
                Step::new(
                    "memory",
                    "Memory",
                    &format!("Load {} from address `{address}`", value(pushed, 0)),
                    &format!(
                        "`{}` pops an address and reads {size} byte{} of linear memory at \
                         {offset}, least significant byte first (little endian).{} {}",
                        load.name(),
                        plural(u64::from(size)),
                        extension(*load),
                        self.frame_hint(address)
                    ),
                )
                .with(concepts::memory())
            },
            | Instr::Store(store, memarg) => {
                let (address, size) = match event.effect {
                    | Effect::Store { address, size } => (address, size),
                    | _ => (0, 0),
                };
                let offset = if memarg.offset == 0 {
                    String::new()
                } else {
                    format!(
                        " (the address operand {} plus the offset {})",
                        value(popped, 0),
                        memarg.offset
                    )
                };
                Step::new(
                    "memory",
                    "Memory",
                    &format!("Store {} at address `{address}`", value(popped, 1)),
                    &format!(
                        "`{}` pops a value and an address, and writes the value's low {size} \
                         byte{} to linear memory at `{address}`{offset}. {}",
                        store.name(),
                        plural(u64::from(size)),
                        self.frame_hint(address)
                    ),
                )
                .with(concepts::memory())
            },
            | Instr::MemorySize => Step::new(
                "memory",
                "Memory",
                &format!("Memory is {} pages", value(pushed, 0)),
                "`memory.size` pushes the size of linear memory in 64 KiB pages.",
            )
            .with(concepts::memory()),
            | Instr::MemoryGrow => {
                let grown = !matches!(pushed.first(), Some(Value::I32(-1)));
                Step::new(
                    "memory",
                    "Memory",
                    &if grown {
                        format!("Grow memory by {} pages", value(popped, 0))
                    } else {
                        "Memory cannot grow".to_owned()
                    },
                    &if grown {
                        format!(
                            "`memory.grow` adds zeroed pages to the end of linear memory and \
                             pushes its old size, {}. This is how C's `malloc` gets more room.",
                            value(pushed, 0)
                        )
                    } else {
                        "`memory.grow` pushes -1: the request would pass the memory's maximum (the \
                         stepper allows 64 MiB). Nothing changed, and the program decides what to \
                         do, as `malloc` returning NULL does."
                            .to_owned()
                    },
                )
                .with(concepts::memory())
            },
            | Instr::MemoryCopy | Instr::MemoryFill | Instr::MemoryInit(_) | Instr::DataDrop(_) =>
                Step::new(
                    "memory",
                    "Memory",
                    &match instr {
                        | Instr::MemoryCopy => format!("Copy {} bytes", value(popped, 2)),
                        | Instr::MemoryFill => format!("Fill {} bytes", value(popped, 2)),
                        | Instr::MemoryInit(_) =>
                            format!("Copy {} bytes from a data segment", value(popped, 2)),
                        | _ => "Drop a data segment".to_owned(),
                    },
                    "The bulk memory instructions move whole ranges at once: compilers use them \
                     for `memcpy`, `memset` and large struct copies. Like every access, the whole \
                     range is bounds checked first.",
                )
                .with(concepts::memory()),
            | Instr::Block { .. } | Instr::Loop { .. } | Instr::If { .. } =>
                self.enter(instr, popped),
            | Instr::Else { .. } => Step::new(
                "control",
                "Control flow",
                "The then-branch is done: skip the else-branch",
                "Reaching `else` means the `if`'s first branch ran to its end, so execution jumps \
                 to the matching `end`.",
            )
            .with(concepts::structured()),
            | Instr::End => self.end(),
            | Instr::Br(_) | Instr::BrIf(_) | Instr::BrTable { .. } | Instr::Return =>
                self.branch(instr, popped),
            | Instr::Call(_) | Instr::CallIndirect { .. } => self.call(instr, popped),
            | Instr::Drop => Step::new(
                "evaluate",
                "Stack",
                &format!("Discard {}", value(popped, 0)),
                "`drop` pops a value and throws it away, as when C evaluates an expression for \
                 its side effects and ignores its value.",
            )
            .with(concepts::stack()),
            | Instr::Select(_) => Step::new(
                "evaluate",
                "Stack",
                &format!("Select {}", value(pushed, 0)),
                &format!(
                    "`select` pops a condition ({}) and two values, and keeps the first if the \
                     condition is nonzero, the second otherwise: C's `c ? a : b` without a branch.",
                    value(popped, 2)
                ),
            )
            .with(concepts::stack()),
            | Instr::Nop => Step::new(
                "control",
                "Control flow",
                "Do nothing",
                "`nop` has no effect.",
            ),
            | Instr::Unreachable => Step::new(
                "fault",
                "Trap",
                "Unreachable",
                "`unreachable` always traps.",
            ),
            | Instr::RefNull(_) | Instr::RefIsNull | Instr::RefFunc(_) => Step::new(
                "evaluate",
                "Reference",
                &format!("Push {}", value(pushed, 0)),
                "References are opaque values that name functions (or host objects). A C function \
                 pointer is not one: it is an index into a table of them.",
            )
            .with(concepts::tables()),
            | Instr::TableGet(_)
            | Instr::TableSet(_)
            | Instr::TableSize(_)
            | Instr::TableGrow(_)
            | Instr::TableFill(_)
            | Instr::TableCopy { .. }
            | Instr::TableInit { .. }
            | Instr::ElemDrop(_) => Step::new(
                "memory",
                "Table",
                &format!("`{}`", self.instruction),
                "Tables hold function references. `call_indirect` calls through one, which is how \
                 C function pointers work: the pointer is an index into the table.",
            )
            .with(concepts::tables()),
        }
    }

    #[expect(clippy::too_many_lines, reason = "One arm per operator family.")]
    fn numeric(&self, op: u8, popped: &[Value], pushed: &[Value], pc: u32) -> Step {
        let name = wasm_stepper::numeric_name(op);
        let operation = name.split('.').nth(1).unwrap_or(name);
        let result = pushed
            .first()
            .map_or_else(|| "?".to_owned(), |value| format!("`{value}`"));
        let a = popped
            .first()
            .map(|value| format!("`{value}`"))
            .unwrap_or_default();
        let b = popped
            .get(1)
            .map(|value| format!("`{value}`"))
            .unwrap_or_default();
        let comparison = |symbol: &str, signed: &str| {
            let truth = if matches!(pushed.first(), Some(Value::I32(1))) {
                "yes"
            } else {
                "no"
            };
            Step::new(
                "evaluate",
                "Compare",
                &format!("Is {a} {symbol} {b}? {}: {result}", capitalize(truth)),
                &format!(
                    "`{name}` pops two values and pushes 1 if {a} {symbol} {b}{signed}, 0 \
                     otherwise. WebAssembly has no boolean type: conditions are i32 values, and \
                     any nonzero value counts as true, as in C."
                ),
            )
            .with(concepts::stack())
        };
        let signed_note = |op: &str| {
            if op.ends_with("_u") {
                ", comparing the bits as unsigned"
            } else if op.ends_with("_s") {
                ", comparing as signed (two's complement) integers"
            } else {
                ""
            }
        };
        match op {
            | 0x45 | 0x50 => Step::new(
                "evaluate",
                "Compare",
                &format!("Is {a} zero? {result}"),
                &format!(
                    "`{name}` pushes 1 if its operand is zero and 0 otherwise. Compilers use it \
                     for C's `!x`, and to invert a condition before a branch."
                ),
            )
            .with(concepts::integers()),
            | 0x46 | 0x51 | 0x5B | 0x61 => comparison("=", ""),
            | 0x47 | 0x52 | 0x5C | 0x62 => comparison("≠", ""),
            | 0x48 | 0x49 | 0x53 | 0x54 | 0x5D | 0x63 => comparison("<", signed_note(operation)),
            | 0x4A | 0x4B | 0x55 | 0x56 | 0x5E | 0x64 => comparison(">", signed_note(operation)),
            | 0x4C | 0x4D | 0x57 | 0x58 | 0x5F | 0x65 => comparison("≤", signed_note(operation)),
            | 0x4E | 0x4F | 0x59 | 0x5A | 0x60 | 0x66 => comparison("≥", signed_note(operation)),
            | 0x6A | 0x7C | 0x92 | 0xA0 => self.arithmetic(name, "+", &a, &b, &result, op, pc),
            | 0x6B | 0x7D | 0x93 | 0xA1 => self.arithmetic(name, "−", &a, &b, &result, op, pc),
            | 0x6C | 0x7E | 0x94 | 0xA2 => self.arithmetic(name, "×", &a, &b, &result, op, pc),
            | 0x6D | 0x6E | 0x7F | 0x80 | 0x95 | 0xA3 => Step::new(
                "evaluate",
                "Compute",
                &format!("Divide {a} by {b}: {result}"),
                &format!(
                    "`{name}` pops two values and pushes their quotient{}.",
                    if op < 0x90 {
                        ", rounded toward zero as C's `/` does. Dividing by zero traps, where C \
                         leaves it undefined"
                    } else {
                        ""
                    }
                ),
            )
            .with(concepts::integers()),
            | 0x6F | 0x70 | 0x81 | 0x82 => Step::new(
                "evaluate",
                "Compute",
                &format!("Remainder of {a} ÷ {b}: {result}"),
                &format!(
                    "`{name}` pushes the remainder, with the sign of the dividend, like C's `%`."
                ),
            )
            .with(concepts::integers()),
            | 0x71..=0x73 | 0x83..=0x85 => {
                let symbol = match operation {
                    | "and" => "&",
                    | "or" => "|",
                    | _ => "^",
                };
                Step::new(
                    "evaluate",
                    "Compute",
                    &format!("{a} {symbol} {b} = {result}"),
                    &format!(
                        "`{name}` combines the two values bit by bit, like C's `{symbol}`.{}",
                        if operation == "and" && matches!(popped.get(1), Some(Value::I32(1))) {
                            " Masking with 1 keeps only the lowest bit: compilers do this after a \
                             comparison to turn it into a clean 0 or 1, C's `_Bool`."
                        } else {
                            ""
                        }
                    ),
                )
                .with(concepts::integers())
            },
            | 0x74..=0x78 | 0x86..=0x8A => Step::new(
                "evaluate",
                "Compute",
                &format!("`{operation}` {a} by {b}: {result}"),
                &format!(
                    "`{name}` shifts or rotates the bits of {a}. The count is taken modulo the \
                     bit width, so it can never be too large; in C, shifting by the width or more \
                     is undefined."
                ),
            )
            .with(concepts::integers()),
            | 0x67..=0x69 | 0x79..=0x7B => Step::new(
                "evaluate",
                "Compute",
                &format!("`{operation}` of {a}: {result}"),
                &format!(
                    "`{name}` counts bits: leading zeros (`clz`), trailing zeros (`ctz`) or ones \
                     (`popcnt`). C exposes these as compiler builtins."
                ),
            )
            .with(concepts::integers()),
            | 0xA7..=0xC4 => Step::new(
                "evaluate",
                "Convert",
                &format!("Convert {a} to {result}"),
                &format!("`{name}`: {}", conversion_story(op)),
            )
            .with(concepts::conversions()),
            | _ => Step::new(
                "evaluate",
                "Compute",
                &format!(
                    "`{operation}` {a}{}{b}: {result}",
                    if b.is_empty() { "" } else { ", " }
                ),
                &format!(
                    "`{name}` computes with IEEE 754 floating point, exactly as specified, so \
                     every engine gets the same bits."
                ),
            )
            .with(concepts::floats()),
        }
    }

    #[expect(clippy::too_many_arguments, reason = "The parts of one sentence.")]
    fn arithmetic(
        &self,
        name: &str,
        symbol: &str,
        a: &str,
        b: &str,
        result: &str,
        op: u8,
        pc: u32,
    ) -> Step {
        let float = op >= 0x8B;
        let frame = if op == 0x6B && self.follows_stack_pointer(pc) {
            " This is the prologue computing where the new frame goes: the stack pointer minus the \
             frame's size, since the shadow stack grows down."
        } else {
            ""
        };
        let wrapped = !float && self.wrapped(op);
        Step::new(
            "evaluate",
            "Compute",
            &format!("{a} {symbol} {b} = {result}"),
            &format!(
                "`{name}` pops the two values on top of the operand stack and pushes the \
                 result.{}{frame}",
                if float {
                    " Floating-point arithmetic follows IEEE 754 exactly."
                } else if wrapped {
                    " The true result does not fit in the type, so it wraps around, keeping the \
                     low bits. In C, signed overflow is undefined behavior; in WebAssembly, \
                     integer arithmetic is always defined, modulo 2^N."
                } else {
                    " Integer arithmetic wraps around modulo 2^N: WebAssembly integers have no \
                     sign of their own, and only some instructions (like `div_s`) treat them as \
                     signed."
                }
            ),
        )
        .with(if float {
            concepts::floats()
        } else {
            concepts::integers()
        })
    }

    /// Whether the last integer `add`, `sub` or `mul` overflowed as a signed
    /// operation.
    fn wrapped(&self, op: u8) -> bool {
        let event = &self.machine.event;
        match (event.popped.as_slice(), op) {
            | ([Value::I32(a), Value::I32(b)], 0x6A) => a.checked_add(*b).is_none(),
            | ([Value::I32(a), Value::I32(b)], 0x6B) => a.checked_sub(*b).is_none(),
            | ([Value::I32(a), Value::I32(b)], 0x6C) => a.checked_mul(*b).is_none(),
            | ([Value::I64(a), Value::I64(b)], 0x7C) => a.checked_add(*b).is_none(),
            | ([Value::I64(a), Value::I64(b)], 0x7D) => a.checked_sub(*b).is_none(),
            | ([Value::I64(a), Value::I64(b)], 0x7E) => a.checked_mul(*b).is_none(),
            | _ => false,
        }
    }

    /// Whether instruction `pc` follows `global.get $__stack_pointer`,
    /// `i32.const N`: the prologue's frame computation.
    fn follows_stack_pointer(&self, pc: u32) -> bool {
        let Some(body) = self.session.program().module.body(self.machine.event.func) else {
            return false;
        };
        pc >= 2
            && matches!(body.code[pc as usize - 1], Instr::I32Const(_))
            && matches!(body.code[pc as usize - 2], Instr::GlobalGet(index) if Some(index) == self.machine.stack_pointer)
    }

    /// For a constant right after `global.get $__stack_pointer`: the frame
    /// size.
    fn constant_hint(&self, pc: u32) -> String {
        let Some(body) = self.session.program().module.body(self.machine.event.func) else {
            return String::new();
        };
        let after_pointer = pc >= 1
            && matches!(body.code[pc as usize - 1], Instr::GlobalGet(index) if Some(index) == self.machine.stack_pointer);
        if after_pointer {
            " Right after reading the shadow stack pointer, it is the size of this function's \
             frame: the bytes its C variables need."
                .to_owned()
        } else {
            String::new()
        }
    }

    /// Where `address` is relative to the current call's shadow stack
    /// frame.
    fn frame_hint(&self, address: u64) -> String {
        let Some(pointer) = self.machine.stack_pointer_value() else {
            return "Linear memory is one flat, bounds-checked array of bytes.".to_owned();
        };
        let entry = self
            .machine
            .frames
            .last()
            .and_then(|frame| frame.stack_pointer);
        match entry {
            | Some(entry) if address >= u64::from(pointer) && address < u64::from(entry) =>
                format!(
                    "That address is {} bytes into this call's shadow stack frame: a C variable \
                     of this function.",
                    address - u64::from(pointer)
                ),
            | Some(entry)
                if address >= u64::from(entry)
                    && self.machine.frames.iter().any(|frame| {
                        frame
                            .stack_pointer
                            .is_some_and(|start| address < u64::from(start))
                    }) =>
                "That address is in a caller's shadow stack frame: this function reached it \
                 through a pointer."
                    .to_owned(),
            | _ if address < u64::from(pointer) && u64::from(pointer) - address <= 256 =>
                "That address is just below `__stack_pointer`. This function calls nothing, so the \
                 compiler lets it keep its C variables there without moving the pointer: no call \
                 can run and reuse the space before it returns."
                    .to_owned(),
            | _ => "Linear memory is one flat, bounds-checked array of bytes.".to_owned(),
        }
    }

    fn enter(&self, instr: &Instr, popped: &[Value]) -> Step {
        let depth = self.label_count();
        let (title, story) = match instr {
            | Instr::Block { .. } => (
                format!("Enter block @{depth}"),
                "`block` opens a region. A branch to it jumps forward, out of the region, to its \
                 `end`. WebAssembly has no `goto` and no jumps to addresses: compilers build C's \
                 `if`, `break`, `continue` and early `return` from nested blocks."
                    .to_owned(),
            ),
            | Instr::Loop { .. } => (
                format!("Enter loop @{depth}"),
                "`loop` opens a region too, but a branch to it jumps backward, to its start. A C \
                 `for` or `while` loop becomes a `loop` whose body ends in a conditional branch \
                 back to the top."
                    .to_owned(),
            ),
            | _ => {
                let taken = matches!(self.machine.event.effect, Effect::Enter { taken: true, .. });
                let condition = popped
                    .first()
                    .map_or_else(String::new, |value| format!("`{value}`"));
                (
                    if taken {
                        format!("{condition} is nonzero: take the then-branch")
                    } else {
                        format!("{condition} is zero: skip to the else-branch")
                    },
                    format!(
                        "`if` pops a condition and runs its first branch if it is nonzero, its \
                         `else` branch (if any) otherwise. It opens label @{depth}, so a branch \
                         can leave it early."
                    ),
                )
            },
        };
        Step::new("control", "Control flow", &title, &story).with(concepts::structured())
    }

    fn end(&self) -> Step {
        match self.machine.event.effect {
            | Effect::Exit { kind } => {
                let depth = self.label_count() + 1;
                let what = if kind == LabelKind::Loop {
                    "The loop body ran to its end without branching back, so the loop is over."
                } else {
                    "Execution reached the end of the region normally."
                };
                Step::new(
                    "control",
                    "Control flow",
                    &format!("Leave {} @{depth}", kind.name()),
                    &format!("{what} Its label is gone; the values it produces stay on the stack."),
                )
                .with(concepts::structured())
            },
            | _ => self.returned("Reaching the final `end` of a function returns from it."),
        }
    }

    fn returned(&self, how: &str) -> Step {
        let event = &self.machine.event;
        let Effect::Return { func, discarded } = event.effect else {
            return Step::new("call", "Return", "Return", how);
        };
        let program = self.session.program();
        let name = program.module.func_name(func);
        let result = event
            .pushed
            .first()
            .map_or_else(String::new, |value| format!(" `{value}`"));
        let caller = self.machine.frames.last().map_or_else(
            || "the host".to_owned(),
            |frame| format!("`${}`", program.module.func_name(frame.func)),
        );
        Step::new(
            "call",
            "Return",
            &format!("`${name}` returns{result} to {caller}"),
            &format!(
                "{how} The call's frame, with its locals{}, is discarded, and its result is left \
                 on the caller's operand stack, right where the `call` expected it.",
                if discarded > 0 {
                    format!(
                        " and {discarded} leftover operand{}",
                        plural(discarded as u64)
                    )
                } else {
                    String::new()
                }
            ),
        )
        .with(concepts::calls())
    }

    fn branch(&self, instr: &Instr, popped: &[Value]) -> Step {
        let Effect::Branch {
            depth,
            taken,
            kind,
            index,
            ..
        } = self.machine.event.effect
        else {
            return self.returned(if matches!(instr, Instr::Return) {
                "`return` leaves the function at once, from inside any number of blocks."
            } else {
                "A branch to the function's own label returns from it."
            });
        };
        let target = self
            .instruction
            .rsplit("(;@")
            .next()
            .and_then(|rest| rest.strip_suffix(";)"))
            .map_or_else(|| format!("label {depth}"), |name| format!("@{name}"));
        let destination = if kind == LabelKind::Loop {
            format!("back to the start of loop {target}")
        } else {
            format!("out of {} {target}, past its `end`", kind.name())
        };
        let relative = format!(
            "Branch targets are relative: `{depth}` means {}. ",
            if depth == 0 {
                "the innermost enclosing label".to_owned()
            } else {
                format!(
                    "{depth} label{} further out than the innermost",
                    plural(u64::from(depth))
                )
            }
        );
        match instr {
            | Instr::BrIf(_) => {
                let condition = popped
                    .first()
                    .map_or_else(String::new, |value| format!("`{value}`"));
                if taken {
                    Step::new(
                        "control",
                        "Branch",
                        &format!("{condition} is nonzero: branch {destination}"),
                        &format!(
                            "`br_if` pops a condition and branches only if it is nonzero. \
                             {relative}This is how a C `if`, `while` or `for` test ends up: a \
                             conditional exit from a block, or a conditional jump back in a loop."
                        ),
                    )
                    .with(concepts::labels())
                } else {
                    Step::new(
                        "control",
                        "Branch",
                        &format!("{condition} is zero: don't branch"),
                        &format!(
                            "`br_if` pops a condition and, since it is zero, falls through to the \
                             next instruction instead of branching {destination}."
                        ),
                    )
                    .with(concepts::labels())
                }
            },
            | Instr::BrTable { .. } => Step::new(
                "control",
                "Branch",
                &format!(
                    "Jump table: index `{}` selects {target}",
                    index.unwrap_or_default()
                ),
                &format!(
                    "`br_table` pops an index and branches to the label it selects from a list \
                     (the last is the default, for any index past the end). A C `switch` often \
                     compiles to one. Here it branches {destination}."
                ),
            )
            .with(concepts::labels()),
            | _ => Step::new(
                "control",
                "Branch",
                &format!("Branch {destination}"),
                &format!(
                    "`br` always branches. {relative}Branching out of a block discards the \
                     operands the block pushed, except the values the label carries."
                ),
            )
            .with(concepts::labels()),
        }
    }

    fn call(&self, instr: &Instr, popped: &[Value]) -> Step {
        let program = self.session.program();
        let event = &self.machine.event;
        let arguments: Vec<String> = popped
            .iter()
            .take(popped.len() - usize::from(matches!(instr, Instr::CallIndirect { .. })))
            .map(|value| format!("`{value}`"))
            .collect();
        let with = if arguments.is_empty() {
            String::new()
        } else {
            format!(" with {}", arguments.join(", "))
        };
        match &event.effect {
            | Effect::Host { callee, note } => {
                let name = program.module.func_name(*callee);
                Step::new(
                    "call",
                    "Host call",
                    &format!("Call the host's `{name}`{with}"),
                    &format!(
                        "`${name}` is imported: the module asks its host for it instead of \
                         defining it. The stepper implements a little of WASI, the system \
                         interface C's standard library is built on; this call {note}."
                    ),
                )
                .with(concepts::host())
            },
            | Effect::Call { callee } => {
                let name = program.module.func_name(*callee);
                let indirect = if matches!(instr, Instr::CallIndirect { .. }) {
                    format!(
                        " `call_indirect` found it at index {} of the table: a C function pointer \
                         is such an index.",
                        popped.last().map_or_else(String::new, ToString::to_string)
                    )
                } else {
                    String::new()
                };
                Step::new(
                    "call",
                    "Call",
                    &format!("Call `${name}`{with}"),
                    &format!(
                        "`call` pops the arguments, which become the callee's first locals, its \
                         parameters, and starts a new frame: execution continues at `${name}`'s \
                         first instruction, and resumes here when it returns.{indirect}"
                    ),
                )
                .with(concepts::calls())
            },
            | _ => Step::new("call", "Call", &format!("`{}`", self.instruction), ""),
        }
    }

    /// Labels of the current call, not counting the function's own.
    fn label_count(&self) -> usize {
        self.machine.frames.last().map_or(0, |frame| {
            (self.machine.labels.len() - frame.label_base).saturating_sub(1)
        })
    }

    fn local_name(&self, index: u32) -> String {
        let Some(frame) = self.machine.frames.last() else {
            return format!("local {index}");
        };
        match self.session.program().module.names.local(frame.func, index) {
            | Some(name) => format!("local `${name}`"),
            | None => format!("local {index}"),
        }
    }

    /// How a local relates to the C function.
    fn local_story(&self, index: u32) -> String {
        let Some(frame) = self.machine.frames.last() else {
            return String::new();
        };
        let params = self
            .session
            .program()
            .module
            .func_type(frame.func)
            .map_or(0, |ty| ty.params.len());
        if (index as usize) < params {
            "It is a parameter: the caller's argument arrived here when the call began.".to_owned()
        } else if self.machine.stack_pointer.is_some() {
            "Locals are a call's private variables, numbered after its parameters. A C variable \
             can live in one only if its address is never needed. Compilers differ here: at `-O0` \
             clang keeps every C variable in the shadow stack and uses locals for scratch values \
             such as the frame's address, while bcc keeps a variable in a local unless the program \
             takes its address."
                .to_owned()
        } else {
            "Locals are a call's private variables, numbered after its parameters, and start at \
             zero."
                .to_owned()
        }
    }

    fn global_name(&self, index: u32) -> String {
        self.session
            .program()
            .module
            .global_name(index)
            .map_or_else(|| index.to_string(), |name| format!("`${name}`"))
    }
}

/// A trap: why it happened and what it means.
fn trapped(message: &str, kind: TrapKind) -> Step {
    let why = match kind {
        | TrapKind::Unresolved =>
            "The module imports this function from its host, but the stepper only provides WASI's \
             basic input and output.",
        | TrapKind::MemoryOutOfBounds =>
            "Every load and store is checked against the size of linear memory. A C program that \
             reads past an array usually does not trap, because the bytes beyond it are still \
             inside memory; this access left memory altogether.",
        | TrapKind::DivideByZero | TrapKind::IntegerOverflow =>
            "Where C leaves division by zero (and the one overflowing quotient, INT_MIN / -1) \
             undefined, WebAssembly defines it: a trap.",
        | TrapKind::Unreachable =>
            "`unreachable` marks code that should never run, such as the end of a function the \
             compiler knows does not return.",
        | TrapKind::CallStackExhausted =>
            "The engine's own call stack is bounded; unbounded recursion ends in a trap rather \
             than corrupting memory.",
        | _ =>
            "WebAssembly checks the conditions C leaves undefined, and stops the program when one \
             fails.",
    };
    Step::new(
        "fault",
        "Trap",
        &format!("Trap: {}", first_clause(message)),
        &format!(
            "{} A trap stops the program immediately: nothing after this instruction runs, and \
             the host is told why. {why}",
            sentence(message)
        ),
    )
    .with(concepts::traps())
}

fn plural(count: u64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `message` up to its first colon, for a title.
fn first_clause(message: &str) -> &str {
    message.split(':').next().unwrap_or(message)
}

fn sentence(message: &str) -> String {
    let mut text = capitalize(message);
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

fn type_description(ty: &str) -> &'static str {
    match ty {
        | "i64" => "64-bit integer (C's `long long`)",
        | "f32" => "32-bit float (C's `float`)",
        | "f64" => "64-bit float (C's `double`)",
        | _ => "32-bit integer (C's `int`, and every pointer in 32-bit WebAssembly)",
    }
}

fn extension(load: Load) -> &'static str {
    match load {
        | Load::I32_8S | Load::I32_16S | Load::I64_8S | Load::I64_16S | Load::I64_32S =>
            " The bytes are sign-extended to fill the result, as reading a C `signed char` or \
             `short` promotes it to `int`.",
        | Load::I32_8U | Load::I32_16U | Load::I64_8U | Load::I64_16U | Load::I64_32U =>
            " The bytes are zero-extended to fill the result, as for an `unsigned char`.",
        | _ => "",
    }
}

fn conversion_story(op: u8) -> &'static str {
    match op {
        | 0xA7 => "keeps the low 32 bits, like C's cast from `long long` to `int`.",
        | 0xA8..=0xAB | 0xAE..=0xB1 =>
            "truncates toward zero, like C's cast to an integer type, but traps if the value does \
             not fit (where C leaves it undefined).",
        | 0xAC => "sign-extends to 64 bits, as C converts an `int` to `long long`.",
        | 0xAD => "zero-extends to 64 bits, as C converts an `unsigned` to `unsigned long long`.",
        | 0xB2..=0xBA => "converts to the nearest representable floating-point value.",
        | 0xBB => "widens the float exactly.",
        | 0xBC..=0xBF => "reinterprets the bits as another type without changing them.",
        | 0xC0 | 0xC2 => "sign-extends the low 8 bits, as converting a `signed char` does.",
        | 0xC1 | 0xC3 => "sign-extends the low 16 bits, as converting a `short` does.",
        | _ => "sign-extends the low 32 bits.",
    }
}

/// The ideas the narration refers to, with the specification sections that
/// define them (WebAssembly Core Specification 2.0).
mod concepts {
    use crate::wasm::Concept;

    fn concept(title: &str, gloss: &str, section: &str, citation: &str) -> Concept {
        Concept {
            title:    title.to_owned(),
            gloss:    gloss.to_owned(),
            section:  section.to_owned(),
            citation: citation.to_owned(),
        }
    }

    pub fn stack() -> Concept {
        concept(
            "A stack machine",
            "Instructions take their operands from the top of an operand stack and push their \
             results there; there are no registers to name. Validation proves every instruction \
             finds operands of the right types.",
            "Wasm §2.4",
            "WebAssembly Core Specification 2.0, §2.4 Instructions",
        )
    }

    pub fn integers() -> Concept {
        concept(
            "Integers wrap",
            "i32 and i64 arithmetic is modulo 2^N. Integers are just bits: the instruction \
             decides whether to treat them as signed (`_s`) or unsigned (`_u`).",
            "Wasm §4.3.2",
            "WebAssembly Core Specification 2.0, §4.3.2 Integer Operations",
        )
    }

    pub fn floats() -> Concept {
        concept(
            "IEEE 754 floats",
            "f32 and f64 follow IEEE 754-2019 with round-to-nearest, so results are the same bits \
             on every engine.",
            "Wasm §4.3.3",
            "WebAssembly Core Specification 2.0, §4.3.3 Floating-Point Operations",
        )
    }

    pub fn conversions() -> Concept {
        concept(
            "Conversions",
            "Each conversion names both types. Truncating a float that does not fit traps; the \
             `_sat` forms clamp instead.",
            "Wasm §4.3.4",
            "WebAssembly Core Specification 2.0, §4.3.4 Conversions",
        )
    }

    pub fn locals() -> Concept {
        concept(
            "Locals",
            "Each call has its own numbered locals: the parameters first, then the declared \
             locals, all starting at zero. They have no address.",
            "Wasm §4.4.5",
            "WebAssembly Core Specification 2.0, §4.4.5 Variable Instructions",
        )
    }

    pub fn globals() -> Concept {
        concept(
            "Globals",
            "Module-wide variables, each mutable or not. Toolchains use a few of them for \
             addresses in linear memory.",
            "Wasm §4.4.5",
            "WebAssembly Core Specification 2.0, §4.4.5 Variable Instructions",
        )
    }

    pub fn shadow_stack() -> Concept {
        concept(
            "The shadow stack",
            "C's stack frames live in linear memory, below the `__stack_pointer` global, so that \
             C objects have addresses. It is a convention between C compilers, not part of \
             WebAssembly.",
            "C ABI",
            "WebAssembly tool conventions, Basic C ABI: the stack pointer global",
        )
    }

    pub fn memory() -> Concept {
        concept(
            "Linear memory",
            "One flat array of bytes, in 64 KiB pages. Addresses are i32 offsets into it, and \
             every access is bounds checked, so a wild pointer can corrupt the program's own data \
             but never escape it.",
            "Wasm §4.4.7",
            "WebAssembly Core Specification 2.0, §4.4.7 Memory Instructions",
        )
    }

    pub fn structured() -> Concept {
        concept(
            "Structured control flow",
            "`block`, `loop` and `if` nest like braces. Branches can only leave enclosing blocks \
             or restart enclosing loops, so the code can be validated in one pass.",
            "Wasm §4.4.8",
            "WebAssembly Core Specification 2.0, §4.4.8 Control Instructions",
        )
    }

    pub fn labels() -> Concept {
        concept(
            "Labels count outward",
            "A branch names its target by depth: 0 is the innermost enclosing block, 1 the one \
             around it. The listing's `@N` comments name each block by nesting level instead.",
            "Wasm §4.4.8",
            "WebAssembly Core Specification 2.0, §4.4.8 Control Instructions (br)",
        )
    }

    pub fn calls() -> Concept {
        concept(
            "Calls and frames",
            "A call takes its arguments from the operand stack and gets a fresh frame of locals; \
             the engine keeps the return address itself, out of the program's reach.",
            "Wasm §4.4.10",
            "WebAssembly Core Specification 2.0, §4.4.10 Function Calls",
        )
    }

    pub fn host() -> Concept {
        concept(
            "Imports and the host",
            "A module can only touch the outside world through functions its host provides. WASI \
             defines a standard set: write to a file descriptor, read the clock, exit.",
            "Wasm §2.5.11",
            "WebAssembly Core Specification 2.0, §2.5.11 Imports",
        )
    }

    pub fn tables() -> Concept {
        concept(
            "Tables",
            "Arrays of function references. C function pointers are indices into one, and \
             `call_indirect` checks the callee's type at run time.",
            "Wasm §4.4.6",
            "WebAssembly Core Specification 2.0, §4.4.6 Table Instructions",
        )
    }

    pub fn traps() -> Concept {
        concept(
            "Traps",
            "An instruction whose rules cannot be satisfied (an access out of bounds, a division \
             by zero) aborts the whole program. WebAssembly has no undefined behavior.",
            "Wasm §4.4",
            "WebAssembly Core Specification 2.0, §4.4 Instructions (execution)",
        )
    }

    pub fn instantiation() -> Concept {
        concept(
            "Instantiation",
            "Turning a module into a running instance: allocate memory, tables and globals, copy \
             data segments in, and run the start function.",
            "Wasm §4.5.4",
            "WebAssembly Core Specification 2.0, §4.5.4 Instantiation",
        )
    }
}
