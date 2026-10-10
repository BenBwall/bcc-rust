//! Length-prefixed lists in an arena.
//!
//! A `&'a [T]` is a pointer and a length, 16 bytes in every node field that
//! holds one. An [`ArenaList`] is a single pointer to an arena block that
//! stores the length followed by the elements, so the field takes 8 bytes and
//! `Option<ArenaList>` still takes 8. Like a shared slice, a list never
//! changes once built, and it dereferences to `[T]`.

use std::{
    alloc::{
        Layout,
        handle_alloc_error,
    },
    fmt,
    marker::PhantomData,
    ops::Deref,
    ptr::{
        self,
        NonNull,
    },
    slice,
};

use allocator_api2::alloc::Allocator;

use super::bump::Bump;

/// The length word every empty list points to, so empty lists allocate
/// nothing.
static EMPTY_LENGTH: usize = 0;

/// An immutable list of `T` in an arena, referred to by one pointer.
///
/// The pointer addresses a `usize` length, and the elements follow it at
/// offset `size_of::<usize>()`. Elements must not be more aligned than
/// `usize`, so that offset is always aligned for them; every syntax-tree
/// element type satisfies this.
pub(crate) struct ArenaList<'a, T> {
    /// The list's length word. It comes from the arena's raw allocation (or
    /// the empty static), never from a reference to the word alone, so its
    /// provenance covers the elements that follow.
    header:   NonNull<usize>,
    /// Borrows the arena like the slice this list stands for, and makes the
    /// list covariant in `'a` and `T`.
    elements: PhantomData<&'a [T]>,
}

impl<'a, T> ArenaList<'a, T> {
    /// A list with no elements. It points at a shared static length.
    pub(crate) fn empty() -> Self {
        const {
            assert!(
                align_of::<T>() <= align_of::<usize>(),
                "list elements follow a usize length"
            );
        }
        Self {
            header:   NonNull::from(&EMPTY_LENGTH),
            elements: PhantomData,
        }
    }

    /// Copies `values` into one new block in `arena`: the length, then the
    /// elements. An empty `values` allocates nothing.
    pub(crate) fn copy_from_slice(arena: &'a Bump, values: &[T]) -> Self
    where
        T: Copy,
    {
        const {
            assert!(
                align_of::<T>() <= align_of::<usize>(),
                "list elements follow a usize length"
            );
        }
        if values.is_empty() {
            return Self::empty();
        }
        let (layout, offset) = Layout::new::<usize>()
            .extend(Layout::array::<T>(values.len()).expect("list layout overflow"))
            .expect("list layout overflow");
        debug_assert_eq!(offset, size_of::<usize>(), "elements follow the length");
        let header = arena
            .allocate(layout)
            .unwrap_or_else(|_| handle_alloc_error(layout))
            .cast::<usize>();
        // SAFETY: the arena returned a fresh block of `layout.size()` bytes,
        // aligned for `usize`, that nothing else refers to. The length takes
        // its first word.
        unsafe {
            header.write(values.len());
        }
        // SAFETY: the elements start one `usize` past the length, at
        // `offset`, inside the block or, for zero-sized `T`, one past its end.
        let elements = unsafe { header.add(1) }.cast::<T>();
        // SAFETY: `elements` is aligned for `T`, which is no more aligned than
        // `usize`, and the layout leaves room for `values.len()` of them.
        // `values` lies outside the new block, so the copy does not overlap.
        unsafe {
            ptr::copy_nonoverlapping(values.as_ptr(), elements.as_ptr(), values.len());
        }
        Self {
            header,
            elements: PhantomData,
        }
    }

    /// The elements, borrowed for as long as the arena is.
    pub(crate) fn as_slice(self) -> &'a [T] {
        // SAFETY: `header` points at `EMPTY_LENGTH` or at the length word of a
        // block that `copy_from_slice` wrote and nothing changes after. The
        // block stays in place for `'a`: a `Bump` frees blocks only through
        // `reset` or drop, which need exclusive access, and this list borrows
        // the arena for `'a`.
        let len = unsafe { self.header.read() };
        // SAFETY: one `usize` past the length is where a block's elements
        // start, or, for the empty list, one past the end of `EMPTY_LENGTH`.
        // Both are in bounds of their allocation.
        let elements = unsafe { self.header.add(1) }.cast::<T>();
        // SAFETY: a block holds `len` initialized elements at `elements`,
        // aligned for `T`, which live and stay unchanged for `'a` as above.
        // For the empty list, `len` is zero and `elements` is non-null and
        // aligned for `T`, which is all a slice of no elements needs.
        unsafe { slice::from_raw_parts(elements.as_ptr(), len) }
    }
}

impl<T> Clone for ArenaList<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ArenaList<'_, T> {}

// SAFETY: a list is a shared view of immutable elements, exactly like
// `&'a [T]`, which is `Send` and `Sync` when `T` is `Sync`. The length word is
// never written after construction.
unsafe impl<T: Sync> Send for ArenaList<'_, T> {}
// SAFETY: as for `Send` above.
unsafe impl<T: Sync> Sync for ArenaList<'_, T> {}

impl<T> Default for ArenaList<'_, T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> Deref for ArenaList<'_, T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<'a, T> IntoIterator for ArenaList<'a, T> {
    type IntoIter = slice::Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a, T> IntoIterator for &ArenaList<'a, T> {
    type IntoIter = slice::Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<T: PartialEq> PartialEq for ArenaList<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

/// Prints the elements as a slice would, so debug output does not change
/// with the representation.
impl<T: fmt::Debug> fmt::Debug for ArenaList<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_slice(), f)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::ArenaList;
    use crate::util::bump::{
        ArenaString,
        Bump,
    };

    /// An element shaped like a syntax identifier: 12 bytes, 4-aligned.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Named {
        name:  u32,
        start: u32,
        len:   u32,
    }

    #[test]
    fn a_list_is_one_pointer_and_keeps_a_niche() {
        assert_eq!(size_of::<ArenaList<'_, u8>>(), size_of::<usize>());
        assert_eq!(
            size_of::<Option<ArenaList<'_, Named>>>(),
            size_of::<usize>()
        );
    }

    #[test]
    fn empty_lists_share_one_length_and_allocate_nothing() {
        let arena = Bump::new();
        let empty = ArenaList::<Named>::copy_from_slice(&arena, &[]);
        assert!(empty.is_empty());
        assert_eq!(*empty, []);
        assert_eq!(empty, ArenaList::default());
        assert_eq!(empty.as_ptr(), ArenaList::<Named>::empty().as_ptr());
        assert_eq!(arena.used(), 0);
        let mut printed = ArenaString::new_in(&arena);
        write!(&mut printed, "{empty:?}").unwrap();
        assert_eq!(&*printed, "[]");
        assert_eq!(empty.iter().count(), 0);
    }

    #[test]
    fn lists_hold_their_elements_after_later_allocations() {
        let arena = Bump::new();
        let bytes = ArenaList::copy_from_slice(&arena, &[1_u8, 2, 3]);
        let named = ArenaList::copy_from_slice(
            &arena,
            &[
                Named {
                    name:  7,
                    start: 1,
                    len:   2,
                },
                Named {
                    name:  9,
                    start: 4,
                    len:   0,
                },
            ],
        );
        let words = ArenaList::copy_from_slice(&arena, &[u64::MAX, 0, 42]);
        for index in 0..1000_u32 {
            let _ = arena.alloc(index);
        }
        assert_eq!(*bytes, [1, 2, 3]);
        assert_eq!(named.len(), 2);
        assert_eq!(named[1].name, 9);
        assert_eq!(*words, [u64::MAX, 0, 42]);
        // One length word plus the elements, each block aligned for `usize`.
        assert!(arena.used() >= 8 + 3 + 8 + 24 + 8 + 24 + 4000);
    }

    #[test]
    fn lists_of_references_read_through_to_the_arena() {
        let arena = Bump::new();
        let values: [&u32; 5] =
            std::array::from_fn(|value| &*arena.alloc(u32::try_from(value).unwrap() * 10));
        let list = ArenaList::copy_from_slice(&arena, &values);
        let mut iter = list.into_iter();
        let read = std::array::from_fn::<_, 5, _>(|_| **iter.next().unwrap());
        assert!(iter.next().is_none());
        assert_eq!(read, [0, 10, 20, 30, 40]);
        let mut by_reference = 0;
        for value in &list {
            by_reference += **value;
        }
        assert_eq!(by_reference, 100);
    }

    #[test]
    fn lists_compare_and_print_like_slices() {
        let arena = Bump::new();
        let first = ArenaList::copy_from_slice(&arena, &[1_u16, 2]);
        let second = ArenaList::copy_from_slice(&arena, &[1_u16, 2]);
        let third = ArenaList::copy_from_slice(&arena, &[1_u16]);
        assert_eq!(first, second);
        assert_ne!(first.as_ptr(), second.as_ptr());
        assert_ne!(first, third);
        let mut list_debug = ArenaString::new_in(&arena);
        let mut slice_debug = ArenaString::new_in(&arena);
        write!(&mut list_debug, "{first:?}").unwrap();
        write!(&mut slice_debug, "{:?}", [1_u16, 2]).unwrap();
        assert_eq!(&*list_debug, &*slice_debug);
        list_debug.clear();
        slice_debug.clear();
        write!(&mut list_debug, "{third:#?}").unwrap();
        write!(&mut slice_debug, "{:#?}", [1_u16]).unwrap();
        assert_eq!(&*list_debug, &*slice_debug);
    }

    #[test]
    fn a_slice_outlives_the_list_it_came_from() {
        let arena = Bump::new();
        let slice = {
            let list = ArenaList::copy_from_slice(&arena, &[5_i32, 6]);
            list.as_slice()
        };
        assert_eq!(slice, [5, 6]);
    }

    #[test]
    fn lists_are_shared_across_threads() {
        let arena = Bump::new();
        let list = ArenaList::copy_from_slice(&arena, &[3_u32, 4, 5]);
        let sum =
            std::thread::scope(|scope| scope.spawn(|| list.iter().sum::<u32>()).join().unwrap());
        assert_eq!(sum, 12);
    }

    /// Lists shorten their lifetimes like the slices they replace.
    #[test]
    fn lists_are_covariant() {
        fn shorten<'short>(
            list: ArenaList<'static, &'static str>,
        ) -> ArenaList<'short, &'short str> {
            list
        }
        assert!(shorten(ArenaList::empty()).is_empty());
    }
}
