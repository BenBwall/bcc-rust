//! Native compilation worker stacks with a fixed, lazily backed reservation.
//!
//! Linux supplies a guarded `MAP_NORESERVE` mapping to pthreads. Windows asks
//! `CreateThread` to reserve the requested size and lets its guard commit
//! pages. Compiler calls have no stack-growth checks. Neither implementation
//! moves active frames or catches overflow; exhausting the reservation remains
//! fatal. The callback and its arenas live on the worker, and completion is
//! synchronous.

#[cfg(target_os = "linux")]
#[path = "compilation_stack/linux.rs"]
mod linux;
#[path = "compilation_stack/telemetry.rs"]
mod telemetry;
#[cfg(windows)]
#[path = "compilation_stack/windows.rs"]
mod windows;

use std::{
    io,
    panic::{
        self,
        AssertUnwindSafe,
    },
    thread,
};

pub(crate) use telemetry::{
    Faults,
    Snapshot,
};

/// Runs one borrowed, sendable callback and joins before returning. Panics are
/// captured inside the native entry point and resumed on the calling thread.
pub(crate) fn run<F, R>(bytes: usize, callback: F) -> io::Result<R>
where
    F: FnOnce(Bounds) -> R + Send,
    R: Send,
{
    #[cfg(target_os = "linux")]
    return linux::run(bytes, callback);
    #[cfg(windows)]
    return windows::run(bytes, callback);
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (bytes, callback);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reserved stacks require Linux or Windows",
        ))
    }
}

/// Usable address interval, excluding explicit Linux guards. Windows fills
/// the interval from `VirtualQuery` on the worker's actual stack reservation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    pub(crate) base:        usize,
    pub(crate) bytes:       usize,
    pub(crate) guard_bytes: usize,
}

impl Bounds {
    fn contains(self, address: usize) -> bool {
        address >= self.base && address - self.base < self.bytes
    }
}

/// Parent-owned packet is never accessed by the parent between native thread
/// creation and successful join. Both native entry points contain all unwinds.
struct Packet<F, R> {
    callback: Option<F>,
    result:   Option<thread::Result<R>>,
    bounds:   Bounds,
}

impl<F, R> Packet<F, R>
where
    F: FnOnce(Bounds) -> R,
{
    fn invoke(&mut self) {
        self.result = Some(panic::catch_unwind(AssertUnwindSafe(|| {
            #[cfg(windows)]
            {
                self.bounds = windows::current_bounds().expect("query native stack reservation");
            }
            let marker = 0_u8;
            assert!(
                self.bounds.contains((&raw const marker).addr()),
                "worker must use reserved stack"
            );
            self.callback.take().expect("one worker invocation")(self.bounds)
        })));
    }

    fn finish(self) -> R {
        match self.result.expect("joined worker supplied a result") {
            | Ok(value) => value,
            | Err(payload) => panic::resume_unwind(payload),
        }
    }
}

#[cfg(test)]
#[cfg(any(target_os = "linux", windows))]
mod tests {
    use super::*;

    #[test]
    fn borrows_and_returns_on_a_lazy_stack() {
        let source = [1, 2, 3];
        let (sum, snapshot) = run(1024 * 1024 * 1024, |bounds| {
            (source.iter().sum::<usize>(), bounds.snapshot())
        })
        .expect("reserve and join stack");
        assert_eq!(sum, 6);
        #[cfg(target_os = "linux")]
        assert!(
            snapshot.resident_bytes < 4 * 1024 * 1024,
            "reservation must not be eagerly resident"
        );
        assert!(snapshot.writable_bytes > 0, "worker stack is writable");
        #[cfg(target_os = "linux")]
        assert!(
            snapshot.noreserve,
            "Linux mapping must bypass reservation commit accounting"
        );
    }

    #[test]
    fn propagates_panics_after_join() {
        let result = panic::catch_unwind(|| {
            run(1024 * 1024, |_| panic!("worker failure")).expect("start worker")
        })
        .expect_err("worker panic must reach caller");
        assert_eq!(result.downcast_ref::<&str>(), Some(&"worker failure"));
    }

    #[test]
    fn rejects_unrepresentable_reservations() {
        let error = run(usize::MAX, |_| ()).expect_err("invalid stack size");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
