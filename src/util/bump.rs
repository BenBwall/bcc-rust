//! Virtual-memory arena allocation for phase-scoped compiler data.
//!
//! An arena reserves its region on its first allocation, so constructing one
//! (including through `Default` or `mem::take`) costs nothing. The reservation
//! never moves. Individual deallocation normally leaves memory in the arena;
//! dropping releases the region and reset rewinds it. Values put here must not
//! need destruction, since the arena does not run destructors.
//!
//! Commit follows use: the region commits pages as the bump pointer reaches
//! them, at most one commit step ahead. A [`TailVec`] takes the rest of the
//! reservation as its capacity and commits as it is written, so a vector
//! whose final length is unknown never commits room it does not fill.

use std::{
    alloc::{
        Layout,
        handle_alloc_error,
    },
    cell::RefCell,
    fmt,
    io::Read,
    marker::PhantomData,
    mem::ManuallyDrop,
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

use super::vm::{
    GrowingRegion,
    REGION_BYTES,
};

#[derive(Clone, Copy)]
struct Last {
    ptr:   NonNull<u8>,
    start: usize,
    size:  usize,
}

struct Inner {
    /// Reserved by the first allocation that needs memory.
    region:     Option<GrowingRegion>,
    /// Every live block lies below this offset.
    used:       usize,
    last:       Option<Last>,
    /// The largest `used` since the arena was created.
    high_water: usize,
    /// Where the open [`TailVec`] starts, if one is open. It owns everything
    /// from there to the end of the reservation, so nothing else may
    /// allocate until it closes.
    tail:       Option<usize>,
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
    /// An empty arena. It holds no region until something is allocated.
    pub(crate) const fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                region:     None,
                used:       0,
                last:       None,
                high_water: 0,
                tail:       None,
            }),
        }
    }

    /// The most bytes this arena has held at once, including alignment
    /// padding and blocks a reset later reclaimed.
    #[cfg_attr(
        not(any(test, feature = "benchmarking-internals")),
        expect(dead_code, reason = "Only measurements read the high-water mark.")
    )]
    pub(crate) fn high_water(&self) -> usize {
        self.inner.borrow().high_water
    }

    /// The bytes now allocated, including alignment padding.
    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.inner.borrow().used
    }

    /// The bytes committed in this arena's region.
    #[cfg(test)]
    pub(crate) fn committed(&self) -> usize {
        self.inner
            .borrow()
            .region
            .as_ref()
            .map_or(0, GrowingRegion::committed)
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
    ///
    /// Running out of arena memory is reported as
    /// [`std::io::ErrorKind::OutOfMemory`] rather than aborting, so a caller
    /// can turn it into a diagnostic.
    pub(crate) fn read_to_str_lossy(&self, path: &Path) -> std::io::Result<&str> {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = ArenaVec::new_in(self);
        let mut buffer = [0_u8; 8192];
        loop {
            match file.read(&mut buffer) {
                | Ok(0) => break,
                | Ok(count) => {
                    bytes
                        .try_reserve(count)
                        .map_err(|_| out_of_arena_memory())?;
                    bytes.extend_from_slice(&buffer[..count]);
                },
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
        let mut push = |part: &[u8]| {
            repaired
                .try_reserve(part.len())
                .map_err(|_| out_of_arena_memory())?;
            repaired.extend_from_slice(part);
            Ok::<_, std::io::Error>(())
        };
        let mut remaining = bytes.as_slice();
        while let Err(error) = std::str::from_utf8(remaining) {
            let valid = error.valid_up_to();
            push(&remaining[..valid])?;
            push("�".as_bytes())?;
            remaining = &remaining[valid + error.error_len().unwrap_or(remaining.len() - valid)..];
        }
        push(remaining)?;
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

    /// A vector at the end of the arena whose capacity is the rest of the
    /// reservation, so it never moves or copies. It commits pages as it is
    /// written. Nothing else may allocate in the arena until the vector is
    /// finished or dropped; an allocation attempted meanwhile fails (and is
    /// a debug assertion), so the two can never overlap.
    pub(crate) fn tail_vec<T: Copy>(&self) -> TailVec<'_, T> {
        const {
            assert!(size_of::<T>() != 0, "tail vectors hold sized elements");
        }
        let mut inner = self.inner.borrow_mut();
        assert!(
            inner.tail.is_none(),
            "an arena has one tail vector at a time"
        );
        // An unreserved region will start page-aligned at offset zero, so it
        // is aligned for any element. If aligning overflows, the start lies
        // past the reservation and every commit fails.
        let start = inner.region.as_ref().map_or(Some(0), |region| {
            aligned_start(
                region.as_ptr().as_ptr() as usize,
                inner.used,
                align_of::<T>(),
            )
        });
        let start = start.unwrap_or(usize::MAX);
        inner.tail = Some(start);
        inner.last = None;
        TailVec {
            arena: self,
            start,
            ptr: NonNull::dangling(),
            len: 0,
            committed: 0,
            restore: inner.used,
            elements: PhantomData,
        }
    }

    /// Rewind the arena while retaining its committed pages for reuse.
    pub(crate) fn reset(&mut self) {
        let inner = self.inner.get_mut();
        inner.used = 0;
        inner.last = None;
    }
}

fn out_of_arena_memory() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        "the compiler's arena memory is exhausted",
    )
}

fn aligned_start(base: usize, used: usize, align: usize) -> Option<usize> {
    base.checked_add(used)?
        .checked_add(align - 1)
        .map(|address| (address & !(align - 1)) - base)
}

impl Inner {
    /// The region, reserved now if this is the first allocation needing one.
    fn region(&mut self) -> Result<&mut GrowingRegion, AllocError> {
        match &mut self.region {
            | Some(region) => Ok(region),
            | region @ None =>
                Ok(region.insert(GrowingRegion::reserve(REGION_BYTES).map_err(|_| AllocError)?)),
        }
    }

    /// A pointer to `start`, which must be below the committed prefix.
    fn pointer_at(region: &GrowingRegion, start: usize) -> NonNull<u8> {
        // SAFETY: callers commit through the end of the block at `start`
        // before asking, so the offset lies inside the reservation. The base
        // is taken after that commit, so it reaches the whole block.
        let address = unsafe { region.as_ptr().as_ptr().add(start) };
        // SAFETY: adding an in-bounds offset to a non-null pointer remains
        // non-null.
        unsafe { NonNull::new_unchecked(address) }
    }

    fn set_used(&mut self, used: usize) {
        self.used = used;
        self.high_water = self.high_water.max(used);
    }

    fn allocate(&mut self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            // A dangling pointer aligned for the layout, as `NonNull::dangling`
            // is for a type.
            let ptr =
                NonNull::new(ptr::without_provenance_mut(layout.align())).ok_or(AllocError)?;
            return Ok(NonNull::slice_from_raw_parts(ptr, 0));
        }
        debug_assert!(
            self.tail.is_none(),
            "nothing else allocates in an arena while its tail vector is open"
        );
        // The open tail owns the rest of the reservation; release builds
        // refuse rather than overlap it.
        if self.tail.is_some() {
            return Err(AllocError);
        }
        let used = self.used;
        let region = self.region()?;
        let start = aligned_start(region.as_ptr().as_ptr() as usize, used, layout.align())
            .ok_or(AllocError)?;
        let end = start.checked_add(layout.size()).ok_or(AllocError)?;
        region.ensure_committed(end).map_err(|_| AllocError)?;
        let ptr = Self::pointer_at(region, start);
        self.set_used(end);
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
            let inner = &mut *inner;
            // A latest block exists, so the region was reserved for it.
            let latest = inner
                .last
                .filter(|last| last.ptr == ptr && old_layout.size() <= last.size)
                .filter(|_| (ptr.as_ptr() as usize).is_multiple_of(new_layout.align()));
            if let Some(last) = latest
                && let Some(region) = inner.region.as_mut()
                && last
                    .start
                    .checked_add(new_layout.size())
                    .is_some_and(|end| region.ensure_committed(end).is_ok())
            {
                // The same address, reached through the newly committed
                // pages.
                let grown = Inner::pointer_at(region, last.start);
                inner.set_used(last.start + new_layout.size());
                inner.last = Some(Last {
                    ptr: grown,
                    size: new_layout.size(),
                    ..last
                });
                return Ok(NonNull::slice_from_raw_parts(grown, new_layout.size()));
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
        let mut inner = self.inner.borrow_mut();
        // Shrinking the latest block returns its tail to the arena, so a
        // vector sized by doubling can end up exact.
        if let Some(last) = inner
            .last
            .filter(|last| last.ptr == ptr && old_layout.size() <= last.size)
            .filter(|_| new_layout.size() != 0)
        {
            inner.used = last.start + new_layout.size();
            inner.last = Some(Last {
                size: new_layout.size(),
                ..last
            });
            return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        }
        Ok(NonNull::slice_from_raw_parts(ptr, old_layout.size()))
    }
}

/// A vector at the end of an arena that commits as it is written; see
/// [`Bump::tail_vec`].
pub(crate) struct TailVec<'a, T: Copy> {
    arena:     &'a Bump,
    /// The first element's offset in the arena's region.
    start:     usize,
    /// The first element, derived after the latest commit so it reaches
    /// every committed element. Dangling until the first commit.
    ptr:       NonNull<T>,
    len:       usize,
    /// How many elements fit in committed memory.
    committed: usize,
    /// The arena's `used` before the vector opened, restored if it is
    /// dropped unfinished.
    restore:   usize,
    elements:  PhantomData<&'a mut [T]>,
}

impl<'a, T: Copy> TailVec<'a, T> {
    pub(crate) const fn len(&self) -> usize {
        self.len
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
            self.commit_for(self.len + 1)?;
        }
        // SAFETY: `len < committed`, so the slot lies inside the committed
        // memory `ptr` was derived to reach, and it lies inside this vector.
        let slot = unsafe { self.ptr.as_ptr().add(self.len) };
        // SAFETY: the slot is committed, aligned (`start` was aligned for
        // `T`), and owned by this vector alone while the tail is open.
        unsafe {
            slot.write(value);
        }
        self.len += 1;
        Ok(())
    }

    #[cold]
    fn commit_for(&mut self, elements: usize) -> Result<(), AllocError> {
        let end = elements
            .checked_mul(size_of::<T>())
            .and_then(|bytes| bytes.checked_add(self.start))
            .ok_or(AllocError)?;
        let mut inner = self.arena.inner.borrow_mut();
        let region = inner.region()?;
        region.ensure_committed(end).map_err(|_| AllocError)?;
        self.committed = (region.committed() - self.start) / size_of::<T>();
        self.ptr = Inner::pointer_at(region, self.start).cast();
        Ok(())
    }

    /// Closes the tail, keeping exactly the written elements in the arena
    /// and returning the rest of the reservation to it.
    pub(crate) fn into_slice(self) -> &'a mut [T] {
        let this = ManuallyDrop::new(self);
        let mut inner = this.arena.inner.borrow_mut();
        inner.tail = None;
        if this.len == 0 {
            inner.used = this.restore;
            return &mut [];
        }
        inner.set_used(this.start + this.len * size_of::<T>());
        drop(inner);
        // SAFETY: the first `len` elements were written into committed memory
        // that `ptr` reaches. They now lie below the arena's `used`, so later
        // allocations never overlap them, and they stay put until reset or
        // drop, which need the exclusive access this `'a` borrow excludes.
        unsafe { std::slice::from_raw_parts_mut(this.ptr.as_ptr(), this.len) }
    }
}

impl<T: Copy> Drop for TailVec<'_, T> {
    /// Closes the tail without keeping anything; the elements need no drop.
    fn drop(&mut self) {
        let mut inner = self.arena.inner.borrow_mut();
        inner.tail = None;
        inner.used = self.restore;
    }
}

pub(crate) type ArenaVec<'a, T> = AllocVec<T, &'a Bump>;
pub(crate) type ArenaMap<'a, K, V> = hashbrown::HashMap<K, V, FxBuildHasher, &'a Bump>;
pub(crate) type ArenaSet<'a, T> = hashbrown::HashSet<T, FxBuildHasher, &'a Bump>;

/// A UTF-8 string allocated in an arena.
pub(crate) struct ArenaString<'a> {
    bytes: ArenaVec<'a, u8>,
}

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
pub(crate) struct ArenaQueue<'a, T> {
    data: ArenaVec<'a, Option<T>>,
    read: usize,
}

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
        if self.read == self.data.len() {
            self.data.clear();
            self.read = 0;
        }
        value
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len() - self.read
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Only the queue's own tests ask this so far.")
    )]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the queue's own tests read it in order so far."
        )
    )]
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.data[self.read..]
            .iter()
            .map(|item| item.as_ref().expect("unread queue item"))
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        self.data[self.read..]
            .iter_mut()
            .map(|item| item.as_mut().expect("unread queue item"))
    }

    pub(crate) fn back(&self) -> Option<&T> {
        self.data[self.read..].last().and_then(Option::as_ref)
    }

    pub(crate) fn back_mut(&mut self) -> Option<&mut T> {
        self.data[self.read..].last_mut().and_then(Option::as_mut)
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        drop(self.data.drain(..self.read));
        self.read = 0;
        self.data
            .retain(|item| keep(item.as_ref().expect("unread queue item")));
    }
}

impl<T> Extend<T> for ArenaQueue<'_, T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for value in iter {
            self.push_back(value);
        }
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
    };
    use crate::util::vm::{
        MAX_COMMIT_STEP,
        accounting,
        faults,
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
        assert!(arena.committed() >= 40_000);
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
        assert!(arena.committed() >= 8 * 1024 * 1024);
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
        assert!(arena.committed() >= new.size());
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
        assert_eq!(arena.used(), 0);
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
        let used = arena.used();
        assert_eq!(used, 16);
        // SAFETY: second is the most recent live allocation.
        unsafe {
            arena.deallocate(second, layout);
        }
        assert_eq!(arena.used(), 8);
        let reused = arena.allocate(layout).unwrap().cast::<u8>();
        assert_eq!(reused, second);
        _ = arena
            .allocate(Layout::from_size_align(8 * 1024 * 1024, 8).unwrap())
            .unwrap();
        let base = arena.inner.borrow().region.as_ref().unwrap().as_ptr();
        let committed = arena.committed();
        arena.reset();
        assert_eq!(arena.used(), 0);
        assert_eq!(arena.committed(), committed);
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
    fn queue_retains_unread_values_after_front_removal() {
        let arena = Bump::new();
        let mut queue = ArenaQueue::new_in(&arena);
        queue.extend([1, 2, 3, 4]);
        assert_eq!(queue.pop_front(), Some(1));
        queue.retain(|value| value % 2 == 0);
        assert_eq!(queue.iter().copied().collect::<Vec<_>>(), [2, 4]);
        *queue.back_mut().unwrap() = 6;
        assert_eq!(queue.back(), Some(&6));
        for value in queue.iter_mut() {
            *value += 1;
        }
        assert_eq!(queue.pop_front(), Some(3));
        assert_eq!(queue.pop_front(), Some(7));
        assert!(queue.is_empty());
        assert_eq!(queue.back(), None);
        queue.push_back(9);
        assert_eq!(queue.pop_front(), Some(9));
    }

    #[test]
    fn arenas_reserve_on_first_allocation() {
        let before = accounting::live();
        let mut arena = Bump::new();
        let taken = std::mem::take(&mut arena);
        let empty = Bump::default();
        assert_eq!(accounting::live(), before);
        _ = taken.alloc_slice_copy::<u8>(&[]);
        assert_eq!(
            accounting::live(),
            before,
            "zero-sized blocks need no region"
        );
        _ = taken.alloc(1_u8);
        assert_eq!(accounting::live().regions, before.regions + 1);
        drop((arena, taken, empty));
        assert_eq!(accounting::live(), before);
    }

    #[test]
    fn failed_reservation_is_an_allocation_error() {
        let arena = Bump::new();
        let layout = Layout::new::<u64>();
        {
            let _failing = faults::fail_reserves(1);
            _ = arena.allocate(layout).unwrap_err();
            let mut values = ArenaVec::<u8>::new_in(&arena);
            // The failure was used up; this reserves normally.
            values.try_reserve(16).unwrap();
        }
        let other = Bump::new();
        let _failing = faults::fail_reserves(1);
        let mut values = ArenaVec::<u8>::new_in(&other);
        _ = values.try_reserve(16).unwrap_err();
    }

    #[test]
    fn failed_commit_is_an_allocation_error_and_the_arena_recovers() {
        let arena = Bump::new();
        let first = arena.alloc(1_u64);
        let committed = arena.committed();
        let large = Layout::from_size_align(committed, 8).unwrap();
        {
            let _failing = faults::fail_commits(2);
            _ = arena.allocate(large).unwrap_err();
        }
        assert_eq!(arena.committed(), committed);
        assert_eq!(arena.used(), 8);
        let next = arena.alloc(2_u64);
        assert_eq!(
            std::ptr::from_mut(next) as usize,
            std::ptr::from_mut(first) as usize + 8
        );
        _ = arena.allocate(large).unwrap();
    }

    #[test]
    fn grow_copies_when_the_in_place_commit_fails() {
        let arena = Bump::new();
        let old = Layout::from_size_align(16, 8).unwrap();
        let block = arena.allocate(old).unwrap().cast::<u8>();
        // SAFETY: the block has sixteen writable bytes.
        unsafe {
            block.as_ptr().write_bytes(0x3C, 16);
        }
        let new = Layout::from_size_align(arena.committed() + 1, 8).unwrap();
        let _failing = faults::fail_commits(2);
        // SAFETY: `block` is live and `new` is larger.
        let grown = unsafe { arena.grow(block, old, new) }.unwrap().cast::<u8>();
        assert_ne!(grown, block);
        // SAFETY: the grown block starts with the sixteen copied bytes.
        let copied = unsafe { std::slice::from_raw_parts(grown.as_ptr(), 16) };
        assert!(copied.iter().all(|&byte| byte == 0x3C));
    }

    #[test]
    fn requests_beyond_the_reservation_fail() {
        let arena = Bump::new();
        let too_large = Layout::from_size_align(super::REGION_BYTES + 1, 1).unwrap();
        _ = arena.allocate(too_large).unwrap_err();
        let mut values = ArenaVec::<u8>::new_in(&arena);
        _ = values.try_reserve(super::REGION_BYTES + 1).unwrap_err();
        values.push(1);
        assert_eq!(values, [1]);
    }

    #[test]
    fn shrinking_the_latest_block_returns_its_tail() {
        let arena = Bump::new();
        let mut values = ArenaVec::with_capacity_in(1000, &arena);
        values.extend(0_u32..10);
        values.shrink_to_fit();
        let values = values.leak();
        assert_eq!(arena.used(), 40);
        let next = arena.alloc(7_u32);
        assert_eq!(
            std::ptr::from_mut(next) as usize,
            values.as_ptr() as usize + 40
        );
        assert_eq!(arena.high_water(), 4000);
    }

    #[test]
    fn high_water_survives_reset() {
        let mut arena = Bump::new();
        _ = arena.alloc_slice_copy(&[0_u8; 100]);
        arena.reset();
        _ = arena.alloc_slice_copy(&[0_u8; 10]);
        assert_eq!(arena.high_water(), 100);
    }

    #[test]
    fn tail_vector_follows_the_last_block_and_keeps_exactly_its_length() {
        let arena = Bump::new();
        let before: *mut u8 = arena.alloc(1_u8);
        let mut values = arena.tail_vec::<u32>();
        for value in 0..10_000 {
            values.push(value);
        }
        assert_eq!(values.len(), 10_000);
        let values = values.into_slice();
        assert_eq!(values[9_999], 9_999);
        // Aligned after the one-byte block, then exactly the elements.
        assert_eq!(values.as_ptr() as usize, before as usize + 4);
        assert_eq!(arena.used(), 4 + 40_000);
        assert_eq!(arena.high_water(), arena.used());
        let next: *mut u8 = arena.alloc(2_u8);
        assert_eq!(next as usize, before as usize + 40_004);
        assert_eq!(arena.tail_vec::<u64>().into_slice(), [0_u64; 0]);
        assert_eq!(arena.used(), 40_005);
    }

    #[cfg_attr(miri, ignore = "writes megabytes an element at a time")]
    #[test]
    fn tail_vector_commits_by_length_not_capacity() {
        let arena = Bump::new();
        _ = arena.alloc_slice_copy(&[0_u8; 1000]);
        let mut values = arena.tail_vec::<[u8; 17]>();
        let count = 5 * MAX_COMMIT_STEP / 17;
        for _ in 0..count {
            values.push([3; 17]);
        }
        assert!(arena.committed() >= 1000 + 17 * count);
        assert!(arena.committed() <= 1000 + 17 * count + MAX_COMMIT_STEP);
        let values = values.into_slice();
        assert_eq!(values.len(), count);
        assert_eq!(arena.used(), 1000 + 17 * count);
    }

    #[test]
    fn a_dropped_tail_vector_returns_everything() {
        let arena = Bump::new();
        let first: *mut u64 = arena.alloc(1_u64);
        {
            let mut values = arena.tail_vec::<u64>();
            values.push(2);
        }
        assert_eq!(arena.used(), 8);
        let second: *mut u64 = arena.alloc(3_u64);
        assert_eq!(second as usize, first as usize + 8);
        // An empty arena opens its tail before reserving a region.
        let empty = Bump::new();
        drop(empty.tail_vec::<u8>());
        assert_eq!(empty.used(), 0);
        let mut values = empty.tail_vec::<u8>();
        values.push(4);
        assert_eq!(values.into_slice(), [4]);
    }

    #[test]
    fn tail_vector_commit_failure_is_an_allocation_error() {
        let arena = Bump::new();
        {
            let _failing = faults::fail_reserves(1);
            let mut values = arena.tail_vec::<u8>();
            _ = values.try_push(1).unwrap_err();
        }
        let mut values = arena.tail_vec::<u8>();
        values.try_push(1).unwrap();
        while values.len() < arena.committed() {
            values.push(2);
        }
        {
            let _failing = faults::fail_commits(usize::MAX);
            _ = values.try_push(3).unwrap_err();
        }
        {
            // A failed step retries with only the page the push needs.
            let _limited = faults::limit_commits(4096);
            values.try_push(3).unwrap();
        }
        let length = values.len();
        let values = values.into_slice();
        assert_eq!(values.len(), length);
        assert_eq!(
            (values[0], values[length - 2], values[length - 1]),
            (1, 2, 3)
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "nothing else allocates in an arena while its tail vector is open")]
    fn allocating_beside_an_open_tail_vector_is_a_bug() {
        let arena = Bump::new();
        let mut values = arena.tail_vec::<u8>();
        values.push(1);
        _ = arena.alloc(2_u8);
    }
}
