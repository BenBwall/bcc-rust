use rustc_hash::FxBuildHasher;

pub(crate) mod arena;
/// Compile checks import the crate-private allocator from its source file.
///
/// ```compile_fail,E0080
/// # #[path = "util/bump.rs"]
/// # mod bump;
/// let arena = bump::Bump::new();
/// arena.alloc(String::from("owned"));
/// ```
pub(crate) mod bump;
pub(crate) mod byte_scan;
pub(crate) mod dedup_arena;
pub(crate) mod packed;
pub(crate) mod region_bit_set;
pub(crate) mod region_vec;
pub(crate) mod shared;
pub(crate) mod string_cache;
pub(crate) mod vector_slice;
pub(crate) mod vm;

pub(crate) type HashMap<K, V> = hashbrown::HashMap<K, V, FxBuildHasher>;
