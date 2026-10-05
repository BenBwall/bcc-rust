//! Storage for 8-byte values at 4-byte alignment.

use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::{
        Hash,
        Hasher,
    },
};

/// A value stored with at most 4-byte alignment.
///
/// Tokens and constants carry `i64`, `u64`, and `f64` payloads next to
/// 4-byte handles; storing those payloads at their natural 8-byte alignment
/// would pad every token and expression. The value is only ever read and
/// written by copy, never through a reference.
#[derive(Clone, Copy)]
#[repr(C, packed(4))]
pub(crate) struct Packed<T: Copy>(T);

impl<T: Copy> Packed<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(value)
    }

    pub(crate) const fn get(self) -> T {
        self.0
    }
}

impl<T: Copy + PartialEq> PartialEq for Packed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl<T: Copy + Eq> Eq for Packed<T> {}

impl<T: Copy + Hash> Hash for Packed<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

impl<T: Copy + Debug> Debug for Packed<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.get().fmt(f)
    }
}

impl<T: Copy + Display> Display for Packed<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        self.get().fmt(f)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_macros,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use super::Packed;

    #[test]
    fn values_round_trip_at_four_byte_alignment() {
        assert_eq!(align_of::<Packed<u64>>(), 4);
        assert_eq!(size_of::<Packed<f64>>(), 8);
        assert_eq!(Packed::new(-7_i64).get(), -7);
        assert_eq!(Packed::new(u64::MAX), Packed::new(u64::MAX));
        assert_eq!(format!("{:?}", Packed::new(1.5_f64)), "1.5");
        assert_eq!(Packed::new(0.0_f64), Packed::new(-0.0_f64));
    }
}
