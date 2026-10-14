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
//! A `switch` becomes a chain of compares, one block per case.

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
        },
        support::Construct,
    },
    ir::{
        BlockCall,
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
            | InstData::Unreachable => self.emit(MInst::Ud2),
            | _ => return Err(Construct::Opcode(data.opcode())),
        }
        Ok(())
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
        for (&param, &arg) in params.iter().zip(args) {
            let ValueKind::Reg(dst) = self.kind(param) else {
                unreachable!("block parameters are registers");
            };
            match self.kind(arg) {
                | ValueKind::Reg(src) if src == dst => {},
                | ValueKind::Reg(src) => moves.push((dst, src)),
                | ValueKind::Unknown => {},
                | _ => leaves.push((dst, arg)),
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
    }

    /// Materializes a constant or address argument into `dst`.
    fn leaf_into(&mut self, dst: Reg, value: Value) {
        match self.kind(value) {
            | ValueKind::Const(bits) => self.constant(dst, bits, self.ty(value)),
            | ValueKind::Addr(base, disp) => self.address_into(dst, base, disp),
            | ValueKind::Reg(_) | ValueKind::Unknown => {},
        }
    }

    /// A `switch` as a chain of compares against each case, one block per
    /// case, ending in a jump to the default.
    fn switch(&mut self, value: Value, default: BlockCall, cases: crate::ir::CaseList) {
        let ty = self.ty(value);
        let body = self.body;
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

/// A case constant's bits at the scrutinee's width.
#[expect(
    clippy::cast_possible_truncation,
    reason = "The masked value has at most 64 bits; i128 is refused."
)]
fn case_bits(case: u128, ty: Type) -> u64 {
    (case as u64) & mask(ty)
}
