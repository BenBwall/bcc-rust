//! Compiler storage comes from arenas and fixed-address virtual-memory
//! regions. Collections build on those stores, byte scans classify source text,
//! and compact carriers represent indices and values. Each utility has its own
//! core operation rather than one shared driver.
//!
//! Read [`bump::Bump`] and [`bump::Bump::alloc`], then
//! [`region_vec::RegionVec::push`]. For source scanning, start with
//! [`byte_scan::identifier_run`].
//!
//! Files by role:
//! - Arena and virtual memory: `memory/bump.rs`, `memory/vm.rs`,
//!   `memory/arena_list.rs`.
//! - Collections: `collections/region_vec.rs`, `collections/region_bit_set.rs`,
//!   `collections/dedup_arena.rs`, `collections/string_cache.rs`,
//!   `collections/vector_slice.rs`, `collections/shared.rs` (tests only).
//! - Scanning: `byte_scan.rs`.
//! - Packed values: `packed.rs`.

// Storage
mod collections;
mod memory;

// Scanning
pub(crate) mod byte_scan;

// Representation
pub(crate) mod packed;

#[cfg(test)]
pub(crate) use collections::shared;
pub(crate) use collections::{
    dedup_arena,
    region_bit_set,
    region_vec,
    string_cache,
    vector_slice,
};
pub(crate) use memory::{
    arena_list,
    bump,
    vm,
};
