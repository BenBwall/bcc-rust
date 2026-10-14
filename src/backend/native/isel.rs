//! Instruction selection: one post-ABI IR function to machine IR over
//! virtual registers.
//!
//! The selector is a macro expander: it walks the reachable blocks in
//! layout order and expands each IR instruction into a fixed x86-64
//! sequence. Three peepholes stand in for pattern matching:
//!
//! - *Leaves* (`iconst`, `null`, `poison`, `stack_addr`, `global_addr`,
//!   `func_addr`, constant `ptr_add`s of addresses, and conversions of
//!   constants) get no register. Each use materializes them on the spot: an
//!   immediate, a memory operand's base, or a fresh register defined right
//!   before the use. *Aliases* (`freeze` and the conversions that change no
//!   bit, such as `trunc`) share their operand's register.
//! - *Address folding*: a load or store through a leaf address or a `ptr_add`
//!   of a register and a small constant uses one memory operand.
//! - *Compare fusion*: an `icmp` whose only use is the `brif` or `select` of
//!   its own block is emitted right before that user's `jcc` or `cmov`.
//!
//! [`select`] classifies every value first, in reverse post-order so that
//! operands are classified before their users, then selects the blocks.
//! Calls, function entry and returns are in `isel/calls.rs`; branches and
//! the parallel copies that replace block parameters in `isel/edges.rs`;
//! loads, stores, `copy` and `fill` in `isel/memory.rs`.
//!
//! Integer values narrower than 64 bits live in registers whose upper bits
//! are unspecified, except `i1`, which is always 0 or 1 in the whole
//! register (comparisons produce it that way). Operations that read upper
//! bits extend first.

// Selection by family
mod calls;
mod edges;
mod memory;

use super::{
    conv::CallConv,
    mir::{
        AluOp,
        Cond,
        FrameSlotData,
        MBlock,
        MFunction,
        MInst,
        Operand,
        PReg,
        Reg,
        ShiftAmount,
        ShiftOp,
        UnaryOp,
        VReg,
        Width,
    },
    support::Construct,
};
use crate::{
    ir::{
        Block,
        Body,
        ControlFlowGraph,
        DominatorTree,
        Entity,
        Function,
        Inst,
        InstData,
        IntCC,
        Module,
        Opcode,
        Signature,
        Symbol,
        Type,
        Value,
        ValueDef,
    },
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Selects `function`'s body into machine IR in `arena`.
pub(crate) fn select<'a>(
    module: &Module<'_>,
    function: &Function<'_>,
    body: &Body<'_>,
    target: Target,
    arena: &'a Bump,
) -> Result<MFunction<'a>, Construct> {
    let cfg = ControlFlowGraph::compute(body, arena);
    let tree = DominatorTree::compute(body, &cfg, arena);
    let mut selector = Selector::new(module, function, body, target, arena);
    for (_, slot) in body.stack_slots() {
        selector.func.slots.push(FrameSlotData {
            size:  slot.size,
            align: u32::try_from(slot.align.bytes()).expect("the support check bounds alignments"),
        });
    }
    for &block in tree.reverse_post_order() {
        selector.classify_block(block);
    }
    for block in body.blocks() {
        if tree.is_reachable(block) {
            selector.count_uses(block);
        }
    }
    for block in body.blocks() {
        if tree.is_reachable(block) {
            selector.find_fusions(block);
            selector.block_map[block.index()] = Some(selector.func.block(arena));
        }
    }
    for block in body.blocks() {
        if let Some(mblock) = selector.block_map[block.index()] {
            selector.current = mblock;
            if block.index() == 0 {
                selector.entry()?;
            }
            for &inst in body.block_insts(block) {
                selector.instruction(inst)?;
            }
        }
    }
    Ok(selector.func)
}

/// How the selector represents an IR value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ValueKind {
    /// Not classified: defined in an unreachable block.
    Unknown,
    /// Held in a virtual register.
    Reg(VReg),
    /// A constant, masked to its type's width.
    Const(u64),
    /// An address: a frame slot or symbol plus a byte offset.
    Addr(AddrBase, i64),
}

/// What a constant address is relative to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AddrBase {
    Slot(u32),
    Symbol(Symbol),
}

/// The selection state of one function.
pub(super) struct Selector<'a, 'm, 'ir> {
    pub(super) module:    &'m Module<'ir>,
    pub(super) function:  &'m Function<'ir>,
    pub(super) body:      &'m Body<'ir>,
    pub(super) signature: Signature<'ir>,
    pub(super) conv:      CallConv,
    pub(super) target:    Target,
    pub(super) arena:     &'a Bump,
    pub(super) func:      MFunction<'a>,
    /// Each value's representation.
    pub(super) values:    ArenaVec<'a, ValueKind>,
    /// Each value's number of uses in reachable code.
    pub(super) uses:      ArenaVec<'a, u32>,
    /// Instructions with nothing to emit at their position: leaves,
    /// aliases, and compares fused into their user.
    pub(super) skipped:   ArenaVec<'a, bool>,
    /// Compares fused into their user.
    pub(super) fused:     ArenaVec<'a, bool>,
    /// Each reachable IR block's machine block.
    pub(super) block_map: ArenaVec<'a, Option<MBlock>>,
    /// The block instructions are appended to.
    pub(super) current:   MBlock,
    /// How `return` hands back a structure result.
    pub(super) result:    calls::ResultArea,
}

impl<'a, 'm, 'ir> Selector<'a, 'm, 'ir> {
    fn new(
        module: &'m Module<'ir>,
        function: &'m Function<'ir>,
        body: &'m Body<'ir>,
        target: Target,
        arena: &'a Bump,
    ) -> Self {
        Self {
            module,
            function,
            body,
            signature: module.signature(function.signature),
            conv: CallConv::of(target),
            target,
            arena,
            func: MFunction::new_in(arena),
            values: filled(arena, body.value_count(), ValueKind::Unknown),
            uses: filled(arena, body.value_count(), 0),
            skipped: filled(arena, body.inst_count(), false),
            fused: filled(arena, body.inst_count(), false),
            block_map: filled(arena, body.block_count(), None),
            current: MBlock::new(0),
            result: calls::ResultArea::None,
        }
    }

    /// Appends an instruction to the current block.
    pub(super) fn emit(&mut self, inst: MInst) {
        self.func.blocks[self.current.index()].insts.push(inst);
    }

    pub(super) fn ty(&self, value: Value) -> Type {
        self.body.value_type(value)
    }

    pub(super) fn kind(&self, value: Value) -> ValueKind {
        self.values[value.index()]
    }

    /// Classifies the parameters and results of one block whose operands'
    /// definitions are already classified.
    fn classify_block(&mut self, block: Block) {
        for &param in self.body.block_params(block) {
            let vreg = self.func.vreg();
            self.values[param.index()] = ValueKind::Reg(vreg);
        }
        for &inst in self.body.block_insts(block) {
            let Some(result) = self.body.inst_result(inst) else {
                continue;
            };
            let kind = self.classify(inst);
            self.skipped[inst.index()] = kind.is_some();
            let kind = match kind {
                | Some(kind) => kind,
                | None => ValueKind::Reg(self.func.vreg()),
            };
            self.values[result.index()] = kind;
        }
    }

    /// The representation of an instruction's result when it needs no code
    /// of its own: a leaf or an alias. `None` means it gets a register.
    fn classify(&self, inst: Inst) -> Option<ValueKind> {
        let ty = self.ty(self.body.inst_result(inst)?);
        let data = *self.body.inst(inst);
        Some(match data {
            | InstData::Const { bits, .. } => ValueKind::Const(bits.get() & mask(ty)),
            | InstData::Nullary {
                opcode: Opcode::Null | Opcode::Poison,
                ..
            } => ValueKind::Const(0),
            | InstData::StackAddr { slot } => ValueKind::Addr(AddrBase::Slot(slot.as_u32()), 0),
            | InstData::GlobalAddr { global } =>
                ValueKind::Addr(AddrBase::Symbol(Symbol::Global(global)), 0),
            | InstData::FuncAddr { func } =>
                ValueKind::Addr(AddrBase::Symbol(Symbol::Function(func)), 0),
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, offset],
                ..
            } => match (self.kind(base), self.kind(offset)) {
                | (ValueKind::Addr(base, disp), ValueKind::Const(offset)) =>
                    ValueKind::Addr(base, disp.wrapping_add(offset.cast_signed())),
                | (ValueKind::Const(base), ValueKind::Const(offset)) =>
                    ValueKind::Const(base.wrapping_add(offset)),
                | _ => return None,
            },
            | InstData::IntCompare {
                cond,
                ty: operand,
                args: [lhs, rhs],
            } => match (self.kind(lhs), self.kind(rhs)) {
                | (ValueKind::Const(lhs), ValueKind::Const(rhs)) =>
                    ValueKind::Const(u64::from(compare(cond, operand, lhs, rhs))),
                | _ => return None,
            },
            | InstData::Unary { opcode, arg, .. } => {
                let from = self.ty(arg);
                match (opcode, self.kind(arg)) {
                    | (_, ValueKind::Unknown) => return None,
                    | (Opcode::Freeze, kind) => kind,
                    | (Opcode::Zext | Opcode::Inttoptr, ValueKind::Const(bits)) =>
                        ValueKind::Const(bits & mask(from) & mask(ty)),
                    | (Opcode::Sext, ValueKind::Const(bits)) =>
                        ValueKind::Const(sign_extend(bits, from) & mask(ty)),
                    | (
                        Opcode::Trunc | Opcode::Ptrtoint | Opcode::Bitcast,
                        ValueKind::Const(bits),
                    ) => ValueKind::Const(bits & mask(ty)),
                    // Conversions that keep every bit the result has. An
                    // `i1` result must be canonical, which a wider value's
                    // register is not.
                    | (Opcode::Trunc | Opcode::Ptrtoint | Opcode::Bitcast, kind)
                        if ty != Type::I1 =>
                        kind,
                    | (Opcode::Inttoptr, kind) if from.bits() == 64 => kind,
                    | (Opcode::Zext, kind @ ValueKind::Reg(_)) if from == Type::I1 => kind,
                    | _ => return None,
                }
            },
            | _ => return None,
        })
    }

    /// Counts the uses of every value in one reachable block.
    fn count_uses(&mut self, block: Block) {
        for &inst in self.body.block_insts(block) {
            let uses = &mut self.uses;
            self.body
                .visit_operands(inst, |value| uses[value.index()] += 1);
        }
    }

    /// Marks the compares of `block` whose only use is the block's `brif`
    /// condition or a later `select` condition in the block.
    fn find_fusions(&mut self, block: Block) {
        let insts = self.body.block_insts(block);
        for &user in insts {
            let (InstData::Brif { cond, .. }
            | InstData::Select {
                args: [cond, ..], ..
            }) = *self.body.inst(user)
            else {
                continue;
            };
            if self.uses[cond.index()] != 1 {
                continue;
            }
            let ValueDef::Result(compare) = self.body.value_def(cond) else {
                continue;
            };
            if matches!(self.body.inst(compare), InstData::IntCompare { .. })
                && !self.skipped[compare.index()]
                && insts.contains(&compare)
            {
                self.fused[compare.index()] = true;
                self.skipped[compare.index()] = true;
            }
        }
    }

    /// Selects one instruction at the end of the current block.
    fn instruction(&mut self, inst: Inst) -> Result<(), Construct> {
        if self.skipped[inst.index()] {
            return Ok(());
        }
        let data = *self.body.inst(inst);
        let result = self.body.inst_result(inst);
        let opcode = data.opcode();
        match data {
            | InstData::Binary { ty, args, .. } => {
                let dst = self.result_reg(result);
                match opcode {
                    | Opcode::Iadd => self.alu(AluOp::Add, ty, dst, args, true),
                    | Opcode::Isub => self.alu(AluOp::Sub, ty, dst, args, false),
                    | Opcode::And => self.alu(AluOp::And, ty, dst, args, true),
                    | Opcode::Or => self.alu(AluOp::Or, ty, dst, args, true),
                    | Opcode::Xor => self.alu(AluOp::Xor, ty, dst, args, true),
                    | Opcode::Imul => self.multiply(ty, dst, args),
                    | Opcode::Sdiv => self.divide(true, false, ty, dst, args),
                    | Opcode::Udiv => self.divide(false, false, ty, dst, args),
                    | Opcode::Srem => self.divide(true, true, ty, dst, args),
                    | Opcode::Urem => self.divide(false, true, ty, dst, args),
                    | Opcode::Shl => self.shift(ShiftOp::Shl, ty, dst, args),
                    | Opcode::Lshr => self.shift(ShiftOp::Shr, ty, dst, args),
                    | Opcode::Ashr => self.shift(ShiftOp::Sar, ty, dst, args),
                    | Opcode::PtrAdd => self.alu(AluOp::Add, Type::I64, dst, args, true),
                    | Opcode::UaddOverflow => self.add_overflow(ty, dst, args),
                    | Opcode::UmulOverflow => self.mul_overflow(ty, dst, args),
                    | _ => return Err(Construct::Opcode(opcode)),
                }
            },
            | InstData::IntCompare { cond, ty, args } => {
                let dst = self.result_reg(result);
                let cond = self.compare(cond, ty, args);
                self.set_condition(cond, dst);
            },
            | InstData::Select { ty, args } => {
                let dst = self.result_reg(result);
                self.select(ty, dst, args);
            },
            | InstData::Unary { opcode, ty, arg } => {
                let dst = self.result_reg(result);
                self.unary(opcode, ty, dst, arg)?;
            },
            | InstData::Load { .. } | InstData::Store { .. } | InstData::MemoryRange { .. } =>
                self.memory(inst, data)?,
            | InstData::Call { .. } | InstData::CallIndirect { .. } => self.call(inst, data)?,
            | InstData::Nullary {
                opcode: Opcode::Trap,
                ..
            } => self.emit(MInst::Ud2),
            | InstData::Jump { .. }
            | InstData::Brif { .. }
            | InstData::Switch { .. }
            | InstData::Return { .. }
            | InstData::Unreachable => self.terminator(data)?,
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// The register an instruction's result was classified into.
    fn result_reg(&self, result: Option<Value>) -> Reg {
        match result.map(|value| self.kind(value)) {
            | Some(ValueKind::Reg(vreg)) => Reg::Virt(vreg),
            | _ => unreachable!("an instruction with code has a register result"),
        }
    }

    /// A fresh virtual register.
    pub(super) fn vreg(&mut self) -> Reg {
        Reg::Virt(self.func.vreg())
    }

    /// `dst ← src` as a 64-bit copy, hinting both sides together.
    pub(super) fn copy(&mut self, dst: Reg, src: Reg) {
        if let Reg::Virt(vreg) = dst {
            self.func.hint(vreg, src);
        }
        if let Reg::Virt(vreg) = src {
            self.func.hint(vreg, dst);
        }
        self.emit(MInst::Mov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        });
    }

    /// The value in a register, materializing a leaf into a fresh one.
    pub(super) fn reg(&mut self, value: Value) -> Reg {
        match self.kind(value) {
            | ValueKind::Reg(vreg) => Reg::Virt(vreg),
            | ValueKind::Const(bits) => {
                let dst = self.vreg();
                self.constant(dst, bits, self.ty(value));
                dst
            },
            | ValueKind::Addr(base, disp) => {
                let dst = self.vreg();
                self.address_into(dst, base, disp);
                dst
            },
            // Only reachable code is selected, and it uses only reachable
            // definitions; any register will do for the rest.
            | ValueKind::Unknown => self.vreg(),
        }
    }

    /// `dst ← bits` for a constant of type `ty`.
    pub(super) fn constant(&mut self, dst: Reg, bits: u64, ty: Type) {
        let signed = bits.cast_signed();
        if ty.bits() <= 32 || u32::try_from(bits).is_ok() {
            // A 32-bit move clears the upper half.
            self.emit(MInst::Mov {
                width: Width::B32,
                dst,
                src: Operand::Imm(low32(bits)),
            });
        } else if let Ok(imm) = i32::try_from(signed) {
            self.emit(MInst::Mov {
                width: Width::B64,
                dst,
                src: Operand::Imm(imm),
            });
        } else {
            self.emit(MInst::MovImm64 { dst, imm: signed });
        }
    }

    /// A source operand of an operation at `width`: an immediate when the
    /// value is a constant the instruction can encode.
    pub(super) fn operand(&mut self, value: Value, width: Width) -> Operand {
        match self.kind(value) {
            | ValueKind::Const(bits) => match immediate(bits, width) {
                | Some(imm) => Operand::Imm(imm),
                | None => Operand::Reg(self.reg(value)),
            },
            | _ => Operand::Reg(self.reg(value)),
        }
    }

    /// `dst = lhs op rhs` in two-address form, at least 32 bits wide.
    fn alu(&mut self, op: AluOp, ty: Type, dst: Reg, [lhs, rhs]: [Value; 2], commutes: bool) {
        let width = op_width(ty);
        let (lhs, rhs) = if commutes
            && matches!(self.kind(lhs), ValueKind::Const(_))
            && !matches!(self.kind(rhs), ValueKind::Const(_))
        {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };
        let lhs = self.reg(lhs);
        let rhs = self.operand(rhs, width);
        self.copy(dst, lhs);
        self.emit(MInst::Alu {
            op,
            width,
            dst,
            src: rhs,
        });
        if ty == Type::I1 && matches!(op, AluOp::Add | AluOp::Sub) {
            self.canonical_bool(dst);
        }
    }

    /// `dst = lhs * rhs`, with the three-operand form for a constant.
    fn multiply(&mut self, ty: Type, dst: Reg, [lhs, rhs]: [Value; 2]) {
        let width = op_width(ty);
        let (lhs, rhs) = if matches!(self.kind(lhs), ValueKind::Const(_)) {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };
        if let Operand::Imm(imm) = self.operand(rhs, width) {
            let src = self.reg(lhs);
            self.emit(MInst::Imul3 {
                width,
                dst,
                src,
                imm,
            });
            return;
        }
        self.alu(AluOp::Imul, ty, dst, [lhs, rhs], true);
    }

    /// Division and remainder through `%rdx:%rax`; narrow operands are
    /// extended to 32 bits first, by their signedness.
    fn divide(
        &mut self,
        signed: bool,
        remainder: bool,
        ty: Type,
        dst: Reg,
        [lhs, rhs]: [Value; 2],
    ) {
        let width = op_width(ty);
        let lhs = self.extended(lhs, signed);
        let divisor = self.extended(rhs, signed);
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rax),
            src:   Operand::Reg(lhs),
        });
        if signed {
            self.emit(MInst::SignExtendAx { width });
        } else {
            self.emit(MInst::Mov {
                width: Width::B32,
                dst:   Reg::Phys(PReg::Rdx),
                src:   Operand::Imm(0),
            });
        }
        self.emit(MInst::Div {
            signed,
            width,
            divisor,
        });
        let result = if remainder { PReg::Rdx } else { PReg::Rax };
        self.copy(dst, Reg::Phys(result));
    }

    /// The value in a register whose bits above its type are its sign
    /// (`signed`) or zero, up to 32 bits; wider values as they are.
    pub(super) fn extended(&mut self, value: Value, signed: bool) -> Reg {
        let ty = self.ty(value);
        if let ValueKind::Const(bits) = self.kind(value) {
            let bits = if signed { sign_extend(bits, ty) } else { bits };
            let dst = self.vreg();
            let wide = if ty.bits() < 32 { Type::I32 } else { ty };
            self.constant(dst, bits & mask(wide), wide);
            return dst;
        }
        let src = self.reg(value);
        match ty {
            | Type::I8 | Type::I16 => {
                let dst = self.vreg();
                self.emit(MInst::Extend {
                    signed,
                    from: width(ty),
                    to: Width::B32,
                    dst,
                    src: Operand::Reg(src),
                });
                dst
            },
            | _ => src,
        }
    }

    /// Shifts, with the count in `%cl` unless it is a constant. Right
    /// shifts of narrow values extend them first; a count at or above the
    /// width is poison, which the hardware's masked count refines.
    fn shift(&mut self, op: ShiftOp, ty: Type, dst: Reg, [value, amount]: [Value; 2]) {
        let width = op_width(ty);
        let src = match op {
            | ShiftOp::Shr if ty != Type::I1 => self.extended(value, false),
            | ShiftOp::Sar if ty != Type::I1 => self.extended(value, true),
            | _ => self.reg(value),
        };
        let amount = match self.kind(amount) {
            | ValueKind::Const(bits) => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "The count is masked to six bits."
                )]
                let count = (bits & u64::from(width.bits() - 1)) as u8;
                ShiftAmount::Imm(count)
            },
            | _ => {
                let count = self.reg(amount);
                self.emit(MInst::Mov {
                    width: Width::B64,
                    dst:   Reg::Phys(PReg::Rcx),
                    src:   Operand::Reg(count),
                });
                ShiftAmount::Cl
            },
        };
        self.copy(dst, src);
        self.emit(MInst::Shift {
            op,
            width,
            dst,
            amount,
        });
        if ty == Type::I1 {
            self.canonical_bool(dst);
        }
    }

    /// `uadd_overflow`: the carry of an addition at the operands' width.
    fn add_overflow(&mut self, ty: Type, dst: Reg, [lhs, rhs]: [Value; 2]) {
        let width = width(ty);
        let lhs = self.reg(lhs);
        let rhs = self.reg(rhs);
        let sum = self.vreg();
        self.copy(sum, lhs);
        self.emit(MInst::Alu {
            op: AluOp::Add,
            width,
            dst: sum,
            src: Operand::Reg(rhs),
        });
        self.set_condition(Cond::B, dst);
    }

    /// `umul_overflow`: whether the unsigned product's high half is
    /// nonzero.
    fn mul_overflow(&mut self, ty: Type, dst: Reg, [lhs, rhs]: [Value; 2]) {
        let width = width(ty);
        let lhs = self.reg(lhs);
        let rhs = self.reg(rhs);
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rax),
            src:   Operand::Reg(lhs),
        });
        self.emit(MInst::MulWide { width, src: rhs });
        self.set_condition(Cond::O, dst);
    }

    /// Emits the comparison of an `icmp` and returns the condition that
    /// holds when the `icmp` is true. Operands are materialized before the
    /// `cmp`, so nothing separates it from the flags' consumer.
    pub(super) fn compare(&mut self, cond: IntCC, ty: Type, [lhs, rhs]: [Value; 2]) -> Cond {
        let width = width(ty);
        let cond = int_cond(cond);
        let (lhs, rhs, cond) = if matches!(self.kind(lhs), ValueKind::Const(_)) {
            (rhs, lhs, cond.swapped())
        } else {
            (lhs, rhs, cond)
        };
        let lhs = self.reg(lhs);
        match self.operand(rhs, width) {
            | Operand::Imm(0) => self.emit(MInst::Test {
                width,
                lhs,
                rhs: lhs,
            }),
            | rhs => self.emit(MInst::Cmp { width, lhs, rhs }),
        }
        cond
    }

    /// The condition under which a selected `i1` value is true, emitting a
    /// fused compare or a test of the register.
    pub(super) fn condition(&mut self, value: Value) -> Cond {
        if let ValueDef::Result(inst) = self.body.value_def(value)
            && self.fused[inst.index()]
            && let InstData::IntCompare { cond, ty, args } = *self.body.inst(inst)
        {
            return self.compare(cond, ty, args);
        }
        let reg = self.reg(value);
        self.emit(MInst::Test {
            width: Width::B32,
            lhs:   reg,
            rhs:   reg,
        });
        Cond::Ne
    }

    /// `dst ← cond ? 1 : 0` as a canonical `i1`.
    fn set_condition(&mut self, cond: Cond, dst: Reg) {
        let byte = self.vreg();
        self.emit(MInst::Setcc { cond, dst: byte });
        self.emit(MInst::Extend {
            signed: false,
            from: Width::B8,
            to: Width::B32,
            dst,
            src: Operand::Reg(byte),
        });
    }

    /// `select` through `cmov`: both arms are in registers before the
    /// condition sets the flags.
    fn select(&mut self, ty: Type, dst: Reg, [cond, then, otherwise]: [Value; 3]) {
        let then = self.reg(then);
        let otherwise = self.reg(otherwise);
        let cond = self.condition(cond);
        self.copy(dst, otherwise);
        self.emit(MInst::Cmov {
            cond,
            width: op_width(ty),
            dst,
            src: then,
        });
    }

    /// Conversions and bit operations.
    fn unary(&mut self, opcode: Opcode, ty: Type, dst: Reg, arg: Value) -> Result<(), Construct> {
        let from = self.ty(arg);
        match opcode {
            | Opcode::Zext | Opcode::Inttoptr => {
                let src = self.reg(arg);
                let (from, to) = match from {
                    | Type::I1 | Type::I32 => (Width::B32, Width::B64),
                    | Type::I8 => (Width::B8, Width::B32),
                    | Type::I16 => (Width::B16, Width::B32),
                    | _ => (Width::B64, Width::B64),
                };
                if from == Width::B64 {
                    self.copy(dst, src);
                } else {
                    self.emit(MInst::Extend {
                        signed: false,
                        from,
                        to,
                        dst,
                        src: Operand::Reg(src),
                    });
                }
            },
            | Opcode::Sext => {
                let src = self.reg(arg);
                if from == Type::I1 {
                    self.copy(dst, src);
                    self.emit(MInst::Unary {
                        op: UnaryOp::Neg,
                        width: Width::B64,
                        dst,
                    });
                } else {
                    self.emit(MInst::Extend {
                        signed: true,
                        from: width(from),
                        to: if ty.bits() > 32 {
                            Width::B64
                        } else {
                            Width::B32
                        },
                        dst,
                        src: Operand::Reg(src),
                    });
                }
            },
            // Every other conversion to `i1`, and `freeze` of an operand in
            // unreachable code, keeps the low bit.
            | Opcode::Trunc | Opcode::Ptrtoint | Opcode::Bitcast | Opcode::Freeze => {
                let src = self.reg(arg);
                self.copy(dst, src);
                if ty == Type::I1 {
                    self.canonical_bool(dst);
                }
            },
            | Opcode::Ctz | Opcode::CtzDefined | Opcode::Clz | Opcode::ClzDefined =>
                self.bit_scan(opcode, ty, dst, arg),
            | Opcode::Popcount => self.popcount(ty, dst, arg),
            | Opcode::Bswap => {
                let src = self.reg(arg);
                self.copy(dst, src);
                if ty == Type::I16 {
                    self.emit(MInst::Shift {
                        op: ShiftOp::Rol,
                        width: Width::B16,
                        dst,
                        amount: ShiftAmount::Imm(8),
                    });
                } else {
                    self.emit(MInst::Unary {
                        op: UnaryOp::Bswap,
                        width: width(ty),
                        dst,
                    });
                }
            },
            | _ => return Err(Construct::Opcode(opcode)),
        }
        Ok(())
    }

    /// Bit counts through `bsf`/`bsr`, which leave their destination
    /// undefined and set ZF for a zero source. A leading-zero count is
    /// `(width - 1) ^ index` of the highest set bit; the defined forms
    /// replace a zero source's index with one that yields the width.
    fn bit_scan(&mut self, opcode: Opcode, ty: Type, dst: Reg, arg: Value) {
        let bits = ty.bits();
        let width = op_width(ty);
        let src = self.extended(arg, false);
        let reverse = matches!(opcode, Opcode::Clz | Opcode::ClzDefined);
        let defined = matches!(opcode, Opcode::CtzDefined | Opcode::ClzDefined);
        let zero_index = if reverse { 2 * bits - 1 } else { bits };
        let fallback = defined.then(|| {
            let fallback = self.vreg();
            self.constant(fallback, u64::from(zero_index), Type::I32);
            fallback
        });
        let index = if reverse { self.vreg() } else { dst };
        self.emit(MInst::BitScan {
            reverse,
            width,
            dst: index,
            src,
        });
        if let Some(fallback) = fallback {
            self.emit(MInst::Cmov {
                cond: Cond::E,
                width,
                dst: index,
                src: fallback,
            });
        }
        if reverse {
            self.copy(dst, index);
            self.emit(MInst::Alu {
                op: AluOp::Xor,
                width,
                dst,
                src: Operand::Imm(i32::try_from(bits - 1).expect("a width fits")),
            });
        }
    }

    /// Population count by the SWAR sequence, since x86-64's baseline has
    /// no `popcnt`: pairs, nibbles and bytes are summed in place, and a
    /// multiplication gathers the bytes into the top one.
    fn popcount(&mut self, ty: Type, dst: Reg, arg: Value) {
        let width = op_width(ty);
        let wide = width == Width::B64;
        let constant_ty = if wide { Type::I64 } else { Type::I32 };
        let pattern = |byte: u64| (byte * 0x0101_0101_0101_0101) & mask(constant_ty);
        let x = self.extended(arg, false);
        // x - ((x >> 1) & 0x55..)
        let half = self.vreg();
        self.copy(half, x);
        self.shift_imm(ShiftOp::Shr, width, half, 1);
        let fives = self.vreg();
        self.constant(fives, pattern(0x55), constant_ty);
        self.alu_reg(AluOp::And, width, half, fives);
        let pairs = self.vreg();
        self.copy(pairs, x);
        self.alu_reg(AluOp::Sub, width, pairs, half);
        // (x & 0x33..) + ((x >> 2) & 0x33..)
        let threes = self.vreg();
        self.constant(threes, pattern(0x33), constant_ty);
        let high = self.vreg();
        self.copy(high, pairs);
        self.shift_imm(ShiftOp::Shr, width, high, 2);
        self.alu_reg(AluOp::And, width, high, threes);
        let nibbles = self.vreg();
        self.copy(nibbles, pairs);
        self.alu_reg(AluOp::And, width, nibbles, threes);
        self.alu_reg(AluOp::Add, width, nibbles, high);
        // (x + (x >> 4)) & 0x0f..
        let upper = self.vreg();
        self.copy(upper, nibbles);
        self.shift_imm(ShiftOp::Shr, width, upper, 4);
        let bytes = self.vreg();
        self.copy(bytes, nibbles);
        self.alu_reg(AluOp::Add, width, bytes, upper);
        let fifteens = self.vreg();
        self.constant(fifteens, pattern(0x0F), constant_ty);
        self.alu_reg(AluOp::And, width, bytes, fifteens);
        // (x * 0x01..) >> (width - 8)
        let ones = self.vreg();
        self.constant(ones, pattern(0x01), constant_ty);
        self.copy(dst, bytes);
        self.alu_reg(AluOp::Imul, width, dst, ones);
        #[expect(clippy::cast_possible_truncation, reason = "The count is 24 or 56.")]
        let top = (width.bits() - 8) as u8;
        self.shift_imm(ShiftOp::Shr, width, dst, top);
    }

    fn shift_imm(&mut self, op: ShiftOp, width: Width, dst: Reg, count: u8) {
        self.emit(MInst::Shift {
            op,
            width,
            dst,
            amount: ShiftAmount::Imm(count),
        });
    }

    fn alu_reg(&mut self, op: AluOp, width: Width, dst: Reg, src: Reg) {
        self.emit(MInst::Alu {
            op,
            width,
            dst,
            src: Operand::Reg(src),
        });
    }

    /// Keeps only the low bit of an `i1` result.
    pub(super) fn canonical_bool(&mut self, dst: Reg) {
        self.emit(MInst::Alu {
            op: AluOp::And,
            width: Width::B32,
            dst,
            src: Operand::Imm(1),
        });
    }
}

/// A vector of `len` copies of `value` in `arena`.
fn filled<T: Clone>(arena: &Bump, len: usize, value: T) -> ArenaVec<'_, T> {
    let mut vec = ArenaVec::with_capacity_in(len, arena);
    vec.resize(len, value);
    vec
}

/// The width a value of `ty` occupies: `i1` as a canonical 32-bit value.
pub(super) const fn width(ty: Type) -> Width {
    match ty {
        | Type::I8 => Width::B8,
        | Type::I16 => Width::B16,
        | Type::I64 | Type::Ptr => Width::B64,
        | _ => Width::B32,
    }
}

/// The width an arithmetic operation on `ty` runs at: at least 32 bits,
/// whose low bits are the narrow result.
pub(super) const fn op_width(ty: Type) -> Width {
    match ty {
        | Type::I64 | Type::Ptr => Width::B64,
        | _ => Width::B32,
    }
}

/// The mask of a type's bits, as 64 bits.
pub(super) const fn mask(ty: Type) -> u64 {
    if ty.bits() >= 64 {
        u64::MAX
    } else {
        (1 << ty.bits()) - 1
    }
}

/// `bits` of type `ty` sign-extended to 64 bits.
pub(super) const fn sign_extend(bits: u64, ty: Type) -> u64 {
    let shift = 64 - ty.bits();
    if shift == 0 || shift >= 64 {
        bits
    } else {
        ((bits << shift).cast_signed() >> shift).cast_unsigned()
    }
}

/// The low 32 bits of `bits` as an immediate.
#[expect(
    clippy::cast_possible_truncation,
    reason = "A 32-bit instruction reads only the low 32 bits."
)]
pub(super) const fn low32(bits: u64) -> i32 {
    bits as u32 as i32
}

/// The immediate encoding a constant at `width`: any value at 32 bits or
/// less (the instruction reads the low bits), and a sign-extended 32-bit
/// value at 64 bits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "An 8- or 16-bit instruction reads only the low bits."
)]
pub(super) fn immediate(bits: u64, width: Width) -> Option<i32> {
    match width {
        | Width::B8 => Some(i32::from(bits as u8 as i8)),
        | Width::B16 => Some(i32::from(bits as u16 as i16)),
        | Width::B32 => Some(low32(bits)),
        | Width::B64 => i32::try_from(bits.cast_signed()).ok(),
    }
}

/// The machine condition of an integer comparison.
const fn int_cond(cond: IntCC) -> Cond {
    match cond {
        | IntCC::Eq => Cond::E,
        | IntCC::Ne => Cond::Ne,
        | IntCC::Slt => Cond::L,
        | IntCC::Sle => Cond::Le,
        | IntCC::Sgt => Cond::G,
        | IntCC::Sge => Cond::Ge,
        | IntCC::Ult => Cond::B,
        | IntCC::Ule => Cond::Be,
        | IntCC::Ugt => Cond::A,
        | IntCC::Uge => Cond::Ae,
    }
}

/// Evaluates a comparison of two constants of type `ty`.
fn compare(cond: IntCC, ty: Type, lhs: u64, rhs: u64) -> bool {
    let (slhs, srhs) = (
        sign_extend(lhs, ty).cast_signed(),
        sign_extend(rhs, ty).cast_signed(),
    );
    match cond {
        | IntCC::Eq => lhs == rhs,
        | IntCC::Ne => lhs != rhs,
        | IntCC::Slt => slhs < srhs,
        | IntCC::Sle => slhs <= srhs,
        | IntCC::Sgt => slhs > srhs,
        | IntCC::Sge => slhs >= srhs,
        | IntCC::Ult => lhs < rhs,
        | IntCC::Ule => lhs <= rhs,
        | IntCC::Ugt => lhs > rhs,
        | IntCC::Uge => lhs >= rhs,
    }
}
