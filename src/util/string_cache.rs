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
    /// Creates a new empty `StringCache`.
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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Lookup by string is retained for parser test and diagnostic helpers."
        )
    )]
    pub(crate) fn get_id_from_string(&self, s: impl AsRef<str>) -> Option<StringCacheId> {
        fn inner(interner: &StringCache, s: &str) -> Option<StringCacheId> {
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
        dead_code,
        clippy::cast_possible_truncation,
        reason = "Incremental cache edits retain the cache-size invariant for future callers."
    )]
    pub(crate) fn pop(&mut self) {
        assert!(
            self.ends.last().copied() != Some(self.data.len() as u32),
            "StringCache: cannot pop across string boundaries."
        );
        _ = self.data.pop();
    }

    #[expect(
        dead_code,
        clippy::cast_possible_truncation,
        reason = "Incremental cache edits retain the cache-size invariant for future callers."
    )]
    pub(crate) fn pop_str(&mut self, len: u32) {
        assert!(
            self.ends.last().copied() < Some(self.data.len() as u32 + len - 1),
            "StringCache: cannot pop across string boundaries."
        );
        self.data.truncate(self.data.len() - len as usize);
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
    fn unfinished_new_intern_keeps_ascii_lookup_consistent_after_growth() {
        let mut cache = StringCache::new();
        cache.push('a');
        _ = cache.intern("");
        let before_growth = cache.intern("a");
        assert_eq!(cache.at(before_growth), "a");
        assert_eq!(cache.get_id_from_string("a"), Some(before_growth));
        for index in 0..400 {
            let spelling = format!("growth-{index}");
            let id = cache.intern(&spelling);
            assert_eq!(cache.at(id), spelling);
        }
        let after_growth = cache.intern("a");
        assert_eq!(cache.at(after_growth), "a");
        assert_eq!(cache.get_id_from_string("a"), Some(after_growth));
        cache.push('a');
        assert_eq!(cache.end_str(), after_growth);
    }

    #[test]
    fn every_ascii_spelling_shares_ids_between_insertion_paths() {
        let mut cache = StringCache::new();
        for byte in 0_u8..=127 {
            let character = char::from(byte);
            let spelling = character.to_string();
            let first = if byte % 2 == 0 {
                cache.intern(&spelling)
            } else {
                cache.push(character);
                cache.end_str()
            };
            assert_eq!(first.to_u32(), u32::from(byte) + 1);
            assert_eq!(cache.at(first), spelling);
            assert_eq!(cache.intern(&spelling), first);
            cache.push(character);
            assert_eq!(cache.end_str(), first);
            assert_eq!(cache.get_id_from_string(&spelling), Some(first));
        }
    }

    #[test]
    fn repeated_interning_preserves_the_unfinished_string() {
        let mut cache = StringCache::new();
        let existing = cache.intern("a");
        cache.push_str("bc");
        assert_eq!(cache.intern("a"), existing);
        cache.push('d');
        let built = cache.end_str();
        assert_eq!(cache.at(built), "bcd");
        assert_eq!(cache.intern("bcd"), built);

        cache.push_str("🦀");
        assert_eq!(cache.intern("a"), existing);
        let crab = cache.end_str();
        assert_eq!(cache.at(crab), "🦀");
    }

    #[test]
    fn empty_and_multibyte_spellings_keep_distinct_shared_ids() {
        let mut cache = StringCache::new();
        let mut ids = Vec::new();
        for spelling in ["", "é", "λ", "🦀", "int", "0\0"] {
            let id = cache.intern(spelling);
            assert!(!ids.contains(&id));
            ids.push(id);
            cache.push_str(spelling);
            assert_eq!(cache.end_str(), id);
            assert_eq!(cache.at(id), spelling);
            assert_eq!(cache.get_id_from_string(spelling), Some(id));
        }
    }

    #[test]
    fn a_new_intern_with_pending_bytes_does_not_poison_later_ascii_ids() {
        let mut cache = StringCache::new();
        cache.push_str("bc");
        _ = cache.intern("a");
        let canonical = cache.intern("a");
        assert_eq!(cache.at(canonical), "a");
        assert_eq!(cache.get_id_from_string("a"), Some(canonical));
        cache.undo_str();
        cache.push('a');
        assert_eq!(cache.end_str(), canonical);
    }

    #[test]
    fn discarded_scratch_does_not_change_committed_ids() {
        let mut cache = StringCache::new();
        let original = cache.intern("a");
        cache.push('a');
        cache.undo_str();
        cache.push('b');
        let built = cache.end_str();
        assert_eq!(cache.at(built), "b");
        assert_ne!(built, original);
        cache.push('a');
        assert_eq!(cache.end_str(), original);
    }

    #[test]
    fn cloned_and_cleared_caches_have_independent_valid_ids() {
        let mut cache = StringCache::new();
        let original = cache.intern("a");
        let mut cloned = cache.clone();
        assert_eq!(cloned.intern("a"), original);
        cloned.push('a');
        assert_eq!(cloned.end_str(), original);
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
        cloned.push('a');
        assert_eq!(cloned.end_str(), after_clear);
        assert_eq!(cache.at(original), "a");
    }
}
