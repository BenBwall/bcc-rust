//! A type-erased bump arena that stores values of many types in chunks.
//!
//! Each chunk ("block") holds values of a single type, packed from its start,
//! and never moves once allocated. The arena therefore grows a block at a
//! time instead of reallocating and copying like a `Vec`, and a reference to
//! a stored value stays valid while further values are added.
//!
//! Values are addressed by 4-byte handles. The handle space is divided into
//! 64 KiB pages; a block owns one page, or several consecutive pages when a
//! single list is larger than a page. Every access checks that the handle's
//! block holds the requested type and that the requested range lies within
//! its initialized part, so a stray or stale handle reads another value of
//! the same type or panics, never reinterprets memory as another type.
//!
//! Values are never dropped. The arena is meant for plain data such as
//! syntax nodes; any value that owns resources is leaked.

use std::{
    alloc::{
        Layout,
        alloc,
        dealloc,
        handle_alloc_error,
    },
    any::TypeId,
    cell::RefCell,
    fmt::{
        Debug,
        Formatter,
        Result as FmtResult,
    },
    marker::PhantomData,
    ptr::NonNull,
};

use allocator_api2::{
    alloc::Allocator,
    vec::Vec as AllocVec,
};

use super::HashMap;

const PAGE_BITS: u32 = 16;
const PAGE_BYTES: usize = 1 << PAGE_BITS;
const OFFSET_MASK: u32 = (1 << PAGE_BITS) - 1;
const MAX_PAGES: usize = 1 << (u32::BITS - PAGE_BITS);
/// Size of a type's first block. Later blocks double up to one page, so a
/// small arena stays small and a large one wastes at most one partly filled
/// page per type.
const FIRST_BLOCK_BYTES: usize = 1 << 10;

/// A contiguous run of values of one type: a handle to its first value and
/// its length. The empty run has no handle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ArenaRun {
    pub(crate) start:  u32,
    pub(crate) length: u32,
}

impl ArenaRun {
    pub(crate) const EMPTY: Self = Self {
        start:  u32::MAX,
        length: 0,
    };
}

/// A point the arena can return to with [`Arena::restore`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArenaCheckpoint {
    blocks:     usize,
    pages:      usize,
    generation: u64,
}

pub(crate) struct Arena {
    inner: RefCell<Inner>,
}

struct Inner {
    blocks:     Vec<Block>,
    /// For each page of handle space, the block it belongs to and the byte
    /// offset of the page's start within that block.
    pages:      Vec<Page>,
    kind_slots: HashMap<TypeId, usize>,
    kinds:      Vec<Kind>,
    /// Bookkeeping changed since the latest checkpoint, newest last.
    undo:       Vec<Undo>,
    generation: u64,
    recording:  bool,
}

struct Block {
    ptr:        NonNull<u8>,
    layout:     Layout,
    /// Bytes from `ptr` that hold initialized values.
    used:       usize,
    type_id:    TypeId,
    first_page: u32,
}

#[derive(Clone, Copy)]
struct Page {
    block:  u32,
    offset: u32,
}

/// Allocation state of one stored type.
struct Kind {
    /// The block new values of this type are added to.
    open_block:       Option<usize>,
    length:           usize,
    next_block_bytes: usize,
}

/// How to undo one addition: the state of its type and open block before it.
struct Undo {
    kind:             usize,
    open_block:       Option<usize>,
    open_block_used:  usize,
    length:           usize,
    next_block_bytes: usize,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub(crate) fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                blocks:     Vec::new(),
                pages:      Vec::new(),
                kind_slots: HashMap::default(),
                kinds:      Vec::new(),
                undo:       Vec::new(),
                generation: 0,
                recording:  false,
            }),
        }
    }

    /// Stores `value` and returns a reference to it. The value never moves,
    /// so the reference stays valid while more values are added.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The syntax store addresses nodes by 4-byte handles instead."
        )
    )]
    pub(crate) fn alloc<T: 'static>(&self, value: T) -> &T {
        let (_, ptr) = self.inner.borrow_mut().push(value);
        // SAFETY: `push` returns a pointer to the initialized value it
        // stored. The value's block is freed only by `restore` or `drop`,
        // which take `&mut self` and so cannot run while this shared borrow
        // lives, and the arena hands out no mutable reference to it then.
        unsafe { ptr.as_ref() }
    }

    /// Stores `value` and returns its handle.
    pub(crate) fn push<T: 'static>(&mut self, value: T) -> u32 {
        self.inner.get_mut().push(value).0
    }

    /// Moves every value out of `values`, in order, into one contiguous run.
    pub(crate) fn extend<T: 'static, A: Allocator>(
        &mut self,
        values: &mut AllocVec<T, A>,
    ) -> ArenaRun {
        self.inner.get_mut().extend(values)
    }

    /// The value at `handle`.
    ///
    /// # Panics
    ///
    /// If `handle` does not address a value of type `T` in this arena.
    pub(crate) fn get<T: 'static>(&self, handle: u32) -> &T {
        let ptr = self.inner.borrow().locate::<T>(handle, 1);
        // SAFETY: `locate` checked that `handle` addresses one initialized,
        // aligned `T` inside a live block. That block lives as long as the
        // shared borrow of `self`, which also excludes mutable access.
        unsafe { ptr.as_ref() }
    }

    /// The value at `handle`, mutably.
    ///
    /// # Panics
    ///
    /// If `handle` does not address a value of type `T` in this arena.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Syntax nodes never change once stored.")
    )]
    pub(crate) fn get_mut<T: 'static>(&mut self, handle: u32) -> &mut T {
        let mut ptr = self.inner.get_mut().locate::<T>(handle, 1);
        // SAFETY: As in `get`; the exclusive borrow of `self` guarantees no
        // other reference to the value exists.
        unsafe { ptr.as_mut() }
    }

    /// The values of `run`.
    ///
    /// # Panics
    ///
    /// If `run` is not a run of `T` values in this arena.
    pub(crate) fn slice<T: 'static>(&self, run: ArenaRun) -> &[T] {
        if run.length == 0 {
            return &[];
        }
        let ptr = self
            .inner
            .borrow()
            .locate::<T>(run.start, run.length as usize);
        // SAFETY: `locate` checked that `run.length` initialized, aligned,
        // consecutive `T` values start at `ptr` inside one live block, which
        // lives as long as the shared borrow of `self`.
        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), run.length as usize) }
    }

    /// The values of `run`, mutably.
    ///
    /// # Panics
    ///
    /// If `run` is not a run of `T` values in this arena.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Syntax nodes never change once stored.")
    )]
    pub(crate) fn slice_mut<T: 'static>(&mut self, run: ArenaRun) -> &mut [T] {
        if run.length == 0 {
            return &mut [];
        }
        let ptr = self
            .inner
            .get_mut()
            .locate::<T>(run.start, run.length as usize);
        // SAFETY: As in `slice`; the exclusive borrow of `self` guarantees no
        // other reference to these values exists.
        unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), run.length as usize) }
    }

    /// The number of values of type `T` stored.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Only tests count nodes by kind.")
    )]
    pub(crate) fn count<T: 'static>(&self) -> usize {
        let inner = self.inner.borrow();
        inner
            .kind_slots
            .get(&TypeId::of::<T>())
            .map_or(0, |&kind| inner.kinds[kind].length)
    }

    /// The number of values of every type stored.
    pub(crate) fn len(&self) -> usize {
        self.inner
            .borrow()
            .kinds
            .iter()
            .map(|kind| kind.length)
            .sum()
    }

    /// Every stored value of type `T`, in the order they were added.
    pub(crate) fn iter<T: 'static>(&self) -> impl Iterator<Item = &T> {
        BlockSlices {
            arena: self,
            next:  0,
            _type: PhantomData,
        }
        .flatten()
    }

    /// Marks the current contents so a later [`Self::restore`] can discard
    /// everything added after it. Only the most recent checkpoint can be
    /// restored.
    pub(crate) fn checkpoint(&mut self) -> ArenaCheckpoint {
        let inner = self.inner.get_mut();
        inner.undo.clear();
        inner.recording = true;
        inner.generation += 1;
        ArenaCheckpoint {
            blocks:     inner.blocks.len(),
            pages:      inner.pages.len(),
            generation: inner.generation,
        }
    }

    /// Discards every value added since `checkpoint`.
    ///
    /// # Panics
    ///
    /// If `checkpoint` is not the most recent checkpoint of this arena.
    pub(crate) fn restore(&mut self, checkpoint: ArenaCheckpoint) {
        let inner = self.inner.get_mut();
        assert_eq!(
            checkpoint.generation, inner.generation,
            "only the most recent arena checkpoint can be restored"
        );
        while let Some(undo) = inner.undo.pop() {
            if let Some(block) = undo.open_block {
                inner.blocks[block].used = undo.open_block_used;
            }
            let kind = &mut inner.kinds[undo.kind];
            kind.open_block = undo.open_block;
            kind.length = undo.length;
            kind.next_block_bytes = undo.next_block_bytes;
        }
        for block in inner.blocks.drain(checkpoint.blocks..) {
            // SAFETY: `block.ptr` was allocated with `block.layout` and no
            // kind refers to it any more. Handles into it now name pages
            // that are removed below or reassigned to new blocks, whose
            // type and bounds every access checks.
            unsafe {
                dealloc(block.ptr.as_ptr(), block.layout);
            }
        }
        inner.pages.truncate(checkpoint.pages);
    }
}

impl Inner {
    fn kind<T: 'static>(&mut self) -> usize {
        const {
            assert!(
                size_of::<T>() != 0,
                "the arena does not store zero-sized types"
            );
        }
        let type_id = TypeId::of::<T>();
        *self.kind_slots.entry(type_id).or_insert_with(|| {
            self.kinds.push(Kind {
                open_block:       None,
                length:           0,
                next_block_bytes: FIRST_BLOCK_BYTES,
            });
            self.kinds.len() - 1
        })
    }

    /// Makes room for `count` values of `T` in one block and returns the
    /// type's slot and that block. Records how to undo the addition when a
    /// checkpoint is active.
    fn reserve<T: 'static>(&mut self, count: usize) -> (usize, usize) {
        let kind = self.kind::<T>();
        let bytes = count
            .checked_mul(size_of::<T>())
            .expect("arena allocation size overflowed");
        let state = &self.kinds[kind];
        if self.recording {
            self.undo.push(Undo {
                kind,
                open_block: state.open_block,
                open_block_used: state.open_block.map_or(0, |block| self.blocks[block].used),
                length: state.length,
                next_block_bytes: state.next_block_bytes,
            });
        }
        if let Some(block) = state.open_block
            && self.blocks[block].layout.size() - self.blocks[block].used >= bytes
        {
            return (kind, block);
        }
        let capacity = if bytes > PAGE_BYTES {
            bytes.next_multiple_of(PAGE_BYTES)
        } else {
            state.next_block_bytes.max(bytes)
        };
        let block = self.allocate_block::<T>(capacity);
        let state = &mut self.kinds[kind];
        // An oversized run gets a block of its own and leaves the open block
        // in place for later small additions.
        if bytes <= PAGE_BYTES {
            state.open_block = Some(block);
            state.next_block_bytes = (state.next_block_bytes * 2).min(PAGE_BYTES);
        }
        (kind, block)
    }

    fn allocate_block<T: 'static>(&mut self, capacity: usize) -> usize {
        let layout = Layout::from_size_align(capacity, align_of::<T>())
            .expect("arena block layout is valid");
        let page_count = capacity.div_ceil(PAGE_BYTES);
        assert!(
            self.pages.len() + page_count <= MAX_PAGES,
            "arena exceeded its 4 GiB handle space"
        );
        // SAFETY: `capacity` is nonzero because `T` is not zero-sized.
        let ptr = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(ptr) else {
            handle_alloc_error(layout)
        };
        let block = self.blocks.len();
        let first_page = self.pages.len();
        self.pages.extend((0..page_count).map(|page| Page {
            block:  u32::try_from(block).expect("block count fits in u32"),
            offset: u32::try_from(page * PAGE_BYTES).expect("block offset fits in u32"),
        }));
        self.blocks.push(Block {
            ptr,
            layout,
            used: 0,
            type_id: TypeId::of::<T>(),
            first_page: u32::try_from(first_page).expect("page count fits in u32"),
        });
        block
    }

    /// The handle of the value `offset` bytes into `block`.
    fn handle(&self, block: usize, offset: usize) -> u32 {
        let page = self.blocks[block].first_page as usize + offset / PAGE_BYTES;
        let page = u32::try_from(page).expect("page index fits in u32");
        let offset = u32::try_from(offset % PAGE_BYTES).expect("page offset fits in u32");
        (page << PAGE_BITS) | offset
    }

    fn push<T: 'static>(&mut self, value: T) -> (u32, NonNull<T>) {
        let (kind, block) = self.reserve::<T>(1);
        let offset = self.blocks[block].used;
        // SAFETY: `reserve` left room for one `T` at `offset`, which is a
        // multiple of `size_of::<T>()` into a block aligned for `T`.
        let ptr = unsafe { self.blocks[block].ptr.add(offset) }.cast::<T>();
        // SAFETY: `ptr` is valid for writes of one `T` and holds no value.
        unsafe {
            ptr.write(value);
        }
        self.commit::<T>(kind, block, 1);
        (self.handle(block, offset), ptr)
    }

    fn extend<T: 'static, A: Allocator>(&mut self, values: &mut AllocVec<T, A>) -> ArenaRun {
        if values.is_empty() {
            return ArenaRun::EMPTY;
        }
        let count = values.len();
        let (kind, block) = self.reserve::<T>(count);
        let offset = self.blocks[block].used;
        // SAFETY: `reserve` left room for `count` values of `T` at `offset`,
        // a multiple of `size_of::<T>()` into a block aligned for `T`.
        let ptr = unsafe { self.blocks[block].ptr.add(offset) }.cast::<T>();
        for (index, value) in values.drain(..).enumerate() {
            // SAFETY: `index < count`, so the slot lies in the reserved room.
            let slot = unsafe { ptr.add(index) };
            // SAFETY: The slot is valid for writes of one `T` and holds no
            // value yet.
            unsafe {
                slot.write(value);
            }
        }
        self.commit::<T>(kind, block, count);
        ArenaRun {
            start:  self.handle(block, offset),
            length: u32::try_from(count).expect("arena run length fits in u32"),
        }
    }

    /// Marks `count` values just written at the end of `block` initialized.
    fn commit<T: 'static>(&mut self, kind: usize, block: usize, count: usize) {
        self.blocks[block].used += count * size_of::<T>();
        self.kinds[kind].length += count;
    }

    /// Checks that `count` consecutive values of `T` start at `handle` and
    /// returns a pointer to the first.
    fn locate<T: 'static>(&self, handle: u32, count: usize) -> NonNull<T> {
        let page = self
            .pages
            .get((handle >> PAGE_BITS) as usize)
            .expect("arena handle names an allocated page");
        let block = &self.blocks[page.block as usize];
        assert!(
            block.type_id == TypeId::of::<T>(),
            "arena handle names a value of another type"
        );
        let start = page.offset as usize + (handle & OFFSET_MASK) as usize;
        assert!(
            start.is_multiple_of(size_of::<T>()),
            "arena handle is not at a value boundary"
        );
        let end = count
            .checked_mul(size_of::<T>())
            .and_then(|bytes| start.checked_add(bytes))
            .expect("arena range overflowed");
        assert!(end <= block.used, "arena handle is out of bounds");
        // SAFETY: `start < end <= used <= capacity`, so the offset stays
        // within the block's allocation.
        unsafe { block.ptr.add(start) }.cast::<T>()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        for block in &self.blocks {
            // SAFETY: Every block was allocated with its stored layout and is
            // freed exactly once, here or in `restore`, which removes it from
            // `blocks`. Values are leaked rather than dropped.
            unsafe {
                dealloc(block.ptr.as_ptr(), block.layout);
            }
        }
    }
}

/// The initialized part of each `T` block, in allocation order.
struct BlockSlices<'a, T> {
    arena: &'a Arena,
    next:  usize,
    _type: PhantomData<fn() -> T>,
}

impl<'a, T: 'static> Iterator for BlockSlices<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<&'a [T]> {
        let inner = self.arena.inner.borrow();
        loop {
            let block = inner.blocks.get(self.next)?;
            self.next += 1;
            if block.type_id == TypeId::of::<T>() {
                // SAFETY: A `T` block holds `used / size_of::<T>()`
                // initialized, aligned values from its start. It lives as
                // long as the shared borrow of the arena, which excludes
                // mutable access.
                return Some(unsafe {
                    std::slice::from_raw_parts(
                        block.ptr.cast::<T>().as_ptr(),
                        block.used / size_of::<T>(),
                    )
                });
            }
        }
    }
}

impl Debug for Arena {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let inner = self.inner.borrow();
        f.debug_struct("Arena")
            .field("blocks", &inner.blocks.len())
            .field(
                "values",
                &inner.kinds.iter().map(|kind| kind.length).sum::<usize>(),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Pair(u64, u8);

    #[test]
    fn values_of_many_types_read_back_by_handle() {
        let mut arena = Arena::new();
        let byte = arena.push(7_u8);
        let pair = arena.push(Pair(u64::MAX, 3));
        let word = arena.push(0x1234_u32);
        assert_eq!(*arena.get::<u8>(byte), 7);
        assert_eq!(*arena.get::<Pair>(pair), Pair(u64::MAX, 3));
        assert_eq!(*arena.get::<u32>(word), 0x1234);
        arena.get_mut::<Pair>(pair).1 = 9;
        assert_eq!(arena.get::<Pair>(pair).1, 9);
        assert_eq!(arena.count::<Pair>(), 1);
        assert_eq!(arena.len(), 3);
    }

    #[test]
    fn references_stay_valid_while_the_arena_grows() {
        let arena = Arena::new();
        let first = arena.alloc(Pair(1, 1));
        let rest: Vec<&Pair> = (0..10_000).map(|i| arena.alloc(Pair(i, 2))).collect();
        assert_eq!(*first, Pair(1, 1));
        assert!(rest.iter().enumerate().all(|(i, pair)| pair.0 == i as u64));
    }

    #[test]
    fn runs_are_contiguous_including_runs_larger_than_a_page() {
        let mut arena = Arena::new();
        let small = arena.extend(&mut allocator_api2::vec![1_u16, 2, 3]);
        let large_values: AllocVec<u64> = (0..20_000).collect();
        let large = arena.extend(&mut large_values.clone());
        let after = arena.push(5_u64);
        assert_eq!(arena.slice::<u16>(small), &[1, 2, 3]);
        assert_eq!(arena.slice::<u64>(large), &large_values[..]);
        assert_eq!(*arena.get::<u64>(after), 5);
        assert_eq!(arena.slice::<u64>(ArenaRun::EMPTY), [0_u64; 0]);
        arena.slice_mut::<u16>(small)[1] = 8;
        assert_eq!(arena.slice::<u16>(small), &[1, 8, 3]);
        let element = ArenaRun {
            start:  large.start,
            length: 1,
        };
        assert_eq!(arena.slice::<u64>(element), &[0]);
    }

    #[test]
    fn iteration_follows_insertion_order_per_type() {
        let mut arena = Arena::new();
        for i in 0..5_000_u32 {
            let _ = arena.push(i);
            let _ = arena.push(u64::from(i) * 2);
        }
        assert!(arena.iter::<u32>().copied().eq(0..5_000));
        assert!(arena.iter::<u64>().copied().eq((0..5_000).map(|i| i * 2)));
        assert_eq!(arena.iter::<u8>().count(), 0);
    }

    #[test]
    #[should_panic(expected = "another type")]
    fn a_handle_cannot_read_another_type() {
        let mut arena = Arena::new();
        let handle = arena.push(1_u32);
        let _ = arena.get::<f32>(handle);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn a_run_cannot_extend_past_its_values() {
        let mut arena = Arena::new();
        let run = arena.extend(&mut allocator_api2::vec![1_u32, 2]);
        let _ = arena.slice::<u32>(ArenaRun {
            start:  run.start,
            length: 3,
        });
    }

    #[test]
    fn restore_discards_values_added_after_the_checkpoint() {
        let mut arena = Arena::new();
        let kept = arena.push(1_u32);
        let checkpoint = arena.checkpoint();
        let _ = arena.push(2_u32);
        let _ = arena.extend(&mut (0..40_000_u32).collect::<AllocVec<_>>());
        let _ = arena.push(3_u8);
        arena.restore(checkpoint);
        assert_eq!(arena.count::<u32>(), 1);
        assert_eq!(arena.count::<u8>(), 0);
        assert!(arena.iter::<u32>().copied().eq([1]));
        assert_eq!(*arena.get::<u32>(kept), 1);
        let reused = arena.push(4_u32);
        assert_eq!(*arena.get::<u32>(reused), 4);
        assert!(arena.iter::<u32>().copied().eq([1, 4]));
    }

    #[test]
    fn values_that_own_resources_are_leaked_not_dropped() {
        use std::rc::Rc;

        let shared = Rc::new(());
        let arena = Arena::new();
        let _ = arena.alloc(Rc::clone(&shared));
        drop(arena);
        assert_eq!(Rc::strong_count(&shared), 2);
        // SAFETY: The arena leaked exactly one strong reference, released
        // here so the allocation is freed and leak checkers stay quiet.
        unsafe {
            Rc::decrement_strong_count(Rc::as_ptr(&shared));
        }
        assert_eq!(Rc::strong_count(&shared), 1);
    }
}
