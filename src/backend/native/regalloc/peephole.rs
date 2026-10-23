//! Peepholes on the allocated function, where registers are known: a copy
//! followed by an addition to the copy becomes one `lea`, an addition to
//! the result of a `lea` folds into it, and a copy straight back after a
//! copy goes.
//!
//! Coalescing already turns `mov d, a; add d, b` into one `add` when `a`
//! dies there; when `a` lives on, the copy stays, and a three-operand `lea`
//! does both. An `add` sets the flags and a `lea` does not, so a pair is
//! combined only where no later instruction reads the flags before another
//! one sets them. A 32-bit `add` also clears the upper half of its register,
//! which a 64-bit `lea` does not; a 32-bit addition computes a value of at
//! most 32 bits, whose upper bits are unspecified (see "Narrow integer
//! representation" in `docs/design/native-backend.md`), and an `i1` sum is
//! masked with `andl $1` right after, so that half is never read.

use crate::backend::native::mir::{
    AluOp,
    Base,
    Index,
    MFunction,
    MInst,
    Mem,
    Operand,
    Width,
};

/// Combines the copy-and-add and `lea`-and-add pairs of `func` and deletes
/// copies back.
pub(super) fn combine(func: &mut MFunction<'_>) {
    for block in &mut func.blocks {
        let insts = &mut block.insts;
        let mut index = 0;
        while index + 1 < insts.len() {
            if copies_back(insts[index], insts[index + 1]) {
                _ = insts.remove(index + 1);
                continue;
            }
            if let Some(lea) = combined(insts[index], insts[index + 1])
                && !flags_read_after(&insts[index + 2..])
            {
                insts[index] = lea;
                _ = insts.remove(index + 1);
                // The `lea` may combine with the next addition too.
                continue;
            }
            index += 1;
        }
    }
}

/// The `lea` that does what `first` and then `second` do, if there is one.
fn combined(first: MInst, second: MInst) -> Option<MInst> {
    let MInst::Alu {
        op: op @ (AluOp::Add | AluOp::Sub),
        width: Width::B32 | Width::B64,
        dst,
        src,
    } = second
    else {
        return None;
    };
    let addr = match first {
        | MInst::Mov {
            width: Width::B64,
            dst: copy,
            src: Operand::Reg(from),
        } if copy == dst && from != dst => Mem::new(Base::Reg(from), 0),
        // A GOT entry's operand has no displacement to fold into.
        | MInst::Lea { dst: result, addr }
            if result == dst && !matches!(addr.base, Base::Got(_) | Base::GotTpoff(_)) =>
            addr,
        | _ => return None,
    };
    let addr = match (op, src) {
        | (AluOp::Add, Operand::Imm(imm)) => Mem {
            disp: addr.disp.checked_add(imm)?,
            ..addr
        },
        | (AluOp::Sub, Operand::Imm(imm)) => Mem {
            disp: addr.disp.checked_sub(imm)?,
            ..addr
        },
        | (AluOp::Add, Operand::Reg(reg))
            if reg != dst && addr.index.is_none() && matches!(addr.base, Base::Reg(_)) =>
            Mem {
                index: Some(Index { reg, scale: 1 }),
                ..addr
            },
        | _ => return None,
    };
    Some(MInst::Lea { dst, addr })
}

/// Whether `second` copies back what `first` copied: `mov b, a; mov a, b`.
fn copies_back(first: MInst, second: MInst) -> bool {
    match (first, second) {
        | (
            MInst::Mov {
                width: Width::B64,
                dst,
                src: Operand::Reg(src),
            },
            MInst::Mov {
                width: Width::B64,
                dst: back,
                src: Operand::Reg(from),
            },
        )
        | (
            MInst::XMov {
                width: Width::B64,
                dst,
                src: Operand::Reg(src),
            },
            MInst::XMov {
                width: Width::B64,
                dst: back,
                src: Operand::Reg(from),
            },
        ) => back == src && from == dst,
        | _ => false,
    }
}

/// Whether an instruction of `rest` reads the flags before one overwrites
/// them. Flags never live across blocks.
fn flags_read_after(rest: &[MInst]) -> bool {
    for inst in rest {
        let (reads, writes) = inst.flags();
        if reads {
            return true;
        }
        if writes {
            return false;
        }
    }
    false
}
