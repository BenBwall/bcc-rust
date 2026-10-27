//! The `<stdio.h>` streams: `stdin`, `stdout` and `stderr`, and files the
//! program opens on the host's file system.
//!
//! A `FILE *` points to a [`ObjectKind::Stream`] object, whose number indexes
//! the host's stream table; the program cannot read or write the object
//! itself. `stdout` writes to the host's captured output and `stderr` to its
//! captured error output, unless `freopen` redirects them; `stdin` reads the
//! process's standard input. A stream used after `fclose`, or after a failed
//! `freopen` closed it, is undefined, including the standard streams that
//! `printf` or `getchar` use implicitly.
//!
//! The captured output keeps glibc's order for a pipe: standard error is
//! unbuffered, while standard output reaches the command line only at
//! `fflush(stdout)`, `fflush(NULL)` or the end of the run (a full buffer's
//! flush is not modelled). [`Console`] records where each error write falls
//! among those flushes.
//!
//! `stdin` reads through a buffer as glibc's does, one `read` of the
//! process's standard input at a time ([`Input`]). glibc line-buffers a
//! terminal's standard input and output, and flushes a line-buffered
//! `stdout` only when a line-buffered input stream has to be refilled
//! (`_IO_new_file_underflow` in `libio/fileops.c`): reading what the buffer
//! still holds, or reading a pipe or file, flushes nothing.
//!
//! Host files are opt-in (`--interpret-allow-fs`): without it, `fopen`,
//! `freopen` and `remove` fail as glibc does on `EACCES`, returning null or
//! -1, and the host counts the refusal for the command line to note;
//! `tmpnam` then names files without looking at the file system. Files are
//! unbuffered, so every write reaches the file at once and a later `fopen` of
//! the same name reads it back. `errno` is not modelled: a failing call only
//! returns its failure value.
//!
//! C99: §7.19.3, pp. 264-266; PDF pp. 276-278 (files); §7.19.4-§7.19.10,
//! pp. 268-304; PDF pp. 280-316 (operations, access, I/O, positioning and
//! errors).

use std::{
    fs::{
        File,
        OpenOptions,
    },
    io::{
        Read as _,
        Seek as _,
        SeekFrom,
        Write as _,
    },
};

use super::{
    DefaultHost,
    FormatArguments,
    HostCall,
    HostReturn,
    integer,
    va_list_operand,
};
use crate::{
    backend::interpreter::{
        arithmetic::sext,
        memory::{
            Memory,
            ObjectKind,
        },
        trap::{
            Fault,
            UbKind,
        },
        value::{
            Pointer,
            RuntimeValue,
        },
    },
    ir::{
        Align,
        Type,
    },
};

/// The functions this module runs.
pub(super) const FUNCTIONS: &[&str] = &[
    "fopen",
    "freopen",
    "fclose",
    "fflush",
    "fputc",
    "putc",
    "fputs",
    "fwrite",
    "fread",
    "fgetc",
    "getc",
    "getchar",
    "fgets",
    "ungetc",
    "feof",
    "ferror",
    "clearerr",
    "fseek",
    "ftell",
    "rewind",
    "remove",
    "tmpnam",
    "fprintf",
    "vfprintf",
    "vprintf",
    "vsprintf",
    "vsnprintf",
    "__printf_chk",
    "__fprintf_chk",
    "__vfprintf_chk",
    "__acrt_iob_func",
];

/// `EOF`.
const EOF: u128 = (-1_i32).cast_unsigned() as u128;

/// glibc's `L_tmpnam`, the size of `tmpnam`'s static buffer.
const TMPNAM_SIZE: u64 = 20;

/// The names `tmpnam` hands out, `bcc000000` to `bcc999999`, each once;
/// after them it returns null. C99 §7.19.4.4p3 leaves the behaviour after
/// `TMP_MAX` calls to the implementation.
const TMPNAM_NAMES: u32 = 1_000_000;

/// The open streams the host can track at once, the standard three
/// included.
const MAX_STREAMS: usize = 64;

/// glibc's `BUFSIZ`, the size of its buffer for a `stdin` that is not a
/// terminal.
const BUFSIZ: usize = 8192;

/// The size of glibc's buffer for a terminal's `stdin`: the device's
/// `st_blksize` (1024 on Linux), which glibc takes when it is below
/// `BUFSIZ`. A refill reads at most this much, so a longer line takes
/// several refills.
const TERMINAL_BUFFER: usize = 1024;

/// What the program wrote to the standard streams, in the order the
/// command line shows it.
pub(super) struct Console<'a> {
    /// Everything written to `stderr`.
    pub(super) errors:        super::ArenaVec<'a, u8>,
    /// For each write to `stderr`: how much of the host's output buffer had
    /// been flushed before it, and the end of its bytes in `errors`.
    pub(super) marks:         super::ArenaVec<'a, (usize, usize)>,
    /// How much of the output buffer `fflush` has released.
    pub(super) flushed:       usize,
    /// Whether standard output is a terminal, which glibc line-buffers: each
    /// newline releases what precedes it, and refilling a terminal's `stdin`
    /// releases all.
    pub(super) line_buffered: bool,
    /// How much of the output buffer [`Console::released`] has searched for
    /// newlines, and the end of the last one it found, so that a run looks
    /// at each byte once however often it writes to `stderr`.
    pub(super) scanned:       usize,
    pub(super) line_end:      usize,
}

impl Console<'_> {
    /// How much of `output` has reached the terminal or pipe by now.
    fn released(&mut self, output: &[u8]) -> usize {
        if self.line_buffered {
            // Formatted text moved out of the buffer leaves it shorter; it is
            // never shorter than what was written before, but search it again
            // rather than trust a stale position.
            if output.len() < self.scanned {
                (self.scanned, self.line_end) = (0, 0);
            }
            if let Some(at) = output[self.scanned..]
                .iter()
                .rposition(|&byte| byte == b'\n')
            {
                self.line_end = self.scanned + at + 1;
            }
            self.scanned = output.len();
        }
        self.flushed.max(self.line_end).min(output.len())
    }
}

/// glibc's buffer for `stdin`: what the last `read` of the process's
/// standard input returned, and how much of it the program has taken.
pub(super) struct Input<'a> {
    pub(super) bytes:    super::ArenaVec<'a, u8>,
    pub(super) taken:    usize,
    /// Whether standard input is a terminal, which glibc line-buffers.
    pub(super) terminal: bool,
}

impl Input<'_> {
    /// The next byte of standard input, or `None` at its end. An empty buffer
    /// is refilled by one `read`, which on a terminal returns at most a line;
    /// refilling a terminal's buffer first releases a line-buffered `stdout`,
    /// whose buffer holds `written` bytes.
    fn next(&mut self, console: &mut Console<'_>, written: usize) -> std::io::Result<Option<u8>> {
        if self.taken == self.bytes.len() {
            if self.terminal && console.line_buffered {
                console.flushed = written;
            }
            let size = if self.terminal {
                TERMINAL_BUFFER
            } else {
                BUFSIZ
            };
            self.bytes.clear();
            self.bytes.resize(size, 0);
            self.taken = 0;
            match std::io::stdin().lock().read(&mut self.bytes) {
                | Ok(count) => self.bytes.truncate(count),
                | Err(error) => {
                    self.bytes.clear();
                    return Err(error);
                },
            }
        }
        let byte = self.bytes.get(self.taken).copied();
        self.taken += usize::from(byte.is_some());
        Ok(byte)
    }
}

/// The host's stream table.
pub(super) struct Streams {
    entries: [Stream; MAX_STREAMS],
    /// Names `tmpnam` has handed out.
    names:   u32,
}

/// One stream's destination and state.
#[derive(Default)]
struct Stream {
    target:       Target,
    /// The end-of-file and error indicators.
    eof:          bool,
    error:        bool,
    /// A byte `ungetc` pushed back.
    pushed:       Option<u8>,
    /// The byte glibc's buffer holds just before the read position: the
    /// last one read, unless it came from a separate pushback area or a
    /// seek or write came after it. After a seek glibc's buffer can hold
    /// the byte before the new position too (when that is not on a block
    /// boundary), which the host does not model.
    behind:       Option<u8>,
    /// Whether `pushed` is the byte `behind`, which glibc's `ungetc` pushes
    /// back by stepping its read position back over it instead of into a
    /// separate pushback area. A failed seek discards the area but keeps the
    /// step back (`_IO_seekoff_unlocked` in `libio/ioseekoff.c`).
    stepped_back: bool,
    /// The memory object of a stream `fopen` made, freed by `fclose`.
    object:       Option<Pointer>,
}

/// Where a stream's bytes go or come from.
#[derive(Default)]
enum Target {
    #[default]
    Closed,
    /// The host's captured standard output.
    Output,
    /// The host's captured standard error.
    Errors,
    /// The process's standard input.
    Input,
    File(File),
}

impl Default for Streams {
    fn default() -> Self {
        let mut entries: [Stream; MAX_STREAMS] = std::array::from_fn(|_| Stream::default());
        entries[0].target = Target::Input;
        entries[1].target = Target::Output;
        entries[2].target = Target::Errors;
        Self { entries, names: 0 }
    }
}

/// The standard stream the C library object `name` holds.
pub(super) fn standard_stream(name: &str) -> Option<u32> {
    match name {
        | "stdin" => Some(0),
        | "stdout" => Some(1),
        | "stderr" => Some(2),
        | _ => None,
    }
}

impl DefaultHost<'_> {
    /// Runs a stdio function, or `None` for another name.
    pub(super) fn stdio(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        FUNCTIONS
            .contains(&call.name)
            .then(|| self.run_stdio(call, memory))
    }

    fn run_stdio(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let int =
            |value: RuntimeValue| -> Result<i128, Fault> { Ok(sext(Type::I32, integer(value)?)) };
        let value = match call.name {
            | "fopen" => self.open(arg(0), arg(1), None, memory)?,
            | "freopen" => {
                let stream = self.stream(arg(2), memory)?;
                self.open(arg(0), arg(1), Some((stream, arg(2))), memory)?
            },
            | "fclose" => {
                let stream = self.stream(arg(0), memory)?;
                self.flush_before_closing(stream);
                let entry = &mut self.streams.entries[stream];
                entry.target = Target::Closed;
                entry.pushed = None;
                if let Some(object) = entry.object.take() {
                    memory.free(object.provenance.expect("streams have provenance"));
                }
                RuntimeValue::Int(0)
            },
            | "fflush" => {
                // C99 §7.19.5.2p3: a null pointer flushes every stream.
                let stream = if arg(0) == RuntimeValue::NULL {
                    1
                } else {
                    self.stream(arg(0), memory)?
                };
                if matches!(self.streams.entries[stream].target, Target::Output) {
                    self.console.flushed = self.output.len();
                }
                RuntimeValue::Int(0)
            },
            | "fputc" | "putc" => {
                let stream = self.stream(arg(1), memory)?;
                let byte = truncate_u8(integer(arg(0))?);
                let entry = &mut self.streams.entries[stream];
                if write_to(entry, &mut self.output, &mut self.console, &[byte]) {
                    RuntimeValue::Int(u128::from(byte))
                } else {
                    RuntimeValue::Int(EOF)
                }
            },
            | "fputs" => {
                let stream = self.stream(arg(1), memory)?;
                self.text.clear();
                memory.read_c_string(arg(0), &mut self.text)?;
                let written = write_to(
                    &mut self.streams.entries[stream],
                    &mut self.output,
                    &mut self.console,
                    &self.text,
                );
                RuntimeValue::Int(if written { 1 } else { EOF })
            },
            | "fwrite" => {
                let stream = self.stream(arg(3), memory)?;
                let (size, count) = (integer(arg(1))?, integer(arg(2))?);
                let total = size.checked_mul(count).ok_or(UbKind::OutOfBounds)?;
                // glibc writes nothing and returns 0 for an empty request.
                if total == 0 {
                    return Ok(HostReturn::Value(Some(RuntimeValue::Int(0))));
                }
                self.text.clear();
                for offset in 0..u64::try_from(total).map_err(|_| UbKind::OutOfBounds)? {
                    let at = memory.ptr_add(arg(0), RuntimeValue::Int(u128::from(offset)), true);
                    self.text.push(truncate_u8(integer(memory.load(
                        Type::I8,
                        at,
                        Align::BYTE,
                    )?)?));
                }
                let written = write_to(
                    &mut self.streams.entries[stream],
                    &mut self.output,
                    &mut self.console,
                    &self.text,
                );
                RuntimeValue::Int(if written { count } else { 0 })
            },
            | "fread" => {
                let stream = self.stream(arg(3), memory)?;
                let (size, count) = (integer(arg(1))?, integer(arg(2))?);
                let total = size.checked_mul(count).ok_or(UbKind::OutOfBounds)?;
                let mut read = 0_u128;
                while read < total {
                    let Some(byte) = self.read(stream) else {
                        break;
                    };
                    let at = memory.ptr_add(arg(0), RuntimeValue::Int(read), true);
                    memory.store(
                        Type::I8,
                        RuntimeValue::Int(u128::from(byte)),
                        at,
                        Align::BYTE,
                    )?;
                    read += 1;
                }
                RuntimeValue::Int(read.checked_div(size).unwrap_or(0))
            },
            | "fgetc" | "getc" | "getchar" => {
                let stream = if call.name == "getchar" {
                    self.standard(0)?
                } else {
                    self.stream(arg(0), memory)?
                };
                RuntimeValue::Int(self.read(stream).map_or(EOF, u128::from))
            },
            | "fgets" => {
                let stream = self.stream(arg(2), memory)?;
                let limit = int(arg(1))?;
                // glibc: no room gives null and leaves the array alone; room
                // for only the terminator stores it without reading.
                if limit <= 0 {
                    return Ok(HostReturn::Value(Some(RuntimeValue::NULL)));
                }
                let mut count = 0_i128;
                while count + 1 < limit {
                    let Some(byte) = self.read(stream) else {
                        break;
                    };
                    let at = memory.ptr_add(arg(0), RuntimeValue::Int(count.unsigned_abs()), true);
                    memory.store(
                        Type::I8,
                        RuntimeValue::Int(u128::from(byte)),
                        at,
                        Align::BYTE,
                    )?;
                    count += 1;
                    if byte == b'\n' {
                        break;
                    }
                }
                // C99 §7.19.7.2p3: a null pointer when end-of-file comes
                // before any character, leaving the array unchanged.
                if count == 0 && limit > 1 {
                    RuntimeValue::NULL
                } else {
                    let end = memory.ptr_add(arg(0), RuntimeValue::Int(count.unsigned_abs()), true);
                    memory.store(Type::I8, RuntimeValue::Int(0), end, Align::BYTE)?;
                    arg(0)
                }
            },
            | "ungetc" => {
                let stream = self.stream(arg(1), memory)?;
                let character = integer(arg(0))?;
                if character & 0xFFFF_FFFF == EOF {
                    RuntimeValue::Int(EOF)
                } else {
                    let byte = truncate_u8(character);
                    self.push_back(stream, byte);
                    RuntimeValue::Int(u128::from(byte))
                }
            },
            | "feof" | "ferror" => {
                let stream = self.stream(arg(0), memory)?;
                let entry = &self.streams.entries[stream];
                RuntimeValue::Int(u128::from(if call.name == "feof" {
                    entry.eof
                } else {
                    entry.error
                }))
            },
            | "clearerr" => {
                let stream = self.stream(arg(0), memory)?;
                let entry = &mut self.streams.entries[stream];
                entry.eof = false;
                entry.error = false;
                return Ok(HostReturn::Value(None));
            },
            | "fseek" | "rewind" => {
                let stream = self.stream(arg(0), memory)?;
                let position = if call.name == "rewind" {
                    Some(SeekFrom::Start(0))
                } else {
                    let offset = sext(call.arg_types[1], integer(arg(1))?);
                    let offset = i64::try_from(offset).map_err(|_| UbKind::SignatureMismatch)?;
                    // A byte `ungetc` pushed back stands before the file
                    // position, so the current position is one less. A
                    // negative start and an offset past the range fail, as
                    // `lseek` does (EINVAL); glibc refuses an unknown
                    // `whence` before it touches the stream.
                    let pushed = i64::from(self.streams.entries[stream].pushed.is_some());
                    match int(arg(2))? {
                        | 0 => u64::try_from(offset).ok().map(SeekFrom::Start),
                        | 1 => offset.checked_sub(pushed).map(SeekFrom::Current),
                        | 2 => Some(SeekFrom::End(offset)),
                        | _ =>
                            return Ok(HostReturn::Value(Some(RuntimeValue::int(Type::I32, EOF)))),
                    }
                };
                let entry = &mut self.streams.entries[stream];
                let moved = match (position, &mut entry.target) {
                    | (Some(position), Target::File(file)) => file.seek(position).is_ok(),
                    | _ => false,
                };
                // C99 §7.19.9.2p5: a successful seek undoes `ungetc` and
                // clears the end-of-file indicator. One that fails keeps
                // the indicator, and glibc still discards a separate
                // pushback area.
                if moved {
                    entry.eof = false;
                    entry.behind = None;
                }
                if moved || !entry.stepped_back {
                    entry.pushed = None;
                }
                if call.name == "rewind" {
                    // glibc's `rewind` clears both indicators even when the
                    // seek fails.
                    entry.eof = false;
                    entry.error = false;
                    return Ok(HostReturn::Value(None));
                }
                RuntimeValue::Int(if moved { 0 } else { EOF })
            },
            | "ftell" => {
                let stream = self.stream(arg(0), memory)?;
                let entry = &mut self.streams.entries[stream];
                let pushed = u64::from(entry.pushed.is_some());
                let position = match &mut entry.target {
                    // A byte pushed back at the start has no position.
                    | Target::File(file) => file
                        .stream_position()
                        .ok()
                        .and_then(|at| at.checked_sub(pushed)),
                    | _ => None,
                };
                position.map_or(RuntimeValue::Int(u128::MAX), |at| {
                    RuntimeValue::Int(u128::from(at))
                })
            },
            | "remove" => {
                let allowed = self.file_access;
                let path = self.path(arg(0), memory)?;
                let removed = allowed
                    && path.is_some_and(|path| {
                        std::fs::remove_file(path).is_ok() || std::fs::remove_dir(path).is_ok()
                    });
                if !allowed {
                    self.deny_file_access();
                }
                RuntimeValue::Int(if removed { 0 } else { EOF })
            },
            | "tmpnam" => self.tmpnam(arg(0), memory)?,
            | "fprintf" | "vfprintf" | "__fprintf_chk" | "__vfprintf_chk" => {
                let stream = self.stream(arg(0), memory)?;
                let format = if call.name.starts_with("__") { 2 } else { 1 };
                self.print_to(stream, call, format, memory)?
            },
            | "vprintf" | "__printf_chk" => {
                let format = usize::from(call.name == "__printf_chk");
                let stream = self.standard(1)?;
                self.print_to(stream, call, format, memory)?
            },
            | "vsprintf" | "vsnprintf" => {
                let bounded = call.name == "vsnprintf";
                let limit = if bounded { integer(arg(1))? } else { u128::MAX };
                let format = if bounded { 2 } else { 1 };
                let list = va_list_operand(call.name).expect("a v function");
                let mut values = FormatArguments::from_va_list(args[list], call.va_list, memory)?;
                let before = self.output.len();
                let result = self.printf(arg(format), &mut values, memory);
                let count = self.output.len() - before;
                let result =
                    result.and_then(|()| self.store_formatted(before, arg(0), limit, memory));
                self.output.truncate(before);
                result?;
                RuntimeValue::Int(count as u128)
            },
            | "__acrt_iob_func" => {
                let stream = u32::try_from(integer(arg(0))?).map_err(|_| UbKind::InvalidStream)?;
                RuntimeValue::Ptr(memory.standard_stream(stream)?)
            },
            | _ => unreachable!("`FUNCTIONS` lists every stdio function"),
        };
        Ok(HostReturn::Value(call.result.map(|ty| match value {
            | RuntimeValue::Int(bits) if ty.is_int() => RuntimeValue::int(ty, bits),
            | value => value,
        })))
    }

    /// Formats the arguments after the format at `format` (its own values, or
    /// its `va_list`) and writes them to `stream`; the count, or a negative
    /// value when the stream fails.
    fn print_to(
        &mut self,
        stream: usize,
        call: HostCall<'_>,
        format: usize,
        memory: &Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        let mut values = match va_list_operand(call.name) {
            | Some(list) => FormatArguments::from_va_list(call.args[list], call.va_list, memory)?,
            | None =>
                FormatArguments::Values(call.args.get(format + 1..).unwrap_or_default().iter()),
        };
        let before = self.output.len();
        let result = self.printf(call.args[format], &mut values, memory);
        let count = self.output.len() - before;
        if let Err(fault) = result {
            self.output.truncate(before);
            return Err(fault);
        }
        let written = self.move_output(stream, before);
        Ok(RuntimeValue::Int(if written { count as u128 } else { EOF }))
    }

    /// The table index of standard stream `index`, which a function such as
    /// `printf` or `getchar` uses implicitly; undefined once it is closed.
    pub(super) fn standard(&self, index: usize) -> Result<usize, Fault> {
        if matches!(self.streams.entries[index].target, Target::Closed) {
            return Err(UbKind::InvalidStream.into());
        }
        Ok(index)
    }

    /// Releases the buffered standard output before `stream` closes or is
    /// reopened, as glibc flushes a stream it closes.
    /// C99: §7.19.5.1 paragraph 2, p. 270; PDF p. 282.
    fn flush_before_closing(&mut self, stream: usize) {
        if matches!(self.streams.entries[stream].target, Target::Output) {
            self.console.flushed = self.output.len();
        }
    }

    /// Counts a refused file-system access and keeps the first path, for the
    /// command line's note.
    fn deny_file_access(&mut self) {
        if self.denied_files == 0 {
            self.denied_path.clear();
            self.denied_path.extend_from_slice(&self.path);
        }
        self.denied_files += 1;
    }

    /// The table index of the open stream `pointer` names.
    pub(super) fn stream(
        &self,
        pointer: RuntimeValue,
        memory: &Memory<'_>,
    ) -> Result<usize, Fault> {
        if pointer.is_poison() {
            return Err(UbKind::PoisonInHostCall.into());
        }
        let stream = memory.stream_at(pointer).ok_or(UbKind::InvalidStream)?;
        let index = usize::try_from(stream).map_err(|_| UbKind::InvalidStream)?;
        match self.streams.entries.get(index) {
            | Some(entry) if !matches!(entry.target, Target::Closed) => Ok(index),
            | _ => Err(UbKind::InvalidStream.into()),
        }
    }

    /// Moves what was formatted into the output buffer after `before` to
    /// `stream`, which may be the output itself; says whether it was written.
    fn move_output(&mut self, stream: usize, before: usize) -> bool {
        let entry = &mut self.streams.entries[stream];
        let written = match &mut entry.target {
            | Target::Output => return true,
            | Target::Errors => {
                self.console
                    .errors
                    .extend_from_slice(&self.output[before..]);
                let end = self.console.errors.len();
                let released = self.console.released(&self.output[..before]);
                self.console.marks.push((released, end));
                true
            },
            | Target::File(file) => {
                entry.behind = None;
                file.write_all(&self.output[before..]).is_ok()
            },
            | Target::Input | Target::Closed => false,
        };
        entry.error |= !written;
        self.output.truncate(before);
        written
    }

    /// Moves what the program wrote to standard output after `before` to
    /// wherever `freopen` redirected `stdout`, if it did.
    pub(super) fn redirect_output(&mut self, before: usize) {
        if self.output.len() > before {
            let _ = self.move_output(1, before);
        }
    }

    /// Reads one byte from `stream`, setting its end-of-file or error
    /// indicator when there is none.
    /// C99: §7.19.7.1 paragraphs 2-3, p. 296; PDF p. 308.
    pub(super) fn read(&mut self, stream: usize) -> Option<u8> {
        let entry = &mut self.streams.entries[stream];
        if let Some(byte) = entry.pushed.take() {
            entry.behind = entry.stepped_back.then_some(byte);
            return Some(byte);
        }
        // A set end-of-file indicator fails the read without reading, as
        // glibc's `_IO_EOF_SEEN` does.
        if entry.eof {
            return None;
        }
        let read = match &mut entry.target {
            | Target::File(file) => {
                let mut byte = [0];
                file.read(&mut byte)
                    .map(|count| (count == 1).then_some(byte[0]))
            },
            | Target::Input => self.input.next(&mut self.console, self.output.len()),
            | _ => {
                entry.error = true;
                return None;
            },
        };
        entry.behind = None;
        match read {
            | Ok(Some(byte)) => {
                entry.behind = Some(byte);
                Some(byte)
            },
            | Ok(None) => {
                entry.eof = true;
                None
            },
            | Err(_) => {
                entry.error = true;
                None
            },
        }
    }

    /// Pushes `byte` back onto `stream`, as `ungetc` does.
    pub(super) fn push_back(&mut self, stream: usize, byte: u8) {
        let entry = &mut self.streams.entries[stream];
        entry.stepped_back = entry.behind == Some(byte);
        entry.behind = None;
        entry.pushed = Some(byte);
        entry.eof = false;
    }

    /// `fopen`, or `freopen` of `reopen`: the stream on success, else null.
    /// C99: §7.19.5.3 paragraphs 3-6, pp. 270-271; PDF pp. 282-283;
    /// §7.19.5.4, p. 272; PDF p. 284.
    fn open(
        &mut self,
        path: RuntimeValue,
        mode: RuntimeValue,
        reopen: Option<(usize, RuntimeValue)>,
        memory: &mut Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        self.text.clear();
        memory.read_c_string(mode, &mut self.text)?;
        // glibc reads the access letter and then up to six modifiers, of
        // which `+` adds the other direction, `x` (C11) refuses an existing
        // file, and the rest (`b`, `e`, `m`, `c`, ...) do not change the
        // host's access.
        let modifiers = self.text.get(1..self.text.len().min(7)).unwrap_or_default();
        let plus = modifiers.contains(&b'+');
        let exclusive = modifiers.contains(&b'x');
        let mut options = OpenOptions::new();
        let _ = match self.text.first() {
            | Some(b'r') => options.read(true).write(plus),
            | Some(b'w') if exclusive => options.write(true).create_new(true).read(plus),
            | Some(b'w') => options.write(true).create(true).truncate(true).read(plus),
            | Some(b'a') if exclusive => options.append(true).create_new(true).read(plus),
            | Some(b'a') => options.append(true).create(true).read(plus),
            | _ => return Ok(RuntimeValue::NULL),
        };
        let allowed = self.file_access;
        let file = self
            .path(path, memory)?
            .filter(|_| allowed)
            .and_then(|path| options.open(path).ok());
        if !allowed {
            self.deny_file_access();
        }
        if let Some((stream, pointer)) = reopen {
            // C99 §7.19.5.4p3: the original stream is flushed and closed
            // even when the new file cannot be opened, so its `FILE *` dies
            // with it.
            self.flush_before_closing(stream);
            let entry = &mut self.streams.entries[stream];
            let object = entry.object;
            *entry = Stream::default();
            let Some(file) = file else {
                if let Some(object) = object {
                    memory.free(object.provenance.expect("streams have provenance"));
                }
                return Ok(RuntimeValue::NULL);
            };
            *entry = Stream {
                target: Target::File(file),
                object,
                ..Stream::default()
            };
            return Ok(pointer);
        }
        let Some(file) = file else {
            return Ok(RuntimeValue::NULL);
        };
        let Some(index) = (3..MAX_STREAMS)
            .find(|&index| matches!(self.streams.entries[index].target, Target::Closed))
        else {
            return Ok(RuntimeValue::NULL);
        };
        let number = u32::try_from(index).expect("a small table");
        let object = memory.allocate(ObjectKind::Stream(number), 0, Align::BYTE)?;
        self.streams.entries[index] = Stream {
            target: Target::File(file),
            object: Some(object),
            ..Stream::default()
        };
        Ok(RuntimeValue::Ptr(object))
    }

    /// The host path the C string at `pointer` names, or `None` when it is
    /// not valid UTF-8.
    fn path(
        &mut self,
        pointer: RuntimeValue,
        memory: &Memory<'_>,
    ) -> Result<Option<&std::path::Path>, Fault> {
        self.path.clear();
        memory.read_c_string(pointer, &mut self.path)?;
        Ok(std::str::from_utf8(&self.path)
            .ok()
            .map(std::path::Path::new))
    }

    /// `tmpnam`: a name no file has, written to `buffer` or to the host's
    /// static buffer when it is null.
    /// C99: §7.19.4.4 paragraphs 2-3, p. 269; PDF p. 281.
    fn tmpnam(
        &mut self,
        buffer: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<RuntimeValue, Fault> {
        // `/tmp/bccNNNNNN` fits `L_tmpnam`; Windows names are relative, as
        // its C libraries' are. Without file access the names are not
        // checked against the file system, so a run names the same files
        // every time.
        let prefix: &[u8] = if cfg!(windows) { b"bcc" } else { b"/tmp/bcc" };
        loop {
            if self.streams.names >= TMPNAM_NAMES {
                return Ok(RuntimeValue::NULL);
            }
            let number = self.streams.names;
            self.streams.names += 1;
            self.path.clear();
            self.path.extend_from_slice(prefix);
            for shift in (0..6).rev() {
                let digit = number / 10_u32.pow(shift) % 10;
                self.path.push(b'0' + u8::try_from(digit).expect("a digit"));
            }
            let taken = self.file_access
                && std::str::from_utf8(&self.path)
                    .is_ok_and(|name| std::path::Path::new(name).exists());
            if !taken {
                break;
            }
        }
        let destination = if buffer == RuntimeValue::NULL {
            match self.tmpnam_buffer {
                | Some(pointer) => RuntimeValue::Ptr(pointer),
                | None => {
                    let pointer = memory.allocate(ObjectKind::Global, TMPNAM_SIZE, Align::BYTE)?;
                    memory.initialize(pointer, &[]);
                    self.tmpnam_buffer = Some(pointer);
                    RuntimeValue::Ptr(pointer)
                },
            }
        } else {
            buffer
        };
        for (offset, &byte) in self.path.iter().chain(&[0]).enumerate() {
            let at = memory.ptr_add(destination, RuntimeValue::Int(offset as u128), true);
            memory.store(
                Type::I8,
                RuntimeValue::Int(u128::from(byte)),
                at,
                Align::BYTE,
            )?;
        }
        Ok(destination)
    }
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}

/// Writes `bytes` to the stream `entry`, setting its error indicator on
/// failure; standard output and error go to the host's buffers.
fn write_to(
    entry: &mut Stream,
    output: &mut super::ArenaVec<'_, u8>,
    console: &mut Console<'_>,
    bytes: &[u8],
) -> bool {
    let written = match &mut entry.target {
        | Target::Output => {
            output.extend_from_slice(bytes);
            true
        },
        | Target::Errors => {
            console.errors.extend_from_slice(bytes);
            let released = console.released(output);
            console.marks.push((released, console.errors.len()));
            true
        },
        | Target::File(file) => {
            entry.behind = None;
            file.write_all(bytes).is_ok()
        },
        | Target::Input | Target::Closed => false,
    };
    entry.error |= !written;
    written
}

#[cfg(test)]
mod tests {
    use super::{
        Console,
        DefaultHost,
        TMPNAM_NAMES,
    };
    use crate::{
        backend::interpreter::{
            memory::Memory,
            value::RuntimeValue,
        },
        util::bump::{
            ArenaVec,
            Bump,
        },
    };

    /// Without file access, `tmpnam` names files in order without looking
    /// at the file system, never repeats a name, and returns null once the
    /// names run out.
    #[test]
    fn tmpnam_names_are_bounded_and_never_repeat() {
        let arena = Bump::new();
        let mut memory = Memory::new(&arena, 1 << 20);
        let mut host = DefaultHost::new_in(&arena);
        let name = |host: &mut DefaultHost<'_>, memory: &mut Memory<'_>| {
            let pointer = host.tmpnam(RuntimeValue::NULL, memory).unwrap();
            if pointer == RuntimeValue::NULL {
                return None;
            }
            let mut text = ArenaVec::new_in(&arena);
            memory.read_c_string(pointer, &mut text).unwrap();
            Some(text.to_vec())
        };
        let first = name(&mut host, &mut memory).unwrap();
        let second = name(&mut host, &mut memory).unwrap();
        assert!(first.ends_with(b"bcc000000"), "{first:?}");
        assert!(second.ends_with(b"bcc000001"), "{second:?}");
        host.streams.names = TMPNAM_NAMES - 1;
        let last = name(&mut host, &mut memory).unwrap();
        assert!(last.ends_with(b"bcc999999"), "{last:?}");
        assert_eq!(name(&mut host, &mut memory), None);
    }

    /// A line-buffered stdout searches each byte of the output buffer for a
    /// newline once, however often the program writes to stderr: these
    /// 200 000 writes look at 3.2 MB in all, where searching the whole
    /// buffer at each would look at hundreds of gigabytes and run for
    /// minutes.
    #[test]
    fn line_buffered_release_searches_each_byte_once() {
        let arena = Bump::new();
        let mut console = Console {
            errors:        ArenaVec::new_in(&arena),
            marks:         ArenaVec::new_in(&arena),
            flushed:       0,
            line_buffered: true,
            scanned:       0,
            line_end:      0,
        };
        let mut output = ArenaVec::new_in(&arena);
        let (mut line_end, mut previous) = (0, 0);
        for write in 0..200_000 {
            output.extend_from_slice(b"no newline here.");
            if write % 50_000 == 0 {
                output.extend_from_slice(b"\n tail");
                previous = line_end;
                line_end = output.len() - 5;
            }
            assert_eq!(console.released(&output), line_end, "write {write}");
            assert_eq!(console.scanned, output.len());
        }
        // A buffer cut short before its last newline is searched again.
        assert_eq!(console.released(&output[..line_end - 1]), previous);
    }
}
