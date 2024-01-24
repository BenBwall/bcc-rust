use std::{fmt::{Debug, Display, Formatter, Result as FmtResult}, ops::Deref, sync::Arc};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Input {
    arc: Arc<Box<str>>,
    ref_: &'static str,
}

impl Debug for Input {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{:?}", self.ref_)
    }
}

impl Display for Input {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.ref_)
    }
}

impl Input {
    pub(crate) fn new(s: String) -> Self {
        Self::from_boxed_str(s.into_boxed_str())
    }
    pub(crate) fn from_boxed_str(s: Box<str>) -> Self {
        let mut ret = Self {
            arc: Arc::new(s),
            ref_: "",
        };
        ret.ref_ = unsafe { &*std::ptr::addr_of!(ret.arc)};
        ret
    }
}

impl Deref for Input {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.ref_
    }
}

impl AsRef<str> for Input {
    fn as_ref(&self) -> &str {
        self.ref_
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