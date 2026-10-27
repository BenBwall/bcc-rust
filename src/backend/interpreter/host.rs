//! Host functions: what a call to a declared but undefined function does.
//!
//! The interpreter hands such a call to a [`Host`] by name. [`DefaultHost`]
//! implements the part of the C library that test programs use to report
//! results, manage memory and handle strings and files, and captures their
//! output in arena buffers. `errno.rs` holds `errno` and its messages,
//! `locale.rs` the locale and its character conversions, `stdio.rs` the
//! streams and files (with the wide character functions in `stdio/wide.rs`),
//! `printf.rs` and `scanf.rs` formatted input and output, `strings.rs` the
//! string, wide-string, character and conversion functions, `math.rs`,
//! `quad.rs` (glibc's `_Float128` functions) and `complex.rs` the numeric
//! library, and `checked.rs` the
//! object-size-checking `__*_chk` entry points of fortified C libraries;
//! `library.rs` gives the signatures of the functions outside the builtin
//! catalog, and `arguments.rs` reads the variadic arguments of formatted
//! output, including from a `va_list`. `qsort` and `bsearch`, which call back
//! into the program, run in the machine (`callbacks.rs`).

// Formatted input and output
mod arguments;
mod floating;
mod printf;
mod scanf;

// Library families
mod checked;
mod complex;
mod errno;
mod library;
mod locale;
mod math;
mod quad;
mod stdio;
mod strings;

pub(crate) use arguments::HostVaList;
use arguments::{
    ArgumentClass,
    FormatArguments,
};

use super::{
    memory::{
        Memory,
        ObjectKind,
    },
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::{
        Pointer,
        RuntimeValue,
    },
};
use crate::{
    ir::{
        Align,
        Type,
    },
    target::{
        Scalar,
        Target,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The functions a program may call without defining them.
pub(crate) trait Host {
    /// Runs `call`, reading and writing the program's memory as it needs.
    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault>;

    /// Whether the host defines `name`, so a weak declaration of it resolves
    /// to the host function rather than to a null address. A host that does
    /// not say defines nothing weak.
    fn provides(&self, _name: &str) -> bool {
        false
    }

    /// The standard stream a declared but undefined object names, if the
    /// host's C library defines that object: `stdin`, `stdout` and `stderr`
    /// (0, 1 and 2) are objects holding a `FILE *`. A host that does not say
    /// defines no streams.
    /// C99: §7.19.1 paragraph 3, p. 263; PDF p. 275.
    fn standard_stream(&self, _name: &str) -> Option<u32> {
        None
    }

    /// The C library object other than a stream that a declared but
    /// undefined object names, if the host's C library defines it. A host
    /// that does not say defines none.
    fn library_object(&self, _name: &str) -> Option<LibraryObject> {
        None
    }

    /// Ends the program normally, after `main` returns or the program calls
    /// `exit` and the destructors have run, but not after `abort` or a
    /// trap: as the C library does then, the host writes out what its
    /// streams hold.
    /// C99: §7.20.4.3 paragraph 4, p. 315; PDF p. 327.
    fn at_exit(&mut self) {}

    /// Ends the program at once, after `_Exit`, `_exit` or `quick_exit`:
    /// what the host's streams hold is lost, as the C library loses it.
    /// C99: §7.20.4.4 paragraph 2, p. 316; PDF p. 328.
    fn at_immediate_exit(&mut self) {}
}

/// An object of the host's C library other than a stream, which the
/// program's declaration of it becomes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LibraryObject {
    /// glibc's `signgam`: the sign of Γ the last `lgamma`, `lgammaf`,
    /// `lgammal` or `gamma` call found (XSI; glibc's own symbol since 2.0).
    Signgam = 0,
}

/// A call the host is asked to run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HostCall<'c> {
    /// The callee's name, without the `@`.
    pub(crate) name:      &'c str,
    /// The arguments, fixed and variadic, in order.
    pub(crate) args:      &'c [RuntimeValue],
    /// Operand types after default promotions and ABI lowering.
    pub(crate) arg_types: &'c [Type],
    /// The callee's declared result type, if any.
    pub(crate) result:    Option<Type>,
    /// How the callee's `va_list` operand, if it takes one, is read.
    pub(crate) va_list:   HostVaList<'c>,
}

/// How a host call ended.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum HostReturn {
    /// It returned, with a value if its signature has a result.
    Value(Option<RuntimeValue>),
    /// It ended the program with an exit status, as `exit` does.
    Exit(i32),
    /// It ended the program abnormally, as `abort` does.
    Abort,
    /// The host does not provide the function.
    NotProvided,
}

/// The default host: `putchar`, `puts`, `printf`, `malloc`, `calloc`,
/// `free`, memory and string operations, `sprintf`, `abs`, `fabs`, `floor`,
/// `exit` and `abort`, with output
/// captured in an arena buffer. Each follows its C library description;
/// misuse the standard leaves undefined, such as an overlapping `memcpy`,
/// traps.
///
/// C99: §7.19.7.9 and §7.19.7.10, p. 299; PDF p. 311.
/// C99: §7.21.2.1, p. 325; PDF p. 337.
/// C99: §7.21.6.1, p. 333; PDF p. 345.
/// C99: §7.21.6.3, p. 334; PDF p. 346.
/// C99: §7.19.6.6, p. 291; PDF p. 303 (`sprintf`).
/// C99: §7.20.6.1, p. 320; PDF p. 332 (`abs`).
/// C99: §7.12.4-§7.12.13, pp. 218-239; PDF pp. 230-251 (real math).
/// C99: §7.4.1.8, p. 183; PDF p. 195 (`isprint` in the initial C locale).
pub(crate) struct DefaultHost<'a> {
    allocator:       &'a Bump,
    output:          ArenaVec<'a, u8>,
    /// What the program wrote to `stderr`, and where it falls among the
    /// flushes of `stdout`.
    console:         stdio::Console<'a>,
    /// Whether the program may open, create and remove host files.
    file_access:     bool,
    /// How many file-system accesses were refused, and the first path.
    denied_files:    u32,
    denied_path:     ArenaVec<'a, u8>,
    /// The open streams.
    streams:         stdio::Streams<'a>,
    /// A file name being passed to the host's file system.
    path:            ArenaVec<'a, u8>,
    /// `tmpnam`'s static buffer, once made.
    tmpnam_buffer:   Option<Pointer>,
    /// The program's `errno` object, once made.
    errno:           Option<Pointer>,
    /// `strerror`'s message strings, by code, once made, and its buffer
    /// for an unknown code.
    messages:        [Option<Pointer>; errno::MESSAGE_COUNT],
    unknown_message: Option<Pointer>,
    /// The program's locale.
    locale:          locale::Locale,
    /// Whether the format being run is a wide one, and whether a conversion
    /// met a character the locale cannot convert, which fails the call.
    wide_format:     bool,
    format_error:    bool,
    /// glibc's character tables, by `strings::ctype_table` slot, once made.
    ctype:           [Option<Pointer>; 3],
    /// Where `strtok` continues.
    strtok:          RuntimeValue,
    scratch:         ArenaVec<'a, u8>,
    text:            ArenaVec<'a, u8>,
    /// Digit space for floating conversions.
    floating:        floating::Scratch<'a>,
    /// The width of `long` in bits, which `printf`'s `l` modifier reads.
    long_width:      u32,
    /// The target's existing long double representation (f64 or unsupported
    /// f80).
    long_double:     Type,
    target:          Target,
}

impl Host for DefaultHost<'_> {
    fn provides(&self, name: &str) -> bool {
        HOST_FUNCTIONS.contains(&name)
            || complex::provides(name)
            || complex::basic_provides(name)
            || checked::provides(name)
            || library::signature(name).is_some()
    }

    fn standard_stream(&self, name: &str) -> Option<u32> {
        stdio::standard_stream(name)
    }

    fn library_object(&self, name: &str) -> Option<LibraryObject> {
        (name == "signgam").then_some(LibraryObject::Signgam)
    }

    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault> {
        // Standard output written by any function follows a `freopen`; a
        // function that writes it implicitly fails when that write does.
        let before = self.output.len();
        let mut result = self.dispatch(call, memory);
        let redirected = self.redirect_output(before, call.name);
        // What remains past `before` went to the captured standard output;
        // a `printf` that met a character it could not convert discards
        // what its staging area held.
        let kind = stdio::written_by(call.name);
        if self.format_error && kind == stdio::Written::Formatted {
            let kept = self
                .console
                .output
                .put_failed_formatted(&self.output, before);
            self.output.truncate(kept);
        } else {
            self.console.output.put(&self.output, before, kind);
        }
        // A stream operation that failed passes its `errno` on.
        if let Some(code) = self.streams.take_errno()
            && result.is_ok()
            && let Err(fault) = self.set_errno(memory, code)
        {
            result = Err(fault);
        }
        // A formatting function fails with EILSEQ on a character the locale
        // cannot convert, after writing what came before it.
        if std::mem::take(&mut self.format_error) && result.is_ok() {
            self.set_errno(memory, errno::EILSEQ)?;
            result = result.map(|value| match value {
                | HostReturn::Value(Some(RuntimeValue::Int(_))) => HostReturn::Value(
                    call.result
                        .map(|ty| RuntimeValue::int(ty, (-1_i32).cast_unsigned().into())),
                ),
                | value => value,
            });
        }
        if redirected || implicit_stream(call.name) != Some(1) {
            return result;
        }
        result.map(|value| match value {
            | HostReturn::Value(Some(RuntimeValue::Int(_))) => HostReturn::Value(
                call.result
                    .map(|ty| RuntimeValue::int(ty, (-1_i32).cast_unsigned().into())),
            ),
            | value => value,
        })
    }

    fn at_exit(&mut self) {
        let _ = self.flush_all();
    }

    fn at_immediate_exit(&mut self) {
        // Only what standard output already released reaches the command
        // line; files keep only what was written out.
        let released = self.console.released(&self.output);
        self.output.truncate(released);
        let written = self.console.error.flushed;
        self.console.errors.truncate(written);
    }
}

impl DefaultHost<'_> {
    fn dispatch(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        if crate::builtins::external_signature(call.name).is_some()
            || HOST_FUNCTIONS.contains(&call.name)
            || checked::provides(call.name)
        {
            check_signature(call, self.long_width, self.long_double)?;
        } else if let Some(signature) = library::signature(call.name) {
            let windows = matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc);
            library::check(call, signature, self.long_width, self.long_double, windows)?;
        }
        // Using a closed standard stream is undefined even without naming it,
        // and a byte function on a wide-oriented stream is undefined.
        if let Some(stream) = implicit_stream(call.name) {
            let _ = self.standard(stream)?;
        }
        if let Some(stream) = self.byte_stream(call, memory)?
            && self.is_wide(stream)
        {
            return Err(UbKind::StreamOrientation.into());
        }
        if let Some(result) = complex::call(call, memory, self.target, self.long_double) {
            return result;
        }
        if let Some(result) = math::call(
            call,
            memory,
            self.long_double,
            self.long_width,
            &mut self.text,
        ) {
            let outcome = result?;
            if outcome.errno != 0 {
                self.set_errno(memory, outcome.errno)?;
            }
            // glibc's `lgamma` family leaves the sign of Γ in `signgam`.
            if let Some(sign) = outcome.signgam
                && let Some(signgam) = memory.library_object(LibraryObject::Signgam)
            {
                memory.store(
                    Type::I32,
                    RuntimeValue::int(Type::I32, u128::from(sign.cast_unsigned())),
                    RuntimeValue::Ptr(signgam),
                    Align::BYTE,
                )?;
            }
            return Ok(HostReturn::Value(Some(outcome.value)));
        }
        if let Some(result) = self.checked(call, memory) {
            return result;
        }
        if let Some(result) = self.errno_functions(call, memory) {
            return result;
        }
        if let Some(result) = self.locale_functions(call, memory) {
            return result;
        }
        if let Some(result) = self.stdio(call, memory) {
            return result;
        }
        if let Some(result) = self.strings(call, memory) {
            return result;
        }
        if let Some(result) = self.quad(call, memory) {
            return result;
        }
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let value = match call.name {
            | "putchar" => {
                let character = truncate_u8(integer(arg(0))?);
                self.output.push(character);
                RuntimeValue::Int(u128::from(character))
            },
            | "puts" => {
                memory.read_c_string(arg(0), &mut self.output)?;
                self.output.push(b'\n');
                RuntimeValue::Int(0)
            },
            | "printf" => {
                let before = self.output.len();
                let mut values = FormatArguments::Values(args.get(1..).unwrap_or_default().iter());
                self.printf(arg(0), &mut values, memory)?;
                RuntimeValue::Int((self.output.len() - before) as u128)
            },
            | "sprintf" | "snprintf" => {
                // C99 §7.19.6.6: format into the destination, including the
                // terminating null, and exclude that null from the count.
                let before = self.output.len();
                // C99 §7.19.6.5p2-3, p. 290; PDF p. 302: snprintf
                // writes at most n-1 bytes plus a null, and returns the full
                // untruncated length; n=0 need not designate any storage.
                let bounded = call.name == "snprintf";
                let limit = if bounded { integer(arg(1))? } else { u128::MAX };
                let format = if bounded { 2 } else { 1 };
                let mut values =
                    FormatArguments::Values(args.get(format + 1..).unwrap_or_default().iter());
                let result = self.printf(arg(format), &mut values, memory);
                let count = self.output.len() - before;
                let result =
                    result.and_then(|()| self.store_formatted(before, arg(0), limit, memory));
                self.output.truncate(before);
                result?;
                RuntimeValue::Int(count as u128)
            },
            | "sscanf" | "__isoc99_sscanf" | "fscanf" | "__isoc99_fscanf" | "scanf"
            | "__isoc99_scanf" | "__isoc23_sscanf" | "__isoc23_fscanf" | "__isoc23_scanf" => {
                let fixed = if call.name.ends_with("_scanf") || call.name == "scanf" {
                    1
                } else {
                    2
                };
                let input = if fixed == 1 {
                    scanf::Input::Stream(0)
                } else if call.name.contains("sscanf") {
                    scanf::Input::String {
                        pointer: arg(0),
                        offset:  0,
                    }
                } else {
                    scanf::Input::Stream(self.stream(arg(0), memory)?)
                };
                let mut values =
                    FormatArguments::Values(args.get(fixed..).unwrap_or_default().iter());
                let dialect = if call.name.starts_with("__isoc23_") {
                    scanf::Dialect::C23
                } else {
                    scanf::Dialect::C99
                };
                let count = self.scanf(input, arg(fixed - 1), &mut values, memory, dialect)?;
                RuntimeValue::int(Type::I32, u128::from(count.cast_unsigned()))
            },
            | "malloc" | "calloc" => {
                let size = if call.name == "malloc" {
                    Some(integer(arg(0))?)
                } else {
                    integer(arg(0))?.checked_mul(integer(arg(1))?)
                };
                let pointer = match size {
                    | Some(size) => allocate(memory, size, call.name == "calloc")?,
                    | None => RuntimeValue::NULL,
                };
                // C99 leaves errno to the implementation; glibc's
                // allocators set ENOMEM when they fail.
                if pointer == RuntimeValue::NULL {
                    self.set_errno(memory, errno::ENOMEM)?;
                }
                pointer
            },
            | "realloc" => {
                // glibc: a null pointer allocates, a zero size frees and
                // returns null, and a failed allocation leaves the old one
                // alone and sets ENOMEM.
                let (old, size) = (arg(0), integer(arg(1))?);
                if old != RuntimeValue::NULL && size == 0 {
                    memory.free_heap(old)?;
                    RuntimeValue::NULL
                } else {
                    let kept = if old == RuntimeValue::NULL {
                        0
                    } else {
                        memory.heap_size(old)?
                    };
                    let pointer = allocate(memory, size, false)?;
                    if pointer == RuntimeValue::NULL {
                        self.set_errno(memory, errno::ENOMEM)?;
                    } else if old != RuntimeValue::NULL {
                        let count = u128::from(kept).min(size);
                        memory.copy(pointer, old, RuntimeValue::Int(count), Align::BYTE, false)?;
                        memory.free_heap(old)?;
                    }
                    pointer
                }
            },
            | "free" => {
                memory.free_heap(arg(0))?;
                return Ok(HostReturn::Value(None));
            },
            | "memcpy" => {
                memory.copy(arg(0), arg(1), arg(2), Align::BYTE, false)?;
                arg(0)
            },
            | "memmove" => {
                memory.copy(arg(0), arg(1), arg(2), Align::BYTE, true)?;
                arg(0)
            },
            | "memcmp" | "strcmp" | "strncmp" => {
                let count = match call.name {
                    | "strcmp" => u128::from(u64::MAX),
                    | _ => integer(arg(2))?,
                };
                compare_bytes(memory, arg(0), arg(1), count, call.name != "memcmp")?
            },
            | "strcpy" => {
                self.scratch.clear();
                memory.read_c_string(arg(1), &mut self.scratch)?;
                let count = RuntimeValue::Int((self.scratch.len() + 1) as u128);
                memory.copy(arg(0), arg(1), count, Align::BYTE, false)?;
                arg(0)
            },
            | "strncpy" => {
                _ = strncpy(memory, arg(0), arg(1), integer(arg(2))?)?;
                arg(0)
            },
            | "memchr" | "strchr" | "strrchr" => {
                // C99: §7.21.5.1-§7.21.5.2, p. 330; PDF p. 342;
                // §7.21.5.5, p. 331; PDF p. 343. Search includes the string
                // terminator; memchr reads only its bounded prefix.
                let count = if call.name == "memchr" {
                    integer(arg(2))?
                } else {
                    u128::from(u64::MAX)
                };
                let count = u64::try_from(count)
                    .map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
                let character = truncate_u8(integer(arg(1))?);
                let mut found = RuntimeValue::NULL;
                for offset in 0..count {
                    let address =
                        memory.ptr_add(arg(0), RuntimeValue::Int(u128::from(offset)), true);
                    let byte = integer(memory.load(Type::I8, address, Align::BYTE)?)?;
                    if byte == u128::from(character) {
                        found = address;
                        if call.name != "strrchr" {
                            break;
                        }
                    }
                    if call.name != "memchr" && byte == 0 {
                        break;
                    }
                }
                found
            },
            | "memset" => {
                memory.fill(arg(0), arg(1), arg(2), Align::BYTE)?;
                arg(0)
            },
            | "strlen" => {
                self.scratch.clear();
                memory.read_c_string(arg(0), &mut self.scratch)?;
                RuntimeValue::Int(self.scratch.len() as u128)
            },
            | "abs" => {
                let value = super::arithmetic::sext(Type::I32, integer(arg(0))?);
                if value == i128::from(i32::MIN) {
                    RuntimeValue::Poison
                } else {
                    RuntimeValue::Int(value.unsigned_abs())
                }
            },
            | "isprint" => {
                let value = super::arithmetic::sext(Type::I32, integer(arg(0))?);
                RuntimeValue::Int(u128::from((32..=126).contains(&value)))
            },
            | "exit" => {
                #[expect(clippy::cast_possible_truncation, reason = "`exit` takes an int.")]
                let status = integer(arg(0))? as u32 as i32;
                return Ok(HostReturn::Exit(status));
            },
            | "abort" => return Ok(HostReturn::Abort),
            | _ => return Ok(HostReturn::NotProvided),
        };
        Ok(HostReturn::Value(call.result.map(|ty| match value {
            | RuntimeValue::Int(bits) if ty.is_int() => RuntimeValue::int(ty, bits),
            | value => value,
        })))
    }
}

/// The operand index of the `va_list` a host function reads, if any.
/// C99: §7.19.6.8-§7.19.6.13, pp. 292-294; PDF pp. 304-306.
pub(crate) fn va_list_operand(name: &str) -> Option<usize> {
    match name {
        | "vprintf" | "vwprintf" => Some(1),
        | "vfprintf" | "vsprintf" | "vfwprintf" => Some(2),
        | "vsnprintf" | "vswprintf" => Some(3),
        | _ => checked::va_list_operand(name),
    }
}

/// The functions [`DefaultHost`] implements.
const HOST_FUNCTIONS: [&str; 26] = [
    "putchar", "puts", "printf", "sprintf", "snprintf", "malloc", "calloc", "free", "memcpy",
    "memmove", "memcmp", "strcmp", "strncmp", "strcpy", "strncpy", "memchr", "strchr", "strrchr",
    "memset", "strlen", "abs", "fabs", "floor", "isprint", "exit", "abort",
];

/// Rejects incompatible calls before a library function performs any effects.
/// All supported targets use i32 int and i64 `size_t`. Open C declarations are
/// checked using their call operands, after default argument promotions.
/// C99: §6.5.2.2 paragraphs 6 and 9, pp. 71-72; PDF pp. 83-84.
fn check_signature(call: HostCall<'_>, long_width: u32, long_double: Type) -> Result<(), Fault> {
    let Some(signature) = crate::builtins::external_signature(call.name) else {
        return Err(UbKind::SignatureMismatch.into());
    };
    if call.result != external_type(signature.result, long_width, long_double)
        || (!signature.variadic && call.arg_types.len() != signature.parameters.len())
        || call.arg_types.len() < signature.parameters.len()
        || call
            .arg_types
            .iter()
            .zip(signature.parameters)
            .any(|(&actual, &expected)| {
                Some(actual) != external_type(expected, long_width, long_double)
            })
    {
        return Err(UbKind::SignatureMismatch.into());
    }
    Ok(())
}

/// `strncpy`: copies at most `count` source bytes, then pads the
/// destination with zeros. A full-length copy need not have a terminating
/// null. Returns how many bytes preceded the source's null, at most `count`.
/// C99: §7.21.2.4 paragraphs 2-3, p. 326; PDF p. 338.
fn strncpy(
    memory: &mut Memory<'_>,
    destination: RuntimeValue,
    source: RuntimeValue,
    count: u128,
) -> Result<u64, Fault> {
    let count = u64::try_from(count).map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
    let copied = string_length(memory, source, count)?;
    // Use the memory copy's overlap check for the accessed prefix.
    let prefix = RuntimeValue::Int(u128::from(copied));
    memory.copy(destination, source, prefix, Align::BYTE, false)?;
    let tail = memory.ptr_add(destination, prefix, true);
    memory.fill(
        tail,
        RuntimeValue::Int(0),
        RuntimeValue::Int(u128::from(count - copied)),
        Align::BYTE,
    )?;
    Ok(copied)
}

/// The bytes before the first null at `string`, reading at most `limit`.
/// C99: §7.1.1 paragraph 1, p. 164; PDF p. 176.
fn string_length(memory: &Memory<'_>, string: RuntimeValue, limit: u64) -> Result<u64, Fault> {
    for offset in 0..limit {
        let address = memory.ptr_add(string, RuntimeValue::Int(u128::from(offset)), true);
        if integer(memory.load(Type::I8, address, Align::BYTE)?)? == 0 {
            return Ok(offset);
        }
    }
    Ok(limit)
}

/// Compares bytes as unsigned char, stopping at n or the first string null.
/// Reading only the selected prefix permits nonterminated strncmp arrays.
/// C99: §7.21.4 paragraphs 1-2, p. 328; PDF p. 340;
/// §7.21.4.1-§7.21.4.4, pp. 328-330; PDF pp. 340-342.
fn compare_bytes(
    memory: &Memory<'_>,
    left: RuntimeValue,
    right: RuntimeValue,
    count: u128,
    string: bool,
) -> Result<RuntimeValue, Fault> {
    let count = u64::try_from(count).map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
    for offset in 0..count {
        let at = |pointer| memory.ptr_add(pointer, RuntimeValue::Int(u128::from(offset)), true);
        let a = integer(memory.load(Type::I8, at(left), Align::BYTE)?)?;
        let b = integer(memory.load(Type::I8, at(right), Align::BYTE)?)?;
        if a != b {
            let difference = a.wrapping_sub(b);
            return Ok(RuntimeValue::int(Type::I32, difference));
        }
        if string && a == 0 {
            break;
        }
    }
    Ok(RuntimeValue::Int(0))
}

impl DefaultHost<'_> {
    /// Writes the formatted bytes after `before` to `destination`: at most
    /// `limit - 1` of them and a terminating null, or nothing for a zero
    /// `limit`. C99: §7.19.6.5p2, p. 290; PDF p. 302; §7.19.6.6p2, p. 291;
    /// PDF p. 303.
    fn store_formatted(
        &self,
        before: usize,
        destination: RuntimeValue,
        limit: u128,
        memory: &mut Memory<'_>,
    ) -> Result<(), Fault> {
        let count = self.output.len() - before;
        let written = if limit == 0 {
            0
        } else {
            count.min(usize::try_from(limit - 1).unwrap_or(usize::MAX))
        };
        for (offset, byte) in self.output[before..before + written]
            .iter()
            .copied()
            .chain((limit != 0).then_some(0))
            .enumerate()
        {
            let address = memory.ptr_add(destination, RuntimeValue::Int(offset as u128), true);
            memory.store(
                Type::I8,
                RuntimeValue::Int(u128::from(byte)),
                address,
                Align::BYTE,
            )?;
        }
        Ok(())
    }

    /// Everything the program has written.
    pub(crate) fn output(&self) -> &[u8] {
        &self.output
    }

    /// Everything the program has written to `stderr`.
    pub(crate) fn errors(&self) -> &[u8] {
        &self.console.errors
    }

    /// Lets the program open, create and remove host files; without it, those
    /// calls fail as for a permission error.
    pub(crate) fn allow_file_access(&mut self) {
        self.file_access = true;
    }

    /// Treats standard output as a terminal, which glibc line-buffers rather
    /// than fully buffering as for a pipe or file.
    pub(crate) fn set_terminal_output(&mut self, terminal: bool) {
        if terminal {
            self.console.output.mode = stdio::Mode::Line;
            self.console.output.block = stdio::TERMINAL_BUFFER;
        } else {
            self.console.output.mode = stdio::Mode::Full;
        }
    }

    /// The buffer sizes glibc would give standard output and error, from
    /// their descriptors' `st_blksize` (4096 for a Linux pipe); `BUFSIZ`
    /// otherwise.
    pub(crate) fn set_console_blocks(&mut self, output: Option<usize>, errors: Option<usize>) {
        let size = |block: Option<usize>| match block {
            | Some(block) if block > 0 && block < stdio::BUFSIZ => block,
            | _ => stdio::BUFSIZ,
        };
        if self.console.output.mode != stdio::Mode::Line {
            self.console.output.block = size(output);
        }
        self.console.error.block = size(errors);
    }

    /// Treats standard input as a terminal, which glibc line-buffers:
    /// refilling its buffer flushes a terminal's stdout.
    pub(crate) fn set_terminal_input(&mut self, terminal: bool) {
        self.streams.set_terminal_input(terminal);
    }

    /// How many file-system accesses were refused, with the first path, if
    /// any were.
    pub(crate) fn denied_file_access(&self) -> Option<(u32, &[u8])> {
        (self.denied_files > 0).then_some((self.denied_files, &self.denied_path[..]))
    }

    /// Passes what the program wrote to `write`, chunk by chunk, in the order
    /// it reached the standard streams: each `stderr` write after the
    /// `stdout` bytes flushed before it, and the unflushed rest of `stdout`
    /// at the end, as at the program's exit.
    pub(crate) fn emit<E>(
        &self,
        mut write: impl FnMut(Channel, &[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        let (mut output, mut errors) = (0, 0);
        for &(flushed, end) in &self.console.marks {
            let flushed = flushed.min(self.output.len());
            if flushed > output {
                write(Channel::Output, &self.output[output..flushed])?;
                output = flushed;
            }
            write(Channel::Errors, &self.console.errors[errors..end])?;
            errors = end;
        }
        if output < self.output.len() {
            write(Channel::Output, &self.output[output..])?;
        }
        if errors < self.console.errors.len() {
            write(Channel::Errors, &self.console.errors[errors..])?;
        }
        Ok(())
    }
}

/// A standard stream of the command line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Channel {
    Output,
    Errors,
}

/// The standard stream a C library function reads or writes without a
/// `FILE *` argument.
fn implicit_stream(name: &str) -> Option<usize> {
    match name {
        | "printf" | "puts" | "putchar" | "vprintf" | "__printf_chk" | "__vprintf_chk"
        | "putwchar" | "wprintf" | "vwprintf" => Some(1),
        | "getchar" | "scanf" | "__isoc99_scanf" | "__isoc23_scanf" | "getwchar" | "wscanf"
        | "__isoc99_wscanf" | "__isoc23_wscanf" => Some(0),
        | "perror" => Some(2),
        | _ => None,
    }
}

/// The payload glibc gives `nan("sequence")` and `nan(sequence)` input: an
/// n-char-sequence (letters, digits and `_`) that `strtoull` reads whole in
/// base 0, with a `0x` or `0` prefix, saturating at `ULLONG_MAX` when it
/// overflows; zero for anything else, a sign or white space included.
/// C99: §7.12.11.2 paragraph 2, p. 237; PDF p. 249; §7.20.1.3 paragraph 3,
/// p. 308; PDF p. 320.
fn nan_payload(text: &str) -> u64 {
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return 0;
    }
    let (digits, radix) =
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (hex, 16)
        } else if text.len() > 1 && text.starts_with('0') {
            (&text[1..], 8)
        } else {
            (text, 10)
        };
    if digits.is_empty() || !digits.chars().all(|character| character.is_digit(radix)) {
        return 0;
    }
    let value = digits.chars().try_fold(0_u64, |value, character| {
        let digit = u64::from(character.to_digit(radix).expect("a checked digit"));
        value.checked_mul(u64::from(radix))?.checked_add(digit)
    });
    value.unwrap_or(u64::MAX)
}

/// An integer argument, which must not be poison.
fn integer(value: RuntimeValue) -> Result<u128, Fault> {
    value.as_int().ok_or(UbKind::PoisonInHostCall.into())
}

/// `malloc` or `calloc`: a heap object of `size` bytes, or null if it is
/// larger than the interpreter allows. `calloc`'s bytes are zero; `malloc`'s
/// are poison until written.
///
/// C99: §7.20.3.1 and §7.20.3.3, pp. 313-314; PDF pp. 325-326.
fn allocate(memory: &mut Memory<'_>, size: u128, zeroed: bool) -> Result<RuntimeValue, Fault> {
    let Ok(size) = u64::try_from(size) else {
        return Ok(RuntimeValue::NULL);
    };
    let align = Align::from_bytes(16).expect("16 is a power of two");
    let pointer: Pointer = match memory.allocate(ObjectKind::Heap, size, align) {
        | Ok(pointer) => pointer,
        | Err(Fault::Unsupported(_)) => return Ok(RuntimeValue::NULL),
        | Err(fault) => return Err(fault),
    };
    if zeroed {
        let pointer = RuntimeValue::Ptr(pointer);
        memory.fill(
            pointer,
            RuntimeValue::Int(0),
            RuntimeValue::Int(u128::from(size)),
            Align::BYTE,
        )?;
    }
    Ok(RuntimeValue::Ptr(pointer))
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}

impl<'a> DefaultHost<'a> {
    /// A host whose output and scratch space live in `arena`.
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            allocator:       arena,
            output:          ArenaVec::new_in(arena),
            console:         stdio::Console {
                errors: ArenaVec::new_in(arena),
                marks:  ArenaVec::new_in(arena),
                output: stdio::Channel::new(stdio::Mode::Full),
                error:  stdio::Channel::new(stdio::Mode::Unbuffered),
            },
            file_access:     false,
            denied_files:    0,
            denied_path:     ArenaVec::new_in(arena),
            streams:         stdio::Streams::new_in(arena),
            path:            ArenaVec::new_in(arena),
            tmpnam_buffer:   None,
            errno:           None,
            messages:        [None; errno::MESSAGE_COUNT],
            unknown_message: None,
            locale:          locale::Locale::new(),
            wide_format:     false,
            format_error:    false,
            ctype:           [None; 3],
            strtok:          RuntimeValue::NULL,
            scratch:         ArenaVec::new_in(arena),
            text:            ArenaVec::new_in(arena),
            floating:        floating::Scratch {
                integer:     ArenaVec::new_in(arena),
                significant: ArenaVec::new_in(arena),
            },
            long_width:      64,
            long_double:     Type::F80,
            target:          Target::LinuxGnu,
        }
    }

    /// A host in `arena` whose C library has `target`'s type widths:
    /// `long` is 64 bits under LP64 (Linux) and 32 under LLP64 (Windows).
    pub(crate) fn for_target(arena: &'a Bump, target: Target) -> Self {
        let long_width = target
            .layout()
            .integer(Scalar::Long)
            .map_or(64, |(bits, _)| bits);
        Self {
            target,
            long_width,
            long_double: if target.layout().long_double_is_double() {
                Type::F64
            } else {
                Type::F80
            },
            ..Self::new_in(arena)
        }
    }
}

/// Projects a neutral external signature to the interpreter's scalar ABI.
/// All current targets have 64-bit pointers and `size_t`; `long` and `long
/// double` follow the configured target. No semantic types enter the host.
fn external_type(
    pattern: crate::builtins::SignatureType,
    long_width: u32,
    long_double: Type,
) -> Option<Type> {
    use crate::{
        builtins::SignatureType as T,
        target::Scalar as S,
    };
    Some(match pattern {
        | T::Scalar(S::Bool | S::Char | S::SignedChar | S::UnsignedChar) => Type::I8,
        | T::Scalar(S::Short | S::UnsignedShort) => Type::I16,
        | T::Scalar(S::Int | S::UnsignedInt) => Type::I32,
        | T::Scalar(S::Long | S::UnsignedLong) if long_width == 32 => Type::I32,
        | T::Scalar(S::Long | S::UnsignedLong | S::LongLong | S::UnsignedLongLong) | T::Size =>
            Type::I64,
        | T::Scalar(S::Int128 | S::UnsignedInt128) => Type::I128,
        | T::Scalar(S::Float) => Type::F32,
        | T::Scalar(S::Double) => Type::F64,
        | T::Scalar(S::LongDouble) => long_double,
        | T::Pointer(..) | T::VaList | T::File | T::VectorPointer(..) => Type::Ptr,
        | _ => return None,
    })
}

#[cfg(test)]
#[test]
fn m7_external_signatures_accept_cataloged_host_calls() {
    for name in HOST_FUNCTIONS.iter().copied().chain([
        "__memcpy_chk",
        "__memmove_chk",
        "__mempcpy_chk",
        "__memset_chk",
        "__strcpy_chk",
        "__stpcpy_chk",
        "__strncpy_chk",
        "__stpncpy_chk",
        "__strcat_chk",
        "__strncat_chk",
        "__sprintf_chk",
        "__snprintf_chk",
        "__vsprintf_chk",
        "__vsnprintf_chk",
        "__vprintf_chk",
        "__vfprintf_chk",
    ]) {
        let signature = crate::builtins::external_signature(name).expect("cataloged signature");
        let arena = Bump::new();
        let mut params = ArenaVec::new_in(&arena);
        params.extend(
            signature
                .parameters
                .iter()
                .map(|&p| external_type(p, 64, Type::F80).unwrap()),
        );
        let call = HostCall {
            name,
            args: &[],
            arg_types: &params,
            result: external_type(signature.result, 64, Type::F80),
            va_list: HostVaList::None,
        };
        assert!(check_signature(call, 64, Type::F80).is_ok(), "{name}");
    }
}
