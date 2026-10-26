//! The `<string.h>`, `<wchar.h>`, `<ctype.h>` and `<stdlib.h>` conversion
//! functions the host adds to its byte primitives: concatenation and search,
//! the wide-string forms (`wchar_t` is 32 bits on GNU/Linux and 16 on
//! Windows), character classes in the "C" locale, including the tables
//! glibc's `<ctype.h>` macros read through `__ctype_b_loc`, and `strtol`
//! and its relatives.
//!
//! C99: §7.21.3-§7.21.6, pp. 326-333; PDF pp. 338-345 (strings); §7.24.4,
//! pp. 377-389; PDF pp. 389-401 (wide strings); §7.4, pp. 181-186; PDF
//! pp. 193-198 (character handling); §7.20.1, pp. 307-312; PDF pp. 319-324
//! (numeric conversion).

use super::{
    DefaultHost,
    HostCall,
    HostReturn,
    integer,
    string_length,
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
            Unsupported,
        },
        value::RuntimeValue,
    },
    ir::{
        Align,
        Type,
    },
    target::Target,
};

/// The functions this module runs.
pub(super) const FUNCTIONS: &[&str] = &[
    "strcat",
    "strncat",
    "strstr",
    "strnlen",
    "strspn",
    "strcspn",
    "strpbrk",
    "strdup",
    "strndup",
    "stpcpy",
    "strtok",
    "memrchr",
    "atoi",
    "atol",
    "atoll",
    "strtol",
    "strtoul",
    "strtoll",
    "strtoull",
    "isalnum",
    "isalpha",
    "isblank",
    "iscntrl",
    "isdigit",
    "isgraph",
    "islower",
    "ispunct",
    "isspace",
    "isupper",
    "isxdigit",
    "tolower",
    "toupper",
    "__ctype_b_loc",
    "__ctype_tolower_loc",
    "__ctype_toupper_loc",
    "wcslen",
    "wcscpy",
    "wcsncpy",
    "wcscat",
    "wcscmp",
    "wcsncmp",
    "wcschr",
    "wcsrchr",
    "wmemcpy",
    "wmemmove",
    "wmemset",
    "wmemcmp",
    "wmemchr",
];

/// glibc's character-class bits, as its little-endian `<ctype.h>` tests them.
const UPPER: u16 = 0x100;
const LOWER: u16 = 0x200;
const ALPHA: u16 = 0x400;
const DIGIT: u16 = 0x800;
const XDIGIT: u16 = 0x1000;
const SPACE: u16 = 0x2000;
const PRINT: u16 = 0x4000;
const GRAPH: u16 = 0x8000;
const BLANK: u16 = 0x1;
const CNTRL: u16 = 0x2;
const PUNCT: u16 = 0x4;
const ALNUM: u16 = 0x8;

/// The classes of `byte` in the "C" locale.
/// C99: §7.4.1, pp. 181-184; PDF pp. 193-196; §7.4 paragraph 1-2 (the "C"
/// locale's classes are those of the basic character set).
const fn classes(byte: u8) -> u16 {
    match byte {
        | b'A'..=b'Z' =>
            UPPER | ALPHA | ALNUM | PRINT | GRAPH | if byte <= b'F' { XDIGIT } else { 0 },
        | b'a'..=b'z' =>
            LOWER | ALPHA | ALNUM | PRINT | GRAPH | if byte <= b'f' { XDIGIT } else { 0 },
        | b'0'..=b'9' => DIGIT | XDIGIT | ALNUM | PRINT | GRAPH,
        | b' ' => SPACE | BLANK | PRINT,
        | b'\t' => SPACE | BLANK | CNTRL,
        | b'\n' | b'\x0B' | b'\x0C' | b'\r' => SPACE | CNTRL,
        | 0..=0x1F | 0x7F => CNTRL,
        | 0x21..=0x7E => PUNCT | PRINT | GRAPH,
        | _ => 0,
    }
}

impl DefaultHost<'_> {
    /// Runs a string, wide-string, character or conversion function, or
    /// `None` for another name.
    pub(super) fn strings(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        FUNCTIONS
            .contains(&call.name)
            .then(|| self.run_strings(call, memory))
    }

    fn run_strings(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let at = |memory: &Memory<'_>, pointer: RuntimeValue, offset: u64| {
            memory.ptr_add(pointer, RuntimeValue::Int(u128::from(offset)), true)
        };
        let byte = |memory: &Memory<'_>, pointer: RuntimeValue, offset: u64| -> Result<u8, Fault> {
            Ok(truncate_u8(integer(memory.load(
                Type::I8,
                at(memory, pointer, offset),
                Align::BYTE,
            )?)?))
        };
        let wide = self.wide_type();
        let value = match call.name {
            | "strcat" | "strncat" | "stpcpy" => {
                let (destination, source) = (arg(0), arg(1));
                let end = if call.name == "stpcpy" {
                    0
                } else {
                    string_length(memory, destination, u64::MAX)?
                };
                let limit = if call.name == "strncat" {
                    count(integer(arg(2))?)?
                } else {
                    u64::MAX
                };
                let length = string_length(memory, source, limit)?;
                let target = at(memory, destination, end);
                memory.copy(
                    target,
                    source,
                    RuntimeValue::Int(u128::from(length)),
                    Align::BYTE,
                    false,
                )?;
                let terminator = at(memory, target, length);
                memory.store(Type::I8, RuntimeValue::Int(0), terminator, Align::BYTE)?;
                if call.name == "stpcpy" {
                    terminator
                } else {
                    destination
                }
            },
            | "strstr" => {
                let (haystack, needle) = (arg(0), arg(1));
                let needle_length = string_length(memory, needle, u64::MAX)?;
                let haystack_length = string_length(memory, haystack, u64::MAX)?;
                let mut found = RuntimeValue::NULL;
                'search: for start in 0..=haystack_length.saturating_sub(needle_length) {
                    if needle_length > haystack_length {
                        break;
                    }
                    for offset in 0..needle_length {
                        if byte(memory, haystack, start + offset)? != byte(memory, needle, offset)?
                        {
                            continue 'search;
                        }
                    }
                    found = at(memory, haystack, start);
                    break;
                }
                found
            },
            | "strnlen" => RuntimeValue::Int(u128::from(string_length(
                memory,
                arg(0),
                count(integer(arg(1))?)?,
            )?)),
            | "strspn" | "strcspn" | "strpbrk" => {
                self.text.clear();
                memory.read_c_string(arg(1), &mut self.text)?;
                let accept = call.name == "strspn";
                let mut offset = 0;
                loop {
                    let current = byte(memory, arg(0), offset)?;
                    let member = self.text.contains(&current);
                    if current == 0 || member != accept {
                        break;
                    }
                    offset += 1;
                }
                if call.name == "strpbrk" {
                    if byte(memory, arg(0), offset)? == 0 {
                        RuntimeValue::NULL
                    } else {
                        at(memory, arg(0), offset)
                    }
                } else {
                    RuntimeValue::Int(u128::from(offset))
                }
            },
            | "strdup" | "strndup" => {
                let limit = if call.name == "strndup" {
                    count(integer(arg(1))?)?
                } else {
                    u64::MAX
                };
                let length = string_length(memory, arg(0), limit)?;
                let copy = super::allocate(memory, u128::from(length) + 1, false)?;
                if copy != RuntimeValue::NULL {
                    memory.copy(
                        copy,
                        arg(0),
                        RuntimeValue::Int(u128::from(length)),
                        Align::BYTE,
                        false,
                    )?;
                    memory.store(
                        Type::I8,
                        RuntimeValue::Int(0),
                        at(memory, copy, length),
                        Align::BYTE,
                    )?;
                }
                copy
            },
            | "strtok" => {
                // C99 §7.21.5.8: a null first argument continues after the
                // previous token.
                let start = if arg(0) == RuntimeValue::NULL {
                    self.strtok
                } else {
                    arg(0)
                };
                if start == RuntimeValue::NULL {
                    RuntimeValue::NULL
                } else {
                    self.text.clear();
                    memory.read_c_string(arg(1), &mut self.text)?;
                    let mut offset = 0;
                    while byte(memory, start, offset)? != 0
                        && self.text.contains(&byte(memory, start, offset)?)
                    {
                        offset += 1;
                    }
                    if byte(memory, start, offset)? == 0 {
                        self.strtok = RuntimeValue::NULL;
                        RuntimeValue::NULL
                    } else {
                        let token = at(memory, start, offset);
                        let mut end = offset;
                        while byte(memory, start, end)? != 0
                            && !self.text.contains(&byte(memory, start, end)?)
                        {
                            end += 1;
                        }
                        if byte(memory, start, end)? == 0 {
                            self.strtok = RuntimeValue::NULL;
                        } else {
                            let terminator = at(memory, start, end);
                            memory.store(
                                Type::I8,
                                RuntimeValue::Int(0),
                                terminator,
                                Align::BYTE,
                            )?;
                            self.strtok = at(memory, start, end + 1);
                        }
                        token
                    }
                }
            },
            | "memrchr" => {
                let character = truncate_u8(integer(arg(1))?);
                let mut found = RuntimeValue::NULL;
                for offset in (0..count(integer(arg(2))?)?).rev() {
                    if byte(memory, arg(0), offset)? == character {
                        found = at(memory, arg(0), offset);
                        break;
                    }
                }
                found
            },
            | "atoi" | "atol" | "atoll" => {
                let parsed = parse_integer(memory, arg(0), 10)?;
                RuntimeValue::Int(parsed.signed(true, 64).0)
            },
            | "strtol" | "strtoul" | "strtoll" | "strtoull" => {
                let base = sext(Type::I32, integer(arg(2))?);
                if base != 0 && !(2..=36).contains(&base) {
                    return Err(Fault::Unsupported(Unsupported::FormatConversion));
                }
                let parsed = parse_integer(memory, arg(0), u32::try_from(base).unwrap_or(10))?;
                if arg(1) != RuntimeValue::NULL {
                    let end = at(memory, arg(0), parsed.consumed);
                    memory.store(Type::Ptr, end, arg(1), Align::BYTE)?;
                }
                let bits = call.result.map_or(64, Type::bits);
                let signed = matches!(call.name, "strtol" | "strtoll");
                RuntimeValue::Int(parsed.signed(signed, bits).0)
            },
            | "tolower" | "toupper" => {
                let value = sext(Type::I32, integer(arg(0))?);
                let mapped = match u8::try_from(value) {
                    | Ok(byte) if call.name == "tolower" => i128::from(byte.to_ascii_lowercase()),
                    | Ok(byte) => i128::from(byte.to_ascii_uppercase()),
                    | Err(_) => value,
                };
                RuntimeValue::Int(mapped.cast_unsigned())
            },
            | "__ctype_b_loc" | "__ctype_tolower_loc" | "__ctype_toupper_loc" =>
                self.ctype_table(call.name, memory)?,
            | name if name.starts_with("is") => {
                let value = sext(Type::I32, integer(arg(0))?);
                let bit = match name {
                    | "isalnum" => ALNUM,
                    | "isalpha" => ALPHA,
                    | "isblank" => BLANK,
                    | "iscntrl" => CNTRL,
                    | "isdigit" => DIGIT,
                    | "isgraph" => GRAPH,
                    | "islower" => LOWER,
                    | "ispunct" => PUNCT,
                    | "isspace" => SPACE,
                    | "isupper" => UPPER,
                    | _ => XDIGIT,
                };
                let member = u8::try_from(value).is_ok_and(|byte| classes(byte) & bit != 0);
                RuntimeValue::Int(u128::from(member))
            },
            | _ => Self::wide(call, memory, wide)?,
        };
        Ok(HostReturn::Value(call.result.map(|ty| match value {
            | RuntimeValue::Int(bits) if ty.is_int() => RuntimeValue::int(ty, bits),
            | value => value,
        })))
    }

    /// The target's `wchar_t`.
    fn wide_type(&self) -> Type {
        if matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc) {
            Type::I16
        } else {
            Type::I32
        }
    }

    /// The `wcs` and `wmem` functions, on elements of type `wide`.
    /// C99: §7.24.4.2-§7.24.4.6, pp. 380-388; PDF pp. 392-400.
    fn wide(
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
        wide: Type,
    ) -> Result<RuntimeValue, Fault> {
        let args = call.args;
        let arg = |index: usize| args.get(index).copied().unwrap_or(RuntimeValue::Poison);
        let size = u64::from(wide.bytes());
        let align = Align::from_bytes(size).expect("a power of two");
        let at = |memory: &Memory<'_>, pointer: RuntimeValue, index: u64| {
            memory.ptr_add(pointer, RuntimeValue::Int(u128::from(index * size)), true)
        };
        let element =
            |memory: &Memory<'_>, pointer: RuntimeValue, index: u64| -> Result<u128, Fault> {
                integer(memory.load(wide, at(memory, pointer, index), align)?)
            };
        let length =
            |memory: &Memory<'_>, pointer: RuntimeValue, limit: u64| -> Result<u64, Fault> {
                for index in 0..limit {
                    if element(memory, pointer, index)? == 0 {
                        return Ok(index);
                    }
                }
                Ok(limit)
            };
        // glibc compares `wchar_t` values, which are signed on GNU/Linux,
        // and returns -1 or 1.
        let order = |a: u128, b: u128| -> RuntimeValue {
            let (a, b) = if wide == Type::I32 {
                (sext(wide, a), sext(wide, b))
            } else {
                (a.cast_signed(), b.cast_signed())
            };
            RuntimeValue::Int(match a.cmp(&b) {
                | std::cmp::Ordering::Less => u128::MAX,
                | std::cmp::Ordering::Equal => 0,
                | std::cmp::Ordering::Greater => 1,
            })
        };
        let bytes = |count: u64| RuntimeValue::Int(u128::from(count * size));
        Ok(match call.name {
            | "wcslen" => RuntimeValue::Int(u128::from(length(memory, arg(0), u64::MAX)?)),
            | "wcscpy" | "wcscat" => {
                let end = if call.name == "wcscat" {
                    length(memory, arg(0), u64::MAX)?
                } else {
                    0
                };
                let source = length(memory, arg(1), u64::MAX)? + 1;
                memory.copy(
                    at(memory, arg(0), end),
                    arg(1),
                    bytes(source),
                    Align::BYTE,
                    false,
                )?;
                arg(0)
            },
            | "wcsncpy" => {
                let limit = count(integer(arg(2))?)?;
                let copied = length(memory, arg(1), limit)?;
                memory.copy(arg(0), arg(1), bytes(copied), Align::BYTE, false)?;
                for index in copied..limit {
                    memory.store(wide, RuntimeValue::Int(0), at(memory, arg(0), index), align)?;
                }
                arg(0)
            },
            | "wcscmp" | "wcsncmp" | "wmemcmp" => {
                let limit = if call.name == "wcscmp" {
                    u64::MAX
                } else {
                    count(integer(arg(2))?)?
                };
                let mut result = RuntimeValue::Int(0);
                for index in 0..limit {
                    let (a, b) = (
                        element(memory, arg(0), index)?,
                        element(memory, arg(1), index)?,
                    );
                    if a != b {
                        result = order(a, b);
                        break;
                    }
                    if a == 0 && call.name != "wmemcmp" {
                        break;
                    }
                }
                result
            },
            | "wcschr" | "wcsrchr" | "wmemchr" => {
                let wanted = integer(arg(1))? & wide.mask();
                let limit = if call.name == "wmemchr" {
                    count(integer(arg(2))?)?
                } else {
                    u64::MAX
                };
                let mut found = RuntimeValue::NULL;
                for index in 0..limit {
                    let current = element(memory, arg(0), index)?;
                    if current == wanted {
                        found = at(memory, arg(0), index);
                        if call.name != "wcsrchr" {
                            break;
                        }
                    }
                    if current == 0 && call.name != "wmemchr" {
                        break;
                    }
                }
                found
            },
            | "wmemcpy" | "wmemmove" => {
                let elements = count(integer(arg(2))?)?;
                memory.copy(
                    arg(0),
                    arg(1),
                    bytes(elements),
                    Align::BYTE,
                    call.name == "wmemmove",
                )?;
                arg(0)
            },
            | "wmemset" => {
                let value = RuntimeValue::int(wide, integer(arg(1))?);
                for index in 0..count(integer(arg(2))?)? {
                    memory.store(wide, value, at(memory, arg(0), index), align)?;
                }
                arg(0)
            },
            | _ => unreachable!("`FUNCTIONS` lists every string function"),
        })
    }

    /// glibc's `__ctype_b_loc`, `__ctype_tolower_loc` and
    /// `__ctype_toupper_loc`: the address of a pointer to entry 0 of a
    /// table indexed from -128 to 255, made on first use. The class table
    /// has `unsigned short` entries, the case tables `int` entries; negative
    /// indexes other than `EOF` map like their `unsigned char` values.
    fn ctype_table(&mut self, name: &str, memory: &mut Memory<'_>) -> Result<RuntimeValue, Fault> {
        let slot = match name {
            | "__ctype_b_loc" => 0,
            | "__ctype_tolower_loc" => 1,
            | _ => 2,
        };
        if let Some(pointer) = self.ctype[slot] {
            return Ok(RuntimeValue::Ptr(pointer));
        }
        let width: u64 = if slot == 0 { 2 } else { 4 };
        let table = memory.allocate(
            ObjectKind::ConstantGlobal,
            384 * width,
            Align::from_bytes(width).expect("a power of two"),
        )?;
        self.text.clear();
        for index in -128_i32..256 {
            let byte = (index & 0xFF).cast_unsigned();
            let byte = u8::try_from(byte).expect("masked");
            let entry: u32 = match slot {
                | 0 if index < 0 => 0,
                | 0 => u32::from(classes(byte)),
                | _ if index == -1 => u32::MAX,
                | 1 => u32::from(byte.to_ascii_lowercase()),
                | _ => u32::from(byte.to_ascii_uppercase()),
            };
            let entry = if slot != 0 && index < -1 {
                u32::from(byte)
            } else {
                entry
            };
            self.text
                .extend_from_slice(&entry.to_le_bytes()[..usize::try_from(width).expect("small")]);
        }
        memory.initialize(table, &self.text);
        let location = memory.allocate(
            ObjectKind::Global,
            8,
            Align::from_bytes(8).expect("a power of two"),
        )?;
        memory.initialize(location, &[]);
        let entry_zero = memory.ptr_add(
            RuntimeValue::Ptr(table),
            RuntimeValue::Int(u128::from(128 * width)),
            true,
        );
        memory.store(
            Type::Ptr,
            entry_zero,
            RuntimeValue::Ptr(location),
            Align::BYTE,
        )?;
        self.ctype[slot] = Some(location);
        Ok(RuntimeValue::Ptr(location))
    }
}

/// An integer `strtol` read: its sign, its magnitude (saturated past
/// `u64`), whether it overflowed `u64`, and how many bytes it consumed
/// (zero when no digits were found).
struct ParsedInteger {
    negative:  bool,
    magnitude: u128,
    overflow:  bool,
    consumed:  u64,
}

impl ParsedInteger {
    /// The value as `bits`-bit two's complement, clamped as `strtol` (signed)
    /// or `strtoul` (unsigned, a negative value negated) clamps it, and
    /// whether it was clamped.
    fn signed(&self, signed: bool, bits: u32) -> (u128, bool) {
        let mask = u128::MAX >> (128 - bits);
        if signed {
            let maximum = mask >> 1;
            if self.negative {
                if self.overflow || self.magnitude > maximum + 1 {
                    ((maximum + 1) & mask, true)
                } else {
                    (self.magnitude.wrapping_neg() & mask, false)
                }
            } else if self.overflow || self.magnitude > maximum {
                (maximum, true)
            } else {
                (self.magnitude, false)
            }
        } else if self.overflow || self.magnitude > mask {
            (mask, true)
        } else if self.negative {
            (self.magnitude.wrapping_neg() & mask, false)
        } else {
            (self.magnitude, false)
        }
    }
}

/// Reads an integer as `strtol` does in the "C" locale: white space, a
/// sign, a base prefix (`0x` for base 16 or 0, `0` for octal in base 0) and
/// digits.
/// C99: §7.20.1.4 paragraphs 2-5, pp. 310-311; PDF pp. 322-323.
fn parse_integer(
    memory: &Memory<'_>,
    string: RuntimeValue,
    base: u32,
) -> Result<ParsedInteger, Fault> {
    let byte = |offset: u64| -> Result<u8, Fault> {
        let at = memory.ptr_add(string, RuntimeValue::Int(u128::from(offset)), true);
        Ok(truncate_u8(integer(memory.load(
            Type::I8,
            at,
            Align::BYTE,
        )?)?))
    };
    let mut position = 0;
    while classes(byte(position)?) & SPACE != 0 {
        position += 1;
    }
    let negative = byte(position)? == b'-';
    if matches!(byte(position)?, b'+' | b'-') {
        position += 1;
    }
    let hex_prefix = |position: u64| -> Result<bool, Fault> {
        Ok(byte(position)? == b'0'
            && matches!(byte(position + 1)?, b'x' | b'X')
            && byte(position + 2)?.is_ascii_hexdigit())
    };
    let base = match base {
        | 0 | 16 if hex_prefix(position)? => {
            position += 2;
            16
        },
        | 0 if byte(position)? == b'0' => 8,
        | 0 => 10,
        | base => base,
    };
    let mut parsed = ParsedInteger {
        negative,
        magnitude: 0,
        overflow: false,
        consumed: 0,
    };
    let start = position;
    loop {
        let digit = match byte(position)? {
            | digit @ b'0'..=b'9' => u32::from(digit - b'0'),
            | letter @ b'a'..=b'z' => u32::from(letter - b'a') + 10,
            | letter @ b'A'..=b'Z' => u32::from(letter - b'A') + 10,
            | _ => break,
        };
        if digit >= base {
            break;
        }
        parsed.magnitude = parsed.magnitude * u128::from(base) + u128::from(digit);
        if parsed.magnitude > u128::from(u64::MAX) {
            parsed.overflow = true;
            parsed.magnitude = u128::from(u64::MAX);
        }
        position += 1;
    }
    parsed.consumed = if position == start { 0 } else { position };
    Ok(parsed)
}

/// An element count the host can address.
fn count(value: u128) -> Result<u64, Fault> {
    u64::try_from(value).map_err(|_| UbKind::OutOfBounds.into())
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}
