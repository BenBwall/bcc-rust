//! Terminators, and the parallel copies that replace block parameters.
//!
//! An edge that passes arguments assigns the target's parameter registers
//! as one parallel copy. After a `jump` the copies go at the end of the
//! source block; an edge out of a `brif` or `switch` gets a block of its
//! own holding the copies and a `jmp`, so a copy never runs on a path that
//! does not take its edge (the lost-copy problem). A parallel copy is
//! sequentialized by emitting each copy whose destination no pending copy
//! still reads, and by breaking a cycle (the swap problem) through a
//! temporary that saves one destination's old value (Boissinot et al.,
//! "Revisiting Out-of-SSA Translation for Correctness, Code Quality, and
//! Efficiency", CGO 2009, Algorithm 1). Constant and address arguments are
//! materialized last, since they read no register.
//!
//! A `switch` becomes a chain of compares, one block per case; on an
//! `i128` each case compares the low half and then the high half.
//!
//! An `indirect_br` (GNU extension: GCC "Labels as Values") becomes
//! `jmpq *reg`. Its edges cannot get blocks of their own, since the
//! address names the target, so a labeled block with parameters gets a
//! *landing block* instead: the label's address is the landing block's,
//! which copies fresh incoming registers into the parameters and jumps to
//! the block. Before the jump, the `indirect_br` assigns every listed
//! target's incoming registers, which nothing else reads, so the copies
//! for the edges not taken are harmless; direct edges still assign the
//! parameters themselves. A labeled block that no path reaches gets a
//! `ud2` block for its address.

use super::{
    Selector,
    ValueKind,
    immediate,
    mask,
    width,
};
use crate::{
    backend::native::{
        mir::{
            Cond,
            MBlock,
            MInst,
            Operand,
            Reg,
            VReg,
            Width,
        },
        support::Construct,
    },
    ir::{
        BlockCall,
        BlockList,
        Entity,
        InstData,
        Type,
        Value,
    },
    util::bump::ArenaVec,
};

impl Selector<'_, '_, '_> {
    /// Selects a terminator.
    pub(super) fn terminator(&mut self, data: InstData) -> Result<(), Construct> {
        match data {
            | InstData::Jump { dest } => self.jump(dest),
            | InstData::Brif { cond, dests } => {
                if let ValueKind::Const(bits) = self.kind(cond) {
                    self.jump(dests[usize::from(bits & 1 == 0)]);
                    return Ok(());
                }
                let mut pending = ArenaVec::new_in(self.arena);
                let taken = self.edge_target(dests[0], &mut pending);
                let not_taken = self.edge_target(dests[1], &mut pending);
                let cond = self.condition(cond);
                self.emit(MInst::Jcc {
                    cond,
                    taken,
                    not_taken,
                });
                self.fill_edges(&pending);
            },
            | InstData::Switch {
                value,
                default,
                cases,
            } => self.switch(value, default, cases),
            | InstData::Return { value } => self.ret(value),
            | InstData::IndirectBr { addr, dests } => self.indirect_branch(addr, dests),
            | InstData::Unreachable => self.emit(MInst::Ud2),
            | _ => return Err(Construct::Opcode(data.opcode())),
        }
        Ok(())
    }

    /// Gives every IR block label the machine block its address names:
    /// the labeled block itself, a landing block that assigns its
    /// parameters from incoming registers, or a `ud2` block when the
    /// labeled block is unreachable.
    pub(super) fn labels(&mut self) {
        let body = self.body;
        let current = self.current;
        for (label, block) in body.labels() {
            let params = body.block_params(block);
            let address = match self.block_map[block.index()] {
                | Some(target) if params.is_empty() => {
                    self.labeled[block.index()] = Some(LabelTarget {
                        block:    target,
                        incoming: 0,
                    });
                    target
                },
                | Some(target) => {
                    let landing = self.func.block(self.arena);
                    let incoming = u32::try_from(self.incoming.len())
                        .expect("incoming register indices fit u32");
                    self.current = landing;
                    for &param in params {
                        let from = self.fresh(self.ty(param));
                        self.incoming.push(from);
                        match (self.kind(param), from) {
                            | (ValueKind::Reg(dst), ValueKind::Reg(src)) =>
                                self.copy(Reg::Virt(dst), Reg::Virt(src)),
                            | (
                                ValueKind::Pair(low, high),
                                ValueKind::Pair(from_low, from_high),
                            ) => {
                                self.copy(Reg::Virt(low), Reg::Virt(from_low));
                                self.copy(Reg::Virt(high), Reg::Virt(from_high));
                            },
                            | _ => unreachable!("block parameters are registers"),
                        }
                    }
                    self.emit(MInst::Jmp { target });
                    self.labeled[block.index()] = Some(LabelTarget {
                        block: landing,
                        incoming,
                    });
                    landing
                },
                | None => {
                    let trap = self.func.block(self.arena);
                    self.current = trap;
                    self.emit(MInst::Ud2);
                    trap
                },
            };
            self.func.labels.push((label.as_u32(), address));
        }
        self.current = current;
    }

    /// An `indirect_br`: the incoming registers of every listed target
    /// that takes arguments, then `jmpq *addr`.
    fn indirect_branch(&mut self, addr: Value, dests: BlockList) {
        let body = self.body;
        let target = self.reg(addr);
        let mut blocks = ArenaVec::with_capacity_in(body.block_list(dests).len(), self.arena);
        for dest in body.block_list(dests) {
            let (block, args) = body.block_call(dest);
            let label = self.labeled[block.index()]
                .expect("the verifier requires every indirect target to be labeled");
            blocks.push(label.block);
            let start = label.incoming as usize;
            for (index, &arg) in args.iter().enumerate() {
                match (self.incoming[start + index], self.kind(arg)) {
                    | (_, ValueKind::Unknown) => {},
                    | (ValueKind::Reg(dst), ValueKind::Reg(src)) =>
                        self.copy(Reg::Virt(dst), Reg::Virt(src)),
                    | (ValueKind::Reg(dst), _) => self.leaf_into(Reg::Virt(dst), arg),
                    | (ValueKind::Pair(low, high), ValueKind::Pair(from_low, from_high)) => {
                        self.copy(Reg::Virt(low), Reg::Virt(from_low));
                        self.copy(Reg::Virt(high), Reg::Virt(from_high));
                    },
                    | (ValueKind::Pair(low, high), ValueKind::Wide(bits)) => {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "The halves are split here."
                        )]
                        let halves = [bits as u64, (bits >> 64) as u64];
                        self.constant(Reg::Virt(low), halves[0], Type::I64);
                        self.constant(Reg::Virt(high), halves[1], Type::I64);
                    },
                    | _ => unreachable!("incoming registers match their parameters"),
                }
            }
        }
        let dests = u32::try_from(self.func.indirect.len()).expect("indirect jumps fit u32");
        self.func.indirect.push(blocks);
        self.emit(MInst::JmpIndirect { target, dests });
    }

    /// The copies of an edge, then a `jmp` to its block.
    fn jump(&mut self, dest: BlockCall) {
        self.edge_copies(dest);
        let target = self.target_block(dest);
        self.emit(MInst::Jmp { target });
    }

    /// The machine block an edge's IR block starts.
    fn target_block(&self, dest: BlockCall) -> MBlock {
        let (block, _) = self.body.block_call(dest);
        self.block_map[block.index()].expect("an edge from reachable code reaches its target")
    }

    /// Where a branch goes for an edge: the target itself, or a new block
    /// for the edge's copies, recorded in `pending` to fill after the
    /// branch.
    fn edge_target(
        &mut self,
        dest: BlockCall,
        pending: &mut ArenaVec<'_, (MBlock, BlockCall)>,
    ) -> MBlock {
        if self.body.block_call(dest).1.is_empty() {
            return self.target_block(dest);
        }
        let block = self.func.block(self.arena);
        pending.push((block, dest));
        block
    }

    /// Fills the edge blocks [`Self::edge_target`] created.
    fn fill_edges(&mut self, pending: &[(MBlock, BlockCall)]) {
        let current = self.current;
        for &(block, dest) in pending {
            self.current = block;
            self.jump(dest);
        }
        self.current = current;
    }

    /// The parallel copy `params := args` of an edge, sequentialized.
    fn edge_copies(&mut self, edge: BlockCall) {
        let (block, args) = self.body.block_call(edge);
        let params = self.body.block_params(block);
        let mut moves: ArenaVec<'_, (VReg, VReg)> = ArenaVec::new_in(self.arena);
        let mut leaves = ArenaVec::new_in(self.arena);
        let mut wide = ArenaVec::new_in(self.arena);
        for (&param, &arg) in params.iter().zip(args) {
            match (self.kind(param), self.kind(arg)) {
                | (ValueKind::Pair(low, high), ValueKind::Pair(from_low, from_high)) =>
                    for (dst, src) in [(low, from_low), (high, from_high)] {
                        if dst != src {
                            moves.push((dst, src));
                        }
                    },
                | (ValueKind::Pair(low, high), ValueKind::Wide(bits)) => {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "The halves are split here."
                    )]
                    let halves = [bits as u64, (bits >> 64) as u64];
                    wide.push(((low, high), halves));
                },
                | (ValueKind::Reg(dst), ValueKind::Reg(src)) if src == dst => {},
                | (ValueKind::Reg(dst), ValueKind::Reg(src)) => moves.push((dst, src)),
                | (_, ValueKind::Unknown) => {},
                | (ValueKind::Reg(dst), _) => leaves.push((dst, arg)),
                | _ => unreachable!("block parameters are registers"),
            }
        }
        while !moves.is_empty() {
            let ready = (0..moves.len()).find(|&index| {
                let dst = moves[index].0;
                !moves.iter().any(|&(_, src)| src == dst)
            });
            if let Some(index) = ready {
                let (dst, src) = moves.swap_remove(index);
                self.copy(Reg::Virt(dst), Reg::Virt(src));
                continue;
            }
            // Every destination is still read: save one and redirect its
            // readers to the copy.
            let saved = moves[0].0;
            let temp = self.func.vreg_of(self.func.classes[saved.index()]);
            self.copy(Reg::Virt(temp), Reg::Virt(saved));
            for (_, src) in &mut moves {
                if *src == saved {
                    *src = temp;
                }
            }
        }
        for (dst, arg) in leaves {
            self.leaf_into(Reg::Virt(dst), arg);
        }
        for ((low, high), [low_bits, high_bits]) in wide {
            self.constant(Reg::Virt(low), low_bits, Type::I64);
            self.constant(Reg::Virt(high), high_bits, Type::I64);
        }
    }

    /// Materializes a constant or address argument into `dst`.
    fn leaf_into(&mut self, dst: Reg, value: Value) {
        match self.kind(value) {
            | ValueKind::Const(bits) => self.constant(dst, bits, self.ty(value)),
            | ValueKind::Addr(base, disp) => self.address_into(dst, base, disp),
            | ValueKind::Reg(_) | ValueKind::Unknown | ValueKind::Pair(..) | ValueKind::Wide(_) => {
            },
        }
    }

    /// A `switch` as a chain of compares against each case, one block per
    /// case, ending in a jump to the default.
    fn switch(&mut self, value: Value, default: BlockCall, cases: crate::ir::CaseList) {
        let ty = self.ty(value);
        let body = self.body;
        if ty == Type::I128 {
            self.wide_switch(value, default, cases);
            return;
        }
        if let ValueKind::Const(bits) = self.kind(value) {
            let dest = body
                .switch_cases(cases)
                .find(|&(case, _)| case_bits(case, ty) == bits)
                .map_or(default, |(_, dest)| dest);
            self.jump(dest);
            return;
        }
        let width = width(ty);
        let scrutinee = self.reg(value);
        let mut pending = ArenaVec::new_in(self.arena);
        for (case, dest) in body.switch_cases(cases) {
            let taken = self.edge_target(dest, &mut pending);
            let next = self.func.block(self.arena);
            let bits = case_bits(case, ty);
            let rhs = match immediate(bits, width) {
                | Some(imm) => Operand::Imm(imm),
                | None => {
                    let reg = self.vreg();
                    self.constant(reg, bits, ty);
                    Operand::Reg(reg)
                },
            };
            self.emit(MInst::Cmp {
                width,
                lhs: scrutinee,
                rhs,
            });
            self.emit(MInst::Jcc {
                cond: Cond::E,
                taken,
                not_taken: next,
            });
            self.current = next;
        }
        let otherwise = self.edge_target(default, &mut pending);
        self.emit(MInst::Jmp { target: otherwise });
        self.fill_edges(&pending);
    }
}

impl Selector<'_, '_, '_> {
    /// A `switch` on an `i128`: per case, a compare of each half.
    fn wide_switch(&mut self, value: Value, default: BlockCall, cases: crate::ir::CaseList) {
        let body = self.body;
        if let ValueKind::Wide(bits) = self.kind(value) {
            let dest = body
                .switch_cases(cases)
                .find(|&(case, _)| case == bits)
                .map_or(default, |(_, dest)| dest);
            self.jump(dest);
            return;
        }
        let (low, high) = self.pair(value);
        let mut pending = ArenaVec::new_in(self.arena);
        for (case, dest) in body.switch_cases(cases) {
            let taken = self.edge_target(dest, &mut pending);
            let next = self.func.block(self.arena);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The halves are split here."
            )]
            let halves = [(low, case as u64), (high, (case >> 64) as u64)];
            for (index, (half, bits)) in halves.into_iter().enumerate() {
                let rhs = match immediate(bits, Width::B64) {
                    | Some(imm) => Operand::Imm(imm),
                    | None => {
                        let reg = self.vreg();
                        self.constant(reg, bits, Type::I64);
                        Operand::Reg(reg)
                    },
                };
                self.emit(MInst::Cmp {
                    width: Width::B64,
                    lhs: half,
                    rhs,
                });
                if index == 0 {
                    let high_half = self.func.block(self.arena);
                    self.emit(MInst::Jcc {
                        cond:      Cond::Ne,
                        taken:     next,
                        not_taken: high_half,
                    });
                    self.current = high_half;
                } else {
                    self.emit(MInst::Jcc {
                        cond: Cond::E,
                        taken,
                        not_taken: next,
                    });
                }
            }
            self.current = next;
        }
        let otherwise = self.edge_target(default, &mut pending);
        self.emit(MInst::Jmp { target: otherwise });
        self.fill_edges(&pending);
    }
}

/// Where a labeled block's address leads.
#[derive(Clone, Copy, Debug)]
pub(in crate::backend::native) struct LabelTarget {
    /// The labeled block, or its landing block.
    pub(super) block:    MBlock,
    /// The first of the block's incoming registers in
    /// [`Selector::incoming`].
    pub(super) incoming: u32,
}

/// A case constant's bits at the scrutinee's width.
#[expect(
    clippy::cast_possible_truncation,
    reason = "The masked value has at most 64 bits; i128 is refused."
)]
fn case_bits(case: u128, ty: Type) -> u64 {
    (case as u64) & mask(ty)
}
