//! Reserved virtual-address regions for arenas and growable collections.
//!
//! A region owns one fixed address range. Reserving it consumes address space
//! but does not make it writable. Callers commit page-aligned subranges before
//! writing and must not retain references across decommit or release.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "The stage 4b allocator will use this OS layer.")
)]

use std::{
    io,
    ptr::NonNull,
};

/// One OS reservation, released when dropped.
pub(crate) struct Region {
    base:     NonNull<u8>,
    reserved: usize,
    page:     usize,
    #[cfg(miri)]
    layout:   std::alloc::Layout,
}

impl Region {
    pub(crate) fn reserve(bytes: usize) -> io::Result<Self> {
        let page = page_size()?;
        let reserved = round_up(bytes, page)
            .filter(|&size| size != 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid region size"))?;
        let base = os_reserve(reserved)?;
        Ok(Self {
            base,
            reserved,
            page,
            #[cfg(miri)]
            layout: std::alloc::Layout::from_size_align(reserved, page)
                .expect("valid mock region layout"),
        })
    }

    pub(crate) fn as_ptr(&self) -> NonNull<u8> {
        self.base
    }

    pub(crate) fn reserved(&self) -> usize {
        self.reserved
    }

    fn checked_range(&self, offset: usize, bytes: usize) -> io::Result<*mut u8> {
        if bytes == 0
            || !offset.is_multiple_of(self.page)
            || !bytes.is_multiple_of(self.page)
            || offset
                .checked_add(bytes)
                .is_none_or(|end| end > self.reserved)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid page range",
            ));
        }
        // The range was checked inside this reservation. `wrapping_add`
        // preserves the OS pointer's provenance without dereferencing it.
        Ok(self.base.as_ptr().wrapping_add(offset))
    }

    /// Make a reserved page range writable. It is the caller's responsibility
    /// to commit before creating any Rust reference into the range.
    pub(crate) fn commit(&self, offset: usize, bytes: usize) -> io::Result<()> {
        os_commit(self.checked_range(offset, bytes)?, bytes)
    }

    /// Return physical storage and revoke access to a committed page range.
    ///
    /// # Safety
    ///
    /// No Rust reference, slice, or live allocation may overlap this range.
    pub(crate) unsafe fn decommit(&self, offset: usize, bytes: usize) -> io::Result<()> {
        os_decommit(self.checked_range(offset, bytes)?, bytes)
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: this is the original reservation base and size, released
        // exactly once. Users of the region cannot outlive its owner.
        unsafe {
            #[cfg(not(miri))]
            os_release(self.base, self.reserved);
            #[cfg(miri)]
            std::alloc::dealloc(self.base.as_ptr(), self.layout);
        }
    }
}

/// A fixed reservation whose writable prefix grows in page-aligned steps.
/// The untouched tail remains inaccessible on native platforms.
pub(crate) struct GrowingRegion {
    region:    Region,
    committed: usize,
}

impl GrowingRegion {
    pub(crate) fn reserve(bytes: usize) -> io::Result<Self> {
        Ok(Self {
            region:    Region::reserve(bytes)?,
            committed: 0,
        })
    }

    pub(crate) fn as_ptr(&self) -> NonNull<u8> {
        self.region.as_ptr()
    }

    #[cfg(test)]
    pub(crate) fn committed(&self) -> usize {
        self.committed
    }

    pub(crate) fn ensure_committed(&mut self, needed: usize) -> io::Result<()> {
        let target = next_commit(
            self.committed,
            needed,
            self.region.reserved(),
            self.region.page,
        )
        .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "region exhausted"))?;
        if target > self.committed {
            self.region
                .commit(self.committed, target - self.committed)?;
            self.committed = target;
        }
        Ok(())
    }
}

/// Pure commit-charge bookkeeping, also exercised under Miri without OS calls.
fn next_commit(committed: usize, needed: usize, reserved: usize, page: usize) -> Option<usize> {
    if needed > reserved {
        return None;
    }
    if needed <= committed {
        return Some(committed);
    }
    let floor = 4 * 1024 * 1024;
    let step = committed.max(floor).min(reserved - committed);
    let target = committed.checked_add(step)?.max(needed);
    round_up(target, page).filter(|&rounded| rounded <= reserved)
}

fn round_up(bytes: usize, page: usize) -> Option<usize> {
    bytes.checked_add(page - 1).map(|value| value / page * page)
}

#[cfg(miri)]
fn page_size() -> io::Result<usize> {
    Ok(4096)
}

#[cfg(all(windows, not(miri)))]
fn page_size() -> io::Result<usize> {
    use windows_sys::Win32::System::SystemInformation::{
        GetSystemInfo,
        SYSTEM_INFO,
    };
    let mut info = std::mem::MaybeUninit::<SYSTEM_INFO>::uninit();
    // SAFETY: GetSystemInfo initializes the SYSTEM_INFO output buffer.
    unsafe {
        GetSystemInfo(info.as_mut_ptr());
    }
    // SAFETY: GetSystemInfo has no failure result and initialized the buffer.
    let info = unsafe { info.assume_init() };
    std::num::NonZeroUsize::new(info.dwPageSize as usize)
        .map(std::num::NonZeroUsize::get)
        .ok_or_else(|| io::Error::other("GetSystemInfo returned a zero page size"))
}

#[cfg(all(unix, not(miri)))]
fn page_size() -> io::Result<usize> {
    // SAFETY: sysconf has no pointer arguments; _SC_PAGESIZE is a valid name.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(size)
        .ok()
        .and_then(std::num::NonZeroUsize::new)
        .map(std::num::NonZeroUsize::get)
        .ok_or_else(io::Error::last_os_error)
}

#[cfg(miri)]
fn os_reserve(bytes: usize) -> io::Result<NonNull<u8>> {
    let layout = std::alloc::Layout::from_size_align(bytes, page_size()?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid mock region layout"))?;
    // SAFETY: layout is nonzero and valid. The mock keeps all bytes accessible;
    // commit boundaries are checked by GrowingRegion before use.
    NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) })
        .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "mock region allocation"))
}

#[cfg(all(windows, not(miri)))]
fn os_reserve(bytes: usize) -> io::Result<NonNull<u8>> {
    use windows_sys::Win32::System::Memory::{
        MEM_RESERVE,
        PAGE_NOACCESS,
        VirtualAlloc,
    };
    // SAFETY: null requests a new reservation; size is nonzero and rounded
    // to a page. PAGE_NOACCESS leaves every page inaccessible.
    let ptr = unsafe { VirtualAlloc(std::ptr::null(), bytes, MEM_RESERVE, PAGE_NOACCESS) };
    NonNull::new(ptr.cast()).ok_or_else(io::Error::last_os_error)
}

#[cfg(all(unix, not(miri)))]
fn os_reserve(bytes: usize) -> io::Result<NonNull<u8>> {
    #[cfg(target_os = "linux")]
    const NO_RESERVE: i32 = libc::MAP_NORESERVE;
    #[cfg(not(target_os = "linux"))]
    const NO_RESERVE: i32 = 0;
    // SAFETY: null requests a new anonymous private mapping. PROT_NONE
    // reserves its address range without exposing readable or writable pages.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            bytes,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | NO_RESERVE,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful mmap returns a non-null mapped address.
    Ok(unsafe { NonNull::new_unchecked(ptr.cast()) })
}

#[cfg(miri)]
fn os_commit(_: *mut u8, _: usize) -> io::Result<()> {
    Ok(())
}

#[cfg(all(windows, not(miri)))]
fn os_commit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT,
        PAGE_READWRITE,
        VirtualAlloc,
    };
    // SAFETY: ptr and bytes cover page-aligned memory inside our reservation;
    // MEM_COMMIT makes that exact range writable without moving it.
    let result = unsafe { VirtualAlloc(ptr.cast(), bytes, MEM_COMMIT, PAGE_READWRITE) };
    if result.is_null() {
        Err(io::Error::last_os_error())
    } else {
        debug_assert_eq!(result.cast::<u8>(), ptr, "committed range moved");
        Ok(())
    }
}

#[cfg(all(unix, not(miri)))]
fn os_commit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    // SAFETY: ptr and bytes cover page-aligned memory inside our mapping.
    let result = unsafe { libc::mprotect(ptr.cast(), bytes, libc::PROT_READ | libc::PROT_WRITE) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(miri)]
fn os_decommit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    // SAFETY: the caller has ended all allocations in the range. The mock
    // zeroes it to model decommit/recommit without changing pointer identity.
    unsafe { ptr.write_bytes(0, bytes) };
    Ok(())
}

#[cfg(all(windows, not(miri)))]
fn os_decommit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    use windows_sys::Win32::System::Memory::{
        MEM_DECOMMIT,
        VirtualFree,
    };
    // SAFETY: ptr and bytes cover page-aligned committed memory whose Rust
    // references have all ended, so revoking access is valid.
    let result = unsafe { VirtualFree(ptr.cast(), bytes, MEM_DECOMMIT) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(unix, not(miri)))]
fn os_decommit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    // SAFETY: ptr and bytes cover page-aligned mapped memory with no live
    // Rust references; MADV_DONTNEED discards the physical pages.
    let advised = unsafe { libc::madvise(ptr.cast(), bytes, libc::MADV_DONTNEED) };
    if advised != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the same mapping remains owned by the region, and no reference
    // overlaps it. PROT_NONE prevents access until the next commit.
    let protected = unsafe { libc::mprotect(ptr.cast(), bytes, libc::PROT_NONE) };
    if protected == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(windows, not(miri)))]
unsafe fn os_release(base: NonNull<u8>, _: usize) {
    use windows_sys::Win32::System::Memory::{
        MEM_RELEASE,
        VirtualFree,
    };
    // SAFETY: base is the exact original reservation and size must be zero
    // for MEM_RELEASE; no caller can use this region after Drop begins.
    let released = unsafe { VirtualFree(base.as_ptr().cast(), 0, MEM_RELEASE) };
    debug_assert_ne!(released, 0, "VirtualFree(MEM_RELEASE) failed");
}

#[cfg(all(unix, not(miri)))]
unsafe fn os_release(base: NonNull<u8>, bytes: usize) {
    // SAFETY: base and bytes are the original mmap result, unmapped once.
    let released = unsafe { libc::munmap(base.as_ptr().cast(), bytes) };
    debug_assert_eq!(released, 0, "munmap failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_rejects_overflow_and_preserves_aligned_sizes() {
        assert_eq!(round_up(1, 4096), Some(4096));
        assert_eq!(round_up(4096, 4096), Some(4096));
        assert_eq!(round_up(usize::MAX, 4096), None);
    }

    #[test]
    fn commit_growth_stays_page_aligned_and_within_reservation() {
        let gib = 1024 * 1024 * 1024;
        let reserved = 100 * gib;
        assert_eq!(next_commit(0, 1, reserved, 4096), Some(4 * 1024 * 1024));
        assert_eq!(
            next_commit(4 * 1024 * 1024, 1, reserved, 4096),
            Some(4 * 1024 * 1024)
        );
        assert_eq!(
            next_commit(4 * 1024 * 1024, 9 * 1024 * 1024, reserved, 4096),
            Some(9 * 1024 * 1024)
        );
        assert_eq!(
            next_commit(reserved - 4096, reserved, reserved, 4096),
            Some(reserved)
        );
        assert_eq!(next_commit(reserved, reserved + 1, reserved, 4096), None);
    }

    #[cfg(not(miri))]
    #[test]
    fn pages_can_be_committed_decommitted_and_recommitted_at_fixed_addresses() {
        let page = page_size().unwrap();
        let region = Region::reserve(4 * page).unwrap();
        assert_eq!(region.reserved(), 4 * page);
        assert!(region.commit(1, page).is_err());
        assert!(region.commit(0, 5 * page).is_err());
        region.commit(0, page).unwrap();
        let first = region.as_ptr().as_ptr();
        // SAFETY: the first page is committed, and only this test accesses it.
        unsafe {
            first.write_volatile(42);
        }
        region.commit(page, page).unwrap();
        let second = first.wrapping_add(page);
        // SAFETY: the second page was committed and is still owned here.
        unsafe {
            second.write_volatile(99);
        }
        // SAFETY: no reference or allocation remains in the second page.
        unsafe {
            region.decommit(page, page).unwrap();
        }
        region.commit(page, page).unwrap();
        // SAFETY: recommit made the second page readable and zero initialized.
        assert_eq!(unsafe { second.read_volatile() }, 0);
        // SAFETY: the first page was not decommitted and remains readable.
        assert_eq!(unsafe { first.read_volatile() }, 42);
    }

    #[cfg(all(not(miri), target_pointer_width = "64"))]
    #[test]
    fn hundred_gib_reservation_only_commits_touched_pages() {
        let region = Region::reserve(100 * 1024 * 1024 * 1024).unwrap();
        assert_eq!(region.reserved(), 100 * 1024 * 1024 * 1024);
        region.commit(0, page_size().unwrap()).unwrap();
        // SAFETY: the first page was committed, and no other reference exists.
        unsafe {
            region.as_ptr().as_ptr().write_volatile(7);
        }
    }
}
