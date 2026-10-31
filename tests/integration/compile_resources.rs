//! Peak memory and time of whole compilations on inputs whose cost once
//! grew faster than the input. Each runs the compiler as a child process,
//! kills it as soon as it outgrows its memory or time budget, and reports
//! its exact peak resident set (`ru_maxrss`) once it exits.

#[cfg(test)]
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        path::PathBuf,
        process::{
            Command,
            Stdio,
        },
        sync::atomic::{
            AtomicUsize,
            Ordering,
        },
        thread,
        time::{
            Duration,
            Instant,
        },
    };

    /// What one compilation may use: its peak resident set and wall time.
    #[derive(Clone, Copy, Debug)]
    struct Budget {
        megabytes: u64,
        seconds:   u64,
    }

    /// A scratch directory for one compilation, removed with its files.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "bcc-compile-resources-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for name in ["input.c", "output.o", "stderr.txt"] {
                drop(fs::remove_file(self.0.join(name)));
            }
            drop(fs::remove_dir(&self.0));
        }
    }

    /// Compiles `source` with `args` and `-c`, failing if the compiler
    /// exceeds `budget` or does not succeed.
    fn compile_within(source: &str, args: &[&str], budget: Budget) {
        let scratch = Scratch::new();
        let input = scratch.0.join("input.c");
        fs::write(&input, source).unwrap();
        let stderr = fs::File::create(scratch.0.join("stderr.txt")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(args)
            .arg("-c")
            .arg(&input)
            .arg("-o")
            .arg(scratch.0.join("output.o"))
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .unwrap();
        let pid = libc::pid_t::try_from(child.id()).unwrap();
        let limit = budget.megabytes << 20;
        let start = Instant::now();
        let (status, peak) = loop {
            let mut status = 0;
            // SAFETY: `rusage` is plain old data that wait4 fills in.
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            // SAFETY: `pid` is this test's unreaped child; the out-pointers
            // are live locals.
            let reaped =
                unsafe { libc::wait4(pid, &raw mut status, libc::WNOHANG, &raw mut usage) };
            assert!(reaped >= 0, "wait4: {}", std::io::Error::last_os_error());
            if reaped == pid {
                // Linux reports `ru_maxrss` in KiB.
                break (status, u64::try_from(usage.ru_maxrss).unwrap() << 10);
            }
            let resident = resident_bytes(pid);
            let elapsed = start.elapsed();
            if resident > limit || elapsed.as_secs() >= budget.seconds {
                // SAFETY: `pid` is this test's unreaped child, so the signal
                // cannot reach another process.
                _ = unsafe { libc::kill(pid, libc::SIGKILL) };
                // SAFETY: As above; this reaps the killed child.
                _ = unsafe { libc::waitpid(pid, &raw mut status, 0) };
                panic!(
                    "{args:?} exceeded {budget:?}: {} MiB resident after {elapsed:?}",
                    resident >> 20
                );
            }
            thread::sleep(Duration::from_millis(5));
        };
        // std's handle must not wait for the child this loop reaped.
        drop(child);
        let messages = fs::read_to_string(scratch.0.join("stderr.txt")).unwrap();
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "{args:?}: status {status:#x}\n{messages}"
        );
        assert!(
            peak <= limit,
            "{args:?}: peak {} MiB exceeds {budget:?}",
            peak >> 20
        );
    }

    /// The child's current resident set, or 0 once it has exited.
    fn resident_bytes(pid: libc::pid_t) -> u64 {
        fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|status| {
                let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
                line.split_whitespace().nth(1)?.parse::<u64>().ok()
            })
            .map_or(0, |kilobytes| kilobytes << 10)
    }

    /// GCC's `limits-fndefn.c` defines a function with 100,000 parameters.
    /// The LLVM back end added each parameter's attributes one at a time,
    /// and LLVM keeps every intermediate attribute list, each as long as the
    /// highest index so far: 1.5 GiB for 20,000 parameters, tens of GiB for
    /// GCC's test. Calls, including variadic ones, annotate their arguments
    /// the same way.
    #[test]
    fn many_parameters_and_arguments_stay_linear() {
        let count = 20_000;
        let mut source = String::from("void f(");
        for index in 0..count {
            write!(source, "int p{index}, ").unwrap();
        }
        source.push_str("int last) {}\nint printf(const char *, ...);\nvoid g(void) { f(");
        for index in 0..count {
            write!(source, "{index}, ").unwrap();
        }
        source.push_str("0); printf(\"\"");
        for index in 0..count {
            write!(source, ", {index}").unwrap();
        }
        source.push_str("); }\n");
        for optimization in ["-O0", "-O2"] {
            compile_within(
                &source,
                &["-w", optimization],
                Budget {
                    megabytes: 400,
                    seconds:   60,
                },
            );
        }
    }

    /// GCC's `pr39323-1.c` asks for 2^29-byte alignment, which GCC rejects
    /// and Clang ignores. Honouring it padded the object to 512 MiB, and
    /// the LLVM back end held about twice that while building it.
    #[test]
    fn alignments_above_two_to_the_28_cost_no_padding() {
        let source = "int foo __attribute__((aligned(1 << 29))) = 20;\n__attribute__((aligned(1 \
                      << 30))) int f(void) { return foo; }\n";
        for backend in ["--backend=llvm", "--backend=native"] {
            compile_within(
                source,
                &["--compiler-identity=clang", "-w", backend],
                Budget {
                    megabytes: 160,
                    seconds:   60,
                },
            );
        }
    }
}
