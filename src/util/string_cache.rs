//! Partially based on the `string_interner` crate.

use std::{
    fmt::{
        self,
        Display,
        Formatter,
    },
    hash::BuildHasher,
    num::NonZeroU32,
};

use hashbrown::{
    HashTable,
    hash_table::Entry,
};
use rustc_hash::FxBuildHasher;
#[derive(Debug, Clone)]
pub(crate) struct StringCache {
    ends:  Vec<u32>,
    data:  String,
    dedup: HashTable<StringCacheId>,
}

impl PartialEq<StringCache> for StringCache {
    fn eq(&self, other: &StringCache) -> bool {
        self.ends == other.ends && self.data == other.data
    }
}

impl Eq for StringCache {}

impl Default for StringCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for StringCache {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "StringCache:")?;
        for i in 1u32.. {
            if let Some(s) = self.get(i) {
                writeln!(f, "\t{i}: {s}")?;
            } else {
                break;
            }
        }
        Ok(())
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct StringCacheId {
    id: NonZeroU32,
}

impl Display for StringCacheId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl From<u32> for StringCacheId {
    fn from(id: u32) -> Self {
        Self::from_u32(id)
    }
}

impl From<StringCacheId> for u32 {
    fn from(id: StringCacheId) -> Self {
        StringCacheId::to_u32(id)
    }
}

impl StringCacheId {
    pub(crate) const fn from_u32(id: u32) -> Self {
        Self {
            id: match NonZeroU32::new(id) {
                | Some(id) => id,
                | None => panic!("StringCacheId: ID cannot be zero."),
            },
        }
    }

    pub(crate) const fn to_u32(self) -> u32 {
        self.id.get()
    }
}

impl StringCache {
    /// Creates a new empty `StringCache`. Does not allocate.
    pub(crate) fn new() -> Self {
        Self {
            ends:  vec![0],
            data:  String::new(),
            dedup: HashTable::new(),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "We're checking that we're inbounds before casting."
    )]
    fn intern_impl(data: &mut String, ends: &mut Vec<u32>, s: &str) -> StringCacheId {
        let len = s.len();
        let start = data.len();
        data.push_str(s);
        let end = start + len;
        assert!(
            end < u32::MAX as usize,
            "StringCache: string cache cannot store more than 4GB."
        );
        ends.push(end as u32);
        StringCacheId::from_u32(ends.len() as u32 - 1)
    }

    /// Interns the given string and returns an ID representing its position in
    /// the string cache.
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> StringCacheId {
        fn inner(interner: &mut StringCache, s: &str) -> StringCacheId {
            let hash = FxBuildHasher.hash_one(s);
            match interner.dedup.entry(
                hash,
                |id| s == StringCache::at_impl(&interner.data, &interner.ends, *id),
                |id| {
                    FxBuildHasher.hash_one(StringCache::at_impl(
                        &interner.data,
                        &interner.ends,
                        *id,
                    ))
                },
            ) {
                | Entry::Occupied(entry) => *entry.get(),
                | Entry::Vacant(entry) => {
                    let symbol =
                        StringCache::intern_impl(&mut interner.data, &mut interner.ends, s);
                    _ = entry.insert(symbol);
                    symbol
                },
            }
        }
        inner(self, s.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn get_id_from_string(&self, s: impl AsRef<str>) -> Option<StringCacheId> {
        let string = s.as_ref();
        let hash = FxBuildHasher.hash_one(string);
        self.dedup
            .find(hash, |symbol| {
                string == StringCache::at_impl(&self.data, &self.ends, *symbol)
            })
            .copied()
    }

    pub(crate) fn push(&mut self, c: impl Into<char>) {
        fn inner(interner: &mut StringCache, c: char) {
            interner.data.push(c);
        }
        inner(self, c.into());
    }

    pub(crate) fn push_str(&mut self, s: impl AsRef<str>) {
        fn inner(interner: &mut StringCache, s: &str) {
            interner.data.push_str(s);
        }
        inner(self, s.as_ref());
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "One of our invariants is that self.data.len() will never be greater than \
                  u32::MAX."
    )]
    pub(crate) fn end_str(&mut self) -> StringCacheId {
        self.ends.push(self.data.len() as u32);
        let id = StringCacheId::from_u32(self.ends.len() as u32 - 1);
        let s = Self::at_impl(&self.data, &self.ends, id);
        let hash = FxBuildHasher.hash_one(s);
        match self.dedup.entry(
            hash,
            |stored_id| s == Self::at_impl(&self.data, &self.ends, *stored_id),
            |stored_id| FxBuildHasher.hash_one(Self::at_impl(&self.data, &self.ends, *stored_id)),
        ) {
            | Entry::Occupied(entry) => {
                let symbol = *entry.get();
                _ = self.ends.pop();
                Self::undo_str_impl(&mut self.data, &self.ends);
                symbol
            },
            | Entry::Vacant(entry) => {
                _ = entry.insert(id);
                id
            },
        }
    }

    pub(crate) fn undo_str(&mut self) {
        self.data.truncate(*self.ends.last().unwrap() as usize);
    }

    fn undo_str_impl(data: &mut String, ends: &[u32]) {
        data.truncate(*ends.last().unwrap() as usize);
    }

    /// Returns the bytes for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<StringCacheId>) -> Option<&str> {
        Self::get_impl(&self.data, &self.ends, id.into())
    }

    fn get_impl<'a>(data: &'a str, ends: &[u32], id: StringCacheId) -> Option<&'a str> {
        let start = *ends.get(id.to_u32() as usize - 1)?;
        let end = *ends.get(id.to_u32() as usize)?;
        let slice = &data[start as usize..end as usize];
        Some(slice)
    }

    fn at_impl<'a>(data: &'a str, ends: &[u32], id: StringCacheId) -> &'a str {
        Self::get_impl(data, ends, id)
            .unwrap_or_else(|| panic!("Compiler bug: StringCacheId is out of bounds: {id:#?}"))
    }

    pub(crate) fn at(&self, id: impl Into<StringCacheId>) -> &str {
        Self::at_impl(&self.data, &self.ends, id.into())
    }
}
