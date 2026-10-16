#![expect(
    missing_docs,
    unused_crate_dependencies,
    reason = "Native runner regression tests."
)]

#[path = "support/native.rs"]
mod native;

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        time::{
            Duration,
            Instant,
        },
    };

    #[test]
    fn completed_program_returns_captured_output() {
        let output =
            super::native::run_native(Command::new(std::env::current_exe().unwrap()).arg("--list"))
                .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(!output.stdout.is_empty(), "the test list is captured");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "Tests inspect owned timeout diagnostic strings."
    )]
    fn inherited_pipes_cannot_outlive_the_reader_deadline() {
        let start = Instant::now();
        let error = super::native::run_native_with_timeout(
            Command::new(std::env::current_exe().unwrap()).args([
                "--ignored",
                "--exact",
                "tests::exiting_parent_fixture",
                "--nocapture",
            ]),
            Duration::from_millis(500),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("parent exited"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2), "{error}");
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
                "tests::descendant_fixture",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        println!("parent exited");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "Tests inspect owned timeout diagnostic strings."
    )]
    fn timeout_kills_descendants_with_inherited_pipes() {
        let start = Instant::now();
        let error = super::native::run_native_with_timeout(
            Command::new(std::env::current_exe().unwrap()).args([
                "--ignored",
                "--exact",
                "tests::parent_fixture",
                "--nocapture",
            ]),
            Duration::from_millis(500),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("descendant started"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2), "{error}");
        #[cfg(windows)]
        {
            let message = error.to_string();
            let pid: u32 = message
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
                "tests::descendant_fixture",
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
        // Finite even on the broken runner, so the red regression cannot hang.
        std::thread::sleep(Duration::from_secs(4));
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
