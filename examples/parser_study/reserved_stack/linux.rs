//! Linux demand-paged native stack with inaccessible guards at both ends.

use std::{
    ffi::c_void,
    io,
    mem::MaybeUninit,
    ptr,
};

use super::{
    Bounds,
    Packet,
};

pub(super) fn run<F, R>(bytes: usize, callback: F) -> io::Result<R>
where
    F: FnOnce(Bounds) -> R + Send,
    R: Send,
{
    // SAFETY: sysconf reads the page size and does not access caller memory.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .map_err(|_| io::Error::last_os_error())?;
    let round = |value: usize| value.checked_add(page - 1).map(|n| n / page * page);
    let bytes = round(bytes)
        .filter(|&n| n >= libc::PTHREAD_STACK_MIN && n <= isize::MAX as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid stack reservation"))?;
    let guard_bytes = round(64 * 1024).expect("guard fits a page multiple");
    let total = bytes
        .checked_add(2 * guard_bytes)
        .filter(|&n| n <= isize::MAX as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "stack reservation overflow"))?;
    // SAFETY: requests a fresh anonymous range, initially inaccessible. No
    // physical stack pages are prefaulted or reserved against Linux commit.
    let base = unsafe {
        libc::mmap(
            ptr::null_mut(),
            total,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK,
            -1,
            0,
        )
    };
    if base == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let mapping = Mapping { base, bytes: total };
    // SAFETY: the low guard is smaller than this live mapping.
    let usable = unsafe { base.cast::<u8>().add(guard_bytes) };
    // SAFETY: this page-aligned subrange belongs exclusively to mapping. RW
    // protection permits demand faults; it does not touch its pages.
    if unsafe { libc::mprotect(usable.cast(), bytes, libc::PROT_READ | libc::PROT_WRITE) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: advice covers our writable stack mapping and changes backing
    // granularity only. Small pages avoid THP obscuring shallow residency.
    if unsafe { libc::madvise(usable.cast(), bytes, libc::MADV_NOHUGEPAGE) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut raw_attr = MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: raw_attr provides writable, aligned storage for one attribute.
    check(unsafe { libc::pthread_attr_init(raw_attr.as_mut_ptr()) })?;
    // SAFETY: pthread_attr_init succeeded and initialized raw_attr.
    let mut attr = Attributes(unsafe { raw_attr.assume_init() });
    // SAFETY: attr is initialized; explicit mapping guards replace pthread's.
    check(unsafe { libc::pthread_attr_setguardsize(&raw mut attr.0, 0) })?;
    // SAFETY: usable points to bytes writable, page-aligned bytes kept mapped
    // until the child exits and is joined; no parent allocations overlap it.
    check(unsafe { libc::pthread_attr_setstack(&raw mut attr.0, usable.cast(), bytes) })?;
    let mut packet = Packet {
        callback: Some(callback),
        result:   None,
        bounds:   Bounds {
            base: usable.addr(),
            bytes,
            guard_bytes,
        },
    };
    let mut thread = MaybeUninit::<libc::pthread_t>::uninit();
    // SAFETY: packet stays at this address until join; the parent does not
    // access it meanwhile. F and R are Send, attributes/stack outlive the
    // child.
    check(unsafe {
        libc::pthread_create(
            thread.as_mut_ptr(),
            &raw const attr.0,
            start::<F, R>,
            (&raw mut packet).cast(),
        )
    })?;
    // SAFETY: successful pthread_create initialized the joinable thread handle.
    let thread = unsafe { thread.assume_init() };
    // SAFETY: this is the only joiner, and no thread cancellation is used.
    let joined = unsafe { libc::pthread_join(thread, ptr::null_mut()) };
    if joined != 0 {
        // Returning or unwinding could unmap a still-live stack/packet.
        std::process::abort();
    }
    drop(mapping);
    Ok(packet.finish())
}

extern "C" fn start<F, R>(argument: *mut c_void) -> *mut c_void
where
    F: FnOnce(Bounds) -> R,
{
    // SAFETY: run passed exclusive access to its live packet until join.
    let packet = unsafe { &mut *argument.cast::<Packet<F, R>>() };
    packet.invoke();
    ptr::null_mut()
}

fn check(code: libc::c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

struct Mapping {
    base:  *mut c_void,
    bytes: usize,
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this owns the entire mapping and no child is using it.
        let _ = unsafe { libc::munmap(self.base, self.bytes) };
    }
}

struct Attributes(libc::pthread_attr_t);

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: these attributes were initialized and are no longer in use.
        let _ = unsafe { libc::pthread_attr_destroy(&raw mut self.0) };
    }
}
