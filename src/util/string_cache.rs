use std::{
    fmt,
    fmt::{
        Display,
        Formatter,
    },
};
#[allow(clippy::assertions_on_constants)]
const _: () = assert!(
    usize::BITS >= 32,
    "StringCache: usize must be at least 32 bits."
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringCache {
    ends: Vec<u32>,
    data: String,
}

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
            data: String::new(),
        }
    }

    /// Interns the given string and returns an ID representing its position in
    /// the slice cache.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> StringCacheId {
        fn inner(interner: &mut StringCache, s: &str) -> StringCacheId {
            let len = s.len();
            let start = interner.data.len();
            interner.data.push_str(s);
            let end = start + len;
            assert!(
                end < u32::MAX as usize,
                "StringCache: string cache cannot store more than 4GB."
            );
            interner.ends.push(end as u32);
            StringCacheId::from_u32(interner.ends.len() as u32 - 1)
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
        StringCacheId::from_u32(self.ends.len() as u32 - 1)
    }

    pub(crate) fn undo_str(&mut self) {
        self.data.truncate(*self.ends.last().unwrap() as usize);
    }

    /// Returns the bytes for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<StringCacheId>) -> Option<&str> {
        fn inner(interner: &StringCache, id: StringCacheId) -> Option<&str> {
            let start = *interner.ends.get(id.to_u32() as usize - 1)?;
            let end = *interner.ends.get(id.to_u32() as usize)?;
            let slice = &interner.data[start as usize..end as usize];
            Some(slice)
        }
        inner(self, id.into())
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
