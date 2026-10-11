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
    start_plus_one: NonZeroU32,
    /// The low 30 bits are the range length; the high two identify its arena.
    tagged_length:  u32,
    // We use `AtomicPtr` so that we're `Send` and `Sync`.
    _marker:        PhantomData<AtomicPtr<T>>,
}

impl<T> VectorSlice<T> {
    pub(crate) fn with_arena_tag(start_index: u32, end_index: u32, arena_tag: u32) -> Self {
        let length = end_index
            .checked_sub(start_index)
            .expect("a slice cannot end before it starts");
        assert!(length <= Self::MAX_LENGTH, "source range length overflow");
        assert!(arena_tag < 4, "source arena tag overflow");
        Self {
            start_plus_one: start_index
                .checked_add(1)
                .and_then(NonZeroU32::new)
                .expect("a slice cannot start at index u32::MAX"),
            tagged_length:  length | (arena_tag << Self::LENGTH_BITS),
            _marker:        PhantomData,
        }
    }

    pub(crate) fn start_index(self) -> u32 {
        self.start_plus_one.get() - 1
    }

    pub(crate) fn length(self) -> u32 {
        self.tagged_length & Self::MAX_LENGTH
    }

    pub(crate) fn arena_tag(self) -> u32 {
        self.tagged_length >> Self::LENGTH_BITS
    }

    /// # Panics
    ///
    /// If `start_index` is `u32::MAX` or the range is longer than 30 bits.
    pub(crate) fn new(start_index: u32, end_index: u32) -> Self {
        Self::with_arena_tag(start_index, end_index, 0)
    }

    pub(crate) fn empty() -> Self {
        Self::default()
    }
}

impl<T> VectorSlice<T> {
    const LENGTH_BITS: u32 = 30;
    pub(crate) const MAX_LENGTH: u32 = (1 << Self::LENGTH_BITS) - 1;
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

impl<T> Debug for VectorSlice<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VectorSlice")
            .field("start_index", &self.start_index())
            .field("length", &self.length())
            .field("arena_tag", &self.arena_tag())
            .finish()
    }
}

impl<T> Hash for VectorSlice<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.start_plus_one.hash(state);
        self.tagged_length.hash(state);
    }
}

impl<T> PartialEq for VectorSlice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.start_plus_one == other.start_plus_one && self.tagged_length == other.tagged_length
    }
}

impl<T> Eq for VectorSlice<T> {}

impl<T> Default for VectorSlice<T> {
    fn default() -> Self {
        // An index no arena reaches, so the empty slice never aliases one.
        Self::new(u32::MAX - 1, u32::MAX - 1)
    }
}
