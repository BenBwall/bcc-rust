//! Reports OS peak memory use for each pipeline stage. Each sample runs in a
//! separate process because OS peak counters cannot be reset.

#![expect(
    unused_crate_dependencies,
    reason = "The memory benchmark uses only the benchmark API and OS counters."
)]

#[cfg(windows)]
use std::mem::size_of;
use std::{
    env,
    process::{
        Command,
        ExitCode,
    },
};

use bcc_rust::BenchmarkInput;

struct PeakMemory {
    commit:      Option<usize>,
    working_set: usize,
}

#[cfg(windows)]
fn peak_memory() -> std::io::Result<PeakMemory> {
    use windows_sys::Win32::System::{
        ProcessStatus::{
            GetProcessMemoryInfo,
            PROCESS_MEMORY_COUNTERS,
            PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::GetCurrentProcess,
    };

    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>())
            .expect("process counters fit in u32"),
        ..Default::default()
    };
    // SAFETY: GetCurrentProcess returns a pseudo-handle for this process.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `process` names this process, and `counters` is a live writable
    // EX structure whose declared size matches the buffer.
    let ok = unsafe {
        GetProcessMemoryInfo(
            process,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(PeakMemory {
        commit:      Some(counters.PeakPagefileUsage),
        working_set: counters.PeakWorkingSetSize,
    })
}

#[cfg(unix)]
fn peak_memory() -> std::io::Result<PeakMemory> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `usage` points to a writable rusage buffer, and RUSAGE_SELF
    // selects this process. The buffer is initialized on success only.
    let ok = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if ok != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a successful getrusage initialized the entire result.
    let usage = unsafe { usage.assume_init() };
    let rss = usize::try_from(usage.ru_maxrss).expect("peak RSS is nonnegative");
    #[cfg(target_os = "macos")]
    let working_set = rss;
    #[cfg(not(target_os = "macos"))]
    let working_set = rss * 1024;
    // getrusage exposes peak resident memory but no per-process peak commit.
    Ok(PeakMemory {
        commit: None,
        working_set,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Mebibyte figures are only printed."
)]
fn mebibytes(bytes: usize) -> f64 {
    bytes as f64 / f64::from(1 << 20)
}

fn sample(input: BenchmarkInput, phases: &str) -> std::io::Result<PeakMemory> {
    // OS peaks include process startup and static source generation.
    _ = input.bytes();
    match phases {
        | "1-3" => _ = bcc_rust::lex(input),
        | "1-6" => _ = bcc_rust::preprocess(input),
        | "1-7" => _ = bcc_rust::parse(input),
        | _ => panic!("unknown benchmark phase"),
    }
    peak_memory()
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if args.len() == 4 && args[1] == "--sample" {
        let index: usize = args[2].parse().expect("sample index is an integer");
        let peak =
            sample(BenchmarkInput::ALL[index], &args[3]).expect("OS peak memory query succeeds");
        println!("{} {}", peak.commit.unwrap_or(0), peak.working_set);
        return ExitCode::SUCCESS;
    }
    assert!(
        args.len() == 1 || (args.len() == 2 && args[1] == "--bench"),
        "expected no arguments, --bench, or --sample"
    );
    println!("| input | phases | strategy | peak commit (MiB) | peak working set (MiB) |");
    println!("|---|---|---|---:|---:|");
    let executable = env::current_exe().expect("benchmark executable has a path");
    for (index, input) in BenchmarkInput::ALL.into_iter().enumerate() {
        for phases in ["1-3", "1-6", "1-7"] {
            let output = Command::new(&executable)
                .args(["--sample", &index.to_string(), phases])
                .output()
                .expect("sample process starts");
            assert!(
                output.status.success(),
                "sample failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).expect("sample output is UTF-8");
            let mut fields = text.split_whitespace();
            let commit: usize = fields
                .next()
                .expect("commit field")
                .parse()
                .expect("integer");
            let working_set: usize = fields
                .next()
                .expect("working set field")
                .parse()
                .expect("integer");
            assert!(fields.next().is_none(), "unexpected sample output");
            let commit = if cfg!(unix) {
                "n/a".to_owned()
            } else {
                format!("{:.1}", mebibytes(commit))
            };
            println!(
                "| {} | {phases} | batch | {commit} | {:.1} |",
                input.name(),
                mebibytes(working_set)
            );
        }
    }
    // Arena counters are exact and need no fresh process.
    println!();
    println!(
        "| input | PP arena high water (MiB) | expansion arena high water (KiB) | parse arena \
         high water (KiB) | peak regions | peak reserved (GiB) | peak arena commit (MiB) |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for input in BenchmarkInput::ALL {
        let usage = bcc_rust::arena_usage(input);
        println!(
            "| {} | {:.1} | {:.1} | {:.1} | {} | {:.0} | {:.1} |",
            input.name(),
            mebibytes(usage.preprocessor_high_water),
            mebibytes(usage.expansion_high_water) * 1024.0,
            mebibytes(usage.parse_high_water) * 1024.0,
            usage.peak_regions,
            mebibytes(usage.peak_reserved) / 1024.0,
            mebibytes(usage.peak_committed)
        );
    }
    ExitCode::SUCCESS
}
