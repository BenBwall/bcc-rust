//! The object-size-checking `__*_chk` entry points of fortified C libraries
//! (glibc's `debug/*_chk.c`), which the checked builtins of
//! `builtins/checked.rs` call. Each first checks the destination size its
//! caller computed, then behaves as the unchecked function. Where the
//! operation would write past that size, the C library reports the overflow
//! and aborts (`__chk_fail`): the host ends the program as `abort` does,
//! before writing any byte, so the overrun never happens. A size of
//! `(size_t)-1` means unknown and admits any length, as in the library.
//!
//! The `flag` operand of the formatting functions enables the library's
//! extra `%n` checks; the host's `printf` implements no `%n`, so every flag
//! formats alike. `__vfprintf_chk` writes to a `FILE`, which the host does
//! not model: it is not provided, and a call traps as an unknown function.
//!
//! GNU extension: GCC "Object Size Checking"; Linux Standard Base Core,
//! "Interfaces for libc" (`__memcpy_chk` ... `__vsnprintf_chk`).
//! C99: §7.21.2.1-§7.21.2.4, pp. 325-326; PDF pp. 337-338; §7.21.3.1-
//! §7.21.3.2, pp. 326-327; PDF pp. 338-339; §7.21.6.1, p. 333; PDF p. 345;
//! §7.19.6.5-§7.19.6.6, pp. 290-291; PDF pp. 302-303; §7.19.6.10,
//! §7.19.6.12-§7.19.6.13, pp. 293-294; PDF pp. 305-306.

use super::{
    DefaultHost,
    FormatArguments,
    HostCall,
    HostReturn,
    integer,
    string_length,
    strncpy,
};
use crate::{
    backend::interpreter::{
        memory::Memory,
        trap::{
            Fault,
            Unsupported,
        },
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
};

/// The checked functions the host runs.
const FUNCTIONS: [&str; 15] = [
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
];

pub(super) fn provides(name: &str) -> bool {
    FUNCTIONS.contains(&name)
}

/// The operand index of the `va_list` a checked function reads, if any.
pub(crate) fn va_list_operand(name: &str) -> Option<usize> {
    match name {
        | "__vprintf_chk" => Some(2),
        | "__vfprintf_chk" => Some(3),
        | "__vsprintf_chk" => Some(4),
        | "__vsnprintf_chk" => Some(5),
        | _ => None,
    }
}

impl DefaultHost<'_> {
    /// Runs a checked function, or `None` for another name.
    pub(super) fn checked(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        if !provides(call.name) {
            return None;
        }
        Some(self.run_checked(call, memory).map(|outcome| match outcome {
            | Checked::Value(value) => HostReturn::Value(Some(match value {
                | RuntimeValue::Int(bits) => RuntimeValue::int(Type::I32, bits),
                | value => value,
            })),
            | Checked::Overflow => HostReturn::Abort,
        }))
    }

    fn run_checked(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<Checked, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let offset = |memory: &Memory<'_>, pointer, bytes: u64| {
            memory.ptr_add(pointer, RuntimeValue::Int(u128::from(bytes)), true)
        };
        let (destination, source) = (arg(0), arg(1));
        Ok(Checked::Value(match call.name {
            | "__memcpy_chk" | "__memmove_chk" | "__mempcpy_chk" => {
                let count = integer(arg(2))?;
                if integer(arg(3))? < count {
                    return Ok(Checked::Overflow);
                }
                let overlap = call.name == "__memmove_chk";
                memory.copy(destination, source, arg(2), Align::BYTE, overlap)?;
                if call.name == "__mempcpy_chk" {
                    offset(memory, destination, bytes(count)?)
                } else {
                    destination
                }
            },
            | "__memset_chk" => {
                if integer(arg(3))? < integer(arg(2))? {
                    return Ok(Checked::Overflow);
                }
                memory.fill(destination, arg(1), arg(2), Align::BYTE)?;
                destination
            },
            | "__strcpy_chk" | "__stpcpy_chk" => {
                let length = c_string_length(memory, source)?;
                if u128::from(length) >= integer(arg(2))? {
                    return Ok(Checked::Overflow);
                }
                let count = RuntimeValue::Int(u128::from(length) + 1);
                memory.copy(destination, source, count, Align::BYTE, false)?;
                if call.name == "__stpcpy_chk" {
                    offset(memory, destination, length)
                } else {
                    destination
                }
            },
            | "__strncpy_chk" | "__stpncpy_chk" => {
                let count = integer(arg(2))?;
                if integer(arg(3))? < count {
                    return Ok(Checked::Overflow);
                }
                let copied = strncpy(memory, destination, source, count)?;
                if call.name == "__stpncpy_chk" {
                    offset(memory, destination, copied)
                } else {
                    destination
                }
            },
            | "__strcat_chk" | "__strncat_chk" => {
                let size = integer(arg(if call.name == "__strcat_chk" { 2 } else { 3 }))?;
                let limit = u64::try_from(size).unwrap_or(u64::MAX);
                // The destination's string must end within its object.
                let existing = string_length(memory, destination, limit)?;
                let appended = if call.name == "__strcat_chk" {
                    c_string_length(memory, source)?
                } else {
                    string_length(memory, source, bytes(integer(arg(2))?)?)?
                };
                let needed = u128::from(existing) + u128::from(appended) + 1;
                if existing == limit || needed > size {
                    return Ok(Checked::Overflow);
                }
                let end = offset(memory, destination, existing);
                memory.copy(
                    end,
                    source,
                    RuntimeValue::Int(u128::from(appended)),
                    Align::BYTE,
                    false,
                )?;
                memory.store(
                    Type::I8,
                    RuntimeValue::Int(0),
                    offset(memory, end, appended),
                    Align::BYTE,
                )?;
                destination
            },
            | "__sprintf_chk" | "__vsprintf_chk" => {
                let size = integer(arg(2))?;
                if size == 0 {
                    return Ok(Checked::Overflow);
                }
                let mut values = Self::format_arguments(call, 3, memory)?;
                return self.format_checked(destination, None, size, arg(3), &mut values, memory);
            },
            | "__snprintf_chk" | "__vsnprintf_chk" => {
                let limit = integer(arg(1))?;
                let size = integer(arg(3))?;
                if size < limit {
                    return Ok(Checked::Overflow);
                }
                let mut values = Self::format_arguments(call, 4, memory)?;
                return self.format_checked(
                    destination,
                    Some(limit),
                    size,
                    arg(4),
                    &mut values,
                    memory,
                );
            },
            | "__vprintf_chk" => {
                let mut values = FormatArguments::from_va_list(arg(2), call.va_list, memory)?;
                let before = self.output.len();
                self.printf(arg(1), &mut values, memory)?;
                RuntimeValue::Int((self.output.len() - before) as u128)
            },
            | _ => unreachable!("`provides` lists every checked function"),
        }))
    }

    /// The arguments after the format at `format`: the call's own variadic
    /// operands, or those of its `va_list` operand.
    fn format_arguments<'c>(
        call: HostCall<'c>,
        format: usize,
        memory: &Memory<'_>,
    ) -> Result<FormatArguments<'c>, Fault> {
        match va_list_operand(call.name) {
            | Some(list) => FormatArguments::from_va_list(call.args[list], call.va_list, memory),
            | None => Ok(FormatArguments::Values(
                call.args.get(format + 1..).unwrap_or_default().iter(),
            )),
        }
    }

    /// Formats into `destination`, which holds `size` bytes: everything and
    /// a null for `sprintf` (`limit` `None`), at most `limit - 1` bytes and a
    /// null for `snprintf`. Output that does not fit is an overflow, found
    /// before any byte is written.
    fn format_checked(
        &mut self,
        destination: RuntimeValue,
        limit: Option<u128>,
        size: u128,
        format: RuntimeValue,
        values: &mut FormatArguments<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<Checked, Fault> {
        let before = self.output.len();
        let result = self.printf(format, values, memory);
        let count = (self.output.len() - before) as u128;
        let stored = limit.unwrap_or(count + 1);
        let result = result.and_then(|()| {
            if limit.is_none() && count >= size {
                return Ok(false);
            }
            self.store_formatted(before, destination, stored, memory)
                .map(|()| true)
        });
        self.output.truncate(before);
        Ok(if result? {
            Checked::Value(RuntimeValue::Int(count))
        } else {
            Checked::Overflow
        })
    }
}

/// How a checked function ended.
enum Checked {
    Value(RuntimeValue),
    /// The operation would have written past the destination's size.
    Overflow,
}

/// The length of the C string at `string`, which must end in its object.
fn c_string_length(memory: &Memory<'_>, string: RuntimeValue) -> Result<u64, Fault> {
    string_length(memory, string, u64::MAX)
}

/// A byte count the host can address.
fn bytes(count: u128) -> Result<u64, Fault> {
    u64::try_from(count).map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))
}
