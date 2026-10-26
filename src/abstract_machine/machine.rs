//! One-operation execution over real activations and an object-aware operand
//! stack. Each operation reports an [`Event`] that `narration.rs` explains.
//!
//! C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (lifetime).
//! C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25 (main fallthrough).
//! C99: §6.9.1 paragraph 12, p. 142; PDF p. 154 (other fallthrough;
//! conservatively rejected regardless of whether its value is used).

use std::mem::size_of;

use super::{
    model::{
        Diagnostic,
        Evaluation,
        Event,
        Frame,
        Instruction,
        MAX_FRAMES,
        MAX_OPERANDS,
        Op,
        Operand,
        Program,
        State,
        Type,
        Value,
    },
    narration,
};

#[derive(Debug, Clone)]
pub(super) struct Machine {
    pub state:       State,
    pub activations: Vec<Activation>,
    pub next_frame:  usize,
    pub cells:       usize,
    pub metadata:    Vec<ObjectMetadata>,
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
        let top = self.activations.len() - 1;
        self.activations[top].pc += 1;
        self.track(program, &instruction);
        match self.operation(&instruction, program) {
            | Ok(event) => {
                if let (Some(node), Some(result)) = (instruction.node, &event.result)
                    && !matches!(instruction.op, Op::Call(..) | Op::Return | Op::Fallthrough)
                {
                    self.record(node, result.clone());
                }
                self.state.event = event;
            },
            | Err(diagnostic) => self.stop(diagnostic),
        }
        self.refresh(program);
    }

    fn operation(
        &mut self,
        instruction: &Instruction,
        program: &Program,
    ) -> Result<Event, Diagnostic> {
        let top = self.activations.len() - 1;
        let range = instruction.range;
        let source = &program.source;
        Ok(match &instruction.op {
            | Op::Enter => {
                self.activations[top].scope += 1;
                narration::enter_block(range)
            },
            | Op::Leave => {
                let frame = &self.activations[top];
                let mut ended = Vec::new();
                for object in &mut self.state.objects {
                    if object.frame_id == frame.id
                        && object.lifetime == "alive"
                        && self.metadata[object.id - 1].scope == frame.scope
                    {
                        object.lifetime = "ended";
                        ended.push(object.id);
                    }
                }
                self.activations[top].scope -= 1;
                self.leave_block(range, ended)
            },
            | Op::Declare(name, ty) => {
                let id = self.allocate(
                    name,
                    *ty,
                    self.activations[top].scope,
                    Value::Indeterminate,
                    false,
                )?;
                self.declare(range, id, *ty)
            },
            | Op::Push(integer) => {
                let value = Operand::Value(Value::Integer(*integer));
                self.push(value.clone())?;
                narration::constant(range, *integer, value)
            },
            | Op::Zero => {
                let value = Operand::Value(Value::Integer(0));
                self.push(value.clone())?;
                narration::implicit_zero(range, value)
            },
            | Op::Address(name) => {
                let id = self.named(name)?;
                let operand = Operand::Address {
                    target: id,
                    offset: 0,
                };
                self.push(operand.clone())?;
                self.designate(range, id, operand)
            },
            | Op::Element(name, index) => {
                let id = self.named(name)?;
                let operand = Operand::Address {
                    target: id,
                    offset: *index,
                };
                self.push(operand.clone())?;
                self.element(range, id, *index, operand)
            },
            | Op::Decay(name) => {
                let id = self.named(name)?;
                let value = Value::Pointer {
                    target: Some(id),
                    offset: 0,
                };
                self.push(Operand::Value(value.clone()))?;
                self.decay(range, id, value)
            },
            | Op::Load => {
                let (target, offset) = self.pop_address()?;
                let value = self.read(target, offset)?;
                self.push(Operand::Value(value.clone()))?;
                self.load(range, target, offset, value)
            },
            | Op::Promote => {
                let value = self.pop_value()?;
                self.push(Operand::Value(value.clone()))?;
                self.promote(range, value)
            },
            | Op::AddressOf => {
                let (target, offset) = self.pop_address()?;
                let value = Value::Pointer {
                    target: Some(target),
                    offset,
                };
                self.push(Operand::Value(value.clone()))?;
                self.address_of(range, target, offset, value)
            },
            | Op::Dereference => {
                let pointer = self.pop_value()?;
                let (target, offset) = self.dereference(&pointer)?;
                self.push(Operand::Address { target, offset })?;
                self.indirection(range, source, pointer, target, offset)
            },
            | Op::Index => {
                let index = self.pop_value()?;
                let base = self.pop_value()?;
                let pointer = self.binary_value("+", base.clone(), index.clone())?;
                let (target, offset) = self.dereference(&pointer)?;
                self.push(Operand::Address { target, offset })?;
                self.subscript(range, base, index, target, offset)
            },
            | Op::Unary(operator) => {
                let value = self.pop_value()?;
                let result = match (operator, &value) {
                    | (&"+", Value::Integer(integer)) => Value::Integer(*integer),
                    | (&"-", Value::Integer(integer)) => Value::Integer(
                        integer
                            .checked_neg()
                            .ok_or_else(|| self.overflow(None, "-", *integer))?,
                    ),
                    | (&"!", value) => Value::Integer(i32::from(!self.truth(value)?)),
                    | _ => return Err(self.error("unsupported unary operand")),
                };
                self.push(Operand::Value(result.clone()))?;
                self.unary(range, operator, value, result)
            },
            | Op::Binary(operator) => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let result = self.binary_value(operator, left.clone(), right.clone())?;
                self.push(Operand::Value(result.clone()))?;
                self.binary(range, source, operator, left, right, result)
            },
            | &Op::Store {
                initializer,
                full_expression,
            } => {
                let value = self.pop_value()?;
                let (target, offset) = self.pop_address()?;
                let previous = self.peek(target, offset);
                let stored = self.store(target, offset, value.clone())?;
                self.stored(
                    range,
                    initializer,
                    full_expression,
                    target,
                    offset,
                    previous,
                    value,
                    stored,
                )
            },
            | &Op::Increment { delta, prefix } => {
                let (target, offset) = self.pop_address()?;
                let current = self.read(target, offset)?;
                let sum = self.binary_value("+", current.clone(), Value::Integer(delta))?;
                let stored = self.store(target, offset, sum.clone())?;
                self.increment(
                    range, source, delta, prefix, target, offset, current, &sum, stored,
                )
            },
            | Op::Pop => {
                let value = self.pop_value()?;
                self.discard(range, source, value)
            },
            | &Op::Jump(target) => {
                let backward = target < self.activations[top].pc;
                self.activations[top].pc = target;
                narration::jump(range, backward)
            },
            | &Op::Branch(target, statement) => {
                let value = self.pop_value()?;
                let truth = self.truth(&value)?;
                if !truth {
                    self.activations[top].pc = target;
                }
                self.branch(range, source, statement, value, truth)
            },
            | Op::Call(name, count) => self.call(program, instruction, name, *count)?,
            | Op::Return => {
                let value = self.pop_value()?;
                self.return_from(program, range, value, false)?
            },
            | Op::Fallthrough =>
                if self.activations[top].function == program.entry {
                    self.return_from(program, range, Value::Integer(0), true)?
                } else {
                    return Err(self.error(
                        "reaching the end of a non-main function is outside the subset; write an \
                         explicit return",
                    ));
                },
        })
    }

    fn call(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        name: &str,
        count: usize,
    ) -> Result<Event, Diagnostic> {
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
        let recursive = self
            .activations
            .iter()
            .any(|activation| activation.function == function);
        let caller = self.activations.len() - 1;
        if let Some(evaluation) = &mut self.activations[caller].evaluation {
            evaluation.calling = instruction.node;
        }
        let id = self.next_frame;
        self.next_frame += 1;
        self.activations.push(Activation {
            id,
            function,
            pc: 0,
            scope: 0,
            operands: Vec::new(),
            evaluation: None,
        });
        let mut parameters = Vec::new();
        for ((name, ty), value) in program.functions[function]
            .parameters
            .iter()
            .zip(arguments.iter().cloned())
        {
            parameters.push(self.allocate(name, *ty, 1, value, true)?);
        }
        Ok(self.called(
            instruction.range,
            name,
            id,
            recursive,
            &arguments,
            parameters,
        ))
    }

    fn return_from(
        &mut self,
        program: &Program,
        range: super::SourceRange,
        value: Value,
        fallthrough: bool,
    ) -> Result<Event, Diagnostic> {
        if !matches!(value, Value::Integer(_)) {
            return Err(self.error("int function returned a non-int value"));
        }
        let top = self.activations.len() - 1;
        let id = self.activations[top].id;
        let function = program.functions[self.activations[top].function]
            .name
            .clone();
        let mut ended = Vec::new();
        for object in &mut self.state.objects {
            if object.frame_id == id && object.lifetime == "alive" {
                object.lifetime = "ended";
                ended.push(object.id);
            }
        }
        let _frame = self.activations.pop();
        let call = if self.activations.is_empty() {
            self.state.return_value = Some(value.clone());
            self.state.status = "finished";
            None
        } else {
            self.push(Operand::Value(value.clone()))?;
            let caller = self.activations.len() - 1;
            let node = self.activations[caller]
                .evaluation
                .as_mut()
                .and_then(|evaluation| evaluation.calling.take());
            if let Some(node) = node {
                self.record(node, Operand::Value(value.clone()));
            }
            node.map(|node| {
                let caller = &program.functions[self.activations[caller].function];
                let range = caller.nodes[node].range;
                &program.source[range.start..range.end]
            })
        };
        Ok(self.returned(range, &function, id, value, ended, fallthrough, call))
    }

    /// Start tracking the full expression `instruction` belongs to in the
    /// innermost activation; statement-level operations clear it.
    fn track(&mut self, program: &Program, instruction: &Instruction) {
        let top = self.activations.len() - 1;
        let activation = &mut self.activations[top];
        let Some(node) = instruction.node else {
            if matches!(
                instruction.op,
                Op::Enter | Op::Leave | Op::Declare(..) | Op::Jump(_)
            ) {
                activation.evaluation = None;
            } else if let Some(evaluation) = &mut activation.evaluation {
                evaluation.last = None;
            }
            return;
        };
        let function = activation.function;
        let root = program.functions[function].nodes[node].root;
        if activation
            .evaluation
            .as_ref()
            .is_none_or(|evaluation| evaluation.root != root)
        {
            activation.evaluation = Some(Evaluation {
                function,
                root,
                values: Vec::new(),
                last: None,
                calling: None,
            });
        }
        if let Some(evaluation) = &mut activation.evaluation {
            evaluation.last = Some(node);
        }
    }

    /// Record `node`'s value in the innermost activation's full expression.
    fn record(&mut self, node: usize, operand: Operand) {
        let top = self.activations.len() - 1;
        if let Some(evaluation) = &mut self.activations[top].evaluation {
            evaluation.values.retain(|(entry, _)| *entry != node);
            evaluation.values.push((node, operand));
        }
    }

    pub fn refresh(&mut self, program: &Program) {
        self.state.frames.clear();
        for (depth, frame) in self.activations.iter().enumerate() {
            let nodes = &program.functions[frame.function].nodes;
            self.state.frames.push(Frame {
                id: frame.id,
                function: program.functions[frame.function].name.clone(),
                depth,
                waiting: frame
                    .evaluation
                    .as_ref()
                    .and_then(|evaluation| evaluation.calling)
                    .and_then(|node| nodes.get(node))
                    .map(|node| node.range),
            });
        }
        self.state.evaluation = self
            .activations
            .last()
            .and_then(|activation| activation.evaluation.clone());
        if self.state.terminal() {
            return;
        }
        let frame = &self.activations[self.activations.len() - 1];
        if let Some(next) = program.functions[frame.function].code.get(frame.pc) {
            self.state.source_range = next.range;
            if let Some(effect) = self.pending_effect(&next.op) {
                self.state.pending_effects.push(effect);
            }
        }
    }

    pub fn new(program: &Program) -> Self {
        let activations = Vec::from([Activation {
            id:         1,
            function:   program.entry,
            pc:         0,
            scope:      0,
            operands:   Vec::new(),
            evaluation: None,
        }]);
        let first = program.functions[program.entry].code[0].range;
        let mut machine = Self {
            state: State {
                step:            0,
                status:          "ready",
                source_range:    first,
                event:           narration::start(first),
                evaluation:      None,
                frames:          Vec::new(),
                objects:         Vec::new(),
                pending_effects: Vec::new(),
                diagnostics:     Vec::new(),
                return_value:    None,
            },
            activations,
            next_frame: 2,
            cells: 0,
            metadata: Vec::new(),
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
        self.state.event = narration::fault(&diagnostic);
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
            | Operand::Address { .. } => Err(self.error("expected an evaluated value")),
        }
    }

    pub fn pop_address(&mut self) -> Result<(usize, i32), Diagnostic> {
        match self.pop()? {
            | Operand::Address { target, offset } => Ok((target, offset)),
            | Operand::Value(_) => Err(self.error("expected a modifiable scalar lvalue")),
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
    // array contents, names, activation operands, expression progress and the
    // event text. A final terminal state can exceed the 8 MiB threshold by one
    // bounded state.
    pub fn retention_cost(&self) -> usize {
        let mut bytes = size_of::<Self>()
            + self.metadata.capacity() * size_of::<ObjectMetadata>()
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
            if let Some(evaluation) = &frame.evaluation {
                bytes += 2 * evaluation.values.capacity() * size_of::<(usize, Operand)>();
            }
        }
        for frame in &self.state.frames {
            bytes += frame.function.capacity();
        }
        let event = &self.state.event;
        bytes += event.title.capacity()
            + event.explanation.capacity()
            + event.clauses.capacity() * size_of::<super::Clause>()
            + event.operands.capacity() * size_of::<Operand>()
            + event.objects.capacity() * size_of::<usize>()
            + self.state.pending_effects.capacity() * size_of::<String>();
        for effect in &self.state.pending_effects {
            bytes += effect.capacity();
        }
        bytes + 1024 // Diagnostic storage and allocator bookkeeping reserve.
    }
}

#[derive(Debug, Clone)]
pub(super) struct Activation {
    pub id:         usize,
    pub function:   usize,
    pub pc:         usize,
    pub scope:      usize,
    pub operands:   Vec<Operand>,
    /// The full expression this activation is evaluating.
    pub evaluation: Option<Evaluation>,
}

#[derive(Debug, Clone)]
pub(super) struct ObjectMetadata {
    pub scope: usize,
    pub ty:    Type,
}
