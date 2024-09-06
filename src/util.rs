use std::{
    borrow::Cow,
    hash::BuildHasherDefault,
    path::Path,
};

use rustc_hash::FxHasher;

pub(crate) mod dedup_arena;
pub(crate) mod shared;
pub(crate) mod small_queue;
pub(crate) mod stack_queue;
pub(crate) mod string_cache;
pub(crate) mod vector_slice;

pub(crate) type HashMap<K, V> = hashbrown::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub(crate) type HashSet<K> = hashbrown::HashSet<K, BuildHasherDefault<FxHasher>>;

#[expect(dead_code, reason = "We aren't using this currently, but it's a very useful trait to have for getting the right variance.")]
pub(crate) trait Captures<U> {}

impl<T: ?Sized, U> Captures<U> for T {}

pub(crate) fn vec_to_string_lossy(vec: Vec<u8>) -> String {
    match String::from_utf8_lossy(&vec) {
        | Cow::Borrowed(..) => unsafe { String::from_utf8_unchecked(vec) },
        | Cow::Owned(s) => s,
    }
}

pub(crate) fn read_to_string_lossy(path: impl AsRef<Path>) -> std::io::Result<String> {
    let buf = std::fs::read(path)?;
    Ok(vec_to_string_lossy(buf))
}

#[allow(dead_code, reason = "This might not be currently used, but it's useful for indicating a code path is cold.")]
#[inline]
#[cold]
fn cold() {}

#[allow(dead_code, reason = "This might not be currently used, but it's useful for indicating a code path is likely.")]
#[inline]
pub(crate) fn likely(b: bool) -> bool {
    if !b {
        cold();
    }
    b
}

#[allow(dead_code, reason = "This might not be currently used, but it's useful for indicating a code path is unlikely.")]
#[inline]
pub(crate) fn unlikely(b: bool) -> bool {
    if b {
        cold();
    }
    b
}
