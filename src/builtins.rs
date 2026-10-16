//! Phase-neutral builtin contracts, indexed by exact spelling and stable ID.
//! Read [`lookup`] and [`BuiltinDescriptor`] first. `index.rs` assigns IDs and
//! supplies the sorted name index; `scalar.rs`, `library.rs`, `math.rs`,
//! `atomic.rs` and `checked.rs` hold scalar operations, general library
//! aliases, C99 real math, atomic contracts and the object-size-checking
//! aliases of fortified headers. `contract.rs` defines closed rules, operations
//! and effects. Semantic handlers check these contracts and retain instances;
//! lowering owns executable implementations. x86, vector and type-generic
//! shards are migration M6 work. `external.rs` is the external-symbol contract:
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
