//! Reports global allocations during one compiler benchmark input.
//!
//! This integration test has its own global allocator, leaving the library's
//! ordinary unit-test process untouched. Once phases use `util::bump`, a
//! classification hook can exclude its chunk allocations from these counters.
#![expect(
    unused_crate_dependencies,
    reason = "This test binary uses only the benchmark API."
)]

#[cfg(test)]
#[cfg(feature = "benchmarking-internals")]
mod measurements {
    use std::{
        alloc::{
            GlobalAlloc,
            Layout,
            System,
        },
        sync::atomic::{
            AtomicBool,
            AtomicUsize,
            Ordering,
        },
    };

    use bcc_rust::BenchmarkInput;

    struct Counting;
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static BYTES: AtomicUsize = AtomicUsize::new(0);

    // SAFETY: every operation delegates to `System` with the caller's
    // arguments.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
            let ptr = unsafe { System.alloc(layout) };
            if ACTIVE.load(Ordering::Relaxed) && !ptr.is_null() {
                _ = CALLS.fetch_add(1, Ordering::Relaxed);
                _ = BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
            unsafe {
                System.dealloc(ptr, layout);
            }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
            let moved = unsafe { System.realloc(ptr, layout, new_size) };
            if ACTIVE.load(Ordering::Relaxed) && !moved.is_null() {
                _ = CALLS.fetch_add(1, Ordering::Relaxed);
                _ = BYTES.fetch_add(new_size, Ordering::Relaxed);
            }
            moved
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    #[test]
    fn report_parser_mix_global_allocations() {
        let input = BenchmarkInput::ParserMix;
        // Force source generation outside the measured interval.
        _ = input.bytes();
        CALLS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        ACTIVE.store(true, Ordering::Relaxed);
        let result = bcc_rust::parse(input);
        ACTIVE.store(false, Ordering::Relaxed);
        println!(
            "parser mix: result={result:?}, global allocation calls={}, requested bytes={}",
            CALLS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed)
        );
    }
}
