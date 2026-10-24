//! Reachable CFG edges must agree on the stack of logical lexical scopes.
//! C99: §6.2.4p5, p. 32; PDF p. 44. This checks representation structure,
//! not source preconditions such as overflow or definite initialization.

use super::model::{
    Error,
    Failure,
    Function,
    InstructionKind,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// A persistent stack node is identified by its Enter instruction's pc + 1.
/// Backedges reuse it; they cannot quietly create a new nested lifetime.
pub(super) fn scope_flow(function: &Function<'_>) -> Result<(), Error> {
    let bad = || Error::new(Failure::InvalidIr, None);
    let arena = Bump::new();
    let mut incoming = ArenaVec::new_in(&arena);
    incoming.resize(function.code.len(), None);
    let mut parents = ArenaVec::new_in(&arena);
    parents.resize(function.code.len() + 1, (0_usize, 0_usize));
    let mut work = ArenaVec::new_in(&arena);
    work.push((0_usize, 0_usize));
    while let Some((pc, mut top)) = work.pop() {
        let op = function.code.get(pc).ok_or_else(bad)?;
        if let Some(previous) = incoming[pc] {
            if previous != top {
                return Err(bad());
            }
            continue;
        }
        incoming[pc] = Some(top);
        match op.kind {
            | InstructionKind::Enter(scope) => {
                let mut ancestor = top;
                while ancestor != 0 {
                    if matches!(function.code[ancestor - 1].kind, InstructionKind::Enter(s) if s == scope)
                    {
                        return Err(bad());
                    }
                    ancestor = parents[ancestor].0;
                }
                parents[pc + 1] = (top, parents[top].1 + 1);
                top = pc + 1;
            },
            | InstructionKind::Exit => {
                if top == 0 {
                    return Err(bad());
                }
                top = parents[top].0;
            },
            | InstructionKind::Jump { label, keep } => {
                if keep > parents[top].1 {
                    return Err(bad());
                }
                while parents[top].1 > keep {
                    top = parents[top].0;
                }
                work.push((function.labels[label], top));
                continue;
            },
            | InstructionKind::Branch {
                then, otherwise, ..
            } => {
                work.push((function.labels[then], top));
                work.push((function.labels[otherwise], top));
                continue;
            },
            | InstructionKind::Return(_) | InstructionKind::Fallthrough => continue,
            | _ => {},
        }
        work.push((pc + 1, top));
    }
    Ok(())
}
