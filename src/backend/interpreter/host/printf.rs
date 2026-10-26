//! `printf`'s format interpretation, shared by every formatted-output host
//! function: flags, field width and precision (from digits or `*`), length
//! modifiers, and the conversions `d i u o x X c s p e E f F g G a A %`.
//! Integers and strings are formatted here; floating conversions in
//! `floating.rs`. Output follows glibc where C leaves a choice, such as
//! `(nil)` for a null `%p` on GNU/Linux.
//!
//! C99: §7.19.6.1 paragraphs 3-9, pp. 274-280; PDF pp. 286-292.

use std::fmt::{
    self,
    Write as _,
};

use super::{
    ArgumentClass,
    DefaultHost,
    FormatArguments,
    floating::{
        self,
        Floating,
        PositiveSign,
    },
    integer,
    string_length,
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
    target::Target,
};

/// Interpreter limit on padding and precision for one conversion (one MiB).
/// C99: §7.19.6.1 paragraph 4, p. 275; PDF p. 287 (decimal field width).
const MAX_FORMAT_WIDTH: usize = 1024 * 1024;

impl DefaultHost<'_> {
    /// Appends the output of `format` with the arguments `args` to the
    /// host's output buffer.
    ///
    /// C99: §7.19.6.1 paragraphs 4-8, pp. 275-280; PDF pp. 287-292.
    pub(super) fn printf(
        &mut self,
        format: RuntimeValue,
        args: &mut FormatArguments<'_>,
        memory: &Memory<'_>,
    ) -> Result<(), Fault> {
        self.scratch.clear();
        memory.read_c_string(format, &mut self.scratch)?;
        let mut position = 0;
        while position < self.scratch.len() {
            let byte = self.scratch[position];
            position += 1;
            if byte != b'%' {
                self.output.push(byte);
                continue;
            }
            let mut spec = Spec::default();
            while let Some(&flag @ (b'-' | b'0' | b'#' | b'+' | b' ')) = self.scratch.get(position)
            {
                match flag {
                    | b'-' => spec.left = true,
                    | b'#' => spec.alternate = true,
                    | b'0' => spec.zero = true,
                    | b'+' => spec.sign = PositiveSign::Plus,
                    | _ if spec.sign == PositiveSign::None => spec.sign = PositiveSign::Space,
                    | _ => {},
                }
                position += 1;
            }
            // C99 §7.19.6.1p5: a negative `*` width is a `-` flag and a
            // positive width.
            if self.scratch.get(position) == Some(&b'*') {
                position += 1;
                let width = sext(
                    Type::I32,
                    integer(args.next(ArgumentClass::Integer(32), memory)?)?,
                );
                spec.left |= width < 0;
                spec.width = bounded(width.unsigned_abs())?;
            } else {
                spec.width = self.decimal(&mut position)?;
            }
            // A negative `*` precision is taken as if it were omitted.
            if self.scratch.get(position) == Some(&b'.') {
                position += 1;
                if self.scratch.get(position) == Some(&b'*') {
                    position += 1;
                    let precision = sext(
                        Type::I32,
                        integer(args.next(ArgumentClass::Integer(32), memory)?)?,
                    );
                    spec.precision = u128::try_from(precision).ok().map(bounded).transpose()?;
                } else {
                    spec.precision = Some(self.decimal(&mut position)?);
                }
            }
            // C99: §7.19.6.1 paragraph 7, pp. 276-277; PDF pp. 288-289:
            // `l` names `long`, whose width is the target's, and `ll`
            // names `long long`; `L` names `long double`.
            let mut length = 32;
            let mut ells = 0;
            let mut long_double = false;
            loop {
                match self.scratch.get(position) {
                    | Some(b'h') => length /= 2,
                    | Some(b'l') => {
                        ells += 1;
                        length = if ells == 1 { self.long_width } else { 64 };
                    },
                    | Some(b'z' | b'j' | b't' | b'q') => length = 64,
                    | Some(b'L') => long_double = true,
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
            self.conversion(conversion, length, long_double, spec, args, memory)?;
        }
        Ok(())
    }

    /// Formats one conversion and pads it into the output.
    fn conversion(
        &mut self,
        conversion: u8,
        length: u32,
        long_double: bool,
        mut spec: Spec,
        args: &mut FormatArguments<'_>,
        memory: &Memory<'_>,
    ) -> Result<(), Fault> {
        let class = match conversion {
            | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' if long_double =>
                ArgumentClass::LongDouble,
            | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => ArgumentClass::Double,
            | b'p' | b's' => ArgumentClass::Pointer,
            | b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'c' => ArgumentClass::Integer(length),
            | _ => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
        };
        let arg = args.next(class, memory)?;
        let mut digits = Digits::default();
        self.text.clear();
        // The length of the sign or base prefix that zero padding follows.
        let mut prefix = 0;
        match conversion {
            | b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                let ty = width_type(length);
                let bits = integer(arg)?;
                let (negative, magnitude) = if matches!(conversion, b'd' | b'i') {
                    let value = sext(ty, bits);
                    (value < 0, value.unsigned_abs())
                } else {
                    (false, bits & ty.mask())
                };
                if negative {
                    self.text.push(b'-');
                } else if matches!(conversion, b'd' | b'i') {
                    match spec.sign {
                        | PositiveSign::Plus => self.text.push(b'+'),
                        | PositiveSign::Space => self.text.push(b' '),
                        | PositiveSign::None => {},
                    }
                }
                // C99 §7.19.6.1p6: hexadecimal has a base prefix only for
                // nonzero values.
                if spec.alternate && magnitude != 0 && matches!(conversion, b'x' | b'X') {
                    self.text
                        .extend_from_slice(if conversion == b'x' { b"0x" } else { b"0X" });
                }
                prefix = self.text.len();
                let body = match conversion {
                    | b'o' => digits.format(format_args!("{magnitude:o}")),
                    | b'x' => digits.format(format_args!("{magnitude:x}")),
                    | b'X' => digits.format(format_args!("{magnitude:X}")),
                    | _ => digits.format(format_args!("{magnitude}")),
                };
                // C99 §7.19.6.1p8: the precision is the minimum number of
                // digits, and zero with precision zero has none; the `0`
                // flag is ignored when a precision is given.
                let body: &[u8] = if magnitude == 0 && spec.precision == Some(0) {
                    &[]
                } else {
                    body
                };
                let minimum = spec.precision.unwrap_or(1);
                let mut zeros = minimum.saturating_sub(body.len());
                // C99 §7.19.6.1p6: `#o` increases the precision so that the
                // first digit is zero.
                if conversion == b'o' && spec.alternate && zeros == 0 && body.first() != Some(&b'0')
                {
                    zeros = 1;
                }
                self.text.extend(std::iter::repeat_n(b'0', zeros));
                self.text.extend_from_slice(body);
                if spec.precision.is_some() {
                    spec.zero = false;
                }
            },
            | b'c' => {
                spec.zero = false;
                self.text.push(truncate_u8(integer(arg)?));
            },
            | b's' => {
                spec.zero = false;
                // C99 §7.19.6.1p8: with a precision, at most that many
                // bytes are read, and the array need not hold a null.
                let limit = spec
                    .precision
                    .map_or(u64::MAX, |precision| precision as u64);
                let count = string_length(memory, arg, limit)?;
                for offset in 0..count {
                    let at = memory.ptr_add(arg, RuntimeValue::Int(u128::from(offset)), true);
                    self.text.push(truncate_u8(integer(memory.load(
                        Type::I8,
                        at,
                        Align::BYTE,
                    )?)?));
                }
            },
            | b'p' => {
                let RuntimeValue::Ptr(pointer) = arg else {
                    return Err(UbKind::PoisonInHostCall.into());
                };
                spec.zero = false;
                if pointer.address == 0 && self.target == Target::LinuxGnu {
                    self.text.extend_from_slice(b"(nil)");
                } else {
                    let body = digits.format(format_args!("0x{:x}", pointer.address));
                    self.text.extend_from_slice(body);
                }
            },
            | _ => {
                let value = match arg {
                    | RuntimeValue::F64(value) => Floating::Double(value),
                    | RuntimeValue::F80(value) => Floating::Extended(value),
                    | RuntimeValue::Poison => return Err(UbKind::PoisonInHostCall.into()),
                    | _ => return Err(UbKind::VaArgTypeMismatch.into()),
                };
                let finite = floating::format(
                    value,
                    conversion,
                    spec.precision,
                    spec.alternate,
                    spec.sign,
                    &mut self.text,
                    &mut self.floating,
                );
                // Infinities and NaNs ignore the `0` flag.
                spec.zero &= finite;
                prefix = usize::from(matches!(self.text.first(), Some(b'-' | b'+' | b' ')));
                if matches!(self.text.get(prefix..prefix + 2), Some(b"0x" | b"0X")) {
                    prefix += 2;
                }
            },
        }
        spec.pad(&mut self.output, &self.text, prefix);
        Ok(())
    }

    /// The decimal digits at `position` in the format, advancing past them.
    fn decimal(&self, position: &mut usize) -> Result<usize, Fault> {
        let mut value: usize = 0;
        while let Some(&digit @ b'0'..=b'9') = self.scratch.get(*position) {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::from(digit - b'0')))
                .filter(|&value| value <= MAX_FORMAT_WIDTH)
                .ok_or(Fault::Unsupported(Unsupported::FormatWidth))?;
            *position += 1;
        }
        Ok(value)
    }
}

/// A width or precision from an argument, within the interpreter's limit.
fn bounded(value: u128) -> Result<usize, Fault> {
    usize::try_from(value)
        .ok()
        .filter(|&value| value <= MAX_FORMAT_WIDTH)
        .ok_or(Fault::Unsupported(Unsupported::FormatWidth))
}

/// A `printf` conversion's flags, field width and precision.
#[derive(Clone, Copy, Default)]
struct Spec {
    left:      bool,
    zero:      bool,
    alternate: bool,
    sign:      PositiveSign,
    width:     usize,
    precision: Option<usize>,
}

impl Spec {
    /// Appends `body` to `output`, padded to the field width: with spaces
    /// after it for `-`, with zeros after its first `prefix` bytes (a sign
    /// or base prefix) for `0`, else with spaces before it.
    /// C99: §7.19.6.1 paragraph 6, pp. 275-276; PDF pp. 287-288.
    fn pad(self, output: &mut crate::util::bump::ArenaVec<'_, u8>, body: &[u8], prefix: usize) {
        let padding = self.width.saturating_sub(body.len());
        if self.left {
            output.extend_from_slice(body);
            output.extend(std::iter::repeat_n(b' ', padding));
        } else if self.zero {
            output.extend_from_slice(&body[..prefix]);
            output.extend(std::iter::repeat_n(b'0', padding));
            output.extend_from_slice(&body[prefix..]);
        } else {
            output.extend(std::iter::repeat_n(b' ', padding));
            output.extend_from_slice(body);
        }
    }
}

/// The digits of one integer conversion, formatted without allocating.
struct Digits {
    // A 128-bit octal value needs 43 digits.
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
            .expect("integer conversions fit in the buffer");
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

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}
