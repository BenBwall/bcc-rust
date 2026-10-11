//! Loop-invariant code motion.
//!
//! The pass finds the natural loops ([`LoopForest`]) and visits them inner
//! loops first. In each loop it walks the blocks in reverse post-order and
//! moves an instruction to the loop's *preheader* when the instruction may
//! run speculatively and every operand is defined outside the loop. Moving
//! an instruction makes its result defined outside the loop, so a chain of
//! invariant instructions moves in one walk; moving an inner loop's
//! instructions into its preheader, which lies in the enclosing loop, lets
//! the enclosing loop's walk move them further out.
//!
//! The preheader is a block outside the loop whose only successor is the
//! header and through which every entry into the loop passes. When the
//! header has a single predecessor outside the loop and that block ends in a
//! `jump`, it is the preheader. Otherwise the pass adds one: a block with a
//! parameter for each of the header's, which passes them on in a `jump` to
//! the header, and redirects every edge into the header from outside the
//! loop to it with the same arguments. Back edges still go to the header.
//! The preheader is made only when the first instruction moves.
//!
//! ```text
//! block0(v0: i32, v1: i32):           block0(v0: i32, v1: i32):
//!     v2 = iconst.i32 0                   v2 = iconst.i32 0
//!     jump block1(v2)                     v4 = imul.i32 v0, v1
//! block1(v3: i32):                        jump block1(v2)
//!     v4 = imul.i32 v0, v1            block1(v3: i32):
//!     v5 = iadd.i32 v3, v4                v5 = iadd.i32 v3, v4
//!     v6 = icmp.i32 slt v5, v1            ...
//!     brif v6, block1(v5), block2
//! ```
//!
//! Why the move is sound: an operand defined outside the loop dominates its
//! use inside it, so it dominates the header and, since every path into the
//! header passes the preheader first, the preheader too. The moved
//! instruction then runs once per entry into the loop, also when the loop
//! body would not have run it. That is harmless for a pure instruction that
//! cannot trap: a violated `nsw`, `nuw`, `exact` or `inbounds` gives poison,
//! which is undefined behaviour only where it is used, and the uses stay
//! where they were. Division and remainder trap on a zero divisor and, when
//! signed, on the most negative dividend divided by -1, so they move only
//! when the divisor is a constant other than 0 and, for a signed division,
//! other than -1. Loads, stores, calls and the `va_*` operations never move,
//! and neither does `freeze` (see `gvn.rs`).
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26 (only observable
//! behaviour must be kept); §6.5.5 paragraph 5, p. 82; PDF p. 94 (division
//! by zero is undefined, so a division must not run where it did not).

use super::{
    draft::{
        Constant,
        Draft,
        Edge,
        Term,
    },
    gvn::is_pure,
    loops::{
        Loop,
        LoopForest,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        DominatorTree,
        Entity,
        Inst,
        InstData,
        Opcode,
    },
    util::bump::ArenaVec,
};

/// Hoists the loop invariants of one function. Returns whether it moved
/// anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let scratch = draft.scratch;
    let cfg = draft.control_flow_graph();
    let tree = DominatorTree::from_cfg(&cfg, scratch);
    let forest = LoopForest::compute(&cfg, &tree, scratch);
    if forest.is_empty() {
        return false;
    }
    let mut state = Hoisting {
        forest,
        order: ArenaVec::new_in(scratch),
        defined_in: ArenaVec::new_in(scratch),
    };
    state.order.extend_from_slice(tree.reverse_post_order());
    state.defined_in.resize(draft.value_count(), None);
    for block in draft.alive_blocks() {
        let data = draft.block(block);
        for &param in &data.params {
            state.defined_in[param.index()] = Some(block);
        }
        for &inst in &data.insts {
            if let Some(result) = draft.inst_result(inst) {
                state.defined_in[result.index()] = Some(block);
            }
        }
    }
    let mut changed = false;
    for lp in state.forest.loops() {
        changed |= state.hoist(draft, lp, report);
        if report.limit_reached() {
            break;
        }
    }
    changed
}

/// What the pass knows while it moves instructions.
struct Hoisting<'s> {
    /// The loops, with the preheaders added so far.
    forest:     LoopForest<'s>,
    /// The reachable blocks in reverse post-order, each added preheader just
    /// before its header, so a block's dominators come before it.
    order:      ArenaVec<'s, Block>,
    /// The block that defines each value now, by value index.
    defined_in: ArenaVec<'s, Option<Block>>,
}

impl Hoisting<'_> {
    /// Moves the invariant instructions of `lp` to its preheader. Returns
    /// whether it moved any.
    fn hoist(
        &mut self,
        draft: &mut Draft<'_, '_>,
        lp: Loop,
        report: &mut OptimizationReport,
    ) -> bool {
        let mut blocks = ArenaVec::new_in(draft.scratch);
        blocks.extend(
            self.order
                .iter()
                .copied()
                .filter(|&block| self.forest.contains(lp, block)),
        );
        let scratch = draft.scratch;
        let mut preheader = None;
        for block in blocks {
            let insts =
                std::mem::replace(&mut draft.block_mut(block).insts, ArenaVec::new_in(scratch));
            let mut kept = ArenaVec::with_capacity_in(insts.len(), scratch);
            for &inst in &insts {
                if !self.is_invariant(draft, lp, inst) || !report.allow() {
                    kept.push(inst);
                    continue;
                }
                let target = match preheader {
                    | Some(target) => target,
                    | None => *preheader.insert(self.preheader(draft, lp)),
                };
                draft.block_mut(target).insts.push(inst);
                if let Some(result) = draft.inst_result(inst) {
                    self.defined_in[result.index()] = Some(target);
                }
            }
            draft.block_mut(block).insts = kept;
        }
        preheader.is_some()
    }

    /// Whether `inst` may move out of `lp`: it is live, may run
    /// speculatively, and uses only values defined outside the loop.
    fn is_invariant(&self, draft: &Draft<'_, '_>, lp: Loop, inst: Inst) -> bool {
        if draft.is_removed(inst) || draft.inst_result(inst).is_none() {
            return false;
        }
        if draft.inst_constant(inst).is_none() && !is_speculatable(draft, draft.inst(inst)) {
            return false;
        }
        let mut invariant = true;
        draft.for_each_operand(inst, |value| {
            invariant &=
                self.defined_in[value.index()].is_none_or(|block| !self.forest.contains(lp, block));
        });
        invariant
    }

    /// The preheader of `lp`: the existing one, or a new block that every
    /// entry edge is redirected through.
    fn preheader(&mut self, draft: &mut Draft<'_, '_>, lp: Loop) -> Block {
        let header = self.forest.header(lp);
        let predecessors = draft.predecessors();
        let mut entries = ArenaVec::new_in(draft.scratch);
        entries.extend(
            predecessors
                .of(header)
                .iter()
                .copied()
                .filter(|&block| !self.forest.contains(lp, block)),
        );
        if let [only] = entries[..]
            && matches!(draft.block(only).term, Term::Jump(_))
        {
            return only;
        }
        let preheader = draft.add_block();
        let mut args = ArenaVec::new_in(draft.scratch);
        for index in 0..draft.block(header).params.len() {
            let ty = draft.value_type(draft.block(header).params[index]);
            let param = draft.add_param(preheader, ty);
            args.push(param);
            self.defined_in.push(Some(preheader));
            debug_assert_eq!(
                self.defined_in.len(),
                draft.value_count(),
                "every value has a defining block"
            );
        }
        draft.block_mut(preheader).term = Term::Jump(Edge {
            target: header,
            args,
        });
        for entry in entries {
            for edge in draft.block_mut(entry).term.edges_mut() {
                if edge.target == header {
                    edge.target = preheader;
                }
            }
        }
        self.forest.add_block(preheader, self.forest.parent(lp));
        let position = self
            .order
            .iter()
            .position(|&block| block == header)
            .expect("a loop header is reachable");
        self.order.insert(position, preheader);
        preheader
    }
}

/// Whether an instruction may run where the program did not run it: it is
/// pure and cannot trap. A division needs a constant divisor that is not 0
/// and, if signed, not -1.
fn is_speculatable(draft: &Draft<'_, '_>, data: &InstData) -> bool {
    match *data {
        | InstData::Binary {
            opcode: opcode @ (Opcode::Sdiv | Opcode::Srem | Opcode::Udiv | Opcode::Urem),
            ty,
            args: [_, divisor],
            ..
        } => match draft.constant(divisor) {
            | Some(Constant::Int(_, bits)) =>
                bits != 0 && (matches!(opcode, Opcode::Udiv | Opcode::Urem) || bits != ty.mask()),
            | _ => false,
        },
        | _ => is_pure(data),
    }
}
