//! Partially based on the `string_interner` crate.

use std::{
    fmt::{
        self,
        Display,
        Formatter,
    },
    hash::{
        BuildHasher,
        BuildHasherDefault,
    },
    num::NonZeroU32,
};

use hashbrown::{
    hash_map::RawEntryMut,
    HashMap,
};
use rustc_hash::FxHasher;
#[allow(clippy::assertions_on_constants)]
const _: () = assert!(
    usize::BITS >= 32,
    "StringCache: usize must be at least 32 bits."
);

#[derive(Debug, Clone)]
pub(crate) struct StringCache {
    ends:   Vec<u32>,
    data:   String,
    dedup:  HashMap<StringCacheId, (), ()>,
    hasher: BuildHasherDefault<FxHasher>,
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
    #[allow(dead_code)]
    pub(crate) const fn from_u32(id: u32) -> Self {
        Self {
            id: match NonZeroU32::new(id) {
                | Some(id) => id,
                | None => panic!("StringCacheId: ID cannot be zero."),
            },
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn to_u32(self) -> u32 {
        self.id.get()
    }
}

impl StringCache {
    /// Creates a new empty `StringCache`. Does not allocate.
    pub(crate) fn new() -> Self {
        Self {
            ends:   vec![0],
            data:   String::new(),
            dedup:  HashMap::default(),
            hasher: BuildHasherDefault::default(),
        }
    }

    #[allow(clippy::cast_possible_truncation)]
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
            let hash = interner.hasher.hash_one(s);
            let entry = interner.dedup.raw_entry_mut().from_hash(hash, |id| {
                // SAFETY: This is safe because we only operate on id that have been previously
                // interned.
                s == unsafe {
                    StringCache::get_impl(&interner.data, &interner.ends, *id).unwrap_unchecked()
                }
            });
            let (&mut symbol, &mut ()) = match entry {
                | RawEntryMut::Occupied(occupied) => occupied.into_key_value(),
                | RawEntryMut::Vacant(vacant) => {
                    let symbol =
                        StringCache::intern_impl(&mut interner.data, &mut interner.ends, s);
                    vacant.insert_with_hasher(hash, symbol, (), |id| {
                        // SAFETY: This is safe because we only operate on symbols that
                        //         we receive from our backend making them valid.
                        let string = unsafe {
                            StringCache::get_impl(&interner.data, &interner.ends, *id)
                                .unwrap_unchecked()
                        };
                        interner.hasher.hash_one(string)
                    })
                },
            };
            symbol
        }
        inner(self, s.as_ref())
    }

    #[allow(dead_code)]
    pub(crate) fn get_id_from_string(&self, s: impl AsRef<str>) -> Option<StringCacheId> {
        fn inner(interner: &StringCache, s: &str) -> Option<StringCacheId> {
            let hash = interner.hasher.hash_one(s);
            interner
                .dedup
                .raw_entry()
                .from_hash(hash, |symbol| {
                    // SAFETY: This is safe because we only operate on id that have been previously
                    // interned.
                    s == unsafe {
                        StringCache::get_impl(&interner.data, &interner.ends, *symbol)
                            .unwrap_unchecked()
                    }
                })
                .map(|(&id, &())| id)
        }
        inner(self, s.as_ref())
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

    #[allow(dead_code)]
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn pop(&mut self) {
        assert!(
            self.ends.last().copied() != Some(self.data.len() as u32),
            "StringCache: cannot pop across string boundaries."
        );
        _ = self.data.pop();
    }

    #[allow(dead_code)]
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn pop_str(&mut self, len: u32) {
        assert!(
            self.ends.last().copied() < Some(self.data.len() as u32 + len - 1),
            "StringCache: cannot pop across string boundaries."
        );
        self.data.truncate(self.data.len() - len as usize);
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn end_str(&mut self) -> StringCacheId {
        self.ends.push(self.data.len() as u32);
        let id = StringCacheId::from_u32(self.ends.len() as u32 - 1);
        let s = unsafe { Self::get_impl(&self.data, &self.ends, id).unwrap_unchecked() };
        let hash = self.hasher.hash_one(s);
        let entry = self.dedup.raw_entry_mut().from_hash(hash, |id| {
            // SAFETY: This is safe because we only operate on id that have been previously
            // interned.
            s == unsafe { Self::get_impl(&self.data, &self.ends, *id).unwrap_unchecked() }
        });
        let (&mut symbol, &mut ()) = match entry {
            | RawEntryMut::Occupied(occupied) => {
                _ = self.ends.pop();
                Self::undo_str_impl(&mut self.data, &self.ends);
                occupied.into_key_value()
            },
            | RawEntryMut::Vacant(vacant) => {
                vacant.insert_with_hasher(hash, id, (), |id| {
                    // SAFETY: This is safe because we only operate on symbols that
                    //         we receive from our backend making them valid.
                    let string =
                        unsafe { Self::get_impl(&self.data, &self.ends, *id).unwrap_unchecked() };
                    self.hasher.hash_one(string)
                })
            },
        };
        symbol
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

    pub(crate) fn at(&self, id: impl Into<StringCacheId>) -> &str {
        fn inner(interner: &StringCache, id: StringCacheId) -> &str {
            interner
                .get(id)
                .unwrap_or_else(|| panic!("Compiler bug: StringCacheId is out of bounds: {id:#?}"))
        }
        inner(self, id.into())
    }

    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.ends.truncate(1);
        self.data.clear();
    }
}
