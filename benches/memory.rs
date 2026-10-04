//! Reports the peak heap use of each pipeline stage, which the timing
//! benchmarks cannot show.

#![expect(
    unused_crate_dependencies,
    reason = "We have a bunch of dependencies that are not used in our memory benchmark."
)]

use std::{
    alloc::{
        GlobalAlloc,
        Layout,
        System,
    },
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use bcc_rust::BenchmarkInput;

/// The system allocator, counting live and peak bytes.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Every method forwards to `System` unchanged and only updates
// counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            _ = PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe {
            System.dealloc(ptr, layout);
        }
        _ = LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller upholds `GlobalAlloc::realloc`'s contract.
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            _ = LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
            _ = PEAK.fetch_max(live, Ordering::Relaxed);
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Peak bytes allocated above the current baseline while `run` executes.
fn peak_during(run: impl FnOnce()) -> usize {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    run();
    PEAK.load(Ordering::Relaxed) - baseline
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Mebibyte figures are only printed."
)]
fn mebibytes(bytes: usize) -> f64 {
    bytes as f64 / f64::from(1 << 20)
}

fn main() {
    // The strategy column keeps the table comparable with results from
    // when the pipeline had several scheduling strategies.
    println!("| input | phases | strategy | peak heap (MiB) |");
    println!("|---|---|---|---:|");
    for input in BenchmarkInput::ALL {
        let lexing = peak_during(|| _ = bcc_rust::lex(input));
        let preprocessing = peak_during(|| _ = bcc_rust::preprocess(input));
        let parsing = peak_during(|| _ = bcc_rust::parse(input));
        for (phases, bytes) in [("1-3", lexing), ("1-6", preprocessing), ("1-7", parsing)] {
            println!(
                "| {} | {phases} | batch | {:.1} |",
                input.name(),
                mebibytes(bytes)
            );
        }
    }
}
