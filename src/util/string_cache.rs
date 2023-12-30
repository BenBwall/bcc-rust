use std::{
    fmt,
    fmt::{
        Display,
        Formatter,
    },
    hash::BuildHasherDefault,
};

use rustc_hash::FxHasher;
use string_interner::{
    backend::StringBackend,
    symbol::SymbolUsize,
    StringInterner,
    Symbol,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringCache {
    inner: StringInterner<StringBackend<SymbolUsize>, BuildHasherDefault<FxHasher>>,
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
pub(crate) struct Id {
    id: usize,
}

impl From<usize> for Id {
    fn from(id: usize) -> Self {
        Self { id }
    }
}

impl Id {
    fn from_symbol(symbol: SymbolUsize) -> Self {
        Self {
            id: symbol.to_usize(),
        }
    }

    pub(crate) const fn from_usize(id: usize) -> Self {
        Self { id }
    }

    pub(crate) const fn to_usize(self) -> usize {
        self.id
    }
}

trait SymbolExt {
    fn to_id(self) -> Id;
}

impl SymbolExt for SymbolUsize {
    fn to_id(self) -> Id {
        Id::from_symbol(self)
    }
}

impl StringCache {
    /// Creates a new empty `StringCache`. Does not allocate.
    pub(crate) fn new() -> Self {
        Self {
            inner: StringInterner::new(),
        }
    }

    /// Interns the given string and returns an ID representing its position in
    /// the string cache.
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> Id {
        fn inner(interner: &mut StringCache, s: &str) -> Id {
            Id::from_symbol(interner.inner.get_or_intern(s))
        }
        inner(self, s.as_ref())
    }

    /// Returns the string for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<Id>) -> Option<&str> {
        fn inner(interner: &StringCache, id: Id) -> Option<&str> {
            interner.inner.resolve(SymbolUsize::try_from_usize(id.id)?)
        }
        inner(self, id.into())
    }

    /// Returns true if the given string is already interned.
    pub(crate) fn contains(&self, string: impl AsRef<str>) -> bool {
        fn inner(interner: &StringCache, string: &str) -> bool {
            interner.inner.get(string).is_some()
        }
        inner(self, string.as_ref())
    }

    /// Returns the number of strings interned in the cache.
    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }
}
