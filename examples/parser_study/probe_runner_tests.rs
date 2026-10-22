//! Opt-in runner evidence on the real native worker, with no parser linking.

use super::{
    reserved_stack,
    touch,
};

#[test]
#[ignore = "explicit native runner measurement; reserves large virtual stacks"]
fn runner_stack_measurements() {
    for bytes in [1024_usize.pow(3), 100 * 1024_usize.pow(3)] {
        for depth in [0, 256, 4096, 65_536] {
            let result = reserved_stack::run(bytes, |bounds| {
                let before = bounds.snapshot();
                let marker = 0_u8;
                let (span, deepest) = touch(depth, (&raw const marker).addr(), bounds);
                let after = bounds.snapshot();
                (bounds, before, deepest, after, span)
            });
            let (bounds, before, deepest, after, span) = match result {
                | Ok(measured) => measured,
                | Err(error) => {
                    println!("runner_stack reserve={bytes} depth={depth} error={error:?}");
                    assert_ne!(bytes, 1024_usize.pow(3), "1 GiB worker must start");
                    break;
                },
            };
            println!(
                "runner_stack reserve={bytes} actual={} depth={depth} span={span} \
                 before_commit={} deepest_commit={} after_commit={} before_rss={} deepest_rss={}",
                bounds.bytes,
                before.writable_bytes,
                deepest.writable_bytes,
                after.writable_bytes,
                before.resident_bytes,
                deepest.resident_bytes,
            );
            assert!(
                bounds.bytes >= bytes,
                "worker owns the requested reservation"
            );
            assert!(span < bounds.bytes, "recursion stays inside reservation");
            #[cfg(windows)]
            {
                assert!(bounds.guard_bytes > 0, "native Windows stack has a guard");
                assert!(
                    before.writable_bytes < 4 * 1024 * 1024,
                    "initial commit is small"
                );
                assert!(
                    after.writable_bytes >= deepest.writable_bytes,
                    "commit survives return"
                );
                if depth > 0 {
                    assert!(
                        deepest.writable_bytes > before.writable_bytes,
                        "commit grows on demand"
                    );
                }
            }
            #[cfg(target_os = "linux")]
            {
                assert!(
                    before.resident_bytes < 4 * 1024 * 1024,
                    "initial RSS is small"
                );
                if depth > 0 {
                    assert!(
                        deepest.resident_bytes > before.resident_bytes,
                        "RSS grows on demand"
                    );
                }
            }
        }
    }
}
