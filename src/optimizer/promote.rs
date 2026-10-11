//! Stack-slot promotion (mem2reg): turns a stack slot that is only loaded and
//! stored as one scalar into SSA values carried by block parameters.
//!
//! A slot is *promotable* when every use of every `stack_addr` of it is the
//! address operand of a non-volatile `load` or `store`, all of one type whose
//! size is the slot's size. Anything else lets the address escape and keeps
//! the slot: passing it to a call or a branch, storing the address itself,
//! offsetting it with `ptr_add`, converting it with `ptrtoint`, or handing it
//! to `copy` or `fill`; so do a volatile access, two access types, and an
//! access narrower than the slot. A slot nothing loads or stores is
//! promotable too, and simply disappears.
//!
//! The pass uses the classic construction of Cytron et al. ("Efficiently
//! computing static single assignment form and the control dependence graph",
//! 1991), pruned by liveness, rather than Braun et al.'s on-the-fly one: the
//! draft already holds a whole CFG, so the dominator tree
//! ([`DominatorTree::from_cfg`]) and its frontiers ([`DominanceFrontiers`])
//! are cheap, and the construction needs neither recursion nor the use lists
//! that Braun's trivial-phi removal relies on. For each slot:
//!
//! 1. **Liveness.** A block is live-in if it loads the slot before storing it,
//!    or if a successor is live-in and the block does not store it; a backwards
//!    worklist over the reachable blocks finds them.
//! 2. **Placement.** The iterated dominance frontier of the blocks that store
//!    the slot, restricted to the live-in blocks, gets one new block parameter
//!    each (pruned SSA: no parameter is born dead).
//! 3. **Renaming.** In reverse post-order, so a block's immediate dominator
//!    comes first, the value at a block's entry is its new parameter if it got
//!    one, else the value at its immediate dominator's exit, else (in the
//!    entry) `poison`, because the slot is uninitialized there. A load is
//!    replaced by the current value, a store makes its operand current, and
//!    both are removed, along with the `stack_addr`s.
//! 4. **Edges.** Every edge into a block with a new parameter passes the value
//!    at its source's exit.
//!
//! For a counter kept in `slot0`,
//!
//! ```text
//! block0(v0: i32):                        block0(v0: i32):
//!     v1 = stack_addr slot0                   v1 = iconst.i32 0
//!     v2 = iconst.i32 0                       jump block1(v1)
//!     store.i32 v2, v1                    block1(v2: i32):
//!     jump block1                             v3 = icmp.i32 slt v2, v0
//! block1:                                     brif v3, block2, block3
//!     v3 = load.i32 v1                    block2:
//!     v4 = icmp.i32 slt v3, v0                v4 = iconst.i32 1
//!     brif v4, block2, block3                 v5 = iadd.i32 v2, v4
//! block2:                                     jump block1(v5)
//!     v5 = iconst.i32 1                   block3:
//!     v6 = iadd.i32 v3, v5                    return v2
//!     store.i32 v6, v1
//!     jump block1
//! block3:
//!     return v3
//! ```
//!
//! `block1` is in the frontier of `block2`, which stores, and loads the slot
//! first, so it gets the parameter `v2`.
//!
//! Blocks the entry cannot reach get no parameters; their loads read
//! `poison` or the value stored before them in the block, which is never
//! observed. Each promoted slot is one rewrite for bisecting.
//!
//! C99: §6.2.4 paragraph 5, p. 32; PDF p. 44 (an automatic object's initial
//! value is indeterminate, so a load before any store reads `poison`);
//! §6.7.3 paragraph 6, p. 109; PDF p. 121 (volatile accesses are kept as the
//! abstract machine performs them).

use super::{
    draft::{
        Constant,
        Draft,
    },
    frontiers::DominanceFrontiers,
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        ControlFlowGraph,
        DominatorTree,
        Entity,
        Inst,
        InstData,
        InstFlags,
        StackSlot,
        Type,
        Value,
    },
    util::bump::ArenaVec,
};

/// Promotes every promotable stack slot of one function. Returns whether it
/// promoted any.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let slots = draft.body.stack_slot_count();
    if slots == 0 {
        return false;
    }
    let accesses = Accesses::collect(draft);
    if (0..slots).all(|index| accesses.promotable(draft, StackSlot::new(index)).is_none()) {
        return false;
    }
    let cfg = draft.control_flow_graph();
    let tree = DominatorTree::from_cfg(&cfg, draft.scratch);
    let frontiers = DominanceFrontiers::compute(&cfg, &tree, draft.scratch);
    let mut promoter = Promoter::new(draft, &cfg, &tree, &frontiers);
    let mut changed = false;
    for index in 0..slots {
        let slot = StackSlot::new(index);
        let Some(slot_use) = accesses.promotable(draft, slot) else {
            continue;
        };
        if !report.allow() {
            break;
        }
        promoter.promote(draft, accesses.of(slot), slot_use);
        draft.remove_stack_slot(slot);
        changed = true;
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// How a slot's address is used, over the whole function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlotUse {
    /// Nothing loads or stores it.
    Unused,
    /// Only non-volatile loads and stores of this type.
    Typed(Type),
    /// Something else uses the address, or the accesses disagree.
    Escaped,
}

/// What an instruction does with a slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    Load,
    Store,
    /// The `stack_addr` itself.
    Address,
}

/// Every slot's use and its accesses in layout and program order.
struct Accesses<'s> {
    uses:   ArenaVec<'s, SlotUse>,
    starts: ArenaVec<'s, u32>,
    rows:   ArenaVec<'s, (Block, Inst, Access)>,
}

impl<'s> Accesses<'s> {
    /// Scans the live blocks, unreachable ones included.
    fn collect(draft: &Draft<'_, 's>) -> Self {
        let slots = draft.body.stack_slot_count();
        let mut uses = ArenaVec::with_capacity_in(slots, draft.scratch);
        uses.resize(slots, SlotUse::Unused);
        let mut addresses: ArenaVec<'s, Option<StackSlot>> =
            ArenaVec::with_capacity_in(draft.value_count(), draft.scratch);
        addresses.resize(draft.value_count(), None);
        let mut pairs: ArenaVec<'s, (StackSlot, Block, Inst, Access)> =
            ArenaVec::new_in(draft.scratch);
        // Every address first, since a use can come before its `stack_addr`
        // in layout order.
        for block in draft.alive_blocks() {
            for &inst in &draft.block(block).insts {
                if let Some(slot) = stack_addr(draft, inst) {
                    let result = draft.inst_result(inst).expect("`stack_addr` has a result");
                    addresses[result.index()] = Some(slot);
                }
            }
        }
        let slot_of = |value: Value| addresses[draft.resolve(value).index()];
        let escape = |uses: &mut ArenaVec<'s, SlotUse>, value: Value| {
            if let Some(slot) = slot_of(value) {
                uses[slot.index()] = SlotUse::Escaped;
            }
        };
        for block in draft.alive_blocks() {
            let data = draft.block(block);
            for &inst in &data.insts {
                if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                    continue;
                }
                let (ty, flags, addr, access) = match *draft.inst(inst) {
                    | InstData::StackAddr { slot } => {
                        if !draft.is_stack_slot_removed(slot) {
                            pairs.push((slot, block, inst, Access::Address));
                        }
                        continue;
                    },
                    | InstData::Load {
                        ty, flags, addr, ..
                    } => (ty, flags, addr, Access::Load),
                    | InstData::Store {
                        ty,
                        flags,
                        args: [value, addr],
                        ..
                    } => {
                        escape(&mut uses, value);
                        (ty, flags, addr, Access::Store)
                    },
                    | _ => {
                        draft.for_each_operand(inst, |value| escape(&mut uses, value));
                        continue;
                    },
                };
                let Some(slot) = slot_of(addr) else {
                    continue;
                };
                let entry = &mut uses[slot.index()];
                *entry = match *entry {
                    | _ if flags.contains(InstFlags::VOLATILE) => SlotUse::Escaped,
                    | SlotUse::Unused => SlotUse::Typed(ty),
                    | SlotUse::Typed(seen) if seen == ty => SlotUse::Typed(ty),
                    | SlotUse::Typed(_) | SlotUse::Escaped => SlotUse::Escaped,
                };
                pairs.push((slot, block, inst, access));
            }
            data.term
                .for_each_control_operand(|value| escape(&mut uses, value));
            for edge in data.term.edges() {
                for &arg in &edge.args {
                    escape(&mut uses, arg);
                }
            }
        }
        // Group the accesses by slot, keeping their order within a slot.
        let mut starts = ArenaVec::with_capacity_in(slots + 1, draft.scratch);
        starts.resize(slots + 1, 0_u32);
        for &(slot, ..) in &pairs {
            starts[slot.index() + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut cursors = ArenaVec::with_capacity_in(slots, draft.scratch);
        cursors.extend_from_slice(&starts[..slots]);
        let mut rows = ArenaVec::with_capacity_in(pairs.len(), draft.scratch);
        rows.resize(pairs.len(), (Block::new(0), Inst::new(0), Access::Address));
        for &(slot, block, inst, access) in &pairs {
            let cursor = &mut cursors[slot.index()];
            rows[*cursor as usize] = (block, inst, access);
            *cursor += 1;
        }
        Self { uses, starts, rows }
    }

    /// How `slot` is used if it can be promoted: [`SlotUse::Unused`], or
    /// [`SlotUse::Typed`] with a type as wide as the slot. `None` if it stays
    /// in memory.
    fn promotable(&self, draft: &Draft<'_, '_>, slot: StackSlot) -> Option<SlotUse> {
        if draft.is_stack_slot_removed(slot) {
            return None;
        }
        match self.uses[slot.index()] {
            | SlotUse::Typed(ty) if ty.bytes() != draft.body.stack_slot(slot).size => None,
            | SlotUse::Escaped => None,
            | slot_use @ (SlotUse::Unused | SlotUse::Typed(_)) => Some(slot_use),
        }
    }

    /// The accesses of `slot`, grouped by block in layout order and in
    /// program order within a block.
    fn of(&self, slot: StackSlot) -> &[(Block, Inst, Access)] {
        let index = slot.index();
        &self.rows[self.starts[index] as usize..self.starts[index + 1] as usize]
    }
}

/// The slot a live `stack_addr` names, if `inst` is one.
fn stack_addr(draft: &Draft<'_, '_>, inst: Inst) -> Option<StackSlot> {
    if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
        return None;
    }
    match *draft.inst(inst) {
        | InstData::StackAddr { slot } if !draft.is_stack_slot_removed(slot) => Some(slot),
        | _ => None,
    }
}

/// The analyses and per-block buffers that promoting one slot after another
/// shares; the buffers are reset for each slot.
struct Promoter<'a, 's> {
    cfg:         &'a ControlFlowGraph<'s>,
    tree:        &'a DominatorTree<'s>,
    frontiers:   &'a DominanceFrontiers<'s>,
    /// Each block's accesses of the slot, as a range of its rows.
    ranges:      ArenaVec<'s, (u32, u32)>,
    /// Whether the block stores the slot.
    stores:      ArenaVec<'s, bool>,
    live_in:     ArenaVec<'s, bool>,
    /// Whether the block is in the iterated dominance frontier of the stores.
    in_frontier: ArenaVec<'s, bool>,
    /// Whether the block was queued for the frontier walk.
    queued:      ArenaVec<'s, bool>,
    /// The new parameter of each block that got one.
    params:      ArenaVec<'s, Option<Value>>,
    /// The slot's value at each block's exit; `None` is `poison`.
    exits:       ArenaVec<'s, Option<Value>>,
    worklist:    ArenaVec<'s, Block>,
    /// The `poison` constant of each type, once added.
    poison:      [Option<Value>; Type::ALL.len()],
}

impl<'a, 's> Promoter<'a, 's> {
    fn new(
        draft: &Draft<'_, 's>,
        cfg: &'a ControlFlowGraph<'s>,
        tree: &'a DominatorTree<'s>,
        frontiers: &'a DominanceFrontiers<'s>,
    ) -> Self {
        let blocks = draft.blocks.len();
        let flags = || {
            let mut flags = ArenaVec::with_capacity_in(blocks, draft.scratch);
            flags.resize(blocks, false);
            flags
        };
        let values = || {
            let mut values = ArenaVec::with_capacity_in(blocks, draft.scratch);
            values.resize(blocks, None);
            values
        };
        let mut ranges = ArenaVec::with_capacity_in(blocks, draft.scratch);
        ranges.resize(blocks, (0, 0));
        Self {
            cfg,
            tree,
            frontiers,
            ranges,
            stores: flags(),
            live_in: flags(),
            in_frontier: flags(),
            queued: flags(),
            params: values(),
            exits: values(),
            worklist: ArenaVec::new_in(draft.scratch),
            poison: [None; Type::ALL.len()],
        }
    }

    /// Replaces the slot whose accesses are `rows` by SSA values of its type,
    /// or only removes its `stack_addr`s if nothing loads or stores it.
    fn promote(
        &mut self,
        draft: &mut Draft<'_, 's>,
        rows: &[(Block, Inst, Access)],
        slot_use: SlotUse,
    ) {
        let SlotUse::Typed(ty) = slot_use else {
            for &(_, inst, _) in rows {
                draft.remove_inst(inst);
            }
            return;
        };
        self.reset();
        self.summarize(rows);
        self.find_live_in();
        self.place_params(draft, ty);
        let order = self.tree.reverse_post_order();
        for &block in order {
            let entry = match (self.params[block.index()], self.tree.idom(block)) {
                | (Some(param), _) => Some(param),
                | (None, Some(idom)) => self.exits[idom.index()],
                | (None, None) => None,
            };
            self.exits[block.index()] = self.rename(draft, rows, block, entry, ty);
        }
        for index in 0..draft.blocks.len() {
            let block = Block::new(index);
            if draft.block(block).alive && !self.tree.is_reachable(block) {
                self.exits[index] = self.rename(draft, rows, block, None, ty);
            }
        }
        self.pass_exits(draft, ty);
    }

    fn reset(&mut self) {
        self.ranges.fill((0, 0));
        self.stores.fill(false);
        self.live_in.fill(false);
        self.in_frontier.fill(false);
        self.queued.fill(false);
        self.params.fill(None);
        self.exits.fill(None);
    }

    /// Records each block's range of rows, which stores the slot, and which
    /// loads it before storing it (those are live-in).
    fn summarize(&mut self, rows: &[(Block, Inst, Access)]) {
        for (index, &(block, _, access)) in rows.iter().enumerate() {
            let row = u32::try_from(index).expect("fewer than 2^32 accesses");
            let range = &mut self.ranges[block.index()];
            if range.0 == range.1 {
                *range = (row, row);
            }
            range.1 = row + 1;
            match access {
                | Access::Load if !self.stores[block.index()] => self.live_in[block.index()] = true,
                | Access::Store => self.stores[block.index()] = true,
                | Access::Load | Access::Address => {},
            }
        }
    }

    /// Step 1: spreads liveness backwards from the blocks that load first,
    /// stopping at blocks that store.
    fn find_live_in(&mut self) {
        self.worklist.clear();
        for &block in self.tree.reverse_post_order() {
            if self.live_in[block.index()] {
                self.worklist.push(block);
            }
        }
        while let Some(block) = self.worklist.pop() {
            for &predecessor in self.cfg.predecessors(block) {
                let index = predecessor.index();
                if self.tree.is_reachable(predecessor)
                    && !self.live_in[index]
                    && !self.stores[index]
                {
                    self.live_in[index] = true;
                    self.worklist.push(predecessor);
                }
            }
        }
    }

    /// Step 2: adds a parameter to each live-in block of the iterated
    /// dominance frontier of the stores, in reverse post-order.
    fn place_params(&mut self, draft: &mut Draft<'_, 's>, ty: Type) {
        self.worklist.clear();
        for &block in self.tree.reverse_post_order() {
            if self.stores[block.index()] {
                self.queued[block.index()] = true;
                self.worklist.push(block);
            }
        }
        while let Some(block) = self.worklist.pop() {
            for &member in self.frontiers.of(block) {
                let index = member.index();
                self.in_frontier[index] = true;
                if !self.queued[index] {
                    self.queued[index] = true;
                    self.worklist.push(member);
                }
            }
        }
        for &block in self.tree.reverse_post_order() {
            let index = block.index();
            if self.in_frontier[index] && self.live_in[index] {
                self.params[index] = Some(draft.add_block_param(block, ty));
            }
        }
    }

    /// Step 3 for one block: rewrites its accesses given the value at its
    /// entry, and returns the value at its exit.
    fn rename(
        &mut self,
        draft: &mut Draft<'_, 's>,
        rows: &[(Block, Inst, Access)],
        block: Block,
        entry: Option<Value>,
        ty: Type,
    ) -> Option<Value> {
        let (start, end) = self.ranges[block.index()];
        let mut current = entry;
        for &(_, inst, access) in &rows[start as usize..end as usize] {
            match access {
                | Access::Load => {
                    let value = self.materialize(draft, current, ty);
                    let result = draft.inst_result(inst).expect("a load has a result");
                    draft.replace(result, value);
                },
                | Access::Store => {
                    let InstData::Store {
                        args: [value, _], ..
                    } = *draft.inst(inst)
                    else {
                        unreachable!("the access was recorded as a store");
                    };
                    current = Some(draft.resolve(value));
                },
                | Access::Address => {},
            }
            draft.remove_inst(inst);
        }
        current
    }

    /// Step 4: appends each block's exit value to its edges into blocks with
    /// a new parameter.
    fn pass_exits(&mut self, draft: &mut Draft<'_, 's>, ty: Type) {
        for index in 0..draft.blocks.len() {
            let block = Block::new(index);
            if !draft.block(block).alive
                || !draft
                    .block(block)
                    .term
                    .edges()
                    .any(|edge| self.params[edge.target.index()].is_some())
            {
                continue;
            }
            let value = self.materialize(draft, self.exits[index], ty);
            for edge in draft.block_mut(block).term.edges_mut() {
                if self.params[edge.target.index()].is_some() {
                    edge.args.push(value);
                }
            }
        }
    }

    /// `value`, or the `poison` of `ty` for `None`.
    fn materialize(&mut self, draft: &mut Draft<'_, 's>, value: Option<Value>, ty: Type) -> Value {
        value.unwrap_or_else(|| {
            *self.poison[ty as usize]
                .get_or_insert_with(|| draft.add_constant(Constant::Poison(ty)))
        })
    }
}
