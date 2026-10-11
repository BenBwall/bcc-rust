//! Deadline-bound process execution shared by unit and integration tests.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Test runners own captured process output outside compilation."
)]

use std::{
    io::{
        self,
        Read,
    },
    process::{
        Child,
        Command,
        ExitStatus,
        Output,
        Stdio,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

/// Runs a program with captured output and a 20-second deadline, overridden
/// by `BCC_TEST_TIMEOUT_MS` (a positive integer number of milliseconds).
pub(crate) fn run_native(command: &mut Command) -> io::Result<Output> {
    let timeout = match std::env::var("BCC_TEST_TIMEOUT_MS") {
        | Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|&value| value > 0)
            .map(Duration::from_millis)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "BCC_TEST_TIMEOUT_MS must be a positive integer",
                )
            })?,
        | Err(std::env::VarError::NotPresent) => Duration::from_secs(20),
        | Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    run_native_with_timeout(command, timeout)
}

/// Runs with an explicit deadline, kills and reaps a timed-out program, and
/// reports its command and captured output. Both pipes drain concurrently so
/// a program printing more than a pipe's capacity can still finish.
pub(crate) fn run_native_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    thread::scope(|scope| {
        let stdout = scope.spawn(|| read_pipe(stdout));
        let stderr = scope.spawn(|| read_pipe(stderr));
        let status = wait_until(&mut child, timeout);
        if status.is_err() {
            // A process may exit between the last poll and kill. Waiting still
            // reaps it in that case, and also closes the captured pipes.
            drop(child.kill());
            _ = child.wait()?;
        }
        let stdout = stdout.join().expect("stdout reader does not panic")?;
        let stderr = stderr.join().expect("stderr reader does not panic")?;
        match status {
            | Ok(status) => Ok(Output {
                status,
                stdout,
                stderr,
            }),
            | Err(error) => Err(io::Error::new(
                error.kind(),
                format!(
                    "{command:?}: {error} after {timeout:?}\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                ),
            )),
        }
    })
}

fn wait_until(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "native execution timed out",
    ))
}

fn read_pipe(mut pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    _ = pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}
