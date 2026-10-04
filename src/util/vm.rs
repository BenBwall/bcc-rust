//! Reserved virtual-address regions for arenas and growable collections.
//!
//! A region owns one fixed address range. Reserving it consumes address space
//! but does not make it writable. Callers commit page-aligned subranges before
//! writing and must not retain references across decommit or release.
//!
//! Tests can make reservations and commits fail ([`faults`]). Under Miri,
//! the OS is modelled by one allocation whose uncommitted bytes are
//! unreachable through the pointers a [`GrowingRegion`] hands out.

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
        #[cfg(test)]
        faults::reserve()?;
        let base = os_reserve(reserved)?;
        #[cfg(any(test, feature = "benchmarking-internals"))]
        accounting::update(|usage| {
            usage.regions += 1;
            usage.reserved += reserved;
        });
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
        let range = self.checked_range(offset, bytes)?;
        #[cfg(test)]
        faults::commit(bytes)?;
        os_commit(range, bytes)
    }

    /// Return physical storage and revoke access to a committed page range.
    ///
    /// # Safety
    ///
    /// No Rust reference, slice, or live allocation may overlap this range.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No arena returns committed pages before release yet."
        )
    )]
    pub(crate) unsafe fn decommit(&self, offset: usize, bytes: usize) -> io::Result<()> {
        os_decommit(self.checked_range(offset, bytes)?, bytes)
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "benchmarking-internals"))]
        accounting::update(|usage| {
            usage.regions -= 1;
            usage.reserved -= self.reserved;
        });
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
    /// Under Miri, the base as seen through the committed prefix only, so an
    /// access past it is reported as undefined behavior, as the inaccessible
    /// pages would fault natively.
    #[cfg(miri)]
    view:      NonNull<u8>,
}

impl GrowingRegion {
    pub(crate) fn reserve(bytes: usize) -> io::Result<Self> {
        let region = Region::reserve(bytes)?;
        Ok(Self {
            #[cfg(miri)]
            view: committed_view(region.as_ptr(), 0),
            region,
            committed: 0,
        })
    }

    /// The region's base. Memory is reachable through it only up to the
    /// committed prefix at the time of the call, so derive pointers into newly
    /// committed memory after [`Self::ensure_committed`] succeeds.
    pub(crate) fn as_ptr(&self) -> NonNull<u8> {
        #[cfg(miri)]
        return self.view;
        #[cfg(not(miri))]
        self.region.as_ptr()
    }

    /// The bytes committed from the region's start.
    pub(crate) fn committed(&self) -> usize {
        self.committed
    }

    /// Commits at least the first `needed` bytes, plus at most one commit
    /// step beyond them, so commit follows what callers write rather than
    /// what they might write. If the step cannot be committed, only the pages
    /// `needed` reaches are tried before reporting failure.
    pub(crate) fn ensure_committed(&mut self, needed: usize) -> io::Result<()> {
        let page = self.region.page;
        let target = next_commit(self.committed, needed, self.region.reserved(), page)
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "region exhausted"))?;
        if target == self.committed {
            return Ok(());
        }
        let target = match self.region.commit(self.committed, target - self.committed) {
            | Ok(()) => target,
            | Err(error) => {
                // `needed` lies inside the page-multiple reservation, so its
                // rounded end does too.
                let minimal = round_up(needed, page).filter(|&minimal| minimal < target);
                let Some(minimal) = minimal else {
                    return Err(error);
                };
                self.region
                    .commit(self.committed, minimal - self.committed)?;
                minimal
            },
        };
        #[cfg(any(test, feature = "benchmarking-internals"))]
        accounting::update(|usage| usage.committed += target - self.committed);
        self.committed = target;
        #[cfg(miri)]
        {
            self.view = committed_view(self.region.as_ptr(), self.committed);
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "benchmarking-internals"))]
impl Drop for GrowingRegion {
    fn drop(&mut self) {
        accounting::update(|usage| usage.committed -= self.committed);
    }
}

/// `base`, reachable for exactly `committed` bytes. Every view derives from
/// the reservation's own pointer with shared read-write permission, so
/// pointers from earlier, shorter views stay valid for the bytes they cover.
#[cfg(miri)]
fn committed_view(base: NonNull<u8>, committed: usize) -> NonNull<u8> {
    let cells = std::ptr::slice_from_raw_parts(
        base.as_ptr()
            .cast_const()
            .cast::<std::cell::UnsafeCell<u8>>(),
        committed,
    );
    // SAFETY: the mock reservation is one live allocation of at least
    // `committed` bytes. A shared reference to `UnsafeCell` bytes neither
    // reads them nor invalidates other pointers, and the raw pointer taken
    // from it may read and write exactly that range.
    NonNull::from(unsafe { &*cells }).cast()
}

/// Every region's reservation: 100 GiB of address space, which costs no
/// commit charge until written. A few dozen of them fit easily in a 64-bit
/// address space. Miri models a reservation as one real allocation, so it
/// gets a small one.
#[cfg(not(miri))]
pub(crate) const REGION_BYTES: usize = 100 * 1024 * 1024 * 1024;
#[cfg(miri)]
pub(crate) const REGION_BYTES: usize = 16 * 1024 * 1024;

/// The first commit step: small, so a compilation touching a dozen regions
/// commits under a mebibyte. Steps double from here up to
/// [`MAX_COMMIT_STEP`].
const FIRST_COMMIT_STEP: usize = 64 * 1024;

/// The most a region commits beyond what it was asked for. Commit follows
/// the write position, so this bounds each region's untouched but committed
/// tail. At 1 MiB (256 pages of 4 KiB) one commit call is amortized over
/// hundreds of first-touch page faults, which cost far more, and the
/// ten-odd regions of a compilation together leave at most about 10 MiB
/// committed ahead of use. Doubling without a cap left up to half of a large
/// region committed but never written.
pub(crate) const MAX_COMMIT_STEP: usize = 1024 * 1024;

/// Pure commit-charge bookkeeping, also exercised under Miri without OS calls.
fn next_commit(committed: usize, needed: usize, reserved: usize, page: usize) -> Option<usize> {
    if needed > reserved {
        return None;
    }
    if needed <= committed {
        return Some(committed);
    }
    let step = committed
        .clamp(FIRST_COMMIT_STEP, MAX_COMMIT_STEP)
        .min(reserved - committed);
    let target = committed.checked_add(step)?.max(needed);
    round_up(target, page).filter(|&rounded| rounded <= reserved)
}

/// Per-thread totals of live regions, for tests and benchmarks. Regions are
/// neither `Send` nor `Sync`, so each one is counted on the thread that owns
/// it.
#[cfg(any(test, feature = "benchmarking-internals"))]
#[expect(
    clippy::missing_const_for_thread_local,
    reason = "The initializers are const blocks; Clippy misreads their expansion."
)]
pub(crate) mod accounting {
    use std::cell::Cell;

    /// Live regions, their reserved address space, and their committed bytes.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Usage {
        pub(crate) regions:   usize,
        pub(crate) reserved:  usize,
        pub(crate) committed: usize,
    }

    const NONE: Usage = Usage {
        regions:   0,
        reserved:  0,
        committed: 0,
    };

    thread_local! {
        static LIVE: Cell<Usage> = const { Cell::new(NONE) };
        static PEAK: Cell<Usage> = const { Cell::new(NONE) };
    }

    pub(super) fn update(change: impl FnOnce(&mut Usage)) {
        let mut live = LIVE.get();
        change(&mut live);
        LIVE.set(live);
        let peak = PEAK.get();
        PEAK.set(Usage {
            regions:   peak.regions.max(live.regions),
            reserved:  peak.reserved.max(live.reserved),
            committed: peak.committed.max(live.committed),
        });
    }

    /// What this thread holds now.
    pub(crate) fn live() -> Usage {
        LIVE.get()
    }

    /// Each total's largest value on this thread since the last reset.
    pub(crate) fn peak() -> Usage {
        PEAK.get()
    }

    /// Starts a new peak measurement from what this thread holds now.
    pub(crate) fn reset_peak() {
        PEAK.set(LIVE.get());
    }
}

/// Injected OS failures for tests, per thread like the regions they affect.
#[cfg(test)]
#[expect(
    clippy::missing_const_for_thread_local,
    reason = "The initializers are const blocks; Clippy misreads their expansion."
)]
pub(crate) mod faults {
    use std::{
        cell::Cell,
        io,
    };

    #[derive(Clone, Copy, Default)]
    struct Plan {
        reserves:       usize,
        commits:        usize,
        commit_at_most: Option<usize>,
    }

    const NO_FAULTS: Plan = Plan {
        reserves:       0,
        commits:        0,
        commit_at_most: None,
    };

    thread_local! {
        static PLAN: Cell<Plan> = const { Cell::new(NO_FAULTS) };
    }

    /// Removes every injected failure when dropped.
    pub(crate) struct Injected(());

    impl Drop for Injected {
        fn drop(&mut self) {
            PLAN.set(Plan::default());
        }
    }

    fn inject(change: impl FnOnce(&mut Plan)) -> Injected {
        let mut plan = PLAN.get();
        change(&mut plan);
        PLAN.set(plan);
        Injected(())
    }

    /// The next `count` reservations on this thread fail.
    pub(crate) fn fail_reserves(count: usize) -> Injected {
        inject(|plan| plan.reserves = count)
    }

    /// The next `count` commits on this thread fail.
    pub(crate) fn fail_commits(count: usize) -> Injected {
        inject(|plan| plan.commits = count)
    }

    /// Commits of more than `bytes` at once fail on this thread.
    pub(crate) fn limit_commits(bytes: usize) -> Injected {
        inject(|plan| plan.commit_at_most = Some(bytes))
    }

    fn injected() -> io::Error {
        io::Error::new(io::ErrorKind::OutOfMemory, "injected failure")
    }

    pub(super) fn reserve() -> io::Result<()> {
        let mut plan = PLAN.get();
        if plan.reserves == 0 {
            return Ok(());
        }
        plan.reserves -= 1;
        PLAN.set(plan);
        Err(injected())
    }

    pub(super) fn commit(bytes: usize) -> io::Result<()> {
        let mut plan = PLAN.get();
        if plan.commit_at_most.is_some_and(|limit| bytes > limit) {
            return Err(injected());
        }
        if plan.commits == 0 {
            return Ok(());
        }
        plan.commits -= 1;
        PLAN.set(plan);
        Err(injected())
    }
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
    // SAFETY: layout is nonzero and valid. The whole mock reservation is one
    // allocation; GrowingRegion hands out only views of its committed prefix.
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
        assert_eq!(next_commit(0, 1, reserved, 4096), Some(FIRST_COMMIT_STEP));
        assert_eq!(
            next_commit(FIRST_COMMIT_STEP, 1, reserved, 4096),
            Some(FIRST_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(FIRST_COMMIT_STEP, FIRST_COMMIT_STEP + 1, reserved, 4096),
            Some(2 * FIRST_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(4 * 1024 * 1024, 9 * 1024 * 1024, reserved, 4096),
            Some(9 * 1024 * 1024)
        );
        // Doubling stops at the largest step: a large region commits one
        // step ahead of what it is asked for, not twice its size.
        assert_eq!(
            next_commit(MAX_COMMIT_STEP / 2, MAX_COMMIT_STEP / 2 + 1, reserved, 4096),
            Some(MAX_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(
                64 * MAX_COMMIT_STEP,
                64 * MAX_COMMIT_STEP + 1,
                reserved,
                4096
            ),
            Some(65 * MAX_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(
                64 * MAX_COMMIT_STEP,
                70 * MAX_COMMIT_STEP + 1,
                reserved,
                4096
            ),
            Some(70 * MAX_COMMIT_STEP + 4096)
        );
        assert_eq!(
            next_commit(reserved - 4096, reserved, reserved, 4096),
            Some(reserved)
        );
        assert_eq!(next_commit(reserved, reserved + 1, reserved, 4096), None);
    }

    #[test]
    fn failed_commit_step_falls_back_to_the_pages_needed() {
        let mut region = GrowingRegion::reserve(16 * 1024 * 1024).unwrap();
        let page = region.region.page;
        region.ensure_committed(FIRST_COMMIT_STEP).unwrap();
        assert_eq!(region.committed(), FIRST_COMMIT_STEP);
        // The geometric step would commit another FIRST_COMMIT_STEP bytes.
        let limited = faults::limit_commits(page);
        region.ensure_committed(FIRST_COMMIT_STEP + 1).unwrap();
        assert_eq!(region.committed(), FIRST_COMMIT_STEP + page);
        // Nothing smaller than the needed pages is tried.
        let error = region
            .ensure_committed(FIRST_COMMIT_STEP + 3 * page)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(region.committed(), FIRST_COMMIT_STEP + page);
        drop(limited);
        region
            .ensure_committed(FIRST_COMMIT_STEP + 3 * page)
            .unwrap();
        assert_eq!(region.committed(), 2 * (FIRST_COMMIT_STEP + page));
    }

    #[test]
    fn failed_commits_and_reservations_leave_the_region_usable() {
        {
            let _failing = faults::fail_reserves(1);
            assert!(GrowingRegion::reserve(1024 * 1024).is_err());
        }
        let mut region = GrowingRegion::reserve(1024 * 1024).unwrap();
        {
            let _failing = faults::fail_commits(2);
            assert_eq!(
                region.ensure_committed(1).unwrap_err().kind(),
                io::ErrorKind::OutOfMemory
            );
            assert_eq!(region.committed(), 0);
        }
        region.ensure_committed(1).unwrap();
        assert_eq!(region.committed(), FIRST_COMMIT_STEP);
        assert_eq!(
            region.ensure_committed(1024 * 1024 + 1).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert_eq!(region.committed(), FIRST_COMMIT_STEP);
    }

    #[test]
    fn accounting_follows_reservations_and_commits() {
        let before = accounting::live();
        accounting::reset_peak();
        let mut region = GrowingRegion::reserve(1024 * 1024).unwrap();
        region.ensure_committed(1).unwrap();
        let during = accounting::live();
        assert_eq!(during.regions, before.regions + 1);
        assert_eq!(during.reserved, before.reserved + 1024 * 1024);
        assert_eq!(during.committed, before.committed + FIRST_COMMIT_STEP);
        drop(region);
        assert_eq!(accounting::live(), before);
        assert_eq!(accounting::peak(), during);
    }

    #[test]
    fn pages_can_be_committed_decommitted_and_recommitted_at_fixed_addresses() {
        let page = page_size().unwrap();
        let region = Region::reserve(4 * page).unwrap();
        assert_eq!(region.reserved(), 4 * page);
        assert_eq!(
            region.commit(1, page).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            region.commit(0, 5 * page).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
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
