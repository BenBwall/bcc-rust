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

use crate::util::{
    bump::Bump,
    region_vec::RegionVec,
};

pub(crate) struct StringCache<'tu> {
    arena: &'tu Bump,
    ends:  RegionVec<u32>,
    data:  RegionVec<u8>,
    dedup: HashTable<StringCacheId, &'tu Bump>,
}

impl<'tu> StringCache<'tu> {
    /// Interns the given string and returns an ID representing its position in
    /// the string cache.
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> StringCacheId {
        fn inner(interner: &mut StringCache<'_>, s: &str) -> StringCacheId {
            let hash = FxBuildHasher.hash_one(s);
            match interner.dedup.entry(
                hash,
                |id| {
                    s == StringCache::at_impl(
                        StringCache::bytes_str(&interner.data),
                        &interner.ends,
                        *id,
                    )
                },
                |id| {
                    FxBuildHasher.hash_one(StringCache::at_impl(
                        StringCache::bytes_str(&interner.data),
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

    /// Interns the concatenation of `parts` without building it elsewhere
    /// first: the parts are written at the end of the cache's buffer, and
    /// taken back when an equal string is already interned.
    pub(crate) fn intern_concat(&mut self, parts: &[&str]) -> StringCacheId {
        let start = self.data.len();
        let additional = parts
            .iter()
            .try_fold(0usize, |length, part| length.checked_add(part.len()))
            .expect("StringCache concatenated string length overflows usize");
        let (end, id) = Self::checked_insert(start, additional, self.ends.len());
        for part in parts {
            self.data.extend_from_slice(part.as_bytes());
        }
        let data = Self::bytes_str(&self.data);
        let candidate = &data[start..];
        let ends = &self.ends;
        match self.dedup.entry(
            FxBuildHasher.hash_one(candidate),
            |id| candidate == Self::at_impl(data, ends, *id),
            |id| FxBuildHasher.hash_one(Self::at_impl(data, ends, *id)),
        ) {
            | Entry::Occupied(entry) => {
                let id = *entry.get();
                self.data.truncate(start);
                id
            },
            | Entry::Vacant(entry) => {
                self.ends.push(end);
                _ = entry.insert(id);
                id
            },
        }
    }

    fn intern_impl(data: &mut RegionVec<u8>, ends: &mut RegionVec<u32>, s: &str) -> StringCacheId {
        let (end, id) = Self::checked_insert(data.len(), s.len(), ends.len());
        data.extend_from_slice(s.as_bytes());
        ends.push(end);
        id
    }

    /// Checks both packed widths before appending bytes or an end offset.
    fn checked_insert(data_len: usize, additional: usize, ends_len: usize) -> (u32, StringCacheId) {
        let end = data_len
            .checked_add(additional)
            .expect("StringCache byte length overflows usize");
        let end = u32::try_from(end).expect("StringCache byte length exceeds u32::MAX");
        let id = u32::try_from(ends_len).expect("StringCache ID exceeds u32::MAX");
        (end, StringCacheId::from_u32(id))
    }

    fn data_str(&self) -> &str {
        Self::bytes_str(&self.data)
    }

    fn bytes_str(data: &[u8]) -> &str {
        // SAFETY: `data` is private; `intern_impl` and `intern_concat` append
        // complete UTF-8 strings and truncate only at string boundaries, so
        // its bytes are valid UTF-8 at every call site.
        unsafe { std::str::from_utf8_unchecked(data) }
    }

    fn at_impl<'a>(data: &'a str, ends: &[u32], id: StringCacheId) -> &'a str {
        Self::get_impl(data, ends, id)
            .unwrap_or_else(|| panic!("Compiler bug: StringCacheId is out of bounds: {id:#?}"))
    }

    pub(crate) fn at(&self, id: impl Into<StringCacheId>) -> &str {
        Self::at_impl(self.data_str(), &self.ends, id.into())
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.ends.truncate(1);
        self.data.clear();
        self.dedup.clear();
    }

    /// Creates a new empty `StringCache`.
    pub(crate) fn new(arena: &'tu Bump) -> Self {
        let mut ends = RegionVec::new();
        ends.push(0);
        Self {
            arena,
            ends,
            data: RegionVec::new(),
            dedup: HashTable::new_in(arena),
        }
    }

    /// The id of `s` if it has been interned, for tests that look names up.
    #[cfg(test)]
    pub(crate) fn get_id_from_string(&self, s: impl AsRef<str>) -> Option<StringCacheId> {
        fn inner(interner: &StringCache<'_>, s: &str) -> Option<StringCacheId> {
            let hash = FxBuildHasher.hash_one(s);
            interner
                .dedup
                .find(hash, |symbol| {
                    s == StringCache::at_impl(
                        StringCache::bytes_str(&interner.data),
                        &interner.ends,
                        *symbol,
                    )
                })
                .copied()
        }
        inner(self, s.as_ref())
    }

    /// Returns the bytes for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<StringCacheId>) -> Option<&str> {
        Self::get_impl(self.data_str(), &self.ends, id.into())
    }

    fn get_impl<'a>(data: &'a str, ends: &[u32], id: StringCacheId) -> Option<&'a str> {
        let start = *ends.get(id.to_u32() as usize - 1)?;
        let end = *ends.get(id.to_u32() as usize)?;
        let slice = &data[start as usize..end as usize];
        Some(slice)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct StringCacheId {
    id: NonZeroU32,
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

impl fmt::Debug for StringCache<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("StringCache")
            .field("ends", &self.ends)
            .field("data", &self.data_str())
            .field("dedup", &self.dedup)
            .finish()
    }
}

impl PartialEq<StringCache<'_>> for StringCache<'_> {
    fn eq(&self, other: &StringCache<'_>) -> bool {
        self.ends == other.ends && self.data_str() == other.data_str()
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
        for index in 1..self.ends.len() {
            let i = u32::try_from(index).expect("string cache index overflow");
            writeln!(f, "\t{i}: {}", self.at(i))?;
        }
        Ok(())
    }
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

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use super::StringCache;

    #[test]
    fn final_string_cache_offset_and_id_are_representable() {
        let (end, id) = StringCache::checked_insert(u32::MAX as usize - 1, 1, u32::MAX as usize);
        assert_eq!(end, u32::MAX);
        assert_eq!(id.to_u32(), u32::MAX);
    }

    #[test]
    #[should_panic(expected = "StringCache byte length overflows usize")]
    fn string_cache_rejects_usize_overflow_before_writing() {
        _ = StringCache::checked_insert(usize::MAX, 1, 1);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    #[should_panic(expected = "StringCache byte length exceeds u32::MAX")]
    fn string_cache_rejects_offsets_beyond_u32() {
        _ = StringCache::checked_insert(u32::MAX as usize, 1, 1);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    #[should_panic(expected = "StringCache ID exceeds u32::MAX")]
    fn string_cache_rejects_ids_beyond_u32() {
        _ = StringCache::checked_insert(0, 0, u32::MAX as usize + 1);
    }

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
    fn concatenated_parts_intern_like_the_whole_string() {
        let arena = crate::util::bump::Bump::new();
        let mut cache = StringCache::new(&arena);
        let whole = cache.intern("12\0");
        assert_eq!(cache.intern_concat(&["12", "\0"]), whole);
        let new = cache.intern_concat(&["3", "4", "\0"]);
        assert_eq!(cache.at(new), "34\0");
        assert_eq!(cache.intern("34\0"), new);
        // Taking back a duplicate leaves the next string where it belongs.
        let after = cache.intern("after");
        assert_eq!(cache.at(after), "after");
        assert_eq!(cache.at(whole), "12\0");
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
