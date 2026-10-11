//! Reserved virtual-address regions for arenas and growable collections.
//!
//! A region owns one fixed address range. Reserving it consumes address space
//! but does not make it writable. Callers commit page-aligned subranges before
//! writing. Decommit invalidates pointers into that range; release invalidates
//! every allocation and raw pointer obtained from the region.
//!
//! On Linux a region of at least one huge page starts on a
//! transparent-huge-page boundary, is advised as huge pages
//! (`MADV_HUGEPAGE`), and commits whole, aligned [`HUGE_PAGE`]s, so the
//! kernel can back each 2 MiB with one page and one fault instead of 512.
//! Other platforms commit ordinary pages in smaller steps.
//!
//! Tests can make reservations and commits fail ([`faults`]). Under Miri,
//! the OS is modelled by one allocation whose uncommitted bytes are
//! unreachable through the pointers a [`GrowingRegion`] hands out. Its bytes
//! start uninitialized rather than zeroed, so Miri also reports any read of
//! memory nothing has written: arena memory is reused after a reset, and its
//! users must not count on fresh pages reading as zero.

use std::{
    io,
    ptr::NonNull,
};

/// A fixed reservation whose writable prefix grows in page-aligned steps.
/// The untouched tail remains inaccessible on native platforms.
pub(crate) struct GrowingRegion {
    region:     Region,
    committed:  usize,
    /// Whether commits cover whole huge pages.
    huge_pages: bool,
    /// Under Miri, the base as seen through the committed prefix only, so an
    /// access past it is reported as undefined behavior, as the inaccessible
    /// pages would fault natively.
    #[cfg(miri)]
    view:       NonNull<u8>,
}

impl GrowingRegion {
    /// Commits at least the first `needed` bytes, plus at most one commit
    /// step beyond them, so commit follows what callers write rather than
    /// what they might write. On a huge-page region the step ends on the next
    /// huge-page boundary, less than one huge page beyond `needed`. If the
    /// step cannot be committed, only the pages `needed` reaches are tried
    /// before reporting failure.
    pub(crate) fn ensure_committed(&mut self, needed: usize) -> io::Result<()> {
        let page = self.region.page;
        let target = next_commit(
            self.committed,
            needed,
            self.region.reserved(),
            page,
            self.huge_pages,
        )
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

    /// Reserves `bytes`. On Linux, a region of at least one huge page asks
    /// for transparent huge pages and commits whole ones; if the kernel
    /// refuses the advice, the region commits ordinary pages instead.
    pub(crate) fn reserve(bytes: usize) -> io::Result<Self> {
        let region = Region::reserve(bytes)?;
        #[cfg(all(target_os = "linux", not(miri)))]
        let huge_pages = region.advise_huge_pages();
        #[cfg(not(all(target_os = "linux", not(miri))))]
        let huge_pages = false;
        Ok(Self::with_huge_pages(region, huge_pages))
    }

    fn with_huge_pages(region: Region, huge_pages: bool) -> Self {
        Self {
            #[cfg(miri)]
            view: committed_view(region.as_ptr(), 0),
            region,
            committed: 0,
            huge_pages,
        }
    }

    /// A region whose commits cover whole huge pages, or not, on every
    /// platform and without asking the OS for huge pages, so tests can pin
    /// either commit pattern (under Miri too) wherever they run.
    #[cfg(test)]
    pub(crate) fn reserve_with_huge_pages(bytes: usize, huge_pages: bool) -> io::Result<Self> {
        Ok(Self::with_huge_pages(Region::reserve(bytes)?, huge_pages))
    }

    /// The bytes committed from the region's start.
    pub(crate) fn committed(&self) -> usize {
        self.committed
    }

    /// The region's base. Memory is reachable through it only up to the
    /// committed prefix at the time of the call, so derive pointers into newly
    /// committed memory after [`Self::ensure_committed`] succeeds.
    /// Dropping this region invalidates all allocations and raw pointers;
    /// using any of them afterwards is the caller's bug.
    pub(crate) fn as_ptr(&self) -> NonNull<u8> {
        #[cfg(miri)]
        return self.view;
        #[cfg(not(miri))]
        self.region.as_ptr()
    }
}

/// Pure commit-charge bookkeeping, also exercised under Miri without OS calls.
/// With `huge_pages`, the target is rounded up to a whole huge page.
fn next_commit(
    committed: usize,
    needed: usize,
    reserved: usize,
    page: usize,
    huge_pages: bool,
) -> Option<usize> {
    if needed > reserved {
        return None;
    }
    if needed <= committed {
        return Some(committed);
    }
    let step = committed
        .clamp(FIRST_COMMIT_STEP, MAX_COMMIT_STEP)
        .min(reserved - committed);
    let mut target = committed.checked_add(step)?.max(needed);
    if huge_pages {
        // `needed` fits the reservation, so capping the rounded end at the
        // reservation still covers it.
        target = round_up(target, HUGE_PAGE)?.min(reserved);
    }
    round_up(target, page).filter(|&rounded| rounded <= reserved)
}

/// One OS reservation, released when dropped. Release invalidates every
/// allocation and raw pointer into it; raw pointers carry no borrow, and
/// using them after release is the caller's bug.
pub(crate) struct Region {
    base:     NonNull<u8>,
    reserved: usize,
    page:     usize,
    #[cfg(miri)]
    layout:   std::alloc::Layout,
}

impl Region {
    /// Make a reserved page range writable. It is the caller's responsibility
    /// to commit before creating any Rust reference into the range.
    pub(crate) fn commit(&self, offset: usize, bytes: usize) -> io::Result<()> {
        let range = self.checked_range(offset, bytes)?;
        #[cfg(test)]
        faults::commit(bytes)?;
        os_commit(range, bytes)
    }

    /// Return physical storage and revoke access to a committed page range.
    /// A successful decommit discards its contents; recommit provides zeros.
    ///
    /// # Safety
    ///
    /// No Rust reference, slice, or live allocation may overlap this range.
    /// All raw pointers into it are invalidated on success and must not be
    /// used again, even after recommit; derive new pointers from the region.
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

    pub(crate) fn reserved(&self) -> usize {
        self.reserved
    }

    /// Advises the whole region as transparent huge pages, reporting whether
    /// the kernel accepted the advice. The region must span at least one huge
    /// page and start on a huge-page boundary; a page size other than 4 KiB,
    /// where a huge page may not be 2 MiB, gets no advice.
    #[cfg(all(target_os = "linux", not(miri)))]
    fn advise_huge_pages(&self) -> bool {
        if self.page != 4096
            || self.reserved < HUGE_PAGE
            || !(self.base.as_ptr() as usize).is_multiple_of(HUGE_PAGE)
        {
            return false;
        }
        // SAFETY: the range is this region's own mapping, and MADV_HUGEPAGE
        // only changes how the kernel backs its pages, not their contents or
        // protection.
        let advised = unsafe {
            libc::madvise(
                self.base.as_ptr().cast(),
                self.reserved,
                libc::MADV_HUGEPAGE,
            )
        };
        advised == 0
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

    /// The reservation's base. Commit before accessing a range, and end all
    /// references and allocations in it before decommit. Decommit invalidates
    /// pointers into that range, and dropping the region invalidates every
    /// allocation and raw pointer obtained from it. Raw pointers carry no
    /// borrow; using one after invalidation is the caller's bug.
    pub(crate) fn as_ptr(&self) -> NonNull<u8> {
        self.base
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
            .cast::<std::cell::UnsafeCell<std::mem::MaybeUninit<u8>>>(),
        committed,
    );
    // SAFETY: the mock reservation is one live allocation of at least
    // `committed` bytes, aligned for these one-byte cells. MaybeUninit permits
    // unwritten bytes; UnsafeCell permits mutation through existing allocation
    // pointers while this shared view is formed. The reference only bounds
    // Miri's read-write permission to the committed prefix; it does not read
    // or assert initialization of the bytes. Every view derives from `base`.
    NonNull::from(unsafe { &*cells }).cast()
}

/// Every region's reservation: 100 GiB of address space, which costs no
/// commit charge until written. This fits the full `u32`-indexed source-vector
/// arena (20 bytes times `u32::MAX`, just under 80 GiB), while a few dozen
/// regions still fit easily in a 64-bit address space. Miri models a
/// reservation as one real allocation, so it gets a small one.
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
/// region committed but never written. A huge-page region instead commits
/// up to the next [`HUGE_PAGE`] boundary, under one huge page beyond what
/// was asked for.
pub(crate) const MAX_COMMIT_STEP: usize = 1024 * 1024;

/// A transparent huge page: one page-table entry maps 2 MiB on x86-64 and
/// 64-bit Arm with 4 KiB pages. A Linux region commits whole, aligned huge
/// pages, because the kernel backs a range with one only when all 2 MiB of
/// it are mapped with the same protection. A huge page is backed in full
/// once any byte of it is touched, so on Linux every region in use holds at
/// least 2 MiB.
pub(crate) const HUGE_PAGE: usize = 2 * 1024 * 1024;

/// The most a region can commit beyond what it was asked for: one commit
/// step, or on Linux, where regions may commit whole huge pages, just under
/// one huge page.
#[cfg(all(test, target_os = "linux", not(miri)))]
pub(crate) const MAX_COMMIT_AHEAD: usize = HUGE_PAGE;

#[cfg(all(test, not(all(target_os = "linux", not(miri)))))]
pub(crate) const MAX_COMMIT_AHEAD: usize = MAX_COMMIT_STEP;

/// Per-thread totals of live regions, for tests and benchmarks. Regions are
/// neither `Send` nor `Sync`, so each one is counted on the thread that owns
/// it.
#[cfg(any(test, feature = "benchmarking-internals"))]
#[cfg_attr(
    windows,
    expect(
        clippy::missing_const_for_thread_local,
        reason = "The initializers are const blocks; Clippy misreads their expansion."
    )
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
#[cfg_attr(
    windows,
    expect(
        clippy::missing_const_for_thread_local,
        reason = "The initializers are const blocks; Clippy misreads their expansion."
    )
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
    // SAFETY: the pointer addresses aligned, writable storage for one
    // SYSTEM_INFO, which GetSystemInfo initializes.
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
    // It is left uninitialized, unlike an OS page, so that a read of bytes
    // never written is reported.
    NonNull::new(unsafe { std::alloc::alloc(layout) })
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
    // VirtualAlloc documents NULL as failure, so the checked conversion is
    // also the OS error check:
    // https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc
    NonNull::new(ptr.cast()).ok_or_else(io::Error::last_os_error)
}

#[cfg(all(unix, not(target_os = "linux"), not(miri)))]
fn os_reserve(bytes: usize) -> io::Result<NonNull<u8>> {
    os_map(bytes)
}

/// Maps `bytes` at a huge-page boundary when they span at least one huge
/// page, so the region's huge pages line up with the kernel's: it maps
/// enough extra to find the boundary and unmaps the unaligned ends.
#[cfg(all(target_os = "linux", not(miri)))]
fn os_reserve(bytes: usize) -> io::Result<NonNull<u8>> {
    if bytes < HUGE_PAGE {
        return os_map(bytes);
    }
    let page = page_size()?;
    let padded = bytes
        .checked_add(HUGE_PAGE - page)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid region size"))?;
    let mapped = os_map(padded)?;
    let address = mapped.as_ptr() as usize;
    // mmap returns a page-aligned address, so the boundary lies within the
    // first `HUGE_PAGE - page` bytes and `bytes` fit after it.
    let head = address.next_multiple_of(HUGE_PAGE) - address;
    let tail = padded - head - bytes;
    let base = mapped.as_ptr().wrapping_add(head);
    // The tail goes first, so if either unmap fails, what is still mapped is
    // one range starting at `mapped`, released without touching addresses
    // another thread may since have mapped.
    for (start, length, left) in [
        (base.wrapping_add(bytes), tail, padded),
        (mapped.as_ptr(), head, head + bytes),
    ] {
        if length == 0 {
            continue;
        }
        // SAFETY: each end lies inside the mapping just made, which nothing
        // else refers to yet, and is page-aligned because the mapping, the
        // boundary, and `bytes` all are.
        if unsafe { libc::munmap(start.cast(), length) } != 0 {
            let error = io::Error::last_os_error();
            // SAFETY: `left` bytes from `mapped` are exactly what is still
            // mapped of this reservation.
            _ = unsafe { libc::munmap(mapped.as_ptr().cast(), left) };
            return Err(error);
        }
    }
    // The offset is inside a non-null mapping and cannot wrap its address
    // range. Still check the trimmed OS address before constructing NonNull.
    mapping_base(base, || {
        // SAFETY: after trimming, this is exactly the remaining mapping;
        // nothing has obtained an allocation or reference into it yet.
        _ = unsafe { libc::munmap(base.cast(), bytes) };
    })
}

/// A new reservation: an anonymous private mapping with no access.
#[cfg(all(unix, not(miri)))]
fn os_map(bytes: usize) -> io::Result<NonNull<u8>> {
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
    // POSIX excludes zero for non-fixed mappings. Still check OS results:
    // Linux's mmap_min_addr is configurable, and the macOS/BSD manuals do
    // not explicitly guarantee nonzero anonymous mappings for a null hint.
    // https://pubs.opengroup.org/onlinepubs/9799919799/functions/mmap.html
    // https://man7.org/linux/man-pages/man2/mmap.2.html
    // https://docs.kernel.org/admin-guide/sysctl/vm.html#mmap-min-addr
    // https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/mmap.2.html
    // https://man.freebsd.org/cgi/man.cgi?query=mmap&sektion=2
    // https://man.netbsd.org/mmap.2 and https://man.openbsd.org/mmap.2
    mapping_base(ptr.cast(), || {
        // SAFETY: mmap succeeded (not MAP_FAILED); even a mapping at zero
        // must be released before rejecting it. No Rust reference exists.
        _ = unsafe { libc::munmap(ptr, bytes) };
    })
}

/// Checks a successful mapping's base, releasing it if it cannot be `NonNull`.
#[cfg(any(all(unix, not(miri)), test))]
fn mapping_base(ptr: *mut u8, release: impl FnOnce()) -> io::Result<NonNull<u8>> {
    match NonNull::new(ptr) {
        | Some(base) => Ok(base),
        | None => {
            release();
            Err(io::Error::other("mapping at address zero"))
        },
    }
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
    // SAFETY: `checked_range` kept the range inside the writable mock
    // reservation, and the caller has ended all allocations in it. The mock
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

#[cfg(all(target_os = "linux", not(miri)))]
fn os_decommit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    // Linux guarantees zero-fill-on-demand after MADV_DONTNEED for private
    // anonymous mappings (our reservations), preserving huge-page advice:
    // https://man7.org/linux/man-pages/man2/madvise.2.html
    // SAFETY: ptr and bytes cover page-aligned mapped memory with no live
    // references or allocations; MADV_DONTNEED discards the physical pages.
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

#[cfg(all(unix, not(target_os = "linux"), not(miri)))]
fn os_decommit(ptr: *mut u8, bytes: usize) -> io::Result<()> {
    // MADV_DONTNEED is only advice on macOS and BSDs, so replace the range
    // with fresh, zero-filled anonymous pages, inaccessible until recommit:
    // https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/madvise.2.html
    // https://man.freebsd.org/cgi/man.cgi?query=madvise&sektion=2
    // https://man.netbsd.org/madvise.2 and https://man.openbsd.org/madvise.2
    // MAP_FIXED replaces our pages at the same address; MAP_ANON zero-fills:
    // https://man.freebsd.org/cgi/man.cgi?query=mmap&sektion=2
    // https://man.netbsd.org/mmap.2
    // SAFETY: the checked page-aligned range is wholly owned by this region
    // and has no live references or allocations. MAP_FIXED replaces only
    // those pages, without releasing the address range for other threads.
    let result = unsafe {
        libc::mmap(
            ptr.cast(),
            bytes,
            libc::PROT_NONE,
            libc::MAP_FIXED | libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if result == libc::MAP_FAILED {
        Err(io::Error::last_os_error())
    } else {
        debug_assert_eq!(result.cast::<u8>(), ptr, "decommitted range moved");
        Ok(())
    }
}

/// Releases a whole reservation.
///
/// # Safety
///
/// `base` must be a reservation from [`os_reserve`] that is not yet released,
/// and no reference or allocation may remain in it. Release invalidates all
/// raw pointers obtained from it; none may be used afterwards.
#[cfg(all(windows, not(miri)))]
unsafe fn os_release(base: NonNull<u8>, _: usize) {
    use windows_sys::Win32::System::Memory::{
        MEM_RELEASE,
        VirtualFree,
    };
    // SAFETY: the caller passes the exact original reservation and no longer
    // uses it; MEM_RELEASE requires the zero size.
    let released = unsafe { VirtualFree(base.as_ptr().cast(), 0, MEM_RELEASE) };
    debug_assert_ne!(released, 0, "VirtualFree(MEM_RELEASE) failed");
}

/// Releases a whole reservation.
///
/// # Safety
///
/// `base` and `bytes` must be a reservation from [`os_reserve`] that is not
/// yet released, and no reference or allocation may remain in it. Release
/// invalidates all raw pointers obtained from it; none may be used afterwards.
#[cfg(all(unix, not(miri)))]
unsafe fn os_release(base: NonNull<u8>, bytes: usize) {
    // SAFETY: base and bytes are the mapping `os_reserve` kept after trimming
    // its ends, which the caller no longer uses; it is unmapped once.
    let released = unsafe { libc::munmap(base.as_ptr().cast(), bytes) };
    debug_assert_eq!(released, 0, "munmap failed");
}

impl Drop for Region {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "benchmarking-internals"))]
        accounting::update(|usage| {
            usage.regions -= 1;
            usage.reserved -= self.reserved;
        });
        // SAFETY: this is the original reservation base and size, released
        // exactly once. Owners end all references and allocations before
        // release. Every raw pointer obtained from it is now invalidated;
        // callers must not use those pointers again, regardless of whether
        // they still exist (raw pointers carry no borrow).
        unsafe {
            #[cfg(not(miri))]
            os_release(self.base, self.reserved);
            #[cfg(miri)]
            std::alloc::dealloc(self.base.as_ptr(), self.layout);
        }
    }
}

#[cfg(any(test, feature = "benchmarking-internals"))]
impl Drop for GrowingRegion {
    fn drop(&mut self) {
        accounting::update(|usage| usage.committed -= self.committed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_address_mappings_are_released_and_rejected() {
        let mut released = false;
        let error = mapping_base(std::ptr::null_mut(), || released = true).unwrap_err();
        assert!(released);
        assert_eq!(error.kind(), io::ErrorKind::Other);
        let mut byte = 0;
        let ptr = std::ptr::from_mut(&mut byte);
        assert_eq!(
            mapping_base(ptr, || released = false).unwrap().as_ptr(),
            ptr
        );
        assert!(released, "a non-null mapping must remain reserved");
    }

    #[cfg(miri)]
    #[test]
    fn committed_views_allow_uninitialized_storage_and_live_allocations() {
        let mut region = GrowingRegion::reserve(4 * FIRST_COMMIT_STEP).unwrap();
        region.ensure_committed(1).unwrap();
        let first = region.as_ptr().cast::<u64>();
        // SAFETY: the first committed slot is aligned and writable.
        unsafe {
            first.as_ptr().write(42);
        }
        // SAFETY: the slot is initialized and exclusively accessed here.
        let live = unsafe { &mut *first.as_ptr() };
        // A larger view covers this live allocation and still-unwritten bytes.
        region.ensure_committed(FIRST_COMMIT_STEP + 1).unwrap();
        *live += 1;
        assert_eq!(*live, 43);
        let next = region.as_ptr().as_ptr().wrapping_add(FIRST_COMMIT_STEP);
        // SAFETY: the new view reaches this newly committed, unwritten slot.
        unsafe {
            next.write(7);
        }
        // SAFETY: that slot is now initialized.
        assert_eq!(unsafe { next.read() }, 7);
    }

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
        assert_eq!(
            next_commit(0, 1, reserved, 4096, false),
            Some(FIRST_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(FIRST_COMMIT_STEP, 1, reserved, 4096, false),
            Some(FIRST_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(
                FIRST_COMMIT_STEP,
                FIRST_COMMIT_STEP + 1,
                reserved,
                4096,
                false
            ),
            Some(2 * FIRST_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(4 * 1024 * 1024, 9 * 1024 * 1024, reserved, 4096, false),
            Some(9 * 1024 * 1024)
        );
        // Doubling stops at the largest step: a large region commits one
        // step ahead of what it is asked for, not twice its size.
        assert_eq!(
            next_commit(
                MAX_COMMIT_STEP / 2,
                MAX_COMMIT_STEP / 2 + 1,
                reserved,
                4096,
                false
            ),
            Some(MAX_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(
                64 * MAX_COMMIT_STEP,
                64 * MAX_COMMIT_STEP + 1,
                reserved,
                4096,
                false
            ),
            Some(65 * MAX_COMMIT_STEP)
        );
        assert_eq!(
            next_commit(
                64 * MAX_COMMIT_STEP,
                70 * MAX_COMMIT_STEP + 1,
                reserved,
                4096,
                false
            ),
            Some(70 * MAX_COMMIT_STEP + 4096)
        );
        assert_eq!(
            next_commit(reserved - 4096, reserved, reserved, 4096, false),
            Some(reserved)
        );
        assert_eq!(
            next_commit(reserved, reserved + 1, reserved, 4096, false),
            None
        );
    }

    #[test]
    fn huge_page_commits_end_on_huge_page_boundaries() {
        let mib = 1024 * 1024;
        let reserved = 100 * 1024 * mib;
        // The first commit is a whole huge page, however little is needed.
        assert_eq!(next_commit(0, 1, reserved, 4096, true), Some(HUGE_PAGE));
        assert_eq!(
            next_commit(0, HUGE_PAGE, reserved, 4096, true),
            Some(HUGE_PAGE)
        );
        assert_eq!(
            next_commit(HUGE_PAGE, HUGE_PAGE, reserved, 4096, true),
            Some(HUGE_PAGE)
        );
        assert_eq!(
            next_commit(HUGE_PAGE, HUGE_PAGE + 1, reserved, 4096, true),
            Some(2 * HUGE_PAGE)
        );
        assert_eq!(
            next_commit(0, 9 * mib, reserved, 4096, true),
            Some(10 * mib)
        );
        // A prefix that a failed step left short of a boundary is completed.
        assert_eq!(
            next_commit(4096, 4097, reserved, 4096, true),
            Some(HUGE_PAGE)
        );
        for committed in [0, HUGE_PAGE, 7 * HUGE_PAGE] {
            for more in [
                1,
                4096,
                MAX_COMMIT_STEP,
                HUGE_PAGE,
                HUGE_PAGE + 1,
                5 * mib + 3,
            ] {
                let needed = committed + more;
                let target = next_commit(committed, needed, reserved, 4096, true).unwrap();
                assert!(target.is_multiple_of(HUGE_PAGE), "{target}");
                assert!(target >= needed, "{target}");
                assert!(target - needed < HUGE_PAGE, "{target}");
            }
        }
        // The reservation's end caps the last, partial huge page.
        let short = 3 * mib;
        assert_eq!(
            next_commit(HUGE_PAGE, HUGE_PAGE + 1, short, 4096, true),
            Some(short)
        );
        assert_eq!(next_commit(short, short + 1, short, 4096, true), None);
    }

    #[test]
    fn huge_page_regions_commit_and_write_whole_huge_pages() {
        let before = accounting::live();
        let mut region = GrowingRegion::reserve_with_huge_pages(16 * 1024 * 1024, true).unwrap();
        region.ensure_committed(1).unwrap();
        assert_eq!(region.committed(), HUGE_PAGE);
        region.ensure_committed(HUGE_PAGE + 1).unwrap();
        assert_eq!(region.committed(), 2 * HUGE_PAGE);
        assert_eq!(
            accounting::live().committed - before.committed,
            2 * HUGE_PAGE
        );
        let base = region.as_ptr().as_ptr();
        let last = base.wrapping_add(region.committed() - 1);
        // SAFETY: both bytes lie in the committed prefix, which `base`
        // reaches because it was taken after the commit.
        unsafe {
            base.write(1);
        }
        // SAFETY: as above.
        unsafe {
            last.write(2);
        }
        // SAFETY: as above, and the byte was just written.
        assert_eq!(unsafe { last.read() }, 2);
        drop(region);
        assert_eq!(accounting::live(), before);
    }

    #[test]
    fn a_failed_huge_page_commit_falls_back_to_the_pages_needed() {
        let mut region = GrowingRegion::reserve_with_huge_pages(16 * 1024 * 1024, true).unwrap();
        let page = region.region.page;
        {
            let _limited = faults::limit_commits(page);
            region.ensure_committed(1).unwrap();
            assert_eq!(region.committed(), page);
        }
        // The next commit completes the huge page the fallback started.
        region.ensure_committed(page + 1).unwrap();
        assert_eq!(region.committed(), HUGE_PAGE);
    }

    #[test]
    fn regions_smaller_than_a_huge_page_have_no_huge_pages() {
        for bytes in [4096, 1024 * 1024, HUGE_PAGE - 4096] {
            let mut region = GrowingRegion::reserve(bytes).unwrap();
            assert!(!region.huge_pages, "{bytes}");
            region.ensure_committed(1).unwrap();
            assert_eq!(region.committed(), FIRST_COMMIT_STEP.min(bytes));
        }
    }

    /// The `/proc/self/smaps` entry of the mapping that starts at `start`:
    /// its end and its `VmFlags`.
    #[cfg(all(target_os = "linux", not(miri)))]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "The test reads the kernel's view of its mappings with std."
    )]
    fn mapping_at(start: usize) -> Option<(usize, String)> {
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
        let mut found = None;
        for line in smaps.lines() {
            let range = line
                .split_once(' ')
                .and_then(|(range, _)| range.split_once('-'))
                .and_then(|(from, to)| {
                    Some((
                        usize::from_str_radix(from, 16).ok()?,
                        usize::from_str_radix(to, 16).ok()?,
                    ))
                });
            if let Some((from, to)) = range {
                found = (from == start).then_some(to);
            } else if let (Some(end), Some(flags)) = (found, line.strip_prefix("VmFlags:")) {
                return Some((end, flags.trim().to_owned()));
            }
        }
        None
    }

    #[cfg(all(target_os = "linux", not(miri)))]
    #[test]
    fn linux_regions_start_on_a_huge_page_and_commit_whole_advised_huge_pages() {
        let mut region = GrowingRegion::reserve(REGION_BYTES).unwrap();
        let base = region.as_ptr().as_ptr() as usize;
        assert!(base.is_multiple_of(HUGE_PAGE), "{base:#x}");
        if !std::path::Path::new("/sys/kernel/mm/transparent_hugepage").exists() {
            // A kernel without transparent huge pages refuses the advice.
            assert!(!region.huge_pages);
            return;
        }
        assert!(region.huge_pages);
        region.ensure_committed(HUGE_PAGE + 1).unwrap();
        assert_eq!(region.committed(), 2 * HUGE_PAGE);
        let byte = region.as_ptr().as_ptr().wrapping_add(HUGE_PAGE);
        // SAFETY: the byte lies in the committed prefix.
        unsafe {
            byte.write_volatile(1);
        }
        let has = |flags: &str, flag: &str| flags.split(' ').any(|each| each == flag);
        // The committed huge pages are one writable, advised mapping, and the
        // reserved rest of the region another, inaccessible one.
        let (committed_end, committed_flags) = mapping_at(base).unwrap();
        assert_eq!(committed_end, base + 2 * HUGE_PAGE);
        assert!(has(&committed_flags, "hg"), "{committed_flags}");
        assert!(has(&committed_flags, "wr"), "{committed_flags}");
        let (rest_end, rest_flags) = mapping_at(committed_end).unwrap();
        assert_eq!(rest_end, base + REGION_BYTES);
        assert!(has(&rest_flags, "hg"), "{rest_flags}");
        assert!(!has(&rest_flags, "wr"), "{rest_flags}");
    }

    #[test]
    fn failed_commit_step_falls_back_to_the_pages_needed() {
        let mut region = GrowingRegion::reserve_with_huge_pages(16 * 1024 * 1024, false).unwrap();
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
    fn decommit_recommit_zeroes_the_whole_range_and_preserves_neighbors() {
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
        region.commit(0, 4 * page).unwrap();
        let first = region.as_ptr().as_ptr();
        // SAFETY: all four pages are committed and exclusively accessed here.
        unsafe {
            first.write_bytes(42, 4 * page);
        }
        for _ in 0..2 {
            // SAFETY: no reference or allocation remains in the middle pages;
            // their old pointers are not used after decommit.
            unsafe {
                region.decommit(page, 2 * page).unwrap();
            }
            region.commit(page, 2 * page).unwrap();
            let middle = region.as_ptr().as_ptr().wrapping_add(page);
            // SAFETY: derive a fresh pointer after recommit, which guarantees
            // zero-filled pages on every backend, not just Linux.
            let bytes = unsafe { std::slice::from_raw_parts(middle, 2 * page) };
            assert!(bytes.iter().all(|&byte| byte == 0));
            // SAFETY: these pages remain committed and the slice is no longer
            // used. Dirty the whole range before the next decommit.
            unsafe {
                middle.write_bytes(99, 2 * page);
            }
        }
        for offset in [0, 3 * page] {
            let neighbor = region.as_ptr().as_ptr().wrapping_add(offset);
            // SAFETY: the untouched neighboring page remains initialized.
            let bytes = unsafe { std::slice::from_raw_parts(neighbor, page) };
            assert!(bytes.iter().all(|&byte| byte == 42));
        }
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
