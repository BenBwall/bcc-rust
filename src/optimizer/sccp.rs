//! Sparse conditional constant propagation (Wegman and Zadeck, "Constant
//! propagation with conditional branches", 1991).
//!
//! The pass solves for a lattice value per SSA value, optimistically: every
//! value starts *unknown* (top), and only blocks reached along *executable*
//! edges are evaluated, so a value that is constant along the edges that can
//! run is found constant even if a dead edge would pass something else. Two
//! worklists drive the solver: newly executable blocks, whose instructions and
//! terminator are evaluated, and values whose lattice value dropped, whose
//! users are evaluated again. A block parameter is the meet of the arguments
//! on the executable edges into its block. A terminator marks the edges its
//! condition allows executable: one for a constant, all for an overdefined
//! condition, none yet for an unknown one.
//!
//! The lattice, from top to bottom: unknown; `poison`; an integer constant;
//! overdefined. `poison` sits above the constants because it may be refined
//! to any value, so a parameter that receives `poison` on one edge and `7` on
//! another is `7`. Evaluation follows the poison rules of the fold pass
//! exactly (`fold/eval.rs` computes the constants): a violated `nsw`, `nuw`
//! or `exact`, or a shift by the width or more, gives `poison`; an operand
//! that is `poison` makes the result `poison`; division or remainder by zero,
//! by `poison`, or of the most negative number by -1 is undefined behaviour
//! and is overdefined; `freeze` of a constant is the constant and of `poison`
//! is overdefined. Absorbing operands decide an operation even when the other
//! operand is overdefined (`x*0`, `x&0`, `x|-1`, `x%1`, `x-x`, `x^x`, and the
//! comparison of a value with itself). Floating-point values, memory and calls
//! are overdefined.
//!
//! Branching on `poison` is undefined behaviour. LLVM's solver leaves such a
//! branch with no executable successor while it solves and, once nothing else
//! changes, makes one successor executable so that the code after it is still
//! analysed. This pass does the same, but makes every successor executable:
//! the branch is treated as overdefined and left in place, as `fold` leaves
//! it, rather than resolved in a direction the program cannot rely on.
//!
//! Once solved, every rewrite asks the report first:
//!
//! - a parameter of an executable block that is a constant and has uses is
//!   replaced by a constant added to the entry block, and removed with the
//!   matching argument of every edge into its block;
//! - an instruction of an executable block whose result is a constant is folded
//!   to it, unless it already is that constant;
//! - a `brif` or `switch` on a constant becomes a `jump`.
//!
//! Blocks the solver never reached are left for `simplify-cfg`, which removes
//! them once the branches into them are gone.
//!
//! For a loop whose counter only ever holds its initial value,
//!
//! ```text
//! block0:
//!     v0 = iconst.i32 0
//!     jump block1(v0)
//! block1(v1: i32):
//!     v2 = icmp.i32 eq v1, v0
//!     brif v2, block2, block3
//! block2:
//!     v3 = iadd.i32 v1, v0
//!     jump block1(v3)
//! block3:
//!     return v1
//! ```
//!
//! the solver first reaches `block1` with `v1 = 0`, decides `v2` true and
//! reaches only `block2`, where `v3 = 0`; the back edge passes `0` again, so
//! `v1` stays `0`, `block3` is never executable and the `brif` becomes
//! `jump block2`.
//!
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (signed overflow is undefined, so
//! `nsw` results may be assumed not to overflow); §6.5.5 paragraph 5, p. 82;
//! PDF p. 94 (division by zero is undefined).

use rustc_hash::FxBuildHasher;

use super::{
    draft::{
        Constant,
        Draft,
        Predecessors,
        Term,
    },
    fold::eval::{
        self,
        Folded,
    },
    report::OptimizationReport,
};
use crate::{
    ir::{
        Block,
        Entity,
        Inst,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
    },
};

/// Propagates constants through one function. Returns whether it rewrote
/// anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let solution = Solver::new(draft).solve();
    let mut rewriter = Rewriter {
        constants: ArenaMap::with_hasher_in(FxBuildHasher, draft.scratch),
        changed:   false,
    };
    // Parameters first, while the edges into each block are as solved.
    let finished = rewriter.replace_params(draft, &solution, report)
        && rewriter.fold_insts(draft, &solution, report);
    if finished {
        rewriter.decide_branches(draft, &solution, report);
    }
    rewriter.changed
}

/// What the solver knows about a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lattice {
    /// Not yet seen to have a value: no executable definition or edge.
    Unknown,
    Poison,
    /// An integer constant, masked to the value's type.
    Int(u128),
    Overdefined,
}

impl Lattice {
    /// The greatest lower bound. `poison` meets a constant as the constant.
    fn meet(self, other: Self) -> Self {
        match (self, other) {
            | (Self::Unknown, value) | (value, Self::Unknown) => value,
            | (Self::Overdefined, _) | (_, Self::Overdefined) => Self::Overdefined,
            | (Self::Poison, value) | (value, Self::Poison) => value,
            | (Self::Int(a), Self::Int(b)) =>
                if a == b {
                    self
                } else {
                    Self::Overdefined
                },
        }
    }

    fn from_constant(constant: Constant) -> Self {
        match constant {
            | Constant::Int(_, bits) => Self::Int(bits),
            | Constant::Poison(_) => Self::Poison,
        }
    }

    /// The constant of type `ty` this is, if it is one.
    fn constant(self, ty: Type) -> Option<Constant> {
        match self {
            | Self::Int(bits) => Some(Constant::Int(ty, bits)),
            | Self::Poison => Some(Constant::Poison(ty)),
            | Self::Unknown | Self::Overdefined => None,
        }
    }
}

/// A use of a value that the solver revisits when the value drops.
#[derive(Clone, Copy, Debug)]
enum User {
    Inst(Inst),
    /// The block's terminator: its condition or an edge argument.
    Term(Block),
}

/// The solver's state over one function.
struct Solver<'d, 'b, 's> {
    draft:       &'d Draft<'b, 's>,
    /// Each value's lattice value, by value index.
    states:      ArenaVec<'s, Lattice>,
    /// Whether each block is executable.
    executable:  ArenaVec<'s, bool>,
    /// Where each block's edges start in `edges`.
    edge_starts: ArenaVec<'s, u32>,
    /// Whether each edge is executable, by block and then edge order.
    edges:       ArenaVec<'s, bool>,
    /// The block of each live instruction, by instruction index.
    inst_blocks: ArenaVec<'s, Block>,
    /// Each value's users, as rows of one array.
    user_starts: ArenaVec<'s, u32>,
    users:       ArenaVec<'s, User>,
    /// Blocks that became executable and await evaluation.
    block_work:  ArenaVec<'s, Block>,
    /// Values whose lattice value dropped and whose users await evaluation.
    value_work:  ArenaVec<'s, Value>,
}

/// The solver's result: each value's lattice value and the executable blocks.
struct Solution<'s> {
    states:     ArenaVec<'s, Lattice>,
    executable: ArenaVec<'s, bool>,
}

impl<'s> Solver<'_, '_, 's> {
    /// Runs both worklists dry, then makes branches on `poison` overdefined
    /// and runs them again, until nothing changes.
    fn solve(mut self) -> Solution<'s> {
        let entry = Draft::entry();
        for &param in &self.draft.block(entry).params {
            self.states[param.index()] = Lattice::Overdefined;
        }
        self.executable[entry.index()] = true;
        self.block_work.push(entry);
        loop {
            if let Some(block) = self.block_work.pop() {
                self.visit_block(block);
            } else if let Some(value) = self.value_work.pop() {
                self.visit_users(value);
            } else if !self.resolve_poison_branches() {
                break;
            }
        }
        Solution {
            states:     self.states,
            executable: self.executable,
        }
    }

    fn visit_block(&mut self, block: Block) {
        let draft = self.draft;
        for &inst in &draft.block(block).insts {
            self.visit_inst(inst);
        }
        self.visit_term(block);
    }

    fn visit_users(&mut self, value: Value) {
        let index = value.index();
        for row in self.user_starts[index]..self.user_starts[index + 1] {
            match self.users[row as usize] {
                | User::Inst(inst) =>
                    if self.executable[self.inst_blocks[inst.index()].index()] {
                        self.visit_inst(inst);
                    },
                | User::Term(block) =>
                    if self.executable[block.index()] {
                        self.visit_term(block);
                    },
            }
        }
    }

    fn visit_inst(&mut self, inst: Inst) {
        if self.draft.is_removed(inst) {
            return;
        }
        let Some(result) = self.draft.inst_result(inst) else {
            return;
        };
        let value = self.evaluate(inst);
        self.lower(result, value);
    }

    /// Marks the edges the terminator may take executable, then passes the
    /// arguments of every executable edge to its target's parameters.
    fn visit_term(&mut self, block: Block) {
        let draft = self.draft;
        let term = &draft.block(block).term;
        match *term {
            | Term::Jump(_) => self.mark_edge(block, 0),
            | Term::Brif { cond, .. } => match self.state(cond) {
                | Lattice::Int(bits) => self.mark_edge(block, usize::from(bits == 0)),
                | Lattice::Overdefined => self.mark_all_edges(block),
                | Lattice::Unknown | Lattice::Poison => {},
            },
            | Term::Switch {
                value, ref cases, ..
            } => match self.state(value) {
                | Lattice::Int(bits) => {
                    let taken = cases
                        .iter()
                        .position(|&(case, _)| case == bits)
                        .map_or(0, |position| position + 1);
                    self.mark_edge(block, taken);
                },
                | Lattice::Overdefined => self.mark_all_edges(block),
                | Lattice::Unknown | Lattice::Poison => {},
            },
            | Term::Return(_) | Term::Unreachable => {},
        }
        let start = self.edge_starts[block.index()] as usize;
        for (index, edge) in term.edges().enumerate() {
            if !self.edges[start + index] {
                continue;
            }
            let params = &draft.block(edge.target).params;
            for (&arg, &param) in edge.args.iter().zip(params) {
                let value = self.state(arg);
                self.lower(param, value);
            }
        }
    }

    fn mark_all_edges(&mut self, block: Block) {
        for index in 0..self.draft.block(block).term.edges().count() {
            self.mark_edge(block, index);
        }
    }

    /// Makes the `index`th edge of `block` executable, and its target with
    /// it. The caller passes the edge's arguments.
    fn mark_edge(&mut self, block: Block, index: usize) {
        let row = self.edge_starts[block.index()] as usize + index;
        if self.edges[row] {
            return;
        }
        self.edges[row] = true;
        let target = self
            .draft
            .block(block)
            .term
            .edges()
            .nth(index)
            .expect("the edge exists")
            .target;
        if !self.executable[target.index()] {
            self.executable[target.index()] = true;
            self.block_work.push(target);
        }
    }

    /// Makes every edge of an executable branch on `poison` (or on a value
    /// still unknown) executable. Returns whether any edge changed.
    fn resolve_poison_branches(&mut self) -> bool {
        let draft = self.draft;
        let mut changed = false;
        for block in draft.alive_blocks() {
            if !self.executable[block.index()] {
                continue;
            }
            let condition = match draft.block(block).term {
                | Term::Brif { cond: value, .. } | Term::Switch { value, .. } => value,
                | Term::Jump(_) | Term::Return(_) | Term::Unreachable => continue,
            };
            if matches!(self.state(condition), Lattice::Unknown | Lattice::Poison) {
                let start = self.edge_starts[block.index()] as usize;
                let count = draft.block(block).term.edges().count();
                if self.edges[start..start + count].iter().all(|&taken| taken) {
                    continue;
                }
                self.mark_all_edges(block);
                self.visit_term(block);
                changed = true;
            }
        }
        changed
    }

    /// Lowers `value`'s lattice value to its meet with `new`, and queues its
    /// users if that changed it.
    fn lower(&mut self, value: Value, new: Lattice) {
        let old = self.states[value.index()];
        let met = old.meet(new);
        if met != old {
            self.states[value.index()] = met;
            self.value_work.push(value);
        }
    }

    fn state(&self, value: Value) -> Lattice {
        self.states[self.draft.resolve(value).index()]
    }

    /// The lattice value of `inst`'s result from its operands' values.
    fn evaluate(&self, inst: Inst) -> Lattice {
        if let Some(constant) = self.draft.inst_constant(inst) {
            return Lattice::from_constant(constant);
        }
        match *self.draft.inst(inst) {
            | InstData::Binary {
                opcode,
                ty,
                flags,
                args: [a, b],
            } if opcode.is_int_binary() => self.binary(opcode, ty, flags, a, b),
            | InstData::Unary {
                opcode: Opcode::Freeze,
                arg,
                ..
            } => match self.state(arg) {
                | value @ (Lattice::Unknown | Lattice::Int(_)) => value,
                | Lattice::Poison | Lattice::Overdefined => Lattice::Overdefined,
            },
            | InstData::Unary { opcode, ty, arg }
                if matches!(opcode, Opcode::Zext | Opcode::Sext | Opcode::Trunc) && ty.is_int() =>
                match self.state(arg) {
                    | Lattice::Int(bits) =>
                        eval::convert(opcode, self.draft.value_type(arg), ty, bits)
                            .map_or(Lattice::Overdefined, Lattice::Int),
                    | value => value,
                },
            | InstData::IntCompare {
                cond,
                ty,
                args: [a, b],
            } if ty.is_int() => self.compare(cond, ty, a, b),
            | InstData::Select {
                args: [cond, a, b], ..
            } => match self.state(cond) {
                | Lattice::Int(bits) => self.state(if bits != 0 { a } else { b }),
                | value @ (Lattice::Unknown | Lattice::Poison) => value,
                | Lattice::Overdefined if self.same(a, b) => self.state(a),
                | Lattice::Overdefined => self.state(a).meet(self.state(b)),
            },
            | _ => Lattice::Overdefined,
        }
    }

    fn binary(&self, opcode: Opcode, ty: Type, flags: InstFlags, a: Value, b: Value) -> Lattice {
        let from_folded = |folded: Option<Folded>| match folded {
            | Some(Folded::Int(bits)) => Lattice::Int(bits),
            | Some(Folded::Poison) => Lattice::Poison,
            | None => Lattice::Overdefined,
        };
        let (left, right) = (self.state(a), self.state(b));
        if matches!(
            opcode,
            Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
        ) {
            // Only a known nonzero divisor is safe to reason about; zero and
            // `poison` divisors are undefined behaviour.
            let divisor = match right {
                | Lattice::Int(bits) if bits != 0 => bits,
                | Lattice::Unknown => return Lattice::Unknown,
                | _ => return Lattice::Overdefined,
            };
            return match left {
                | Lattice::Int(dividend) =>
                    from_folded(eval::binary(opcode, ty, flags, dividend, divisor)),
                | Lattice::Poison | Lattice::Unknown => left,
                | Lattice::Overdefined
                    if divisor == 1 && matches!(opcode, Opcode::Srem | Opcode::Urem) =>
                    Lattice::Int(0),
                | Lattice::Overdefined => Lattice::Overdefined,
            };
        }
        match (left, right) {
            | (Lattice::Int(left_bits), Lattice::Int(right_bits)) =>
                from_folded(eval::binary(opcode, ty, flags, left_bits, right_bits)),
            | (Lattice::Poison, _) | (_, Lattice::Poison) => Lattice::Poison,
            | (Lattice::Unknown, _) | (_, Lattice::Unknown) => Lattice::Unknown,
            | _ => {
                let either = |bits: u128| left == Lattice::Int(bits) || right == Lattice::Int(bits);
                match opcode {
                    | Opcode::Imul | Opcode::And if either(0) => Lattice::Int(0),
                    | Opcode::Or if either(ty.mask()) => Lattice::Int(ty.mask()),
                    | Opcode::Isub | Opcode::Xor if self.same(a, b) => Lattice::Int(0),
                    | _ => Lattice::Overdefined,
                }
            },
        }
    }

    fn compare(&self, cond: IntCC, ty: Type, a: Value, b: Value) -> Lattice {
        let boolean = |value: bool| Lattice::Int(u128::from(value));
        match (self.state(a), self.state(b)) {
            | (Lattice::Int(m), Lattice::Int(n)) => boolean(eval::compare(cond, ty, m, n)),
            | (Lattice::Poison, _) | (_, Lattice::Poison) => Lattice::Poison,
            | _ if self.same(a, b) => boolean(matches!(
                cond,
                IntCC::Eq | IntCC::Sle | IntCC::Sge | IntCC::Ule | IntCC::Uge
            )),
            | (Lattice::Unknown, _) | (_, Lattice::Unknown) => Lattice::Unknown,
            | _ => Lattice::Overdefined,
        }
    }

    fn same(&self, a: Value, b: Value) -> bool {
        self.draft.resolve(a) == self.draft.resolve(b)
    }
}

impl<'d, 'b, 's> Solver<'d, 'b, 's> {
    /// A solver with every value unknown and every block unexecuted, and the
    /// use lists and edge numbering it needs.
    fn new(draft: &'d Draft<'b, 's>) -> Self {
        let scratch = draft.scratch;
        let blocks = draft.blocks.len();
        let mut states = ArenaVec::with_capacity_in(draft.value_count(), scratch);
        states.resize(draft.value_count(), Lattice::Unknown);
        let mut executable = ArenaVec::with_capacity_in(blocks, scratch);
        executable.resize(blocks, false);
        let mut edge_starts = ArenaVec::with_capacity_in(blocks + 1, scratch);
        edge_starts.push(0_u32);
        let mut inst_blocks = ArenaVec::with_capacity_in(draft.inst_count(), scratch);
        inst_blocks.resize(draft.inst_count(), Draft::entry());
        let mut pairs: ArenaVec<'s, (Value, User)> = ArenaVec::new_in(scratch);
        for index in 0..blocks {
            let block = Block::new(index);
            let data = draft.block(block);
            let edges = if data.alive {
                data.term.edges().count()
            } else {
                0
            };
            let previous = *edge_starts.last().expect("the first start was pushed");
            edge_starts.push(previous + u32::try_from(edges).expect("fewer than 2^32 edges"));
            if !data.alive {
                continue;
            }
            for &inst in &data.insts {
                if draft.is_removed(inst) {
                    continue;
                }
                inst_blocks[inst.index()] = block;
                draft.for_each_operand(inst, |value| pairs.push((value, User::Inst(inst))));
            }
            data.term.for_each_control_operand(|value| {
                pairs.push((draft.resolve(value), User::Term(block)));
            });
            for edge in data.term.edges() {
                for &arg in &edge.args {
                    pairs.push((draft.resolve(arg), User::Term(block)));
                }
            }
        }
        let mut edges = ArenaVec::with_capacity_in(edge_starts[blocks] as usize, scratch);
        edges.resize(edge_starts[blocks] as usize, false);
        let values = draft.value_count();
        let mut user_starts = ArenaVec::with_capacity_in(values + 1, scratch);
        user_starts.resize(values + 1, 0_u32);
        for &(value, _) in &pairs {
            user_starts[value.index() + 1] += 1;
        }
        for index in 1..user_starts.len() {
            user_starts[index] += user_starts[index - 1];
        }
        let mut cursors = ArenaVec::with_capacity_in(values, scratch);
        cursors.extend_from_slice(&user_starts[..values]);
        let mut users = ArenaVec::with_capacity_in(pairs.len(), scratch);
        users.resize(pairs.len(), User::Term(Draft::entry()));
        for &(value, user) in &pairs {
            let cursor = &mut cursors[value.index()];
            users[*cursor as usize] = user;
            *cursor += 1;
        }
        Self {
            draft,
            states,
            executable,
            edge_starts,
            edges,
            inst_blocks,
            user_starts,
            users,
            block_work: ArenaVec::new_in(scratch),
            value_work: ArenaVec::new_in(scratch),
        }
    }
}

impl Solution<'_> {
    /// The constant `value` was found to be, if any.
    fn constant(&self, draft: &Draft<'_, '_>, value: Value) -> Option<Constant> {
        let value = draft.resolve(value);
        match self.states.get(value.index()) {
            | Some(state) => state.constant(draft.value_type(value)),
            // A constant the rewrite added after solving.
            | None => draft.constant(value),
        }
    }

    fn is_executable(&self, block: Block) -> bool {
        self.executable[block.index()]
    }
}

/// Applies a solution. Each step returns `false` once the report refuses a
/// rewrite, which ends the pass.
struct Rewriter<'s> {
    /// The constants added to the entry block, shared by the parameters they
    /// replace.
    constants: ArenaMap<'s, (Type, Option<u128>), Value>,
    changed:   bool,
}

impl<'s> Rewriter<'s> {
    /// Replaces each constant parameter of an executable block that has uses
    /// by a constant in the entry block, and removes it.
    fn replace_params(
        &mut self,
        draft: &mut Draft<'_, 's>,
        solution: &Solution<'_>,
        report: &mut OptimizationReport,
    ) -> bool {
        let counts = draft.use_counts();
        let predecessors = draft.predecessors();
        for index in 1..draft.blocks.len() {
            let block = Block::new(index);
            if !draft.block(block).alive || !solution.is_executable(block) {
                continue;
            }
            let mut position = 0;
            while let Some(&param) = draft.block(block).params.get(position) {
                let constant = solution.constant(draft, param);
                let Some(constant) = constant
                    .filter(|_| counts[param.index()] != 0 && draft.resolve(param) == param)
                else {
                    position += 1;
                    continue;
                };
                if !report.allow() {
                    return false;
                }
                let value = self.constant_value(draft, constant);
                draft.replace(param, value);
                remove_param(draft, &predecessors, block, position);
                self.changed = true;
            }
        }
        true
    }

    /// Folds each instruction of an executable block whose result is a
    /// constant.
    fn fold_insts(
        &mut self,
        draft: &mut Draft<'_, '_>,
        solution: &Solution<'_>,
        report: &mut OptimizationReport,
    ) -> bool {
        for index in 0..draft.blocks.len() {
            let block = Block::new(index);
            if !draft.block(block).alive || !solution.is_executable(block) {
                continue;
            }
            for position in 0..draft.block(block).insts.len() {
                let inst = draft.block(block).insts[position];
                if draft.is_removed(inst) {
                    continue;
                }
                let Some(result) = draft.inst_result(inst) else {
                    continue;
                };
                let Some(constant) = solution.constant(draft, result) else {
                    continue;
                };
                if draft.inst_constant(inst) == Some(constant) {
                    continue;
                }
                if !report.allow() {
                    return false;
                }
                draft.set_constant(inst, constant);
                self.changed = true;
            }
        }
        true
    }

    /// Turns each `brif` or `switch` of an executable block on a constant
    /// into a `jump`.
    fn decide_branches(
        &mut self,
        draft: &mut Draft<'_, '_>,
        solution: &Solution<'_>,
        report: &mut OptimizationReport,
    ) {
        for index in 0..draft.blocks.len() {
            let block = Block::new(index);
            if !draft.block(block).alive || !solution.is_executable(block) {
                continue;
            }
            let chosen = match &draft.block(block).term {
                | Term::Brif {
                    cond,
                    then,
                    otherwise,
                } => match solution.constant(draft, *cond) {
                    | Some(Constant::Int(_, bits)) =>
                        Some(if bits != 0 { then } else { otherwise }),
                    | _ => None,
                },
                | Term::Switch {
                    value,
                    default,
                    cases,
                } => match solution.constant(draft, *value) {
                    | Some(Constant::Int(_, bits)) => Some(
                        cases
                            .iter()
                            .find(|(case, _)| *case == bits)
                            .map_or(default, |(_, edge)| edge),
                    ),
                    | _ => None,
                },
                | Term::Jump(_) | Term::Return(_) | Term::Unreachable => None,
            };
            let Some(edge) = chosen.cloned() else {
                continue;
            };
            if !report.allow() {
                return;
            }
            draft.block_mut(block).term = Term::Jump(edge);
            self.changed = true;
        }
    }

    /// The value of an added entry-block constant equal to `constant`.
    fn constant_value(&mut self, draft: &mut Draft<'_, '_>, constant: Constant) -> Value {
        let key = match constant {
            | Constant::Int(ty, bits) => (ty, Some(bits)),
            | Constant::Poison(ty) => (ty, None),
        };
        *self
            .constants
            .entry(key)
            .or_insert_with(|| draft.add_constant(constant))
    }
}

/// Removes the `position`th parameter of `block` and the matching argument of
/// every edge into it.
fn remove_param(
    draft: &mut Draft<'_, '_>,
    predecessors: &Predecessors<'_>,
    block: Block,
    position: usize,
) {
    _ = draft.block_mut(block).params.remove(position);
    for &predecessor in predecessors.of(block) {
        for edge in draft.block_mut(predecessor).term.edges_mut() {
            if edge.target == block {
                _ = edge.args.remove(position);
            }
        }
    }
}
