//! Running Linux commands under WSL from a Windows host, with bounded retries
//! when `wsl.exe` itself fails.

use std::{
    io,
    process::{
        Command,
        Output,
    },
};

use super::native::run_native;

/// Runs a `wsl.exe` command like `run_native`, retrying it at most
/// `HOST_RETRIES` times when WSL fails on the Windows side, with a note on
/// stderr for each retry.
///
/// On the Windows CI runner, `wsl.exe` intermittently exits with
/// `0xC000070A` after the Linux program has printed its complete output. Such
/// a status says nothing about the Linux program; see `is_host_failure` for
/// why it cannot be one. Timeouts and every other status are returned
/// unchanged, and the last attempt's result stands.
pub(crate) fn run_wsl(command: &mut Command) -> io::Result<Output> {
    let label = format!("{command:?}");
    retry_host_failures(
        &label,
        || run_native(command),
        |output| output.status.code(),
    )
}

/// The retry loop of `run_wsl`, over any attempt whose result has an exit
/// status, so that tests can count attempts without WSL.
fn retry_host_failures<T>(
    label: &str,
    mut attempt: impl FnMut() -> io::Result<T>,
    status: impl Fn(&T) -> Option<i32>,
) -> io::Result<T> {
    for retry in 1..=HOST_RETRIES {
        let result = attempt()?;
        match status(&result) {
            | Some(code) if is_host_failure(code) => eprintln!(
                "retrying {label} ({retry} of {HOST_RETRIES}): WSL failed on the host side with \
                 {:#010X}",
                code.cast_unsigned()
            ),
            | _ => return Ok(result),
        }
    }
    attempt()
}

/// How often `run_wsl` retries a command after a host-side failure.
const HOST_RETRIES: u32 = 2;

/// Whether `wsl.exe`'s exit code is a Windows NTSTATUS error from facility 0
/// (`0xC0000000..=0xC000FFFF`, read as unsigned).
///
/// A Linux program's status reaches Windows only as what WSL relays: its exit
/// code, which Linux truncates to a byte, or for a program killed by a
/// signal, a wait status built from the signal number and the core-dump bit.
/// Both are small non-negative numbers, far from this range, which only
/// Windows produces. The exact relayed value for a signal varies between WSL
/// versions, so nothing here depends on it.
fn is_host_failure(code: i32) -> bool {
    code.cast_unsigned() & 0xFFFF_0000 == 0xC000_0000
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        io,
    };

    use super::{
        HOST_RETRIES,
        is_host_failure,
        retry_host_failures,
    };

    #[test]
    fn only_facility_zero_ntstatus_errors_are_host_failures() {
        // -1073740022 is the 0xC000070A that CI's WSL launcher returned.
        assert!(is_host_failure(-1_073_740_022));
        for code in [0xC000_0000_u32, 0xC000_0005, 0xC000_FFFF] {
            assert!(is_host_failure(code.cast_signed()), "{code:#X}");
        }
        // Every status a Linux program can produce through WSL is below 256.
        for code in 0..=255 {
            assert!(!is_host_failure(code), "{code}");
        }
        for code in [0xBFFF_FFFF_u32, 0xC001_0000, 0x8000_0005, 0xFFFF_FFFF] {
            assert!(!is_host_failure(code.cast_signed()), "{code:#X}");
        }
    }

    /// Runs `retry_host_failures` over `statuses` in order and returns the
    /// status it settled on and how many attempts it made.
    fn settle(statuses: &[i32]) -> (i32, usize) {
        let attempts = Cell::new(0);
        let status = retry_host_failures(
            "test",
            || {
                attempts.set(attempts.get() + 1);
                Ok(statuses[attempts.get() - 1])
            },
            |&status| Some(status),
        )
        .unwrap();
        (status, attempts.get())
    }

    #[test]
    fn host_failures_are_retried_at_most_twice() {
        const HOST: i32 = 0xC000_070A_u32.cast_signed();
        assert_eq!(HOST_RETRIES, 2);
        assert_eq!(settle(&[0]), (0, 1));
        assert_eq!(settle(&[HOST, 0]), (0, 2));
        assert_eq!(settle(&[HOST, HOST, 3]), (3, 3));
        // A failure that persists is reported after the last attempt.
        assert_eq!(settle(&[HOST, HOST, HOST]), (HOST, 3));
    }

    #[test]
    fn program_failures_and_errors_are_not_retried() {
        for status in [1, 6, 11, 134, 139, 255] {
            assert_eq!(settle(&[status, 0]), (status, 1), "{status}");
        }
        let attempts = Cell::new(0);
        let error = retry_host_failures(
            "test",
            || -> io::Result<i32> {
                attempts.set(attempts.get() + 1);
                Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"))
            },
            |&status| Some(status),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(attempts.get(), 1);
    }
}
