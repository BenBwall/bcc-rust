//! Host functions: what a call to a declared but undefined function does.
//!
//! The interpreter hands such a call to a [`Host`] by name. [`DefaultHost`]
//! implements the small part of the C library that test programs use to
//! report results and manage memory, and captures their output in an arena
//! buffer.

mod math;

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
    output:      ArenaVec<'a, u8>,
    scratch:     ArenaVec<'a, u8>,
    text:        ArenaVec<'a, u8>,
    /// The width of `long` in bits, which `printf`'s `l` modifier reads.
    long_width:  u32,
    /// The target's existing long double representation (f64 or unsupported
    /// f80).
    long_double: Type,
}

impl Host for DefaultHost<'_> {
    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault> {
        check_signature(call)?;
        if let Some(value) = math::call(
            call,
            memory,
            self.long_double,
            self.long_width,
            &mut self.text,
        ) {
            return value.map(|value| HostReturn::Value(Some(value)));
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
                self.printf(arg(0), args.get(1..).unwrap_or_default(), memory)?;
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
                let result = self.printf(
                    arg(format),
                    args.get(format + 1..).unwrap_or_default(),
                    memory,
                );
                let count = self.output.len() - before;
                let result = result.and_then(|()| {
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
                        let address =
                            memory.ptr_add(arg(0), RuntimeValue::Int(offset as u128), true);
                        memory.store(
                            Type::I8,
                            RuntimeValue::Int(u128::from(byte)),
                            address,
                            Align::BYTE,
                        )?;
                    }
                    Ok(())
                });
                self.output.truncate(before);
                result?;
                RuntimeValue::Int(count as u128)
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
                // C99: §7.21.2.4 paragraphs 2-3, p. 326; PDF p. 338.
                // Copy at most n source bytes, then pad the destination with
                // zeros. A full-length copy need not have a terminating null.
                let count = integer(arg(2))?;
                let count = u64::try_from(count)
                    .map_err(|_| Fault::Unsupported(Unsupported::ObjectSize))?;
                let mut copied = 0;
                for offset in 0..count {
                    let address =
                        memory.ptr_add(arg(1), RuntimeValue::Int(u128::from(offset)), true);
                    let byte = integer(memory.load(Type::I8, address, Align::BYTE)?)?;
                    if byte == 0 {
                        break;
                    }
                    copied += 1;
                }
                // Use the memory copy's overlap check for the accessed prefix.
                let prefix = RuntimeValue::Int(u128::from(copied));
                memory.copy(arg(0), arg(1), prefix, Align::BYTE, false)?;
                let tail = memory.ptr_add(arg(0), prefix, true);
                memory.fill(
                    tail,
                    RuntimeValue::Int(0),
                    RuntimeValue::Int(u128::from(count - copied)),
                    Align::BYTE,
                )?;
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

/// Rejects incompatible calls before a library function performs any effects.
/// All supported targets use i32 int and i64 `size_t`. Open C declarations are
/// checked using their call operands, after default argument promotions.
/// C99: §6.5.2.2 paragraphs 6 and 9, pp. 71-72; PDF pp. 83-84.
fn check_signature(call: HostCall<'_>) -> Result<(), Fault> {
    use Type::{
        I32,
        I64,
        Ptr,
    };
    let (params, result, variadic): (&[Type], Option<Type>, bool) = match call.name {
        | "putchar" | "abs" | "isprint" => (&[I32], Some(I32), false),
        | "puts" => (&[Ptr], Some(I32), false),
        | "printf" => (&[Ptr], Some(I32), true),
        | "sprintf" => (&[Ptr, Ptr], Some(I32), true),
        | "snprintf" => (&[Ptr, I64, Ptr], Some(I32), true),
        | "malloc" => (&[I64], Some(Ptr), false),
        | "calloc" => (&[I64, I64], Some(Ptr), false),
        | "free" => (&[Ptr], None, false),
        | "memcpy" | "memmove" | "strncpy" => (&[Ptr, Ptr, I64], Some(Ptr), false),
        | "memcmp" | "strncmp" => (&[Ptr, Ptr, I64], Some(I32), false),
        | "strcmp" => (&[Ptr, Ptr], Some(I32), false),
        | "strcpy" => (&[Ptr, Ptr], Some(Ptr), false),
        | "memchr" | "memset" => (&[Ptr, I32, I64], Some(Ptr), false),
        | "strchr" | "strrchr" => (&[Ptr, I32], Some(Ptr), false),
        | "strlen" => (&[Ptr], Some(I64), false),
        | "exit" => (&[I32], None, false),
        | "abort" => (&[], None, false),
        | _ => return Ok(()),
    };
    if call.result != result
        || (!variadic && call.arg_types.len() != params.len())
        || !call.arg_types.starts_with(params)
    {
        return Err(UbKind::SignatureMismatch.into());
    }
    Ok(())
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
    /// `printf` with the conversions `d i u o x X c s p %`, the flags `-`,
    /// `0` and `#`, a field width, and the length modifiers `hh h l ll z j`.
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
            while let Some(&flag @ (b'-' | b'0' | b'#')) = self.scratch.get(position) {
                if flag == b'-' {
                    spec.left = true;
                } else if flag == b'#' {
                    spec.alternate = true;
                } else {
                    spec.zero = true;
                }
                position += 1;
            }
            while let Some(&digit @ b'0'..=b'9') = self.scratch.get(position) {
                spec.width = spec
                    .width
                    .checked_mul(10)
                    .and_then(|width| width.checked_add(usize::from(digit - b'0')))
                    .filter(|&width| width <= MAX_FORMAT_WIDTH)
                    .ok_or(Fault::Unsupported(Unsupported::FormatWidth))?;
                position += 1;
            }
            // C99: §7.19.6.1 paragraph 7, pp. 276-277; PDF pp. 288-289:
            // `l` names `long`, whose width is the target's, and `ll`
            // names `long long`.
            let mut length = 32;
            let mut ells = 0;
            loop {
                match self.scratch.get(position) {
                    | Some(b'h') => length /= 2,
                    | Some(b'l') => {
                        ells += 1;
                        length = if ells == 1 { self.long_width } else { 64 };
                    },
                    | Some(b'z' | b'j') => length = 64,
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
                | b'o' | b'x' | b'X' => {
                    let value = integer(arg)? & width_type(length).mask();
                    // C99 §7.19.6.1p6: octal starts with zero;
                    // hexadecimal has a base prefix only for nonzero values.
                    match (conversion, spec.alternate && value != 0) {
                        | (b'o', true) => digits.format(format_args!("0{value:o}")),
                        | (b'o', false) => digits.format(format_args!("{value:o}")),
                        | (b'x', true) => digits.format(format_args!("0x{value:x}")),
                        | (b'x', false) => digits.format(format_args!("{value:x}")),
                        | (_, true) => digits.format(format_args!("0X{value:X}")),
                        | (_, false) => digits.format(format_args!("{value:X}")),
                    }
                },
                | b'c' => {
                    spec.zero = false;
                    digits.bytes[0] = truncate_u8(integer(arg)?);
                    &digits.bytes[..1]
                },
                | b'f' | b'F' => {
                    let RuntimeValue::F64(value) = arg else {
                        return Err(UbKind::PoisonInHostCall.into());
                    };
                    // C99 §7.19.6.1p8: variadic float is promoted to double;
                    // the default f conversion has six fractional digits.
                    if value.is_nan() || value.is_infinite() {
                        spec.zero = false;
                        let name = match (value.is_nan(), conversion) {
                            | (true, b'F') => "NAN",
                            | (true, _) => "nan",
                            | (false, b'F') => "INF",
                            | (false, _) => "inf",
                        };
                        digits.format(format_args!(
                            "{}{name}",
                            if value.is_sign_negative() { "-" } else { "" }
                        ))
                    } else {
                        digits.format(format_args!("{value:.6}"))
                    }
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
    left:      bool,
    zero:      bool,
    alternate: bool,
    width:     usize,
}

/// Interpreter limit on padding for one conversion (one MiB). Precision
/// conversions are unsupported and trap before any precision is accumulated.
/// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287 (decimal field width).
const MAX_FORMAT_WIDTH: usize = 1024 * 1024;

impl Spec {
    /// Appends `body` to `output`, padded to the field width: with spaces
    /// after it for `-`, with zeros after its sign or base prefix for `0`, else
    /// with spaces before it.
    /// C99: §7.19.6.1 paragraph 6, pp. 275-276; PDF pp. 287-288.
    fn pad(self, output: &mut ArenaVec<'_, u8>, body: &[u8]) {
        let padding = self.width.saturating_sub(body.len());
        if self.left {
            output.extend_from_slice(body);
            output.extend(std::iter::repeat_n(b' ', padding));
        } else if self.zero {
            let prefix = if body.starts_with(b"0x") || body.starts_with(b"0X") {
                2
            } else {
                usize::from(body.first() == Some(&b'-'))
            };
            output.extend_from_slice(&body[..prefix]);
            output.extend(std::iter::repeat_n(b'0', padding));
            output.extend_from_slice(&body[prefix..]);
        } else {
            output.extend(std::iter::repeat_n(b' ', padding));
            output.extend_from_slice(body);
        }
    }
}

/// The text of one numeric conversion, formatted without allocating.
struct Digits {
    // A maximum finite binary64 f conversion needs 309 integer digits,
    // a sign, a decimal point and six fractional digits.
    bytes: [u8; 320],
    len:   usize,
}

impl Default for Digits {
    fn default() -> Self {
        Self {
            bytes: [0; 320],
            len:   0,
        }
    }
}

impl Digits {
    /// Formats `value` and returns its text.
    fn format(&mut self, value: fmt::Arguments<'_>) -> &[u8] {
        self.len = 0;
        self.write_fmt(value)
            .expect("integer and default binary64 f conversions fit in the buffer");
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
            output:      ArenaVec::new_in(arena),
            scratch:     ArenaVec::new_in(arena),
            text:        ArenaVec::new_in(arena),
            long_width:  64,
            long_double: Type::F80,
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
