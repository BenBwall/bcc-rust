use std::{
    borrow::Borrow,
    cell::Cell,
    cmp::Ordering as CmpOrdering,
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
    ops::Deref,
    panic::{
        RefUnwindSafe,
        UnwindSafe,
    },
    process::abort,
    ptr::NonNull,
};

/// An Arc but without the weak reference count and with the contents stored in
/// a separate allocation.
pub(crate) struct Shared<T>
where
    T: ?Sized,
{
    ref_count: NonNull<Cell<usize>>,
    contents:  NonNull<T>,
}

impl<T> Drop for Shared<T>
where
    T: ?Sized,
{
    fn drop(&mut self) {
        self.ref_cnt().set(self.ref_cnt().get() - 1);
        if self.ref_cnt().get() == 0 {
            #[cold]
            #[inline(never)]
            fn drop_slow<T: ?Sized>(this: &mut Shared<T>) {
                // SAFETY: This is okay because self.ref_count and self.contents point at valid
                // boxes and we only drop them when the ref count is 0.
                #[expect(
                    clippy::multiple_unsafe_ops_per_block,
                    reason = "The safety comment explains why both operations are okay."
                )]
                unsafe {
                    drop(Box::from_raw(this.ref_count.as_ptr()));
                    drop(Box::from_raw(this.contents.as_ptr()));
                }
            }
            drop_slow(self);
        }
    }
}

impl<T> Clone for Shared<T>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        if self.ref_cnt().get() == usize::MAX {
            // Integer overflow.
            // This can realistically only happen if someone leaks usize::MAX Shareds. If
            // this happens, we just abort.
            abort();
        }

        self.ref_cnt().set(self.ref_cnt().get() + 1);

        Self {
            ref_count: self.ref_count,
            contents:  self.contents,
        }
    }
}

impl<T> Debug for Shared<T>
where
    T: Debug + ?Sized,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Shared")
            .field("ref_count", &self.ref_cnt())
            .field("contents", &self.as_ref())
            .finish()
    }
}

impl<T> PartialEq for Shared<T>
where
    T: PartialEq + ?Sized,
{
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<T> Eq for Shared<T> where T: Eq + ?Sized {}

impl<T> Hash for Shared<T>
where
    T: Hash + ?Sized,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl<T> PartialOrd for Shared<T>
where
    T: PartialOrd + ?Sized,
{
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl<T> Ord for Shared<T>
where
    T: Ord + ?Sized,
{
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl<T> Shared<T> {
    pub(crate) fn new(t: T) -> Self {
        Self::from_boxed(Box::new(t))
    }
}

impl<T> Shared<T>
where
    T: ?Sized,
{
    pub(crate) fn from_boxed(t: Box<T>) -> Self {
        Self {
            ref_count: NonNull::from(Box::leak(Box::new(Cell::new(1)))),
            contents:  NonNull::from(Box::leak(t)),
        }
    }

    pub(crate) fn strong_reference_count(&self) -> usize {
        self.ref_cnt().get()
    }

    /// The caller of this function must maintain the invariant that the ref
    /// count accurately reflects how many references there are to the contents.
    /// This function is not marked unsafe because it's private to this module.
    fn ref_cnt(&self) -> &Cell<usize> {
        // SAFETY: self.ref_count always points to a valid instance of Cell<usize>.
        // This function should arguably be unsafe, but it's not marked as such to avoid
        // unsafe contamination. It's private to this module to our invariants aren't
        // broken in external code.
        unsafe { self.ref_count.as_ref() }
    }
}

impl SharedString {
    pub(crate) fn as_str(&self) -> &str {
        self
    }

    pub(crate) fn from_string(s: String) -> Self {
        Self::from_boxed(s.into_boxed_str())
    }
}

impl<T> SharedVec<T> {
    pub(crate) fn as_slice(&self) -> &[T] {
        self
    }

    pub(crate) fn from_vec(v: Vec<T>) -> Self {
        Self::from_boxed(v.into_boxed_slice())
    }
}

impl<T> Deref for Shared<T>
where
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: self.contents always points to a valid instance of T.
        unsafe { self.contents.as_ref() }
    }
}

impl<T> AsRef<T> for Shared<T>
where
    T: ?Sized,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> Borrow<T> for Shared<T>
where
    T: ?Sized,
{
    fn borrow(&self) -> &T {
        self
    }
}

impl From<String> for SharedString {
    fn from(s: String) -> Self {
        Self::from_string(s)
    }
}

impl<T> From<Box<T>> for Shared<T>
where
    T: ?Sized,
{
    fn from(t: Box<T>) -> Self {
        Self::from_boxed(t)
    }
}

impl<T> From<T> for Shared<T> {
    fn from(t: T) -> Self {
        Self::new(t)
    }
}

impl<T> From<Vec<T>> for SharedVec<T> {
    fn from(v: Vec<T>) -> Self {
        Self::from_vec(v)
    }
}

impl<T> Default for Shared<T>
where
    Box<T>: Default,
    T: ?Sized,
{
    fn default() -> Self {
        Self::from_boxed(Box::default())
    }
}

impl<T> RefUnwindSafe for Shared<T> where T: RefUnwindSafe + ?Sized {}

/// Shared is always Unpin because it is a pointer type.
impl<T> Unpin for Shared<T> where T: ?Sized {}
impl<T> UnwindSafe for Shared<T> where T: UnwindSafe + ?Sized {}

pub(crate) type SharedString = Shared<str>;
pub(crate) type SharedVec<T> = Shared<[T]>;
