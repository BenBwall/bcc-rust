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
//! Streams read as glibc's do ([`stream`]): through a buffer of the size
//! glibc picks, refilled by one `read` at a time, with glibc's pushback and
//! seeks. glibc line-buffers a terminal, and refilling a line-buffered input
//! stream releases a line-buffered `stdout`; reading what the buffer still
//! holds, reading straight into a large `fread` array, or reading a pipe or
//! file releases nothing.
//!
//! Host files are opt-in (`--interpret-allow-fs`): without it, `fopen`,
//! `freopen` and `remove` fail as glibc does on `EACCES`, returning null or
//! -1, and the host counts the refusal for the command line to note;
//! `tmpnam` then names files without looking at the file system. Writes to
//! files are buffered as glibc buffers them, so a later `fopen` of the same
//! name reads them back only once they are written out, at the latest when
//! the program ends normally. `errno` is not modelled: a failing call only
//! returns its failure value.
//!
//! C99: §7.19.3, pp. 264-266; PDF pp. 276-278 (files); §7.19.4-§7.19.10,
//! pp. 268-304; PDF pp. 280-316 (operations, access, I/O, positioning and
//! errors).

mod stream;

use std::{
    fs::OpenOptions,
    io::{
        Seek as _,
        SeekFrom,
    },
};

use stream::{
    Release,
    Stream,
    Target,
    Whence,
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
        value::RuntimeValue,
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
    /// newline releases what precedes it, and refilling a line-buffered input
    /// stream releases all.
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

/// The host's stream table.
pub(super) struct Streams<'a> {
    entries: [Stream<'a>; MAX_STREAMS],
    /// Names `tmpnam` has handed out.
    names:   u32,
    /// How many streams have been opened, the standard three included.
    opened:  u64,
}

impl<'a> Streams<'a> {
    /// The standard streams open, the rest closed, with their buffers in
    /// `arena`.
    pub(super) fn new_in(arena: &'a crate::util::bump::Bump) -> Self {
        let mut entries: [Stream<'a>; MAX_STREAMS] = std::array::from_fn(|_| Stream::new_in(arena));
        entries[0].reopen(Target::Input, true, false, false);
        entries[1].reopen(Target::Output, false, true, false);
        entries[2].reopen(Target::Errors, false, true, false);
        // glibc's list starts as `stderr`, `stdout`, `stdin`.
        for (index, entry) in entries[..3].iter_mut().enumerate() {
            entry.opened = index as u64;
        }
        Self {
            entries,
            names: 0,
            opened: 3,
        }
    }

    /// Stream `stream`, to read, and the `stdout` stream unless that is the
    /// same one.
    fn reading(&mut self, stream: usize) -> (&mut Stream<'a>, Option<&mut Stream<'a>>) {
        if stream == 1 {
            return (&mut self.entries[1], None);
        }
        let [entry, stdout] = self
            .entries
            .get_disjoint_mut([stream, 1])
            .expect("distinct streams in the table");
        (entry, Some(stdout))
    }

    /// The order of a stream opened now.
    fn open_next(&mut self) -> u64 {
        self.opened += 1;
        self.opened
    }

    /// Treats the process's standard input as a terminal.
    pub(super) fn set_terminal_input(&mut self, terminal: bool) {
        self.entries[0].terminal = terminal;
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
                let flushed = self.flush_before_closing(stream);
                let entry = &mut self.streams.entries[stream];
                entry.reopen(Target::Closed, false, false, false);
                if let Some(object) = entry.object.take() {
                    memory.free(object.provenance.expect("streams have provenance"));
                }
                RuntimeValue::Int(if flushed { 0 } else { EOF })
            },
            | "fflush" => {
                // C99 §7.19.5.2p3: a null pointer flushes every stream.
                // glibc 2.43 also gives back what each stream read ahead
                // (`_IO_flush_all`); 2.39 leaves streams that read alone.
                let flushed = if arg(0) == RuntimeValue::NULL {
                    self.flush_all()
                } else {
                    let stream = self.stream(arg(0), memory)?;
                    let entry = &mut self.streams.entries[stream];
                    if matches!(entry.target, Target::Output) {
                        self.console.flushed = self.output.len();
                        true
                    } else {
                        entry.sync()
                    }
                };
                RuntimeValue::Int(if flushed { 0 } else { EOF })
            },
            | "fputc" | "putc" => {
                let stream = self.stream(arg(1), memory)?;
                let byte = truncate_u8(integer(arg(0))?);
                let entry = &mut self.streams.entries[stream];
                if write_to(entry, &mut self.output, &mut self.console, &[byte], false) == 1 {
                    RuntimeValue::Int(u128::from(byte))
                } else {
                    RuntimeValue::Int(EOF)
                }
            },
            | "fputs" => {
                let stream = self.stream(arg(1), memory)?;
                self.text.clear();
                memory.read_c_string(arg(0), &mut self.text)?;
                let taken = write_to(
                    &mut self.streams.entries[stream],
                    &mut self.output,
                    &mut self.console,
                    &self.text,
                    true,
                );
                RuntimeValue::Int(if taken == self.text.len() { 1 } else { EOF })
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
                let taken = write_to(
                    &mut self.streams.entries[stream],
                    &mut self.output,
                    &mut self.console,
                    &self.text,
                    true,
                );
                RuntimeValue::Int(if taken == self.text.len() {
                    count
                } else {
                    taken as u128 / size
                })
            },
            | "fread" => {
                let stream = self.stream(arg(3), memory)?;
                let (size, count) = (integer(arg(1))?, integer(arg(2))?);
                let total = size.checked_mul(count).ok_or(UbKind::OutOfBounds)?;
                // glibc reads nothing and returns 0 for an empty request,
                // before it touches the stream.
                if total == 0 {
                    return Ok(HostReturn::Value(Some(RuntimeValue::Int(0))));
                }
                let total = usize::try_from(total).map_err(|_| UbKind::OutOfBounds)?;
                let mut stored = 0_u128;
                let read = self.read_block(stream, total, |bytes| {
                    for &byte in bytes {
                        let at = memory.ptr_add(arg(0), RuntimeValue::Int(stored), true);
                        memory.store(
                            Type::I8,
                            RuntimeValue::Int(u128::from(byte)),
                            at,
                            Align::BYTE,
                        )?;
                        stored += 1;
                    }
                    Ok::<(), Fault>(())
                })?;
                RuntimeValue::Int((read as u128).checked_div(size).unwrap_or(0))
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
                // glibc fails a call that meets a new read error, even after
                // reading part of a line, and keeps an earlier error set.
                // Only a call that reads orients the stream.
                let entry = &mut self.streams.entries[stream];
                entry.oriented |= limit > 1;
                let earlier_error = std::mem::take(&mut entry.error);
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
                let entry = &mut self.streams.entries[stream];
                let failed = entry.error;
                entry.error |= earlier_error;
                // C99 §7.19.7.2p3: a null pointer when end-of-file comes
                // before any character, leaving the array unchanged.
                if (count == 0 || failed) && limit > 1 {
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
                let (offset, whence) = if call.name == "rewind" {
                    (0, Whence::Start)
                } else {
                    let offset = sext(call.arg_types[1], integer(arg(1))?);
                    let offset = i64::try_from(offset).map_err(|_| UbKind::SignatureMismatch)?;
                    // glibc refuses an unknown `whence` before it touches
                    // the stream.
                    let whence = match int(arg(2))? {
                        | 0 => Whence::Start,
                        | 1 => Whence::Current,
                        | 2 => Whence::End,
                        | _ =>
                            return Ok(HostReturn::Value(Some(RuntimeValue::int(Type::I32, EOF)))),
                    };
                    (offset, whence)
                };
                let entry = &mut self.streams.entries[stream];
                let moved = entry.seek(offset, whence);
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
                let position = self.streams.entries[stream].tell();
                RuntimeValue::Int(position.map_or(u128::MAX, |at| at.cast_unsigned().into()))
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
        let written = self.move_output(stream, before, Written::Formatted);
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

    /// Writes out what `stream` holds before it closes or is reopened, as
    /// glibc flushes a stream it closes; whether that worked.
    /// C99: §7.19.5.1 paragraph 2, p. 270; PDF p. 282.
    fn flush_before_closing(&mut self, stream: usize) -> bool {
        let entry = &mut self.streams.entries[stream];
        if matches!(entry.target, Target::Output) {
            self.console.flushed = self.output.len();
            return true;
        }
        entry.write_out()
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
    /// `stream`, which may be the output itself, writing it as `kind`
    /// writes; says whether it was written.
    fn move_output(&mut self, stream: usize, before: usize, kind: Written) -> bool {
        let entry = &mut self.streams.entries[stream];
        let bytes = &self.output[before..];
        let written = match entry.target {
            | Target::Output => return true,
            | Target::Errors => {
                self.console.errors.extend_from_slice(bytes);
                let end = self.console.errors.len();
                let released = self.console.released(&self.output[..before]);
                self.console.marks.push((released, end));
                true
            },
            | _ => match kind {
                | Written::Formatted => entry.put_formatted(bytes),
                | Written::Byte => bytes.iter().all(|&byte| entry.put_char(byte)),
                // `puts` writes the string as a block and then its newline.
                | Written::Line => {
                    let (line, newline) = bytes.split_at(bytes.len().saturating_sub(1));
                    entry.put_block(line) == line.len()
                        && newline.iter().all(|&byte| entry.put_char(byte))
                },
            },
        };
        self.output.truncate(before);
        written
    }

    /// Moves what a function that writes standard output implicitly wrote
    /// after `before` to wherever `freopen` redirected `stdout`, if it did;
    /// says whether it was written. A `printf` that formats nothing still
    /// fails on a `stdout` that may not write.
    pub(super) fn redirect_output(&mut self, before: usize, function: &str) -> bool {
        let kind = match function {
            | "putchar" => Written::Byte,
            | "puts" => Written::Line,
            | "printf" | "__vprintf_chk" => Written::Formatted,
            | _ if self.output.len() > before => Written::Formatted,
            | _ => return true,
        };
        self.move_output(1, before, kind)
    }

    /// Writes out what every stream holds, in glibc's `_IO_list_all` order
    /// (the most recently opened first), and gives back what streams that
    /// read have read ahead, as glibc 2.43's `fflush(NULL)` and the flush at
    /// exit do (`_IO_flush_all`); whether every write worked. 2.39 leaves
    /// streams that read alone.
    pub(super) fn flush_all(&mut self) -> bool {
        let mut order: [usize; MAX_STREAMS] = std::array::from_fn(|index| index);
        order.sort_unstable_by_key(|&index| std::cmp::Reverse(self.streams.entries[index].opened));
        let mut flushed = true;
        for index in order {
            let entry = &mut self.streams.entries[index];
            match entry.target {
                | Target::Output => self.console.flushed = self.output.len(),
                | Target::Closed => {},
                | _ => {
                    flushed &= entry.write_out();
                    if entry.reads_ahead() {
                        flushed &= entry.sync();
                    }
                },
            }
        }
        flushed
    }

    /// Reads one byte from `stream`, setting its end-of-file or error
    /// indicator when there is none.
    pub(super) fn read(&mut self, stream: usize) -> Option<u8> {
        let (entry, stdout) = self.streams.reading(stream);
        let mut release = Release {
            console: &mut self.console,
            written: self.output.len(),
            stdout,
        };
        entry.read(&mut release)
    }

    /// Reads up to `wanted` bytes from `stream` for `fread`, passing them to
    /// `store`; how many it read.
    fn read_block<E>(
        &mut self,
        stream: usize,
        wanted: usize,
        store: impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<usize, E> {
        let (entry, stdout) = self.streams.reading(stream);
        let mut release = Release {
            console: &mut self.console,
            written: self.output.len(),
            stdout,
        };
        entry.read_block(wanted, &mut release, store)
    }

    /// Pushes `byte` back onto `stream`, as `ungetc` does.
    pub(super) fn push_back(&mut self, stream: usize, byte: u8) {
        self.streams.entries[stream].push_back(byte);
    }

    /// Orients `stream` for bytes, as `fscanf` does before it reads.
    pub(super) fn orient(&mut self, stream: usize) {
        self.streams.entries[stream].oriented = true;
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
        if let Some((stream, _)) = reopen {
            // C99 §7.19.5.4p4: the original stream is closed first, written
            // out with any failure ignored, so that reopening its own file
            // for writing truncates what it held too (glibc's `freopen`
            // syncs and closes before it opens).
            let _ = self.flush_before_closing(stream);
            self.streams.entries[stream].reopen(Target::Closed, false, false, false);
        }
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
        let access = self.text.first().copied();
        let known = match access {
            | Some(b'r') => Some(options.read(true).write(plus)),
            | Some(b'w') if exclusive => Some(options.write(true).create_new(true).read(plus)),
            | Some(b'w') => Some(options.write(true).create(true).truncate(true).read(plus)),
            | Some(b'a') if exclusive => Some(options.append(true).create_new(true).read(plus)),
            | Some(b'a') => Some(options.append(true).create(true).read(plus)),
            | _ => None,
        }
        .is_some();
        let readable = access == Some(b'r') || plus;
        let writable = access != Some(b'r') || plus;
        let appending = access == Some(b'a');
        let allowed = self.file_access;
        let mut file = if known {
            self.path(path, memory)?
                .filter(|_| allowed)
                .and_then(|path| options.open(path).ok())
        } else {
            None
        };
        // glibc starts a stream that only appends at the end of the file
        // (`_IO_file_open`); one that also reads starts at its beginning.
        if appending
            && !readable
            && let Some(opened) = &mut file
        {
            _ = opened.seek(SeekFrom::End(0)).ok();
        }
        if known && !allowed {
            self.deny_file_access();
        }
        if let Some((stream, pointer)) = reopen {
            // C99 §7.19.5.4p3: the original stream stays closed when the new
            // file cannot be opened, so its `FILE *` dies with it.
            let entry = &mut self.streams.entries[stream];
            let Some(file) = file else {
                if let Some(object) = entry.object.take() {
                    memory.free(object.provenance.expect("streams have provenance"));
                }
                return Ok(RuntimeValue::NULL);
            };
            entry.reopen(Target::File(file), readable, writable, appending);
            self.streams.entries[stream].opened = self.streams.open_next();
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
        let opened = self.streams.open_next();
        let entry = &mut self.streams.entries[index];
        entry.reopen(Target::File(file), readable, writable, appending);
        entry.object = Some(object);
        entry.opened = opened;
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

/// Writes `bytes` to the stream `entry`, as `fputs` and `fwrite` write a
/// `block`, else as `fputc` writes a byte; how many it took. Standard
/// output and error go to the host's buffers at once.
fn write_to(
    entry: &mut Stream<'_>,
    output: &mut super::ArenaVec<'_, u8>,
    console: &mut Console<'_>,
    bytes: &[u8],
    block: bool,
) -> usize {
    match entry.target {
        | Target::Output => output.extend_from_slice(bytes),
        | Target::Errors => {
            console.errors.extend_from_slice(bytes);
            let released = console.released(output);
            console.marks.push((released, console.errors.len()));
        },
        | _ if block => return entry.put_block(bytes),
        | _ => return usize::from(entry.put_char(bytes[0])),
    }
    entry.oriented = true;
    bytes.len()
}

/// How a function writes the bytes it hands a stream.
#[derive(Clone, Copy)]
enum Written {
    /// Byte by byte, as `fputc` and `putchar`.
    Byte,
    /// As the `printf` family: byte by byte, failing even with nothing to
    /// write on a stream that may not write.
    Formatted,
    /// As `puts`: a block and then a newline.
    Line,
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
