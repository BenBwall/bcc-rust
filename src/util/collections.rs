//! Collections keep compiler values in arena or virtual-memory storage.
//! Region vectors and bit sets grow without relocating their reservations.
//! Deduplication and string interning assign stable identities to repeated
//! values. Vector slices store compact ranges over separate backing storage.
//!
//! Read [`region_vec::RegionVec::push`],
//! [`dedup_arena::DedupArena::intern_by`],
//! and [`string_cache::StringCache::intern`].
//!
//! Files by role:
//! - Region storage: `collections/region_vec.rs`,
//!   `collections/region_bit_set.rs`.
//! - Interning: `collections/dedup_arena.rs`, `collections/string_cache.rs`.
//! - Range representation: `collections/vector_slice.rs`.
//! - Shared test values: `collections/shared.rs`.

// Region storage
pub(crate) mod region_bit_set;
pub(crate) mod region_vec;

// Interning
pub(crate) mod dedup_arena;
pub(crate) mod string_cache;

// Range representation
pub(crate) mod vector_slice;

// Test storage
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "A test-only reference-counted heap container; non-test builds do not compile it."
)]
pub(crate) mod shared;
