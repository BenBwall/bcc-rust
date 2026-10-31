//! GNU assembly statements inside functions, lowered to one `asm`
//! instruction from the facts semantic analysis retained.
//!
//! The operands are evaluated in order: an output or memory operand as a
//! place, an input as its converted value, a `+` operand as both. Then
//! [`FunctionLowerer::finish_assembly`] builds the descriptor: memory
//! operands pass their address; a register or flag output passes a fresh
//! slot the assembly's result is stored to, which is then read and assigned
//! to the output's lvalue, so SSA variables, bit-fields and volatile objects
//! are written as an assignment writes them; a `+` operand becomes an
//! output and an input tied to it. A value whose class is the other kind of
//! register (a `float` in a general register, an `int` in an SSE register)
//! is reinterpreted with `bitcast`. The template is rewritten in LLVM's
//! syntax with the IR's operand numbers.
//!
//! `asm goto` becomes an `asm` with one more output, a selector in memory
//! that the template sets, so it needs no register: the assembly falls
//! through with 0, and each goto label names a local stub inside the same
//! template that sets its label's number and joins the fall-through. A `switch`
//! on the selector then takes the label's jump, with any cleanups it leaves, so
//! outputs are valid on every path and no IR terminator needs to know about
//! assembly.
//!
//! For `asm ("addl %1, %0" : "+r" (x) : "g" (5));` the instruction is
//! `asm sideeffect "addl $1, $0" (out "r" i32 v1, in "i" i32 v2, in "0" i32
//! v3)`, where `v1` addresses a fresh slot, `v2` is the constant 5 and `v3`
//! the value of `x`; `x` is assigned the slot's value afterwards.
//!
//! C99: §J.5.10, p. 514; PDF p. 526 (`asm` as a common extension); outputs
//! are assigned as by §6.5.16.1p2, p. 92; PDF p. 104.
//! GNU extension: GCC manual, "Extended Asm".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>

use std::fmt::Write as _;

use super::{
    Construct,
    FunctionLowerer,
    Item,
    LoweringError,
    Operand,
    Place,
    Task,
    ssa::{
        Op,
        Terminator,
    },
    types,
};
use crate::{
    ir::{
        ASM_CONDITIONS,
        Align,
        AsmClobbers,
        AsmConstraint,
        AsmOperand,
        InlineAsm,
        InstData,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Statement,
        semantic_analysis::{
            AsmFacts,
            AsmOperandFacts,
            AsmOperandKind,
            AsmPiece,
            AsmRegisterClass,
            AsmRole,
            ControlTarget,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

impl<'tu, 's> FunctionLowerer<'_, '_, 'tu, '_, 's> {
    /// Schedules the operands of a GNU `asm` statement, in order, before
    /// [`Task::Assembly`].
    pub(super) fn assembly(
        &mut self,
        statement: &'tu Statement<'tu>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let facts = self.assembly_facts(statement, source)?;
        self.tasks.push(Task::Assembly(statement));
        for operand in facts.operands.iter().rev() {
            let expression = operand.expression;
            match (operand.role, operand.kind) {
                // The assembly reads and writes a memory operand in place.
                | (AsmRole::Output, _)
                | (AsmRole::Update | AsmRole::Input, AsmOperandKind::Memory) =>
                    self.tasks.push(Task::EvaluateRaw(expression)),
                | (AsmRole::Update, _) => {
                    self.tasks.push(Task::ReadForUpdate(expression));
                    self.tasks.push(Task::EvaluateRaw(expression));
                },
                // The retained constant is the value; an integer constant
                // expression has no side effects to evaluate (§6.6p3).
                | (AsmRole::Input, AsmOperandKind::Immediate { symbolic: false }) => {},
                | (AsmRole::Input, _) => self.tasks.push(Task::Evaluate(expression)),
            }
        }
        Ok(())
    }

    /// Emits the `asm` instruction once its operands are on the stack, then
    /// assigns its register outputs and dispatches an `asm goto`.
    pub(super) fn finish_assembly(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) -> Result<(), LoweringError> {
        let source = statement.source_vectors;
        let facts = self.assembly_facts(statement, source)?;
        let facts = &facts;
        let count = facts.operands.iter().map(stack_items).sum::<usize>();
        let mut items = ArenaVec::with_capacity_in(count, self.scratch);
        for _ in 0..count {
            items.push(self.pop());
        }
        items.reverse();
        let mut items = items.into_iter();
        let mut outputs = ArenaVec::new_in(self.scratch);
        let mut inputs = ArenaVec::new_in(self.scratch);
        let mut tied = ArenaVec::new_in(self.scratch);
        // The read of each `+` memory operand, as Clang's `=*m,*m`.
        let mut reads = ArenaVec::new_in(self.scratch);
        // Each register output's slot, lvalue and register types.
        let mut writes = ArenaVec::new_in(self.scratch);
        // The register an output shares with its tied inputs holds the
        // widest of them, as in GCC; each reference prints at its own
        // operand's width.
        let widths = self.tied_widths(facts, source)?;
        let mut prints = ArenaVec::with_capacity_in(facts.operands.len(), self.scratch);
        for operand in facts.operands {
            let source = operand.expression.source_vectors;
            let mut next = || {
                items.next().ok_or_else(|| {
                    LoweringError::missing("an asm operand was not evaluated", source)
                })
            };
            if operand.role.writes() {
                let place = next()?;
                let widened = widths[outputs.len()];
                let (descriptor, value, print) =
                    self.output_operand(operand, place, widened, &mut writes)?;
                prints.push(print);
                if operand.role == AsmRole::Update && operand.kind == AsmOperandKind::Memory {
                    reads.push((
                        AsmOperand {
                            output: false,
                            ..descriptor
                        },
                        value,
                    ));
                } else if operand.role == AsmRole::Update {
                    let current = next()?;
                    let value = if widened.is_some() {
                        self.tied_value(current, descriptor.ty, source)?
                    } else {
                        self.register_value(current, descriptor.ty, source)?
                    };
                    let index = u32::try_from(outputs.len())
                        .map_err(|_| LoweringError::missing("too many asm operands", source))?;
                    tied.push((
                        AsmOperand {
                            output:        false,
                            early_clobber: false,
                            constraint:    AsmConstraint::Tied(index),
                            ty:            descriptor.ty,
                            bytes:         descriptor.bytes,
                        },
                        value,
                    ));
                }
                outputs.push((descriptor, value));
            } else {
                let item = match operand.kind {
                    | AsmOperandKind::Immediate { symbolic: false } => None,
                    | _ => Some(next()?),
                };
                let (descriptor, value, print) = self.input_operand(operand, item, &outputs)?;
                prints.push(print);
                inputs.push((descriptor, value));
            }
        }
        let gcc_outputs = outputs.len();
        // The selector is memory, so a statement that clobbers every
        // register can still set it.
        let selector = if facts.jumps.is_empty() {
            None
        } else {
            let slot = self.fresh_object(4, Align::from_bytes(4).unwrap_or(Align::BYTE), source)?;
            outputs.push((
                AsmOperand {
                    output:        true,
                    early_clobber: false,
                    constraint:    AsmConstraint::Memory,
                    ty:            Type::I32,
                    bytes:         4,
                },
                slot,
            ));
            Some(slot)
        };
        let mut operands = ArenaVec::new_in(self.scratch);
        let mut args = ArenaVec::new_in(self.scratch);
        for &(operand, value) in outputs.iter().chain(&inputs).chain(&tied).chain(&reads) {
            operands.push(operand);
            args.push(value);
        }
        let template = llvm_template(
            facts,
            &prints,
            gcc_outputs,
            selector.map(|_| gcc_outputs),
            self.scratch,
        );
        let asm = self.scratch.alloc(InlineAsm {
            template,
            operands: operands.leak(),
            clobbers: clobbers(facts),
            side_effects: facts.volatile,
        });
        // A combination of operands the IR cannot express is refused here,
        // never handed to the verifier.
        let mut broken = None;
        asm.check(|rule| _ = broken.get_or_insert(rule));
        if broken.is_some() {
            return Err(LoweringError::unsupported(
                Construct::InlineAssemblyOperand,
                source,
            ));
        }
        _ = self.draft.push(Op::InlineAsm(asm, args.leak()), None);
        for &(slot, place, register, value_type, ty) in &writes {
            let value = self.load_register(slot, register);
            // A widened output keeps its low bits, a flag zero-extends, and
            // a value in the other kind of register keeps its bits.
            let value = self.reinterpret(value, value_type, false);
            let item = Item {
                operand: Operand::Value(value),
                ty,
            };
            _ = self.write(place, item, source)?;
        }
        if let Some(slot) = selector {
            self.goto_dispatch(facts, slot, source)?;
        }
        Ok(())
    }

    /// `constraint`, narrowed to the register a GNU local register variable
    /// operand names, as GCC and Clang place one: the only effect of its
    /// register (GCC manual, "Local Register Variables"). Without `register`
    /// GCC and Clang ignore the label. A register lowering cannot place the
    /// operand in (not a general register, or outside the operand's class,
    /// as `%rsp` is outside `r`) is refused, never ignored.
    fn pinned(
        &self,
        operand: &AsmOperandFacts<'tu>,
        constraint: AsmConstraint,
    ) -> Result<AsmConstraint, LoweringError> {
        let mut expression = operand.expression;
        while let crate::translation_phases::parsing::syntax::ExpressionType::Parenthesized {
            expression: inner,
        } = expression.kind
        {
            expression = inner;
        }
        let Some(register) = self
            .info(expression)
            .ok()
            .filter(|info| info.register)
            .and_then(|info| info.binding)
            .and_then(|binding| self.unit.sema.entities.register_variable(binding))
        else {
            return Ok(constraint);
        };
        let refused = || {
            LoweringError::unsupported(
                Construct::RegisterVariableOperand,
                operand.expression.source_vectors,
            )
        };
        let (AsmConstraint::General(mask), Some(register)) = (constraint, register) else {
            return Err(refused());
        };
        let pinned = 1_u16 << register;
        if mask & pinned == 0 {
            return Err(refused());
        }
        Ok(AsmConstraint::General(pinned))
    }

    /// The descriptor and value of an output: its address for memory, else
    /// a fresh slot whose value [`Self::finish_assembly`] assigns to the
    /// lvalue.
    fn output_operand(
        &mut self,
        operand: &AsmOperandFacts<'tu>,
        place: Item,
        widened: Option<Type>,
        writes: &mut ArenaVec<'_, Write>,
    ) -> Result<(AsmOperand, Value, Option<Type>), LoweringError> {
        let source = operand.expression.source_vectors;
        if operand.kind == AsmOperandKind::Memory {
            let (address, ty, bytes) = self.memory_operand(place, source)?;
            return Ok((
                AsmOperand {
                    output: true,
                    early_clobber: operand.early_clobber,
                    constraint: AsmConstraint::Memory,
                    ty,
                    bytes,
                },
                address,
                None,
            ));
        }
        let value_ty = place.ty.unqualified();
        let value_type = self.value_type(value_ty, source)?;
        let (constraint, register) = match operand.kind {
            | AsmOperandKind::Register(class) => {
                let (constraint, register) = register_class(class, value_type, source)?;
                (self.pinned(operand, constraint)?, register)
            },
            | AsmOperandKind::Flag(condition) if value_type.is_int() => {
                let index = ASM_CONDITIONS
                    .iter()
                    .position(|&name| name == condition.name())
                    .and_then(|index| u8::try_from(index).ok())
                    .ok_or_else(|| LoweringError::missing("an unknown flag condition", source))?;
                // The condition is 0 or 1; a wider integer is zero-extended
                // from it when assigned, as GCC does for `__int128`.
                let register = if value_type.bits() > 64 {
                    Type::I32
                } else {
                    value_type
                };
                (AsmConstraint::Flag(index), register)
            },
            | _ =>
                return Err(LoweringError::missing(
                    "an asm output has no location",
                    source,
                )),
        };
        let base = register;
        // The shared register keeps the output's class: an SSE output
        // widens to the `f64` of the wider input's bits.
        let register = widened.map_or(base, |widened| same_class(base, widened.bits()));
        let print = (register != base).then_some(base);
        let bytes = register.bytes();
        let align = Align::from_bytes(u64::from(bytes)).unwrap_or(Align::BYTE);
        let slot = self.fresh_object(u64::from(bytes), align, source)?;
        writes.push((slot, place, register, value_type, value_ty));
        Ok((
            AsmOperand {
                output: true,
                early_clobber: operand.early_clobber,
                constraint,
                ty: register,
                bytes,
            },
            slot,
            print,
        ))
    }

    /// The register type each output shares with its tied inputs: the
    /// widest integer among them, as GCC gives the matched pair the wider
    /// mode, or `None` when the output's own type is widest. A pointer
    /// counts as a 64-bit integer.
    fn tied_widths(
        &self,
        facts: &AsmFacts<'tu>,
        source: SourceVectors,
    ) -> Result<ArenaVec<'s, Option<Type>>, LoweringError> {
        let mut widths = ArenaVec::with_capacity_in(facts.operands.len(), self.scratch);
        widths.resize(facts.operands.len(), None);
        for operand in facts.operands {
            let AsmOperandKind::Tied(output) = operand.kind else {
                continue;
            };
            let output_facts = &facts.operands[output as usize];
            let info = self.info(output_facts.expression)?;
            let own = self.value_type(info.ty.unqualified(), source)?;
            let info = self.info(operand.expression)?;
            let converted = self
                .conversions(operand.expression)
                .last()
                .map_or(info.ty, |conversion| conversion.ty);
            let input = match self.value_type(converted.unqualified(), source)? {
                | Type::Ptr => Type::I64,
                | ty => ty,
            };
            let current = widths[output as usize].unwrap_or(own);
            if input.is_int() && own.is_int() && input.bits() > current.bits() {
                widths[output as usize] = Some(input);
            }
        }
        Ok(widths)
    }

    /// The descriptor and value of an input; `item` is its evaluated value
    /// or place, absent for an integer constant.
    fn input_operand(
        &mut self,
        operand: &AsmOperandFacts<'tu>,
        item: Option<Item>,
        outputs: &[(AsmOperand, Value)],
    ) -> Result<(AsmOperand, Value, Option<Type>), LoweringError> {
        let source = operand.expression.source_vectors;
        let missing = || LoweringError::missing("an asm input was not evaluated", source);
        let descriptor = |constraint, ty: Type| AsmOperand {
            output: false,
            early_clobber: false,
            constraint,
            ty,
            bytes: ty.bytes(),
        };
        let mut print = None;
        let (descriptor, value) = match operand.kind {
            | AsmOperandKind::Memory => {
                let (address, ty, bytes) =
                    self.memory_operand(item.ok_or_else(missing)?, source)?;
                (
                    AsmOperand {
                        bytes,
                        ..descriptor(AsmConstraint::Memory, ty)
                    },
                    address,
                )
            },
            | AsmOperandKind::Register(class) => {
                let item = item.ok_or_else(missing)?;
                let value_type = self.value_type(item.ty, source)?;
                let (constraint, register) = register_class(class, value_type, source)?;
                let constraint = self.pinned(operand, constraint)?;
                let value = self.register_value(item, register, source)?;
                (descriptor(constraint, register), value)
            },
            | AsmOperandKind::Tied(output) => {
                let item = item.ok_or_else(missing)?;
                let (tied, _) = outputs.get(output as usize).ok_or_else(|| {
                    LoweringError::missing("a tied asm output is missing", source)
                })?;
                let own = self.value_type(item.ty, source)?;
                if own.is_int() && own.bits() < tied.ty.bits() {
                    print = Some(own);
                }
                let value = self.tied_value(item, tied.ty, source)?;
                (descriptor(AsmConstraint::Tied(output), tied.ty), value)
            },
            | AsmOperandKind::Immediate { symbolic: false } => {
                let info = self.info(operand.expression)?;
                let value = info
                    .integer
                    .and_then(crate::translation_phases::semantic_analysis::Integer::to_i128)
                    .ok_or_else(|| {
                        LoweringError::missing("an asm immediate has no value", source)
                    })?;
                let ty = match self.repr(info.ty.unqualified(), source) {
                    | Ok(types::Repr::Int { ty, .. }) if ty != Type::I128 => ty,
                    | Ok(types::Repr::Bool) => Type::I8,
                    | _ => Type::I64,
                };
                let value = self.iconst(ty, value);
                (descriptor(AsmConstraint::Immediate, ty), value)
            },
            | AsmOperandKind::Immediate { symbolic: true } => {
                let value = self.value(item.ok_or_else(missing)?, source)?;
                if !self.draft.is_symbol_address(value) {
                    return Err(LoweringError::unsupported(
                        Construct::InlineAssemblyOperand,
                        source,
                    ));
                }
                (descriptor(AsmConstraint::Immediate, Type::Ptr), value)
            },
            | AsmOperandKind::Flag(_) =>
                return Err(LoweringError::missing("a flag operand is an input", source)),
        };
        Ok((descriptor, value, print))
    }

    /// The address of a memory operand's lvalue, the scalar type it holds
    /// (`i8` for an aggregate) and its size.
    fn memory_operand(
        &mut self,
        place: Item,
        source: SourceVectors,
    ) -> Result<(Value, Type, u32), LoweringError> {
        let address = match place.operand {
            | Operand::Place(Place::Memory(address)) | Operand::Aggregate(address) => address,
            | Operand::Place(Place::Function(func)) => self.nested_address(func, source)?,
            | _ =>
                return Err(LoweringError::missing(
                    "an asm memory operand is not in memory",
                    source,
                )),
        };
        let layout =
            types::layout(&self.unit.sema.types, place.ty).map_err(|kind| LoweringError {
                kind,
                source: Some(source),
            })?;
        let bytes = u32::try_from(layout.size)
            .ok()
            .filter(|&bytes| bytes != 0)
            .ok_or(LoweringError::unsupported(
                Construct::InlineAssemblyOperand,
                source,
            ))?;
        let ty = self
            .repr(place.ty.unqualified(), source)
            .ok()
            .and_then(types::Repr::value_type)
            .filter(|ty| ty.bytes() == bytes)
            .unwrap_or(Type::I8);
        Ok((address, ty, bytes))
    }

    /// An operand's value in `register`'s type: the same bits, reinterpreted
    /// between an integer and a floating type of one width.
    fn register_value(
        &mut self,
        item: Item,
        register: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let value = self.value(item, source)?;
        let ty = self.draft.value_type(value);
        Ok(if ty == register {
            value
        } else {
            self.convert(Opcode::Bitcast, register, value)
        })
    }

    /// A tied input's value in its output's type: an integer narrower than
    /// the output is extended by its signedness, as GCC and Clang do, a
    /// wider one keeps its low bits, which is what the narrower output
    /// register holds, and pointers convert to and from integers.
    fn tied_value(
        &mut self,
        item: Item,
        output: Type,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let signed = self.repr(item.ty, source)?.signed();
        let value = self.value(item, source)?;
        Ok(self.reinterpret(value, output, signed))
    }

    /// `value` as IR type `to` through its bits: a pointer or floating value
    /// becomes the integer of its width, that integer is extended (by
    /// `signed`) or truncated to `to`'s width, and becomes `to`.
    fn reinterpret(&mut self, value: Value, to: Type, signed: bool) -> Value {
        let from = self.draft.value_type(value);
        if from == to {
            return value;
        }
        let bits = match from {
            | Type::Ptr => self.convert(Opcode::Ptrtoint, Type::I64, value),
            | _ if from.is_float() => self.convert(Opcode::Bitcast, integer_of(from), value),
            | _ => value,
        };
        let target = integer_of(to);
        let bits = match integer_of(from).bits().cmp(&target.bits()) {
            | std::cmp::Ordering::Less => self.extend(bits, signed, target),
            | std::cmp::Ordering::Greater => self.convert(Opcode::Trunc, target, bits),
            | std::cmp::Ordering::Equal => bits,
        };
        match to {
            | Type::Ptr => self.convert(Opcode::Inttoptr, to, bits),
            | _ if to.is_float() => self.convert(Opcode::Bitcast, to, bits),
            | _ => bits,
        }
    }

    fn load_register(&mut self, slot: Value, ty: Type) -> Value {
        let align = Align::from_bytes(u64::from(ty.bytes())).unwrap_or(Align::BYTE);
        self.inst(
            InstData::Load {
                ty,
                flags: crate::ir::InstFlags::empty(),
                align,
                tag: None,
                addr: slot,
            },
            ty,
        )
    }

    /// Branches on an `asm goto`'s selector: 0 continues after the
    /// statement, `N + 1` takes the jump to label `N`, running the cleanups
    /// of the scopes it leaves.
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149 (the jumps are checked as
    /// `goto`s are).
    fn goto_dispatch(
        &mut self,
        facts: &AsmFacts<'tu>,
        slot: Value,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let selector = self.load_register(slot, Type::I32);
        let next = self.draft.create_block(&[]);
        let mut cases = ArenaVec::new_in(self.draft.scratch());
        let mut stubs = ArenaVec::new_in(self.scratch);
        for label in 0..facts.jumps.len() {
            let stub = self.draft.create_block(&[]);
            let case = i128::try_from(label + 1)
                .map_err(|_| LoweringError::missing("too many asm goto labels", source))?;
            cases.push((case, self.draft.edge(stub, &[])));
            stubs.push(stub);
        }
        let default = self.draft.edge(next, &[]);
        self.draft.terminate(Terminator::Switch {
            value: selector,
            default,
            cases,
        });
        for (index, &stub) in stubs.iter().enumerate() {
            self.draft.seal(stub);
            self.draft.switch_to(stub);
            let jump = self
                .unit
                .sema
                .control
                .jumps
                .get(facts.jumps[index].0)
                .copied()
                .ok_or_else(|| {
                    LoweringError::missing("an asm goto jump is not retained", source)
                })?;
            self.cleanup_transition(jump.transition, jump.provenance)?;
            let ControlTarget::Label(label) = jump.target else {
                return Err(LoweringError::missing(
                    "an asm goto jump does not reach a label",
                    source,
                ));
            };
            let block = self.label_block(label);
            self.draft.jump(block, &[]);
        }
        self.draft.seal(next);
        self.draft.switch_to(next);
        Ok(())
    }

    fn assembly_facts(
        &self,
        statement: &'tu Statement<'tu>,
        source: SourceVectors,
    ) -> Result<AsmFacts<'tu>, LoweringError> {
        self.unit
            .sema
            .assembly
            .statement(statement)
            .ok_or_else(|| LoweringError::missing("asm statement facts are not retained", source))
    }
}

/// The items an operand's evaluation leaves on the operand stack.
fn stack_items(operand: &AsmOperandFacts<'_>) -> usize {
    match (operand.role, operand.kind) {
        | (AsmRole::Update, AsmOperandKind::Memory) => 1,
        | (AsmRole::Update, _) => 2,
        | (AsmRole::Input, AsmOperandKind::Immediate { symbolic: false }) => 0,
        | _ => 1,
    }
}

/// The IR constraint and register type of a value of IR type `ty` in a
/// register of `class`: integers and pointers in general registers, `f32`
/// and `f64` in SSE registers, and a float or integer of the other kind
/// reinterpreted at the same width.
fn register_class(
    class: AsmRegisterClass,
    ty: Type,
    source: SourceVectors,
) -> Result<(AsmConstraint, Type), LoweringError> {
    let unsupported = || LoweringError::unsupported(Construct::InlineAssemblyOperand, source);
    match class {
        | AsmRegisterClass::General(mask) => {
            let register = match ty {
                | Type::F32 => Type::I32,
                | Type::F64 => Type::I64,
                | Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr => ty,
                | _ => return Err(unsupported()),
            };
            Ok((
                AsmConstraint::General(mask & AsmRegisterClass::ANY_GENERAL),
                register,
            ))
        },
        | AsmRegisterClass::Sse => {
            let register = match ty {
                | Type::I32 | Type::F32 => Type::F32,
                | Type::I64 | Type::F64 => Type::F64,
                | _ => return Err(unsupported()),
            };
            Ok((AsmConstraint::Sse, register))
        },
        | AsmRegisterClass::Other(_) => Err(unsupported()),
    }
}

/// The IR clobbers of a statement.
fn clobbers(facts: &AsmFacts<'_>) -> AsmClobbers {
    let clobbers = facts.clobbers;
    AsmClobbers {
        memory:  clobbers.memory,
        flags:   clobbers.flags,
        general: clobbers.general,
        sse:     clobbers.sse,
        x87:     clobbers.x87,
        mmx:     clobbers.mmx,
        mask:    clobbers.mask,
    }
}

/// The template in LLVM's syntax. GCC numbers operands as the IR orders
/// them, except that an `asm goto` selector follows the `outputs` written
/// outputs, so later operands move up by one. Goto labels become local
/// stubs appended after the template.
fn llvm_template<'s>(
    facts: &AsmFacts<'_>,
    prints: &[Option<Type>],
    outputs: usize,
    selector: Option<usize>,
    scratch: &'s crate::util::bump::Bump,
) -> &'s str {
    let mut text = ArenaString::new_in(scratch);
    let operand = |index: u32| {
        let index = index as usize;
        if selector.is_some() && index >= outputs {
            index + 1
        } else {
            index
        }
    };
    for piece in facts.template {
        match *piece {
            | AsmPiece::Text(literal) =>
                for c in literal.chars() {
                    if c == '$' {
                        text.push_str("$$");
                    } else {
                        text.push(c);
                    }
                },
            | AsmPiece::Operand {
                operand: index,
                modifier,
            } => {
                // A reference to an operand narrower than the register it
                // shares prints at the operand's own width.
                let modifier = modifier.or_else(|| {
                    prints
                        .get(index as usize)
                        .copied()
                        .flatten()
                        .and_then(size_modifier)
                });
                let index = operand(index);
                _ = match modifier {
                    | Some(modifier) => write!(text, "${{{index}:{}}}", char::from(modifier)),
                    | None => write!(text, "${index}"),
                };
            },
            | AsmPiece::Label(label) => _ = write!(text, "{GOTO_PREFIX}${{:uid}}_{label}"),
            | AsmPiece::UniqueId => text.push_str("${:uid}"),
        }
    }
    if let Some(selector) = selector {
        let end = "_end";
        _ = write!(
            text,
            "\n\tmovl $$0, ${selector}\n\tjmp {GOTO_PREFIX}${{:uid}}{end}"
        );
        for label in 0..facts.jumps.len() {
            _ = write!(
                text,
                "\n{GOTO_PREFIX}${{:uid}}_{label}:\n\tmovl $${}, ${selector}\n\tjmp \
                 {GOTO_PREFIX}${{:uid}}{end}",
                label + 1
            );
        }
        _ = write!(text, "\n{GOTO_PREFIX}${{:uid}}{end}:");
    }
    text.into_str()
}

/// The register type of `base`'s class at `bits`: the floating type for an
/// SSE register, else the integer.
fn same_class(base: Type, bits: u32) -> Type {
    match (base.is_float(), bits) {
        | (true, 32) => Type::F32,
        | (true, _) => Type::F64,
        | (false, _) => types::int_type(bits),
    }
}

/// The integer type with `ty`'s bits; a pointer is a 64-bit integer.
fn integer_of(ty: Type) -> Type {
    if ty == Type::Ptr {
        Type::I64
    } else {
        types::int_type(ty.bits())
    }
}

/// The x86 modifier that names an integer register at `ty`'s width.
fn size_modifier(ty: Type) -> Option<u8> {
    match ty {
        | Type::I8 => Some(b'b'),
        | Type::I16 => Some(b'w'),
        | Type::I32 => Some(b'k'),
        | Type::I64 | Type::Ptr => Some(b'q'),
        | _ => None,
    }
}

/// A register output's slot and lvalue, the register type it shares with
/// its tied inputs, and its value's IR and C types.
type Write = (
    Value,
    Item,
    Type,
    Type,
    crate::translation_phases::semantic_analysis::TypeId,
);

/// The prefix of an `asm goto` statement's local labels; `.L` keeps them
/// out of the symbol table on ELF and COFF.
const GOTO_PREFIX: &str = ".Lbcc_asm_goto";
