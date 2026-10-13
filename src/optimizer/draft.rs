//! The editable form of one function that the passes rewrite.
//!
//! The IR keeps no use lists and stores each block's instructions
//! contiguously, so a pass cannot splice a body in place. Instead the
//! optimizer *lifts* a body into a [`Draft`]: per block, its live parameters,
//! its instructions as ids into the original body, and its terminator with
//! owned edges (a target and the arguments it passes). Passes edit the draft
//! freely:
//!
//! - **replacing a value** records `from -> to` in a replacement map, applied
//!   to every operand when it is read or when the draft is lowered;
//! - **folding to a constant** records the constant for an instruction, which
//!   keeps its result value;
//! - **removing an instruction** marks it, and `sweep` drops the marked ones;
//! - **editing control flow** changes terminators, block parameters and `alive`
//!   flags directly;
//! - **moving an instruction** takes its id out of one block's list and puts it
//!   into another's;
//! - **adding a block** appends one, numbered after the body's blocks;
//! - **adding instructions** inserts any non-terminator at a chosen position,
//!   with ids numbered after the body and optional results; inline operands
//!   stay in the record and new call operand lists are arena-owned;
//! - **adding values** appends block parameters or instruction results;
//! - **rewriting instructions** keeps result identity and clears old folds;
//! - **weakening flags** records new flags for an instruction, which
//!   [`Draft::inst`] then reports;
//! - **adding stack slots and constant bits** copies foreign frame objects and
//!   wide constants without retaining foreign pool handles;
//! - **removing a stack slot** drops it from the lowered body, whose remaining
//!   slots are renumbered.
//!
//! The original body is never mutated, so ids into it and the pool handles of
//! instructions that were not touched stay valid for the whole run. [`lower`]
//! writes a fresh body with blocks, values and instructions renumbered
//! densely in layout order, which is the numbering the textual form uses.
//! The invariants a pass keeps are those of verified IR: each edge passes as
//! many arguments as its target has parameters, and two edges of one
//! terminator to one block pass the same arguments.
//!
//! [`lower`]: Draft::lower

mod analysis;
mod lower;

pub(super) use analysis::Predecessors;

use crate::{
    ir::{
        Block,
        Body,
        Entity,
        Inst,
        InstData,
        InstFlags,
        Opcode,
        StackSlot,
        StackSlotData,
        Type,
        Value,
        ValueData,
        ValueDef,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A function being optimized; see the module docs.
pub(super) struct Draft<'b, 's> {
    pub(super) body:    &'b Body<'b>,
    pub(super) scratch: &'s Bump,
    /// One entry per block of the original body, by block index. Block 0 is
    /// the entry and is never removed.
    pub(super) blocks:  ArenaVec<'s, DraftBlock<'s>>,
    /// Where each original value's uses go; a value maps to itself unless it
    /// was replaced.
    replacements:       ArenaVec<'s, Value>,
    /// The constant an instruction was folded to, by instruction index.
    folded:             ArenaVec<'s, Option<Constant>>,
    /// Whether an instruction was removed, by instruction index.
    removed:            ArenaVec<'s, bool>,
    /// Rewritten records and records of added instructions, by instruction id.
    edited:             ArenaVec<'s, Option<InstData>>,
    /// The values the passes added; value `body.value_count() + i` is entry
    /// `i`.
    added_values:       ArenaVec<'s, ValueData>,
    /// Optional results of added instructions, numbered after the body's.
    added_insts:        ArenaVec<'s, Option<Value>>,
    /// Owned call operands for inserted or rewritten calls. Other operands
    /// fit inline in `InstData`, including all three select operands.
    call_args:          ArenaVec<'s, Option<&'s [Value]>>,
    /// Whether a stack slot was removed, by slot index.
    removed_slots:      ArenaVec<'s, bool>,
    /// Slots copied from an inlined body, after the original slots.
    added_slots:        ArenaVec<'s, StackSlotData>,
    /// Owned bits for copied wide constants; foreign pool handles are never
    /// read.
    constant_bits:      ArenaVec<'s, Option<u128>>,
}

/// A block of a draft.
#[derive(Clone, Debug)]
pub(super) struct DraftBlock<'s> {
    /// Whether the block is still part of the function.
    pub(super) alive:  bool,
    /// The block's parameters that remain.
    pub(super) params: ArenaVec<'s, Value>,
    /// The instructions before the terminator, in program order.
    pub(super) insts:  ArenaVec<'s, Inst>,
    pub(super) term:   Term<'s>,
}

/// A terminator with its edges.
#[derive(Clone, Debug)]
pub(super) enum Term<'s> {
    Jump(Edge<'s>),
    Brif {
        cond:      Value,
        then:      Edge<'s>,
        otherwise: Edge<'s>,
    },
    Switch {
        value:   Value,
        default: Edge<'s>,
        /// Each case's constant bits and edge, in order.
        cases:   ArenaVec<'s, (u128, Edge<'s>)>,
    },
    Return(Option<Value>),
    Unreachable,
}

/// A branch edge: a target and the arguments for its parameters.
#[derive(Clone, Debug)]
pub(super) struct Edge<'s> {
    pub(super) target: Block,
    pub(super) args:   ArenaVec<'s, Value>,
}

/// A value known at compile time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    variant_size_differences,
    reason = "A constant is a pair of words at most; boxing the bits would cost more."
)]
pub(super) enum Constant {
    /// An integer of the type; the bits are masked to its width.
    Int(Type, u128),
    /// The poison of the type.
    Poison(Type),
}

impl<'b, 's> Draft<'b, 's> {
    /// Lifts a verified body. Panics if a block has no terminator.
    pub(super) fn lift(body: &'b Body<'b>, scratch: &'s Bump) -> Self {
        let mut blocks = ArenaVec::with_capacity_in(body.block_count(), scratch);
        for block in body.blocks() {
            let terminator = body
                .terminator(block)
                .expect("the optimizer takes verified IR, whose blocks end in terminators");
            let insts = body.block_insts(block);
            let mut params = ArenaVec::with_capacity_in(body.block_params(block).len(), scratch);
            params.extend_from_slice(body.block_params(block));
            let mut kept = ArenaVec::with_capacity_in(insts.len() - 1, scratch);
            kept.extend_from_slice(&insts[..insts.len() - 1]);
            blocks.push(DraftBlock {
                alive: true,
                params,
                insts: kept,
                term: lift_terminator(body, terminator, scratch),
            });
        }
        let mut replacements = ArenaVec::with_capacity_in(body.value_count(), scratch);
        replacements.extend((0..body.value_count()).map(Value::new));
        let mut folded = ArenaVec::with_capacity_in(body.inst_count(), scratch);
        folded.resize(body.inst_count(), None);
        let mut removed = ArenaVec::with_capacity_in(body.inst_count(), scratch);
        removed.resize(body.inst_count(), false);
        let mut edited = ArenaVec::with_capacity_in(body.inst_count(), scratch);
        edited.resize(body.inst_count(), None);
        let mut call_args = ArenaVec::with_capacity_in(body.inst_count(), scratch);
        call_args.resize(body.inst_count(), None);
        let mut removed_slots = ArenaVec::with_capacity_in(body.stack_slot_count(), scratch);
        removed_slots.resize(body.stack_slot_count(), false);
        Self {
            body,
            scratch,
            blocks,
            replacements,
            folded,
            removed,
            edited,
            added_values: ArenaVec::new_in(scratch),
            added_insts: ArenaVec::new_in(scratch),
            removed_slots,
            call_args,
            added_slots: ArenaVec::new_in(scratch),
            constant_bits: {
                let mut bits = ArenaVec::new_in(scratch);
                bits.resize(body.inst_count(), None);
                bits
            },
        }
    }

    // Values

    /// The value that uses of `value` now refer to.
    pub(super) fn resolve(&self, mut value: Value) -> Value {
        // Replacements may point at new values; the bound catches cycles.
        for _ in 0..=self.replacements.len() {
            let next = self.replacements[value.index()];
            if next == value {
                return value;
            }
            value = next;
        }
        panic!("the replacements of value {value} form a cycle")
    }

    /// Redirects every use of `from` to `to`, either of which may be an added
    /// value. `to` must dominate the uses of `from`.
    pub(super) fn replace(&mut self, from: Value, to: Value) {
        let from = self.resolve(from);
        let to = self.resolve(to);
        if from != to {
            self.replacements[from.index()] = to;
        }
    }

    /// How many values the draft has: the body's, then the added ones.
    pub(super) fn value_count(&self) -> usize {
        self.body.value_count() + self.added_values.len()
    }

    pub(super) fn value_type(&self, value: Value) -> Type {
        self.value_data(value).ty
    }

    /// Where a value is defined. An added parameter records the position it
    /// was added at, which later edits to the block's parameters may change.
    pub(super) fn value_def(&self, value: Value) -> ValueDef {
        self.value_data(value).def
    }

    fn value_data(&self, value: Value) -> ValueData {
        match value.index().checked_sub(self.body.value_count()) {
            | Some(added) => self.added_values[added],
            | None => ValueData {
                ty:  self.body.value_type(value),
                def: self.body.value_def(value),
            },
        }
    }

    /// Appends a parameter of type `ty` to `block` and returns it. The pass
    /// must append a matching argument to every edge into the block.
    pub(super) fn add_block_param(&mut self, block: Block, ty: Type) -> Value {
        let index = u32::try_from(self.block(block).params.len())
            .expect("a block has fewer than 2^32 parameters");
        let value = self.add_value(ValueData {
            ty,
            def: ValueDef::Param(block, index),
        });
        self.block_mut(block).params.push(value);
        value
    }

    /// Adds an instruction defining `constant` to the entry block, after the
    /// constants added before it and ahead of the original instructions, and
    /// returns its result, which dominates every block.
    pub(super) fn add_constant(&mut self, constant: Constant) -> Value {
        let ty = match constant {
            | Constant::Int(ty, _) | Constant::Poison(ty) => ty,
        };
        let (inst, result) = self.new_inst(
            InstData::Nullary {
                opcode: Opcode::Poison,
                ty,
            },
            Some(ty),
        );
        let value = result.expect("a constant defines a value");
        self.folded[inst.index()] = Some(constant);
        let original = self.body.inst_count();
        let entry = &self.blocks[Self::entry().index()].insts;
        let position = entry
            .iter()
            .position(|inst| inst.index() < original || self.folded[inst.index()].is_none())
            .unwrap_or(entry.len());
        self.blocks[Self::entry().index()]
            .insts
            .insert(position, inst);
        value
    }

    fn add_value(&mut self, data: ValueData) -> Value {
        let value = Value::new(self.value_count());
        self.added_values.push(data);
        self.replacements.push(value);
        value
    }

    /// The constant a value is, if it is the result of an integer constant or
    /// of an instruction folded to one, or poison. Floats are not tracked.
    pub(super) fn constant(&self, value: Value) -> Option<Constant> {
        match self.value_def(self.resolve(value)) {
            | ValueDef::Result(inst) => self.inst_constant(inst),
            | ValueDef::Param(..) => None,
        }
    }

    /// The constant an instruction defines, as written or as folded.
    pub(super) fn inst_constant(&self, inst: Inst) -> Option<Constant> {
        if let Some(constant) = self.folded[inst.index()] {
            return Some(constant);
        }
        match *self.inst(inst) {
            | InstData::Const {
                opcode: Opcode::Iconst,
                ty,
                ..
            }
            | InstData::WideConst {
                opcode: Opcode::Iconst,
                ty,
                ..
            } => self
                .const_bits(inst)
                .map(|bits| Constant::Int(ty, bits & ty.mask())),
            | InstData::Nullary {
                opcode: Opcode::Poison,
                ty,
            } => Some(Constant::Poison(ty)),
            | _ => None,
        }
    }

    /// Makes `inst`, which must define a value of the constant's type, define
    /// the constant instead. The result value stays.
    pub(super) fn set_constant(&mut self, inst: Inst, constant: Constant) {
        self.folded[inst.index()] = Some(constant);
    }

    // Instructions

    /// The current record, including general rewrites and added instructions.
    /// For a folded instruction use [`Draft::inst_constant`] first.
    pub(super) fn inst(&self, inst: Inst) -> &InstData {
        self.edited[inst.index()]
            .as_ref()
            .unwrap_or_else(|| self.body.inst(inst))
    }

    /// Inserts a general non-terminator at `position`. Calls need their
    /// signature's result type; pass `None` for instructions without a result.
    /// Pool handles in `data` refer to the original body. Use `set_call_args`
    /// for new call operand lists, and `add_constant` for new wide integers.
    pub(super) fn add_inst(
        &mut self,
        block: Block,
        position: usize,
        data: InstData,
        result_type: Option<Type>,
    ) -> Option<Value> {
        let (inst, result) = self.new_inst(data, result_type);
        self.block_mut(block).insts.insert(position, inst);
        result
    }

    /// Rewrites a non-terminator while preserving its result and type.
    /// Clears a previous fold and any owned call operand list.
    pub(super) fn replace_inst(&mut self, inst: Inst, data: InstData) {
        assert!(!data.is_terminator(), "terminators belong in Term");
        self.edited[inst.index()] = Some(data);
        self.folded[inst.index()] = None;
        self.call_args[inst.index()] = None;
        self.constant_bits[inst.index()] = None;
    }

    /// Moves an instruction without changing its result identity. Its operands
    /// must dominate the new position, which must dominate all its uses.
    pub(super) fn move_inst(&mut self, inst: Inst, from: Block, to: Block) {
        let position = self
            .block(from)
            .insts
            .iter()
            .position(|&id| id == inst)
            .expect("the source block contains the instruction");
        _ = self.block_mut(from).insts.remove(position);
        self.block_mut(to).insts.push(inst);
    }

    /// Copies a call's variable-length operands into the draft's arena.
    pub(super) fn set_call_args(&mut self, inst: Inst, args: &[Value]) {
        assert!(
            matches!(
                self.inst(inst),
                InstData::Call { .. } | InstData::CallIndirect { .. }
            ),
            "owned argument lists belong only to calls"
        );
        self.call_args[inst.index()] = Some(self.scratch.alloc_slice_copy(args));
    }

    pub(super) fn call_operands(&self, inst: Inst, list: crate::ir::ValueList) -> &[Value] {
        self.call_args[inst.index()].unwrap_or_else(|| self.body.value_list(list))
    }

    fn new_inst(&mut self, data: InstData, result_type: Option<Type>) -> (Inst, Option<Value>) {
        assert!(!data.is_terminator(), "terminators belong in Term");
        let inst = Inst::new(self.inst_count());
        let result = result_type.map(|ty| {
            self.add_value(ValueData {
                ty,
                def: ValueDef::Result(inst),
            })
        });
        self.added_insts.push(result);
        self.folded.push(None);
        self.removed.push(false);
        self.edited.push(Some(data));
        self.call_args.push(None);
        self.constant_bits.push(None);
        (inst, result)
    }

    /// Owns a foreign constant's bits independently of its pool handle.
    pub(super) fn set_const_bits(&mut self, inst: Inst, bits: u128) {
        self.constant_bits[inst.index()] = Some(bits);
    }

    pub(super) fn const_bits(&self, inst: Inst) -> Option<u128> {
        self.constant_bits[inst.index()].or_else(|| self.body.const_bits(self.inst(inst)))
    }

    /// Adds an independent frame object for one inlined call site.
    pub(super) fn add_stack_slot(&mut self, data: StackSlotData) -> StackSlot {
        let slot = StackSlot::new(self.stack_slot_count());
        self.added_slots.push(data);
        self.removed_slots.push(false);
        slot
    }

    pub(super) fn stack_slot_count(&self) -> usize {
        self.body.stack_slot_count() + self.added_slots.len()
    }

    pub(super) fn stack_slot(&self, slot: StackSlot) -> StackSlotData {
        match slot.index().checked_sub(self.body.stack_slot_count()) {
            | Some(index) => self.added_slots[index],
            | None => self.body.stack_slot(slot),
        }
    }

    /// Gives `inst`, a `Binary` instruction, the flags `flags`. Passes only
    /// weaken flags, which makes the result poison in fewer cases.
    pub(super) fn set_flags(&mut self, inst: Inst, flags: InstFlags) {
        let InstData::Binary {
            opcode, ty, args, ..
        } = *self.inst(inst)
        else {
            panic!("only a binary instruction has flags a pass may change");
        };
        self.edited[inst.index()] = Some(InstData::Binary {
            opcode,
            ty,
            flags,
            args,
        });
    }

    pub(super) fn inst_result(&self, inst: Inst) -> Option<Value> {
        match inst.index().checked_sub(self.body.inst_count()) {
            | Some(added) => self.added_insts[added],
            | None => self.body.inst_result(inst),
        }
    }

    /// How many instructions the draft has: the body's, then the added ones.
    pub(super) fn inst_count(&self) -> usize {
        self.body.inst_count() + self.added_insts.len()
    }

    /// Calls `visit` with each value the instruction uses, after
    /// replacement. A folded instruction, an added one included, uses
    /// nothing.
    pub(super) fn for_each_operand(&self, inst: Inst, mut visit: impl FnMut(Value)) {
        if self.folded[inst.index()].is_none() {
            let mut operand = |value| visit(self.resolve(value));
            match *self.inst(inst) {
                | InstData::Binary { args, .. }
                | InstData::IntCompare { args, .. }
                | InstData::FloatCompare { args, .. }
                | InstData::Store { args, .. } => args.into_iter().for_each(operand),
                | InstData::Select { args, .. } | InstData::MemoryRange { args, .. } =>
                    args.into_iter().for_each(operand),
                | InstData::Unary { arg, .. }
                | InstData::VaArgAggregate { list: arg, .. }
                | InstData::DynAlloca { size: arg, .. } => operand(arg),
                | InstData::Load { addr, .. } => operand(addr),
                | InstData::Call { args, .. } | InstData::CallIndirect { args, .. } => self
                    .call_operands(inst, args)
                    .iter()
                    .copied()
                    .for_each(operand),
                | _ => {},
            }
        }
    }

    /// Marks an instruction for removal by [`Draft::sweep`]. The pass must
    /// have redirected or dropped every use of its result.
    pub(super) fn remove_inst(&mut self, inst: Inst) {
        self.removed[inst.index()] = true;
    }

    pub(super) fn is_removed(&self, inst: Inst) -> bool {
        self.removed[inst.index()]
    }

    /// Drops the instructions marked for removal from their blocks.
    pub(super) fn sweep(&mut self) {
        let removed = &self.removed;
        for block in &mut self.blocks {
            block.insts.retain(|inst| !removed[inst.index()]);
        }
    }

    // Stack slots

    /// Drops a stack slot from the function. The pass must have removed every
    /// `stack_addr` of it.
    pub(super) fn remove_stack_slot(&mut self, slot: StackSlot) {
        self.removed_slots[slot.index()] = true;
    }

    pub(super) fn is_stack_slot_removed(&self, slot: StackSlot) -> bool {
        self.removed_slots[slot.index()]
    }

    // Blocks

    pub(super) fn block(&self, block: Block) -> &DraftBlock<'s> {
        &self.blocks[block.index()]
    }

    pub(super) fn block_mut(&mut self, block: Block) -> &mut DraftBlock<'s> {
        &mut self.blocks[block.index()]
    }

    pub(super) fn entry() -> Block {
        Block::new(0)
    }

    /// Appends an empty block that ends in `unreachable`; lowering lays it
    /// out after the body's blocks. The pass gives it a terminator and the
    /// edges into it.
    pub(super) fn add_block(&mut self) -> Block {
        let block = Block::new(self.blocks.len());
        self.blocks.push(DraftBlock {
            alive:  true,
            params: ArenaVec::new_in(self.scratch),
            insts:  ArenaVec::new_in(self.scratch),
            term:   Term::Unreachable,
        });
        block
    }

    /// The blocks still part of the function, in layout order.
    pub(super) fn alive_blocks(&self) -> impl Iterator<Item = Block> {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.alive)
            .map(|(index, _)| Block::new(index))
    }

    /// Whether two edges go to one block with the same arguments, after
    /// replacement.
    pub(super) fn same_edge(&self, a: &Edge<'_>, b: &Edge<'_>) -> bool {
        a.target == b.target
            && a.args.len() == b.args.len()
            && a.args
                .iter()
                .zip(&b.args)
                .all(|(&x, &y)| self.resolve(x) == self.resolve(y))
    }
}

impl<'s> Term<'s> {
    /// The terminator's edges: a `brif`'s in order, a `switch`'s default and
    /// then its cases.
    pub(super) fn edges(&self) -> impl Iterator<Item = &Edge<'s>> {
        let (first, second, rest): (Option<&Edge<'s>>, Option<&Edge<'s>>, &[(u128, Edge<'s>)]) =
            match self {
                | Self::Jump(edge) => (Some(edge), None, &[]),
                | Self::Brif {
                    then, otherwise, ..
                } => (Some(then), Some(otherwise), &[]),
                | Self::Switch { default, cases, .. } => (Some(default), None, cases),
                | Self::Return(_) | Self::Unreachable => (None, None, &[]),
            };
        first
            .into_iter()
            .chain(second)
            .chain(rest.iter().map(|(_, edge)| edge))
    }

    /// Like [`Term::edges`], mutably.
    pub(super) fn edges_mut(&mut self) -> impl Iterator<Item = &mut Edge<'s>> {
        type Cases<'a, 's> = &'a mut [(u128, Edge<'s>)];
        let (first, second, rest): (Option<&mut Edge<'s>>, Option<&mut Edge<'s>>, Cases<'_, 's>) =
            match self {
                | Self::Jump(edge) => (Some(edge), None, &mut []),
                | Self::Brif {
                    then, otherwise, ..
                } => (Some(then), Some(otherwise), &mut []),
                | Self::Switch { default, cases, .. } => (Some(default), None, cases),
                | Self::Return(_) | Self::Unreachable => (None, None, &mut []),
            };
        first
            .into_iter()
            .chain(second)
            .chain(rest.iter_mut().map(|(_, edge)| edge))
    }

    /// Calls `visit` with the values the terminator uses that are not edge
    /// arguments: a branch condition, a switch value or a returned value.
    pub(super) fn for_each_control_operand(&self, mut visit: impl FnMut(Value)) {
        match *self {
            | Self::Brif { cond: value, .. }
            | Self::Switch { value, .. }
            | Self::Return(Some(value)) => visit(value),
            | Self::Jump(_) | Self::Return(None) | Self::Unreachable => {},
        }
    }
}

fn lift_edge<'s>(body: &Body<'_>, call: crate::ir::BlockCall, scratch: &'s Bump) -> Edge<'s> {
    let (target, args) = body.block_call(call);
    let mut copied = ArenaVec::with_capacity_in(args.len(), scratch);
    copied.extend_from_slice(args);
    Edge {
        target,
        args: copied,
    }
}

fn lift_terminator<'s>(body: &Body<'_>, terminator: Inst, scratch: &'s Bump) -> Term<'s> {
    match *body.inst(terminator) {
        | InstData::Jump { dest } => Term::Jump(lift_edge(body, dest, scratch)),
        | InstData::Brif {
            cond,
            dests: [then, otherwise],
        } => Term::Brif {
            cond,
            then: lift_edge(body, then, scratch),
            otherwise: lift_edge(body, otherwise, scratch),
        },
        | InstData::Switch {
            value,
            default,
            cases,
        } => {
            let mut lifted = ArenaVec::new_in(scratch);
            for (bits, dest) in body.switch_cases(cases) {
                lifted.push((bits, lift_edge(body, dest, scratch)));
            }
            Term::Switch {
                value,
                default: lift_edge(body, default, scratch),
                cases: lifted,
            }
        },
        | InstData::Return { value } => Term::Return(value),
        | InstData::Unreachable => Term::Unreachable,
        | _ => unreachable!("a block's last instruction is a terminator"),
    }
}
