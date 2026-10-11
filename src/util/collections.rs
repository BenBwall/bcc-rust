pub(crate) mod dedup_arena;

pub(crate) mod region_bit_set;

pub(crate) mod region_vec;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "A test-only reference-counted heap container; non-test builds do not compile it."
)]
pub(crate) mod shared;

pub(crate) mod string_cache;

pub(crate) mod vector_slice;
