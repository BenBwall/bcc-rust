//! The `<stdio.h>` streams: `stdin`, `stdout` and `stderr`, and files the
//! program opens on the host's file system.
//!
//! A `FILE *` points to a [`ObjectKind::Stream`] object, whose number indexes
//! the host's stream table; the program cannot read or write the object
//! itself. `stdout` writes to the host's captured output and `stderr` to its
//! captured error output, unless `freopen` redirects them; `stdin` reads the
//! process's standard input. Files are unbuffered, so every write reaches the
//! file at once and a later `fopen` of the same name reads it back. `errno` is
//! not modelled: a failing call only returns its failure value.
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

/// The open streams the host can track at once, the standard three
/// included.
const MAX_STREAMS: usize = 64;

/// The host's stream table.
pub(super) struct Streams {
    entries: [Stream; MAX_STREAMS],
    /// Names `tmpnam` has handed out.
    names:   u32,
}

/// One stream's destination and state.
#[derive(Default)]
struct Stream {
    target: Target,
    /// The end-of-file and error indicators.
    eof:    bool,
    error:  bool,
    /// A byte `ungetc` pushed back.
    pushed: Option<u8>,
    /// The memory object of a stream `fopen` made, freed by `fclose`.
    object: Option<Pointer>,
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
                let entry = &mut self.streams.entries[stream];
                entry.target = Target::Closed;
                entry.pushed = None;
                if let Some(object) = entry.object.take() {
                    memory.free(object.provenance.expect("streams have provenance"));
                }
                RuntimeValue::Int(0)
            },
            | "fflush" => {
                if arg(0) != RuntimeValue::NULL {
                    let _ = self.stream(arg(0), memory)?;
                }
                RuntimeValue::Int(0)
            },
            | "fputc" | "putc" => {
                let stream = self.stream(arg(1), memory)?;
                let byte = truncate_u8(integer(arg(0))?);
                let entry = &mut self.streams.entries[stream];
                if write_to(entry, &mut self.output, &mut self.errors, &[byte]) {
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
                    &mut self.errors,
                    &self.text,
                );
                RuntimeValue::Int(if written { 1 } else { EOF })
            },
            | "fwrite" => {
                let stream = self.stream(arg(3), memory)?;
                let (size, count) = (integer(arg(1))?, integer(arg(2))?);
                let total = size.checked_mul(count).ok_or(UbKind::OutOfBounds)?;
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
                    &mut self.errors,
                    &self.text,
                );
                RuntimeValue::Int(if written || size == 0 { count } else { 0 })
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
                    0
                } else {
                    self.stream(arg(0), memory)?
                };
                RuntimeValue::Int(self.read(stream).map_or(EOF, u128::from))
            },
            | "fgets" => {
                let stream = self.stream(arg(2), memory)?;
                let limit = int(arg(1))?;
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
                    let entry = &mut self.streams.entries[stream];
                    let byte = truncate_u8(character);
                    entry.pushed = Some(byte);
                    entry.eof = false;
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
                    SeekFrom::Start(0)
                } else {
                    let offset = sext(call.arg_types[1], integer(arg(1))?);
                    let offset = i64::try_from(offset).map_err(|_| UbKind::SignatureMismatch)?;
                    match int(arg(2))? {
                        | 0 => SeekFrom::Start(offset.cast_unsigned()),
                        | 1 => SeekFrom::Current(offset),
                        | _ => SeekFrom::End(offset),
                    }
                };
                let entry = &mut self.streams.entries[stream];
                entry.pushed = None;
                entry.eof = false;
                if call.name == "rewind" {
                    entry.error = false;
                }
                let moved = match &mut entry.target {
                    | Target::File(file) => file.seek(position).is_ok(),
                    | _ => false,
                };
                if call.name == "rewind" {
                    return Ok(HostReturn::Value(None));
                }
                RuntimeValue::Int(if moved { 0 } else { EOF })
            },
            | "ftell" => {
                let stream = self.stream(arg(0), memory)?;
                let entry = &mut self.streams.entries[stream];
                let pushed = u64::from(entry.pushed.is_some());
                let position = match &mut entry.target {
                    | Target::File(file) => file.stream_position().ok().map(|at| at - pushed),
                    | _ => None,
                };
                position.map_or(RuntimeValue::Int(u128::MAX), |at| {
                    RuntimeValue::Int(u128::from(at))
                })
            },
            | "remove" => {
                let path = self.path(arg(0), memory)?;
                let removed = path.is_some_and(|path| {
                    std::fs::remove_file(path).is_ok() || std::fs::remove_dir(path).is_ok()
                });
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
                self.print_to(1, call, format, memory)?
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
                self.errors.extend_from_slice(&self.output[before..]);
                true
            },
            | Target::File(file) => file.write_all(&self.output[before..]).is_ok(),
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
    pub(super) fn read(&mut self, stream: usize) -> Option<u8> {
        let entry = &mut self.streams.entries[stream];
        if let Some(byte) = entry.pushed.take() {
            return Some(byte);
        }
        let mut byte = [0];
        let read = match &mut entry.target {
            | Target::File(file) => file.read(&mut byte),
            | Target::Input => std::io::stdin().lock().read(&mut byte),
            | _ => {
                entry.error = true;
                return None;
            },
        };
        match read {
            | Ok(1) => Some(byte[0]),
            | Ok(_) => {
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
        let mut options = OpenOptions::new();
        let plus = self.text.contains(&b'+');
        let _ = match self.text.first() {
            | Some(b'r') => options.read(true).write(plus),
            | Some(b'w') => options.write(true).create(true).truncate(true).read(plus),
            | Some(b'a') => options.append(true).create(true).read(plus),
            | _ => return Ok(RuntimeValue::NULL),
        };
        let file = self
            .path(path, memory)?
            .and_then(|path| options.open(path).ok());
        if let Some((stream, pointer)) = reopen {
            let entry = &mut self.streams.entries[stream];
            *entry = Stream {
                object: entry.object,
                ..Stream::default()
            };
            return Ok(match file {
                | Some(file) => {
                    entry.target = Target::File(file);
                    pointer
                },
                | None => RuntimeValue::NULL,
            });
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
        // its C libraries' are.
        let prefix: &[u8] = if cfg!(windows) { b"bcc" } else { b"/tmp/bcc" };
        loop {
            self.streams.names += 1;
            let number = (std::process::id() % 1000) * 1000 + self.streams.names % 1000;
            self.path.clear();
            self.path.extend_from_slice(prefix);
            for shift in (0..6).rev() {
                let digit = number / 10_u32.pow(shift) % 10;
                self.path.push(b'0' + u8::try_from(digit).expect("a digit"));
            }
            let taken = std::str::from_utf8(&self.path)
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
    errors: &mut super::ArenaVec<'_, u8>,
    bytes: &[u8],
) -> bool {
    let written = match &mut entry.target {
        | Target::Output => {
            output.extend_from_slice(bytes);
            true
        },
        | Target::Errors => {
            errors.extend_from_slice(bytes);
            true
        },
        | Target::File(file) => file.write_all(bytes).is_ok(),
        | Target::Input | Target::Closed => false,
    };
    entry.error |= !written;
    written
}
