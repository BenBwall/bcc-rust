//! Phase-neutral builtin contracts, indexed by exact spelling and stable ID.
//! Every builtin spelling and keyword form the compiler recognizes has exactly
//! one descriptor here; semantic lookup, the family checkers and
//! preprocessing's `__has_builtin` all start from [`lookup`].
//! Read [`lookup`] and [`BuiltinDescriptor`] first. `index.rs` assigns IDs and
//! supplies the sorted name index; `scalar.rs`, `library.rs`, `math.rs`,
//! `atomic.rs` and `checked.rs` hold scalar operations, general library
//! aliases, C99 real math, atomic contracts and the object-size-checking
//! aliases of fortified headers. `x86.rs` is generated from the resource
//! headers' x86 calls; `vector.rs` and `type_generic.rs` hold the vector and
//! type-generic families, whose calls their own semantic handlers type.
//! `contract.rs` defines closed rules, operations and effects. Semantic
//! handlers check these contracts and retain instances; lowering owns
//! executable implementations. `external.rs` is the external-symbol contract:
//! the library functions that return twice or restore an environment, which
//! every phase reads from there instead of matching spellings itself. No
//! syntax, semantic type IDs or IR types belong here.
//!
//! C99: §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84 (call contracts);
//! §7.1.4, pp. 166-167; PDF pp. 178-179 (library equivalence).
//! Scalar intrinsics are GNU extensions documented by their contract shard.

// Contract vocabulary
mod contract;

// Static index and contract shards
mod atomic;
mod checked;
mod external;
mod index;
mod library;
mod math;
mod scalar;
mod type_generic;
mod vector;
mod x86;

pub(crate) use contract::*;
pub(crate) use external::{
    ExternalContract,
    external_contract,
    returns_twice,
};
#[cfg(test)]
pub(crate) use index::descriptors as test_descriptors;
pub(crate) use index::{
    BuiltinId,
    lookup,
};
pub(crate) use library::ordinary;
#[cfg(test)]
pub(crate) use x86::MERGED as test_merged_x86;
