pub(crate) mod string_cache;

pub(crate) trait Captures<U> {}

impl<T: ?Sized, U> Captures<U> for T {}
