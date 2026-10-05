//! Counts global allocations made while the compiler runs.
//!
//! This integration test has its own global allocator, leaving the library's
//! ordinary unit-test process untouched. Arena memory comes from the
//! operating system's virtual memory, so everything counted here is an
//! allocation outside `util::bump`. Counting is per thread, so tests running
//! in parallel do not see each other's allocations.
//!
//! Set `BCC_ALLOCATION_SITES=1` to also record where each counted allocation
//! comes from: [`tests::report_allocation_sites`] then prints the call sites
//! with their counts for the golden diagnostic fixtures and for every file
//! listed, one path per line, in the file named by `BCC_ALLOCATION_INPUTS`.
#![expect(
    unused_crate_dependencies,
    reason = "This test binary uses only the measurement API."
)]

#[cfg(test)]
#[expect(
    clippy::missing_const_for_thread_local,
    reason = "Every thread-local initializer here is `const`; the lint misreads the macro in this \
              test crate."
)]
mod counting {
    use std::{
        alloc::{
            GlobalAlloc,
            Layout,
            System,
        },
        backtrace::Backtrace,
        cell::{
            Cell,
            RefCell,
        },
        collections::BTreeMap,
    };

    /// Calls and requested bytes.
    pub(crate) type Totals = (usize, usize);

    // `RECORDING` is set while this thread records a site, whose own
    // allocations are not counted.
    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static CAPTURE: Cell<bool> = const { Cell::new(false) };
        static RECORDING: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
        static BYTES: Cell<usize> = const { Cell::new(0) };
        static SITES: RefCell<BTreeMap<String, Totals>> = const { RefCell::new(BTreeMap::new()) };
    }

    pub(crate) struct Counting;

    // SAFETY: every operation delegates to `System` with the caller's
    // arguments.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                record(layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
            unsafe {
                System.dealloc(ptr, layout);
            }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
            let moved = unsafe { System.realloc(ptr, layout, new_size) };
            if !moved.is_null() {
                record(new_size);
            }
            moved
        }
    }

    fn record(size: usize) {
        // Thread-local storage may already be gone while a thread exits.
        let counted = ACTIVE.try_with(Cell::get).unwrap_or(false)
            && !RECORDING.try_with(Cell::get).unwrap_or(true);
        if !counted {
            return;
        }
        CALLS.set(CALLS.get() + 1);
        BYTES.set(BYTES.get() + size);
        if CAPTURE.get() {
            RECORDING.set(true);
            let call_stack = allocation_site();
            SITES.with_borrow_mut(|sites| {
                let totals = sites.entry(call_stack).or_default();
                totals.0 += 1;
                totals.1 += size;
            });
            RECORDING.set(false);
        }
    }

    /// The innermost compiler frames of the current call stack, each with its
    /// source location, preceded by the frame they called into.
    fn allocation_site() -> String {
        let trace = Backtrace::force_capture().to_string();
        let mut frames = Vec::new();
        let mut callee = None;
        let mut lines = trace.lines().peekable();
        while let Some(line) = lines.next() {
            let line = line.trim_start();
            if line.starts_with("at ") {
                continue;
            }
            let Some((_, name)) = line.split_once(": ") else {
                continue;
            };
            let location = lines
                .peek()
                .and_then(|next| next.trim_start().strip_prefix("at "))
                .map(|path| {
                    let path = path.replace('\\', "/");
                    path.rfind("/src/")
                        .map_or(path.clone(), |start| path[start + 1..].to_owned())
                });
            // Compiler functions, methods, and trait implementations, but not
            // library code instantiated with compiler types.
            let compiler = name.starts_with("bcc_rust::")
                || name.starts_with("<bcc_rust::")
                || (name.starts_with('<') && name.contains(" as bcc_rust::"));
            if compiler {
                if name.contains("compile_file_measured") {
                    break;
                }
                frames.push(match location {
                    | Some(location) => format!("{name} ({location})"),
                    | None => name.to_owned(),
                });
                if frames.len() == 4 {
                    break;
                }
            } else if frames.is_empty() && !name.starts_with("allocation_count::") {
                callee = Some(name.to_owned());
            }
        }
        let mut site = frames.join("\n      < ");
        if let Some(callee) = callee {
            site.push_str("\n      > ");
            site.push_str(&callee);
        }
        site
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// Whether `BCC_ALLOCATION_SITES` asks for allocation sites.
    pub(crate) fn sites_requested() -> bool {
        std::env::var_os("BCC_ALLOCATION_SITES").is_some_and(|value| value == "1")
    }

    /// Runs `body`, returning the global allocations this thread made in it.
    pub(crate) fn count(capture: bool, body: impl FnOnce()) -> Totals {
        CALLS.set(0);
        BYTES.set(0);
        CAPTURE.set(capture);
        ACTIVE.set(true);
        body();
        ACTIVE.set(false);
        CAPTURE.set(false);
        (CALLS.get(), BYTES.get())
    }

    /// Takes the sites recorded on this thread so far.
    pub(crate) fn take_sites() -> BTreeMap<String, Totals> {
        SITES.take()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io,
        path::{
            Path,
            PathBuf,
        },
    };

    use bcc_rust::CompileStep;

    use crate::counting::{
        Totals,
        count,
        sites_requested,
        take_sites,
    };

    fn golden_fixtures() -> Vec<PathBuf> {
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
        let mut fixtures: Vec<_> = fs::read_dir(directory)
            .expect("diagnostic fixtures must be readable")
            .map(|entry| entry.expect("fixture entry must be readable").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
            .collect();
        fixtures.sort();
        assert!(
            !fixtures.is_empty(),
            "the diagnostic corpus must not be empty"
        );
        fixtures
    }

    /// Compiles `path`, counting the global allocations of `measured` steps.
    fn compile(path: &Path, measured: &[CompileStep], capture: bool) -> Vec<(CompileStep, Totals)> {
        let mut totals = Vec::new();
        bcc_rust::compile_file_measured(path, &mut io::sink(), |step, run| {
            if measured.contains(&step) {
                totals.push((step, count(capture, run)));
            } else {
                run();
            }
        })
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        totals
    }

    /// The counter sees an allocation made through the global allocator, so a
    /// count of zero means none was made.
    #[test]
    fn counting_sees_global_allocations() {
        let (calls, bytes) = count(false, || drop(std::hint::black_box(Box::new(7_u64))));
        assert_eq!((calls, bytes), (1, 8));
    }

    /// Building, folding, ordering, and rendering every golden diagnostic,
    /// as the CLI reports them, allocates only in arenas.
    #[test]
    fn reporting_golden_diagnostics_makes_no_global_allocations() {
        let mut failures = Vec::new();
        for fixture in golden_fixtures() {
            // Room for the whole report, so writing it does not allocate.
            let mut stderr = Vec::with_capacity(1 << 20);
            let mut totals = (0, 0);
            bcc_rust::compile_file_measured(&fixture, &mut stderr, |step, run| {
                if step == CompileStep::Report {
                    totals = count(sites_requested(), run);
                } else {
                    run();
                }
            })
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.display()));
            assert!(!stderr.is_empty(), "{} reported nothing", fixture.display());
            let (calls, bytes) = totals;
            if calls != 0 {
                failures.push(format!(
                    "{}: {calls} global allocations, {bytes} bytes",
                    fixture.display()
                ));
            }
        }
        for (site, (calls, _)) in take_sites() {
            println!("{calls:>6}  {site}");
        }
        assert!(
            failures.is_empty(),
            "reporting diagnostics allocated outside the arenas; rerun with \
             BCC_ALLOCATION_SITES=1 and --nocapture to print their sites:\n{}",
            failures.join("\n")
        );
    }

    /// Prints where global allocations come from, per compile step, when
    /// `BCC_ALLOCATION_SITES=1`; otherwise does nothing.
    #[test]
    fn report_allocation_sites() {
        if !sites_requested() {
            return;
        }
        let mut inputs = golden_fixtures();
        if let Some(list) = std::env::var_os("BCC_ALLOCATION_INPUTS") {
            let list = fs::read_to_string(list).expect("the input list must be readable");
            inputs.extend(
                list.lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from),
            );
        }
        for step in [CompileStep::Parse, CompileStep::Report] {
            let mut calls = 0;
            let mut bytes = 0;
            let mut files = 0;
            drop(take_sites());
            for input in &inputs {
                for (_, totals) in compile(input, &[step], true) {
                    calls += totals.0;
                    bytes += totals.1;
                    files += usize::from(totals.0 > 0);
                }
            }
            let mut sites: Vec<_> = take_sites().into_iter().collect();
            sites.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.0));
            println!(
                "== {step:?}: {calls} global allocations, {bytes} bytes, in {files} of {} inputs",
                inputs.len()
            );
            for (site, (calls, bytes)) in sites {
                println!("{calls:>8} calls {bytes:>10} bytes  {site}");
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "benchmarking-internals")]
mod measurements {
    use bcc_rust::BenchmarkInput;

    use crate::counting::count;

    #[test]
    fn report_parser_mix_global_allocations() {
        let input = BenchmarkInput::ParserMix;
        // Force source generation outside the measured interval.
        _ = input.bytes();
        let mut result = None;
        let (calls, bytes) = count(false, || result = Some(bcc_rust::parse(input)));
        println!(
            "parser mix: result={result:?}, global allocation calls={calls}, requested \
             bytes={bytes}"
        );
    }
}
