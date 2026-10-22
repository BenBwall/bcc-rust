//! Portable native-stack reservation probe, independent of parser/LLVM linking.
//! Use scripts/parser_study_windows.py to cross-build this same source with
//! repository lint settings and the actual reserved-stack modules.
#![expect(
    unused_crate_dependencies,
    reason = "This standalone probe exercises only the worker stack implementation."
)]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "The experiment owns arguments and output outside the compiler."
)]

#[path = "parser_study/reserved_stack.rs"]
mod reserved_stack;

use std::{
    hint::black_box,
    panic,
    time::Instant,
};

fn main() {
    #[cfg(windows)]
    // SAFETY: suppress crash dialogs for this probe and any children.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(
            windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX
                | windows_sys::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS,
        );
    }
    let args: Vec<String> = std::env::args().collect();
    let bytes = args.get(1).map_or(100 * 1024_usize.pow(3), |arg| {
        arg.parse().expect("reservation in bytes")
    });
    let depth = args.get(2).map_or(0, |arg| arg.parse().expect("depth"));
    let started = Instant::now();
    let result = reserved_stack::run(bytes, |bounds| {
        let launch_ns = started.elapsed().as_nanos();
        let before = bounds.snapshot();
        let marker = 0_u8;
        let top = (&raw const marker).addr();
        let recursion_started = Instant::now();
        let (span, deepest) = touch(depth, top, bounds);
        let recurse_ns = recursion_started.elapsed().as_nanos();
        let after = bounds.snapshot();
        (
            bounds,
            before,
            deepest,
            after,
            span,
            launch_ns,
            recurse_ns,
        )
    });
    let total_ns = started.elapsed().as_nanos();
    let (bounds, before, deepest, after, span, launch_ns, recurse_ns) =
        result.expect("reserve worker stack");
    println!(
        "reserve={} actual={} guard={} depth={} span={} before={} deepest={} after={} \
         resident={} launch_ns={} recurse_ns={} total_ns={}",
        bytes,
        bounds.bytes,
        bounds.guard_bytes,
        depth,
        span,
        before.writable_bytes,
        deepest.writable_bytes,
        after.writable_bytes,
        deepest.resident_bytes,
        launch_ns,
        recurse_ns,
        total_ns,
    );
    // Verify that a foreign-thread entry contains panics and returns them to
    // the caller before any borrowed packet can be dropped.
    panic::set_hook(Box::new(|_| {}));
    let propagated = panic::catch_unwind(|| {
        reserved_stack::run(1024 * 1024, |_| panic!("probe panic")).expect("panic worker")
    });
    assert_eq!(
        propagated.expect_err("worker panic reaches caller").downcast_ref::<&str>(),
        Some(&"probe panic")
    );
    let borrowed = [1, 2, 3];
    assert_eq!(
        reserved_stack::run(1024 * 1024, |_| borrowed.iter().sum::<usize>()).expect("borrowed worker"),
        6
    );
    let invalid = reserved_stack::run(usize::MAX, |_| ()).expect_err("reject invalid reservation");
    assert_eq!(invalid.kind(), std::io::ErrorKind::InvalidInput);
    println!("checks=panic,borrow,invalid passed=3");
}

/// One live page per level, plus ABI/compiler overhead. Keep the page live
/// across the call and sample commit at maximum depth, before it can shrink.
#[inline(never)]
fn touch(
    depth: usize,
    top: usize,
    bounds: reserved_stack::Bounds,
) -> (usize, reserved_stack::Snapshot) {
    let mut page = [0_u8; 4096];
    black_box(&mut page);
    let result = if depth == 0 {
        (top.abs_diff(page.as_ptr().addr()), bounds.snapshot())
    } else {
        touch(black_box(depth - 1), top, bounds)
    };
    black_box(&mut page);
    result
}
