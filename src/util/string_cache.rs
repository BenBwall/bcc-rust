use std::{
    fmt,
    fmt::{
        Display,
        Formatter,
    },
    hash::BuildHasherDefault,
};

/// Data structure based on StringBackend from the `string-interner` crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interner {
    storage: Vec<u8>,
    dedup:   HashSet<Id>,
    ends:    Vec<usize>,
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Interner {
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
    id: u64,
}

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl From<usize> for Id {
    fn from(id: usize) -> Self {
        Self { id }
    }
}

impl From<Id> for usize {
    fn from(id: Id) -> Self {
        id.id
    }
}

impl Id {
    fn from_symbol(symbol: SymbolUsize) -> Self {
        Self {
            id: symbol.to_usize(),
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn from_usize(id: usize) -> Self {
        Self { id }
    }

    #[allow(dead_code)]
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

impl Interner {
    /// Creates a new empty `StringCache`. Does not allocate.
    pub(crate) fn new() -> Self {
        Self {
            inner: StringInterner::new(),
        }
    }

    /// Interns the given string and returns an ID representing its position in
    /// the string cache.
    pub(crate) fn intern(&mut self, s: impl AsRef<str>) -> Id {
        fn inner(interner: &mut Interner, s: &str) -> Id {
            Id::from_symbol(interner.inner.get_or_intern(s))
        }
        inner(self, s.as_ref())
    }

    /// Returns the string for the given ID if it exists in the cache.
    pub(crate) fn get(&self, id: impl Into<Id>) -> Option<&str> {
        fn inner(interner: &Interner, id: Id) -> Option<&str> {
            interner.inner.resolve(SymbolUsize::try_from_usize(id.id)?)
        }
        inner(self, id.into())
    }

    #[allow(dead_code)]
    /// Returns true if the given string is already interned.
    pub(crate) fn contains(&self, string: impl AsRef<str>) -> bool {
        fn inner(interner: &Interner, string: &str) -> bool {
            interner.inner.get(string).is_some()
        }
        inner(self, string.as_ref())
    }

    /// Clears the string cache.
    pub(crate) fn clear(&mut self) {
        self.inner.clear();
    }
}
