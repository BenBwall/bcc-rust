use std::{
    borrow::Borrow,
    hash::{
        BuildHasher,
        Hash,
    },
    ops::Deref,
};

use hashbrown::raw::RawTable;

#[allow(dead_code)]
pub(crate) struct DedupArena<T, H> {
    indices: RawTable<u32>,
    data:    Vec<T>,
    hasher:  H,
}

impl<T, H> DedupArena<T, H> {
    pub(crate) fn with_hasher(hasher: H) -> Self {
        Self {
            indices: RawTable::new(),
            data: Vec::new(),
            hasher,
        }
    }

    pub(crate) fn new() -> Self
    where
        H: Default,
    {
        Self::with_hasher(H::default())
    }

    #[allow(dead_code)]
    pub(crate) fn with_capacity_and_hasher(capacity: usize, hasher: H) -> Self {
        Self {
            indices: RawTable::with_capacity(capacity),
            data: Vec::with_capacity(capacity),
            hasher,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_capacity(capacity: usize) -> Self
    where
        H: Default,
    {
        Self::with_capacity_and_hasher(capacity, H::default())
    }

    /// Intern a value into the arena, returning the index of the value. If the
    /// value is already in the arena, the index of the existing value is
    /// returned. The value is not cloned. Value is dropped if it already
    /// exists. Returns `Err` if the value is already in the arena. `Ok`
    /// otherwise.
    #[allow(dead_code)]
    pub(crate) fn try_intern(&mut self, value: T) -> Result<u32, u32>
    where
        H: BuildHasher,
        T: Hash + Eq,
    {
        let hash = self.hasher.hash_one(&value);
        let index = u32::try_from(self.data.len()).expect("DedupArena: Too many values.");

        match self.indices.find_or_find_insert_slot(
            hash,
            |x| self.data[*x as usize] == value,
            |x| self.hasher.hash_one(&self.data[*x as usize]),
        ) {
            | Ok(bucket) =>
            // SAFETY: The bucket is guaranteed to be valid because it was returned by
            // `find_or_find_insert_slot`.
            unsafe { Err(*bucket.as_ref()) },
            | Err(slot) => unsafe {
                // SAFETY:
                // Based on the implementation of HashMap in hashbrown. Inserting into the slot
                // is valid because it was returned by `find_or_find_insert_slot`.
                self.data.push(value);
                _ = self.indices.insert_in_slot(hash, slot, index);
                Ok(index)
            },
        }
    }

    /// SAFETY: The caller must ensure that the values in the arena remain
    /// unique.
    #[allow(dead_code)]
    pub(crate) unsafe fn as_mut_slice(&mut self) -> &mut [T] {
        self.data.as_mut_slice()
    }

    /// Intern a value into the arena, returning the index of the value. Returns
    /// the index of the old value if the value is already in the arena. Value
    /// is dropped if it already exists. Value is not cloned.
    #[allow(dead_code)]
    pub(crate) fn intern(&mut self, value: T) -> u32
    where
        H: BuildHasher,
        T: Hash + Eq,
    {
        match self.try_intern(value) {
            | Err(index) | Ok(index) => index,
        }
    }
}

impl<T, H> std::ops::Index<u32> for DedupArena<T, H> {
    type Output = T;

    fn index(&self, index: u32) -> &T {
        &self.data[index as usize]
    }
}

impl<T, H> std::ops::IndexMut<u32> for DedupArena<T, H> {
    fn index_mut(&mut self, index: u32) -> &mut T {
        &mut self.data[index as usize]
    }
}

impl<T, H> Deref for DedupArena<T, H> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.data
    }
}

impl<T, H> Default for DedupArena<T, H>
where
    H: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, H> AsRef<[T]> for DedupArena<T, H> {
    fn as_ref(&self) -> &[T] {
        &self.data
    }
}

impl<T, H> Borrow<[T]> for DedupArena<T, H> {
    fn borrow(&self) -> &[T] {
        &self.data
    }
}
