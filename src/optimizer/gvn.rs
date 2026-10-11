//! Global value numbering over the dominator tree.
//!
//! The pass walks the dominator tree depth first, on an explicit stack, with
//! a scoped hash table from an instruction's *key* to the first instruction
//! with that key. Entering a block opens a scope and leaving it drops every
//! entry the block added, so the table holds exactly the instructions of the
//! block's dominators and of the block above the current point. An
//! instruction whose key is in the table computes the value of a dominating
//! instruction: its uses are redirected there and it is removed. This is the
//! scoped elaboration of Cranelift's aegraph pass without e-classes, and the
//! "dominator-based value numbering" of Briggs, Cooper and Simpson
//! ("Value Numbering", 1997).
//!
//! In a diamond, an expression computed in the entry is reused by both arms
//! and by the join; one computed in an arm is reused by nothing outside that
//! arm, since the other arm and the join are not dominated by it:
//!
//! ```text
//! block0:  v2 = iadd v0, v1    ; first: in the table for every block
//! block1:  v3 = iadd v0, v1    ; replaced by v2
//!          v4 = imul v0, v1    ; first in block1's scope only
//! block2:  v5 = imul v0, v1    ; stays: block1 does not dominate block2
//! ```
//!
//! The key is the opcode, the type, the condition of a comparison, the
//! operands after replacement and the immediate (constant bits, slot, global
//! or function). The operands of a commutative operation (`iadd`, `imul`,
//! `and`, `or`, `xor`, `fadd`, `fmul`, `icmp eq`/`ne`, and the symmetric
//! `fcmp` conditions) are put in value order; an ordered comparison with its
//! operands out of order is swapped (`icmp sgt a, b` is `icmp slt b, a`). A
//! constant folded by an earlier pass has the key of the constant.
//!
//! Only pure instructions have a key: arithmetic, division included,
//! comparisons, conversions, `fneg`, `select`, constants, `poison`, `null`,
//! the address-of instructions and `ptr_add`. A division with a dominating
//! twin cannot trap where the twin did not. Loads, stores, calls, the `va_*`
//! operations and `freeze` have none. LLVM's `freeze` returns "an arbitrary,
//! but fixed, value" for poison, and different `freeze` instructions of one
//! poison may return different values (LLVM Language Reference, `freeze`).
//! Merging two of them would pick equal values, one of the allowed outcomes,
//! so it would be a refinement; the pass still leaves them alone, because the
//! point of a `freeze` is a value the program chose once, and keeping each is
//! the conservative reading.
//!
//! The poison-generating flags (`nsw`, `nuw`, `exact`, `inbounds`) are not
//! part of the key. When an instruction is replaced by a dominating twin, the
//! twin keeps only the flags both had, as LLVM's GVN does with
//! `andIRFlags`: `add nsw x, y` followed by `add x, y` leaves one `add x, y`.
//! The surviving result is then poison in no case where either original was
//! not, so both sets of uses see a refinement of what they saw before.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (a conforming
//! implementation need not evaluate an expression whose value it already has,
//! and may change anything that is not observable).

use rustc_hash::FxBuildHasher;

use super::{
    draft::{
        Constant,
        Draft,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        DominatorTree,
        Entity,
        FloatCC,
        FuncId,
        GlobalId,
        Inst,
        InstData,
        IntCC,
        Opcode,
        StackSlot,
        Type,
        Value,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Numbers the values of one function. Returns whether it replaced anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let scratch = draft.scratch;
    let cfg = draft.control_flow_graph();
    let tree = DominatorTree::from_cfg(&cfg, scratch);
    let children = DominatorChildren::compute(&tree, draft.blocks.len(), scratch);
    let mut table: ArenaMap<'_, Key, Inst> = ArenaMap::with_hasher_in(FxBuildHasher, scratch);
    // The keys added since each open scope began, newest last.
    let mut added: ArenaVec<'_, Key> = ArenaVec::new_in(scratch);
    let mut stack = ArenaVec::new_in(scratch);
    if let Some(&entry) = tree.reverse_post_order().first() {
        stack.push(Visit::Enter(entry));
    }
    let mut changed = false;
    while let Some(visit) = stack.pop() {
        match visit {
            | Visit::Enter(block) => {
                stack.push(Visit::Leave(added.len()));
                for &child in children.of(block).iter().rev() {
                    stack.push(Visit::Enter(child));
                }
                changed |= number_block(draft, block, &mut table, &mut added, report);
            },
            | Visit::Leave(mark) =>
                for key in added.drain(mark..) {
                    _ = table.remove(&key);
                },
        }
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// A step of the walk over the dominator tree.
#[derive(Clone, Copy, Debug)]
enum Visit {
    /// Number a block's instructions, then visit its children.
    Enter(Block),
    /// Close a scope: drop the keys added after the first `usize`.
    Leave(usize),
}

/// Replaces each instruction of `block` that has a twin in `table`, and adds
/// the others. Returns whether it replaced any.
fn number_block(
    draft: &mut Draft<'_, '_>,
    block: Block,
    table: &mut ArenaMap<'_, Key, Inst>,
    added: &mut ArenaVec<'_, Key>,
    report: &mut OptimizationReport,
) -> bool {
    let mut changed = false;
    for index in 0..draft.block(block).insts.len() {
        let inst = draft.block(block).insts[index];
        if draft.is_removed(inst) {
            continue;
        }
        let Some(key) = key(draft, inst) else {
            continue;
        };
        let Some(&twin) = table.get(&key) else {
            _ = table.insert(key, inst);
            added.push(key);
            continue;
        };
        if !report.allow() {
            continue;
        }
        let twin_flags = draft.inst(twin).flags();
        let common = twin_flags & draft.inst(inst).flags();
        if common != twin_flags {
            draft.set_flags(twin, common);
        }
        let result = draft
            .inst_result(inst)
            .expect("a keyed instruction has a result");
        let twin_result = draft
            .inst_result(twin)
            .expect("a keyed instruction has a result");
        draft.replace(result, twin_result);
        draft.remove_inst(inst);
        changed = true;
    }
    changed
}

/// What makes two pure instructions compute the same value; see the module
/// docs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Binary {
        opcode: Opcode,
        ty:     Type,
        args:   [Value; 2],
    },
    Unary {
        opcode: Opcode,
        ty:     Type,
        arg:    Value,
    },
    IntCompare {
        cond: IntCC,
        ty:   Type,
        args: [Value; 2],
    },
    FloatCompare {
        cond: FloatCC,
        ty:   Type,
        args: [Value; 2],
    },
    Select {
        ty:   Type,
        args: [Value; 3],
    },
    /// `iconst` or `fconst` with its bits.
    Const {
        opcode: Opcode,
        ty:     Type,
        bits:   u128,
    },
    Poison(Type),
    Null,
    StackAddr(StackSlot),
    GlobalAddr(GlobalId),
    FuncAddr(FuncId),
}

/// The key of `inst`, or `None` if it is not pure or has no result.
fn key(draft: &Draft<'_, '_>, inst: Inst) -> Option<Key> {
    _ = draft.inst_result(inst)?;
    if let Some(constant) = draft.inst_constant(inst) {
        return Some(match constant {
            | Constant::Int(ty, bits) => Key::Const {
                opcode: Opcode::Iconst,
                ty,
                bits,
            },
            | Constant::Poison(ty) => Key::Poison(ty),
        });
    }
    let data = *draft.inst(inst);
    if !is_pure(&data) {
        return None;
    }
    let resolve = |value| draft.resolve(value);
    Some(match data {
        | InstData::Binary {
            opcode,
            ty,
            args: [a, b],
            ..
        } => {
            let (a, b) = (resolve(a), resolve(b));
            let args = if is_commutative(opcode) && b < a {
                [b, a]
            } else {
                [a, b]
            };
            Key::Binary { opcode, ty, args }
        },
        | InstData::Unary { opcode, ty, arg } => Key::Unary {
            opcode,
            ty,
            arg: resolve(arg),
        },
        | InstData::IntCompare {
            cond,
            ty,
            args: [a, b],
        } => {
            let (a, b) = (resolve(a), resolve(b));
            if b < a {
                Key::IntCompare {
                    cond: swapped_int(cond),
                    ty,
                    args: [b, a],
                }
            } else {
                Key::IntCompare {
                    cond,
                    ty,
                    args: [a, b],
                }
            }
        },
        | InstData::FloatCompare {
            cond,
            ty,
            args: [a, b],
        } => {
            let (a, b) = (resolve(a), resolve(b));
            if b < a {
                Key::FloatCompare {
                    cond: swapped_float(cond),
                    ty,
                    args: [b, a],
                }
            } else {
                Key::FloatCompare {
                    cond,
                    ty,
                    args: [a, b],
                }
            }
        },
        | InstData::Select { ty, args } => Key::Select {
            ty,
            args: args.map(resolve),
        },
        | InstData::Const { opcode, ty, .. } | InstData::WideConst { opcode, ty, .. } =>
            Key::Const {
                opcode,
                ty,
                bits: draft.body.const_bits(&data)?,
            },
        | InstData::Nullary {
            opcode: Opcode::Null,
            ..
        } => Key::Null,
        | InstData::Nullary { ty, .. } => Key::Poison(ty),
        | InstData::StackAddr { slot } => Key::StackAddr(slot),
        | InstData::GlobalAddr { global } => Key::GlobalAddr(global),
        | InstData::FuncAddr { func } => Key::FuncAddr(func),
        | _ => return None,
    })
}

/// Whether an instruction only computes its result from its operands: it
/// reads and writes no memory, has no other effect, and two copies with
/// equal operands give equal results. Division is pure in this sense even
/// though it can trap; code motion that may execute it where it did not run
/// must check the divisor too. `freeze` is not; see the module docs.
pub(super) fn is_pure(data: &InstData) -> bool {
    match *data {
        | InstData::Binary { opcode, .. } => opcode != Opcode::VaCopy,
        | InstData::Unary { opcode, .. } => opcode == Opcode::Fneg || opcode.is_conversion(),
        | InstData::IntCompare { .. }
        | InstData::FloatCompare { .. }
        | InstData::Select { .. }
        | InstData::Const { .. }
        | InstData::WideConst { .. }
        | InstData::Nullary { .. }
        | InstData::StackAddr { .. }
        | InstData::GlobalAddr { .. }
        | InstData::FuncAddr { .. } => true,
        | _ => false,
    }
}

/// Whether swapping the operands of `opcode` keeps its result. Floating
/// addition and multiplication are commutative in IEEE 754, NaNs included.
fn is_commutative(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iadd
            | Opcode::Imul
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::Fadd
            | Opcode::Fmul
    )
}

/// The condition that gives the same result with the operands swapped.
fn swapped_int(cond: IntCC) -> IntCC {
    match cond {
        | IntCC::Eq | IntCC::Ne => cond,
        | IntCC::Slt => IntCC::Sgt,
        | IntCC::Sle => IntCC::Sge,
        | IntCC::Sgt => IntCC::Slt,
        | IntCC::Sge => IntCC::Sle,
        | IntCC::Ult => IntCC::Ugt,
        | IntCC::Ule => IntCC::Uge,
        | IntCC::Ugt => IntCC::Ult,
        | IntCC::Uge => IntCC::Ule,
    }
}

/// The floating condition that gives the same result with the operands
/// swapped.
fn swapped_float(cond: FloatCC) -> FloatCC {
    match cond {
        | FloatCC::Oeq
        | FloatCC::One
        | FloatCC::Ord
        | FloatCC::Ueq
        | FloatCC::Une
        | FloatCC::Uno => cond,
        | FloatCC::Olt => FloatCC::Ogt,
        | FloatCC::Ole => FloatCC::Oge,
        | FloatCC::Ogt => FloatCC::Olt,
        | FloatCC::Oge => FloatCC::Ole,
        | FloatCC::Ult => FloatCC::Ugt,
        | FloatCC::Ule => FloatCC::Uge,
        | FloatCC::Ugt => FloatCC::Ult,
        | FloatCC::Uge => FloatCC::Ule,
    }
}

/// The children of every block in the dominator tree, in reverse
/// post-order, as rows of one array.
struct DominatorChildren<'s> {
    starts: ArenaVec<'s, u32>,
    rows:   ArenaVec<'s, Block>,
}

impl<'s> DominatorChildren<'s> {
    fn compute(tree: &DominatorTree<'_>, blocks: usize, scratch: &'s Bump) -> Self {
        let mut starts = ArenaVec::with_capacity_in(blocks + 1, scratch);
        starts.resize(blocks + 1, 0_u32);
        for &block in tree.reverse_post_order() {
            if let Some(parent) = tree.idom(block) {
                starts[parent.index() + 1] += 1;
            }
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut cursors = ArenaVec::with_capacity_in(blocks, scratch);
        cursors.extend_from_slice(&starts[..blocks]);
        let mut rows = ArenaVec::with_capacity_in(starts[blocks] as usize, scratch);
        rows.resize(starts[blocks] as usize, Block::new(0));
        for &block in tree.reverse_post_order() {
            if let Some(parent) = tree.idom(block) {
                let cursor = &mut cursors[parent.index()];
                rows[*cursor as usize] = block;
                *cursor += 1;
            }
        }
        Self { starts, rows }
    }

    fn of(&self, block: Block) -> &[Block] {
        let index = block.index();
        &self.rows[self.starts[index] as usize..self.starts[index + 1] as usize]
    }
}
