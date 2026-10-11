//! A function body: its blocks, instructions, values, stack slots and pools,
//! and the queries passes make over them.
//!
//! The pools hold what does not fit in a 16-byte instruction record. The
//! value pool stores each list with its length first: a [`ValueList`] is
//! `[len, values...]`, a [`BlockCall`] is `[len, block, args...]`, and a
//! [`CaseList`] is `[len, (constant, block call)...]`. Lengths, blocks and
//! constants are stored as raw indices in the pool's `Value` slots.

use super::{
    entities::{
        Block,
        ConstId,
        Entity,
        Inst,
        StackSlot,
        Table,
        Value,
    },
    instructions::{
        Align,
        BlockCall,
        CaseList,
        InstData,
        MemFlags,
        ValueList,
    },
    types::Type,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The definition of a function: blocks in layout order, each holding its
/// parameters and its instructions in program order.
#[derive(Debug)]
pub(crate) struct Body<'ir> {
    pub(super) blocks:      Table<'ir, Block, BlockData<'ir>>,
    pub(super) insts:       Table<'ir, Inst, InstData>,
    /// The result of each instruction, beside its record.
    pub(super) results:     Table<'ir, Inst, Option<Value>>,
    pub(super) values:      Table<'ir, Value, ValueData>,
    pub(super) stack_slots: Table<'ir, StackSlot, StackSlotData>,
    pub(super) pool:        ArenaVec<'ir, Value>,
    pub(super) constants:   Table<'ir, ConstId, u128>,
}

impl<'ir> Body<'ir> {
    /// The entry block, block 0, if the body has any block.
    pub(crate) fn entry_block(&self) -> Option<Block> {
        (!self.blocks.is_empty()).then(|| Block::new(0))
    }

    /// Every block, in layout order.
    pub(crate) fn blocks(&self) -> impl DoubleEndedIterator<Item = Block> + use<'ir> {
        self.blocks.keys()
    }

    pub(crate) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub(crate) fn block_params(&self, block: Block) -> &[Value] {
        &self.blocks[block].params
    }

    /// The block's instructions in program order.
    pub(crate) fn block_insts(&self, block: Block) -> &[Inst] {
        &self.blocks[block].insts
    }

    /// The block's last instruction, if it is a terminator.
    pub(crate) fn terminator(&self, block: Block) -> Option<Inst> {
        let last = *self.blocks[block].insts.last()?;
        self.insts[last].is_terminator().then_some(last)
    }

    pub(crate) fn inst(&self, inst: Inst) -> &InstData {
        &self.insts[inst]
    }

    pub(crate) fn inst_result(&self, inst: Inst) -> Option<Value> {
        self.results[inst]
    }

    pub(crate) fn inst_count(&self) -> usize {
        self.insts.len()
    }

    pub(crate) fn value_type(&self, value: Value) -> Type {
        self.values[value].ty
    }

    pub(crate) fn value_def(&self, value: Value) -> ValueDef {
        self.values[value].def
    }

    pub(crate) fn value_count(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn stack_slot(&self, slot: StackSlot) -> StackSlotData {
        self.stack_slots[slot]
    }

    pub(crate) fn stack_slot_count(&self) -> usize {
        self.stack_slots.len()
    }

    pub(crate) fn stack_slots(
        &self,
    ) -> impl DoubleEndedIterator<Item = (StackSlot, StackSlotData)> {
        self.stack_slots.iter().map(|(slot, &data)| (slot, data))
    }

    pub(crate) fn value_list(&self, list: ValueList) -> &[Value] {
        let start = list.0 as usize;
        let len = self.pool[start].index();
        &self.pool[start + 1..start + 1 + len]
    }

    /// The target block of an edge and the arguments it passes.
    pub(crate) fn block_call(&self, call: BlockCall) -> (Block, &[Value]) {
        let start = call.0 as usize;
        let len = self.pool[start].index();
        let block = Block::new(self.pool[start + 1].index());
        (block, &self.pool[start + 2..start + 2 + len])
    }

    /// Each case's constant bits and edge, in order.
    pub(crate) fn switch_cases(
        &self,
        cases: CaseList,
    ) -> impl ExactSizeIterator<Item = (u128, BlockCall)> {
        let start = cases.0 as usize;
        let len = self.pool[start].index();
        self.pool[start + 1..start + 1 + 2 * len]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                (
                    self.constants[ConstId::new(pair[0].index())],
                    BlockCall(pair[1].as_u32()),
                )
            })
    }

    pub(crate) fn constant(&self, constant: ConstId) -> u128 {
        self.constants[constant]
    }

    pub(crate) fn constant_count(&self) -> usize {
        self.constants.len()
    }

    /// The bits of an `iconst` or `fconst`.
    pub(crate) fn const_bits(&self, data: &InstData) -> Option<u128> {
        match *data {
            | InstData::Const { bits, .. } => Some(u128::from(bits.get())),
            | InstData::WideConst { constant, .. } => self.constants.get(constant).copied(),
            | _ => None,
        }
    }

    /// The access facts of a `load` or `store`.
    pub(crate) fn mem_flags(&self, inst: Inst) -> Option<MemFlags> {
        match self.insts[inst] {
            | InstData::Load {
                flags, align, tag, ..
            }
            | InstData::Store {
                flags, align, tag, ..
            } => Some(MemFlags {
                volatile: flags.contains(super::InstFlags::VOLATILE),
                align,
                tag,
            }),
            | _ => None,
        }
    }

    /// Calls `visit` with each value the instruction uses, including the
    /// arguments it passes on its edges, in textual order.
    pub(crate) fn visit_operands(&self, inst: Inst, mut visit: impl FnMut(Value)) {
        let edge = |call: BlockCall, visit: &mut dyn FnMut(Value)| {
            for &value in self.block_call(call).1 {
                visit(value);
            }
        };
        match self.insts[inst] {
            | InstData::Binary { args, .. }
            | InstData::IntCompare { args, .. }
            | InstData::FloatCompare { args, .. }
            | InstData::Store { args, .. } => args.into_iter().for_each(visit),
            | InstData::Select { args, .. } | InstData::MemoryRange { args, .. } =>
                args.into_iter().for_each(visit),
            | InstData::Unary { arg: value, .. } | InstData::Load { addr: value, .. } =>
                visit(value),
            | InstData::Call { args, .. } | InstData::CallIndirect { args, .. } =>
                self.value_list(args).iter().for_each(|&value| visit(value)),
            | InstData::Jump { dest } => edge(dest, &mut visit),
            | InstData::Brif { cond, dests } => {
                visit(cond);
                for dest in dests {
                    edge(dest, &mut visit);
                }
            },
            | InstData::Switch {
                value,
                default,
                cases,
            } => {
                visit(value);
                edge(default, &mut visit);
                for (_, dest) in self.switch_cases(cases) {
                    edge(dest, &mut visit);
                }
            },
            | InstData::Return { value } => value.into_iter().for_each(visit),
            | InstData::Const { .. }
            | InstData::WideConst { .. }
            | InstData::Nullary { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. }
            | InstData::Unreachable => {},
        }
    }

    /// Calls `visit` with each edge of a terminator: the `brif` targets in
    /// order, or a `switch`'s default and then its cases.
    pub(crate) fn visit_destinations(&self, inst: Inst, mut visit: impl FnMut(BlockCall)) {
        match self.insts[inst] {
            | InstData::Jump { dest } => visit(dest),
            | InstData::Brif { dests, .. } => dests.into_iter().for_each(visit),
            | InstData::Switch { default, cases, .. } => {
                visit(default);
                self.switch_cases(cases).for_each(|(_, dest)| visit(dest));
            },
            | _ => {},
        }
    }
}

/// A block's parameters and its instructions in program order.
#[derive(Debug)]
pub(crate) struct BlockData<'ir> {
    pub(super) params: ArenaVec<'ir, Value>,
    pub(super) insts:  ArenaVec<'ir, Inst>,
}

impl<'ir> BlockData<'ir> {
    pub(super) fn new_in(arena: &'ir Bump) -> Self {
        Self {
            params: ArenaVec::new_in(arena),
            insts:  ArenaVec::new_in(arena),
        }
    }
}

/// A value's type and where it is defined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ValueData {
    pub(crate) ty:  Type,
    pub(crate) def: ValueDef,
}

/// Where a value is defined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ValueDef {
    /// The result of an instruction.
    Result(Inst),
    /// The parameter of a block at an index.
    Param(Block, u32),
}

/// A stack slot's size in bytes and its alignment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct StackSlotData {
    pub(crate) size:  u32,
    pub(crate) align: Align,
}

impl<'ir> Body<'ir> {
    /// An empty body.
    pub(crate) fn new_in(arena: &'ir Bump) -> Self {
        Self {
            blocks:      Table::new_in(arena),
            insts:       Table::new_in(arena),
            results:     Table::new_in(arena),
            values:      Table::new_in(arena),
            stack_slots: Table::new_in(arena),
            pool:        ArenaVec::new_in(arena),
            constants:   Table::new_in(arena),
        }
    }
}
