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
    time::Instant,
};

use bcc_rust::ParserStudyMode;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() >= 2,
        "usage: parser_study SOURCE [REPEATS] [--mode \
         machine|recursive|growing|hybrid:N|limited:N] [--inspect DEST] [--stack BYTES] \
         [--sample-stack]"
    );
    let source = fs::read_to_string(&args[1]).expect("read study source");
    let repeats = args
        .get(2)
        .filter(|text| !text.starts_with('-'))
        .map_or(1, |text| text.parse::<usize>().expect("integer repeats"));
    let option = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .map(|index| args.get(index + 1).expect("option value").clone())
    };
    let mode = match option("--mode").as_deref().unwrap_or("machine") {
        | "machine" => ParserStudyMode::Machine,
        | "recursive" => ParserStudyMode::Recursive,
        | "growing" => ParserStudyMode::Growing,
        | mode if mode.starts_with("hybrid:") => ParserStudyMode::Hybrid {
            depth: mode[7..].parse().expect("integer hybrid depth"),
        },
        | mode if mode.starts_with("limited:") => ParserStudyMode::Limited {
            depth: mode[8..].parse().expect("integer limit depth"),
        },
        | mode => panic!("unknown parser mode {mode}"),
    };
    let inspection = option("--inspect");
    let sample_stack = args.iter().any(|arg| arg == "--sample-stack");
    if let Some(stack) = option("--stack") {
        let stack = stack.parse::<usize>().expect("integer stack bytes");
        let before = Instant::now();
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || {
                println!(
                    "thread_launch_ns={} stack_bytes={stack}",
                    before.elapsed().as_nanos()
                );
                run(&source, repeats, mode, sample_stack, inspection.as_deref());
            })
            .expect("spawn parser thread")
            .join()
            .expect("join parser thread");
    } else {
        run(&source, repeats, mode, sample_stack, inspection.as_deref());
    }
}

fn run(
    source: &str,
    repeats: usize,
    mode: ParserStudyMode,
    sample_stack: bool,
    inspection: Option<&str>,
) {
    for index in 0..repeats {
        let result = bcc_rust::with_study_source(source, |prepared| {
            prepared.measure_with_mode(
                mode,
                sample_stack,
                inspection.is_some(),
                || {
                    CALLS.set(0);
                    BYTES.set(0);
                    ACTIVE.set(true);
                },
                || ACTIVE.set(false),
            )
        });
        println!(
            "sample={index} ns={} tokens={} roots={} diagnostics={} parse_bytes={} tu_bytes={} \
             commit_bytes={} regions={} global_calls={} global_bytes={} expressions={} \
             statements={} compounds={} depth={} stack_span={} fallbacks={} limit_hits={} \
             rss_before={} rss_after={}",
            result.nanoseconds,
            result.tokens,
            result.roots,
            result.diagnostics,
            result.parse_bytes,
            result.tu_bytes,
            result.peak_commit_bytes,
            result.peak_regions,
            CALLS.get(),
            BYTES.get(),
            result.expression_entries,
            result.statement_entries,
            result.compound_entries,
            result.max_depth,
            result.native_stack_span,
            result.fallback_count,
            result.depth_limit_hits,
            result.peak_rss_before,
            result.peak_rss_after
        );
        if let Some(text) = &result.inspection {
            fs::write(inspection.expect("inspection destination"), text).expect("write inspection");
        }
        drop(black_box(result));
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
