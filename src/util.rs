use std::{
    borrow::Cow,
    path::Path,
};

pub(crate) mod input;
pub(crate) mod string_cache;
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
