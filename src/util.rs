use std::{borrow::Cow, path::Path};

use rustc_hash::FxBuildHasher;

pub(crate) mod dedup_arena;
pub(crate) mod last_entry;
pub(crate) mod shared;
pub(crate) mod small_queue;
pub(crate) mod stack_queue;
pub(crate) mod string_cache;
pub(crate) mod vector_slice;

pub(crate) type HashMap<K, V> = hashbrown::HashMap<K, V, FxBuildHasher>;
pub(crate) type HashSet<K> = hashbrown::HashSet<K, FxBuildHasher>;

#[expect(
    dead_code,
    reason = "We aren't using this currently, but it's a very useful trait to have for getting \
              the right variance."
)]
pub(crate) trait Captures<U> {}

impl<T: ?Sized, U> Captures<U> for T {}

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
