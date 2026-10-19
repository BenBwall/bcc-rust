//! ABI substitutions ignore storage alignment throughout a derived type,
//! while retaining exact qualifiers, nominal types and function prototypes.
//! GNU aligned is a storage contract, not part of the linked C type.
//! C99: §6.2.7p1-2, p. 40; PDF p. 52; GCC Common Type Attributes, aligned.

use super::super::{
    Analyzer,
    TypeId,
    TypeKind,
};
use crate::util::bump::ArenaVec;

impl Analyzer<'_, '_, '_> {
    pub(super) fn same_abi_type(&self, left: TypeId, right: TypeId) -> bool {
        if left.with_alignment(0) == right.with_alignment(0) {
            return true;
        }
        let mut work = ArenaVec::new_in(self.scratch);
        work.push((left, right));
        while let Some((left, right)) = work.pop() {
            let (left, right) = (left.with_alignment(0), right.with_alignment(0));
            if left == right {
                continue;
            }
            if left.qualifiers != right.qualifiers {
                return false;
            }
            let (a, b) = (self.types.nodes[left.index], self.types.nodes[right.index]);
            match (a, b) {
                | (TypeKind::Pointer(a), TypeKind::Pointer(b))
                | (TypeKind::Atomic(a), TypeKind::Atomic(b)) => work.push((a, b)),
                | (TypeKind::Array(a, ab), TypeKind::Array(b, bb)) if ab == bb => work.push((a, b)),
                | (
                    TypeKind::Vector {
                        element: a,
                        count: ac,
                        ..
                    },
                    TypeKind::Vector {
                        element: b,
                        count: bc,
                        ..
                    },
                ) if ac == bc => work.push((a, b)),
                | (
                    TypeKind::Function {
                        result: a,
                        parameters: ap,
                        prototype: aproto,
                        variadic: av,
                    },
                    TypeKind::Function {
                        result: b,
                        parameters: bp,
                        prototype: bproto,
                        variadic: bv,
                    },
                ) if aproto == bproto && av == bv && ap.len() == bp.len() => {
                    work.push((a, b));
                    work.extend(ap.iter().copied().zip(bp.iter().copied()));
                },
                | _ if a == b => {},
                | _ => return false,
            }
        }
        true
    }
}
