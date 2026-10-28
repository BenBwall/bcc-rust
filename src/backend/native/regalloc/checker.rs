//! The symbolic allocation checker: an abstract interpretation of the
//! allocated function that proves it computes what the virtual-register
//! program computed, whatever the allocator did in between.
//!
//! The checker never trusts the allocator's own records. It sees the
//! virtual-register program as selection left it, the rewritten function,
//! and for each rewritten instruction its *origin*: the program instruction
//! it stands for, or none for a move the allocator inserted (a spill, a
//! reload, a split or a resolution move). Its abstract state is a set of
//! facts "location `L` holds the current value of `v`", where a location is
//! a physical register or a spill slot and a value is a virtual register's
//! or the one a physical register holds in the program (an argument, a
//! call's result, a clobber's leftover). Walking an instruction:
//!
//! - a program instruction must be the original with each register replaced by
//!   a physical one (precolored registers unchanged); every register it reads
//!   must hold the value the original read there, and each register it writes
//!   then holds only the written value, which no other location holds any more
//!   (the old value is stale);
//! - a program copy may instead be any whole-register move, load or store of a
//!   spill slot, or a copy of a spill slot to itself (a placeholder for no
//!   instruction at all, deleted after the check): its destination then holds
//!   everything the source held, plus the copy's destination value;
//! - an inserted instruction must be such a move; its destination holds what
//!   its source held;
//! - or an inserted `lea` that adds two registers, scaled: its destination then
//!   holds the *address* `b + i × s` for every value `b` its base held and `i`
//!   its index held. A program instruction whose indexed memory operand `d(b,
//!   i, s)` became `d(r)` must find that address in `r`. Writing a new value of
//!   `b` or `i` makes every address of the old one stale.
//!
//! At a join the facts of the predecessors intersect. The facts only
//! shrink, so the forward dataflow over the blocks reaches a fixed point;
//! a last pass then checks every instruction against the state before it.
//! The design is `regalloc2`'s checker (Cranelift, `regalloc2/src/checker.rs`)
//! applied to the final machine code rather than to the allocator's edit
//! list, so the rewrite is checked as well.
//!
//! Facts about a virtual register the program no longer reads are dropped:
//! at the start of each block those of values not live into it, and after
//! each program instruction those of the values it mentions for the last
//! time (the liveness of the program, not of the allocation). Every check
//! reads a live value, so the answers, and with them the verdict, stay the
//! same, and a wrong liveness could only make the checker reject more. The
//! state then holds facts about the live values and the physical
//! registers' own values alone, so an instruction costs time in proportion
//! to the live set rather than to every value spilled before it, and
//! straight-line code with a bounded live set checks in time linear in its
//! length. The liveness is the allocator's, whose sets hold only the live
//! registers, so a long function of many blocks pays for its live sets
//! rather than for blocks × virtual registers.

use std::fmt;

use super::liveness::{
    LiveSet,
    Liveness,
    transfer,
};
use crate::{
    backend::native::mir::{
        Base,
        FrameObject,
        Index,
        MBlock,
        MFunction,
        MInst,
        Mem,
        Operand,
        PReg,
        Reg,
        RegClass,
        Role,
        VReg,
        Width,
    },
    ir::Entity,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The origin of a rewritten instruction the allocator inserted.
pub(crate) const INSERTED: u32 = u32::MAX;

/// Checks that `func`, allocated and rewritten, computes what `original`
/// did: `original[b]` is block `b` of the virtual-register program and
/// `origins[b][i]` the index in it of rewritten instruction `i` of block
/// `b`, or [`INSERTED`]. On success, returns the number of facts in the
/// largest state the walk held, which bounds its work per instruction.
pub(crate) fn check<'a>(
    func: &MFunction<'_>,
    original: &[&[MInst]],
    origins: &[&[u32]],
    arena: &'a Bump,
) -> Result<usize, &'a CheckError> {
    if original.len() != func.blocks.len() || origins.len() != func.blocks.len() {
        return Err(arena.alloc(CheckError {
            block:     MBlock::new(0),
            index:     0,
            kind:      ErrorKind::Blocks,
            original:  None,
            rewritten: None,
        }));
    }
    let liveness = Liveness::of_program(func, original.iter().copied(), arena);
    let deaths = last_mentions(func, &liveness, original, arena);
    let mut checker = Checker {
        arena,
        func,
        original,
        origins,
        liveness,
        deaths,
        largest: 0,
        state: ArenaVec::new_in(arena),
        values: ArenaVec::new_in(arena),
        operands: ArenaVec::new_in(arena),
        pairs: ArenaVec::new_in(arena),
    };
    let blocks = func.blocks.len();
    let mut entry: ArenaVec<'_, Option<ArenaVec<'_, Fact>>> =
        ArenaVec::with_capacity_in(blocks, arena);
    entry.resize_with(blocks, || None);
    let mut start = ArenaVec::with_capacity_in(PReg::COUNT, arena);
    // At entry every register holds the program's value of itself.
    for register in PReg::ALL {
        start.push((Loc::Reg(register), Value::Phys(register)));
    }
    entry[0] = Some(start);
    let mut queued = ArenaVec::with_capacity_in(blocks, arena);
    queued.resize(blocks, false);
    let mut work = ArenaVec::new_in(arena);
    work.push(MBlock::new(0));
    queued[0] = true;
    while let Some(block) = work.pop() {
        queued[block.index()] = false;
        checker.state.clear();
        checker.state.extend_from_slice(
            entry[block.index()]
                .as_ref()
                .expect("queued blocks are reached"),
        );
        checker.block(block, false)?;
        for successor in func.successors(block) {
            let changed = match &mut entry[successor.index()] {
                | Some(facts) => meet(facts, &checker.state),
                | slot @ None => {
                    let mut facts = ArenaVec::with_capacity_in(checker.state.len(), arena);
                    facts.extend_from_slice(&checker.state);
                    *slot = Some(facts);
                    true
                },
            };
            if changed && !queued[successor.index()] {
                queued[successor.index()] = true;
                work.push(successor);
            }
        }
    }
    for (index, facts) in entry.iter().enumerate() {
        // A block no path reaches runs in no state; nothing can go wrong.
        let Some(facts) = facts else { continue };
        checker.state.clear();
        checker.state.extend_from_slice(facts);
        checker.block(MBlock::new(index), true)?;
    }
    Ok(checker.largest)
}

/// For each block of the program, the virtual registers each of its
/// instructions mentions while not live after it, as `(instruction,
/// register)` in instruction order: there the checker forgets them.
fn last_mentions<'a>(
    func: &MFunction<'_>,
    liveness: &Liveness<'_>,
    original: &[&[MInst]],
    arena: &'a Bump,
) -> ArenaVec<'a, &'a [(u32, VReg)]> {
    let mut live = LiveSet::new(func.vreg_count(), arena);
    let mut mentions = ArenaVec::new_in(arena);
    let mut starts = ArenaVec::with_capacity_in(original.len() + 1, arena);
    for (block, insts) in original.iter().enumerate() {
        let start = mentions.len();
        starts.push(start);
        liveness.live_out(func, block, &mut live);
        for (index, inst) in insts.iter().enumerate().rev() {
            let index = u32::try_from(index).expect("instruction numbers fit u32");
            let mut inst = *inst;
            inst.visit_registers(|reg, _| {
                if let Reg::Virt(vreg) = *reg
                    && !live.contains(vreg)
                {
                    mentions.push((index, vreg));
                }
            });
            transfer(&mut inst, &mut live);
        }
        mentions[start..].reverse();
    }
    starts.push(mentions.len());
    let mentions: &'a [(u32, VReg)] = mentions.leak();
    let mut blocks = ArenaVec::with_capacity_in(original.len(), arena);
    blocks.extend(
        starts
            .windows(2)
            .map(|bounds| &mentions[bounds[0]..bounds[1]]),
    );
    blocks
}

/// Why a rewritten function does not compute what its program did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CheckError {
    pub(crate) block:     MBlock,
    /// The rewritten instruction's index in its block.
    pub(crate) index:     usize,
    pub(crate) kind:      ErrorKind,
    /// The program instruction, when there is one.
    pub(crate) original:  Option<MInst>,
    pub(crate) rewritten: Option<MInst>,
}

/// What went wrong at one instruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ErrorKind {
    /// The rewritten function has another number of blocks.
    Blocks,
    /// The program's instructions are not all there, once each, in order.
    Order,
    /// An instruction the allocator inserted is not a whole-register move
    /// or the `lea` of an address.
    NotAMove,
    /// A program instruction became something other than itself with its
    /// registers replaced.
    Shape,
    /// An instruction reads a location that does not hold the value the
    /// program read there.
    Missing { value: Value, loc: Loc },
}

/// A value of the virtual-register program.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Value {
    /// What the program holds in a precolored register.
    Phys(PReg),
    Virt(VReg),
    /// The sum of the values of two program registers, the second scaled:
    /// an indexed address computed ahead of the instruction that uses it.
    Address {
        base:  Reg,
        index: Reg,
        scale: u8,
    },
}

/// A place the rewritten function keeps values in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[expect(
    variant_size_differences,
    reason = "Facts are short-lived pairs; packing them would obscure the matches."
)]
pub(crate) enum Loc {
    Reg(PReg),
    Spill(u32),
}

/// "The location holds the value", the unit of the abstract state.
type Fact = (Loc, Value);

/// The walk's working state and scratch buffers.
struct Checker<'f, 'a> {
    arena:    &'a Bump,
    func:     &'f MFunction<'f>,
    original: &'f [&'f [MInst]],
    origins:  &'f [&'f [u32]],
    /// The liveness of the program's virtual registers.
    liveness: Liveness<'a>,
    /// Each block's [`last_mentions`].
    deaths:   ArenaVec<'a, &'a [(u32, VReg)]>,
    /// The most facts the state has held.
    largest:  usize,
    /// The facts before the current instruction, sorted.
    state:    ArenaVec<'a, Fact>,
    /// Values copied by a move.
    values:   ArenaVec<'a, Value>,
    /// The rewritten instruction's registers and roles.
    operands: ArenaVec<'a, (Reg, Role)>,
    /// Program register, rewritten register and role, per operand.
    pairs:    ArenaVec<'a, (Reg, PReg, Role)>,
}

impl<'a> Checker<'_, 'a> {
    /// Walks `block` from the current state; with `checking`, an
    /// instruction reading a location without its value is an error.
    fn block(&mut self, block: MBlock, checking: bool) -> Result<(), &'a CheckError> {
        let insts = &self.func.blocks[block.index()].insts;
        let origins = self.origins[block.index()];
        let original = self.original[block.index()];
        let arena = self.arena;
        let error = |index: usize, kind, original| -> &'a CheckError {
            arena.alloc(CheckError {
                block,
                index,
                kind,
                original,
                rewritten: insts.get(index).copied(),
            })
        };
        if origins.len() != insts.len() {
            return Err(error(0, ErrorKind::Order, None));
        }
        let live_in = self.liveness.live_in(block.index());
        self.state.retain(|&(_, value)| is_live(value, live_in));
        let mut deaths = self.deaths[block.index()];
        let mut next = 0;
        for (index, (&inst, &origin)) in insts.iter().zip(origins).enumerate() {
            self.largest = self.largest.max(self.state.len());
            if origin == INSERTED {
                if let Some((dst, base, index, scale)) = as_address(inst) {
                    self.address(dst, base, index, scale);
                    continue;
                }
                let (dst, src) =
                    as_move(inst).ok_or_else(|| error(index, ErrorKind::NotAMove, None))?;
                self.copy(dst, src, None);
                continue;
            }
            if origin as usize != next || next >= original.len() {
                return Err(error(index, ErrorKind::Order, None));
            }
            next += 1;
            let program = original[origin as usize];
            self.instruction(program, inst, checking)
                .map_err(|kind| error(index, kind, Some(program)))?;
            let dying = deaths.partition_point(|&(at, _)| at <= origin);
            if dying > 0 {
                let dead = &deaths[..dying];
                self.state.retain(|&(_, value)| {
                    !dead
                        .iter()
                        .any(|&(_, vreg)| depends_on(value, Value::Virt(vreg)))
                });
                deaths = &deaths[dying..];
            }
        }
        if next != original.len() {
            return Err(error(insts.len(), ErrorKind::Order, None));
        }
        Ok(())
    }

    /// Checks one program instruction against its rewritten form and
    /// applies its effect.
    fn instruction(
        &mut self,
        original: MInst,
        rewritten: MInst,
        checking: bool,
    ) -> Result<(), ErrorKind> {
        let shape = ErrorKind::Shape;
        if let Some((dst, src)) = register_copy(original) {
            let (dst_loc, src_loc) = as_move(rewritten).ok_or(shape)?;
            if !self.placed(dst, dst_loc) || !self.placed(src, src_loc) {
                return Err(shape);
            }
            let value = value(src);
            if checking && !self.holds(src_loc, value) {
                return Err(ErrorKind::Missing {
                    value,
                    loc: src_loc,
                });
            }
            self.copy(dst_loc, src_loc, Some(self::value(dst)));
            return Ok(());
        }
        // A value read straight from its spill slot.
        let mut original = original;
        let mut copy = rewritten;
        if let (Some(&mut Operand::Reg(reg @ Reg::Virt(_))), Some(&mut Operand::Mem(mem))) =
            (original.memory_source(), copy.memory_source())
            && let Some(loc) = spill(mem)
        {
            if checking && !self.holds(loc, value(reg)) {
                return Err(ErrorKind::Missing {
                    value: value(reg),
                    loc,
                });
            }
            *original.memory_source().expect("matched above") = Operand::Mem(mem);
        }
        // An indexed address computed into a register first: proven here,
        // then left out of both sides of the comparison below.
        if let (
            Some(&mut Mem {
                base: Base::Reg(base),
                index: Some(Index { reg: index, scale }),
                disp,
            }),
            Some(&mut Mem {
                base: Base::Reg(Reg::Phys(register)),
                index: None,
                disp: rewritten_disp,
            }),
        ) = (original.memory_operand(), copy.memory_operand())
            && disp == rewritten_disp
        {
            let (value, loc) = (Value::Address { base, index, scale }, Loc::Reg(register));
            if checking && !self.holds(loc, value) {
                return Err(ErrorKind::Missing { value, loc });
            }
            *original.memory_operand().expect("matched above") = PROVEN_ADDRESS;
            *copy.memory_operand().expect("matched above") = PROVEN_ADDRESS;
        }
        // The original with the rewritten registers put in must be the
        // rewritten instruction.
        self.operands.clear();
        copy.visit_registers(|reg, role| self.operands.push((*reg, role)));
        self.pairs.clear();
        let mut expected = original;
        let mut position = 0;
        let mut matches = true;
        expected.visit_registers(|reg, role| {
            match self.operands.get(position) {
                | Some(&(Reg::Phys(register), rewritten_role)) if rewritten_role == role => {
                    self.pairs.push((*reg, register, role));
                    *reg = Reg::Phys(register);
                },
                | _ => matches = false,
            }
            position += 1;
        });
        if !matches || position != self.operands.len() || expected != copy {
            return Err(shape);
        }
        for index in 0..self.pairs.len() {
            let (reg, register, _) = self.pairs[index];
            if !self.placed(reg, Loc::Reg(register)) {
                return Err(shape);
            }
        }
        if checking {
            for &(reg, register, role) in &self.pairs {
                let (value, loc) = (value(reg), Loc::Reg(register));
                if role.reads() && !self.holds(loc, value) {
                    return Err(ErrorKind::Missing { value, loc });
                }
            }
        }
        for index in 0..self.pairs.len() {
            let (reg, register, role) = self.pairs[index];
            if role.writes() {
                self.define(Loc::Reg(register), value(reg));
            }
        }
        Ok(())
    }

    /// Whether a program register may live in `loc`: a precolored register
    /// only in itself, a virtual one in a register of its class or a spill
    /// slot.
    fn placed(&self, reg: Reg, loc: Loc) -> bool {
        match (reg, loc) {
            | (Reg::Phys(register), Loc::Reg(other)) => register == other,
            | (Reg::Phys(_), Loc::Spill(_)) => false,
            | (Reg::Virt(vreg), Loc::Reg(register)) =>
                register.class() == self.func.classes[vreg.index()]
                    && !matches!(register, PReg::Rsp),
            | (Reg::Virt(_), Loc::Spill(_)) => true,
        }
    }

    fn holds(&self, loc: Loc, value: Value) -> bool {
        self.state.binary_search(&(loc, value)).is_ok()
    }

    /// The facts about `loc`.
    fn range(&self, loc: Loc) -> std::ops::Range<usize> {
        let start = self.state.partition_point(|&(other, _)| other < loc);
        let end = self.state.partition_point(|&(other, _)| other <= loc);
        start..end
    }

    /// `loc` now holds `value` and nothing else, and no other location
    /// holds `value` or an address computed from its old value.
    fn define(&mut self, loc: Loc, value: Value) {
        self.state
            .retain(|&(other_loc, other)| other_loc != loc && !depends_on(other, value));
        let at = self.state.partition_point(|&fact| fact < (loc, value));
        self.state.insert(at, (loc, value));
    }

    /// `dst` now holds what `src` holds, and the program value `defined`
    /// if a program copy defines one.
    fn copy(&mut self, dst: Loc, src: Loc, defined: Option<Value>) {
        self.values.clear();
        let range = self.range(src);
        self.values
            .extend(self.state[range].iter().map(|&(_, value)| value));
        if let Some(value) = defined {
            self.state.retain(|&(_, other)| !depends_on(other, value));
            self.values.retain(|&other| !depends_on(other, value));
            let at = self.values.partition_point(|&other| other < value);
            self.values.insert(at, value);
        }
        self.replace(dst);
    }

    /// `dst` now holds `base + index × scale` for each pair of a value
    /// `base` holds and a value `index` holds, and nothing else.
    fn address(&mut self, dst: Loc, base: Loc, index: Loc, scale: u8) {
        self.values.clear();
        let (bases, indexes) = (self.range(base), self.range(index));
        for &(_, base) in &self.state[bases] {
            for &(_, index) in &self.state[indexes.clone()] {
                if let (Some(base), Some(index)) = (register(base), register(index)) {
                    self.values.push(Value::Address { base, index, scale });
                }
            }
        }
        self.values.sort_unstable();
        self.replace(dst);
    }

    /// `dst` now holds exactly the sorted `values`.
    fn replace(&mut self, dst: Loc) {
        let range = self.range(dst);
        let at = range.start;
        drop(self.state.drain(range));
        for (offset, &value) in self.values.iter().enumerate() {
            self.state.insert(at + offset, (dst, value));
        }
    }
}

/// Intersects `facts` with `other` in place; whether `facts` changed.
fn meet(facts: &mut ArenaVec<'_, Fact>, other: &[Fact]) -> bool {
    let before = facts.len();
    let mut kept = 0;
    let mut cursor = 0;
    for index in 0..facts.len() {
        let fact = facts[index];
        while cursor < other.len() && other[cursor] < fact {
            cursor += 1;
        }
        if other.get(cursor) == Some(&fact) {
            facts[kept] = fact;
            kept += 1;
        }
    }
    facts.truncate(kept);
    kept != before
}

/// The program value a register operand names.
const fn value(reg: Reg) -> Value {
    match reg {
        | Reg::Phys(register) => Value::Phys(register),
        | Reg::Virt(vreg) => Value::Virt(vreg),
    }
}

/// The program register whose value `value` is; an address is none.
const fn register(value: Value) -> Option<Reg> {
    match value {
        | Value::Phys(register) => Some(Reg::Phys(register)),
        | Value::Virt(vreg) => Some(Reg::Virt(vreg)),
        | Value::Address { .. } => None,
    }
}

/// Whether the program may still read `value`, or the registers an address
/// is computed from, where the virtual registers in the sorted `live` are
/// live.
fn is_live(value: Value, live: &[VReg]) -> bool {
    let live_reg = |reg: Reg| match reg {
        | Reg::Virt(vreg) => live.binary_search(&vreg).is_ok(),
        | Reg::Phys(_) => true,
    };
    match value {
        | Value::Virt(vreg) => live.binary_search(&vreg).is_ok(),
        | Value::Phys(_) => true,
        | Value::Address { base, index, .. } => live_reg(base) && live_reg(index),
    }
}

/// Whether `fact` is `value`, or an address computed from it, so that a new
/// `value` makes it stale.
fn depends_on(fact: Value, value: Value) -> bool {
    match fact {
        | Value::Address { base, index, .. } =>
            self::value(base) == value || self::value(index) == value,
        | _ => fact == value,
    }
}

/// What an indexed memory operand becomes on both sides of the comparison
/// once the register holding its address is proven: no register, so the
/// rest of the instruction is compared alone.
const PROVEN_ADDRESS: Mem = Mem::new(Base::Constant(u32::MAX), 0);

/// The destination, base, index and scale of an inserted `lea` that adds
/// two general registers, the second scaled, with no displacement: an
/// address the rewrite computed for an instruction reading more spilled
/// registers than there are scratch registers.
fn as_address(inst: MInst) -> Option<(Loc, Loc, Loc, u8)> {
    let MInst::Lea {
        dst: Reg::Phys(dst),
        addr:
            Mem {
                base: Base::Reg(Reg::Phys(base)),
                index:
                    Some(Index {
                        reg: Reg::Phys(index),
                        scale,
                    }),
                disp: 0,
            },
    } = inst
    else {
        return None;
    };
    [dst, base, index]
        .iter()
        .all(|register| register.class() == RegClass::Gpr)
        .then_some((Loc::Reg(dst), Loc::Reg(base), Loc::Reg(index), scale))
}

/// The destination and source of a program copy: a whole-register move
/// between two registers.
const fn register_copy(inst: MInst) -> Option<(Reg, Reg)> {
    match inst {
        | MInst::Mov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        }
        | MInst::XMov {
            width: Width::B64,
            dst,
            src: Operand::Reg(src),
        } => Some((dst, src)),
        | _ => None,
    }
}

/// The destination and source of a rewritten whole-register move: between
/// two registers of one class, between a register and a spill slot, or from
/// a spill slot to itself.
fn as_move(inst: MInst) -> Option<(Loc, Loc)> {
    let (class, dst, src) = match inst {
        | MInst::Mov {
            width: Width::B64,
            dst: Reg::Phys(dst),
            src,
        } => (RegClass::Gpr, Loc::Reg(dst), source(src)?),
        | MInst::XMov {
            width: Width::B64,
            dst: Reg::Phys(dst),
            src,
        } => (RegClass::Xmm, Loc::Reg(dst), source(src)?),
        | MInst::Store {
            width: Width::B64,
            dst,
            src: Operand::Reg(Reg::Phys(src)),
        } => (RegClass::Gpr, spill(dst)?, Loc::Reg(src)),
        | MInst::XStore {
            width: Width::B64,
            dst,
            src: Reg::Phys(src),
        } => (RegClass::Xmm, spill(dst)?, Loc::Reg(src)),
        // Of either class: no register takes part.
        | MInst::Store {
            width: Width::B64,
            dst,
            src: Operand::Mem(src),
        } => (RegClass::Gpr, spill(dst)?, spill(src)?),
        | _ => return None,
    };
    // `%rsp` may be a side of a program copy (`stack_save`); `placed`
    // keeps virtual registers out of it.
    let fits = |loc| match loc {
        | Loc::Reg(register) => register.class() == class,
        | Loc::Spill(_) => true,
    };
    let between_slots = matches!((dst, src), (Loc::Spill(dst), Loc::Spill(src)) if dst != src);
    (fits(dst) && fits(src) && !between_slots).then_some((dst, src))
}

/// The location a move's source operand reads.
fn source(operand: Operand) -> Option<Loc> {
    match operand {
        | Operand::Reg(Reg::Phys(register)) => Some(Loc::Reg(register)),
        | Operand::Mem(mem) => spill(mem),
        | _ => None,
    }
}

/// The spill slot a memory operand names exactly.
const fn spill(mem: Mem) -> Option<Loc> {
    match mem {
        | Mem {
            base: Base::Frame(FrameObject::Spill(slot)),
            index: None,
            disp: 0,
        } => Some(Loc::Spill(slot)),
        | _ => None,
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}, instruction {}: ", self.block, self.index)?;
        match self.kind {
            | ErrorKind::Blocks => write!(f, "the blocks changed")?,
            | ErrorKind::Order => write!(
                f,
                "the program's instructions are not all present once, in order"
            )?,
            | ErrorKind::NotAMove =>
                write!(f, "an inserted instruction is not a whole-register move")?,
            | ErrorKind::Shape => write!(
                f,
                "the instruction is not the program's with registers replaced"
            )?,
            | ErrorKind::Missing { value, loc } => write!(f, "{loc:?} does not hold {value:?}")?,
        }
        if let Some(original) = self.original {
            write!(f, "; program: {original:?}")?;
        }
        if let Some(rewritten) = self.rewritten {
            write!(f, "; rewritten: {rewritten:?}")?;
        }
        Ok(())
    }
}
