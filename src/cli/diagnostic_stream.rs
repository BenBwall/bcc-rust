//! The stream a job writes its diagnostics to: stderr, as GCC, Clang and TCC
//! write theirs, or stdout, where `cl` writes its diagnostics, its fatal
//! errors (`C1083`) and its `/showIncludes` notes while it compiles. `cl`
//! keeps stdout for the text under `/E` and `/EP` and writes everything else
//! to stderr then, also under `/P`; the `cl` personality asks for stdout
//! only for compile and `/Zs` jobs (`--diagnostics-to-stdout`). Ninja reads
//! the notes from stdout for its `deps = msvc` dependencies, and `MSBuild` and
//! IDEs read the diagnostics there. The command line's own messages (`cl :
//! Command line error D8003 : ...`) go to stderr in both, as `cl` writes
//! them.
//!
//! The driver runs one job at a time in one process, so the choice is a
//! process-wide switch [`select`] sets before each job runs.

use std::{
    fmt,
    io::{
        self,
        Write as _,
    },
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
};

/// Writes to the stream [`select`] chose last.
pub(super) struct DiagnosticStream;

impl io::Write for DiagnosticStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if TO_STDOUT.load(Ordering::Relaxed) {
            io::stdout().lock().write(buf)
        } else {
            io::stderr().lock().write(buf)
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if TO_STDOUT.load(Ordering::Relaxed) {
            let mut stdout = io::stdout().lock();
            stdout.write_all(buf).and_then(|()| stdout.flush())
        } else {
            io::stderr().lock().write_all(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if TO_STDOUT.load(Ordering::Relaxed) {
            io::stdout().lock().flush()
        } else {
            io::stderr().lock().flush()
        }
    }
}

/// Writes `text` and a newline to the diagnostic stream, ignoring a closed
/// stream.
pub(super) fn report(text: fmt::Arguments<'_>) {
    drop(writeln!(DiagnosticStream, "{text}"));
}

/// Whether diagnostics go to stdout.
static TO_STDOUT: AtomicBool = AtomicBool::new(false);

/// Sends the diagnostics of the jobs that follow to stdout, or to stderr.
pub(super) fn select(to_stdout: bool) {
    TO_STDOUT.store(to_stdout, Ordering::Relaxed);
}
