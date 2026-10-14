//! Peephole combining: rewrites short chains of integer instructions into
//! fewer, simpler ones, in the style of LLVM's `InstCombine`.
//!
//! The pass walks the reachable blocks in reverse post-order and tries each
//! rule on each instruction; a rule reads the instruction's operands'
//! definitions, so a rewritten instruction is seen with its rewritten
//! operands. The rules:
//!
//! 1. **Booleans compared with constants.** `icmp ne b, 0` and `icmp eq b, 1`
//!    of an `i1` are `b`; `icmp eq b, 0` and `icmp ne b, 1` are `xor b, true`.
//!    The same holds for the zero extension of an `i1` compared with 0 or 1,
//!    and other constants decide the comparison.
//! 2. **Negated comparisons.** `xor (icmp c a, b), true` is the comparison with
//!    the inverse condition, and `xor (xor x, true), true` is `x`.
//! 3. **Boolean selects.** `select c, true, false` is `c`; `select c, false,
//!    true` is `xor c, true`; `select (xor c, true), a, b` is `select c, b, a`.
//! 4. **Range checks.** Two comparisons of one value with constants, combined
//!    by `and` or by `select a, b, false` (lowered `&&`), become one unsigned
//!    comparison when they bound a single interval, and their union by `or` or
//!    `select a, true, b` (lowered `||`) its negation:
//!
//!    ```text
//!    x >= lo && x <= hi    is    x - lo <= hi - lo    (unsigned, wrapping)
//!    x < lo || x > hi      is    x - lo > hi - lo
//!    ```
//!
//!    Disjoint bounds give `false` and `true`.
//! 5. **Constant reassociation.** `iadd (iadd x, c1), c2` is `iadd x, c1 + c2`,
//!    and likewise with either operation a subtraction.
//!
//! **Poison.** Each rewrite is poison exactly where the original is, or less:
//! a comparison or extension is poison exactly when its operand is, and the
//! two comparisons of a range check share their operand, so the `select`'s
//! masking of its unchosen arm never hides a poison the other comparison
//! would not also have. Reassociation drops `nsw` and `nuw`: a wrapping sum
//! is defined wherever the original pair was.
//!
//! Each rewrite consumes one `allow()` for bisecting. New instructions are
//! inserted just before the instruction they replace, and constants go into
//! the entry block, so every operand dominates its use.
//!
//! C99: §5.1.2.3 paragraph 5, pp. 13-14; PDF pp. 25-26; §6.5.13 and §6.5.14,
//! pp. 89-90; PDF pp. 101-102 (`&&` and `||` yield 0 or 1, which lowering
//! leaves as `i1` joins and selects for this pass).

use super::{
    draft::{
        Constant,
        Draft,
    },
    fold::eval,
    report::OptimizationReport,
};
use crate::ir::{
    Block,
    InstData,
    InstFlags,
    IntCC,
    Opcode,
    Type,
    Value,
    ValueDef,
};

/// Combines one function. Returns whether it rewrote anything.
pub(super) fn run(draft: &mut Draft<'_, '_>, report: &mut OptimizationReport) -> bool {
    let mut changed = false;
    for block in draft.reverse_post_order() {
        let mut index = 0;
        while index < draft.block(block).insts.len() {
            let inst = draft.block(block).insts[index];
            index += 1;
            if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
                continue;
            }
            let Some(rewrite) = combine(draft, *draft.inst(inst)) else {
                continue;
            };
            if !report.allow() {
                return changed;
            }
            let result = draft
                .inst_result(inst)
                .expect("a combined instruction has a value");
            match rewrite {
                | Rewrite::Value(value) => {
                    draft.replace(result, value);
                    draft.remove_inst(inst);
                },
                | Rewrite::Constant(bits) => {
                    let ty = draft.value_type(result);
                    draft.set_constant(inst, Constant::Int(ty, bits & ty.mask()));
                },
                | Rewrite::Inst(data) => draft.replace_inst(inst, data),
                | Rewrite::Not(value) => {
                    let one = draft.add_constant(Constant::Int(Type::I1, 1));
                    draft.replace_inst(
                        inst,
                        InstData::Binary {
                            opcode: Opcode::Xor,
                            ty:     Type::I1,
                            flags:  InstFlags::empty(),
                            args:   [value, one],
                        },
                    );
                },
                | Rewrite::AddConstant { value, bits } => {
                    let ty = draft.value_type(value);
                    let constant = draft.add_constant(Constant::Int(ty, bits & ty.mask()));
                    draft.replace_inst(
                        inst,
                        InstData::Binary {
                            opcode: Opcode::Iadd,
                            ty,
                            flags: InstFlags::empty(),
                            args: [value, constant],
                        },
                    );
                },
                | Rewrite::RangeCheck {
                    value,
                    low,
                    span,
                    inside,
                } => {
                    let ty = draft.value_type(value);
                    let low = draft.add_constant(Constant::Int(ty, low));
                    let span = draft.add_constant(Constant::Int(ty, span));
                    let position = draft
                        .block(block)
                        .insts
                        .iter()
                        .position(|&other| other == inst)
                        .expect("the instruction is in its block");
                    let offset = draft
                        .add_inst(
                            block,
                            position,
                            InstData::Binary {
                                opcode: Opcode::Isub,
                                ty,
                                flags: InstFlags::empty(),
                                args: [value, low],
                            },
                            Some(ty),
                        )
                        .expect("`isub` has a result");
                    index = position + 2;
                    draft.replace_inst(
                        inst,
                        InstData::IntCompare {
                            cond: if inside { IntCC::Ule } else { IntCC::Ugt },
                            ty,
                            args: [offset, span],
                        },
                    );
                },
            }
            changed = true;
        }
        changed |= combine_branch(draft, block, report);
    }
    if changed {
        draft.sweep();
    }
    changed
}

/// What an instruction becomes.
#[derive(Clone, Copy, Debug)]
enum Rewrite {
    /// An existing value that dominates every use.
    Value(Value),
    /// A constant of the instruction's type.
    Constant(u128),
    /// A new record for the instruction, with the same result type.
    Inst(InstData),
    /// `xor value, true`.
    Not(Value),
    /// `iadd value, bits`, without flags.
    AddConstant { value: Value, bits: u128 },
    /// `icmp ule (isub value, low), span` when `inside`, else `ugt`.
    RangeCheck {
        value:  Value,
        low:    u128,
        span:   u128,
        inside: bool,
    },
}

/// The rewrite of an instruction with record `data`, if a rule applies.
fn combine(draft: &Draft<'_, '_>, data: InstData) -> Option<Rewrite> {
    match data {
        | InstData::IntCompare {
            cond,
            ty,
            args: [a, b],
        } => compare_boolean(draft, cond, ty, a, b),
        | InstData::Binary {
            opcode: Opcode::Xor,
            ty: Type::I1,
            args: [a, b],
            ..
        } => {
            let (other, constant) = constant_operand(draft, a, b)?;
            if constant != 1 {
                return None;
            }
            not(draft, other)
        },
        | InstData::Binary {
            opcode: opcode @ (Opcode::And | Opcode::Or),
            ty: Type::I1,
            args: [a, b],
            ..
        } => range_check(draft, a, b, opcode == Opcode::And),
        | InstData::Select {
            ty,
            args: [cond, a, b],
        } => select(draft, ty, cond, a, b),
        | InstData::Binary {
            opcode: opcode @ (Opcode::Iadd | Opcode::Isub),
            ty,
            args: [a, b],
            ..
        } if ty.is_int() => reassociate(draft, opcode, ty, a, b),
        | _ => None,
    }
}

/// Rule 1: a boolean, or its zero extension, compared with a constant.
fn compare_boolean(
    draft: &Draft<'_, '_>,
    cond: IntCC,
    ty: Type,
    a: Value,
    b: Value,
) -> Option<Rewrite> {
    if !matches!(cond, IntCC::Eq | IntCC::Ne) || !ty.is_int() {
        return None;
    }
    let (other, constant) = constant_operand(draft, a, b)?;
    let boolean = if ty == Type::I1 {
        other
    } else {
        match definition(draft, other)? {
            | InstData::Unary {
                opcode: Opcode::Zext,
                arg,
                ..
            } if draft.value_type(draft.resolve(arg)) == Type::I1 => draft.resolve(arg),
            | _ => return None,
        }
    };
    match (cond, constant) {
        | (IntCC::Ne, 0) | (IntCC::Eq, 1) => Some(Rewrite::Value(boolean)),
        | (IntCC::Eq, 0) | (IntCC::Ne, 1) => not(draft, boolean).or(Some(Rewrite::Not(boolean))),
        // A zero extension of a boolean is neither 0 nor 1 only for other
        // constants: equality is false and inequality true.
        | (IntCC::Eq, _) => Some(Rewrite::Constant(0)),
        | (IntCC::Ne, _) => Some(Rewrite::Constant(1)),
        | _ => None,
    }
}

/// Rule 2: the negation of `value` without a new `xor`, if it has one: the
/// inverse comparison, or the operand of a negation.
fn not(draft: &Draft<'_, '_>, value: Value) -> Option<Rewrite> {
    match definition(draft, value)? {
        | InstData::IntCompare { cond, ty, args } => Some(Rewrite::Inst(InstData::IntCompare {
            cond: inverse(cond),
            ty,
            args,
        })),
        | InstData::Binary {
            opcode: Opcode::Xor,
            ty: Type::I1,
            args: [a, b],
            ..
        } => match constant_operand(draft, a, b)? {
            | (other, 1) => Some(Rewrite::Value(other)),
            | _ => None,
        },
        | _ => None,
    }
}

/// Rules 3 and 4 for `select`.
fn select(draft: &Draft<'_, '_>, ty: Type, cond: Value, a: Value, b: Value) -> Option<Rewrite> {
    let constant = |value| match draft.constant(value) {
        | Some(Constant::Int(_, bits)) => Some(bits),
        | _ => None,
    };
    if ty == Type::I1 {
        match (constant(a), constant(b)) {
            | (Some(1), Some(0)) => return Some(Rewrite::Value(draft.resolve(cond))),
            | (Some(0), Some(1)) => return not(draft, cond).or(Some(Rewrite::Not(cond))),
            | (_, Some(0)) =>
                if let Some(rewrite) = range_check(draft, cond, a, true) {
                    return Some(rewrite);
                },
            | (Some(1), _) =>
                if let Some(rewrite) = range_check(draft, cond, b, false) {
                    return Some(rewrite);
                },
            | _ => {},
        }
    }
    // A negated condition swaps the arms.
    if let Some(InstData::Binary {
        opcode: Opcode::Xor,
        ty: Type::I1,
        args: [x, y],
        ..
    }) = definition(draft, cond)
        && let Some((inner, 1)) = constant_operand(draft, x, y)
    {
        return Some(Rewrite::Inst(InstData::Select {
            ty,
            args: [inner, b, a],
        }));
    }
    None
}

/// Rule 4: `a and b` (`both`) or `a or b` of two comparisons of one value
/// with constants, if they bound one interval.
fn range_check(draft: &Draft<'_, '_>, a: Value, b: Value, both: bool) -> Option<Rewrite> {
    let first = bound(draft, a)?;
    let second = bound(draft, b)?;
    if first.value != second.value || first.signed != second.signed || first.ty != second.ty {
        return None;
    }
    let ty = first.ty;
    // For `or`, take the complements: `x < lo || x > hi` is the negation of
    // `x >= lo && x <= hi`.
    let (first, second) = if both {
        (first, second)
    } else {
        (first.complement()?, second.complement()?)
    };
    let ((Side::AtLeast(low), Side::AtMost(high)) | (Side::AtMost(high), Side::AtLeast(low))) =
        (first.kind, second.kind)
    else {
        return None;
    };
    let order = |bits: u128| {
        if first.signed {
            eval::signed(bits, ty)
        } else {
            i128::try_from(bits).unwrap_or(i128::MAX)
        }
    };
    if order(low) > order(high) {
        // An empty interval: `and` is false, `or` (its negation) is true.
        return Some(Rewrite::Constant(u128::from(!both)));
    }
    Some(Rewrite::RangeCheck {
        value: first.value,
        low,
        span: high.wrapping_sub(low) & ty.mask(),
        inside: both,
    })
}

/// A comparison of a value with a constant as one side of an interval.
#[derive(Clone, Copy, Debug)]
struct Bound {
    value:  Value,
    ty:     Type,
    signed: bool,
    kind:   Side,
}

/// Which side of an inclusive interval a [`Bound`] gives.
#[derive(Clone, Copy, Debug)]
enum Side {
    AtLeast(u128),
    AtMost(u128),
}

impl Bound {
    /// The bound of the values the comparison is false for.
    fn complement(self) -> Option<Self> {
        let (min, max) = self.limits();
        let kind = match self.kind {
            | Side::AtLeast(low) if low != min =>
                Side::AtMost(low.wrapping_sub(1) & self.ty.mask()),
            | Side::AtMost(high) if high != max => Side::AtLeast((high + 1) & self.ty.mask()),
            | _ => return None,
        };
        Some(Self { kind, ..self })
    }

    /// The least and greatest value of the order, as bits.
    fn limits(self) -> (u128, u128) {
        let mask = self.ty.mask();
        if self.signed {
            let sign = 1 << (self.ty.bits() - 1);
            (sign, mask >> 1)
        } else {
            (0, mask)
        }
    }
}

/// `value` as a [`Bound`], if it is a comparison of an integer with a
/// constant by an ordering condition.
fn bound(draft: &Draft<'_, '_>, value: Value) -> Option<Bound> {
    let InstData::IntCompare {
        cond,
        ty,
        args: [a, b],
    } = definition(draft, value)?
    else {
        return None;
    };
    // `i128` bounds do not fit the signed order used to compare them.
    if !ty.is_int() || ty == Type::I1 || ty == Type::I128 {
        return None;
    }
    let (subject, constant, cond) = match (draft.constant(a), draft.constant(b)) {
        | (None, Some(Constant::Int(_, bits))) => (a, bits, cond),
        | (Some(Constant::Int(_, bits)), None) => (b, bits, swapped(cond)),
        | _ => return None,
    };
    let signed = matches!(cond, IntCC::Slt | IntCC::Sle | IntCC::Sgt | IntCC::Sge);
    let mut bound = Bound {
        value: draft.resolve(subject),
        ty,
        signed,
        kind: Side::AtLeast(0),
    };
    let (min, max) = bound.limits();
    let mask = ty.mask();
    bound.kind = match cond {
        | IntCC::Sge | IntCC::Uge => Side::AtLeast(constant),
        | IntCC::Sle | IntCC::Ule => Side::AtMost(constant),
        | IntCC::Sgt | IntCC::Ugt if constant != max => Side::AtLeast((constant + 1) & mask),
        | IntCC::Slt | IntCC::Ult if constant != min =>
            Side::AtMost(constant.wrapping_sub(1) & mask),
        | _ => return None,
    };
    Some(bound)
}

/// Rule 5: an addition or subtraction of a constant applied to another
/// becomes one addition of the combined constant.
fn reassociate(
    draft: &Draft<'_, '_>,
    opcode: Opcode,
    ty: Type,
    a: Value,
    b: Value,
) -> Option<Rewrite> {
    let mask = ty.mask();
    let addend = |opcode: Opcode, value: Value| match draft.constant(value) {
        | Some(Constant::Int(_, bits)) if opcode == Opcode::Isub =>
            Some(bits.wrapping_neg() & mask),
        | Some(Constant::Int(_, bits)) => Some(bits),
        | _ => None,
    };
    let outer = addend(opcode, b)?;
    let Some(InstData::Binary {
        opcode: inner_opcode @ (Opcode::Iadd | Opcode::Isub),
        ty: inner_ty,
        args: [inner, inner_constant],
        ..
    }) = definition(draft, a)
    else {
        return None;
    };
    if inner_ty != ty || draft.constant(inner).is_some() {
        return None;
    }
    let inner_addend = addend(inner_opcode, inner_constant)?;
    Some(Rewrite::AddConstant {
        value: draft.resolve(inner),
        bits:  inner_addend.wrapping_add(outer) & mask,
    })
}

/// Rewrites `brif (xor c, true), a, b` as `brif c, b, a`. Returns whether it
/// did.
fn combine_branch(
    draft: &mut Draft<'_, '_>,
    block: Block,
    report: &mut OptimizationReport,
) -> bool {
    let crate::optimizer::draft::Term::Brif { cond, .. } = &draft.block(block).term else {
        return false;
    };
    let Some(InstData::Binary {
        opcode: Opcode::Xor,
        ty: Type::I1,
        args: [x, y],
        ..
    }) = definition(draft, *cond)
    else {
        return false;
    };
    let Some((inner, 1)) = constant_operand(draft, x, y) else {
        return false;
    };
    if !report.allow() {
        return false;
    }
    if let crate::optimizer::draft::Term::Brif {
        cond,
        then,
        otherwise,
    } = &mut draft.block_mut(block).term
    {
        *cond = inner;
        std::mem::swap(then, otherwise);
    }
    true
}

/// The record defining `value`, if an unfolded instruction defines it.
fn definition(draft: &Draft<'_, '_>, value: Value) -> Option<InstData> {
    let ValueDef::Result(inst) = draft.value_def(draft.resolve(value)) else {
        return None;
    };
    if draft.is_removed(inst) || draft.inst_constant(inst).is_some() {
        return None;
    }
    Some(*draft.inst(inst))
}

/// The non-constant operand and the constant of a pair with one integer
/// constant.
fn constant_operand(draft: &Draft<'_, '_>, a: Value, b: Value) -> Option<(Value, u128)> {
    match (draft.constant(a), draft.constant(b)) {
        | (None, Some(Constant::Int(_, bits))) => Some((draft.resolve(a), bits)),
        | (Some(Constant::Int(_, bits)), None) => Some((draft.resolve(b), bits)),
        | _ => None,
    }
}

const fn inverse(cond: IntCC) -> IntCC {
    match cond {
        | IntCC::Eq => IntCC::Ne,
        | IntCC::Ne => IntCC::Eq,
        | IntCC::Slt => IntCC::Sge,
        | IntCC::Sle => IntCC::Sgt,
        | IntCC::Sgt => IntCC::Sle,
        | IntCC::Sge => IntCC::Slt,
        | IntCC::Ult => IntCC::Uge,
        | IntCC::Ule => IntCC::Ugt,
        | IntCC::Ugt => IntCC::Ule,
        | IntCC::Uge => IntCC::Ult,
    }
}

const fn swapped(cond: IntCC) -> IntCC {
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
