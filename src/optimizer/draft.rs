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
//! - **adding a block or a block parameter** appends to the draft: added blocks
//!   are numbered after the body's, and so are added values;
//! - **weakening flags** records new flags for an instruction, which
//!   [`Draft::inst`] then reports.
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
        Type,
        Value,
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
    /// An instruction's record with flags a pass changed, by instruction
    /// index.
    edited:             ArenaVec<'s, Option<InstData>>,
    /// The type and definition of each value a pass added; value
    /// `body.value_count() + i` is entry `i`.
    added_values:       ArenaVec<'s, (Type, ValueDef)>,
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
        Self {
            body,
            scratch,
            blocks,
            replacements,
            folded,
            removed,
            edited,
            added_values: ArenaVec::new_in(scratch),
        }
    }

    // Values

    /// The value that uses of `value` now refer to.
    pub(super) fn resolve(&self, mut value: Value) -> Value {
        // Replacements point at values that existed first, so chains end; the
        // bound turns a pass bug into a panic instead of a hang.
        for _ in 0..=self.replacements.len() {
            let next = self.replacements[value.index()];
            if next == value {
                return value;
            }
            value = next;
        }
        panic!("the replacements of value {value} form a cycle")
    }

    /// Redirects every use of `from` to `to`. `to` must dominate the uses of
    /// `from`.
    pub(super) fn replace(&mut self, from: Value, to: Value) {
        let from = self.resolve(from);
        let to = self.resolve(to);
        if from != to {
            self.replacements[from.index()] = to;
        }
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
        match *self.body.inst(inst) {
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
                .body
                .const_bits(self.body.inst(inst))
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

    /// The number of values: the body's, then those passes added.
    pub(super) fn value_count(&self) -> usize {
        self.replacements.len()
    }

    pub(super) fn value_type(&self, value: Value) -> Type {
        match value.index().checked_sub(self.body.value_count()) {
            | Some(added) => self.added_values[added].0,
            | None => self.body.value_type(value),
        }
    }

    /// Where a value is defined. An added parameter keeps the block and
    /// position it was added with.
    pub(super) fn value_def(&self, value: Value) -> ValueDef {
        match value.index().checked_sub(self.body.value_count()) {
            | Some(added) => self.added_values[added].1,
            | None => self.body.value_def(value),
        }
    }

    /// Appends a parameter of type `ty` to `block` and returns it. The pass
    /// must add a matching argument to every edge into the block.
    pub(super) fn add_param(&mut self, block: Block, ty: Type) -> Value {
        let value = Value::new(self.replacements.len());
        let position = u32::try_from(self.block(block).params.len()).expect("too many parameters");
        self.replacements.push(value);
        self.added_values
            .push((ty, ValueDef::Param(block, position)));
        self.block_mut(block).params.push(value);
        value
    }

    // Instructions

    /// The instruction's current data: its original record, with the flags
    /// a pass set. For a folded instruction it is still that record; use
    /// [`Draft::inst_constant`] first.
    pub(super) fn inst(&self, inst: Inst) -> &InstData {
        self.edited[inst.index()]
            .as_ref()
            .unwrap_or_else(|| self.body.inst(inst))
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
        self.body.inst_result(inst)
    }

    /// Calls `visit` with each value the instruction uses, after
    /// replacement. A folded instruction uses nothing.
    pub(super) fn for_each_operand(&self, inst: Inst, mut visit: impl FnMut(Value)) {
        if self.folded[inst.index()].is_none() {
            self.body
                .visit_operands(inst, |value| visit(self.resolve(value)));
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
