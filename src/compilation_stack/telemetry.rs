//! Untimed OS snapshots. Linux smaps identifies the actual custom stack range;
//! reservation size is kept distinct from residency, swap and writable pages.

use super::Bounds;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Snapshot {
    pub(crate) resident_bytes: usize,
    pub(crate) swap_bytes:     usize,
    pub(crate) writable_bytes: usize,
    pub(crate) noreserve:      bool,
    pub(crate) huge_bytes:     usize,
}

impl Bounds {
    #[cfg_attr(
        target_os = "linux",
        expect(
            clippy::disallowed_methods,
            reason = "Untimed study stack telemetry reads procfs outside compiler measurement."
        )
    )]
    pub(crate) fn snapshot(self) -> Snapshot {
        #[cfg(target_os = "linux")]
        {
            let text = std::fs::read_to_string("/proc/self/smaps").expect("read stack smaps");
            let mut snapshot = Snapshot::default();
            let mut inside = false;
            for line in text.lines() {
                let mut fields = line.split_whitespace();
                let Some(first) = fields.next() else { continue };
                if let Some((lo, hi)) = first.split_once('-') {
                    let lo = usize::from_str_radix(lo, 16).expect("smaps lower address");
                    let hi = usize::from_str_radix(hi, 16).expect("smaps upper address");
                    inside = lo >= self.base && hi <= self.base + self.bytes;
                    if inside
                        && fields
                            .next()
                            .expect("mapping permissions")
                            .starts_with("rw")
                    {
                        snapshot.writable_bytes += hi - lo;
                    }
                } else if inside {
                    match first {
                        | "Rss:" =>
                            snapshot.resident_bytes += fields
                                .next()
                                .expect("RSS value")
                                .parse::<usize>()
                                .expect("RSS integer")
                                * 1024,
                        | "Swap:" =>
                            snapshot.swap_bytes += fields
                                .next()
                                .expect("swap value")
                                .parse::<usize>()
                                .expect("swap integer")
                                * 1024,
                        | "VmFlags:" => snapshot.noreserve |= fields.any(|flag| flag == "nr"),
                        | "AnonHugePages:" =>
                            snapshot.huge_bytes += fields
                                .next()
                                .expect("huge page value")
                                .parse::<usize>()
                                .expect("huge page integer")
                                * 1024,
                        | _ => {},
                    }
                }
            }
            snapshot
        }
        #[cfg(windows)]
        {
            super::windows::snapshot(self)
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = self;
            Snapshot::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Faults {
    pub(crate) minor: usize,
    pub(crate) major: usize,
}

impl Faults {
    pub(crate) fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
            // SAFETY: getrusage initializes this properly aligned output.
            if unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) } == 0 {
                // SAFETY: the successful call initialized the rusage.
                let usage = unsafe { usage.assume_init() };
                return Self {
                    minor: usize::try_from(usage.ru_minflt).unwrap_or(0),
                    major: usize::try_from(usage.ru_majflt).unwrap_or(0),
                };
            }
        }
        Self::default()
    }
}
