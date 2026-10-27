//! POSIX file descriptors: `open`, `openat`, `creat`, `close`, `read`,
//! `write`, `lseek`, `dup`, `dup2`, `dup3`, `pipe`, `fstat`, `stat`,
//! `lstat`, `unlink` and `isatty`, as glibc's Linux wrappers behave,
//! `errno` included.
//!
//! The program's descriptor table (`posix/descriptors.rs`) starts with 0,
//! 1 and 2 on the console, which work without host file access; the host
//! file system is reached only with `--interpret-allow-fs`, and without it
//! `open`, `stat` and `unlink` fail with `EACCES`, as `fopen` does. A
//! descriptor on a host file holds the host's own descriptor, so offsets,
//! `O_APPEND` and duplicates behave as the host's. A pipe is a host pipe
//! kept non-blocking: the program is the only process, so a read of an
//! empty pipe whose write end is open, or a write to a full one, would
//! wait forever, and the interpreter stops it instead; a write to a pipe
//! without a reader, which raises `SIGPIPE`, stops it too.
//!
//! Streams and descriptors share the console's order and each other's
//! files (`stdio/descriptors.rs`): `write(1, ...)` appears before what
//! `stdout` still buffers, `fileno` and `fdopen` connect the two, and a
//! stream follows its descriptor number through `close` and `dup2`.
//! `fstat` of the console reports a pipe (a terminal under
//! `--interpret-terminal-stdout`) with the buffer size glibc gives it.
//!
//! POSIX.1-2008 (XSH): "open", "close", "read", "write", "lseek", "dup",
//! "pipe", "fstat", "stat", "unlink", "isatty"; `dup3` and the `__*_chk`
//! and `__*_2` entry points are glibc's.

// The descriptor table and `struct stat`
mod descriptors;
mod status;

use std::{
    fs::File,
    io::{
        self,
        IsTerminal as _,
        Read as _,
        Seek as _,
        SeekFrom,
        Write as _,
    },
};

pub(super) use descriptors::{
    Descriptor,
    Descriptors,
    MAX_DESCRIPTORS,
    Open,
    Sink,
};
use status::Status;

use super::{
    DefaultHost,
    HostCall,
    HostReturn,
    errno,
    integer,
};
use crate::{
    backend::interpreter::{
        arithmetic::sext,
        memory::Memory,
        trap::{
            Fault,
            UbKind,
            Unsupported,
        },
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
};

/// The functions this module runs.
pub(super) const FUNCTIONS: &[&str] = &[
    "open",
    "open64",
    "__open_2",
    "__open64_2",
    "openat",
    "openat64",
    "__openat_2",
    "__openat64_2",
    "creat",
    "creat64",
    "close",
    "read",
    "__read_chk",
    "write",
    "lseek",
    "lseek64",
    "dup",
    "dup2",
    "dup3",
    "pipe",
    "fstat",
    "fstat64",
    "__fxstat",
    "__fxstat64",
    "stat",
    "stat64",
    "lstat",
    "lstat64",
    "__xstat",
    "__xstat64",
    "__lxstat",
    "__lxstat64",
    "unlink",
    "isatty",
];

/// Linux's `open` flags on x86-64, which the program passes.
mod flags {
    pub(super) const ACCESS: i32 = 0o3;
    pub(super) const WRITE_ONLY: i32 = 0o1;
    pub(super) const READ_WRITE: i32 = 0o2;
    pub(super) const CREATE: i32 = 0o100;
    pub(super) const TRUNCATE: i32 = 0o1000;
    pub(super) const CLOSE_ON_EXEC: i32 = 0o2_000_000;
    /// `O_TMPFILE`, which includes `O_DIRECTORY`.
    pub(super) const TEMPORARY: i32 = 0o20_200_000;
}

/// `AT_FDCWD`.
const CURRENT_DIRECTORY: i128 = -100;

/// glibc's `_STAT_VER` on x86-64, which `__xstat` and its relatives check.
const STAT_VERSION: i128 = 1;

/// How much one host `read` asks for at a time.
const READ_CHUNK: usize = 1 << 16;

/// What a descriptor function gives back: its value, or -1 with the
/// `errno` it sets.
type Outcome = Result<i128, i32>;

impl DefaultHost<'_> {
    /// Runs a descriptor function, or `None` for another name or on a
    /// Windows target, whose C runtime has other conventions.
    pub(super) fn posix(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        let windows = matches!(
            self.target,
            crate::target::Target::WindowsGnu | crate::target::Target::WindowsMsvc
        );
        (!windows && FUNCTIONS.contains(&call.name)).then(|| self.run_posix(call, memory))
    }

    fn run_posix(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let int =
            |index: usize| -> Result<i128, Fault> { Ok(sext(Type::I32, integer(arg(index))?)) };
        let long = |index: usize| -> Result<i128, Fault> {
            Ok(sext(call.arg_types[index], integer(arg(index))?))
        };
        let outcome = match call.name {
            | "open" | "open64" | "__open_2" | "__open64_2" => {
                let flags = int(1)?;
                let Some(mode) = mode(call, 2, flags)? else {
                    return Ok(HostReturn::Abort);
                };
                self.open_at(CURRENT_DIRECTORY, arg(0), flags, mode, memory)?
            },
            | "openat" | "openat64" | "__openat_2" | "__openat64_2" => {
                let flags = int(2)?;
                let Some(mode) = mode(call, 3, flags)? else {
                    return Ok(HostReturn::Abort);
                };
                self.open_at(int(0)?, arg(1), flags, mode, memory)?
            },
            | "creat" | "creat64" => {
                let flags = flags::CREATE | flags::WRITE_ONLY | flags::TRUNCATE;
                self.open_at(CURRENT_DIRECTORY, arg(0), flags.into(), int(1)?, memory)?
            },
            | "close" => {
                let number = int(0)?;
                if self.descriptors.close(number) {
                    self.retarget_streams(number);
                    Ok(0)
                } else {
                    Err(errno::EBADF)
                }
            },
            | "read" | "__read_chk" => {
                let count = integer(arg(2))?;
                // glibc's fortified `read` fails a count beyond the buffer.
                if call.name == "__read_chk" && count > integer(arg(3))? {
                    return Ok(HostReturn::Abort);
                }
                self.read_descriptor(int(0)?, arg(1), count, memory)?
            },
            | "write" => self.write_descriptor(int(0)?, arg(1), integer(arg(2))?, memory)?,
            | "lseek" | "lseek64" => self.seek_descriptor(int(0)?, long(1)?, int(2)?),
            | "dup" => self.duplicate(int(0)?, None),
            | "dup2" => self.duplicate(int(0)?, Some(int(1)?)),
            | "dup3" => {
                let (old, new) = (int(0)?, int(1)?);
                if int(2)? & !i128::from(flags::CLOSE_ON_EXEC) != 0 || old == new {
                    Err(errno::EINVAL)
                } else {
                    self.duplicate(old, Some(new))
                }
            },
            | "pipe" => self.pipe(arg(0), memory)?,
            | "fstat" | "fstat64" => self.status_of_descriptor(int(0)?, arg(1), memory)?,
            | "__fxstat" | "__fxstat64" =>
                if int(0)? == STAT_VERSION {
                    self.status_of_descriptor(int(1)?, arg(2), memory)?
                } else {
                    Err(errno::EINVAL)
                },
            | "stat" | "stat64" | "lstat" | "lstat64" => {
                let follow = !call.name.starts_with('l');
                self.status_of_path(arg(0), arg(1), follow, memory)?
            },
            | "__xstat" | "__xstat64" | "__lxstat" | "__lxstat64" =>
                if int(0)? == STAT_VERSION {
                    let follow = !call.name.starts_with("__l");
                    self.status_of_path(arg(1), arg(2), follow, memory)?
                } else {
                    Err(errno::EINVAL)
                },
            | "unlink" => self.unlink(arg(0), memory)?,
            | "isatty" => {
                // `isatty` gives 0, not -1, with its `errno`.
                return match self.is_terminal(int(0)?) {
                    | Ok(terminal) => Ok(result(call, i128::from(terminal))),
                    | Err(code) => {
                        self.set_errno(memory, code)?;
                        Ok(result(call, 0))
                    },
                };
            },
            | _ => unreachable!("`FUNCTIONS` lists every descriptor function"),
        };
        let value = match outcome {
            | Ok(value) => value,
            | Err(code) => {
                self.set_errno(memory, code)?;
                -1
            },
        };
        Ok(result(call, value))
    }

    /// `openat` (and `open` at `AT_FDCWD`): the lowest free descriptor on
    /// the file `path` names relative to `directory`.
    fn open_at(
        &mut self,
        directory: i128,
        path: RuntimeValue,
        flags: i128,
        mode: i128,
        memory: &mut Memory<'_>,
    ) -> Result<Outcome, Fault> {
        self.path.clear();
        memory.read_c_string(path, &mut self.path)?;
        if !self.file_access {
            self.deny_file_access();
            return Ok(Err(errno::EACCES));
        }
        let flags = i32::try_from(flags).unwrap_or(0);
        let relative_to = if self.path.first() == Some(&b'/') || directory == CURRENT_DIRECTORY {
            None
        } else {
            match self.descriptors.get(directory).map(|entry| &entry.open) {
                | None => return Ok(Err(errno::EBADF)),
                | Some(Open::Console(_)) => return Ok(Err(errno::ENOTDIR)),
                | Some(Open::File(file)) => Some(file),
            }
        };
        let mode = u32::try_from(mode & 0o7777).expect("masked");
        let file = match host_open(relative_to, &mut self.path, flags, mode) {
            | Ok(file) => file,
            | Err(error) => return Ok(Err(errno::from_io(&error))),
        };
        let access = flags & flags::ACCESS;
        let descriptor = Descriptor {
            open:     Open::File(file),
            readable: access == 0 || access == flags::READ_WRITE,
            writable: access == flags::WRITE_ONLY || access == flags::READ_WRITE,
            pipe:     false,
        };
        Ok(self.insert_descriptor(descriptor))
    }

    /// Opens `descriptor` at the lowest free number, pointing streams left
    /// on that number at it.
    pub(super) fn insert_descriptor(&mut self, descriptor: Descriptor) -> Outcome {
        let number = self
            .descriptors
            .insert(descriptor)
            .map_err(|_| errno::EMFILE)?;
        self.retarget_streams(number.into());
        Ok(number.into())
    }

    /// `read`: up to `count` bytes from descriptor `number` into
    /// `buffer`, the whole request for a regular file.
    fn read_descriptor(
        &mut self,
        number: i128,
        buffer: RuntimeValue,
        count: u128,
        memory: &mut Memory<'_>,
    ) -> Result<Outcome, Fault> {
        let Some(descriptor) = self.descriptors.get(number) else {
            return Ok(Err(errno::EBADF));
        };
        if !descriptor.readable {
            return Ok(Err(errno::EBADF));
        }
        let pipe = descriptor.pipe;
        let Some(file) = self.descriptors.file(number) else {
            return Ok(Err(errno::EBADF));
        };
        let regular = file.metadata().is_ok_and(|metadata| metadata.is_file());
        let mut total = 0_u128;
        while total < count {
            let chunk =
                usize::try_from(count - total).map_or(READ_CHUNK, |rest| rest.min(READ_CHUNK));
            self.io_buffer.clear();
            self.io_buffer.resize(chunk, 0);
            let file = self.descriptors.file(number).expect("an open file");
            let read = match file.read(&mut self.io_buffer) {
                | Ok(read) => read,
                | Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                | Err(error) if error.kind() == io::ErrorKind::WouldBlock && pipe =>
                    return Err(Fault::Unsupported(Unsupported::PipeWait)),
                | Err(_) if total > 0 => break,
                | Err(error) => return Ok(Err(errno::from_io(&error))),
            };
            for (offset, &byte) in self.io_buffer[..read].iter().enumerate() {
                let at = memory.ptr_add(buffer, RuntimeValue::Int(total + offset as u128), true);
                memory.store(Type::I8, RuntimeValue::Int(byte.into()), at, Align::BYTE)?;
            }
            total += read as u128;
            if read < chunk || !regular {
                break;
            }
        }
        Ok(Ok(total.cast_signed()))
    }

    /// `write`: `count` bytes from `buffer` to descriptor `number`.
    fn write_descriptor(
        &mut self,
        number: i128,
        buffer: RuntimeValue,
        count: u128,
        memory: &Memory<'_>,
    ) -> Result<Outcome, Fault> {
        let Some(descriptor) = self.descriptors.get(number) else {
            return Ok(Err(errno::EBADF));
        };
        if !descriptor.writable {
            return Ok(Err(errno::EBADF));
        }
        let mut bytes = std::mem::replace(
            &mut self.io_buffer,
            crate::util::bump::ArenaVec::new_in(self.allocator),
        );
        bytes.clear();
        let loaded = (|| {
            for offset in 0..u64::try_from(count).map_err(|_| UbKind::OutOfBounds)? {
                let at = memory.ptr_add(buffer, RuntimeValue::Int(offset.into()), true);
                let byte = integer(memory.load(Type::I8, at, Align::BYTE)?)?;
                bytes.push(u8::try_from(byte & 0xFF).expect("masked"));
            }
            Ok::<(), Fault>(())
        })();
        let outcome = loaded.and_then(|()| {
            Ok(
                match self.descriptors.get(number).map(|entry| &entry.open) {
                    | Some(&Open::Console(sink)) => {
                        self.write_console(sink, &bytes);
                        Ok(bytes.len() as i128)
                    },
                    | _ => {
                        let file = self.descriptors.file(number).expect("an open file");
                        match write_all(file, &bytes) {
                            | Ok(()) => Ok(bytes.len() as i128),
                            | Err(errno::EAGAIN) =>
                                return Err(Fault::Unsupported(Unsupported::PipeWait)),
                            | Err(errno::EPIPE) =>
                                return Err(Fault::Unsupported(Unsupported::PipeSignal)),
                            | Err(code) => Err(code),
                        }
                    },
                },
            )
        });
        self.io_buffer = bytes;
        outcome
    }

    /// `lseek`: moves descriptor `number`'s offset; the console, like a
    /// pipe, cannot seek.
    fn seek_descriptor(&mut self, number: i128, offset: i128, whence: i128) -> Outcome {
        let Some(descriptor) = self.descriptors.get(number) else {
            return Err(errno::EBADF);
        };
        if matches!(descriptor.open, Open::Console(Sink::Output | Sink::Errors)) {
            return Err(errno::ESPIPE);
        }
        let offset = i64::try_from(offset).map_err(|_| errno::EINVAL)?;
        let position = match whence {
            | 0 => SeekFrom::Start(u64::try_from(offset).map_err(|_| errno::EINVAL)?),
            | 1 => SeekFrom::Current(offset),
            | 2 => SeekFrom::End(offset),
            | _ => return Err(errno::EINVAL),
        };
        let file = self.descriptors.file(number).ok_or(errno::EBADF)?;
        match file.seek(position) {
            | Ok(at) => Ok(at.into()),
            | Err(error) => Err(errno::from_io(&error)),
        }
    }

    /// `dup` (`new` is `None`) and `dup2`: a descriptor on `old`'s open file
    /// description, the lowest free one or `new`, closing what `new` was.
    fn duplicate(&mut self, old: i128, new: Option<i128>) -> Outcome {
        let duplicate = match self.descriptors.duplicate(old) {
            | None => return Err(errno::EBADF),
            | Some(Err(error)) => return Err(errno::from_io(&error)),
            | Some(Ok(duplicate)) => duplicate,
        };
        let Some(new) = new else {
            return self.insert_descriptor(duplicate);
        };
        let index = usize::try_from(new)
            .ok()
            .filter(|&index| index < MAX_DESCRIPTORS)
            .ok_or(errno::EBADF)?;
        if new != old {
            self.descriptors.place(index, duplicate);
            self.retarget_streams(new);
        }
        Ok(new)
    }

    /// `pipe`: a host pipe's read and write ends at the two lowest free
    /// descriptors, stored in `ends`.
    fn pipe(&mut self, ends: RuntimeValue, memory: &mut Memory<'_>) -> Result<Outcome, Fault> {
        let Some(read_end) = self.descriptors.lowest_free(0) else {
            return Ok(Err(errno::EMFILE));
        };
        if self.descriptors.lowest_free(read_end + 1).is_none() {
            return Ok(Err(errno::EMFILE));
        }
        let (reader, writer) = match host_pipe() {
            | Ok(ends) => ends,
            | Err(error) => return Ok(Err(errno::from_io(&error))),
        };
        let end = |file, readable| Descriptor {
            open: Open::File(file),
            readable,
            writable: !readable,
            pipe: true,
        };
        let numbers = [
            self.insert_descriptor(end(reader, true)),
            self.insert_descriptor(end(writer, false)),
        ];
        for (index, number) in numbers.into_iter().enumerate() {
            let number = number.expect("two free descriptors");
            let at = memory.ptr_add(ends, RuntimeValue::Int(index as u128 * 4), true);
            memory.store(
                Type::I32,
                RuntimeValue::int(Type::I32, number.cast_unsigned()),
                at,
                Align::BYTE,
            )?;
        }
        Ok(Ok(0))
    }

    /// `fstat`: the status of descriptor `number`.
    fn status_of_descriptor(
        &mut self,
        number: i128,
        buffer: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<Outcome, Fault> {
        let status = match self.descriptors.get(number).map(|entry| &entry.open) {
            | None => return Ok(Err(errno::EBADF)),
            | Some(Open::Console(Sink::Output)) =>
                Status::console(self.terminal_output, self.console.output.block),
            | Some(Open::Console(Sink::Errors)) => Status::console(false, self.console.error.block),
            | Some(_) => {
                let file = self.descriptors.file(number).ok_or(UbKind::InvalidStream)?;
                match file.metadata() {
                    | Ok(metadata) => Status::of(&metadata),
                    | Err(error) => return Ok(Err(errno::from_io(&error))),
                }
            },
        };
        status.store(memory, buffer)?;
        Ok(Ok(0))
    }

    /// `stat` (following a final symbolic link) or `lstat`.
    fn status_of_path(
        &mut self,
        path: RuntimeValue,
        buffer: RuntimeValue,
        follow: bool,
        memory: &mut Memory<'_>,
    ) -> Result<Outcome, Fault> {
        let allowed = self.file_access;
        let metadata = match self.path(path, memory)? {
            | _ if !allowed => Err(errno::EACCES),
            | None => Err(errno::ENOENT),
            | Some(path) => if follow {
                std::fs::metadata(path)
            } else {
                std::fs::symlink_metadata(path)
            }
            .map_err(|error| errno::from_io(&error)),
        };
        if !allowed {
            self.deny_file_access();
        }
        Ok(match metadata {
            | Ok(metadata) => {
                Status::of(&metadata).store(memory, buffer)?;
                Ok(0)
            },
            | Err(code) => Err(code),
        })
    }

    /// `unlink`: removes the name `path`; a directory fails with `EISDIR`.
    fn unlink(&mut self, path: RuntimeValue, memory: &Memory<'_>) -> Result<Outcome, Fault> {
        let allowed = self.file_access;
        let outcome = match self.path(path, memory)? {
            | _ if !allowed => Err(errno::EACCES),
            | None => Err(errno::ENOENT),
            | Some(path) => std::fs::remove_file(path)
                .map(|()| 0)
                .map_err(|error| errno::from_io(&error)),
        };
        if !allowed {
            self.deny_file_access();
        }
        Ok(outcome)
    }

    /// `isatty`: whether descriptor `number` is a terminal, or the `errno`
    /// glibc's `tcgetattr` reports (`ENOTTY`, `EBADF`).
    fn is_terminal(&mut self, number: i128) -> Result<bool, i32> {
        let terminal = match self.descriptors.get(number).map(|entry| &entry.open) {
            | None => return Err(errno::EBADF),
            | Some(Open::Console(Sink::Output)) => self.terminal_output,
            | Some(Open::Console(Sink::Errors)) => false,
            | Some(Open::Console(Sink::Input)) => self.terminal_input || io::stdin().is_terminal(),
            | Some(Open::File(file)) => file.is_terminal(),
        };
        if terminal {
            Ok(true)
        } else {
            Err(errno::ENOTTY)
        }
    }
}

/// The call's result of `value`.
fn result(call: HostCall<'_>, value: i128) -> HostReturn {
    HostReturn::Value(
        call.result
            .map(|ty| RuntimeValue::int(ty, value.cast_unsigned())),
    )
}

/// The permission bits an `open` passes after its flags at `index`,
/// which glibc reads only for `O_CREAT` and `O_TMPFILE`; `None` for a
/// fortified `__open_2` or `__openat_2` that needs them and has none,
/// which glibc aborts.
fn mode(call: HostCall<'_>, index: usize, flags: i128) -> Result<Option<i128>, Fault> {
    let flags = i32::try_from(flags).unwrap_or(0);
    let needs_mode = flags & flags::CREATE != 0 || flags & flags::TEMPORARY == flags::TEMPORARY;
    if !needs_mode {
        return Ok(Some(0));
    }
    if call.name.ends_with("_2") {
        return Ok(None);
    }
    // The mode is a variadic `mode_t`, promoted to `int`.
    let mode = call
        .args
        .get(index)
        .copied()
        .ok_or(UbKind::SignatureMismatch)?;
    Ok(Some(sext(Type::I32, integer(mode)?)))
}

/// Writes all of `bytes` to `file`, or the `errno` of the write that
/// failed.
pub(super) fn write_all(file: &mut File, mut bytes: &[u8]) -> Result<(), i32> {
    while !bytes.is_empty() {
        match file.write(bytes) {
            | Ok(0) => return Err(errno::EIO),
            | Ok(written) => bytes = &bytes[written..],
            | Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            | Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(errno::EAGAIN),
            | Err(error) => return Err(errno::from_io(&error)),
        }
    }
    Ok(())
}

/// Sets `O_APPEND` on the host file, as `fdopen` with an appending mode
/// does.
pub(super) fn set_append(file: &File) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd as _;
        let descriptor = file.as_raw_fd();
        // SAFETY: `descriptor` is open for as long as `file` lives;
        // `F_GETFL` only reads its status flags.
        let status = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if status >= 0 {
            // SAFETY: as above; `F_SETFL` only sets them.
            _ = unsafe { libc::fcntl(descriptor, libc::F_SETFL, status | libc::O_APPEND) };
        }
    }
    #[cfg(not(unix))]
    {
        _ = file;
    }
}

/// Opens `path` (null-terminated here) on the host with the program's
/// Linux `flags` and `mode`, relative to `directory` when given, never
/// inherited by child processes.
#[cfg(unix)]
fn host_open(
    directory: Option<&File>,
    path: &mut crate::util::bump::ArenaVec<'_, u8>,
    flags: i32,
    mode: u32,
) -> io::Result<File> {
    use std::os::fd::{
        AsRawFd as _,
        FromRawFd as _,
    };
    let directory = directory.map_or(libc::AT_FDCWD, File::as_raw_fd);
    if path.contains(&0) {
        return Err(io::Error::from_raw_os_error(errno::EINVAL));
    }
    path.push(0);
    let flags = host_flags(flags) | libc::O_CLOEXEC;
    // SAFETY: `path` is null-terminated and outlives the call; `openat`
    // returns a new descriptor that the `File` then owns.
    let descriptor = unsafe {
        libc::openat(
            directory,
            path.as_ptr().cast(),
            flags,
            libc::c_uint::from(mode),
        )
    };
    _ = path.pop();
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `descriptor` is open and owned by nothing else.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

/// The host's `open` flags for the program's: the same bits on Linux, and
/// the portable ones translated elsewhere.
#[cfg(unix)]
fn host_flags(flags: i32) -> i32 {
    if cfg!(target_os = "linux") {
        return flags;
    }
    let mut host = match flags & flags::ACCESS {
        | flags::WRITE_ONLY => libc::O_WRONLY,
        | flags::READ_WRITE => libc::O_RDWR,
        | _ => libc::O_RDONLY,
    };
    for (linux, native) in [
        (0o100, libc::O_CREAT),
        (0o200, libc::O_EXCL),
        (0o400, libc::O_NOCTTY),
        (0o1000, libc::O_TRUNC),
        (0o2000, libc::O_APPEND),
        (0o4000, libc::O_NONBLOCK),
        (0o200_000, libc::O_DIRECTORY),
        (0o400_000, libc::O_NOFOLLOW),
    ] {
        if flags & linux != 0 {
            host |= native;
        }
    }
    host
}

/// Without POSIX `openat`, the host opens with its portable options.
#[cfg(not(unix))]
fn host_open(
    directory: Option<&File>,
    path: &mut crate::util::bump::ArenaVec<'_, u8>,
    flags: i32,
    _mode: u32,
) -> io::Result<File> {
    if directory.is_some() {
        return Err(io::ErrorKind::Unsupported.into());
    }
    let path = std::str::from_utf8(path).map_err(|_| io::ErrorKind::NotFound)?;
    let access = flags & flags::ACCESS;
    std::fs::OpenOptions::new()
        .read(access != flags::WRITE_ONLY)
        .write(access != 0)
        .append(flags & 0o2000 != 0)
        .truncate(flags & flags::TRUNCATE != 0)
        .create(flags & flags::CREATE != 0 && flags & 0o200 == 0)
        .create_new(flags & flags::CREATE != 0 && flags & 0o200 != 0)
        .open(path)
}

/// A host pipe's read and write ends, which never block.
#[cfg(unix)]
fn host_pipe() -> io::Result<(File, File)> {
    use std::os::fd::{
        AsRawFd as _,
        OwnedFd,
    };
    let (reader, writer) = io::pipe()?;
    let (reader, writer) = (
        File::from(OwnedFd::from(reader)),
        File::from(OwnedFd::from(writer)),
    );
    for end in [&reader, &writer] {
        // SAFETY: the descriptor is open while `end` lives; `F_GETFL`
        // only reads its status flags.
        let status = unsafe { libc::fcntl(end.as_raw_fd(), libc::F_GETFL) };
        if status < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above; `F_SETFL` only sets them.
        let set = unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFL, status | libc::O_NONBLOCK) };
        if set < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((reader, writer))
}

/// Without non-blocking pipes, `pipe` is unavailable.
#[cfg(not(unix))]
fn host_pipe() -> io::Result<(File, File)> {
    Err(io::Error::from_raw_os_error(errno::ENOSYS))
}
