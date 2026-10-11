//! Constant folding and algebraic simplification.
//!
//! The pass walks the reachable blocks in reverse post-order, so a constant
//! defined in a dominating block is known when its users are met, and rewrites
//! each instruction it can decide:
//!
//! - an integer operation, comparison or cast of constants becomes a constant,
//!   with the IR's exact poison semantics (`eval.rs`): a violated `nsw`, `nuw`
//!   or `exact` flag, or a shift by at least the width, gives poison; division
//!   or remainder by constant zero (and the most negative number divided by -1)
//!   is undefined behaviour and is left alone;
//! - an operation with a poison operand is poison, except division by a
//!   possibly-zero divisor, which is left alone;
//! - identities replace an instruction by one of its operands or by a constant
//!   (listed below);
//! - `select` with a constant condition gives the chosen arm, with equal arms
//!   gives that arm, and with a poison condition is poison; `icmp` of one value
//!   with itself is decided; `freeze` of a constant is the constant;
//! - `brif` on a constant condition and `switch` on a constant become `jump`.
//!   Branching on poison is undefined behaviour and is left alone.
//!
//! The identities, with the constant on either side of a commutative
//! operation: `x+0`, `x-0`, `x*1`, `x&-1`, `x|0`, `x^0`, `x<<0`, `x>>0`,
//! `x/1` and `ptr_add p, 0` give the operand; `x*0`, `x&0`, `x|-1`, `x-x`,
//! `x^x` and `x%1` give a constant; `x&x` and `x|x` give `x`. Replacing a
//! possibly-poison operand by a defined value only refines the program, which
//! the poison rules allow.
//!
//! Floating-point operations are not folded in this prototype: exact folding
//! needs target-faithful `f32`, `f64`, `f80` and `f128` arithmetic. A
//! folded instruction keeps its result value; the dead-code pass removes it
//! once nothing uses it. An instruction replaced by an existing value is
//! removed at once.
//!
//! C99: §6.5 paragraph 5, p. 67; PDF p. 79 (signed overflow is undefined, so
//! `nsw` results may be assumed not to overflow); §6.5.5 paragraph 5, p. 82;
//! PDF p. 94 (division by zero is undefined); §6.5.7 paragraph 3, p. 84; PDF
//! p. 96 (a shift by the width or more is undefined in C and poison here).

pub(super) mod eval;

use eval::Folded;

use super::{
    draft::{
        Constant,
        Draft,
        Term,
    },
    report::OptimizationReport,
};
use crate::ir::{
    Block,
    Inst,
    InstData,
    InstFlags,
    Opcode,
    Type,
    Value,
};

/// Folds one function. Returns whether it rewrote anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    for block in draft.reverse_post_order() {
        for index in 0..draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            let Some(outcome) = fold_inst(draft, inst) else {
                continue;
            };
            if !report.allow() {
                continue;
            }
            apply(draft, inst, outcome);
            changed = true;
        }
        if fold_terminator(draft, block, report) {
            changed = true;
        }
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// What an instruction folds to.
#[derive(Clone, Copy, Debug)]
enum Outcome {
    /// Another value, which dominates every use of the instruction's result.
    Value(Value),
    Constant(Constant),
}

/// Rewrites `inst` as `outcome` says.
fn apply(draft: &mut Draft<'_, '_>, inst: Inst, outcome: Outcome) {
    match outcome {
        | Outcome::Value(value) => {
            let result = draft
                .inst_result(inst)
                .expect("a folded instruction has a result");
            draft.replace(result, value);
            draft.remove_inst(inst);
        },
        | Outcome::Constant(constant) => draft.set_constant(inst, constant),
    }
}

/// What `inst` folds to, if anything.
fn fold_inst(draft: &Draft<'_, '_>, inst: Inst) -> Option<Outcome> {
    if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
        return None;
    }
    _ = draft.inst_result(inst)?;
    match *draft.inst(inst) {
        | InstData::Binary {
            opcode: Opcode::PtrAdd,
            args: [base, offset],
            ..
        } => match draft.constant(offset) {
            | Some(Constant::Int(_, 0)) => Some(Outcome::Value(base)),
            | _ => None,
        },
        | InstData::Binary {
            opcode,
            ty,
            flags,
            args: [a, b],
        } if opcode.is_int_binary() => fold_binary(draft, opcode, ty, flags, a, b),
        | InstData::Unary { opcode, ty, arg } => fold_unary(draft, opcode, ty, arg),
        | InstData::IntCompare {
            cond,
            ty,
            args: [a, b],
        } => fold_compare(draft, cond, ty, a, b),
        | InstData::Select {
            ty,
            args: [cond, a, b],
        } => match draft.constant(cond) {
            | Some(Constant::Int(_, bits)) => Some(Outcome::Value(if bits != 0 { a } else { b })),
            | Some(Constant::Poison(_)) => Some(Outcome::Constant(Constant::Poison(ty))),
            | None if draft.resolve(a) == draft.resolve(b) => Some(Outcome::Value(a)),
            | None => None,
        },
        | _ => None,
    }
}

fn fold_binary(
    draft: &Draft<'_, '_>,
    opcode: Opcode,
    ty: Type,
    flags: InstFlags,
    a: Value,
    b: Value,
) -> Option<Outcome> {
    let (constant_a, constant_b) = (draft.constant(a), draft.constant(b));
    let int = |constant: Option<Constant>| match constant {
        | Some(Constant::Int(_, bits)) => Some(bits),
        | _ => None,
    };
    let poison = Outcome::Constant(Constant::Poison(ty));
    let from_folded = |folded: Folded| match folded {
        | Folded::Int(bits) => Outcome::Constant(Constant::Int(ty, bits)),
        | Folded::Poison => poison,
    };
    if matches!(
        opcode,
        Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
    ) {
        // A divisor that may be zero or poison makes the division undefined
        // behaviour; only a known nonzero divisor is safe to reason about.
        let divisor = int(constant_b).filter(|&bits| bits != 0)?;
        return match constant_a {
            | Some(Constant::Int(_, dividend)) =>
                eval::binary(opcode, ty, flags, dividend, divisor).map(from_folded),
            | Some(Constant::Poison(_)) => Some(poison),
            | None if divisor == 1 => Some(if matches!(opcode, Opcode::Sdiv | Opcode::Udiv) {
                Outcome::Value(a)
            } else {
                Outcome::Constant(Constant::Int(ty, 0))
            }),
            | None => None,
        };
    }
    if let (Some(x), Some(y)) = (int(constant_a), int(constant_b)) {
        return eval::binary(opcode, ty, flags, x, y).map(from_folded);
    }
    if matches!(constant_a, Some(Constant::Poison(_)))
        || matches!(constant_b, Some(Constant::Poison(_)))
    {
        return Some(poison);
    }
    identity(
        opcode,
        ty,
        (draft.resolve(a), draft.resolve(b)),
        (int(constant_a), int(constant_b)),
    )
}

/// The simplification of `opcode` by an identity or absorbing constant, or
/// by equal operands.
fn identity(
    opcode: Opcode,
    ty: Type,
    (a, b): (Value, Value),
    (constant_a, constant_b): (Option<u128>, Option<u128>),
) -> Option<Outcome> {
    let ones = ty.mask();
    let zero = Outcome::Constant(Constant::Int(ty, 0));
    let all_ones = Outcome::Constant(Constant::Int(ty, ones));
    let same = a == b;
    match opcode {
        | Opcode::Iadd
        | Opcode::Or
        | Opcode::Xor
        | Opcode::Isub
        | Opcode::Shl
        | Opcode::Lshr
        | Opcode::Ashr
            if constant_b == Some(0) =>
            Some(Outcome::Value(a)),
        | Opcode::Iadd | Opcode::Or | Opcode::Xor if constant_a == Some(0) =>
            Some(Outcome::Value(b)),
        | Opcode::Imul if constant_b == Some(1) => Some(Outcome::Value(a)),
        | Opcode::Imul if constant_a == Some(1) => Some(Outcome::Value(b)),
        | Opcode::Imul | Opcode::And if constant_a == Some(0) || constant_b == Some(0) =>
            Some(zero),
        | Opcode::And if constant_b == Some(ones) => Some(Outcome::Value(a)),
        | Opcode::And if constant_a == Some(ones) => Some(Outcome::Value(b)),
        | Opcode::Or if constant_a == Some(ones) || constant_b == Some(ones) => Some(all_ones),
        | Opcode::And | Opcode::Or if same => Some(Outcome::Value(a)),
        | Opcode::Isub | Opcode::Xor if same => Some(zero),
        | _ => None,
    }
}

fn fold_unary(draft: &Draft<'_, '_>, opcode: Opcode, ty: Type, arg: Value) -> Option<Outcome> {
    let constant = draft.constant(arg)?;
    match (opcode, constant) {
        | (Opcode::Freeze, Constant::Int(..)) => Some(Outcome::Value(arg)),
        | (Opcode::Zext | Opcode::Sext | Opcode::Trunc, Constant::Poison(_)) if ty.is_int() =>
            Some(Outcome::Constant(Constant::Poison(ty))),
        | (Opcode::Zext | Opcode::Sext | Opcode::Trunc, Constant::Int(from, bits)) if ty.is_int() =>
            eval::convert(opcode, from, ty, bits)
                .map(|bits| Outcome::Constant(Constant::Int(ty, bits))),
        | _ => None,
    }
}

fn fold_compare(
    draft: &Draft<'_, '_>,
    cond: crate::ir::IntCC,
    ty: Type,
    a: Value,
    b: Value,
) -> Option<Outcome> {
    use crate::ir::IntCC;

    if !ty.is_int() {
        return None;
    }
    let boolean = |value: bool| {
        Some(Outcome::Constant(Constant::Int(
            Type::I1,
            u128::from(value),
        )))
    };
    match (draft.constant(a), draft.constant(b)) {
        | (Some(Constant::Int(_, x)), Some(Constant::Int(_, y))) =>
            boolean(eval::compare(cond, ty, x, y)),
        | (Some(Constant::Poison(_)), _) | (_, Some(Constant::Poison(_))) =>
            Some(Outcome::Constant(Constant::Poison(Type::I1))),
        | _ if draft.resolve(a) == draft.resolve(b) => boolean(matches!(
            cond,
            IntCC::Eq | IntCC::Sle | IntCC::Sge | IntCC::Ule | IntCC::Uge
        )),
        | _ => None,
    }
}

/// Turns a `brif` or `switch` on a constant into a `jump`.
fn fold_terminator(
    draft: &mut Draft<'_, '_>,
    block: Block,
    report: &mut OptimizationReport,
) -> bool {
    let chosen = match &draft.block(block).term {
        | Term::Brif {
            cond,
            then,
            otherwise,
        } => match draft.constant(*cond) {
            | Some(Constant::Int(_, bits)) => Some(if bits != 0 { then } else { otherwise }),
            | _ => None,
        },
        | Term::Switch {
            value,
            default,
            cases,
        } => match draft.constant(*value) {
            | Some(Constant::Int(_, bits)) => Some(
                cases
                    .iter()
                    .find(|(case, _)| *case == bits)
                    .map_or(default, |(_, edge)| edge),
            ),
            | _ => None,
        },
        | _ => None,
    };
    let Some(edge) = chosen else {
        return false;
    };
    if !report.allow() {
        return false;
    }
    let edge = edge.clone();
    draft.block_mut(block).term = Term::Jump(edge);
    true
}
