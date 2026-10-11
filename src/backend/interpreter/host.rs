//! Host functions: what a call to a declared but undefined function does.
//!
//! The interpreter hands such a call to a [`Host`] by name. [`DefaultHost`]
//! implements the small part of the C library that test programs use to
//! report results and manage memory, and captures their output in an arena
//! buffer.

use std::fmt::{
    self,
    Write as _,
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
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The functions a program may call without defining them.
pub(crate) trait Host {
    /// Runs `call`, reading and writing the program's memory as it needs.
    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault>;
}

/// A call the host is asked to run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HostCall<'c> {
    /// The callee's name, without the `@`.
    pub(crate) name:   &'c str,
    /// The arguments, fixed and variadic, in order.
    pub(crate) args:   &'c [RuntimeValue],
    /// The callee's declared result type, if any.
    pub(crate) result: Option<Type>,
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
/// `free`, `memcpy`, `memset`, `strlen`, `exit` and `abort`, with output
/// captured in an arena buffer. Each follows its C library description;
/// misuse the standard leaves undefined, such as an overlapping `memcpy`,
/// traps.
///
/// C99: §7.19.7.9 and §7.19.7.10, p. 299; PDF p. 311.
/// C99: §7.21.2.1, p. 325; PDF p. 337.
/// C99: §7.21.6.1, p. 333; PDF p. 345.
/// C99: §7.21.6.3, p. 334; PDF p. 346.
pub(crate) struct DefaultHost<'a> {
    output:  ArenaVec<'a, u8>,
    scratch: ArenaVec<'a, u8>,
    text:    ArenaVec<'a, u8>,
}

impl Host for DefaultHost<'_> {
    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault> {
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
                self.printf(arg(0), args.get(1..).unwrap_or_default(), memory)?;
                RuntimeValue::Int((self.output.len() - before) as u128)
            },
            | "malloc" => allocate(memory, integer(arg(0))?, false)?,
            | "calloc" => {
                let size = integer(arg(0))?.checked_mul(integer(arg(1))?);
                match size {
                    | Some(size) => allocate(memory, size, true)?,
                    | None => RuntimeValue::NULL,
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
            | "memset" => {
                memory.fill(arg(0), arg(1), arg(2), Align::BYTE)?;
                arg(0)
            },
            | "strlen" => {
                self.scratch.clear();
                memory.read_c_string(arg(0), &mut self.scratch)?;
                RuntimeValue::Int(self.scratch.len() as u128)
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

impl DefaultHost<'_> {
    /// `printf` with the conversions `d i u x X c s p %`, the flags `-` and
    /// `0`, a field width, and the length modifiers `hh h l ll z j`.
    ///
    /// C99: §7.19.6.1 paragraphs 4-8, pp. 275-280; PDF pp. 287-292.
    fn printf(
        &mut self,
        format: RuntimeValue,
        args: &[RuntimeValue],
        memory: &Memory<'_>,
    ) -> Result<(), Fault> {
        self.scratch.clear();
        memory.read_c_string(format, &mut self.scratch)?;
        let mut args = args.iter().copied();
        let mut position = 0;
        while position < self.scratch.len() {
            let byte = self.scratch[position];
            position += 1;
            if byte != b'%' {
                self.output.push(byte);
                continue;
            }
            let mut spec = Spec::default();
            while let Some(&flag @ (b'-' | b'0')) = self.scratch.get(position) {
                if flag == b'-' {
                    spec.left = true;
                } else {
                    spec.zero = true;
                }
                position += 1;
            }
            while let Some(&digit @ b'0'..=b'9') = self.scratch.get(position) {
                spec.width = spec.width * 10 + usize::from(digit - b'0');
                position += 1;
            }
            let mut length = 32;
            loop {
                match self.scratch.get(position) {
                    | Some(b'h') => length /= 2,
                    | Some(b'l' | b'z' | b'j') => length = 64,
                    | _ => break,
                }
                position += 1;
            }
            let Some(&conversion) = self.scratch.get(position) else {
                return Err(Fault::Unsupported(Unsupported::FormatConversion));
            };
            position += 1;
            if conversion == b'%' {
                self.output.push(b'%');
                continue;
            }
            let arg = args.next().ok_or(UbKind::MissingFormatArgument)?;
            let mut digits = Digits::default();
            let body: &[u8] = match conversion {
                | b'd' | b'i' => {
                    let value = super::arithmetic::sext(width_type(length), integer(arg)?);
                    digits.format(format_args!("{value}"))
                },
                | b'u' => digits.format(format_args!(
                    "{}",
                    integer(arg)? & width_type(length).mask()
                )),
                | b'x' => digits.format(format_args!(
                    "{:x}",
                    integer(arg)? & width_type(length).mask()
                )),
                | b'X' => digits.format(format_args!(
                    "{:X}",
                    integer(arg)? & width_type(length).mask()
                )),
                | b'c' => {
                    spec.zero = false;
                    digits.bytes[0] = truncate_u8(integer(arg)?);
                    &digits.bytes[..1]
                },
                | b'p' => {
                    let RuntimeValue::Ptr(pointer) = arg else {
                        return Err(UbKind::PoisonInHostCall.into());
                    };
                    spec.zero = false;
                    digits.format(format_args!("0x{:x}", pointer.address))
                },
                | b's' => {
                    spec.zero = false;
                    self.text.clear();
                    memory.read_c_string(arg, &mut self.text)?;
                    &self.text
                },
                | _ => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
            };
            spec.pad(&mut self.output, body);
        }
        Ok(())
    }

    /// Everything the program has written.
    pub(crate) fn output(&self) -> &[u8] {
        &self.output
    }
}

/// A `printf` conversion's flags and field width.
#[derive(Clone, Copy, Default)]
struct Spec {
    left:  bool,
    zero:  bool,
    width: usize,
}

impl Spec {
    /// Appends `body` to `output`, padded to the field width: with spaces
    /// after it for `-`, with zeros after its sign for `0`, else with spaces
    /// before it.
    fn pad(self, output: &mut ArenaVec<'_, u8>, body: &[u8]) {
        let padding = self.width.saturating_sub(body.len());
        if self.left {
            output.extend_from_slice(body);
            output.extend(std::iter::repeat_n(b' ', padding));
        } else if self.zero {
            let sign = usize::from(body.first() == Some(&b'-'));
            output.extend_from_slice(&body[..sign]);
            output.extend(std::iter::repeat_n(b'0', padding));
            output.extend_from_slice(&body[sign..]);
        } else {
            output.extend(std::iter::repeat_n(b' ', padding));
            output.extend_from_slice(body);
        }
    }
}

/// The text of one numeric conversion, formatted without allocating.
struct Digits {
    bytes: [u8; 48],
    len:   usize,
}

impl Default for Digits {
    fn default() -> Self {
        Self {
            bytes: [0; 48],
            len:   0,
        }
    }
}

impl Digits {
    /// Formats `value` and returns its text.
    fn format(&mut self, value: fmt::Arguments<'_>) -> &[u8] {
        self.len = 0;
        self.write_fmt(value)
            .expect("a 128-bit number fits in the buffer");
        &self.bytes[..self.len]
    }
}

impl fmt::Write for Digits {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        self.bytes
            .get_mut(self.len..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// The integer type a length modifier names.
const fn width_type(length: u32) -> Type {
    match length {
        | 8 => Type::I8,
        | 16 => Type::I16,
        | 64 => Type::I64,
        | _ => Type::I32,
    }
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
            output:  ArenaVec::new_in(arena),
            scratch: ArenaVec::new_in(arena),
            text:    ArenaVec::new_in(arena),
        }
    }
}
