//! Opt-in whole-compilation worker with lowering-only timing and differential
//! IR inspection. Every prepared compiler object stays inside the worker.
#![expect(
    unused_crate_dependencies,
    reason = "The study uses the library's benchmark API."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The harness owns inputs and output outside the compiler interval."
)]

use std::{
    fs,
    time::Instant,
};

use bcc_rust::{
    measure_lowering_source,
    with_compilation_stack,
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() >= 2,
        "usage: lowering_study SOURCE [machine|recursive] [REPEATS] [--inspect] [--sample-stack]"
    );
    let source = fs::read_to_string(&args[1]).expect("read source");
    let recursive = match args.get(2).map(String::as_str).unwrap_or("machine") {
        | "machine" => false,
        | "recursive" => true,
        | other => panic!("unknown mode {other}"),
    };
    let repeats = args
        .get(3)
        .filter(|s| !s.starts_with('-'))
        .map_or(1, |s| s.parse().expect("repeat count"));
    let inspect = args.iter().any(|s| s == "--inspect");
    let sample = args.iter().any(|s| s == "--sample-stack");
    for repeat in 0..repeats {
        let wall = Instant::now();
        let measured = with_compilation_stack(100 * 1024_usize.pow(3), || {
            measure_lowering_source(&source, recursive, sample, inspect, || {}, || {})
        })
        .expect("compilation worker");
        println!(
            "sample={repeat} mode={} ns={} total_ns={} functions={} diagnostics={} errors={} \
             scratch_bytes={} ir_bytes={} commit_bytes={} entries={} depth={} stack_span={}",
            if recursive { "recursive" } else { "machine" },
            measured.nanoseconds,
            wall.elapsed().as_nanos(),
            measured.functions,
            measured.diagnostics,
            measured.errors,
            measured.scratch_bytes,
            measured.ir_bytes,
            measured.peak_commit_bytes,
            measured.entries,
            measured.max_depth,
            measured.stack_span
        );
        if let Some(text) = measured.inspection {
            println!("{text}");
        }
    }
}

#[cfg(test)]
#[path = "lowering_study/runner.rs"]
mod runner;
