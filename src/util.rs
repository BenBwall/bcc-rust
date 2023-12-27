use std::borrow::Cow;

pub(crate) mod string_cache;

pub(crate) trait Captures<U> {}

impl<T: ?Sized, U> Captures<U> for T {}

pub(crate) fn vec_to_string_lossy(vec: Vec<u8>) -> String {
    match String::from_utf8_lossy(&vec) {
        | Cow::Borrowed(..) => unsafe { String::from_utf8_unchecked(vec) },
        | Cow::Owned(s) => s,
    }
}
