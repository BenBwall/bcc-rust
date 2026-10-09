//! An arena-backed interner: each distinct value is stored once and named by
//! a `u32` index in insertion order.

use std::{
    borrow::Borrow,
    hash::{
        BuildHasher,
        Hash,
    },
    ops::Deref,
};

use hashbrown::{
    HashTable,
    hash_table::Entry,
};

use super::bump::{
    ArenaVec,
    Bump,
};

pub(crate) struct DedupArena<'a, T, H> {
    indices: HashTable<u32, &'a Bump>,
    data:    ArenaVec<'a, T>,
    hasher:  H,
}

impl<'a, T, H> DedupArena<'a, T, H> {
    pub(crate) fn with_hasher(hasher: H, arena: &'a Bump) -> Self {
        Self {
            indices: HashTable::new_in(arena),
            data: ArenaVec::new_in(arena),
            hasher,
        }
    }

    pub(crate) fn new(arena: &'a Bump) -> Self
    where
        H: Default,
    {
        Self::with_hasher(H::default(), arena)
    }

    /// The index the next value appended to `data` gets.
    fn next_index(data: &[T]) -> u32 {
        u32::try_from(data.len()).expect("DedupArena index exceeds u32::MAX")
    }

    /// Appends a value that interning never returns, giving it an identity
    /// distinct from every equal value. Indexed values stay unique.
    pub(crate) fn push_unindexed(&mut self, value: T) -> u32 {
        let index = Self::next_index(&self.data);
        self.data.push(value);
        index
    }

    /// Only materialize a value in the arena after checking for an existing
    /// equal value. This matters when the value's storage cannot be reclaimed.
    pub(crate) fn intern_by<Q>(&mut self, lookup: &Q, make: impl FnOnce() -> T) -> u32
    where
        H: BuildHasher,
        T: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = self.hasher.hash_one(lookup);
        match self.indices.entry(
            hash,
            |&stored_index| self.data[stored_index as usize].borrow() == lookup,
            |&stored_index| {
                self.hasher
                    .hash_one(self.data[stored_index as usize].borrow())
            },
        ) {
            | Entry::Occupied(entry) => *entry.get(),
            | Entry::Vacant(entry) => {
                let index = Self::next_index(&self.data);
                self.data.push(make());
                _ = entry.insert(index);
                index
            },
        }
    }
}

impl<T, H> std::ops::Index<u32> for DedupArena<'_, T, H> {
    type Output = T;

    fn index(&self, index: u32) -> &T {
        &self.data[index as usize]
    }
}

impl<T, H> Deref for DedupArena<'_, T, H> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.data
    }
}

impl<T, H> AsRef<[T]> for DedupArena<'_, T, H> {
    fn as_ref(&self) -> &[T] {
        &self.data
    }
}

impl<T, H> Borrow<[T]> for DedupArena<'_, T, H> {
    fn borrow(&self) -> &[T] {
        &self.data
    }
}
