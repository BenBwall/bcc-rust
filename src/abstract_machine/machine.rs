//! One-operation execution over real activations and an object-aware operand
//! stack. Each operation reports an [`Event`] that `narration.rs` explains.
//!
//! C99: §6.2.4 paragraph 2, p. 32; PDF p. 44 (lifetime).
//! C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25 (main fallthrough).
//! C99: §6.9.1 paragraph 12, p. 142; PDF p. 154 (other fallthrough;
//! conservatively rejected regardless of whether its value is used).

use std::{
    mem::size_of,
    sync::Arc,
};

use super::{
    clauses,
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
        Place,
        Program,
        State,
        Structure,
        Type,
        Undefined,
        Value,
    },
    narration::{
        self,
        Designation,
    },
    parser,
};

#[derive(Debug, Clone)]
pub(super) struct Machine {
    pub state:        State,
    pub activations:  Vec<Activation>,
    pub next_frame:   usize,
    pub cells:        usize,
    pub metadata:     Vec<ObjectMetadata>,
    /// The program's structure types, for layouts and member names.
    pub structures:   Arc<[Structure]>,
    /// How many of the program's directives have run: each is a step of
    /// its own, before the static objects exist.
    pub preprocessed: usize,
    /// How many of the program's static objects exist so far: the machine
    /// creates and initializes them, one per step, before `main` runs.
    pub initialized:  usize,
    /// Whether `main` has begun: its first operation has run.
    pub started:      bool,
}

impl Machine {
    pub fn execute(&mut self, program: &Program) {
        self.state.written = 0;
        if self.preprocessed < program.directives.len() {
            let index = self.preprocessed;
            self.preprocessed += 1;
            self.state.status = "running";
            self.state.source_range = program.directives[index].range;
            self.state.event = narration::directive(program, index);
            self.refresh(program);
            return;
        }
        if self.initialized < program.statics.len() {
            let index = self.initialized;
            self.initialized += 1;
            self.state.status = "running";
            self.state.source_range = program.statics[index].range;
            match self.allocate_static(program, index) {
                | Ok(id) => {
                    let mut event = self.static_storage(program, index, id);
                    let declared = &program.statics[index];
                    for &used in &declared.macros {
                        event = narration::macro_use(
                            event,
                            &program.macros[used],
                            &program.source,
                            declared.literal,
                        );
                    }
                    self.state.event = event;
                },
                | Err(diagnostic) => self.stop(diagnostic),
            }
            self.refresh(program);
            return;
        }
        self.started = true;
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
            | Ok(mut event) => {
                if let Some(used) = instruction.macro_use {
                    let lead = !matches!(instruction.op, Op::Declare { .. });
                    event =
                        narration::macro_use(event, &program.macros[used], &program.source, lead);
                }
                if let (Some(node), Some(result)) = (instruction.node, &event.result)
                    && !matches!(
                        instruction.op,
                        Op::Call(..) | Op::Return(_) | Op::Fallthrough | Op::StaticLocal(_)
                    )
                {
                    self.record(node, result.clone());
                }
                self.state.event = event;
            },
            | Err(diagnostic) => {
                let subscript = diagnostic.undefined == Some(Undefined::NullSubscript);
                self.stop(diagnostic);
                // The lead quotes the program's own subscript.
                if subscript
                    && let Some([base, index]) = self.subscript_texts(program, &instruction)
                {
                    self.state.event.summary = narration::null_subscript(&base, &index);
                }
            },
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
            | &Op::Declare {
                ref name,
                ty,
                constant,
            } => {
                let id = self.allocate(
                    name,
                    ty,
                    self.activations[top].scope,
                    Value::Indeterminate,
                    false,
                )?;
                if constant {
                    self.make_const(id);
                }
                self.declare(range, id, ty)
            },
            | &Op::StaticLocal(index) => self.static_local(range, index + 1),
            | &Op::Push(integer, ty) => {
                let value = Operand::Value(Value::Integer(integer));
                self.push(value.clone())?;
                // A macro name stands for its replacement's spelling.
                let spelling = instruction.macro_use.map_or_else(
                    || source.get(range.start..range.end).unwrap_or_default(),
                    |used| program.macros[used].replacement.as_str(),
                );
                if spelling.starts_with('\'') {
                    narration::character(range, integer, spelling, value)
                } else {
                    narration::constant(range, integer, ty, spelling, value)
                }
            },
            | &Op::Sizeof(size, ty) => {
                let value = Value::Integer(i128::try_from(size).unwrap_or(i128::MAX));
                self.push(Operand::Value(value.clone()))?;
                self.size_of(range, source, size, ty, value)
            },
            | Op::Zero => {
                let value = Operand::Value(Value::Integer(0));
                self.push(value.clone())?;
                narration::implicit_zero(range, value)
            },
            | Op::Address(place) => {
                let id = self.named(place)?;
                let structure = match self.object_type(id) {
                    | Some(Type::Struct(structure)) => Some(structure),
                    | _ => None,
                };
                let operand = Operand::Address {
                    target: id,
                    offset: 0,
                    structure,
                };
                self.push(operand.clone())?;
                let role = self.designation(program, instruction);
                self.designate(range, id, operand, role)
            },
            | Op::Element {
                name,
                offset,
                structure,
            } => {
                let id = self.named(&Place::Local(name.clone()))?;
                let operand = Operand::Address {
                    target:    id,
                    offset:    *offset,
                    structure: *structure,
                };
                self.push(operand.clone())?;
                self.element(range, id, *offset, operand)
            },
            | Op::Decay(place) => {
                let id = self.named(place)?;
                let length = match self.object_type(id) {
                    | Some(Type::Array(_, length)) => i32::try_from(length).unwrap_or(i32::MAX),
                    | _ => 1,
                };
                let value = Value::pointer(id, 0, 0, length);
                self.push(Operand::Value(value.clone()))?;
                self.decay(range, id, value)
            },
            | Op::Load => {
                let (target, offset) = self.pop_address()?;
                let value = self.read(target, offset)?;
                self.push(Operand::Value(value.clone()))?;
                self.load(range, target, offset, value)
            },
            | &Op::LoadStruct(structure) => {
                let (target, offset) = self.pop_address()?;
                let value = self.read_structure(target, offset, structure)?;
                self.push(Operand::Value(value.clone()))?;
                self.load_structure(range, target, offset, structure, value)
            },
            | Op::Member {
                name,
                offset,
                arrow,
                structure,
                array,
            } => {
                let (target, base, whole, pointer) = if *arrow {
                    let pointer = self.pop_value()?;
                    if matches!(pointer, Value::Pointer { target: None, .. }) {
                        return Err(Diagnostic::ub(
                            self.state.source_range,
                            &format!(
                                "`->{name}` through a null pointer, which points to no structure"
                            ),
                            &clauses::INDIRECTION,
                            Undefined::NullIndirection,
                        ));
                    }
                    let (target, base, whole) = self.dereference(&pointer)?;
                    (target, base, whole, Some(pointer))
                } else {
                    let Operand::Address {
                        target,
                        offset: base,
                        structure: whole,
                    } = self.pop()?
                    else {
                        return Err(self.error("expected a structure lvalue"));
                    };
                    (target, base, whole, None)
                };
                let at = base + offset;
                let result = if let Some(length) = array {
                    let length = i32::try_from(*length).unwrap_or(i32::MAX);
                    Operand::Value(Value::pointer(target, at, at, length))
                } else {
                    Operand::Address {
                        target,
                        offset: at,
                        structure: *structure,
                    }
                };
                self.push(result.clone())?;
                self.member(range, source, target, base, whole, pointer, result)
            },
            | &Op::Promote(from) => {
                let value = self.pop_value()?;
                self.push(Operand::Value(value.clone()))?;
                self.promote(range, from, value)
            },
            | &Op::Convert {
                from,
                to,
                other,
                depth,
            } => {
                let operands = &self.activations[top].operands;
                let slot = operands
                    .len()
                    .checked_sub(depth + 1)
                    .ok_or_else(|| self.error("empty operand stack"))?;
                let Operand::Value(Value::Integer(integer)) = operands[slot] else {
                    return Err(self.error("expected an integer operand"));
                };
                let converted = to.convert(integer);
                self.activations[top].operands[slot] = Operand::Value(Value::Integer(converted));
                let whole = self.operator_text(program, instruction);
                Self::arithmetic_conversion(
                    range, &whole, from, to, other, depth, integer, converted,
                )
            },
            | &Op::Cast { from, to } => {
                let value = self.pop_value()?;
                let converted = match (to, &value) {
                    | (Type::Integer(integer), &Value::Integer(number)) =>
                        Value::Integer(integer.convert(number)),
                    | _ => value.clone(),
                };
                self.push(Operand::Value(converted.clone()))?;
                self.cast(range, source, from, to, value, converted)
            },
            | &Op::DefaultPromote(from) => {
                let value = self.pop_value()?;
                self.push(Operand::Value(value.clone()))?;
                self.default_promotion(range, from, value)
            },
            | Op::InitializeString {
                bytes,
                length,
                full_expression,
                parts,
            } => {
                let (target, offset) = self.pop_address()?;
                let mut stored = Vec::new();
                for at in 0..*length {
                    let byte = bytes.get(at).copied().unwrap_or(0);
                    let place = offset + i32::try_from(at).unwrap_or(i32::MAX);
                    stored.push(self.store(target, place, parser::char_value(byte))?);
                }
                self.initialized_string(
                    range,
                    target,
                    offset,
                    bytes,
                    Value::Array(stored),
                    *full_expression,
                    *parts,
                )
            },
            | Op::Library(library, types) =>
                self.library_call(program, instruction, *library, types)?,
            | Op::AddressOf => {
                let Operand::Address {
                    target,
                    offset,
                    structure,
                } = self.pop()?
                else {
                    return Err(self.error("expected an lvalue"));
                };
                // An object that is not an array element counts as an array
                // of one (C99 §6.5.6p7).
                let value = Value::Pointer {
                    target: Some(target),
                    offset,
                    first: offset,
                    length: 1,
                    structure,
                };
                self.push(Operand::Value(value.clone()))?;
                self.address_of(range, target, offset, structure, value)
            },
            | Op::Dereference => {
                let pointer = self.pop_value()?;
                let (target, offset, structure) = self.dereference(&pointer)?;
                self.push(Operand::Address {
                    target,
                    offset,
                    structure,
                })?;
                self.indirection(range, source, pointer, target, offset, structure)
            },
            | Op::Index => {
                let index = self.pop_value()?;
                let base = self.pop_value()?;
                if matches!(base, Value::Pointer { target: None, .. }) {
                    // C99 §6.5.2.1p2: p[i] is *(p + i), whose addition
                    // already has no meaning for a null pointer.
                    return Err(Diagnostic::ub(
                        self.state.source_range,
                        "subscripting a null pointer, which points into no array",
                        &clauses::SUBSCRIPT,
                        Undefined::NullSubscript,
                    ));
                }
                let pointer = self.binary_value("+", None, base.clone(), index.clone())?;
                let (target, offset, _) = self.dereference(&pointer)?;
                self.push(Operand::Address {
                    target,
                    offset,
                    structure: None,
                })?;
                self.subscript(range, base, index, target, offset)
            },
            | &Op::Unary(operator, ty) => {
                let value = self.pop_value()?;
                let result = match (operator, ty, &value) {
                    | ("+", _, Value::Integer(integer)) => Value::Integer(*integer),
                    // An unsigned negation is reduced modulo 2^N, so it is
                    // always defined (C99 §6.2.5p9).
                    | ("-", Some(ty), &Value::Integer(integer)) if !ty.signed() =>
                        Value::Integer(ty.convert(-integer)),
                    | ("-", ty, &Value::Integer(integer)) => {
                        let ty = ty.unwrap_or(super::integers::Integer::Int);
                        if !ty.fits(-integer) {
                            return Err(self.overflow(ty, None, "-", integer));
                        }
                        Value::Integer(-integer)
                    },
                    | ("!", _, value) => Value::Integer(i128::from(!self.truth(value)?)),
                    | _ => return Err(self.error("unsupported unary operand")),
                };
                self.push(Operand::Value(result.clone()))?;
                self.unary(range, operator, ty, value, result)
            },
            | &Op::Binary(operator, ty) => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let result = self.binary_value(operator, ty, left.clone(), right.clone())?;
                self.push(Operand::Value(result.clone()))?;
                self.binary(range, source, operator, ty, left, right, result)
            },
            | Op::ElementAddress => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let result = self.binary_value("+", None, left.clone(), right.clone())?;
                self.push(Operand::Value(result.clone()))?;
                self.binary(range, source, "&[]", None, left, right, result)
            },
            | &Op::Store {
                initializer,
                full_expression,
                from,
                structure,
            } => {
                let value = self.pop_value()?;
                let (target, offset) = self.pop_address()?;
                if !initializer {
                    self.check_writable(target, offset)?;
                }
                if let Some(structure) = structure {
                    let previous = self.read_structure(target, offset, structure)?;
                    let stored = self.store(target, offset, value)?;
                    self.copied(
                        range,
                        initializer,
                        full_expression,
                        target,
                        offset,
                        structure,
                        &previous,
                        stored,
                    )
                } else {
                    let previous = self.peek(target, offset);
                    let stored = self.store(target, offset, value.clone())?;
                    self.stored(
                        range,
                        initializer,
                        full_expression,
                        from,
                        target,
                        offset,
                        previous.as_ref(),
                        value,
                        stored,
                    )
                }
            },
            | &Op::Increment {
                delta,
                prefix,
                nested,
            } => {
                let (target, offset) = self.pop_address()?;
                self.check_writable(target, offset)?;
                let current = self.read(target, offset)?;
                // `--` subtracts 1, so an overflow reads as a subtraction.
                // An integer narrower than int is promoted first, and the
                // sum converted back when it is stored (C99 §6.5.2.4p2).
                let operator = if delta > 0 { "+" } else { "-" };
                let ty = self
                    .leaf_type(target, offset)
                    .and_then(Type::integer)
                    .map(super::integers::Integer::promoted);
                let sum = self.binary_value(operator, ty, current.clone(), Value::Integer(1))?;
                let stored = self.store(target, offset, sum.clone())?;
                if nested {
                    let value = if prefix { &stored } else { &current };
                    self.push(Operand::Value(value.clone()))?;
                }
                self.increment(
                    range, source, delta, prefix, nested, target, offset, current, &sum, stored,
                )
            },
            | &Op::Compound(operator, common) => {
                let right = self.pop_value()?;
                let (target, offset) = self.pop_address()?;
                self.check_writable(target, offset)?;
                let current = self.read(target, offset)?;
                // Both operands convert to the type the operator computes
                // in, as in `E1 = E1 op (E2)` (C99 §6.5.16.2p3).
                let shift = matches!(operator, "<<" | ">>");
                let (left, right_converted) = match (common, &current, &right) {
                    | (Some(common), &Value::Integer(left), &Value::Integer(value)) => (
                        Value::Integer(common.convert(left)),
                        Value::Integer(if shift { value } else { common.convert(value) }),
                    ),
                    | _ => (current.clone(), right.clone()),
                };
                let result =
                    self.binary_value(operator, common, left.clone(), right_converted.clone())?;
                let stored = self.store(target, offset, result.clone())?;
                self.compound(
                    range,
                    source,
                    operator,
                    common,
                    target,
                    offset,
                    [current, right, left, right_converted],
                    &result,
                    stored,
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
                // An if with an else ends its first branch with a forward
                // jump over the else branch; a loop's jump goes back.
                let code = &program.functions[self.activations[top].function].code;
                let has_else = statement == "if"
                    && target
                        .checked_sub(1)
                        .and_then(|last| code.get(last))
                        .is_some_and(|last| matches!(last.op, Op::Jump(over) if over >= target));
                self.branch(range, source, statement, value, truth, has_else)
            },
            | Op::Call(name, types) => self.call(program, instruction, name, types)?,
            | &Op::Return(from) => {
                let value = self.pop_value()?;
                self.return_from(program, range, from, value, false)?
            },
            | Op::Fallthrough =>
                if self.activations[top].function == program.entry {
                    self.return_from(program, range, Type::INT, Value::Integer(0), true)?
                } else {
                    return Err(self.error(
                        "reaching the end of a non-main function is outside the subset; write an \
                         explicit return",
                    ));
                },
        })
    }

    /// The source of the operator whose operand `instruction` evaluates:
    /// the parent of its node in the expression tree.
    fn operator_text(&self, program: &Program, instruction: &Instruction) -> String {
        let function = &program.functions[self.activations[self.activations.len() - 1].function];
        instruction
            .node
            .and_then(|node| {
                function
                    .nodes
                    .iter()
                    .find(|candidate| candidate.children.contains(&node))
            })
            .and_then(|parent| program.source.get(parent.range.start..parent.range.end))
            .map_or_else(String::new, String::from)
    }

    /// The base and index of the subscript `instruction` evaluates, as
    /// written.
    fn subscript_texts(&self, program: &Program, instruction: &Instruction) -> Option<[String; 2]> {
        let function = &program.functions[self.activations[self.activations.len() - 1].function];
        let node = function.nodes.get(instruction.node?)?;
        let text = |child: usize| {
            let range = function.nodes.get(child)?.range;
            program.source.get(range.start..range.end).map(String::from)
        };
        match *node.children.as_slice() {
            | [base, index] => Some([text(base)?, text(index)?]),
            | _ => None,
        }
    }

    /// What the operator around the name `instruction` evaluates will do
    /// with its object: the next operation reads it, or its parent node in
    /// the expression tree stores into it, initializes it, takes its address
    /// or updates it.
    fn designation(&self, program: &Program, instruction: &Instruction) -> Designation {
        let activation = &self.activations[self.activations.len() - 1];
        let function = &program.functions[activation.function];
        // The program counter is already past `instruction`.
        let reads = function.code.get(activation.pc).is_some_and(|next| {
            matches!(next.op, Op::Load | Op::LoadStruct(_)) && next.node == instruction.node
        });
        let parent = instruction.node.and_then(|node| {
            function
                .nodes
                .iter()
                .position(|candidate| candidate.children.contains(&node))
        });
        let initializes = parent.is_some_and(|parent| {
            function.code.iter().any(|operation| {
                operation.node == Some(parent)
                    && matches!(
                        operation.op,
                        Op::Store {
                            initializer: true,
                            ..
                        } | Op::InitializeString { .. }
                    )
            })
        });
        match parent.map(|parent| function.nodes[parent].label.as_str()) {
            | _ if reads => Designation::Read,
            | _ if initializes => Designation::Initialized,
            | Some("&") => Designation::Address,
            | Some("++" | "--") => Designation::Updated,
            | Some("+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=") => Designation::Compound,
            | Some(label) if label.starts_with('.') => Designation::Member,
            | _ => Designation::Assigned,
        }
    }

    fn call(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        name: &str,
        types: &[Type],
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
        for _ in types {
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
            types,
            parameters,
        ))
    }

    fn return_from(
        &mut self,
        program: &Program,
        range: super::SourceRange,
        from: Type,
        value: Value,
        fallthrough: bool,
    ) -> Result<Event, Diagnostic> {
        if !matches!(value, Value::Integer(_) | Value::Struct { .. }) {
            return Err(self.error("function returned a value of an unsupported type"));
        }
        let top = self.activations.len() - 1;
        let id = self.activations[top].id;
        let defined = &program.functions[self.activations[top].function];
        let function = defined.name.clone();
        // The value converts to the return type as if by assignment (C99
        // §6.8.6.4p3).
        let returns = defined.returns;
        let original = value.clone();
        let value = match (returns, value) {
            | (Type::Integer(integer), Value::Integer(number)) =>
                Value::Integer(integer.convert(number)),
            | (_, value) => value,
        };
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
        Ok(self.returned(
            range,
            &function,
            id,
            (from, returns, original),
            value,
            ended,
            fallthrough,
            call,
        ))
    }

    /// Start tracking the full expression `instruction` belongs to in the
    /// innermost activation; statement-level operations clear it.
    fn track(&mut self, program: &Program, instruction: &Instruction) {
        let top = self.activations.len() - 1;
        let activation = &mut self.activations[top];
        let Some(node) = instruction.node else {
            if matches!(
                instruction.op,
                Op::Enter | Op::Leave | Op::Declare { .. } | Op::Jump(_)
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
        // Directives run in translation, and static initialization comes
        // before program startup: no call has begun until main's first
        // operation (C99 §5.1.1.2p1, §5.1.2p1).
        if !self.started && (!program.statics.is_empty() || !program.directives.is_empty()) {
            self.state.evaluation = None;
            if !self.state.terminal() {
                self.state.source_range = self.before_main(program);
            }
            return;
        }
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

    /// The next operation before `main`: a directive, a static object's
    /// initialization, or `main`'s first operation.
    fn before_main(&self, program: &Program) -> super::SourceRange {
        program
            .directives
            .get(self.preprocessed)
            .map(|directive| directive.range)
            .or_else(|| {
                program
                    .statics
                    .get(self.initialized)
                    .map(|object| object.range)
            })
            .unwrap_or(program.functions[program.entry].code[0].range)
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
        let first = program
            .directives
            .first()
            .map(|directive| directive.range)
            .or_else(|| program.statics.first().map(|object| object.range))
            .unwrap_or(program.functions[program.entry].code[0].range);
        let mut machine = Self {
            state: State {
                step:            0,
                status:          "ready",
                source_range:    first,
                event:           narration::start(first, program),
                evaluation:      None,
                frames:          Vec::new(),
                objects:         Vec::new(),
                pending_effects: Vec::new(),
                diagnostics:     Vec::new(),
                return_value:    None,
                output:          Vec::new(),
                written:         0,
            },
            activations,
            next_frame: 2,
            cells: 0,
            metadata: Vec::new(),
            structures: Arc::clone(&program.structures),
            preprocessed: 0,
            initialized: 0,
            started: false,
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
            | Operand::Address { target, offset, .. } => Ok((target, offset)),
            | Operand::Value(_) => Err(self.error("expected a modifiable scalar lvalue")),
        }
    }

    pub fn pop(&mut self) -> Result<Operand, Diagnostic> {
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
            bytes += object.name.capacity()
                + object.r#type.capacity()
                + object.function.as_ref().map_or(0, String::capacity)
                + value_bytes(&object.value);
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
            + event.note.capacity()
            + event.clauses.capacity() * size_of::<super::Clause>()
            + event.operands.capacity() * size_of::<Operand>()
            + event.objects.capacity() * size_of::<usize>()
            + self.state.pending_effects.capacity() * size_of::<String>()
            + self.state.output.capacity();
        for effect in &self.state.pending_effects {
            bytes += effect.capacity();
        }
        bytes + 1024 // Diagnostic storage and allocator bookkeeping reserve.
    }
}

/// The heap bytes an aggregate value holds beyond its own size.
fn value_bytes(value: &Value) -> usize {
    match value {
        | Value::Array(elements) => elements.capacity() * size_of::<Value>(),
        | Value::Struct { members, .. } =>
            members.capacity() * size_of::<Value>() + members.iter().map(value_bytes).sum::<usize>(),
        | _ => 0,
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
    pub scope:    usize,
    pub ty:       Type,
    /// Whether the object is defined `const` (C99 §6.7.3p5).
    pub constant: bool,
}
