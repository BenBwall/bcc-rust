use std::{
    fmt,
    fmt::{
        Display,
        Formatter,
    },
};

const _: () = assert!(usize::BITS >= 32, "StringCache: usize must be at least 32 bits.");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringCache {
    ends: Vec<u32>,
    data: Vec<u8>,
}

impl Default for StringCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for StringCache {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "StringCache:")?;
        for i in 0usize.. {
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
    id: u32,
}

impl Display for StringCacheId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl From<u32> for StringCacheId {
    fn from(id: u32) -> Self {
        Self { id }
    }
}

impl From<StringCacheId> for u32 {
    fn from(id: StringCacheId) -> Self {
        id.id
    }
}

impl StringCacheId {
    #[allow(dead_code)]
    pub(crate) const fn from_u32(id: u32) -> Self {
        Self { id }
    }

    #[allow(dead_code)]
    pub(crate) const fn to_u32(self) -> u32 {
        self.id
    }
}

impl StringCache {
    /// Creates a new empty `StringCache`. Does not allocate.
    pub(crate) fn new() -> Self {
        Self {
            ends: vec![0],
            data: vec![],
        }
    }

    /// Interns the given string and returns an ID representing its position in
    /// the slice cache.
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> StringCacheId {
        fn inner(interner: &mut StringCache, s: &str) -> StringCacheId {
            let len = s.len();
            let start = self.data.len();
            self.data.extend_from_slice(s.as_bytes());
            let end = start + len;
            assert!(
                end < u32::MAX as usize,
                "StringCache: string cache cannot store more than 4GB."
            );
            self.ends.push(end as u32);
            StringCacheId::from_u32(self.ends.len() as u32 - 1)
        }
        inner(self, s.as_ref())
    }

    /// Returns the bytes for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<StringCacheId>) -> Option<str> {
        fn inner(interner: &StringCache, id: StringCacheId) -> Option<str> {
            let start = *interner.ends.get(id.to_u32() as usize - 1)?;
            let end = *interner.ends.get(id.to_u32() as usize)?;
            let slice = &interner.data[start as usize..end as usize];
            // SAFETY: The slice is guaranteed to be valid UTF-8 because it was interned and we only allow interning strings.
            Some(unsafe { std::str::from_utf8_unchecked(slice) })
        }
        inner(self, id.into())
    }

    #[allow(dead_code)]
    /// Returns true if the given string is already interned.
    pub(crate) fn contains(&self, string: impl AsRef<str>) -> bool {
        fn inner(interner: &StringCache, string: &str) -> bool {
            interner.inner.get(string).is_some()
        }
        inner(self, string.as_ref())
    }

    pub(crate) fn clear(&mut self) {
        self.ends.truncate(1);
        self.data.clear();
    }
}
