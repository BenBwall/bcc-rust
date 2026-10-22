//! Thread-local allocation hooks delimit lowering, excluding preparation and
//! untimed inspection. Identical rules to the parser study allocator.

use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
};

pub(super) fn begin() {
    CALLS.set(0);
    BYTES.set(0);
    ACTIVE.set(true);
}
pub(super) fn end() {
    ACTIVE.set(false);
}
pub(super) fn counts() -> (usize, usize) {
    (CALLS.get(), BYTES.get())
}

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: each allocation operation delegates unchanged to the system
// allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplies this allocator's live block and its
        // layout.
        let result = unsafe { System.realloc(ptr, layout, new_size) };
        if !result.is_null() {
            record(new_size);
        }
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller supplies this allocator's live block and its
        // layout.
        unsafe {
            System.dealloc(ptr, layout);
        }
    }
}

fn record(bytes: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        CALLS.set(CALLS.get() + 1);
        BYTES.set(BYTES.get() + bytes);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;
