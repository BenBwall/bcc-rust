//! A vector that owns one virtual-memory region and commits as it is written.
//!
//! Its capacity is the region's whole reservation from the start, so it
//! never grows, reallocates, or moves its elements. Pages are committed a
//! step at a time just ahead of the last element written (on Linux, whole
//! 2 MiB huge pages), so the commit charge follows the vector's length
//! rather than a doubled capacity.
//! This is why it is its own type rather than an `allocator_api2` vector over
//! an arena: such a vector asks its allocator for whole capacities, and an
//! allocator has to commit every byte it hands out.

use std::{
    alloc::{
        Layout,
        handle_alloc_error,
    },
    fmt,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{
        Deref,
        DerefMut,
        Range,
    },
    ptr::{
        self,
        NonNull,
    },
};

use allocator_api2::alloc::AllocError;

use super::vm::{
    GrowingRegion,
    REGION_BYTES,
};

/// A growable vector in its own fixed-address region. See the module docs.
pub(crate) struct RegionVec<T> {
    /// Reserved by the first element written.
    region:    Option<GrowingRegion>,
    /// The first element, derived after the latest commit so it reaches
    /// every committed element. Dangling until the first commit.
    ptr:       NonNull<T>,
    len:       usize,
    /// How many elements fit in committed memory.
    committed: usize,
    elements:  PhantomData<T>,
}

impl<T> Default for RegionVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RegionVec<T> {
    /// An empty vector. It reserves its region when first written.
    pub(crate) const fn new() -> Self {
        const {
            assert!(size_of::<T>() != 0, "region vectors hold sized elements");
            // Regions start page-aligned, and pages are at least 4 KiB.
            assert!(align_of::<T>() <= 4096, "elements fit a page's alignment");
        }
        Self {
            region:    None,
            ptr:       NonNull::dangling(),
            len:       0,
            committed: 0,
            elements:  PhantomData,
        }
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        // SAFETY: the first `len` elements are initialized in committed
        // memory that `ptr` reaches; with none, `ptr` is dangling but
        // aligned, which an empty slice allows.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: as in `as_slice`, and `&mut self` makes the access
        // exclusive.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    pub(crate) fn push(&mut self, value: T) {
        if self.try_push(value).is_err() {
            handle_alloc_error(Layout::new::<T>());
        }
    }

    /// Appends `value`, committing another step of pages first if the
    /// committed ones are full.
    pub(crate) fn try_push(&mut self, value: T) -> Result<(), AllocError> {
        if self.len == self.committed {
            self.commit_for(1)?;
        }
        // SAFETY: room for one more element is committed.
        unsafe {
            self.push_committed(value);
        }
        Ok(())
    }

    /// Commits room for `additional` more elements beyond the length.
    pub(crate) fn try_reserve(&mut self, additional: usize) -> Result<(), AllocError> {
        if self.committed - self.len >= additional {
            return Ok(());
        }
        self.commit_for(additional)
    }

    fn reserve(&mut self, additional: usize) {
        if self.try_reserve(additional).is_err() {
            handle_alloc_error(Layout::new::<T>());
        }
    }

    #[cold]
    fn commit_for(&mut self, additional: usize) -> Result<(), AllocError> {
        let end = self
            .len
            .checked_add(additional)
            .and_then(|elements| elements.checked_mul(size_of::<T>()))
            .ok_or(AllocError)?;
        let region = match &mut self.region {
            | Some(region) => region,
            | region @ None =>
                region.insert(GrowingRegion::reserve(REGION_BYTES).map_err(|_| AllocError)?),
        };
        region.ensure_committed(end).map_err(|_| AllocError)?;
        self.committed = region.committed() / size_of::<T>();
        // The region starts page-aligned, so its base is aligned for `T`.
        self.ptr = region.as_ptr().cast();
        Ok(())
    }

    /// # Safety
    ///
    /// Room for at least one more element must be committed.
    unsafe fn push_committed(&mut self, value: T) {
        debug_assert!(self.len < self.committed, "pushing into committed room");
        // SAFETY: `len < committed`, so the slot lies inside the committed
        // memory that `ptr` was derived to reach.
        let slot = unsafe { self.ptr.as_ptr().add(self.len) };
        // SAFETY: the slot is committed, aligned, unused, and owned by this
        // vector alone.
        unsafe {
            slot.write(value);
        }
        self.len += 1;
    }

    /// Drops the elements from `len` on, keeping the committed pages for
    /// reuse.
    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        // SAFETY: `len < self.len`, inside the initialized elements.
        let first = unsafe { self.ptr.as_ptr().add(len) };
        let dropped = ptr::slice_from_raw_parts_mut(first, self.len - len);
        // Forget the elements first, so a panicking destructor cannot lead to
        // a second drop.
        self.len = len;
        // SAFETY: the elements were initialized, and the length no longer
        // covers them, so nothing else drops or reads them.
        unsafe {
            ptr::drop_in_place(dropped);
        }
    }

    /// Drops every element, keeping the committed pages for reuse.
    pub(crate) fn clear(&mut self) {
        self.truncate(0);
    }
}

impl<T: Clone> RegionVec<T> {
    pub(crate) fn extend_from_slice(&mut self, values: &[T]) {
        self.reserve(values.len());
        for value in values {
            // SAFETY: room for every value was committed above.
            unsafe {
                self.push_committed(value.clone());
            }
        }
    }

    /// Appends clones of the elements in `range`.
    pub(crate) fn extend_from_within(&mut self, range: Range<usize>) {
        assert!(
            range.start <= range.end && range.end <= self.len,
            "range {range:?} lies inside {} elements",
            self.len
        );
        self.reserve(range.len());
        for index in range {
            let value = self[index].clone();
            // SAFETY: room for every element of `range` was committed above.
            unsafe {
                self.push_committed(value);
            }
        }
    }
}

impl<T> Drop for RegionVec<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<T> Deref for RegionVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> DerefMut for RegionVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T> Extend<T> for RegionVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.push(value);
        }
    }
}

impl<T: PartialEq> PartialEq for RegionVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: fmt::Debug> fmt::Debug for RegionVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a, T> IntoIterator for &'a RegionVec<T> {
    type IntoIter = std::slice::Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T> IntoIterator for RegionVec<T> {
    type IntoIter = IntoIter<T>;
    type Item = T;

    fn into_iter(self) -> IntoIter<T> {
        let mut vec = ManuallyDrop::new(self);
        IntoIter {
            ptr:      vec.ptr,
            next:     0,
            end:      vec.len,
            _region:  vec.region.take(),
            elements: PhantomData,
        }
    }
}

/// The elements of a [`RegionVec`], by value. The region is released when
/// the iterator is dropped.
pub(crate) struct IntoIter<T> {
    ptr:      NonNull<T>,
    /// Elements before `next` were moved out; those from `next` to `end` are
    /// still owned here.
    next:     usize,
    end:      usize,
    /// Keeps the elements' memory reserved and committed.
    _region:  Option<GrowingRegion>,
    elements: PhantomData<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.next == self.end {
            return None;
        }
        // SAFETY: `next < end`, so the element is inside the vector's
        // initialized, committed prefix, which `ptr` reaches.
        let slot = unsafe { self.ptr.as_ptr().add(self.next) };
        self.next += 1;
        // SAFETY: the element is initialized and, now that `next` has moved
        // past it, read exactly once.
        Some(unsafe { slot.read() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.next;
        (remaining, Some(remaining))
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        // SAFETY: `next <= end`, inside the vector's elements.
        let first = unsafe { self.ptr.as_ptr().add(self.next) };
        let remaining = ptr::slice_from_raw_parts_mut(first, self.end - self.next);
        self.next = self.end;
        // SAFETY: the remaining elements are initialized and owned only
        // here; the region field is dropped after this, releasing their
        // memory once they are gone.
        unsafe {
            ptr::drop_in_place(remaining);
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use std::rc::Rc;

    use super::RegionVec;
    use crate::util::vm::{
        MAX_COMMIT_STEP,
        accounting,
        faults,
    };

    #[test]
    fn elements_stay_in_place_as_the_vector_grows() {
        let mut values = RegionVec::new();
        values.push(42_u64);
        let first = values.as_ptr();
        for value in 0..100_000 {
            values.push(value);
        }
        assert_eq!(values.as_ptr(), first);
        assert_eq!(values[0], 42);
        assert_eq!(values[100_000], 99_999);
        assert_eq!(values.len(), 100_001);
    }

    #[cfg_attr(miri, ignore = "writes megabytes an element at a time")]
    #[test]
    fn commit_follows_the_length_not_a_capacity() {
        let before = accounting::live();
        let mut values = RegionVec::new();
        assert_eq!(
            accounting::live(),
            before,
            "an empty vector reserves nothing"
        );
        let length = 3 * MAX_COMMIT_STEP + 12_345;
        values.extend_from_slice(&[7_u8; 5]);
        values.extend(std::iter::repeat_n(9_u8, length - 5));
        assert_eq!(values.len(), length);
        let committed = accounting::live().committed - before.committed;
        assert!(committed >= length, "{committed}");
        assert!(committed <= length + MAX_COMMIT_STEP, "{committed}");
        assert_eq!(accounting::live().regions, before.regions + 1);
        drop(values);
        assert_eq!(accounting::live(), before);
    }

    #[test]
    fn failed_commits_are_allocation_errors_and_the_vector_recovers() {
        let mut values = RegionVec::new();
        {
            let _failing = faults::fail_reserves(1);
            _ = values.try_push(1_u32).unwrap_err();
        }
        assert!(values.is_empty());
        values.try_push(1).unwrap();
        // Fill the committed pages, so the next push has to commit.
        while values.len() < values.committed {
            values.push(2);
        }
        {
            let _failing = faults::fail_commits(usize::MAX);
            _ = values.try_push(3).unwrap_err();
            _ = values.try_reserve(MAX_COMMIT_STEP).unwrap_err();
        }
        let length = values.len();
        assert_eq!(values[0], 1);
        assert_eq!(values[length - 1], 2);
        values.try_push(3).unwrap();
        assert_eq!(values[length], 3);
    }

    #[test]
    fn a_failed_commit_step_falls_back_to_the_pages_needed() {
        let mut values = RegionVec::<u8>::new();
        values.push(0);
        let filled = 64 * 1024;
        values.extend(std::iter::repeat_n(1, filled - 1));
        // The next step would commit another 64 KiB at once.
        let _limited = faults::limit_commits(4096);
        values.try_push(2).unwrap();
        assert_eq!(values.len(), filled + 1);
    }

    #[test]
    fn requests_beyond_the_reservation_fail() {
        let mut values = RegionVec::<u8>::new();
        _ = values
            .try_reserve(crate::util::vm::REGION_BYTES + 1)
            .unwrap_err();
        values.push(1);
        assert_eq!(values.as_slice(), [1]);
    }

    #[test]
    fn elements_are_dropped_once() {
        let counted = Rc::new(());
        let mut values = RegionVec::new();
        values.extend((0..10).map(|_| Rc::clone(&counted)));
        values.extend_from_within(2..5);
        assert_eq!(Rc::strong_count(&counted), 14);
        values.truncate(12);
        assert_eq!(Rc::strong_count(&counted), 13);
        values.truncate(20);
        assert_eq!(values.len(), 12);
        values.clear();
        assert_eq!(Rc::strong_count(&counted), 1);
        values.extend((0..10).map(|_| Rc::clone(&counted)));
        let mut iter = values.into_iter();
        drop(iter.next());
        drop(iter.next());
        assert_eq!(iter.len(), 8);
        assert_eq!(Rc::strong_count(&counted), 9);
        drop(iter);
        assert_eq!(Rc::strong_count(&counted), 1);
        let mut values = RegionVec::new();
        values.push(Rc::clone(&counted));
        drop(values);
        assert_eq!(Rc::strong_count(&counted), 1);
    }

    #[test]
    fn copies_from_within_and_iterates_by_value() {
        let mut values = RegionVec::new();
        values.extend_from_slice(&[1, 2, 3]);
        values.extend_from_within(1..3);
        values[0] = 10;
        assert_eq!(values.as_slice(), [10, 2, 3, 2, 3]);
        assert_eq!(format!("{values:?}"), "[10, 2, 3, 2, 3]");
        assert_eq!((&values).into_iter().sum::<i32>(), 20);
        assert_eq!(values.into_iter().collect::<Vec<_>>(), [10, 2, 3, 2, 3]);
        assert_eq!(RegionVec::<u8>::default().into_iter().next(), None);
    }
}
