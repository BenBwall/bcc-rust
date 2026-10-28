//! The host: the subset of WASI preview 1 (`wasi_snapshot_preview1`) a C
//! program built for `wasm32-wasip1` needs to print and exit.
//!
//! Output to standard output and standard error is kept in order for the
//! page; standard input is empty. The program sees one argument (its name)
//! and no environment. Time and randomness are simulated from the step
//! count and a fixed seed, so every run of a program is the same. Anything
//! else answers `ENOSYS`, or `EBADF` for file descriptors other than 0-2,
//! which is how wasi-libc learns there are no preopened directories.

use crate::{
    exec::{
        Memory,
        Trap,
        TrapKind,
    },
    module::{
        FuncType,
        ValType,
    },
    value::Value,
};

/// The import module name WASI preview 1 functions come from.
pub const MODULE: &str = "wasi_snapshot_preview1";

/// The program name `args_get` reports.
const PROGRAM: &str = "program";
/// How much output the host keeps (1 MiB); more is counted but dropped.
const OUTPUT_LIMIT: usize = 1 << 20;

/// WASI preview 1 error numbers (`errno`).
const SUCCESS: i32 = 0;
const EBADF: i32 = 8;
const EINVAL: i32 = 28;
const ENOSYS: i32 = 52;
const ESPIPE: i32 = 70;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasiFunction {
    FdWrite,
    FdRead,
    FdClose,
    FdSeek,
    FdFdstatGet,
    FdPrestatGet,
    FdPrestatDirName,
    ProcExit,
    ArgsSizesGet,
    ArgsGet,
    EnvironSizesGet,
    EnvironGet,
    ClockTimeGet,
    ClockResGet,
    RandomGet,
    SchedYield,
    /// Any other preview 1 function that returns an error number: it
    /// answers `ENOSYS`, as runtimes do for what they do not implement.
    Unsupported,
}

impl WasiFunction {
    /// The WASI function `module.name` names.
    pub fn resolve(module: &str, name: &str) -> Option<Self> {
        if module != MODULE {
            return None;
        }
        Some(match name {
            | "fd_write" => Self::FdWrite,
            | "fd_read" => Self::FdRead,
            | "fd_close" => Self::FdClose,
            | "fd_seek" => Self::FdSeek,
            | "fd_fdstat_get" => Self::FdFdstatGet,
            | "fd_prestat_get" => Self::FdPrestatGet,
            | "fd_prestat_dir_name" => Self::FdPrestatDirName,
            | "proc_exit" => Self::ProcExit,
            | "args_sizes_get" => Self::ArgsSizesGet,
            | "args_get" => Self::ArgsGet,
            | "environ_sizes_get" => Self::EnvironSizesGet,
            | "environ_get" => Self::EnvironGet,
            | "clock_time_get" => Self::ClockTimeGet,
            | "clock_res_get" => Self::ClockResGet,
            | "random_get" => Self::RandomGet,
            | "sched_yield" => Self::SchedYield,
            | _ => Self::Unsupported,
        })
    }

    /// The function's WASI signature; an import of another type is left
    /// unresolved.
    pub fn ty(self) -> FuncType {
        use ValType::{
            I32,
            I64,
        };
        let (params, results): (&[ValType], &[ValType]) = match self {
            | Self::FdWrite | Self::FdRead => (&[I32, I32, I32, I32], &[I32]),
            | Self::FdClose => (&[I32], &[I32]),
            | Self::FdSeek => (&[I32, I64, I32, I32], &[I32]),
            | Self::FdFdstatGet
            | Self::FdPrestatGet
            | Self::ArgsSizesGet
            | Self::ArgsGet
            | Self::EnvironSizesGet
            | Self::EnvironGet
            | Self::ClockResGet
            | Self::RandomGet => (&[I32, I32], &[I32]),
            | Self::FdPrestatDirName => (&[I32, I32, I32], &[I32]),
            | Self::ProcExit => (&[I32], &[]),
            | Self::ClockTimeGet => (&[I32, I64, I32], &[I32]),
            // An unsupported function's parameters are whatever it imports
            // (see `provides`); only its error number is fixed.
            | Self::SchedYield | Self::Unsupported => (&[], &[I32]),
        };
        FuncType {
            params:  params.to_vec(),
            results: results.to_vec(),
        }
    }
}

/// Whether the host provides `module.name` with type `ty`.
pub fn provides(module: &str, name: &str, ty: Option<&FuncType>) -> bool {
    let Some(ty) = ty else {
        return false;
    };
    match WasiFunction::resolve(module, name) {
        | Some(WasiFunction::Unsupported) => ty.results == [ValType::I32],
        | Some(function) => *ty == function.ty(),
        | None => false,
    }
}

/// Which stream a piece of output went to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// The host's state: output so far and the simulated random generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// Output in the order it was written; consecutive writes to one stream
    /// are merged.
    pub output:  Vec<(Stream, Vec<u8>)>,
    /// Bytes written past [`OUTPUT_LIMIT`].
    pub dropped: u64,
    random:      u64,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            output:  Vec::new(),
            dropped: 0,
            random:  0x853C_49E6_748F_EA9B,
        }
    }
}

/// What a host call did.
pub enum HostOutcome {
    /// It returned `results`; `note` says what it did, for the narration.
    Return {
        results: Vec<Value>,
        note:    String,
    },
    /// `proc_exit` ended the program.
    Exit(i32),
}

impl Host {
    /// Call `function` with `args`. Pointers it is given are checked like
    /// any memory access.
    ///
    /// # Errors
    /// A bad pointer traps, as an out-of-bounds access would.
    #[expect(clippy::too_many_lines, reason = "One arm per WASI function.")]
    pub fn call(
        &mut self,
        function: WasiFunction,
        args: &[Value],
        memory: Option<&mut Memory>,
        steps: u64,
    ) -> Result<HostOutcome, Trap> {
        let int = |index: usize| match args.get(index) {
            | Some(Value::I32(value)) => *value as u32,
            | _ => 0,
        };
        if function == WasiFunction::ProcExit {
            return Ok(HostOutcome::Exit(int(0) as i32));
        }
        if function == WasiFunction::SchedYield {
            return Ok(done(SUCCESS, "yielded (there is nothing else to run)"));
        }
        let memory = memory.ok_or_else(|| {
            Trap::new(
                TrapKind::Host,
                "a WASI call needs the module to have a memory",
            )
        })?;
        let (errno, note) = match function {
            | WasiFunction::FdWrite => self.fd_write(memory, int(0), int(1), int(2), int(3))?,
            | WasiFunction::FdRead =>
                if int(0) == 0 {
                    memory.set_u32(int(3), 0)?;
                    (SUCCESS, "read standard input, which is empty".to_owned())
                } else {
                    (EBADF, format!("refused to read file descriptor {}", int(0)))
                },
            | WasiFunction::FdClose => standard(int(0), "closed"),
            | WasiFunction::FdSeek =>
                if int(0) <= 2 {
                    (
                        ESPIPE,
                        "refused to seek: the standard streams are not files".to_owned(),
                    )
                } else {
                    (EBADF, format!("refused to seek file descriptor {}", int(0)))
                },
            | WasiFunction::FdFdstatGet => {
                let fd = int(0);
                if fd <= 2 {
                    // filetype 2 is a character device; all rights.
                    let at = int(1);
                    memory.write(at, &[2, 0, 0, 0, 0, 0, 0, 0])?;
                    memory.set_u64(at + 8, u64::MAX)?;
                    memory.set_u64(at + 16, u64::MAX)?;
                    (
                        SUCCESS,
                        format!("described file descriptor {fd} as a terminal"),
                    )
                } else {
                    (EBADF, format!("file descriptor {fd} is not open"))
                }
            },
            | WasiFunction::FdPrestatGet | WasiFunction::FdPrestatDirName =>
                (EBADF, format!("there is no preopened directory {}", int(0))),
            | WasiFunction::ArgsSizesGet => {
                memory.set_u32(int(0), 1)?;
                memory.set_u32(int(1), PROGRAM.len() as u32 + 1)?;
                (SUCCESS, format!("reported one argument, \"{PROGRAM}\""))
            },
            | WasiFunction::ArgsGet => {
                let (pointers, buffer) = (int(0), int(1));
                memory.set_u32(pointers, buffer)?;
                let mut name = PROGRAM.as_bytes().to_vec();
                name.push(0);
                memory.write(buffer, &name)?;
                (SUCCESS, format!("wrote the argument \"{PROGRAM}\""))
            },
            | WasiFunction::EnvironSizesGet => {
                memory.set_u32(int(0), 0)?;
                memory.set_u32(int(1), 0)?;
                (SUCCESS, "reported an empty environment".to_owned())
            },
            | WasiFunction::EnvironGet => (SUCCESS, "wrote no environment variables".to_owned()),
            | WasiFunction::ClockTimeGet => {
                // One simulated microsecond per instruction.
                memory.set_u64(int(2), steps.saturating_mul(1_000))?;
                (
                    SUCCESS,
                    format!(
                        "read the clock: {steps} instructions so far, one simulated microsecond \
                         each"
                    ),
                )
            },
            | WasiFunction::ClockResGet => {
                memory.set_u64(int(1), 1_000)?;
                (SUCCESS, "reported a one-microsecond clock".to_owned())
            },
            | WasiFunction::RandomGet => {
                let (at, length) = (int(0), int(1));
                // Bounds first, then fill in place: the length is the
                // program's to choose.
                for byte in memory.slice_mut(at, length)? {
                    *byte = self.next_random();
                }
                (
                    SUCCESS,
                    format!("filled {length} bytes from a fixed-seed generator"),
                )
            },
            | WasiFunction::Unsupported => (
                ENOSYS,
                "answered that it is not implemented: the stepper provides only a little of WASI"
                    .to_owned(),
            ),
            | WasiFunction::ProcExit | WasiFunction::SchedYield =>
                (ENOSYS, "unreachable".to_owned()),
        };
        Ok(done(errno, &note))
    }

    fn fd_write(
        &mut self,
        memory: &mut Memory,
        fd: u32,
        iovs: u32,
        count: u32,
        written_at: u32,
    ) -> Result<(i32, String), Trap> {
        let stream = match fd {
            | 1 => Stream::Stdout,
            | 2 => Stream::Stderr,
            | _ => return Ok((EBADF, format!("refused to write to file descriptor {fd}"))),
        };
        if count > 1024 {
            return Ok((
                EINVAL,
                "refused an fd_write with too many buffers".to_owned(),
            ));
        }
        // Check every buffer before writing any, and copy only what the
        // output limit keeps, so a large write costs no more than that.
        let mut buffers = Vec::with_capacity(count as usize);
        let mut total = 0_u64;
        for index in 0..count {
            let at = iovs.checked_add(index * 8).ok_or_else(|| {
                Trap::new(
                    TrapKind::MemoryOutOfBounds,
                    "out of bounds memory access: fd_write's buffer list runs past 4 GiB",
                )
            })?;
            let (base, length) = (memory.u32_at(at)?, memory.u32_at(at.saturating_add(4))?);
            memory.read(base, length)?;
            buffers.push((base, length));
            total += u64::from(length);
        }
        let Ok(total) = u32::try_from(total) else {
            return Ok((EINVAL, "refused an fd_write of more than 4 GiB".to_owned()));
        };
        memory.set_u32(written_at, total)?;
        for (base, length) in buffers {
            self.append(stream, memory.read(base, length)?);
        }
        let name = if stream == Stream::Stdout {
            "standard output"
        } else {
            "standard error"
        };
        Ok((
            SUCCESS,
            format!(
                "wrote {total} byte{} to {name}",
                if total == 1 { "" } else { "s" }
            ),
        ))
    }

    /// Keep `bytes` as output, up to the limit; count the rest as dropped.
    fn append(&mut self, stream: Stream, bytes: &[u8]) {
        let kept: usize = self.output.iter().map(|(_, bytes)| bytes.len()).sum();
        let room = OUTPUT_LIMIT.saturating_sub(kept);
        let (kept, dropped) = bytes.split_at(bytes.len().min(room));
        self.dropped += dropped.len() as u64;
        if kept.is_empty() {
            return;
        }
        match self.output.last_mut() {
            | Some((last, previous)) if *last == stream => previous.extend_from_slice(kept),
            | _ => self.output.push((stream, kept.to_vec())),
        }
    }

    /// What a copy of the host's state costs, for checkpoints.
    pub fn retained_bytes(&self) -> usize {
        self.output.iter().map(|(_, bytes)| bytes.len()).sum()
    }

    /// xorshift64*: deterministic, not secure, and only for `random_get`.
    fn next_random(&mut self) -> u8 {
        let mut x = self.random;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.random = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
    }
}

fn standard(fd: u32, verb: &str) -> (i32, String) {
    if fd <= 2 {
        (SUCCESS, format!("{verb} file descriptor {fd}"))
    } else {
        (EBADF, format!("file descriptor {fd} is not open"))
    }
}

fn done(errno: i32, note: &str) -> HostOutcome {
    HostOutcome::Return {
        results: vec![Value::I32(errno)],
        note:    if errno == SUCCESS {
            note.to_owned()
        } else {
            format!("{note} (error {errno})")
        },
    }
}
