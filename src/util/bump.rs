//! Chunked arena allocation for phase-scoped compiler data.
//!
//! Chunks never move. Individual deallocation normally leaves memory in the
//! arena; dropping or resetting the arena releases whole chunks. Values put
//! here must not need destruction, since the arena does not run destructors.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "Later arena migration stages use this module.")
)]

use std::{
    alloc::{
        Layout,
        alloc,
        dealloc,
        handle_alloc_error,
    },
    cell::RefCell,
    fmt,
    ops::Deref,
    ptr::{
        self,
        NonNull,
    },
};

use allocator_api2::{
    alloc::{
        AllocError,
        Allocator,
    },
    vec::Vec as AllocVec,
};
use rustc_hash::FxBuildHasher;

const FIRST_CHUNK_BYTES: usize = 4096;
const CHUNK_ALIGN: usize = 64;

struct Chunk {
    ptr:    NonNull<u8>,
    layout: Layout,
    used:   usize,
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `alloc` with this exact layout and is freed
        // once.
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

#[derive(Clone, Copy)]
struct Last {
    ptr:   NonNull<u8>,
    chunk: usize,
    start: usize,
    size:  usize,
}

struct Inner {
    chunks:           Vec<Chunk>,
    current:          Option<usize>,
    last:             Option<Last>,
    next_chunk_bytes: usize,
}

/// An arena whose allocations remain at fixed addresses until it is reset or
/// dropped.
pub(crate) struct Bump {
    inner: RefCell<Inner>,
}

impl Default for Bump {
    fn default() -> Self {
        Self::new()
    }
}

impl Bump {
    pub(crate) fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                chunks:           Vec::new(),
                current:          None,
                last:             None,
                next_chunk_bytes: FIRST_CHUNK_BYTES,
            }),
        }
    }

    /// Store a value that needs no destructor.
    #[expect(
        clippy::mut_from_ref,
        reason = "The arena owns stable storage through interior mutability."
    )]
    pub(crate) fn alloc<T>(&self, value: T) -> &mut T {
        const {
            assert!(
                !core::mem::needs_drop::<T>(),
                "arena values must not need drop"
            );
        }
        let layout = Layout::new::<T>();
        let ptr = self
            .allocate(layout)
            .unwrap_or_else(|_| handle_alloc_error(layout));
        let typed = ptr.cast::<T>();
        // SAFETY: the allocator returned aligned, writable memory for `T`.
        unsafe {
            typed.as_ptr().write(value);
        }
        // SAFETY: the chunk cannot move, and reset/drop require exclusive
        // access to self.
        unsafe { &mut *typed.as_ptr() }
    }

    #[expect(
        clippy::mut_from_ref,
        reason = "The arena owns stable storage through interior mutability."
    )]
    pub(crate) fn alloc_slice_copy<T: Copy>(&self, values: &[T]) -> &mut [T] {
        let layout = Layout::array::<T>(values.len()).expect("slice layout overflow");
        let ptr = self
            .allocate(layout)
            .unwrap_or_else(|_| handle_alloc_error(layout));
        let typed = ptr.cast::<T>();
        // SAFETY: source and new arena allocation do not overlap, and both
        // cover `values.len()` items.
        unsafe {
            ptr::copy_nonoverlapping(values.as_ptr(), typed.as_ptr(), values.len());
        }
        // SAFETY: all elements were initialized above and remain live with the
        // arena.
        unsafe { std::slice::from_raw_parts_mut(typed.as_ptr(), values.len()) }
    }

    #[expect(
        clippy::mut_from_ref,
        reason = "The arena owns stable storage through interior mutability."
    )]
    pub(crate) fn alloc_str(&self, value: &str) -> &mut str {
        let bytes = self.alloc_slice_copy(value.as_bytes());
        // SAFETY: copied bytes came from a valid UTF-8 string.
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }

    pub(crate) fn alloc_slice_fill_iter<T>(&self, values: impl IntoIterator<Item = T>) -> &mut [T] {
        const {
            assert!(
                !core::mem::needs_drop::<T>(),
                "arena values must not need drop"
            );
        }
        let mut vec = ArenaVec::new_in(self);
        vec.extend(values);
        vec.leak()
    }

    /// Release all chunks except the largest, which is reused from its start.
    pub(crate) fn reset(&mut self) {
        let inner = self.inner.get_mut();
        let keep = inner
            .chunks
            .iter()
            .enumerate()
            .max_by_key(|(_, chunk)| chunk.layout.size())
            .map(|(index, _)| index);
        let kept = keep.map(|index| inner.chunks.swap_remove(index));
        inner.chunks.clear();
        inner.current = None;
        inner.last = None;
        if let Some(mut chunk) = kept {
            chunk.used = 0;
            inner.current = Some(0);
            inner.chunks.push(chunk);
        }
    }
}

fn aligned_start(base: usize, used: usize, align: usize) -> Option<usize> {
    base.checked_add(used)?
        .checked_add(align - 1)
        .map(|address| (address & !(align - 1)) - base)
}

impl Inner {
    fn allocate(&mut self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            let ptr = NonNull::new(layout.align() as *mut u8).ok_or(AllocError)?;
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }
        let slot = self.current.and_then(|index| {
            let chunk = &self.chunks[index];
            let start = aligned_start(chunk.ptr.as_ptr() as usize, chunk.used, layout.align())?;
            (start.checked_add(layout.size())? <= chunk.layout.size()).then_some((index, start))
        });
        let (index, start) = if let Some(slot) = slot {
            slot
        } else {
            let dedicated = layout.size() > self.next_chunk_bytes || layout.align() > CHUNK_ALIGN;
            let bytes = if dedicated {
                layout.size()
            } else {
                self.next_chunk_bytes
            };
            let chunk_layout = Layout::from_size_align(bytes, layout.align().max(CHUNK_ALIGN))
                .map_err(|_| AllocError)?;
            // SAFETY: `chunk_layout` is valid; a null result is reported as
            // `AllocError`.
            let raw = unsafe { alloc(chunk_layout) };
            let ptr = NonNull::new(raw).ok_or(AllocError)?;
            self.chunks.push(Chunk {
                ptr,
                layout: chunk_layout,
                used: 0,
            });
            let index = self.chunks.len() - 1;
            if !dedicated {
                self.current = Some(index);
                self.next_chunk_bytes = bytes.saturating_mul(2).max(bytes);
            }
            (index, 0)
        };
        let chunk = &mut self.chunks[index];
        chunk.used = start + layout.size();
        // SAFETY: `start` was aligned and checked to lie within the allocated
        // chunk.
        // SAFETY: the checked offset is within the allocated chunk.
        let address = unsafe { chunk.ptr.as_ptr().add(start) };
        // SAFETY: adding an in-bounds offset to a non-null pointer remains
        // non-null.
        let ptr = unsafe { NonNull::new_unchecked(address) };
        self.last = Some(Last {
            ptr,
            chunk: index,
            start,
            size: layout.size(),
        });
        Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
    }
}

// SAFETY: each returned block belongs to a stable chunk, and exclusive
// reset/drop cannot occur while an allocation borrowed from `&Bump` is in use.
unsafe impl Allocator for &Bump {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.inner.borrow_mut().allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if let Some(last) = inner
            .last
            .filter(|last| last.ptr == ptr && layout.size() <= last.size)
        {
            inner.chunks[last.chunk].used = last.start;
            inner.last = None;
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        if old_layout.size() != 0 {
            let mut inner = self.inner.borrow_mut();
            if let Some(last) = inner
                .last
                .filter(|last| last.ptr == ptr && old_layout.size() <= last.size)
            {
                let chunk = &mut inner.chunks[last.chunk];
                if (ptr.as_ptr() as usize).is_multiple_of(new_layout.align())
                    && last
                        .start
                        .checked_add(new_layout.size())
                        .is_some_and(|end| end <= chunk.layout.size())
                {
                    chunk.used = last.start + new_layout.size();
                    inner.last = Some(Last {
                        size: new_layout.size(),
                        ..last
                    });
                    return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
                }
            }
        }
        let new_ptr = self.allocate(new_layout)?;
        // SAFETY: caller guarantees the old block is live; the fresh allocation
        // is disjoint and large enough for the old contents.
        unsafe {
            ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_ptr().cast(), old_layout.size());
        }
        Ok(new_ptr)
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: the caller supplies a live old allocation and a larger
        // layout.
        let grown = unsafe { self.grow(ptr, old_layout, new_layout) }?;
        // SAFETY: the returned allocation contains at least `new_layout.size()`
        // bytes.
        let tail = unsafe { grown.as_ptr().cast::<u8>().add(old_layout.size()) };
        // SAFETY: the tail contains the newly allocated, writable bytes.
        unsafe {
            tail.write_bytes(0, new_layout.size() - old_layout.size());
        }
        Ok(grown)
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        if !(ptr.as_ptr() as usize).is_multiple_of(new_layout.align()) {
            return Err(AllocError);
        }
        Ok(NonNull::slice_from_raw_parts(ptr, old_layout.size()))
    }
}

pub(crate) type ArenaVec<'a, T> = AllocVec<T, &'a Bump>;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Later arena migration stages use these collections."
    )
)]
pub(crate) type ArenaMap<'a, K, V> = hashbrown::HashMap<K, V, FxBuildHasher, &'a Bump>;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Later arena migration stages use these collections."
    )
)]
pub(crate) type ArenaSet<'a, T> = hashbrown::HashSet<T, FxBuildHasher, &'a Bump>;

/// A UTF-8 string allocated in an arena.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Later arena migration stages use this collection."
    )
)]
pub(crate) struct ArenaString<'a> {
    bytes: ArenaVec<'a, u8>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Later arena migration stages use these methods.")
)]
impl<'a> ArenaString<'a> {
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            bytes: ArenaVec::new_in(arena),
        }
    }

    pub(crate) fn push_str(&mut self, value: &str) {
        self.bytes.extend_from_slice(value.as_bytes());
    }

    pub(crate) fn push(&mut self, value: char) {
        let mut buf = [0; 4];
        self.push_str(value.encode_utf8(&mut buf));
    }

    pub(crate) fn as_str(&self) -> &str {
        // SAFETY: construction and mutation only append valid UTF-8 sequences.
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
}

impl Deref for ArenaString<'_> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Display for ArenaString<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl fmt::Debug for ArenaString<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl fmt::Write for ArenaString<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        self.push(c);
        Ok(())
    }
}

/// A FIFO queue with cheap front removal and arena-backed storage.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Later arena migration stages use this collection."
    )
)]
pub(crate) struct ArenaQueue<'a, T> {
    data: ArenaVec<'a, Option<T>>,
    read: usize,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Later arena migration stages use these methods.")
)]
impl<'a, T> ArenaQueue<'a, T> {
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            data: ArenaVec::new_in(arena),
            read: 0,
        }
    }

    pub(crate) fn push_back(&mut self, value: T) {
        self.data.push(Some(value));
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        if self.read == self.data.len() {
            return None;
        }
        let value = self.data[self.read].take();
        self.read += 1;
        value
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len() - self.read
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.data[self.read..]
            .iter()
            .map(|item| item.as_ref().expect("unread queue item"))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        alloc::Layout,
        fmt::Write,
    };

    use allocator_api2::alloc::Allocator;
    use rustc_hash::FxBuildHasher;

    use super::{
        ArenaMap,
        ArenaQueue,
        ArenaSet,
        ArenaString,
        ArenaVec,
        Bump,
        FIRST_CHUNK_BYTES,
    };

    #[test]
    fn alignment_and_zero_sized_values() {
        let arena = Bump::new();
        for align in [1, 2, 4, 8, 16, 32, 64, 128, 4096] {
            let layout = Layout::from_size_align(7, align).unwrap();
            let allocation = (&arena).allocate(layout).unwrap();
            assert_eq!((allocation.as_ptr().cast::<u8>() as usize) % align, 0);
        }
        let unit = arena.alloc(());
        assert_eq!(*unit, ());
        let empty = arena.alloc_slice_copy::<u32>(&[]);
        assert_eq!(empty.len(), 0);
        let zero = (&arena)
            .allocate(Layout::from_size_align(0, 1024).unwrap())
            .unwrap();
        assert_eq!((zero.as_ptr().cast::<u8>() as usize) % 1024, 0);
    }

    #[test]
    fn many_allocations_and_stable_references() {
        let arena = Bump::new();
        let first = arena.alloc(0xDEAD_BEEFu64);
        let address: *mut u64 = first;
        for index in 0..10_000 {
            assert_eq!(*arena.alloc(index), index);
        }
        assert!(arena.inner.borrow().chunks.len() > 1);
        assert_eq!(*first, 0xDEAD_BEEF);
        let after: *mut u64 = first;
        assert_eq!(after, address);
        assert_eq!(arena.alloc_str("héllo"), "héllo");
        assert_eq!(arena.alloc_slice_fill_iter(0..4), [0, 1, 2, 3]);
    }

    #[test]
    fn oversized_chunk_does_not_displace_current_small_chunk() {
        let arena = Bump::new();
        _ = arena.alloc(1_u8);
        let current = arena.inner.borrow().current;
        let large = arena.alloc_slice_copy(&vec![7_u8; FIRST_CHUNK_BYTES * 4]);
        assert_eq!(large[0], 7);
        assert_eq!(arena.inner.borrow().current, current);
        assert_eq!(arena.inner.borrow().chunks.len(), 2);
    }

    #[test]
    fn grow_in_place_and_by_copy() {
        let arena = Bump::new();
        let old = Layout::from_size_align(8, 8).unwrap();
        let new = Layout::from_size_align(32, 8).unwrap();
        let first = (&arena).allocate(old).unwrap().cast::<u8>();
        // SAFETY: the allocation has at least eight writable bytes.
        unsafe {
            first.as_ptr().write_bytes(0x5A, 8);
        }
        // SAFETY: `first` is live and `new` is larger.
        let grown = unsafe { (&arena).grow(first, old, new) }
            .unwrap()
            .cast::<u8>();
        assert_eq!(first, grown);
        // SAFETY: `grown` is live and has 32 initialized bytes at its start.
        assert_eq!(unsafe { *grown.as_ptr() }, 0x5A);
        _ = (&arena).allocate(old).unwrap();
        // SAFETY: `grown` is still live and `new` is larger than `old`.
        let copied = unsafe { (&arena).grow(grown, new, Layout::from_size_align(64, 8).unwrap()) }
            .unwrap()
            .cast::<u8>();
        assert_ne!(grown, copied);
        // SAFETY: the first byte was copied from the earlier allocation.
        assert_eq!(unsafe { *copied.as_ptr() }, 0x5A);
    }

    #[test]
    fn shrink_keeps_storage_and_latest_allocation_can_regrow() {
        let arena = Bump::new();
        let large = Layout::from_size_align(64, 8).unwrap();
        let small = Layout::from_size_align(16, 8).unwrap();
        let ptr = (&arena).allocate(large).unwrap().cast::<u8>();
        // SAFETY: `ptr` is live and the new layout is smaller.
        let shrunk = unsafe { (&arena).shrink(ptr, large, small) }.unwrap();
        assert_eq!(shrunk.cast::<u8>(), ptr);
        // SAFETY: the returned block fits `small`, and `large` is larger.
        let regrown = unsafe { (&arena).grow(ptr, small, large) }.unwrap();
        assert_eq!(regrown.cast::<u8>(), ptr);
        // SAFETY: the latest block is live and fits `small`.
        unsafe {
            (&arena).deallocate(ptr, small);
        }
        assert_eq!(arena.inner.borrow().chunks[0].used, 0);
    }

    #[test]
    fn deallocate_rolls_back_only_latest_and_reset_reuses_largest() {
        let mut arena = Bump::new();
        let layout = Layout::new::<u64>();
        let first = (&arena).allocate(layout).unwrap().cast::<u8>();
        let second = (&arena).allocate(layout).unwrap().cast::<u8>();
        // SAFETY: both pointers came from this allocator with this layout.
        unsafe {
            (&arena).deallocate(first, layout);
        }
        let used = arena.inner.borrow().chunks[0].used;
        assert_eq!(used, 16);
        // SAFETY: second is the most recent live allocation.
        unsafe {
            (&arena).deallocate(second, layout);
        }
        assert_eq!(arena.inner.borrow().chunks[0].used, 8);
        let reused = (&arena).allocate(layout).unwrap().cast::<u8>();
        assert_eq!(reused, second);
        _ = (&arena)
            .allocate(Layout::from_size_align(FIRST_CHUNK_BYTES * 4, 8).unwrap())
            .unwrap();
        let largest = arena
            .inner
            .borrow()
            .chunks
            .iter()
            .max_by_key(|chunk| chunk.layout.size())
            .unwrap()
            .ptr;
        arena.reset();
        assert_eq!(arena.inner.borrow().chunks.len(), 1);
        assert_eq!(arena.inner.borrow().chunks[0].ptr, largest);
        let after = (&arena).allocate(layout).unwrap().cast::<u8>();
        assert_eq!(after, largest);
    }

    #[test]
    fn collections_and_utf8() {
        let arena = Bump::new();
        let mut vec = ArenaVec::new_in(&arena);
        vec.extend(0..100);
        assert_eq!(vec.len(), 100);
        assert_eq!(vec[99], 99);
        let mut map = ArenaMap::with_hasher_in(FxBuildHasher, &arena);
        assert_eq!(map.insert("a", 3), None);
        assert_eq!(map["a"], 3);
        let mut set = ArenaSet::with_hasher_in(FxBuildHasher, &arena);
        assert!(set.insert(9));
        assert!(set.contains(&9));
        let mut string = ArenaString::new_in(&arena);
        string.push_str("h");
        string.push('é');
        write!(string, " {}", 42).unwrap();
        assert_eq!(string.as_str(), "hé 42");
        assert_eq!(&*string, "hé 42");
        assert_eq!(format!("{string}"), "hé 42");
        assert_eq!(format!("{string:?}"), "\"hé 42\"");
        let mut queue = ArenaQueue::new_in(&arena);
        assert!(queue.is_empty());
        queue.push_back(String::from("one"));
        queue.push_back(String::from("two"));
        assert_eq!(queue.pop_front().as_deref(), Some("one"));
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.iter().map(String::as_str).collect::<Vec<_>>(),
            ["two"]
        );
        assert_eq!(queue.pop_front().as_deref(), Some("two"));
        assert!(queue.is_empty());
    }
}
