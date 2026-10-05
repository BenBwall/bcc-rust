use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    marker::PhantomData,
    num::NonZeroU32,
    sync::atomic::AtomicPtr,
};

pub(crate) struct VectorSlice<T> {
    /// The start index plus one. Index `u32::MAX` is never stored, which
    /// lets `Option<VectorSlice<T>>` use zero as `None` and stay 8 bytes.
    start_plus_one:    NonZeroU32,
    pub(crate) length: u32,
    // We use `AtomicPtr` so that we're `Send` and `Sync`.
    _marker:           PhantomData<AtomicPtr<T>>,
}

// The niche that `start_plus_one` provides: syntax nodes hold many optional
// source vectors, and each must stay as small as a plain one.
const _: () = assert!(
    size_of::<Option<VectorSlice<u8>>>() == size_of::<VectorSlice<u8>>(),
    "an optional vector slice is as small as a vector slice"
);

impl<T> Copy for VectorSlice<T> {}
impl<T> Clone for VectorSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "The start is stored offset by one; Debug shows the real start."
)]
impl<T> Debug for VectorSlice<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VectorSlice")
            .field("start_index", &self.start_index())
            .field("length", &self.length)
            .finish()
    }
}

impl<T> Hash for VectorSlice<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.start_plus_one.hash(state);
        self.length.hash(state);
    }
}

impl<T> PartialEq for VectorSlice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.start_plus_one == other.start_plus_one && self.length == other.length
    }
}

impl<T> Eq for VectorSlice<T> {}

impl<T> VectorSlice<T> {
    /// # Panics
    ///
    /// If `start_index` is `u32::MAX`.
    pub(crate) fn new(start_index: u32, end_index: u32) -> Self {
        Self {
            start_plus_one: start_index
                .checked_add(1)
                .and_then(NonZeroU32::new)
                .expect("a slice cannot start at index u32::MAX"),
            length:         end_index - start_index,
            _marker:        PhantomData,
        }
    }

    pub(crate) fn start_index(self) -> u32 {
        self.start_plus_one.get() - 1
    }

    pub(crate) fn empty() -> Self {
        Self::default()
    }
}

impl<T> Default for VectorSlice<T> {
    fn default() -> Self {
        // An index no arena reaches, so the empty slice never aliases one.
        Self::new(u32::MAX - 1, u32::MAX - 1)
    }
}

pub(crate) trait UsizeExt {
    fn to_u32(self) -> u32;
}

impl UsizeExt for usize {
    fn to_u32(self) -> u32 {
        self.try_into().expect("VectorSlice length overflowed u32")
    }
}
