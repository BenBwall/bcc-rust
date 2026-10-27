//! `errno` and the functions that describe it, as glibc on GNU/Linux keeps
//! them: `__errno_location`, which glibc's `<errno.h>` reads `errno`
//! through, `strerror`, both forms of `strerror_r`, and `perror`.
//!
//! `errno` is one `int` object in the program's memory, zero at startup,
//! which the program reads and writes through the pointer
//! `__errno_location` returns and the host functions set when glibc's do:
//! the allocation, conversion, stream and file functions here and the math
//! functions (`math_errhandling` is `MATH_ERRNO | MATH_ERREXCEPT`). No host
//! function clears it. The codes are Linux's (`<asm-generic/errno.h>`) and
//! the messages glibc's "C" locale texts (`sysdeps/gnu/errlist.h`, the same
//! in glibc 2.39 and 2.43); an unknown code reads "Unknown error N".
//! `strerror_r` is the GNU form, which returns its message, possibly
//! without touching the buffer; glibc's headers map the XSI form, which
//! returns an error number, to `__xpg_strerror_r` when `_GNU_SOURCE` is
//! not defined and `_POSIX_C_SOURCE` is at least 200112L.
//!
//! C99: §7.5 paragraphs 2-3, p. 186; PDF p. 198 (`errno`); §7.21.6.2, p. 334; PDF p. 346
//! (`strerror`); §7.19.10.4, p. 305; PDF p. 317 (`perror`).

use std::io;

use super::{
    DefaultHost,
    HostCall,
    HostReturn,
    integer,
};
use crate::{
    backend::interpreter::{
        arithmetic::sext,
        memory::{
            Memory,
            ObjectKind,
        },
        trap::Fault,
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
    "__errno_location",
    "strerror",
    "strerror_r",
    "__xpg_strerror_r",
    "perror",
];

/// The Linux error numbers the host sets.
pub(super) const EPERM: i32 = 1;
pub(super) const ENOENT: i32 = 2;
pub(super) const E2BIG: i32 = 7;
pub(super) const EBADF: i32 = 9;
pub(super) const ENOMEM: i32 = 12;
pub(super) const EACCES: i32 = 13;
pub(super) const EEXIST: i32 = 17;
pub(super) const ENOTDIR: i32 = 20;
pub(super) const EISDIR: i32 = 21;
pub(super) const EINVAL: i32 = 22;
pub(super) const EMFILE: i32 = 24;
pub(super) const ESPIPE: i32 = 29;
pub(super) const EDOM: i32 = 33;
pub(super) const ERANGE: i32 = 34;
pub(super) const EILSEQ: i32 = 84;

impl DefaultHost<'_> {
    /// Runs an `errno` function, or `None` for another name.
    pub(super) fn errno_functions(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        FUNCTIONS
            .contains(&call.name)
            .then(|| self.run_errno(call, memory))
    }

    fn run_errno(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let code = |value: RuntimeValue| -> Result<i32, Fault> {
            Ok(i32::try_from(sext(Type::I32, integer(value)?)).expect("an int"))
        };
        let value = match call.name {
            | "__errno_location" => RuntimeValue::Ptr(self.errno_object(memory)?),
            | "strerror" => self.strerror(code(arg(0))?, memory)?,
            | "strerror_r" => {
                // The GNU form: a known code's own message, else the
                // unknown-code text formatted into the buffer.
                let errnum = code(arg(0))?;
                match message(errnum) {
                    | Some(_) => self.strerror(errnum, memory)?,
                    | None => {
                        unknown(errnum, &mut self.text);
                        self.store_truncated(arg(1), integer(arg(2))?, memory)?;
                        arg(1)
                    },
                }
            },
            | "__xpg_strerror_r" => {
                // The XSI form copies the message, truncated, and reports a
                // code it does not know as `EINVAL` and a buffer too small
                // as `ERANGE` (`string/xpg-strerror.c`).
                let errnum = code(arg(0))?;
                let size = integer(arg(2))?;
                self.text.clear();
                let result = match message(errnum) {
                    | Some(text) => {
                        self.text.extend_from_slice(text.as_bytes());
                        if size <= text.len() as u128 { ERANGE } else { 0 }
                    },
                    | None => {
                        unknown(errnum, &mut self.text);
                        EINVAL
                    },
                };
                self.store_truncated(arg(1), size, memory)?;
                RuntimeValue::int(Type::I32, u128::from(result.cast_unsigned()))
            },
            | "perror" => {
                self.perror(arg(0), memory)?;
                return Ok(HostReturn::Value(None));
            },
            | _ => unreachable!("`FUNCTIONS` lists every errno function"),
        };
        Ok(HostReturn::Value(Some(value)))
    }

    /// Sets `errno` to `code`, as a library function that fails does.
    pub(super) fn set_errno(&mut self, memory: &mut Memory<'_>, code: i32) -> Result<(), Fault> {
        let object = self.errno_object(memory)?;
        memory.store(
            Type::I32,
            RuntimeValue::int(Type::I32, u128::from(code.cast_unsigned())),
            RuntimeValue::Ptr(object),
            Align::from_bytes(4).expect("a power of two"),
        )
    }

    /// The program's `errno` object, zero until something sets it, made the
    /// first time anything reads or sets it.
    fn errno_object(&mut self, memory: &mut Memory<'_>) -> Result<Pointer, Fault> {
        if let Some(object) = self.errno {
            return Ok(object);
        }
        let align = Align::from_bytes(4).expect("a power of two");
        let object = memory.allocate(ObjectKind::Global, 4, align)?;
        memory.initialize(object, &[]);
        self.errno = Some(object);
        Ok(object)
    }

    /// `strerror`: glibc's message for `errnum`, a constant string, or for an
    /// unknown code "Unknown error N" in a buffer the next such call frees,
    /// as glibc's per-thread buffer is.
    /// C99: §7.21.6.2 paragraph 2, p. 334; PDF p. 346.
    fn strerror(&mut self, errnum: i32, memory: &mut Memory<'_>) -> Result<RuntimeValue, Fault> {
        if let Some(text) = message(errnum) {
            let slot = usize::try_from(errnum).expect("a known code");
            if let Some(object) = self.messages[slot] {
                return Ok(RuntimeValue::Ptr(object));
            }
            let object = constant_string(memory, ObjectKind::ConstantGlobal, text.as_bytes())?;
            self.messages[slot] = Some(object);
            return Ok(RuntimeValue::Ptr(object));
        }
        if let Some(previous) = self.unknown_message.take() {
            memory.free(previous.provenance.expect("host objects have provenance"));
        }
        unknown(errnum, &mut self.text);
        let object = constant_string(memory, ObjectKind::Global, &self.text)?;
        self.unknown_message = Some(object);
        Ok(RuntimeValue::Ptr(object))
    }

    /// Writes the text in `self.text` to `buffer` as `snprintf` with `size`
    /// would: at most `size - 1` bytes and a terminating null, or nothing
    /// for a zero size.
    fn store_truncated(
        &self,
        buffer: RuntimeValue,
        size: u128,
        memory: &mut Memory<'_>,
    ) -> Result<(), Fault> {
        if size == 0 {
            return Ok(());
        }
        let length = self
            .text
            .len()
            .min(usize::try_from(size - 1).unwrap_or(usize::MAX));
        for (offset, &byte) in self.text[..length].iter().chain(&[0]).enumerate() {
            let at = memory.ptr_add(buffer, RuntimeValue::Int(offset as u128), true);
            memory.store(
                Type::I8,
                RuntimeValue::Int(u128::from(byte)),
                at,
                Align::BYTE,
            )?;
        }
        Ok(())
    }

    /// `perror`: writes `s: ` (unless `s` is null or empty), the message for
    /// the current `errno` and a newline to `stderr`, without orienting it
    /// (glibc writes through a duplicate of an unoriented stream).
    /// C99: §7.19.10.4 paragraph 2, p. 305; PDF p. 317.
    fn perror(&mut self, prefix: RuntimeValue, memory: &mut Memory<'_>) -> Result<(), Fault> {
        let object = self.errno_object(memory)?;
        let errnum = integer(memory.load(
            Type::I32,
            RuntimeValue::Ptr(object),
            Align::from_bytes(4).expect("a power of two"),
        )?)?;
        let errnum = i32::try_from(sext(Type::I32, errnum)).expect("an int");
        let before = self.output.len();
        if prefix != RuntimeValue::NULL {
            memory.read_c_string(prefix, &mut self.output)?;
            if self.output.len() > before {
                self.output.extend_from_slice(b": ");
            }
        }
        match message(errnum) {
            | Some(text) => self.output.extend_from_slice(text.as_bytes()),
            | None => {
                unknown(errnum, &mut self.text);
                self.output.extend_from_slice(&self.text);
            },
        }
        self.output.push(b'\n');
        let stderr = self.standard(2)?;
        let _ = self.write_unoriented(stderr, before);
        Ok(())
    }
}

/// glibc's message for a known `errnum`.
fn message(errnum: i32) -> Option<&'static str> {
    MESSAGES
        .get(usize::try_from(errnum).ok()?)
        .copied()
        .flatten()
}

/// Replaces `text` with glibc's text for a code it has no message for.
fn unknown(errnum: i32, text: &mut super::ArenaVec<'_, u8>) {
    text.clear();
    text.extend_from_slice(b"Unknown error ");
    if errnum < 0 {
        text.push(b'-');
    }
    let start = text.len();
    let mut magnitude = errnum.unsigned_abs();
    loop {
        text.insert(start, b'0' + u8::try_from(magnitude % 10).expect("a digit"));
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
}

/// A fresh object of `kind` holding `bytes` and a terminating null.
fn constant_string(memory: &mut Memory<'_>, kind: ObjectKind, bytes: &[u8]) -> Result<Pointer, Fault> {
    let object = memory.allocate(kind, bytes.len() as u64 + 1, Align::BYTE)?;
    memory.initialize(object, bytes);
    Ok(object)
}

/// The `errno` an I/O error of the host's file system reports: the
/// system's own code on Unix, where it is Linux's or close to it, and the
/// nearest Linux code for an error kind elsewhere.
pub(super) fn from_io(error: &io::Error) -> i32 {
    #[cfg(unix)]
    if let Some(code) = error.raw_os_error() {
        return code;
    }
    match error.kind() {
        | io::ErrorKind::NotFound => ENOENT,
        | io::ErrorKind::PermissionDenied => EACCES,
        | io::ErrorKind::AlreadyExists => EEXIST,
        | io::ErrorKind::NotADirectory => ENOTDIR,
        | io::ErrorKind::IsADirectory => EISDIR,
        | io::ErrorKind::NotSeekable => ESPIPE,
        | io::ErrorKind::OutOfMemory => ENOMEM,
        | _ => EINVAL,
    }
}

/// glibc's messages by Linux error number; `None` where Linux defines no
/// code.
const MESSAGES: [Option<&str>; MESSAGE_COUNT] = [
    Some("Success"),
    Some("Operation not permitted"),
    Some("No such file or directory"),
    Some("No such process"),
    Some("Interrupted system call"),
    Some("Input/output error"),
    Some("No such device or address"),
    Some("Argument list too long"),
    Some("Exec format error"),
    Some("Bad file descriptor"),
    Some("No child processes"),
    Some("Resource temporarily unavailable"),
    Some("Cannot allocate memory"),
    Some("Permission denied"),
    Some("Bad address"),
    Some("Block device required"),
    Some("Device or resource busy"),
    Some("File exists"),
    Some("Invalid cross-device link"),
    Some("No such device"),
    Some("Not a directory"),
    Some("Is a directory"),
    Some("Invalid argument"),
    Some("Too many open files in system"),
    Some("Too many open files"),
    Some("Inappropriate ioctl for device"),
    Some("Text file busy"),
    Some("File too large"),
    Some("No space left on device"),
    Some("Illegal seek"),
    Some("Read-only file system"),
    Some("Too many links"),
    Some("Broken pipe"),
    Some("Numerical argument out of domain"),
    Some("Numerical result out of range"),
    Some("Resource deadlock avoided"),
    Some("File name too long"),
    Some("No locks available"),
    Some("Function not implemented"),
    Some("Directory not empty"),
    Some("Too many levels of symbolic links"),
    None,
    Some("No message of desired type"),
    Some("Identifier removed"),
    Some("Channel number out of range"),
    Some("Level 2 not synchronized"),
    Some("Level 3 halted"),
    Some("Level 3 reset"),
    Some("Link number out of range"),
    Some("Protocol driver not attached"),
    Some("No CSI structure available"),
    Some("Level 2 halted"),
    Some("Invalid exchange"),
    Some("Invalid request descriptor"),
    Some("Exchange full"),
    Some("No anode"),
    Some("Invalid request code"),
    Some("Invalid slot"),
    None,
    Some("Bad font file format"),
    Some("Device not a stream"),
    Some("No data available"),
    Some("Timer expired"),
    Some("Out of streams resources"),
    Some("Machine is not on the network"),
    Some("Package not installed"),
    Some("Object is remote"),
    Some("Link has been severed"),
    Some("Advertise error"),
    Some("Srmount error"),
    Some("Communication error on send"),
    Some("Protocol error"),
    Some("Multihop attempted"),
    Some("RFS specific error"),
    Some("Bad message"),
    Some("Value too large for defined data type"),
    Some("Name not unique on network"),
    Some("File descriptor in bad state"),
    Some("Remote address changed"),
    Some("Can not access a needed shared library"),
    Some("Accessing a corrupted shared library"),
    Some(".lib section in a.out corrupted"),
    Some("Attempting to link in too many shared libraries"),
    Some("Cannot exec a shared library directly"),
    Some("Invalid or incomplete multibyte or wide character"),
    Some("Interrupted system call should be restarted"),
    Some("Streams pipe error"),
    Some("Too many users"),
    Some("Socket operation on non-socket"),
    Some("Destination address required"),
    Some("Message too long"),
    Some("Protocol wrong type for socket"),
    Some("Protocol not available"),
    Some("Protocol not supported"),
    Some("Socket type not supported"),
    Some("Operation not supported"),
    Some("Protocol family not supported"),
    Some("Address family not supported by protocol"),
    Some("Address already in use"),
    Some("Cannot assign requested address"),
    Some("Network is down"),
    Some("Network is unreachable"),
    Some("Network dropped connection on reset"),
    Some("Software caused connection abort"),
    Some("Connection reset by peer"),
    Some("No buffer space available"),
    Some("Transport endpoint is already connected"),
    Some("Transport endpoint is not connected"),
    Some("Cannot send after transport endpoint shutdown"),
    Some("Too many references: cannot splice"),
    Some("Connection timed out"),
    Some("Connection refused"),
    Some("Host is down"),
    Some("No route to host"),
    Some("Operation already in progress"),
    Some("Operation now in progress"),
    Some("Stale file handle"),
    Some("Structure needs cleaning"),
    Some("Not a XENIX named type file"),
    Some("No XENIX semaphores available"),
    Some("Is a named type file"),
    Some("Remote I/O error"),
    Some("Disk quota exceeded"),
    Some("No medium found"),
    Some("Wrong medium type"),
    Some("Operation canceled"),
    Some("Required key not available"),
    Some("Key has expired"),
    Some("Key has been revoked"),
    Some("Key was rejected by service"),
    Some("Owner died"),
    Some("State not recoverable"),
    Some("Operation not possible due to RF-kill"),
    Some("Memory page has hardware error"),
];

/// How many codes [`MESSAGES`] covers: 0 to 133 (`EHWPOISON`).
pub(super) const MESSAGE_COUNT: usize = 134;
