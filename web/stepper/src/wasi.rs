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
            | _ => return None,
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
            | Self::SchedYield => (&[], &[I32]),
        };
        FuncType {
            params:  params.to_vec(),
            results: results.to_vec(),
        }
    }
}

/// Whether the host provides `module.name` with type `ty`.
pub fn provides(module: &str, name: &str, ty: Option<&FuncType>) -> bool {
    WasiFunction::resolve(module, name).is_some_and(|function| ty == Some(&function.ty()))
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
                let bytes: Vec<u8> = (0..length).map(|_| self.next_random()).collect();
                memory.write(at, &bytes)?;
                (
                    SUCCESS,
                    format!("filled {length} bytes from a fixed-seed generator"),
                )
            },
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
        let mut bytes = Vec::new();
        for index in 0..count {
            let at = iovs + index * 8;
            let (base, length) = (memory.u32_at(at)?, memory.u32_at(at + 4)?);
            bytes.extend_from_slice(memory.read(base, length)?);
        }
        let total = bytes.len();
        memory.set_u32(written_at, total as u32)?;
        self.append(stream, bytes);
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

    fn append(&mut self, stream: Stream, mut bytes: Vec<u8>) {
        let kept: usize = self.output.iter().map(|(_, bytes)| bytes.len()).sum();
        let room = OUTPUT_LIMIT.saturating_sub(kept);
        if bytes.len() > room {
            self.dropped += (bytes.len() - room) as u64;
            bytes.truncate(room);
        }
        match self.output.last_mut() {
            | Some((last, previous)) if *last == stream => previous.extend(bytes),
            | _ if bytes.is_empty() => {},
            | _ => self.output.push((stream, bytes)),
        }
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
