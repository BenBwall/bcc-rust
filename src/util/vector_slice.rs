use std::{
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    marker::PhantomData,
    sync::atomic::AtomicPtr,
};

pub(crate) struct VectorSlice<T> {
    pub(crate) start_index: u32,
    pub(crate) length:      u32,
    // We use `AtomicPtr` so we're `Send` and `Sync`.
    _marker:                PhantomData<AtomicPtr<T>>,
}

impl<T> Copy for VectorSlice<T> {}
impl<T> Clone for VectorSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Debug for VectorSlice<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("VectorSlice")
            .field("start_index", &self.start_index)
            .field("length", &self.length)
            .finish()
    }
}

impl<T> Hash for VectorSlice<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.start_index.hash(state);
        self.length.hash(state);
    }
}

impl<T> PartialEq for VectorSlice<T> {
    fn eq(&self, other: &Self) -> bool {
        self.start_index == other.start_index && self.length == other.length
    }
}

impl<T> Eq for VectorSlice<T> {}

impl<T> VectorSlice<T> {
    pub(crate) fn new(start_index: u32, length: u32) -> Self {
        Self {
            start_index,
            length,
            _marker: PhantomData,
        }
    }
}

impl<T> Default for VectorSlice<T> {
    fn default() -> Self {
        Self {
            start_index: u32::MAX,
            length:      u32::MAX,
            _marker:     PhantomData,
        }
    }
}
