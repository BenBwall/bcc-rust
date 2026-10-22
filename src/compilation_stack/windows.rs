//! Windows native stack reservation. The OS commits pages behind its moving
//! guard; this module installs no fault handler and performs no stack
//! switching.

use std::{
    ffi::c_void,
    io,
    mem::MaybeUninit,
    ptr,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle,
        WAIT_OBJECT_0,
    },
    System::{
        Memory::{
            MEM_COMMIT,
            MEMORY_BASIC_INFORMATION,
            PAGE_GUARD,
            VirtualQuery,
        },
        Threading::{
            CreateThread,
            INFINITE,
            STACK_SIZE_PARAM_IS_A_RESERVATION,
            WaitForSingleObject,
        },
    },
};

use super::{
    Bounds,
    Packet,
    Snapshot,
};

pub(super) fn run<F, R>(bytes: usize, callback: F) -> io::Result<R>
where
    F: FnOnce(Bounds) -> R + Send,
    R: Send,
{
    if bytes < 64 * 1024 || bytes > isize::MAX as usize - 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid stack reservation",
        ));
    }
    let mut packet = Packet {
        callback: Some(callback),
        result:   None,
        bounds:   Bounds {
            base: 0,
            bytes,
            guard_bytes: 0,
        },
    };
    // SAFETY: packet stays live and exclusively worker-owned until the wait
    // completes. F and R are Send; CreateThread manages its stack and TLS.
    let handle = unsafe {
        CreateThread(
            ptr::null(),
            bytes,
            Some(start::<F, R>),
            (&raw mut packet).cast(),
            STACK_SIZE_PARAM_IS_A_RESERVATION,
            ptr::null_mut(),
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this owns the live thread handle and waits without a deadline.
    if unsafe { WaitForSingleObject(handle, INFINITE) } != WAIT_OBJECT_0 {
        // The worker may still own packet. Never return/unwind in this case.
        std::process::abort();
    }
    // SAFETY: the worker has exited, and this owns its only handle.
    let _ = unsafe { CloseHandle(handle) };
    Ok(packet.finish())
}

unsafe extern "system" fn start<F, R>(argument: *mut c_void) -> u32
where
    F: FnOnce(Bounds) -> R,
{
    // SAFETY: run passes exclusive access to its live packet until join.
    let packet = unsafe { &mut *argument.cast::<Packet<F, R>>() };
    packet.invoke();
    0
}

pub(super) fn current_bounds() -> io::Result<Bounds> {
    let marker = 0_u8;
    let first = query((&raw const marker).addr())?;
    let base = first.AllocationBase.addr();
    let mut end = base;
    let mut guard_bytes = 0;
    loop {
        let info = query(end)?;
        if info.AllocationBase.addr() != base {
            break;
        }
        if info.Protect & PAGE_GUARD != 0 {
            guard_bytes += info.RegionSize;
        }
        end += info.RegionSize;
    }
    Ok(Bounds {
        base,
        bytes: end - base,
        guard_bytes,
    })
}

pub(super) fn snapshot(bounds: Bounds) -> Snapshot {
    // VirtualQuery reports commit/protection, not physical residency. Windows
    // RSS/swap fields deliberately remain zero (unavailable); Linux is
    // measured.
    let mut snapshot = Snapshot::default();
    let mut cursor = bounds.base;
    while cursor < bounds.base + bounds.bytes {
        let info = query(cursor).expect("query stack commit");
        if info.State == MEM_COMMIT && info.Protect & PAGE_GUARD == 0 {
            snapshot.writable_bytes += info.RegionSize;
        }
        cursor += info.RegionSize;
    }
    snapshot
}

fn query(address: usize) -> io::Result<MEMORY_BASIC_INFORMATION> {
    let mut info = MaybeUninit::<MEMORY_BASIC_INFORMATION>::uninit();
    // SAFETY: VirtualQuery inspects the address without dereferencing it and
    // writes one complete structure into correctly aligned output storage.
    if unsafe {
        VirtualQuery(
            ptr::with_exposed_provenance::<c_void>(address),
            info.as_mut_ptr(),
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful VirtualQuery initialized info.
    Ok(unsafe { info.assume_init() })
}
