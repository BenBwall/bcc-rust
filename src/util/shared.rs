use std::{
    borrow::Borrow,
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
    ptr::NonNull,
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

/// An Arc but without the weak reference count and with the contents stored in
/// a separate allocation.
pub(crate) struct Shared<T>
where
    T: ?Sized,
{
    ref_count: NonNull<AtomicUsize>,
    contents:  NonNull<T>,
}

impl<T> Drop for Shared<T>
where
    T: ?Sized,
{
    fn drop(&mut self) {
        if self
            .ref_cnt()
            .fetch_sub(1, std::sync::atomic::Ordering::Release)
            == 1
        {
            unsafe {
                // Fence maybe unnecessary?
                std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
                drop(Box::from_raw(self.ref_count.as_ptr()));
                drop(Box::from_raw(self.contents.as_ptr()));
            }
        }
    }
}

impl<T> Clone for Shared<T>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        _ = self.ref_cnt().fetch_add(1, Ordering::Acquire);

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
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl<T> Ord for Shared<T>
where
    T: Ord + ?Sized,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
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
            ref_count: NonNull::from(Box::leak(Box::new(AtomicUsize::new(1)))),
            contents:  NonNull::from(Box::leak(t)),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn reference_count(&self) -> usize {
        self.ref_cnt().load(Ordering::Relaxed)
    }

    fn ref_cnt(&self) -> &AtomicUsize {
        unsafe { self.ref_count.as_ref() }
    }
}

impl Shared<str> {
    #[allow(dead_code)]
    pub(crate) fn as_str(&self) -> &str {
        self
    }

    pub(crate) fn from_string(s: String) -> Self {
        Self::from_boxed(s.into_boxed_str())
    }
}

impl<T> Shared<[T]> {
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

impl From<String> for Shared<str> {
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

impl<T> From<Vec<T>> for Shared<[T]> {
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

// Safety: Shared is Send and Sync if T is Send and Sync. This is safe for the
// same reason that Arc<T> is Send and Sync if T is Send and Sync.
unsafe impl<T> Send for Shared<T> where T: Send + ?Sized {}
unsafe impl<T> Sync for Shared<T> where T: Sync + ?Sized {}

impl<T> RefUnwindSafe for Shared<T> where T: RefUnwindSafe + ?Sized {}

/// Shared is always Unpin because it is a pointer type.
impl<T> Unpin for Shared<T> where T: ?Sized {}
impl<T> UnwindSafe for Shared<T> where T: UnwindSafe + ?Sized {}

pub(crate) type SharedString = Shared<str>;
pub(crate) type SharedVec<T> = Shared<[T]>;
