//! One-operation execution over real activations and an object-aware operand
//! stack. C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (lifetime).

use std::{
    fmt::{
        self,
        Write,
    },
    mem::size_of,
};

use super::model::{
    Diagnostic,
    Frame,
    Instruction,
    MAX_FRAMES,
    MAX_OPERANDS,
    Op,
    Program,
    State,
    Value,
};

#[derive(Debug, Clone)]
pub(super) struct Machine {
    pub state:       State,
    pub activations: Vec<Activation>,
    pub next_frame:  usize,
    pub cells:       usize,
}

impl Machine {
    pub fn execute(&mut self, program: &Program) {
        let frame = &self.activations[self.activations.len() - 1];
        let Some(instruction) = program.functions[frame.function]
            .code
            .get(frame.pc)
            .cloned()
        else {
            self.stop(self.error("instruction pointer outside compiled code"));
            return;
        };
        self.state.source_range = instruction.range;
        self.state.status = "running";
        self.state.pending_effects.clear();
        self.state.event = text(format_args!("{}", operation_name(&instruction.op)));
        let top = self.activations.len() - 1;
        self.activations[top].pc += 1;
        if let Err(diagnostic) = self.operation(&instruction, program) {
            self.stop(diagnostic);
        }
        self.refresh(program);
    }

    fn operation(
        &mut self,
        instruction: &Instruction,
        program: &Program,
    ) -> Result<(), Diagnostic> {
        let top = self.activations.len() - 1;
        match &instruction.op {
            | Op::Enter => {
                self.activations[top].scope += 1;
            },
            | Op::Leave => {
                let frame = &self.activations[top];
                for object in &mut self.state.objects {
                    if object.frame_id == frame.id && object.scope == frame.scope {
                        object.lifetime = "ended";
                    }
                }
                self.activations[top].scope -= 1;
            },
            | Op::Declare(name, ty) => {
                self.allocate(name, *ty, self.activations[top].scope, Value::Indeterminate)?;
            },
            | Op::Push(integer) => self.push(Operand::Value(Value::Integer(*integer)))?,
            | Op::Address(name) => {
                let id = self.named(name)?;
                self.push(Operand::Address {
                    target: id,
                    offset: 0,
                })?;
            },
            | Op::Decay(name) => {
                let id = self.named(name)?;
                self.push(Operand::Value(Value::Pointer {
                    target: Some(id),
                    offset: 0,
                }))?;
            },
            | Op::Load => {
                let (target, offset) = self.pop_address()?;
                let value = self.read(target, offset)?;
                self.push(Operand::Value(value))?;
            },
            | Op::AddressOf => {
                let (target, offset) = self.pop_address()?;
                self.push(Operand::Value(Value::Pointer {
                    target: Some(target),
                    offset,
                }))?;
            },
            | Op::Dereference => {
                let pointer = self.pop_value()?;
                let (target, offset) = self.dereference(&pointer)?;
                self.push(Operand::Address { target, offset })?;
            },
            | Op::Index => {
                let index = self.pop_value()?;
                let base = self.pop_value()?;
                let pointer = self.binary_value("+", base, index)?;
                let (target, offset) = self.dereference(&pointer)?;
                self.push(Operand::Address { target, offset })?;
            },
            | Op::Unary(operator) => {
                let value = self.pop_value()?;
                let result = match (operator, value) {
                    | (&"+", Value::Integer(integer)) => Value::Integer(integer),
                    | (&"-", Value::Integer(integer)) =>
                        Value::Integer(integer.checked_neg().ok_or_else(|| self.overflow())?),
                    | (&"!", value) => Value::Integer(i32::from(!self.truth(&value)?)),
                    | _ => return Err(self.error("unsupported unary operand")),
                };
                self.push(Operand::Value(result))?;
            },
            | Op::Binary(operator) => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let result = self.binary_value(operator, left, right)?;
                self.push(Operand::Value(result))?;
            },
            | Op::Store => {
                let value = self.pop_value()?;
                let (target, offset) = self.pop_address()?;
                self.store(target, offset, value)?;
                self.state.event = text(format_args!("stored object {target} at element {offset}"));
            },
            | Op::Increment(delta) => {
                let (target, offset) = self.pop_address()?;
                let current = self.read(target, offset)?;
                let result = self.binary_value("+", current, Value::Integer(*delta))?;
                self.store(target, offset, result)?;
            },
            | Op::Pop => {
                self.pop_value()?;
            },
            | Op::Jump(target) => self.activations[top].pc = *target,
            | Op::Branch(target) => {
                let value = self.pop_value()?;
                if !self.truth(&value)? {
                    self.activations[top].pc = *target;
                }
            },
            | Op::Call(name, count) => self.call(program, name, *count)?,
            | Op::Return => {
                let value = self.pop_value()?;
                self.return_from(value)?;
            },
            | Op::Fallthrough =>
                if self.activations[top].function == program.entry {
                    self.return_from(Value::Integer(0))?;
                } else {
                    return Err(self.error(
                        "reaching the end of a non-main function is outside the subset; write an \
                         explicit return",
                    ));
                },
        }
        Ok(())
    }

    fn call(&mut self, program: &Program, name: &str, count: usize) -> Result<(), Diagnostic> {
        if self.activations.len() >= MAX_FRAMES {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "call stack limit reached",
            ));
        }
        let Some(function) = program
            .functions
            .iter()
            .position(|function| function.name == name)
        else {
            return Err(self.error("unknown function"));
        };
        let mut arguments = Vec::new();
        for _ in 0..count {
            arguments.push(self.pop_value()?);
        }
        arguments.reverse();
        let id = self.next_frame;
        self.next_frame += 1;
        self.activations.push(Activation {
            id,
            function,
            pc: 0,
            scope: 0,
            operands: Vec::new(),
        });
        for ((name, ty), value) in program.functions[function].parameters.iter().zip(arguments) {
            self.allocate(name, *ty, 1, value)?;
        }
        self.state.event = text(format_args!("called {name}, frame {id}"));
        Ok(())
    }

    fn return_from(&mut self, value: Value) -> Result<(), Diagnostic> {
        if !matches!(value, Value::Integer(_)) {
            return Err(self.error("int function returned a non-int value"));
        }
        let top = self.activations.len() - 1;
        let id = self.activations[top].id;
        for object in &mut self.state.objects {
            if object.frame_id == id {
                object.lifetime = "ended";
            }
        }
        self.activations.pop();
        self.state.event = text(format_args!("returned from frame {id}"));
        if self.activations.is_empty() {
            self.state.return_value = Some(value);
            self.state.status = "finished";
        } else {
            self.push(Operand::Value(value))?;
        }
        Ok(())
    }

    pub fn refresh(&mut self, program: &Program) {
        self.state.frames.clear();
        for (depth, frame) in self.activations.iter().enumerate() {
            self.state.frames.push(Frame {
                id: frame.id,
                function: program.functions[frame.function].name.clone(),
                depth,
            });
        }
        if self.state.terminal() {
            return;
        }
        let frame = &self.activations[self.activations.len() - 1];
        if let Some(next) = program.functions[frame.function].code.get(frame.pc) {
            self.state.source_range = next.range;
            if matches!(next.op, Op::Store | Op::Increment(_)) {
                self.state.pending_effects.push(String::from(
                    "next operation commits a write to the selected object",
                ));
            }
        }
    }

    pub fn new(program: &Program) -> Self {
        let mut activations = Vec::new();
        activations.push(Activation {
            id:       1,
            function: program.entry,
            pc:       0,
            scope:    0,
            operands: Vec::new(),
        });
        let mut machine = Self {
            state: State {
                step:            0,
                status:          "ready",
                source_range:    program.functions[program.entry].code[0].range,
                event:           String::from("entered main; no operations executed"),
                frames:          Vec::new(),
                objects:         Vec::new(),
                pending_effects: Vec::new(),
                diagnostics:     Vec::new(),
                return_value:    None,
            },
            activations,
            next_frame: 2,
            cells: 0,
        };
        machine.refresh(program);
        machine
    }

    pub fn stop(&mut self, diagnostic: Diagnostic) {
        self.state.status = match diagnostic.kind {
            | "undefined_behavior" => "undefined_behavior",
            | "limit" => "limit",
            | _ => "error",
        };
        self.state.source_range = diagnostic.source_range;
        self.state.event.clone_from(&diagnostic.message);
        self.state.pending_effects.clear();
        self.state.diagnostics.clear();
        self.state.diagnostics.push(diagnostic);
    }

    pub fn error(&self, message: &str) -> Diagnostic {
        Diagnostic::new("unsupported", self.state.source_range, message)
    }

    pub fn push(&mut self, operand: Operand) -> Result<(), Diagnostic> {
        if self
            .activations
            .iter()
            .map(|frame| frame.operands.len())
            .sum::<usize>()
            >= MAX_OPERANDS
        {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "operand stack limit reached",
            ));
        }
        let top = self.activations.len() - 1;
        self.activations[top].operands.push(operand);
        Ok(())
    }

    pub fn pop_value(&mut self) -> Result<Value, Diagnostic> {
        match self.pop()? {
            | Operand::Value(value) => Ok(value),
            | _ => Err(self.error("expected an evaluated value")),
        }
    }

    pub fn pop_address(&mut self) -> Result<(usize, i32), Diagnostic> {
        match self.pop()? {
            | Operand::Address { target, offset } => Ok((target, offset)),
            | _ => Err(self.error("expected a modifiable scalar lvalue")),
        }
    }

    fn pop(&mut self) -> Result<Operand, Diagnostic> {
        let top = self.activations.len() - 1;
        self.activations[top]
            .operands
            .pop()
            .ok_or_else(|| self.error("empty operand stack"))
    }

    // Conservative heap-retention accounting includes capacity slack, nested
    // array contents, names, activation operands and observation frames. A
    // final terminal state can exceed the 8 MiB threshold by one bounded state.
    pub fn retention_cost(&self) -> usize {
        let mut bytes = size_of::<Self>()
            + self.state.objects.capacity() * size_of::<super::Object>()
            + self.activations.capacity() * size_of::<Activation>()
            + self.state.frames.capacity() * size_of::<Frame>();
        for object in &self.state.objects {
            bytes += object.name.capacity() + object.r#type.capacity();
            if let Value::Array(elements) = &object.value {
                bytes += elements.capacity() * size_of::<Value>();
            }
        }
        for frame in &self.activations {
            bytes += frame.operands.capacity() * size_of::<Operand>();
        }
        for frame in &self.state.frames {
            bytes += frame.function.capacity();
        }
        bytes += self.state.event.capacity()
            + self.state.pending_effects.capacity() * size_of::<String>();
        for effect in &self.state.pending_effects {
            bytes += effect.capacity();
        }
        bytes + 1024 // Diagnostic storage and allocator bookkeeping reserve.
    }
}

#[derive(Debug, Clone)]
pub(super) struct Activation {
    pub id:       usize,
    pub function: usize,
    pub pc:       usize,
    pub scope:    usize,
    pub operands: Vec<Operand>,
}

#[derive(Debug, Clone)]
pub(super) enum Operand {
    Value(Value),
    Address { target: usize, offset: i32 },
}

pub(super) fn text(arguments: fmt::Arguments<'_>) -> String {
    let mut output = String::new();
    output
        .write_fmt(arguments)
        .expect("writing into a String is infallible");
    output
}

fn operation_name(op: &Op) -> &'static str {
    match op {
        | Op::Enter => "entered block scope",
        | Op::Leave => "ended block scope",
        | Op::Declare(_, _) => "allocated indeterminate automatic storage",
        | Op::Push(_) => "evaluated integer constant",
        | Op::Address(_) => "selected an object",
        | Op::Load => "read stored value",
        | Op::Decay(_) => "array converted to pointer to first element",
        | Op::AddressOf => "formed pointer to object",
        | Op::Dereference => "selected pointer target",
        | Op::Index => "selected array element",
        | Op::Unary(_) => "evaluated unary operator",
        | Op::Binary(_) => "evaluated binary operator",
        | Op::Store => "committed store",
        | Op::Increment(_) => "committed increment",
        | Op::Pop => "completed expression statement",
        | Op::Jump(_) => "continued control flow",
        | Op::Branch(_) => "tested condition",
        | Op::Call(_, _) => "entered function",
        | Op::Return | Op::Fallthrough => "returned from function",
    }
}
