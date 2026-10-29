//! Arena blocks too large to grow by copying, each in its own region.
//!
//! A vector or hash table in a [`super::bump::Bump`] grows by asking for a
//! block twice the size and copying into it. Only the arena's latest block
//! can grow in place, so a table that grows while anything else allocates
//! leaves each outgrown copy behind: about as many bytes again as its final
//! capacity, committed and written, until the arena is dropped. A large
//! block instead gets a region of its own, grows in place by committing
//! further into that region, and returns the region to the operating system
//! when it is deallocated, so a large table costs only its capacity.
//!
//! Each region starts with a one-page [`Header`] that links it into its
//! arena's list; the block follows at [`BLOCK_OFFSET`].

use std::{
    alloc::Layout,
    ptr::{
        self,
        NonNull,
    },
};

use allocator_api2::alloc::AllocError;

use crate::util::vm::{
    GrowingRegion,
    REGION_BYTES,
};

/// The smallest block given its own region. Below it, outgrown copies are
/// bounded by the threshold itself, and a compilation's regions stay few:
/// only per-translation-unit tables reach it, as one entry per token or
/// expression of a large input does.
pub(super) const LARGE_BLOCK_BYTES: usize = 16 * 1024 * 1024;

/// Whether large blocks get regions of their own. WebAssembly regions are
/// real allocations of linear memory and the explorer's inputs are small,
/// Miri models a region as one allocation, and the study profile shrinks
/// regions, so there every block stays in the arena.
pub(super) const ENABLED: bool = cfg!(not(any(
    miri,
    target_family = "wasm",
    feature = "parser-study-small-regions"
)));

/// Where a block starts in its region: one page past the header, so a
/// block may ask for any alignment up to a page.
pub(super) const BLOCK_OFFSET: usize = 4096;

const _: () = assert!(
    size_of::<Header>() <= BLOCK_OFFSET,
    "a header fits before its block"
);

/// The large blocks of one arena, as a doubly linked list through their
/// regions' headers.
pub(super) struct LargeBlocks {
    first: Option<NonNull<Header>>,
    /// The bytes the live blocks hold.
    bytes: usize,
}

/// The start of a large block's region.
struct Header {
    /// The region this header lies in, moved out to release it.
    region:   GrowingRegion,
    /// The block's current size.
    size:     usize,
    previous: Option<NonNull<Header>>,
    next:     Option<NonNull<Header>>,
}

impl LargeBlocks {
    /// Whether a block of `layout` gets its own region.
    pub(super) const fn wants(layout: Layout) -> bool {
        ENABLED && layout.size() >= LARGE_BLOCK_BYTES && layout.align() <= BLOCK_OFFSET
    }

    /// A new block of `layout` in a region of its own.
    pub(super) fn allocate(&mut self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let end = BLOCK_OFFSET.checked_add(layout.size()).ok_or(AllocError)?;
        let mut region = GrowingRegion::reserve(REGION_BYTES).map_err(|_| AllocError)?;
        region.ensure_committed(end).map_err(|_| AllocError)?;
        let header = region.as_ptr().cast::<Header>();
        let next = self.first;
        // SAFETY: the region's first page is committed, aligned for any
        // header (a page is), and nothing else uses it. Moving the region
        // value into the memory it maps keeps the mapping where it is.
        unsafe {
            header.write(Header {
                region,
                size: layout.size(),
                previous: None,
                next,
            });
        }
        if let Some(next) = next {
            // SAFETY: `next` heads this list, so its region is live.
            unsafe { Self::entry(next) }.previous = Some(header);
        }
        self.first = Some(header);
        self.bytes += layout.size();
        // SAFETY: the header was written above.
        let block = unsafe { Self::block(header) };
        Ok(NonNull::slice_from_raw_parts(block, layout.size()))
    }

    /// Grows or shrinks the large block at `ptr` to `size` bytes where it
    /// is, committing further into its region when it grows.
    ///
    /// # Safety
    ///
    /// `ptr` must be a live block this list allocated.
    pub(super) unsafe fn resize(
        &mut self,
        ptr: NonNull<u8>,
        size: usize,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: the caller passes a live block of this list.
        let header = unsafe { Self::header(ptr) };
        // SAFETY: the block is live, so its header is.
        let entry = unsafe { Self::entry(header) };
        let end = BLOCK_OFFSET.checked_add(size).ok_or(AllocError)?;
        entry.region.ensure_committed(end).map_err(|_| AllocError)?;
        self.bytes = self.bytes - entry.size + size;
        entry.size = size;
        // SAFETY: the header is live; the base is read after the commit, so
        // the returned pointer reaches the whole block.
        let block = unsafe { Self::block(header) };
        Ok(NonNull::slice_from_raw_parts(block, size))
    }

    /// Releases the large block at `ptr` and its region.
    ///
    /// # Safety
    ///
    /// `ptr` must be a live block this list allocated, and nothing may use
    /// the block afterwards.
    pub(super) unsafe fn release(&mut self, ptr: NonNull<u8>) {
        // SAFETY: the caller passes a live block of this list.
        let header = unsafe { Self::header(ptr) };
        // SAFETY: the block is live, so its header is.
        let Header {
            previous,
            next,
            size,
            ..
        } = *unsafe { Self::entry(header) };
        match previous {
            // SAFETY: the neighbors of a live entry are live entries.
            | Some(previous) => unsafe { Self::entry(previous) }.next = next,
            | None => self.first = next,
        }
        if let Some(next) = next {
            // SAFETY: as for `previous`.
            unsafe { Self::entry(next) }.previous = previous;
        }
        self.bytes -= size;
        // SAFETY: the entry is unlinked, and the caller uses its block no
        // more.
        unsafe {
            Self::unmap(header);
        }
    }

    /// Releases every large block.
    pub(super) fn release_all(&mut self) {
        while let Some(header) = self.first {
            // SAFETY: `header` heads the list, so it is live.
            self.first = unsafe { Self::entry(header) }.next;
            // SAFETY: the entry is unlinked, and releasing every block ends
            // their use.
            unsafe {
                Self::unmap(header);
            }
        }
        self.bytes = 0;
    }

    /// The bytes the live blocks hold.
    pub(super) const fn bytes(&self) -> usize {
        self.bytes
    }

    pub(super) const fn new() -> Self {
        Self {
            first: None,
            bytes: 0,
        }
    }

    /// The header at `header`, for as long as the caller holds no other
    /// reference to it.
    ///
    /// # Safety
    ///
    /// `header` must be live, and no other reference to it may be in use.
    const unsafe fn entry<'a>(mut header: NonNull<Header>) -> &'a mut Header {
        // SAFETY: the caller guarantees the header is live and unshared.
        unsafe { header.as_mut() }
    }

    /// The block of the region `header` starts.
    ///
    /// # Safety
    ///
    /// `header` must be live.
    unsafe fn block(header: NonNull<Header>) -> NonNull<u8> {
        // SAFETY: the caller guarantees the header is live.
        let base = unsafe { Self::entry(header) }.region.as_ptr();
        // SAFETY: the block lies `BLOCK_OFFSET` bytes into the committed
        // region.
        unsafe { base.add(BLOCK_OFFSET) }
    }

    /// The header of the region whose block starts at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` must be a live block of a list.
    const unsafe fn header(ptr: NonNull<u8>) -> NonNull<Header> {
        // SAFETY: a block starts `BLOCK_OFFSET` bytes past its header, in
        // the same region, and the pointer derives from the region's base.
        unsafe { ptr.sub(BLOCK_OFFSET) }.cast()
    }

    /// Releases the region `header` starts.
    ///
    /// # Safety
    ///
    /// `header` must be live and unlinked, and nothing may use its block
    /// afterwards.
    unsafe fn unmap(header: NonNull<Header>) {
        // SAFETY: the caller guarantees the header is live.
        let entry = unsafe { Self::entry(header) };
        // SAFETY: the region value is moved out before it unmaps the memory
        // it was read from, and the header is not read again.
        let region = unsafe { ptr::read(&raw const entry.region) };
        drop(region);
    }
}

impl Drop for LargeBlocks {
    fn drop(&mut self) {
        self.release_all();
    }
}

// Tests
#[cfg(test)]
mod tests {
    use std::alloc::Layout;

    use allocator_api2::alloc::Allocator;

    use super::{
        LARGE_BLOCK_BYTES as LARGE,
        LargeBlocks,
    };
    use crate::util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        vm::accounting,
    };

    #[test]
    #[cfg_attr(
        any(miri, target_family = "wasm", feature = "parser-study-small-regions"),
        ignore = "blocks have no regions of their own here"
    )]
    fn a_large_vector_grows_in_place_without_leaving_copies() {
        assert!(LargeBlocks::wants(Layout::array::<u64>(LARGE / 8).unwrap()));
        let arena = Bump::new();
        let mut values = ArenaVec::new_in(&arena);
        // Another allocation after each growth keeps the vector from being
        // the arena's latest block.
        let mut others = 0_usize;
        for value in 0..(4 * LARGE / size_of::<u64>()) as u64 {
            if values.len() == values.capacity() {
                _ = arena.alloc(value);
                others += 1;
            }
            values.push(value);
        }
        let capacity = values.capacity() * size_of::<u64>();
        assert!(values.iter().copied().eq(0..values.len() as u64));
        // Below the threshold, outgrown copies stay in the arena; from it on,
        // the vector grows where it is.
        assert!(
            arena.high_water() <= capacity + 2 * LARGE + others * 16,
            "high water {} for a capacity of {capacity}",
            arena.high_water()
        );
    }

    #[test]
    #[cfg_attr(
        any(miri, target_family = "wasm", feature = "parser-study-small-regions"),
        ignore = "blocks have no regions of their own here"
    )]
    fn deallocating_a_large_block_releases_its_region() {
        let arena = Bump::new();
        let before = accounting::live();
        let layout = Layout::from_size_align(LARGE, 64).unwrap();
        let block = arena.allocate(layout).unwrap().cast::<u8>();
        assert_eq!(accounting::live().regions, before.regions + 1);
        assert!((block.as_ptr() as usize).is_multiple_of(64));
        // SAFETY: the block was just allocated with this layout.
        unsafe {
            block.as_ptr().write_bytes(0xA5, layout.size());
        }
        // SAFETY: the block is live and not used again.
        unsafe {
            arena.deallocate(block, layout);
        }
        assert_eq!(accounting::live().regions, before.regions);
    }

    #[test]
    #[cfg_attr(
        any(miri, target_family = "wasm", feature = "parser-study-small-regions"),
        ignore = "blocks have no regions of their own here"
    )]
    fn large_blocks_survive_growth_and_shrinking_and_go_with_reset() {
        let mut arena = Bump::new();
        let before = accounting::live();
        let small = arena.alloc(7_u32);
        let mut table = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, &arena);
        for key in 0..(LARGE / 8) {
            _ = table.insert(key, key * 3);
        }
        assert_eq!(*small, 7);
        assert!((0..LARGE / 8).all(|key| table[&key] == key * 3));
        // Each resize released the outgrown table's region.
        assert_eq!(accounting::live().regions, before.regions + 2);
        table.shrink_to_fit();
        assert!((0..LARGE / 8).all(|key| table[&key] == key * 3));
        drop(table);
        assert_eq!(accounting::live().regions, before.regions + 1);
        let leaked = ArenaVec::<u8>::with_capacity_in(LARGE, &arena).leak();
        assert_eq!(leaked, [0_u8; 0]);
        assert_eq!(accounting::live().regions, before.regions + 2);
        arena.reset();
        assert_eq!(accounting::live().regions, before.regions + 1);
    }
}
