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

use super::bump::{
    ArenaString,
    ArenaVec,
    Bump,
};

pub(crate) struct StringCache<'tu> {
    arena: &'tu Bump,
    ends:  ArenaVec<'tu, u32>,
    data:  ArenaString<'tu>,
    dedup: HashTable<StringCacheId, &'tu Bump>,
}

impl fmt::Debug for StringCache<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("StringCache")
            .field("ends", &self.ends)
            .field("data", &self.data)
            .field("dedup", &self.dedup)
            .finish()
    }
}

impl PartialEq<StringCache<'_>> for StringCache<'_> {
    fn eq(&self, other: &StringCache<'_>) -> bool {
        self.ends == other.ends && self.data.as_str() == other.data.as_str()
    }
}

impl Eq for StringCache<'_> {}

impl Clone for StringCache<'_> {
    fn clone(&self) -> Self {
        let mut cloned = Self::new(self.arena);
        for index in 1..self.ends.len() {
            _ = cloned.intern(
                self.get(u32::try_from(index).expect("string cache index overflow"))
                    .expect("string cache index"),
            );
        }
        cloned
    }
}

impl Display for StringCache<'_> {
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

impl<'tu> StringCache<'tu> {
    /// Creates a new empty `StringCache`.
    pub(crate) fn new(arena: &'tu Bump) -> Self {
        let mut ends = ArenaVec::new_in(arena);
        ends.push(0);
        Self {
            arena,
            ends,
            data: ArenaString::new_in(arena),
            dedup: HashTable::new_in(arena),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "We're checking that we're inbounds before casting."
    )]
    fn intern_impl(
        data: &mut ArenaString<'tu>,
        ends: &mut ArenaVec<'tu, u32>,
        s: &str,
    ) -> StringCacheId {
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
        fn inner(interner: &mut StringCache<'_>, s: &str) -> StringCacheId {
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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Lookup by string is retained for parser test and diagnostic helpers."
        )
    )]
    pub(crate) fn get_id_from_string(&self, s: impl AsRef<str>) -> Option<StringCacheId> {
        fn inner(interner: &StringCache<'_>, s: &str) -> Option<StringCacheId> {
            let hash = FxBuildHasher.hash_one(s);
            interner
                .dedup
                .find(hash, |symbol| {
                    s == StringCache::at_impl(&interner.data, &interner.ends, *symbol)
                })
                .copied()
        }
        inner(self, s.as_ref())
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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Resetting a cache is retained for future callers."
        )
    )]
    pub(crate) fn clear(&mut self) {
        self.ends.truncate(1);
        self.data.clear();
        self.dedup.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::StringCache;

    #[test]
    fn interning_keeps_lookup_consistent_after_growth() {
        let arena = crate::util::bump::Bump::new();
        let mut cache = StringCache::new(&arena);
        let before_growth = cache.intern("a");
        for index in 0..400 {
            let spelling = format!("growth-{index}");
            let id = cache.intern(&spelling);
            assert_eq!(cache.at(id), spelling);
        }
        assert_eq!(cache.intern("a"), before_growth);
        assert_eq!(cache.at(before_growth), "a");
        assert_eq!(cache.get_id_from_string("a"), Some(before_growth));
    }

    #[test]
    fn every_ascii_spelling_gets_a_stable_id() {
        let arena = crate::util::bump::Bump::new();
        let mut cache = StringCache::new(&arena);
        for byte in 0_u8..=127 {
            let spelling = char::from(byte).to_string();
            let first = cache.intern(&spelling);
            assert_eq!(first.to_u32(), u32::from(byte) + 1);
            assert_eq!(cache.at(first), spelling);
            assert_eq!(cache.intern(&spelling), first);
            assert_eq!(cache.get_id_from_string(&spelling), Some(first));
        }
    }

    #[test]
    fn empty_and_multibyte_spellings_keep_distinct_ids() {
        let arena = crate::util::bump::Bump::new();
        let mut cache = StringCache::new(&arena);
        let mut ids = Vec::new();
        for spelling in ["", "é", "λ", "🦀", "int", "0\0"] {
            let id = cache.intern(spelling);
            assert!(!ids.contains(&id));
            ids.push(id);
            assert_eq!(cache.intern(spelling), id);
            assert_eq!(cache.at(id), spelling);
            assert_eq!(cache.get_id_from_string(spelling), Some(id));
        }
    }

    #[test]
    fn cloned_and_cleared_caches_have_independent_valid_ids() {
        let arena = crate::util::bump::Bump::new();
        let mut cache = StringCache::new(&arena);
        let original = cache.intern("a");
        let mut cloned = cache.clone();
        assert_eq!(cloned.intern("a"), original);
        let new = cloned.intern("z");
        assert_eq!(cloned.at(new), "z");
        assert_eq!(cache.get_id_from_string("z"), None);

        cloned.clear();
        let reused = cloned.intern("z");
        assert_eq!(reused.to_u32(), 1);
        assert_eq!(cloned.at(reused), "z");
        let after_clear = cloned.intern("a");
        assert_ne!(after_clear, reused);
        assert_eq!(cloned.at(after_clear), "a");
        assert_eq!(cache.at(original), "a");
    }
}
