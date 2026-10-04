//! Virtual-memory arena allocation for phase-scoped compiler data.
//!
//! The reservation never moves. Individual deallocation normally leaves memory
//! in the arena; dropping releases the region and reset rewinds it. Values put
//! here must not need destruction, since the arena does not run destructors.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "Later arena migration stages use this module.")
)]

use std::{
    alloc::{
        Layout,
        handle_alloc_error,
    },
    cell::RefCell,
    fmt,
    io::Read,
    ops::Deref,
    path::Path,
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

use super::vm::GrowingRegion;

#[cfg(not(miri))]
const REGION_BYTES: usize = 100 * 1024 * 1024 * 1024;
#[cfg(miri)]
const REGION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Last {
    ptr:   NonNull<u8>,
    start: usize,
    size:  usize,
}

struct Inner {
    region: GrowingRegion,
    used:   usize,
    last:   Option<Last>,
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
                region: GrowingRegion::reserve(REGION_BYTES)
                    .expect("failed to reserve virtual address space for arena"),
                used:   0,
                last:   None,
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
        // SAFETY: the region cannot move, and reset/drop require exclusive
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

    /// Read a source file into stable arena storage, replacing malformed
    /// UTF-8 in the same way as `String::from_utf8_lossy`.
    pub(crate) fn read_to_str_lossy(&self, path: &Path) -> std::io::Result<&str> {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = ArenaVec::new_in(self);
        let mut buffer = [0_u8; 8192];
        loop {
            match file.read(&mut buffer) {
                | Ok(0) => break,
                | Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                | Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
                | Err(error) => return Err(error),
            }
        }
        if std::str::from_utf8(&bytes).is_ok() {
            let bytes = bytes.leak();
            // SAFETY: UTF-8 validation above covered these exact bytes.
            return Ok(unsafe { std::str::from_utf8_unchecked(bytes) });
        }

        let mut repaired = ArenaVec::new_in(self);
        let mut remaining = bytes.as_slice();
        while let Err(error) = std::str::from_utf8(remaining) {
            let valid = error.valid_up_to();
            repaired.extend_from_slice(&remaining[..valid]);
            repaired.extend_from_slice("�".as_bytes());
            remaining = &remaining[valid + error.error_len().unwrap_or(remaining.len() - valid)..];
        }
        repaired.extend_from_slice(remaining);
        let bytes = repaired.leak();
        // SAFETY: each valid run was checked above and replacements are UTF-8.
        Ok(unsafe { std::str::from_utf8_unchecked(bytes) })
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

    /// Rewind the arena while retaining its committed pages for reuse.
    pub(crate) fn reset(&mut self) {
        let inner = self.inner.get_mut();
        inner.used = 0;
        inner.last = None;
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
        let start = aligned_start(
            self.region.as_ptr().as_ptr() as usize,
            self.used,
            layout.align(),
        )
        .ok_or(AllocError)?;
        let end = start.checked_add(layout.size()).ok_or(AllocError)?;
        self.region.ensure_committed(end).map_err(|_| AllocError)?;
        self.used = end;
        // SAFETY: the checked offset lies within the reserved region and the
        // entire allocation was committed before constructing this pointer.
        let address = unsafe { self.region.as_ptr().as_ptr().add(start) };
        // SAFETY: adding an in-bounds offset to a non-null pointer remains
        // non-null.
        let ptr = unsafe { NonNull::new_unchecked(address) };
        self.last = Some(Last {
            ptr,
            start,
            size: layout.size(),
        });
        Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
    }
}

// SAFETY: each returned block belongs to a stable reservation. Moving Bump
// does not move that reservation; reset/drop require exclusive access.
unsafe impl Allocator for Bump {
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
            inner.used = last.start;
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
                .filter(|_| (ptr.as_ptr() as usize).is_multiple_of(new_layout.align()))
                .filter(|last| {
                    last.start
                        .checked_add(new_layout.size())
                        .is_some_and(|end| inner.region.ensure_committed(end).is_ok())
                })
            {
                inner.used = last.start + new_layout.size();
                inner.last = Some(Last {
                    size: new_layout.size(),
                    ..last
                });
                return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
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
/// A growable vector that owns its own fixed-address virtual-memory region.
pub(crate) type RegionVec<T> = AllocVec<T, Bump>;
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
        RegionVec,
    };

    #[test]
    fn alignment_and_zero_sized_values() {
        let arena = Bump::new();
        for align in [1, 2, 4, 8, 16, 32, 64, 128, 4096] {
            let layout = Layout::from_size_align(7, align).unwrap();
            let allocation = arena.allocate(layout).unwrap();
            assert_eq!((allocation.as_ptr().cast::<u8>() as usize) % align, 0);
        }
        let unit = arena.alloc(());
        assert_eq!(*unit, ());
        let empty = arena.alloc_slice_copy::<u32>(&[]);
        assert_eq!(empty.len(), 0);
        let zero = arena
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
        assert!(arena.inner.borrow().region.committed() >= 40_000);
        assert_eq!(*first, 0xDEAD_BEEF);
        let after: *mut u64 = first;
        assert_eq!(after, address);
        assert_eq!(arena.alloc_str("héllo"), "héllo");
        assert_eq!(arena.alloc_slice_fill_iter(0..4), [0, 1, 2, 3]);
    }

    #[cfg(not(miri))]
    #[test]
    fn file_reads_match_lossy_utf8_across_buffer_boundaries() {
        let path = std::env::temp_dir().join(format!("bcc-bump-read-{}.c", std::process::id()));
        let mut source = vec![b'a'; 8191];
        source.extend_from_slice(&[0xE2, 0x82, 0xAC, 0xFF, 0xF0, 0x9F, 0x92]);
        source.extend_from_slice(&vec![b'b'; 9000]);
        std::fs::write(&path, &source).unwrap();

        let arena = Bump::new();
        let text = arena.read_to_str_lossy(&path).unwrap();
        assert_eq!(text, String::from_utf8_lossy(&source));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            arena.read_to_str_lossy(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn large_allocation_keeps_previous_addresses_stable() {
        let arena = Bump::new();
        let first = arena.alloc(1_u8);
        let first_address: *mut u8 = first;
        let large = arena.alloc_slice_copy(&vec![7_u8; 8 * 1024 * 1024]);
        assert_eq!(large[0], 7);
        assert_eq!(*first, 1);
        assert_eq!(first_address as usize, std::ptr::from_ref(first) as usize);
        assert!(arena.inner.borrow().region.committed() >= 8 * 1024 * 1024);
    }

    #[test]
    fn grow_in_place_and_by_copy() {
        let arena = Bump::new();
        let old = Layout::from_size_align(8, 8).unwrap();
        let new = Layout::from_size_align(32, 8).unwrap();
        let first = arena.allocate(old).unwrap().cast::<u8>();
        // SAFETY: the allocation has at least eight writable bytes.
        unsafe {
            first.as_ptr().write_bytes(0x5A, 8);
        }
        // SAFETY: `first` is live and `new` is larger.
        let grown = unsafe { arena.grow(first, old, new) }.unwrap().cast::<u8>();
        assert_eq!(first, grown);
        // SAFETY: `grown` is live and has 32 initialized bytes at its start.
        assert_eq!(unsafe { *grown.as_ptr() }, 0x5A);
        _ = arena.allocate(old).unwrap();
        // SAFETY: `grown` is still live and `new` is larger than `old`.
        let copied = unsafe { arena.grow(grown, new, Layout::from_size_align(64, 8).unwrap()) }
            .unwrap()
            .cast::<u8>();
        assert_ne!(grown, copied);
        // SAFETY: the first byte was copied from the earlier allocation.
        assert_eq!(unsafe { *copied.as_ptr() }, 0x5A);
    }

    #[test]
    fn grow_across_commit_boundary_stays_in_place_and_zeroes_tail() {
        let arena = Bump::new();
        let old = Layout::from_size_align(3 * 1024 * 1024, 64).unwrap();
        let new = Layout::from_size_align(5 * 1024 * 1024, 64).unwrap();
        let first = arena.allocate(old).unwrap().cast::<u8>();
        // SAFETY: the first byte lies in the committed old allocation.
        unsafe {
            first.as_ptr().write(0xA5);
        }
        // SAFETY: `first` is live and `new` is larger and at least as aligned.
        let grown = unsafe { arena.grow_zeroed(first, old, new) }
            .unwrap()
            .cast::<u8>();
        assert_eq!(grown, first);
        assert!(arena.inner.borrow().region.committed() >= new.size());
        // SAFETY: all reads lie in the live grown allocation; the new tail
        // was initialized by `grow_zeroed`.
        assert_eq!(unsafe { grown.as_ptr().read() }, 0xA5);
        // SAFETY: this is the first byte of the initialized tail.
        assert_eq!(unsafe { grown.as_ptr().wrapping_add(old.size()).read() }, 0);
        // SAFETY: this is the last byte of the initialized tail.
        let final_byte = unsafe { grown.as_ptr().wrapping_add(new.size() - 1).read() };
        assert_eq!(final_byte, 0);
    }

    #[test]
    fn shrink_keeps_storage_and_latest_allocation_can_regrow() {
        let arena = Bump::new();
        let large = Layout::from_size_align(64, 8).unwrap();
        let small = Layout::from_size_align(16, 8).unwrap();
        let ptr = arena.allocate(large).unwrap().cast::<u8>();
        // SAFETY: `ptr` is live and the new layout is smaller.
        let shrunk = unsafe { arena.shrink(ptr, large, small) }.unwrap();
        assert_eq!(shrunk.cast::<u8>(), ptr);
        // SAFETY: the returned block fits `small`, and `large` is larger.
        let regrown = unsafe { arena.grow(ptr, small, large) }.unwrap();
        assert_eq!(regrown.cast::<u8>(), ptr);
        // SAFETY: the latest block is live and fits `small`.
        unsafe {
            arena.deallocate(ptr, small);
        }
        assert_eq!(arena.inner.borrow().used, 0);
    }

    #[test]
    fn deallocate_rolls_back_only_latest_and_reset_reuses_region() {
        let mut arena = Bump::new();
        let layout = Layout::new::<u64>();
        let first = arena.allocate(layout).unwrap().cast::<u8>();
        let second = arena.allocate(layout).unwrap().cast::<u8>();
        // SAFETY: both pointers came from this allocator with this layout.
        unsafe {
            arena.deallocate(first, layout);
        }
        let used = arena.inner.borrow().used;
        assert_eq!(used, 16);
        // SAFETY: second is the most recent live allocation.
        unsafe {
            arena.deallocate(second, layout);
        }
        assert_eq!(arena.inner.borrow().used, 8);
        let reused = arena.allocate(layout).unwrap().cast::<u8>();
        assert_eq!(reused, second);
        _ = arena
            .allocate(Layout::from_size_align(8 * 1024 * 1024, 8).unwrap())
            .unwrap();
        let base = arena.inner.borrow().region.as_ptr();
        let committed = arena.inner.borrow().region.committed();
        arena.reset();
        assert_eq!(arena.inner.borrow().used, 0);
        assert_eq!(arena.inner.borrow().region.committed(), committed);
        let after = arena.allocate(layout).unwrap().cast::<u8>();
        assert_eq!(after, base);
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

    #[test]
    fn owning_vector_grows_without_moving_its_elements() {
        let mut values = RegionVec::new_in(Bump::new());
        values.push(42_u64);
        let first = values.as_ptr();
        for value in 0..100_000 {
            values.push(value);
        }
        assert_eq!(values.as_ptr(), first);
        assert_eq!(values[0], 42);
        assert_eq!(values[100_000], 99_999);
    }
}
