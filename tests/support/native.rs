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
    sync::{
        Arc,
        Mutex,
    },
    thread::{
        self,
        JoinHandle,
    },
    time::{
        Duration,
        Instant,
    },
};

/// Runs a program with captured output and the `DEFAULT_TIMEOUT` deadline,
/// overridden by `BCC_TEST_TIMEOUT_MS` (a positive integer number of
/// milliseconds).
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
        | Err(std::env::VarError::NotPresent) => DEFAULT_TIMEOUT,
        | Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    run_native_with_timeout(command, timeout)
}

/// How long `run_native` lets a program run. The deadline only stops hangs,
/// so it is generous: a loaded Windows CI runner has taken more than 20
/// seconds for a WSL `clang` link and for an `--emit=ir` run that finishes in
/// well under a second on an idle machine.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs with an explicit deadline, kills and reaps a timed-out program, and
/// reports its command and captured output. Both pipes drain concurrently so
/// a program printing more than a pipe's capacity can still finish.
pub(crate) fn run_native_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<Output> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Establish the group before exec, so descendants inherit it.
        _ = command.process_group(0);
    }
    #[cfg(windows)]
    let tree = ProcessTree::new()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    #[cfg(windows)]
    if let Err(error) = tree.assign(&child) {
        drop(child.kill());
        drop(child.wait());
        return Err(error);
    }
    #[cfg(not(windows))]
    let tree = ProcessTree(child.id());
    let stdout = CapturedPipe::spawn(child.stdout.take().expect("stdout is piped"));
    let stderr = CapturedPipe::spawn(child.stderr.take().expect("stderr is piped"));
    let start = Instant::now();
    let mut status = wait_until(&mut child, timeout.saturating_sub(start.elapsed()));
    if status.is_ok() && !wait_readers(&stdout, &stderr, timeout.saturating_sub(start.elapsed())) {
        status = Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "native execution timed out: captured pipes stayed open",
        ));
    }
    if status.is_err() {
        tree.kill(&mut child);
        _ = child.wait()?;
        // A descendant may escape the group/tree or inherit pipes elsewhere.
        // Detached readers must never extend the deadline indefinitely.
        _ = wait_readers(&stdout, &stderr, Duration::from_millis(250));
    }
    let stdout = stdout.finish()?;
    let stderr = stderr.finish()?;
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
}

/// Owns the child's process tree. The job handle is not inheritable, so
/// closing our sole handle terminates all assigned Windows descendants.
#[cfg(windows)]
struct ProcessTree(std::os::windows::io::OwnedHandle);

#[cfg(not(windows))]
struct ProcessTree(u32);

impl ProcessTree {
    /// Stops the whole tree before reaping the direct child.
    fn kill(self, child: &mut Child) {
        #[cfg(windows)]
        drop(self);
        #[cfg(unix)]
        {
            if let Ok(group) = libc::pid_t::try_from(self.0) {
                // SAFETY: the child was spawned as leader of its own process
                // group. A negative pid targets that group, not the test
                // runner.
                _ = unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
        drop(child.kill());
    }
}

#[cfg(windows)]
impl ProcessTree {
    fn new() -> io::Result<Self> {
        use std::os::windows::io::{
            AsRawHandle as _,
            FromRawHandle as _,
            OwnedHandle,
        };

        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW,
            JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: null attributes create a non-inheritable handle; null name
        // creates an anonymous job owned exclusively by this runner invocation.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a valid owned handle. This owner
        // closes it exactly once, including configuration/assignment failure.
        let tree = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // This job becomes the child's innermost one, and Windows applies only
        // that job's limits, so it repeats the cargo runner's crash-dialog
        // limit (scripts/crash_dialogs.py): an unhandled exception terminates
        // the program instead of opening an error dialog.
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .expect("job information size fits DWORD");
        // SAFETY: the job handle is live and the pointer and length describe
        // initialized extended-limit storage for this information class.
        let configured = unsafe {
            SetInformationJobObject(
                tree.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(tree)
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle as _;

        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: both handles are live; the freshly spawned child handle has
        // the rights needed for job assignment. No reader is started
        // beforehand.
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn wait_readers(stdout: &CapturedPipe, stderr: &CapturedPipe, timeout: Duration) -> bool {
    let start = Instant::now();
    while !stdout.thread.is_finished() || !stderr.thread.is_finished() {
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())));
    }
    true
}

/// Each reader publishes bytes without holding the lock across a blocking
/// read, so even an abandoned reader's partial output can be reported.
struct CapturedPipe {
    thread: JoinHandle<io::Result<()>>,
    bytes:  Arc<Mutex<Vec<u8>>>,
}

impl CapturedPipe {
    fn spawn(mut pipe: impl Read + Send + 'static) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&bytes);
        let thread = thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                let count = match pipe.read(&mut buffer) {
                    | Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    | result => result?,
                };
                if count == 0 {
                    return Ok(());
                }
                output
                    .lock()
                    .expect("output lock is not poisoned")
                    .extend_from_slice(&buffer[..count]);
            }
        });
        Self { thread, bytes }
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        if self.thread.is_finished() {
            self.thread.join().expect("output reader does not panic")?;
        }
        // Dropping an unfinished JoinHandle detaches it; no scoped join can
        // block the timeout path after the grace period.
        Ok(std::mem::take(
            &mut *self.bytes.lock().expect("output lock is not poisoned"),
        ))
    }
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
