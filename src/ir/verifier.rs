//! Checks the invariants every pass and back end relies on.
//!
//! The verifier runs after lowering, after each pass in debug and test
//! builds, and on every parsed test input. It never panics on broken IR: it
//! records each problem as a [`VerifierError`] and keeps going where it
//! safely can. A function is checked in four steps: its block structure, the
//! records of where each value is defined, each instruction's types, flags
//! and edges, and finally that every use is dominated by its definition.

use std::fmt;

use super::{
    Body,
    FuncId,
    GlobalId,
    Module,
    cfg::ControlFlowGraph,
    dominators::DominatorTree,
    entities::{
        Block,
        Entity,
        Inst,
        Value,
    },
    function::ValueDef,
    instructions::{
        BlockCall,
        InstData,
        InstFlags,
        Opcode,
    },
    module::{
        GlobalInit,
        Signature,
        Symbol,
    },
    types::Type,
};
use crate::util::bump::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    Bump,
};

/// Verifies every global and function of `module`, returning the errors in
/// `scratch`. An empty vector means the module is well formed for `profile`.
pub(crate) fn verify_module<'m, 's>(
    module: &'m Module<'_>,
    profile: Profile,
    scratch: &'s Bump,
) -> ArenaVec<'s, VerifierError<'m>> {
    let mut errors = ArenaVec::new_in(scratch);
    for (global, _) in module.globals() {
        verify_global(module, global, &mut errors);
    }
    for (func, _) in module.functions() {
        verify_function_into(module, func, profile, scratch, &mut errors);
    }
    errors
}

/// Verifies one function, returning the errors in `scratch`.
pub(crate) fn verify_function<'m, 's>(
    module: &'m Module<'_>,
    func: FuncId,
    profile: Profile,
    scratch: &'s Bump,
) -> ArenaVec<'s, VerifierError<'m>> {
    let mut errors = ArenaVec::new_in(scratch);
    verify_function_into(module, func, profile, scratch, &mut errors);
    errors
}

fn verify_function_into<'m, 's>(
    module: &'m Module<'_>,
    func: FuncId,
    profile: Profile,
    scratch: &'s Bump,
    errors: &mut ArenaVec<'s, VerifierError<'m>>,
) {
    let function = module.function(func);
    let Some(signature) = module.get_signature(function.signature) else {
        errors.push(VerifierError {
            item:     function.name,
            location: Location::Item,
            kind:     VerifierErrorKind::InvalidReference(
                EntityKind::Signature,
                function.signature.as_u32(),
            ),
        });
        return;
    };
    let Some(body) = &function.body else {
        return;
    };
    let mut positions = ArenaVec::with_capacity_in(body.inst_count(), scratch);
    positions.resize(body.inst_count(), None);
    let mut verifier = Verifier {
        module,
        body,
        name: function.name,
        signature,
        profile,
        scratch,
        errors,
        positions,
    };
    if !verifier.structure() {
        return;
    }
    verifier.definitions();
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            verifier.instruction(inst);
        }
    }
    verifier.dominance();
}

/// Which stage of lowering a module is in, and so which constraints apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Profile {
    /// Before ABI lowering: `va_arg` is an operation.
    PreAbi,
    /// After ABI lowering: `va_arg` has been expanded for the target. Calls
    /// pass only scalars and pointers in both profiles today, since
    /// aggregates are never values; by-value aggregate parameters, when they
    /// arrive, are pre-ABI only.
    PostAbi,
}

impl Profile {
    /// Whether `opcode` may appear in this profile.
    pub(crate) const fn allows(self, opcode: Opcode) -> bool {
        !matches!((self, opcode), (Self::PostAbi, Opcode::VaArg))
    }
}

struct Verifier<'a, 'm, 'ir, 's> {
    module:    &'m Module<'ir>,
    body:      &'m Body<'ir>,
    name:      &'m str,
    signature: Signature<'ir>,
    profile:   Profile,
    scratch:   &'s Bump,
    errors:    &'a mut ArenaVec<'s, VerifierError<'m>>,
    /// Where each placed instruction is: its block and index there.
    positions: ArenaVec<'s, Option<(Block, usize)>>,
}

impl Verifier<'_, '_, '_, '_> {
    /// Checks that the body has an entry block matching the signature, that
    /// each block is a run of non-terminators ended by one terminator, and
    /// that each instruction appears once. Returns whether the later checks
    /// can rely on the block lists.
    fn structure(&mut self) -> bool {
        let Some(entry) = self.body.entry_block() else {
            self.error(Location::Item, VerifierErrorKind::NoBlocks);
            return false;
        };
        let params = self.body.block_params(entry);
        if params.len() == self.signature.params.len() {
            for (index, (&param, &expected)) in params.iter().zip(self.signature.params).enumerate()
            {
                if param.index() < self.body.value_count()
                    && self.body.value_type(param) != expected
                {
                    self.error(
                        Location::Block(entry),
                        VerifierErrorKind::EntryParameterType {
                            index,
                            expected,
                            found: self.body.value_type(param),
                        },
                    );
                }
            }
        } else {
            self.error(
                Location::Block(entry),
                VerifierErrorKind::EntryParameterCount {
                    expected: self.signature.params.len(),
                    found:    params.len(),
                },
            );
        }
        let mut sound = true;
        for block in self.body.blocks() {
            let insts = self.body.block_insts(block);
            if insts.is_empty() {
                self.error(Location::Block(block), VerifierErrorKind::EmptyBlock);
                continue;
            }
            for (index, &inst) in insts.iter().enumerate() {
                let Some(position) = self.positions.get_mut(inst.index()) else {
                    self.error(
                        Location::Block(block),
                        VerifierErrorKind::InvalidReference(EntityKind::Inst, inst.as_u32()),
                    );
                    sound = false;
                    continue;
                };
                if position.is_some() {
                    self.error(Location::Inst(inst), VerifierErrorKind::InstructionReused);
                    sound = false;
                    continue;
                }
                *position = Some((block, index));
                let terminator = self.body.inst(inst).is_terminator();
                if terminator && index + 1 != insts.len() {
                    self.error(Location::Inst(inst), VerifierErrorKind::TerminatorNotLast);
                } else if !terminator && index + 1 == insts.len() {
                    self.error(Location::Block(block), VerifierErrorKind::MissingTerminator);
                }
            }
        }
        sound
    }

    /// Checks that the value table and the definitions agree: a result's
    /// value names its instruction and a parameter's value its block and
    /// position, in both directions, with the type the instruction defines.
    fn definitions(&mut self) {
        let body = self.body;
        for index in 0..body.value_count() {
            let value = Value::new(index);
            let consistent = match body.value_def(value) {
                | ValueDef::Result(inst) =>
                    inst.index() < body.inst_count() && body.inst_result(inst) == Some(value),
                | ValueDef::Param(block, position) =>
                    block.index() < body.block_count()
                        && body.block_params(block).get(position as usize) == Some(&value),
            };
            if !consistent {
                self.error(Location::Item, VerifierErrorKind::ValueDefinition(value));
            }
        }
        for block in body.blocks() {
            for &param in body.block_params(block) {
                if param.index() >= body.value_count() {
                    self.error(
                        Location::Block(block),
                        VerifierErrorKind::InvalidReference(EntityKind::Value, param.as_u32()),
                    );
                }
            }
            for &inst in body.block_insts(block) {
                let result = body.inst_result(inst);
                if let Some(value) = result
                    && value.index() >= body.value_count()
                {
                    self.error(
                        Location::Inst(inst),
                        VerifierErrorKind::InvalidReference(EntityKind::Value, value.as_u32()),
                    );
                    continue;
                }
                let expected = self.module.result_type(body.inst(inst));
                let found = result.map(|value| body.value_type(value));
                if expected != found {
                    self.error(
                        Location::Inst(inst),
                        VerifierErrorKind::ResultType { expected, found },
                    );
                }
            }
        }
    }

    /// Checks one instruction's flags, profile, types and edges.
    fn instruction(&mut self, inst: Inst) {
        let data = *self.body.inst(inst);
        let opcode = data.opcode();
        let illegal = data.flags().difference(opcode.allowed_flags());
        if !illegal.is_empty() {
            self.error(
                Location::Inst(inst),
                VerifierErrorKind::IllegalFlags {
                    opcode,
                    flags: illegal,
                },
            );
        }
        if !self.profile.allows(opcode) {
            self.error(
                Location::Inst(inst),
                VerifierErrorKind::NotAllowedInProfile {
                    opcode,
                    profile: self.profile,
                },
            );
        }
        let mut checker = InstChecker {
            verifier: self,
            inst,
            opcode,
        };
        match data {
            | InstData::Binary { ty, args, .. } => {
                let operands = if opcode.is_int_binary() {
                    checker.require(ty.is_int(), ty);
                    [ty, ty]
                } else if opcode.is_float_binary() {
                    checker.require(ty.is_float(), ty);
                    [ty, ty]
                } else if opcode == Opcode::PtrAdd {
                    checker.require(ty == Type::Ptr, ty);
                    [Type::Ptr, Type::I64]
                } else {
                    checker.require(opcode == Opcode::VaCopy, ty);
                    [Type::Ptr, Type::Ptr]
                };
                checker.operands(&args, &operands);
            },
            | InstData::Unary { ty, arg, .. } => match opcode {
                | Opcode::Fneg => {
                    checker.require(ty.is_float(), ty);
                    checker.operand(0, arg, ty);
                },
                | Opcode::Freeze => checker.operand(0, arg, ty),
                | Opcode::VaArg | Opcode::VaStart | Opcode::VaEnd =>
                    checker.operand(0, arg, Type::Ptr),
                | _ if opcode.is_conversion() => {
                    if let Some(from) = checker.value_type(arg)
                        && !conversion_is_legal(opcode, from, ty)
                    {
                        checker.report(VerifierErrorKind::Conversion {
                            opcode,
                            from,
                            to: ty,
                        });
                    }
                },
                | _ => checker.require(false, ty),
            },
            | InstData::IntCompare { ty, args, .. } => {
                checker.require(ty.is_int() || ty == Type::Ptr, ty);
                checker.operands(&args, &[ty, ty]);
            },
            | InstData::FloatCompare { ty, args, .. } => {
                checker.require(ty.is_float(), ty);
                checker.operands(&args, &[ty, ty]);
            },
            | InstData::Select { ty, args } => checker.operands(&args, &[Type::I1, ty, ty]),
            | InstData::Const { ty, .. } | InstData::WideConst { ty, .. } => {
                checker.require(
                    match opcode {
                        | Opcode::Iconst => ty.is_int(),
                        | Opcode::Fconst => ty.is_float(),
                        | _ => false,
                    },
                    ty,
                );
                match checker.verifier.body.const_bits(&data) {
                    | None =>
                        if let InstData::WideConst { constant, .. } = data {
                            checker.report(VerifierErrorKind::InvalidReference(
                                EntityKind::Constant,
                                constant.as_u32(),
                            ));
                        },
                    | Some(bits) if bits & !ty.mask() != 0 =>
                        checker.report(VerifierErrorKind::ConstantOutOfRange(ty)),
                    | Some(_) => {},
                }
            },
            | InstData::Nullary { ty, .. } => checker.require(
                opcode == Opcode::Poison || (opcode == Opcode::Null && ty == Type::Ptr),
                ty,
            ),
            | InstData::StackAddr { slot } => {
                if slot.index() >= checker.verifier.body.stack_slot_count() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::StackSlot,
                        slot.as_u32(),
                    ));
                }
            },
            | InstData::GlobalAddr { global } => {
                if checker.verifier.module.get_global(global).is_none() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Global,
                        global.as_u32(),
                    ));
                }
            },
            | InstData::FuncAddr { func } => {
                if checker.verifier.module.get_function(func).is_none() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Function,
                        func.as_u32(),
                    ));
                }
            },
            | InstData::Load { addr, .. } => checker.operand(0, addr, Type::Ptr),
            | InstData::Store { ty, args, .. } => checker.operands(&args, &[ty, Type::Ptr]),
            | InstData::MemoryRange { args, .. } => match opcode {
                | Opcode::Copy => checker.operands(&args, &[Type::Ptr, Type::Ptr, Type::I64]),
                | Opcode::Fill => checker.operands(&args, &[Type::Ptr, Type::I8, Type::I64]),
                | _ => checker.require(false, Type::Ptr),
            },
            | InstData::Call { func, args } => {
                let module = checker.verifier.module;
                match module
                    .get_function(func)
                    .and_then(|function| module.get_signature(function.signature))
                {
                    | Some(signature) => {
                        let args = checker.verifier.body.value_list(args);
                        checker.call_arguments(signature, args);
                    },
                    | None => checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Function,
                        func.as_u32(),
                    )),
                }
            },
            | InstData::CallIndirect { sig, args } =>
                match checker.verifier.module.get_signature(sig) {
                    | Some(signature) => {
                        let values = checker.verifier.body.value_list(args);
                        if let Some((&callee, args)) = values.split_first() {
                            checker.operand(0, callee, Type::Ptr);
                            checker.call_arguments(signature, args);
                        } else {
                            checker.report(VerifierErrorKind::MissingCallee);
                        }
                    },
                    | None => checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Signature,
                        sig.as_u32(),
                    )),
                },
            | InstData::Jump { dest } => checker.edges(&[dest]),
            | InstData::Brif { cond, dests } => {
                checker.operand(0, cond, Type::I1);
                checker.edges(&dests);
            },
            | InstData::Switch {
                value,
                default,
                cases,
            } => checker.switch(value, default, cases),
            | InstData::Return { value } => {
                let expected = checker.verifier.signature.result;
                let found = value.and_then(|value| checker.value_type(value));
                if value.is_some() && found.is_none() {
                    return;
                }
                if expected != found {
                    checker.report(VerifierErrorKind::ReturnValue { expected, found });
                }
            },
            | InstData::Unreachable => {},
        }
    }

    /// Checks that every use in a reachable block is dominated by its
    /// definition: a parameter of a dominating block, or a result defined
    /// earlier in the same block or in a dominating block. A branch argument
    /// is used at its terminator. Uses in unreachable blocks are not checked.
    fn dominance(&mut self) {
        let body = self.body;
        let cfg = ControlFlowGraph::compute(body, self.scratch);
        let tree = DominatorTree::compute(body, &cfg, self.scratch);
        for &block in tree.reverse_post_order() {
            for (index, &inst) in body.block_insts(block).iter().enumerate() {
                let mut undominated = None;
                body.visit_operands(inst, |value| {
                    if undominated.is_some() || value.index() >= body.value_count() {
                        return;
                    }
                    let dominated = match body.value_def(value) {
                        | ValueDef::Param(def_block, _) => tree.dominates(def_block, block),
                        | ValueDef::Result(def_inst) =>
                            match self.positions.get(def_inst.index()).copied().flatten() {
                                | Some((def_block, def_index)) if def_block == block =>
                                    def_index < index,
                                | Some((def_block, _)) => tree.dominates(def_block, block),
                                | None => false,
                            },
                    };
                    if !dominated {
                        undominated = Some(value);
                    }
                });
                if let Some(value) = undominated {
                    self.error(Location::Inst(inst), VerifierErrorKind::NotDominated(value));
                }
            }
        }
    }

    fn error(&mut self, location: Location, kind: VerifierErrorKind) {
        self.errors.push(VerifierError {
            item: self.name,
            location,
            kind,
        });
    }
}

/// The checks of one instruction, which share its location.
struct InstChecker<'v, 'a, 'm, 'ir, 's> {
    verifier: &'v mut Verifier<'a, 'm, 'ir, 's>,
    inst:     Inst,
    opcode:   Opcode,
}

impl InstChecker<'_, '_, '_, '_, '_> {
    fn report(&mut self, kind: VerifierErrorKind) {
        self.verifier.error(Location::Inst(self.inst), kind);
    }

    /// Reports the controlling type unless `legal`.
    fn require(&mut self, legal: bool, ty: Type) {
        if !legal {
            let opcode = self.opcode;
            self.report(VerifierErrorKind::InstructionType { opcode, ty });
        }
    }

    /// The type of `value`, reporting it if it does not exist.
    fn value_type(&mut self, value: Value) -> Option<Type> {
        if value.index() < self.verifier.body.value_count() {
            Some(self.verifier.body.value_type(value))
        } else {
            self.report(VerifierErrorKind::InvalidReference(
                EntityKind::Value,
                value.as_u32(),
            ));
            None
        }
    }

    fn operand(&mut self, index: usize, value: Value, expected: Type) {
        if let Some(found) = self.value_type(value)
            && found != expected
        {
            self.report(VerifierErrorKind::OperandType {
                index,
                expected,
                found,
            });
        }
    }

    fn operands(&mut self, values: &[Value], expected: &[Type]) {
        for (index, (&value, &ty)) in values.iter().zip(expected).enumerate() {
            self.operand(index, value, ty);
        }
    }

    /// Checks call arguments against a signature: the fixed ones by type,
    /// and any extra ones only if the signature is variadic.
    fn call_arguments(&mut self, signature: Signature<'_>, args: &[Value]) {
        let fixed = signature.params.len();
        if args.len() < fixed || (args.len() > fixed && !signature.variadic) {
            self.report(VerifierErrorKind::CallArgumentCount {
                expected: fixed,
                found:    args.len(),
                variadic: signature.variadic,
            });
            return;
        }
        for (index, (&arg, &expected)) in args.iter().zip(signature.params).enumerate() {
            if let Some(found) = self.value_type(arg)
                && found != expected
            {
                self.report(VerifierErrorKind::CallArgumentType {
                    index,
                    expected,
                    found,
                });
            }
        }
        for &arg in &args[fixed..] {
            _ = self.value_type(arg);
        }
    }

    /// Checks each edge's target and arguments, and that edges to one block
    /// pass identical arguments, so each maps to one LLVM phi entry.
    fn edges(&mut self, dests: &[BlockCall]) {
        let body = self.verifier.body;
        let mut seen: ArenaMap<'_, Block, BlockCall> =
            ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        for &dest in dests {
            let (target, args) = body.block_call(dest);
            if target.index() >= body.block_count() {
                self.report(VerifierErrorKind::InvalidReference(
                    EntityKind::Block,
                    target.as_u32(),
                ));
                continue;
            }
            if Some(target) == body.entry_block() {
                self.report(VerifierErrorKind::BranchToEntry);
            }
            let params = body.block_params(target);
            if params.len() == args.len() {
                for (index, (&arg, &param)) in args.iter().zip(params).enumerate() {
                    let expected = body.value_type(param);
                    if let Some(found) = self.value_type(arg)
                        && found != expected
                    {
                        self.report(VerifierErrorKind::EdgeArgumentType {
                            target,
                            index,
                            expected,
                            found,
                        });
                    }
                }
            } else {
                self.report(VerifierErrorKind::EdgeArgumentCount {
                    target,
                    expected: params.len(),
                    found: args.len(),
                });
            }
            match seen.get(&target) {
                | Some(&previous) if body.block_call(previous).1 != args =>
                    self.report(VerifierErrorKind::DuplicateEdgeArguments(target)),
                | Some(_) => {},
                | None => _ = seen.insert(target, dest),
            }
        }
    }

    /// Checks a switch: an integer value, cases that fit its type and are
    /// distinct, and its edges.
    ///
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146 (distinct case values).
    fn switch(&mut self, value: Value, default: BlockCall, cases: super::CaseList) {
        let body = self.verifier.body;
        let ty = self.value_type(value);
        if let Some(ty) = ty
            && !ty.is_int()
        {
            self.report(VerifierErrorKind::OperandType {
                index:    0,
                expected: Type::I64,
                found:    ty,
            });
        }
        let mut dests =
            ArenaVec::with_capacity_in(1 + body.switch_cases(cases).len(), self.verifier.scratch);
        dests.push(default);
        let mut seen: ArenaSet<'_, u128> =
            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        for (bits, dest) in body.switch_cases(cases) {
            dests.push(dest);
            if let Some(ty) = ty
                && bits & !ty.mask() != 0
            {
                self.report(VerifierErrorKind::ConstantOutOfRange(ty));
            }
            if !seen.insert(bits) {
                self.report(VerifierErrorKind::DuplicateSwitchCase(bits));
            }
        }
        self.edges(&dests);
    }
}

/// Whether a conversion from `from` to `to` is one `opcode` performs.
fn conversion_is_legal(opcode: Opcode, from: Type, to: Type) -> bool {
    match opcode {
        | Opcode::Zext | Opcode::Sext => from.is_int() && to.is_int() && from.bits() < to.bits(),
        | Opcode::Trunc => from.is_int() && to.is_int() && from.bits() > to.bits(),
        | Opcode::Fpext => from.is_float() && to.is_float() && from.bits() < to.bits(),
        | Opcode::Fptrunc => from.is_float() && to.is_float() && from.bits() > to.bits(),
        | Opcode::Fptosi | Opcode::Fptoui => from.is_float() && to.is_int(),
        | Opcode::Sitofp | Opcode::Uitofp => from.is_int() && to.is_float(),
        | Opcode::Ptrtoint => from == Type::Ptr && to.is_int(),
        | Opcode::Inttoptr => from.is_int() && to == Type::Ptr,
        | Opcode::Bitcast => from.bits() == to.bits() && (from == Type::Ptr) == (to == Type::Ptr),
        | _ => false,
    }
}

/// Checks a global's initializer against its size, and its relocations.
fn verify_global<'m>(
    module: &'m Module<'_>,
    global: GlobalId,
    errors: &mut ArenaVec<'_, VerifierError<'m>>,
) {
    let data = module.global(global);
    let mut error = |kind| {
        errors.push(VerifierError {
            item: data.name,
            location: Location::Item,
            kind,
        });
    };
    let Some(GlobalInit::Bytes { bytes, relocations }) = data.init else {
        return;
    };
    if bytes.len() as u64 != data.size {
        error(VerifierErrorKind::InitializerSize {
            expected: data.size,
            found:    bytes.len() as u64,
        });
    }
    for relocation in relocations {
        if relocation
            .offset
            .checked_add(8)
            .is_none_or(|end| end > data.size)
        {
            error(VerifierErrorKind::RelocationRange(relocation.offset));
        }
        let (kind, index, exists) = match relocation.symbol {
            | Symbol::Function(func) => (
                EntityKind::Function,
                func.as_u32(),
                module.get_function(func).is_some(),
            ),
            | Symbol::Global(target) => (
                EntityKind::Global,
                target.as_u32(),
                module.get_global(target).is_some(),
            ),
        };
        if !exists {
            error(VerifierErrorKind::InvalidReference(kind, index));
        }
    }
}

/// One problem the verifier found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct VerifierError<'m> {
    /// The function or global it is in.
    pub(crate) item:     &'m str,
    pub(crate) location: Location,
    pub(crate) kind:     VerifierErrorKind,
}

/// Where in a function or global a problem is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Location {
    /// The function or global as a whole.
    Item,
    Block(Block),
    Inst(Inst),
}

/// What the verifier found wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VerifierErrorKind {
    /// A defined function has no blocks.
    NoBlocks,
    EmptyBlock,
    /// A block's last instruction is not a terminator.
    MissingTerminator,
    /// A terminator is followed by more instructions.
    TerminatorNotLast,
    /// An instruction appears more than once in the block lists.
    InstructionReused,
    /// An entity index is out of range.
    InvalidReference(EntityKind, u32),
    /// The value table and the instruction or block that defines a value
    /// disagree.
    ValueDefinition(Value),
    EntryParameterCount {
        expected: usize,
        found:    usize,
    },
    EntryParameterType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// The entry block has no predecessors; its parameters are the
    /// function's.
    BranchToEntry,
    /// The controlling type is not one the opcode accepts.
    InstructionType {
        opcode: Opcode,
        ty:     Type,
    },
    OperandType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// The instruction's result is missing, extra or of the wrong type.
    ResultType {
        expected: Option<Type>,
        found:    Option<Type>,
    },
    IllegalFlags {
        opcode: Opcode,
        flags:  InstFlags,
    },
    Conversion {
        opcode: Opcode,
        from:   Type,
        to:     Type,
    },
    /// Constant bits do not fit the type.
    ConstantOutOfRange(Type),
    EdgeArgumentCount {
        target:   Block,
        expected: usize,
        found:    usize,
    },
    EdgeArgumentType {
        target:   Block,
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// Two edges of one terminator reach a block with different arguments.
    DuplicateEdgeArguments(Block),
    DuplicateSwitchCase(u128),
    CallArgumentCount {
        expected: usize,
        found:    usize,
        variadic: bool,
    },
    CallArgumentType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// A `call_indirect` without a callee operand.
    MissingCallee,
    ReturnValue {
        expected: Option<Type>,
        found:    Option<Type>,
    },
    /// A use is not dominated by the value's definition.
    NotDominated(Value),
    NotAllowedInProfile {
        opcode:  Opcode,
        profile: Profile,
    },
    /// A global's bytes are not as long as the global.
    InitializerSize {
        expected: u64,
        found:    u64,
    },
    /// A relocation's eight bytes do not fit in the global.
    RelocationRange(u64),
}

/// The kind of entity an [`VerifierErrorKind::InvalidReference`] names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntityKind {
    Value,
    Block,
    Inst,
    StackSlot,
    Constant,
    Function,
    Global,
    Signature,
}

impl fmt::Display for VerifierError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.item)?;
        match self.location {
            | Location::Item => {},
            | Location::Block(block) => write!(f, ", {block}")?,
            | Location::Inst(inst) => write!(f, ", {inst}")?,
        }
        write!(f, ": {}", self.kind)
    }
}

impl fmt::Display for VerifierErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let optional = |ty: Option<Type>| ty.map_or("nothing", Type::name);
        match *self {
            | Self::NoBlocks => f.write_str("a defined function has no blocks"),
            | Self::EmptyBlock => f.write_str("the block is empty"),
            | Self::MissingTerminator => f.write_str("the block does not end in a terminator"),
            | Self::TerminatorNotLast => f.write_str("a terminator is followed by instructions"),
            | Self::InstructionReused => f.write_str("the instruction appears more than once"),
            | Self::InvalidReference(kind, index) => write!(f, "{kind} {index} does not exist"),
            | Self::ValueDefinition(value) =>
                write!(f, "{value} is not defined where the value table says"),
            | Self::EntryParameterCount { expected, found } => write!(
                f,
                "the entry block has {found} parameters but the signature has {expected}"
            ),
            | Self::EntryParameterType {
                index,
                expected,
                found,
            } => write!(
                f,
                "entry parameter {index} is {found}, but the signature says {expected}"
            ),
            | Self::BranchToEntry => f.write_str("a branch targets the entry block"),
            | Self::InstructionType { opcode, ty } => write!(f, "{opcode} cannot have type {ty}"),
            | Self::OperandType {
                index,
                expected,
                found,
            } => write!(f, "operand {index} is {found}, expected {expected}"),
            | Self::ResultType { expected, found } => write!(
                f,
                "the result is {}, expected {}",
                optional(found),
                optional(expected)
            ),
            | Self::IllegalFlags { opcode, flags } => write!(f, "{opcode} cannot carry `{flags}`"),
            | Self::Conversion { opcode, from, to } =>
                write!(f, "{opcode} cannot convert {from} to {to}"),
            | Self::ConstantOutOfRange(ty) => write!(f, "a constant does not fit in {ty}"),
            | Self::EdgeArgumentCount {
                target,
                expected,
                found,
            } => write!(
                f,
                "the edge to {target} passes {found} arguments for {expected} parameters"
            ),
            | Self::EdgeArgumentType {
                target,
                index,
                expected,
                found,
            } => write!(
                f,
                "argument {index} to {target} is {found}, expected {expected}"
            ),
            | Self::DuplicateEdgeArguments(target) =>
                write!(f, "edges to {target} pass different arguments"),
            | Self::DuplicateSwitchCase(bits) => write!(f, "the case {bits} appears twice"),
            | Self::CallArgumentCount {
                expected,
                found,
                variadic,
            } => write!(
                f,
                "the call passes {found} arguments for {}{expected} parameters",
                if variadic { "at least " } else { "" }
            ),
            | Self::CallArgumentType {
                index,
                expected,
                found,
            } => write!(f, "call argument {index} is {found}, expected {expected}"),
            | Self::MissingCallee => f.write_str("call_indirect has no callee"),
            | Self::ReturnValue { expected, found } => write!(
                f,
                "the function returns {}, but the return gives {}",
                optional(expected),
                optional(found)
            ),
            | Self::NotDominated(value) =>
                write!(f, "the use of {value} is not dominated by its definition"),
            | Self::NotAllowedInProfile { opcode, profile } =>
                write!(f, "{opcode} is not allowed in the {profile:?} profile"),
            | Self::InitializerSize { expected, found } => write!(
                f,
                "the initializer has {found} bytes for a size of {expected}"
            ),
            | Self::RelocationRange(offset) => write!(
                f,
                "the relocation at offset {offset} does not fit in the global"
            ),
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            | Self::Value => "value",
            | Self::Block => "block",
            | Self::Inst => "instruction",
            | Self::StackSlot => "stack slot",
            | Self::Constant => "constant",
            | Self::Function => "function",
            | Self::Global => "global",
            | Self::Signature => "signature",
        })
    }
}
