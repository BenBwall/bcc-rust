mod memory;

mod collections;

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

pub(crate) mod byte_scan;

pub(crate) mod packed;
