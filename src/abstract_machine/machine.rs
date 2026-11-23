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
    floating::{
        Floating,
        Real,
    },
    model::{
        Alias,
        Arithmetic,
        Diagnostic,
        Enumeration,
        Evaluation,
        Event,
        Frame,
        Instruction,
        MAX_FRAMES,
        MAX_OPERANDS,
        Named,
        Op,
        Operand,
        Place,
        PointerSourceKind,
        Program,
        Signature,
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
    sequencing::{
        Access,
        Touch,
    },
};

#[derive(Debug, Clone)]
pub(super) struct Machine {
    pub state:        State,
    pub activations:  Vec<Activation>,
    pub next_frame:   usize,
    pub cells:        usize,
    pub metadata:     Vec<ObjectMetadata>,
    /// The objects whose declarations, reached again without an
    /// initializer, made their values indeterminate once more (C99
    /// §6.8p3), each with what it held before: kept apart from the
    /// metadata, which every retained step copies for every object.
    pub forgotten:    Vec<(usize, Value)>,
    /// The program's structure types, for layouts and member names.
    pub structures:   Arc<[Structure]>,
    /// The program's function types, for spelling pointers to functions.
    pub signatures:   Arc<[Signature]>,
    /// The program's functions' names, by index, for the pointers to them.
    pub functions:    Arc<[String]>,
    /// The program's enumerations, for their types' names and constants.
    pub enumerations: Arc<[Enumeration]>,
    /// The program's typedef names, for the explanations.
    pub aliases:      Arc<[Alias]>,
    /// How many of the program's file-scope enumerations have been
    /// declared: each is a step of its own, after the directives and
    /// before the static objects exist.
    pub enumerated:   usize,
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
        if self.enumerated < program.file_enumerations.len() {
            let index = program.file_enumerations[self.enumerated];
            self.enumerated += 1;
            self.state.status = "running";
            self.state.source_range = program.enumerations[index].range;
            self.state.event = self.enumeration(program, index);
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
                        Op::Call(..)
                            | Op::Return(_)
                            | Op::Fallthrough
                            | Op::StaticLocal(_)
                            | Op::Switch { .. }
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
            // Entry into a block creates every automatic object it declares,
            // indeterminate; their names come into scope only at their
            // declarations (C99 §6.2.4p5, §6.2.1p7).
            | Op::Enter { objects, iteration } => {
                let block = self.activations[top].pc - 1;
                let begun = self.begin_block(objects, block)?;
                let written = source.get(range.start..range.end).unwrap_or("");
                self.enter_block(range, written, begun, *iteration)
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
                let event = self.leave_block(range, ended);
                if source.get(range.start..range.end) == Some("continue") {
                    narration::left_by_continue(event)
                } else {
                    event
                }
            },
            // The object exists already; from here its name designates it.
            // Without an initializer, reaching the declaration stores an
            // indeterminate value (C99 §6.8p3): this subset reaches each
            // declaration once per entry into its block, when nothing has
            // been stored yet, so the value is unchanged.
            | Op::Declare { .. } => self.declare_operation(instruction)?,
            | &Op::StaticLocal(index) => self.static_local(range, index + 1),
            | &Op::Enumeration(index) => self.enumeration(program, index),
            | Op::Enumerator { .. } => self.enumerator_operation(instruction)?,
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
            | &Op::PushReal { value, exact } => {
                let operand = Operand::Value(Value::Floating(value));
                self.push(operand.clone())?;
                let spelling = instruction.macro_use.map_or_else(
                    || source.get(range.start..range.end).unwrap_or_default(),
                    |used| program.macros[used].replacement.as_str(),
                );
                narration::floating_constant(range, value, exact, spelling, operand)
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
                let event = self.designate(range, id, operand, role);
                self.hiding(event, id)
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
                let length = self
                    .object_type(id)
                    .and_then(Type::length)
                    .map_or(1, |length| i32::try_from(length).unwrap_or(i32::MAX));
                let value = Value::pointer(id, 0, 0, length);
                self.push(Operand::Value(value.clone()))?;
                let event = self.decay(range, id, value);
                self.hiding(event, id)
            },
            | Op::Load => {
                let (target, offset) = self.pop_address()?;
                self.access(program, instruction.node, (target, offset, 1), false, false)?;
                let value = self.read(target, offset)?;
                self.push(Operand::Value(value.clone()))?;
                self.load(range, target, offset, value)
            },
            | &Op::LoadStruct(structure) => {
                let (target, offset) = self.pop_address()?;
                let leaves = self.leaf_count(Type::Struct(structure));
                self.access(
                    program,
                    instruction.node,
                    (target, offset, leaves),
                    false,
                    false,
                )?;
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
                let Operand::Value(value) = operands[slot].clone() else {
                    return Err(self.error("expected an arithmetic operand"));
                };
                let converted = self.converted(&value, to)?;
                self.activations[top].operands[slot] = Operand::Value(converted.clone());
                let whole = self.operator_text(program, instruction);
                match (from, to, other, &value, &converted) {
                    | (
                        Arithmetic::Integer(from),
                        Arithmetic::Integer(to),
                        Arithmetic::Integer(other),
                        &Value::Integer(integer),
                        &Value::Integer(result),
                    ) => Self::arithmetic_conversion(
                        range, &whole, from, to, other, depth, integer, result,
                    ),
                    | _ => Self::floating_conversion(
                        range, &whole, from, to, other, depth, value, converted,
                    ),
                }
            },
            | &Op::Cast { from, to } => {
                let value = self.pop_value()?;
                let converted = match to.arithmetic() {
                    | Some(arithmetic) => self.converted(&value, arithmetic)?,
                    // `(void *)0` converts a null pointer constant to a
                    // null pointer (C99 §6.3.2.3p3).
                    | None if to == Type::VoidPointer => Value::null(),
                    | None => value.clone(),
                };
                self.push(Operand::Value(converted.clone()))?;
                self.cast(range, source, from, to, value, converted)
            },
            | &Op::DefaultPromote(from) => {
                let value = self.pop_value()?;
                let promoted = match from {
                    | Arithmetic::Floating(_) =>
                        self.converted(&value, Arithmetic::Floating(Floating::Double))?,
                    | Arithmetic::Integer(_) => value.clone(),
                };
                self.push(Operand::Value(promoted.clone()))?;
                self.default_promotion(range, from, value, promoted)
            },
            | Op::InitializeString {
                bytes,
                length,
                full_expression,
                parts,
            } => {
                let (target, offset) = self.pop_address()?;
                self.activations[top].listing = !*full_expression;
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
            | Op::Function { .. } | Op::FunctionIndirection { .. } | Op::CallPointer { .. } =>
                self.function_operation(instruction, program)?,
            | &Op::Dereference { object, operand } =>
                self.dereference_operation(range, source, object, operand)?,
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
                let written = self.subscript_texts(program, instruction);
                self.subscript(range, (base, index), written.as_ref(), target, offset)
            },
            | &Op::Unary(operator, ty) => {
                let value = self.pop_value()?;
                let integer_type = ty.and_then(Arithmetic::integer);
                let result = match (operator, integer_type, &value) {
                    | ("+", _, Value::Integer(integer)) => Value::Integer(*integer),
                    | ("+", _, &Value::Floating(real)) => Value::Floating(real),
                    // Unary minus flips a floating value's sign, a zero's and
                    // a NaN's too (C99 §F.3).
                    | ("-", _, &Value::Floating(real)) => Value::Floating(real.negate()),
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
                    // Every bit flipped: -x - 1 in two's complement, and
                    // for an unsigned type its largest value minus x (C99
                    // §6.5.3.3p4).
                    | ("~", Some(ty), &Value::Integer(integer)) =>
                        Value::Integer(ty.convert(!integer)),
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
                let constant = self.null_constant_text(program, instruction, (&left, &right));
                self.binary(
                    range,
                    (source, constant.as_deref()),
                    operator,
                    ty,
                    left,
                    right,
                    result,
                )
            },
            | Op::ElementAddress => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let result = self.binary_value("+", None, left.clone(), right.clone())?;
                self.push(Operand::Value(result.clone()))?;
                self.binary(range, (source, None), "&[]", None, left, right, result)
            },
            | &Op::Store {
                initializer,
                full_expression,
                from,
                structure,
                nested,
            } => self.store_operation(
                program,
                instruction,
                (initializer, full_expression),
                from,
                structure,
                nested,
            )?,
            | &Op::Increment {
                delta,
                prefix,
                nested,
                full_expression,
            } => {
                let (target, offset) = self.pop_address()?;
                self.check_writable(target, offset)?;
                self.access(program, instruction.node, (target, offset, 1), true, false)?;
                let current = self.read(target, offset)?;
                // `--` subtracts 1, so an overflow reads as a subtraction.
                // An integer narrower than int is promoted first, and the
                // sum converted back when it is stored; a floating object
                // adds the 1 of its own type (C99 §6.5.2.4p2).
                let operator = if delta > 0 { "+" } else { "-" };
                let ty = self
                    .leaf_type(target, offset)
                    .and_then(Type::arithmetic)
                    .map(Arithmetic::promoted);
                let one = match ty {
                    | Some(Arithmetic::Floating(floating)) =>
                        Value::Floating(Real::from_integer(1, floating).value),
                    | _ => Value::Integer(1),
                };
                let sum = self.binary_value(operator, ty, current.clone(), one)?;
                let stored = self.store(target, offset, sum.clone())?;
                if nested.is_some() {
                    let value = if prefix { &stored } else { &current };
                    self.push(Operand::Value(value.clone()))?;
                }
                self.increment(
                    range,
                    source,
                    delta,
                    (prefix, nested, full_expression),
                    target,
                    offset,
                    current,
                    &sum,
                    stored,
                )
            },
            | &Op::Compound(operator, common, right_type, full_expression, nested) => self
                .compound_assign(
                    program,
                    instruction,
                    operator,
                    (common, right_type),
                    (full_expression, nested),
                )?,
            | Op::Test { .. } | Op::Yield { .. } | Op::Comma { .. } =>
                self.sequenced(instruction, program)?,
            // Discarding a value does not use it, even a pointer whose
            // object's lifetime has ended.
            | Op::Pop => {
                let value = match self.pop()? {
                    | Operand::Value(value) => value,
                    | Operand::Address { .. } =>
                        return Err(self.error("expected an evaluated value")),
                };
                self.discard(range, source, value)
            },
            | &Op::Jump(target) => {
                let backward = target < self.activations[top].pc;
                self.activations[top].pc = target;
                narration::jump(range, backward)
            },
            | &Op::Break(target, statement, skipped) => {
                self.activations[top].pc = target;
                narration::break_statement(range, statement, skipped)
            },
            | Op::PastLabels(_) | Op::Do | Op::Continue { .. } | Op::Goto(_) =>
                self.jump_operation(instruction, program)?,
            | Op::Switch { .. } => self.switch_operation(program, instruction)?,
            | &Op::Branch(target, statement) => {
                let value = self.pop_value()?;
                let truth = self.truth(&value)?;
                // A do statement's test jumps back to its body while the
                // value is nonzero; every other condition jumps past the
                // code it controls when it is zero (C99 §6.8.5.2p1).
                if truth == (statement == "do") {
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
            | Op::Call(name, types) => {
                let Some(function) = program
                    .functions
                    .iter()
                    .position(|function| function.name == *name)
                else {
                    return Err(self.error("unknown function"));
                };
                let arguments = self.arguments(types)?;
                self.call(program, instruction, function, types, &arguments, false)?
            },
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

    /// Reaching the declaration of an automatic object its block created:
    /// its name is in scope from here (C99 §6.2.1p7), and without an
    /// initializer it holds an indeterminate value (C99 §6.8p3).
    fn declare_operation(&mut self, instruction: &Instruction) -> Result<Event, Diagnostic> {
        let &Op::Declare {
            ref name,
            ty,
            initialized,
            named,
        } = &instruction.op
        else {
            return Err(self.error("not a declaration"));
        };
        let top = self.activations.len() - 1;
        let range = instruction.range;
        let frame = &self.activations[top];
        let id = self
            .state
            .objects
            .iter()
            .rev()
            .find(|object| {
                object.name == *name
                    && object.frame_id == frame.id
                    && object.lifetime == "alive"
                    && !object.declared
                    && self.metadata[object.id - 1].scope == frame.scope
            })
            .map(|object| object.id)
            .ok_or_else(|| self.error("declaration of an object its block did not create"))?;
        self.state.objects[id - 1].declared = true;
        // Reaching the declaration again, after a jump back over
        // it, stores an indeterminate value anew when it has no
        // initializer (C99 §6.2.4p5, §6.8p3).
        let again = std::mem::replace(&mut self.metadata[id - 1].reached, true);
        let before = self.state.objects[id - 1].value.clone();
        if !initialized {
            self.state.objects[id - 1].value = ty.indeterminate(&self.structures);
        }
        self.forgotten.retain(|&(object, _)| object != id);
        if again && !initialized {
            self.forgotten.push((id, before.clone()));
        }
        let event = self.declare(range, id, ty, (initialized, named));
        Ok(if again {
            let name = self.state.objects[id - 1].name.clone();
            self.declared_again(event, &name, &before, initialized)
        } else {
            event
        })
    }

    /// A step that moves control without testing anything: running on
    /// past case labels, a `do` entering its body, a `continue`, or a
    /// `goto`'s steps.
    fn jump_operation(
        &mut self,
        instruction: &Instruction,
        program: &Program,
    ) -> Result<Event, Diagnostic> {
        let top = self.activations.len() - 1;
        let range = instruction.range;
        let source = &program.source;
        Ok(match &instruction.op {
            | &Op::PastLabels(count) => {
                // Labels first in a block follow its entry, not a statement.
                let code = &program.functions[self.activations[top].function].code;
                let first = self.activations[top]
                    .pc
                    .checked_sub(2)
                    .and_then(|before| code.get(before))
                    .is_some_and(|before| matches!(before.op, Op::Enter { .. }));
                narration::past_labels(range, source, count, first)
            },
            | Op::Do => self.do_statement(program, instruction),
            | &Op::Continue {
                target,
                statement,
                next,
            } => {
                // The blocks the continue left, in the steps before it.
                let code = &program.functions[self.activations[top].function].code;
                let at = self.activations[top].pc - 1;
                let left = code[..at]
                    .iter()
                    .rev()
                    .take_while(|before| {
                        matches!(before.op, Op::Leave) && before.range == instruction.range
                    })
                    .count();
                self.activations[top].pc = target;
                self.continued(program, range, statement, next, left)
            },
            | Op::Goto(goto) => self.goto_operation(program, instruction, goto)?,
            | _ => return Err(self.error("not a jump")),
        })
    }

    /// `*p` of a pointer to an object: the lvalue it points to. `object`
    /// says whether its operand, written at `operand`, holds the pointer.
    fn dereference_operation(
        &mut self,
        range: super::SourceRange,
        source: &str,
        object: bool,
        operand: super::SourceRange,
    ) -> Result<Event, Diagnostic> {
        let pointer = self.pop_value()?;
        let (target, offset, structure) = self.dereference(&pointer)?;
        self.push(Operand::Address {
            target,
            offset,
            structure,
        })?;
        Ok(self.indirection(
            range,
            &narration::snippet(source, operand),
            (pointer, object),
            target,
            offset,
            structure,
        ))
    }

    /// An enumeration constant: push its value, an int.
    fn enumerator_operation(&mut self, instruction: &Instruction) -> Result<Event, Diagnostic> {
        let Op::Enumerator {
            value,
            enumeration,
            constant,
        } = instruction.op
        else {
            return Err(self.error("not an enumeration constant"));
        };
        self.push(Operand::Value(Value::Integer(value)))?;
        Ok(self.enumerator(instruction.range, (enumeration, constant), value))
    }

    /// `switch`: jump to the case label whose constant equals the
    /// controlling value, to `default` when none does, or past the body.
    fn switch_operation(
        &mut self,
        program: &Program,
        instruction: &Instruction,
    ) -> Result<Event, Diagnostic> {
        let Op::Switch {
            cases,
            default,
            end,
            promoted,
        } = &instruction.op
        else {
            return Err(self.error("not a switch"));
        };
        let value = self.pop_value()?;
        let Value::Integer(number) = value else {
            return Err(self.error("a switch's controlling value is not an integer"));
        };
        // C99 §6.8.4.2p5: the integer promotions apply to the controlling
        // expression, and each case constant was converted to the promoted
        // type when it was compiled.
        let promoted_value = promoted.convert(number);
        let matched = cases.iter().find(|case| case.value == promoted_value);
        let top = self.activations.len() - 1;
        let landing = matched.map(|case| &case.landing).or(default.as_ref());
        let entered = match landing {
            | Some(landing) => {
                self.activations[top].pc = landing.target;
                self.land(program, landing)?
            },
            | None => {
                self.activations[top].pc = *end;
                Vec::new()
            },
        };
        let mut event = Self::switched(
            program,
            instruction.range,
            promoted_value,
            matched,
            default.is_some(),
        );
        if !entered.is_empty() {
            event = self.entered_by_jump(
                event,
                program,
                landing.map_or(&[], |landing| &landing.enter),
                &entered,
            );
        }
        Ok(event)
    }

    /// Begin the lifetimes of the automatic objects `objects` of the block
    /// whose `Enter` instruction is `block`, one scope deeper: each is
    /// indeterminate, and its name is not in scope until its declaration
    /// (C99 §6.2.4p5, §6.2.1p7).
    fn begin_block(
        &mut self,
        objects: &[super::model::BlockObject],
        block: usize,
    ) -> Result<Vec<usize>, Diagnostic> {
        let top = self.activations.len() - 1;
        self.activations[top].scope += 1;
        let scope = self.activations[top].scope;
        let mut begun = Vec::with_capacity(objects.len());
        for object in objects {
            let id = self.allocate(&object.name, object.ty, scope, Value::Indeterminate, false)?;
            if let Some(typedef) = object.typedef {
                self.name_type(id, typedef);
            }
            self.name(id, object.named);
            if object.constant {
                self.make_const(id);
            }
            self.state.objects[id - 1].declared = false;
            self.metadata[id - 1].block = u32::try_from(block).ok();
            self.metadata[id - 1].line = u32::try_from(object.range.line).unwrap_or(0);
            begun.push(id);
        }
        Ok(begun)
    }

    /// A jump has moved the program counter to `landing`'s target: enter
    /// the blocks between, beginning their objects' lifetimes. The objects
    /// whose declarations come before the target are in scope there, and
    /// indeterminate, since the jump passed their initializers (C99
    /// §6.2.4p5, §6.8p3). Returns the objects begun.
    pub(super) fn land(
        &mut self,
        program: &Program,
        landing: &super::model::Landing,
    ) -> Result<Vec<usize>, Diagnostic> {
        let top = self.activations.len() - 1;
        let function = self.activations[top].function;
        let mut begun = Vec::new();
        for entry in &landing.enter {
            let Some(Op::Enter { objects, .. }) = program.functions[function]
                .code
                .get(entry.block)
                .map(|instruction| &instruction.op)
            else {
                return Err(self.error("a jump enters a block that is not one"));
            };
            let ids = self.begin_block(objects, entry.block)?;
            for &id in ids.iter().take(entry.declared) {
                self.state.objects[id - 1].declared = true;
                self.metadata[id - 1].reached = true;
            }
            begun.extend(ids);
        }
        Ok(begun)
    }

    /// `goto`: while the innermost block open here is one the label is not
    /// in, leave it, in a step of its own, and run the goto again; then
    /// jump, entering the blocks the label is in, and bring into scope
    /// exactly the names declared before the label in the innermost block
    /// that stays open (C99 §6.8.6.1p2, §6.2.1p7).
    fn goto_operation(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        goto: &super::model::Goto,
    ) -> Result<Event, Diagnostic> {
        let top = self.activations.len() - 1;
        let frame = &self.activations[top];
        if frame.scope > goto.common {
            let left = goto.from.get(frame.scope - 1).copied();
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
            self.activations[top].pc -= 1;
            let event = self.leave_block(instruction.range, ended);
            return Ok(self.left_by_goto(event, program, goto, left));
        }
        // The innermost block that stays open: the names declared in it
        // before the label are in scope there, the others not.
        let stays = goto
            .from
            .get(goto.common.wrapping_sub(1))
            .and_then(|&block| u32::try_from(block).ok());
        let mut passed = Vec::new();
        let mut unscoped = Vec::new();
        let frame_id = self.activations[top].id;
        let mut index = 0;
        for object in &mut self.state.objects {
            if object.frame_id == frame_id
                && object.lifetime == "alive"
                && stays.is_some()
                && self.metadata[object.id - 1].block == stays
            {
                let declared = index < goto.declared;
                index += 1;
                if declared && !object.declared {
                    passed.push(object.id);
                } else if !declared && object.declared {
                    unscoped.push(object.id);
                }
                object.declared = declared;
                if declared {
                    self.metadata[object.id - 1].reached = true;
                }
            }
        }
        self.activations[top].pc = goto.landing.target;
        let entered = self.land(program, &goto.landing)?;
        Ok(self.goto_jump(program, instruction, goto, &entered, &passed, &unscoped))
    }

    /// A store into the lvalue below the value on the stack: an
    /// assignment's or an initializer's, of one scalar or of a whole
    /// structure. A nested assignment leaves the value stored on the
    /// stack for the expression around it (C99 §6.5.16p3).
    fn store_operation(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        (initializer, full_expression): (bool, bool),
        from: Type,
        structure: Option<usize>,
        nested: Option<super::model::Nested>,
    ) -> Result<Event, Diagnostic> {
        let (range, source) = (instruction.range, program.source.as_str());
        let top = self.activations.len() - 1;
        let value = self.pop_value()?;
        let (target, offset) = self.pop_address()?;
        if !initializer {
            self.check_writable(target, offset)?;
        }
        // An initializer list's store that does not end it.
        self.activations[top].listing = initializer && !full_expression;
        // An initializer's stores happen in list order, each one
        // replacing an earlier one for the same subobject (C99
        // §6.7.8p19); only an assignment's store is a side effect.
        if !initializer {
            let leaves = structure.map_or(1, |structure| self.leaf_count(Type::Struct(structure)));
            self.access(
                program,
                instruction.node,
                (target, offset, leaves),
                true,
                false,
            )?;
        }
        if let Some(structure) = structure {
            let previous = self.read_structure(target, offset, structure)?;
            let stored = self.store(target, offset, value)?;
            if nested.is_some() {
                self.push(Operand::Value(stored.clone()))?;
            }
            let event = self.copied(
                range,
                initializer,
                full_expression,
                target,
                offset,
                structure,
                &previous,
                stored.clone(),
            );
            Ok(self.nested_store(event, source, nested, target, offset, &stored))
        } else {
            let previous = self.peek(target, offset);
            let stored = self.store(target, offset, value.clone())?;
            if nested.is_some() {
                self.push(Operand::Value(stored.clone()))?;
            }
            let event = self.stored(
                range,
                initializer,
                full_expression,
                from,
                target,
                offset,
                previous.as_ref(),
                value,
                stored.clone(),
            );
            Ok(self.nested_store(event, source, nested, target, offset, &stored))
        }
    }

    /// `E1 op= E2`: read the object, apply `op` in `common`, store the
    /// result (C99 §6.5.16.2p3).
    fn compound_assign(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        operator: &'static str,
        (common, right_type): (Option<Arithmetic>, Option<Arithmetic>),
        (full_expression, nested): (bool, Option<super::model::Nested>),
    ) -> Result<Event, Diagnostic> {
        let (range, source) = (instruction.range, program.source.as_str());
        let right = self.pop_value()?;
        let (target, offset) = self.pop_address()?;
        self.check_writable(target, offset)?;
        self.access(program, instruction.node, (target, offset, 1), true, false)?;
        let current = self.read(target, offset)?;
        // Both operands convert to the type the operator computes
        // in, as in `E1 = E1 op (E2)` (C99 §6.5.16.2p3).
        let shift = matches!(operator, "<<" | ">>");
        let (left, right_converted) = match common {
            | Some(common) if !shift => (
                self.converted(&current, common)?,
                self.converted(&right, common)?,
            ),
            | Some(common) => (self.converted(&current, common)?, right.clone()),
            | None => (current.clone(), right.clone()),
        };
        let result = self.binary_value(operator, common, left.clone(), right_converted.clone())?;
        let stored = self.store(target, offset, result.clone())?;
        if nested.is_some() {
            self.push(Operand::Value(stored.clone()))?;
        }
        let event = self.compound(
            range,
            source,
            operator,
            (common, right_type),
            target,
            offset,
            [current, right, left, right_converted],
            &result,
            stored.clone(),
            full_expression,
        );
        Ok(self.nested_store(event, source, nested, target, offset, &stored))
    }

    /// The operations around a sequence point inside a full expression:
    /// the test of the first operand of `&&`, `||` or `?:`, their value,
    /// and the comma operator's (C99 Annex C).
    fn sequenced(
        &mut self,
        instruction: &Instruction,
        program: &Program,
    ) -> Result<Event, Diagnostic> {
        let top = self.activations.len() - 1;
        let range = instruction.range;
        Ok(match &instruction.op {
            | &Op::Test { operator, target } => {
                let value = self.pop_value()?;
                let truth = self.truth(&value)?;
                let texts = self.operand_texts(program, instruction);
                // `&&` of 0 is 0 and `||` of a nonzero operand is 1: the
                // second operand cannot change it, so it is never
                // evaluated (C99 §6.5.13p4, §6.5.14p4). `?:` evaluates one
                // of its others (§6.5.15p4).
                let (settled, skipped) = match operator {
                    | "&&" if !truth => (Some(0), Some(1)),
                    | "||" if truth => (Some(1), Some(1)),
                    | "?:" if truth => (None, Some(2)),
                    | "?:" => (None, Some(1)),
                    | _ => (None, None),
                };
                if let Some(child) = skipped {
                    self.skip(program, instruction, child);
                }
                if settled.is_some() || (operator == "?:" && !truth) {
                    self.activations[top].pc = target;
                }
                if let Some(settled) = settled {
                    self.push(Operand::Value(Value::Integer(settled)))?;
                }
                if operator == "?:" {
                    self.conditional_test(range, &texts, value, truth)
                } else {
                    self.logical_test(range, operator, &texts, value, truth, settled)
                }
            },
            | &Op::Yield {
                operator,
                to,
                target,
            } => {
                let value = self.pop_value()?;
                let result = match (operator, to) {
                    | ("&&" | "||", _) => Value::Integer(i128::from(self.truth(&value)?)),
                    | (_, Some(ty)) if ty.is_pointer() && value == Value::Integer(0) =>
                        Value::null(),
                    | (_, Some(ty)) => match ty.arithmetic() {
                        | Some(arithmetic) => self.converted(&value, arithmetic)?,
                        | None => value.clone(),
                    },
                    | (_, None) => value.clone(),
                };
                self.push(Operand::Value(result.clone()))?;
                if let Some(target) = target {
                    self.activations[top].pc = target;
                }
                let texts = self.operand_texts(program, instruction);
                // After `?:`'s second operand the yield skips the third.
                let chosen = if target.is_some() { 1 } else { texts.len() - 1 };
                self.yielded(range, operator, &texts, chosen, to, value, result)
            },
            | &Op::Comma { discard } => {
                let value = if discard {
                    Some(self.pop_value()?)
                } else {
                    None
                };
                let texts = self.operand_texts(program, instruction);
                self.comma(range, &texts, value)
            },
            | _ => return Err(self.error("expected a sequenced operation")),
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
            | Some("+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=" | "&=" | "^=" | "|=") =>
                Designation::Compound,
            | Some(label) if label.starts_with('.') => Designation::Member,
            | _ => Designation::Assigned,
        }
    }

    /// An operation on a pointer to a function: a function's name becoming
    /// one, `*fp`, or a call through one.
    fn function_operation(
        &mut self,
        instruction: &Instruction,
        program: &Program,
    ) -> Result<Event, Diagnostic> {
        let range = instruction.range;
        let source = &program.source;
        Ok(match &instruction.op {
            | &Op::Function { ref name, address } => {
                let Some(function) = self.functions.iter().position(|known| known == name) else {
                    return Err(self.error("unknown function"));
                };
                let value = Value::Function(function);
                self.push(Operand::Value(value.clone()))?;
                self.function_designator(range, program, function, address, value)
            },
            | &Op::FunctionIndirection { operand, form } => {
                let pointer = self.pop_value()?;
                let Value::Function(function) = pointer else {
                    // A null pointer points to no function, and `*` of it
                    // is undefined (C99 §6.5.3.2p4, footnote 87).
                    return Err(Diagnostic::ub(
                        self.state.source_range,
                        &format!(
                            "`{}` applies * to a null pointer, which points to no function: there \
                             is no function for it to designate",
                            source.get(range.start..range.end).unwrap_or("*")
                        ),
                        &clauses::INDIRECTION,
                        Undefined::NullFunctionIndirection,
                    ));
                };
                self.push(Operand::Value(pointer.clone()))?;
                let operand = source
                    .get(operand.start..operand.end)
                    .unwrap_or("the pointer");
                self.function_indirection(range, (operand, form), program, function, pointer)
            },
            | Op::CallPointer {
                signature, types, ..
            } => {
                let arguments = self.arguments(types)?;
                let pointer = self.pop_value()?;
                let function = self.called_function(program, instruction, &pointer, *signature)?;
                self.call(program, instruction, function, types, &arguments, true)?
            },
            | _ => return Err(self.error("expected an operation on a pointer to a function")),
        })
    }

    /// The arguments of a call, popped in order: one per type.
    fn arguments(&mut self, types: &[Type]) -> Result<Vec<Value>, Diagnostic> {
        let mut arguments = Vec::new();
        for _ in types {
            arguments.push(self.pop_value()?);
        }
        arguments.reverse();
        Ok(arguments)
    }

    /// The function a call through `pointer`, of a type that points to
    /// functions of type `signature`, runs. A null pointer points to no
    /// function: C defines a call only of a function (C99 §6.5.2.2p3,
    /// §6.3.2.3p3), and leaves this one undefined by giving it no meaning
    /// (§4p2). A function of another type than the one the pointer's type
    /// points to, which only a cast lets the pointer reach, is undefined
    /// too (§6.5.2.2p9, §6.3.2.3p8).
    fn called_function(
        &self,
        program: &Program,
        instruction: &Instruction,
        pointer: &Value,
        signature: usize,
    ) -> Result<usize, Diagnostic> {
        let callee = self.callee_text(program, instruction);
        let Value::Function(function) = *pointer else {
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "the call `{callee}(…)` goes through a null pointer, which points to no \
                     function (C99 §6.3.2.3p3). A call's first expression denotes the function it \
                     calls (§6.5.2.2p3), and C defines no call of anything else, so this one is \
                     undefined by the omission of any definition (§4p2)"
                ),
                &clauses::UNDEFINED_BY_OMISSION,
                Undefined::NullCall,
            ));
        };
        let defined = program.functions[function].signature;
        if defined != signature {
            let spell = |index: usize| {
                program.signatures[index].spelling(&self.structures, &self.signatures)
            };
            return Err(Diagnostic::ub(
                self.state.source_range,
                &format!(
                    "`{callee}` points to {}, a function of type {}, but its own type says it \
                     points to a function of type {}: a cast converted the pointer, and a call \
                     through the wrong type is undefined",
                    program.functions[function].name,
                    spell(defined),
                    spell(signature)
                ),
                &clauses::CALL_TYPE,
                Undefined::CallType,
            ));
        }
        Ok(function)
    }

    /// The source of the expression that denotes the function a call
    /// through a pointer calls: `fp`, `(*fp)` or `ops[i]`.
    fn callee_text<'p>(&self, program: &'p Program, instruction: &Instruction) -> &'p str {
        let top = self.activations.len() - 1;
        let nodes = &program.functions[self.activations[top].function].nodes;
        instruction
            .node
            .and_then(|node| nodes.get(node))
            .and_then(|node| node.children.first())
            .and_then(|&child| nodes.get(child))
            .and_then(|child| program.source.get(child.range.start..child.range.end))
            .unwrap_or("the pointer")
    }

    fn call(
        &mut self,
        program: &Program,
        instruction: &Instruction,
        function: usize,
        types: &[Type],
        arguments: &[Value],
        through_pointer: bool,
    ) -> Result<Event, Diagnostic> {
        if self.activations.len() >= MAX_FRAMES {
            return Err(Diagnostic::limit(
                self.state.source_range,
                "call stack limit reached",
            ));
        }
        let name = program.functions[function].name.as_str();
        // Read before the callee's activation becomes the innermost.
        let callee_source = self.callee_text(program, instruction);
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
        let watermark = self.state.objects.len();
        self.activations.push(Activation {
            id,
            function,
            pc: 0,
            scope: 0,
            operands: Vec::new(),
            evaluation: None,
            accesses: Vec::new(),
            footprint: Vec::new(),
            watermark,
            listing: false,
        });
        let mut parameters = Vec::new();
        for ((name, ty), value) in program.functions[function]
            .parameters
            .iter()
            .zip(arguments.iter().cloned())
        {
            parameters.push(self.allocate(name, *ty, 1, value, true)?);
        }
        for (&id, &enumeration) in parameters
            .iter()
            .zip(&program.functions[function].enumerations)
        {
            self.name(
                id,
                Named {
                    alias: None,
                    enumeration,
                },
            );
        }
        let mut event = self.called(
            instruction.range,
            name,
            id,
            recursive,
            arguments,
            types,
            parameters,
        );
        if through_pointer {
            let kind = match instruction.op {
                | Op::CallPointer { callee, .. } => callee.kind,
                | _ => PointerSourceKind::Value,
            };
            event = self.called_through(event, program, function, (callee_source, kind));
        }
        Ok(event)
    }

    fn return_from(
        &mut self,
        program: &Program,
        range: super::SourceRange,
        from: Type,
        value: Value,
        fallthrough: bool,
    ) -> Result<Event, Diagnostic> {
        let top = self.activations.len() - 1;
        let id = self.activations[top].id;
        let defined = &program.functions[self.activations[top].function];
        // A function may return a pointer, to an object or to a function,
        // or a null one, which a null pointer constant such as 0 converts
        // to (C99 §6.3.2.3p3).
        let pointer = defined.returns.is_pointer();
        let value = match value {
            | Value::Integer(0) if pointer => Value::null(),
            | value => value,
        };
        let supported = match value {
            | Value::Integer(_) | Value::Floating(_) | Value::Struct { .. } => !pointer,
            | Value::Function(_) | Value::Pointer { .. } => pointer,
            | _ => false,
        };
        if !supported {
            return Err(self.error("function returned a value of an unsupported type"));
        }
        let function = defined.name.clone();
        // The value converts to the return type as if by assignment (C99
        // §6.8.6.4p3).
        let returns = defined.returns;
        let original = value.clone();
        let value = match returns.arithmetic() {
            | Some(arithmetic) => self.converted(&value, arithmetic)?,
            | None => value,
        };
        let mut ended = Vec::new();
        for object in &mut self.state.objects {
            if object.frame_id == id && object.lifetime == "alive" {
                object.lifetime = "ended";
                ended.push(object.id);
            }
        }
        let frame = self.activations.pop();
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
                // What the body read and wrote outside itself now belongs to
                // the call in the caller's full expression.
                let footprint = frame.map_or_else(Vec::new, |frame| frame.footprint);
                self.returned_footprint(program, node, &footprint)?;
            }
            node.map(|node| {
                let caller = &program.functions[self.activations[caller].function];
                let range = caller.nodes[node].range;
                &program.source[range.start..range.end]
            })
        };
        // A pointer to one of the call's own objects, which end now.
        let dangling = match value {
            | Value::Pointer {
                target: Some(target),
                ..
            } if ended.contains(&target) => Some(target),
            | _ => None,
        };
        let event = self.returned(
            range,
            &function,
            id,
            (from, returns, original),
            value,
            ended,
            fallthrough,
            call,
        );
        Ok(match dangling {
            | Some(target) => self.returned_dangling(event, &function, target),
            | None => event,
        })
    }

    /// Start tracking the full expression `instruction` belongs to in the
    /// innermost activation; statement-level operations clear it.
    fn track(&mut self, program: &Program, instruction: &Instruction) {
        let top = self.activations.len() - 1;
        let activation = &mut self.activations[top];
        let Some(node) = instruction.node else {
            if matches!(
                instruction.op,
                Op::Enter { .. }
                    | Op::Leave
                    | Op::Declare { .. }
                    | Op::Jump(_)
                    | Op::Break(..)
                    | Op::PastLabels(_)
                    | Op::Enumeration(_)
                    | Op::Do
                    | Op::Continue { .. }
                    | Op::Goto(_)
            ) {
                activation.evaluation = None;
                activation.accesses.clear();
                activation.listing = false;
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
            // The elements of an initializer list are one full expression,
            // though each has a tree of its own.
            if !activation.listing {
                activation.accesses.clear();
            }
            activation.evaluation = Some(Evaluation {
                function,
                root,
                values: Vec::new(),
                last: None,
                skipped: Vec::new(),
                calling: None,
            });
        }
        if let Some(evaluation) = &mut activation.evaluation {
            evaluation.last = Some(node);
        }
    }

    /// The source of each operand of the node `instruction` evaluates, in
    /// source order: `x > 0` and `x < 10` for `x > 0 && x < 10`.
    fn operand_texts(&self, program: &Program, instruction: &Instruction) -> Vec<String> {
        let function = &program.functions[self.activations[self.activations.len() - 1].function];
        instruction
            .node
            .and_then(|node| function.nodes.get(node))
            .map_or_else(Vec::new, |node| {
                node.children
                    .iter()
                    .filter_map(|&child| function.nodes.get(child))
                    .map(|child| {
                        program
                            .source
                            .get(child.range.start..child.range.end)
                            .map_or_else(String::new, String::from)
                    })
                    .collect()
            })
    }

    /// The spelling of the operand of a binary operator that is an integer
    /// 0 beside a pointer: a null pointer constant (C99 §6.3.2.3p3), which
    /// the narration names as written.
    fn null_constant_text(
        &self,
        program: &Program,
        instruction: &Instruction,
        (left, right): (&Value, &Value),
    ) -> Option<String> {
        let pointer = |value: &Value| matches!(value, Value::Pointer { .. } | Value::Function(_));
        let at = match (left, right) {
            | (value, Value::Integer(0)) if pointer(value) => 1,
            | (Value::Integer(0), value) if pointer(value) => 0,
            | _ => return None,
        };
        self.operand_texts(program, instruction).into_iter().nth(at)
    }

    /// Record that operand `child` of the node `instruction` evaluates,
    /// and every node of its subtree, will not be evaluated.
    fn skip(&mut self, program: &Program, instruction: &Instruction, child: usize) {
        let top = self.activations.len() - 1;
        let function = &program.functions[self.activations[top].function];
        let Some(&root) = instruction
            .node
            .and_then(|node| function.nodes.get(node))
            .and_then(|node| node.children.get(child))
        else {
            return;
        };
        let first = function.nodes[root].first;
        if let Some(evaluation) = &mut self.activations[top].evaluation {
            evaluation.skipped.extend(first..=root);
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
        self.state.upcoming = None;
        // Directives run in translation, and static initialization comes
        // before program startup: no call has begun until main's first
        // operation (C99 §5.1.1.2p1, §5.1.2p1).
        if !self.started
            && (!program.statics.is_empty()
                || !program.directives.is_empty()
                || !program.file_enumerations.is_empty())
        {
            self.state.evaluation = None;
            if !self.state.terminal() {
                self.state.source_range = self.before_main(program);
                self.state.upcoming = Some(self.upcoming_before_main(program));
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
            self.state.upcoming = Some(self.upcoming(program, next));
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
                    .file_enumerations
                    .get(self.enumerated)
                    .map(|&index| program.enumerations[index].range)
            })
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
            accesses:   Vec::new(),
            footprint:  Vec::new(),
            watermark:  0,
            listing:    false,
        }]);
        let first = program
            .directives
            .first()
            .map(|directive| directive.range)
            .or_else(|| {
                program
                    .file_enumerations
                    .first()
                    .map(|&index| program.enumerations[index].range)
            })
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
                upcoming:        None,
                diagnostics:     Vec::new(),
                return_value:    None,
                output:          Vec::new(),
                written:         0,
            },
            activations,
            next_frame: 2,
            cells: 0,
            metadata: Vec::new(),
            forgotten: Vec::new(),
            structures: Arc::clone(&program.structures),
            signatures: Arc::clone(&program.signatures),
            functions: program
                .functions
                .iter()
                .map(|function| function.name.clone())
                .collect(),
            enumerations: Arc::clone(&program.enumerations),
            aliases: Arc::clone(&program.aliases),
            enumerated: 0,
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
        self.state.upcoming = None;
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

    /// The value on top of the stack, for an operation that uses it: a
    /// pointer a call returned to one of its own objects, which ended with
    /// the call, stops the machine here (C99 §6.2.4p2).
    pub fn pop_value(&mut self) -> Result<Value, Diagnostic> {
        match self.pop()? {
            | Operand::Value(value) => {
                self.check_dangling(&value)?;
                Ok(value)
            },
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
            + self.state.frames.capacity() * size_of::<Frame>()
            + self.forgotten.capacity() * size_of::<(usize, Value)>();
        for (_, value) in &self.forgotten {
            bytes += value_bytes(value);
        }
        for object in &self.state.objects {
            bytes += object.name.capacity()
                + object.r#type.capacity()
                + object.function.as_ref().map_or(0, String::capacity)
                + value_bytes(&object.value);
        }
        for frame in &self.activations {
            bytes += frame.operands.capacity() * size_of::<Operand>()
                + frame.accesses.capacity() * size_of::<Access>()
                + frame.footprint.capacity() * size_of::<Touch>();
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
            + self
                .state
                .upcoming
                .as_ref()
                .map_or(0, |upcoming| upcoming.text.capacity())
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
    /// The reads and stores of that full expression so far, which
    /// `sequencing.rs` checks each new access against.
    pub accesses:   Vec<Access>,
    /// The reads and stores this call, and the calls it made, performed on
    /// objects that existed before it began: what the caller's expression
    /// must not also touch in an order C leaves open.
    pub footprint:  Vec<Touch>,
    /// How many objects existed when the call began; IDs up to it are the
    /// caller's world.
    pub watermark:  usize,
    /// Whether the last store was an element of an initializer list that
    /// more elements follow, so the full expression goes on.
    pub listing:    bool,
}

#[derive(Debug, Clone)]
pub(super) struct ObjectMetadata {
    pub scope:    usize,
    pub ty:       Type,
    /// Whether the object is defined `const` (C99 §6.7.3p5).
    pub constant: bool,
    /// The typedef name its type was spelled with, which names the type
    /// where the program did (C99 §6.7.7p3).
    pub typedef:  Option<&'static str>,
    /// The `Enter` instruction of the block that created it, for an
    /// automatic object a block declares.
    pub block:    Option<u32>,
    /// The line its declaration names it on, for an object a declaration
    /// creates; 0 for any other.
    pub line:     u32,
    /// Whether execution has reached its declaration since its block
    /// began it, so that reaching it again is a jump's doing.
    pub reached:  bool,
}
