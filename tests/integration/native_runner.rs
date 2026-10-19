//! Tests of the shared deadline-bound runner, and `run_until_timeout` for
//! tests that expect its deadline to fire.

use std::{
    io,
    process::Command,
    time::{
        Duration,
        Instant,
    },
};

/// Runs `command`, which must not finish, until the runner's deadline stops
/// it after it has printed `ready`, and returns the runner's timeout report.
///
/// A run that times out before printing `ready` only shows that the host was
/// too slow to start the program, as a loaded Windows CI runner can be, so it
/// is retried with the next, longer deadline. Every attempt must time out,
/// and the runner must return within `KILL_MARGIN` of the deadline.
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Tests inspect owned timeout reports."
)]
pub(crate) fn run_until_timeout(command: &mut Command, ready: &str) -> String {
    for deadline in DEADLINES {
        let start = Instant::now();
        let error = crate::native::run_native_with_timeout(command, deadline)
            .expect_err("the program outlives its deadline");
        let elapsed = start.elapsed();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(elapsed < deadline + KILL_MARGIN, "{elapsed:?}: {error}");
        let message = error.to_string();
        let (_, captured) = message
            .split_once("\nstdout:\n")
            .expect("timeouts report the captured output");
        if captured.contains(ready) {
            return message;
        }
        eprintln!("{command:?} did not print {ready:?} within {deadline:?}; retrying");
    }
    panic!("{command:?} never printed {ready:?} before its deadline");
}

/// The deadlines `run_until_timeout` tries, shortest first. The first keeps
/// the tests fast on an idle host; the others give a loaded one ample time to
/// start the program.
const DEADLINES: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

/// How soon after its deadline the runner must have killed the program,
/// reaped it, and given up on pipes an escaped descendant still holds.
const KILL_MARGIN: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{
        DEADLINES,
        KILL_MARGIN,
        run_until_timeout,
    };

    #[test]
    fn completed_program_returns_captured_output() {
        let output =
            crate::native::run_native(Command::new(std::env::current_exe().unwrap()).arg("--list"))
                .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(!output.stdout.is_empty(), "the test list is captured");
    }

    #[test]
    fn inherited_pipes_cannot_outlive_the_reader_deadline() {
        drop(run_until_timeout(
            Command::new(std::env::current_exe().unwrap()).args([
                "--ignored",
                "--exact",
                "native_runner::tests::exiting_parent_fixture",
                "--nocapture",
            ]),
            "parent exited",
        ));
    }

    #[test]
    #[ignore = "Subprocess fixture whose descendant outlives the direct child."]
    #[expect(
        clippy::zombie_processes,
        reason = "Deliberately exits with a live, finite descendant."
    )]
    fn exiting_parent_fixture() {
        let _descendant = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "native_runner::tests::descendant_fixture",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        println!("parent exited");
    }

    #[test]
    fn timeout_kills_descendants_with_inherited_pipes() {
        let report = run_until_timeout(
            Command::new(std::env::current_exe().unwrap()).args([
                "--ignored",
                "--exact",
                "native_runner::tests::parent_fixture",
                "--nocapture",
            ]),
            "descendant started",
        );
        #[cfg(not(windows))]
        drop(report);
        #[cfg(windows)]
        {
            let pid: u32 = report
                .lines()
                .find_map(|line| line.strip_prefix("descendant pid "))
                .expect("parent reported its descendant")
                .parse()
                .unwrap();
            assert!(
                !process_alive(pid),
                "descendant {pid} survived tree termination"
            );
        }
    }

    #[test]
    #[ignore = "Subprocess fixture invoked by the timeout regression."]
    fn parent_fixture() {
        let mut descendant = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "native_runner::tests::descendant_fixture",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        println!("descendant pid {}", descendant.id());
        assert!(descendant.wait().unwrap().success());
    }

    #[test]
    #[ignore = "Subprocess fixture that inherits its parent's captured pipes."]
    fn descendant_fixture() {
        println!("descendant started");
        // Outlives every deadline and its kill margin, so a runner that waits
        // for the descendant fails the margin check, yet finite even on the
        // broken runner, so the red regression cannot hang.
        let longest = DEADLINES[DEADLINES.len() - 1];
        std::thread::sleep(longest + KILL_MARGIN * 2);
    }

    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        use windows_sys::Win32::{
            Foundation::{
                CloseHandle,
                STILL_ACTIVE,
            },
            System::Threading::{
                GetExitCodeProcess,
                OpenProcess,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: query access opens an owned handle for the reported child
        // pid.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        // SAFETY: the handle is live and code points to writable storage.
        let queried = unsafe { GetExitCodeProcess(handle, &raw mut code) };
        // SAFETY: this test owns the handle and closes it exactly once.
        _ = unsafe { CloseHandle(handle) };
        assert_ne!(queried, 0, "query descendant status");
        code == STILL_ACTIVE as u32
    }
}
