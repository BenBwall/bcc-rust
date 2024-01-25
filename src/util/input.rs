use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    ops::Deref,
    sync::Arc, ptr::NonNull, borrow::Borrow,
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Input {
    arc:  Arc<Box<str>>,
    ptr: NonNull<str>,
}

impl Debug for Input {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{:?}", self.as_str())
    }
}

impl Display for Input {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.as_str())
    }
}

impl Input {
    pub(crate) fn new(s: String) -> Self {
        Self::from_boxed_str(s.into_boxed_str())
    }

    pub(crate) fn from_boxed_str(s: Box<str>) -> Self {
        let mut ret = Self {
            arc:  Arc::new(s),
            ptr: NonNull::from(""),
        };
        ret.ptr = NonNull::from(&**ret.arc);
        ret
    }

    #[allow(dead_code)]
    pub(crate) fn as_str(&self) -> &str {
        unsafe { self.ptr.as_ref() }
    }
}

impl Deref for Input {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for Input {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Input {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for Input {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<Box<str>> for Input {
    fn from(s: Box<str>) -> Self {
        Self::new(s.into())
    }
}
