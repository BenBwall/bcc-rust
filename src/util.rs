use std::{
    borrow::Cow,
    path::Path,
};

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
pub(crate) mod chunked_queue;
pub(crate) mod dedup_arena;
pub(crate) mod last_entry;
pub(crate) mod packed;
pub(crate) mod shared;
pub(crate) mod string_cache;
pub(crate) mod vector_slice;

pub(crate) type HashMap<K, V> = hashbrown::HashMap<K, V, FxBuildHasher>;
pub(crate) type HashSet<K> = hashbrown::HashSet<K, FxBuildHasher>;

pub(crate) fn vec_to_string_lossy(vec: Vec<u8>) -> String {
    match String::from_utf8_lossy(&vec) {
        // SAFETY: If `[String::from_utf8_lossy]` returns `Cow::Borrowed`, then the input was
        // correct UTF-8.
        | Cow::Borrowed(..) => unsafe { String::from_utf8_unchecked(vec) },
        | Cow::Owned(s) => s,
    }
}

pub(crate) fn read_to_string_lossy(path: impl AsRef<Path>) -> std::io::Result<String> {
    let buf = std::fs::read(path)?;
    Ok(vec_to_string_lossy(buf))
}
