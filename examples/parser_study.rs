//! Standalone phase-7 study runner. Preparation and optional syntax inspection
//! are outside the wall-time and global-allocation interval.
#![expect(
    unused_crate_dependencies,
    reason = "The study executable uses only the library measurement API."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "The study harness owns command arguments, input text, and reporting buffers outside \
              compilation."
)]

use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    cell::Cell,
    fs,
    hint::black_box,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 2,
        "usage: parser_study SOURCE [REPEATS] [INSPECTION_DEST]"
    );
    let source = fs::read_to_string(&args[1]).expect("read study source");
    let repeats = args
        .get(2)
        .map_or(1, |text| text.parse::<usize>().expect("integer repeats"));
    for index in 0..repeats {
        let inspect = args.get(3).is_some();
        let result = bcc_rust::with_study_source(&source, |prepared| {
            prepared.measure(
                inspect,
                || {
                    CALLS.set(0);
                    BYTES.set(0);
                    ACTIVE.set(true);
                },
                || ACTIVE.set(false),
            )
        });
        println!(
            "sample={index} ns={} roots={} diagnostics={} parse_bytes={} tu_bytes={} \
             commit_bytes={} regions={} global_calls={} global_bytes={}",
            result.nanoseconds,
            result.roots,
            result.diagnostics,
            result.parse_bytes,
            result.tu_bytes,
            result.peak_commit_bytes,
            result.peak_regions,
            CALLS.get(),
            BYTES.get()
        );
        if let Some(text) = &result.inspection {
            fs::write(&args[3], text).expect("write inspection");
        }
        black_box(result);
    }
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

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the caller supplies this allocator's live block and its
        // layout.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller supplies this allocator's live block and its
        // layout.
        unsafe {
            System.dealloc(pointer, layout);
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
