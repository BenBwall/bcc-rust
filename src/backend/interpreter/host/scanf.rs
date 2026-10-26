//! `scanf`'s format interpretation for `sscanf`, `fscanf` and `scanf`, as
//! glibc reads input: white space, ordinary characters, and the conversions
//! `d i u o x X c s [ n e f g a E F G A %`, with assignment suppression, a
//! field width and the length modifiers `hh h l ll j z t q`. Input comes
//! from a string in memory or a stream, with one character of pushback.
//!
//! Every directive ends in success, a matching failure (the input does not
//! fit; the call returns the assignments so far) or an input failure (the
//! input ended first; the call returns `EOF` if nothing was assigned, as
//! glibc does even after a suppressed conversion). A numeric field that
//! stops after a prefix such as a sign, `0x`, `1e`, `nan(` or `infin` is a
//! matching failure, the prefix consumed. White space is the "C" locale's,
//! `\v` included. The wide forms `%lc`, `%ls` and `%l[` and `long double`
//! input (`%Lf`) are not implemented and trap as unsupported.
//!
//! C99: §7.19.6.2 paragraphs 3-14, pp. 282-288; PDF pp. 294-300.

use super::{
    ArgumentClass,
    DefaultHost,
    FormatArguments,
    integer,
    strings::is_space,
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

/// Where a `scanf` call reads.
#[derive(Clone, Copy)]
#[expect(
    variant_size_differences,
    reason = "A short-lived cursor; boxing the string position would allocate."
)]
pub(super) enum Input {
    /// The C string at `pointer`, from byte `offset`.
    String { pointer: RuntimeValue, offset: u64 },
    /// The stream with this table index.
    Stream(usize),
}

/// Why a directive failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Failure {
    /// The input ended (or failed) before the directive matched.
    Input,
    /// The input did not match the directive.
    Matching,
}

/// A length modifier, interpreted per conversion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Length {
    None,
    Char,
    Short,
    Long,
    LongLong,
    /// `j`, `z`, `t`: 64 bits on every target.
    Wide,
    /// `L`.
    LongDouble,
}

impl DefaultHost<'_> {
    /// Runs `format` against `input`, storing through `args`, and returns
    /// the number of assigned conversions, or `EOF` (-1) when an input
    /// failure comes before any assignment.
    pub(super) fn scanf(
        &mut self,
        mut input: Input,
        format: RuntimeValue,
        args: &mut FormatArguments<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<i32, Fault> {
        let mut directives = super::ArenaVec::new_in(self.allocator);
        memory.read_c_string(format, &mut directives)?;
        let mut assigned: i32 = 0;
        let mut consumed: u64 = 0;
        let mut position = 0;
        let failure = loop {
            let Some(&directive) = directives.get(position) else {
                break None;
            };
            position += 1;
            if is_space(directive) {
                self.skip_space(&mut input, &mut consumed, memory)?;
                continue;
            }
            if directive != b'%' || directives.get(position) == Some(&b'%') {
                if directive == b'%' {
                    position += 1;
                    self.skip_space(&mut input, &mut consumed, memory)?;
                }
                match self.next(&mut input, memory)? {
                    | Some(byte) if byte == directive => consumed += 1,
                    | Some(byte) => {
                        self.unread(&mut input, byte);
                        break Some(Failure::Matching);
                    },
                    | None => break Some(Failure::Input),
                }
                continue;
            }
            let suppress = directives.get(position) == Some(&b'*');
            position += usize::from(suppress);
            let mut width = 0_u64;
            while let Some(&digit @ b'0'..=b'9') = directives.get(position) {
                width = width
                    .saturating_mul(10)
                    .saturating_add(u64::from(digit - b'0'));
                position += 1;
            }
            let width = if width == 0 { u64::MAX } else { width };
            let length = Self::length(&directives, &mut position);
            let Some(&conversion) = directives.get(position) else {
                return Err(Fault::Unsupported(Unsupported::FormatConversion));
            };
            position += 1;
            let size = self.store_size(conversion, length)?;
            let destination = if suppress {
                RuntimeValue::NULL
            } else {
                args.next(ArgumentClass::Pointer, memory)?
            };
            if conversion == b'n' {
                if !suppress {
                    store_integer(memory, destination, u128::from(consumed), size)?;
                }
                continue;
            }
            if !matches!(conversion, b'[' | b'c') {
                self.skip_space(&mut input, &mut consumed, memory)?;
                if self.at_end(&mut input, memory)? {
                    break Some(Failure::Input);
                }
            }
            let mut field = Field {
                width,
                used: 0,
                text: super::ArenaVec::new_in(self.allocator),
            };
            let result = match conversion {
                | b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                    let base = match conversion {
                        | b'd' | b'u' => 10,
                        | b'i' => 0,
                        | b'o' => 8,
                        | _ => 16,
                    };
                    self.scan_integer(&mut input, &mut field, base, memory)?
                        .map(|value| {
                            if !suppress {
                                store_integer(memory, destination, value, size)?;
                            }
                            Ok::<(), Fault>(())
                        })
                        .transpose()?
                },
                | b'c' | b's' | b'[' => {
                    let (set, negated) = if conversion == b'[' {
                        Self::scanset(&directives, &mut position)
                    } else {
                        ([false; 256], false)
                    };
                    let width = if conversion == b'c' && width == u64::MAX {
                        1
                    } else {
                        width
                    };
                    let mut count = 0;
                    let mut ended = false;
                    while count < width {
                        let Some(byte) = self.next(&mut input, memory)? else {
                            ended = true;
                            break;
                        };
                        let accepted = match conversion {
                            | b'c' => true,
                            | b's' => !is_space(byte),
                            | _ => set[usize::from(byte)] != negated,
                        };
                        if !accepted {
                            self.unread(&mut input, byte);
                            break;
                        }
                        if !suppress {
                            let at = memory.ptr_add(
                                destination,
                                RuntimeValue::Int(u128::from(count)),
                                true,
                            );
                            memory.store(
                                Type::I8,
                                RuntimeValue::Int(u128::from(byte)),
                                at,
                                Align::BYTE,
                            )?;
                        }
                        count += 1;
                    }
                    consumed += count;
                    // `%c` needs all its characters; `%[` and `%s` at least
                    // one. Running out of input is an input failure.
                    if (conversion == b'c' && count < width) || count == 0 {
                        break Some(if ended {
                            Failure::Input
                        } else {
                            Failure::Matching
                        });
                    }
                    if conversion != b'c' && !suppress {
                        let at =
                            memory.ptr_add(destination, RuntimeValue::Int(u128::from(count)), true);
                        memory.store(Type::I8, RuntimeValue::Int(0), at, Align::BYTE)?;
                    }
                    Some(())
                },
                | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                    let single = size == 4;
                    self.scan_float(&mut input, &mut field, single, memory)?
                        .map(|bits| {
                            if !suppress {
                                let value = if single {
                                    RuntimeValue::F32(f32::from_bits(
                                        u32::try_from(bits).expect("binary32 bits"),
                                    ))
                                } else {
                                    RuntimeValue::F64(f64::from_bits(bits))
                                };
                                let ty = if single { Type::F32 } else { Type::F64 };
                                memory.store(ty, value, destination, Align::BYTE)?;
                            }
                            Ok::<(), Fault>(())
                        })
                        .transpose()?
                },
                | _ => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
            };
            if !matches!(conversion, b'c' | b's' | b'[') {
                consumed += field.used;
                if result.is_none() {
                    break Some(Failure::Matching);
                }
            }
            assigned = assigned.saturating_add(i32::from(!suppress));
        };
        Ok(match failure {
            | Some(Failure::Input) if assigned == 0 => -1,
            | _ => assigned,
        })
    }

    /// The length modifier at `position`, advancing past it.
    fn length(directives: &[u8], position: &mut usize) -> Length {
        let mut length = Length::None;
        loop {
            length = match (directives.get(*position), length) {
                | (Some(b'h'), Length::Short) => Length::Char,
                | (Some(b'h'), _) => Length::Short,
                | (Some(b'l'), Length::Long) | (Some(b'q'), _) => Length::LongLong,
                | (Some(b'l'), _) => Length::Long,
                | (Some(b'j' | b'z' | b't'), _) => Length::Wide,
                | (Some(b'L'), _) => Length::LongDouble,
                | _ => return length,
            };
            *position += 1;
        }
    }

    /// The bytes a conversion stores under `length`: an integer's width, 4 or
    /// 8 for a floating conversion, 1 for the character conversions. The
    /// combinations the host does not implement are refused.
    fn store_size(&self, conversion: u8, length: Length) -> Result<u32, Fault> {
        let unsupported = Err(Fault::Unsupported(Unsupported::FormatConversion));
        Ok(match conversion {
            | b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'n' => match length {
                | Length::Char => 1,
                | Length::Short => 2,
                | Length::None => 4,
                | Length::Long => self.long_width / 8,
                // glibc reads `L` with an integer conversion as `ll`.
                | Length::LongLong | Length::Wide | Length::LongDouble => 8,
            },
            | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => match length {
                | Length::None => 4,
                | Length::Long => 8,
                // glibc reads `ll` and `q` with a floating conversion as `L`.
                | _ => return unsupported,
            },
            | b'c' | b's' | b'[' if length == Length::None => 1,
            | _ => return unsupported,
        })
    }

    /// The next input byte, or `None` at the end.
    fn next(&mut self, input: &mut Input, memory: &Memory<'_>) -> Result<Option<u8>, Fault> {
        match input {
            | Input::String { pointer, offset } => {
                let at = memory.ptr_add(*pointer, RuntimeValue::Int(u128::from(*offset)), true);
                let byte = truncate_u8(integer(memory.load(Type::I8, at, Align::BYTE)?)?);
                if byte == 0 {
                    return Ok(None);
                }
                *offset += 1;
                Ok(Some(byte))
            },
            | Input::Stream(stream) => Ok(self.read(*stream)),
        }
    }

    /// Pushes back the byte `next` just returned.
    fn unread(&mut self, input: &mut Input, byte: u8) {
        match input {
            | Input::String { offset, .. } => *offset -= 1,
            | Input::Stream(stream) => self.push_back(*stream, byte),
        }
    }

    fn at_end(&mut self, input: &mut Input, memory: &Memory<'_>) -> Result<bool, Fault> {
        Ok(match self.next(input, memory)? {
            | Some(byte) => {
                self.unread(input, byte);
                false
            },
            | None => true,
        })
    }

    fn skip_space(
        &mut self,
        input: &mut Input,
        consumed: &mut u64,
        memory: &Memory<'_>,
    ) -> Result<(), Fault> {
        while let Some(byte) = self.next(input, memory)? {
            if !is_space(byte) {
                self.unread(input, byte);
                break;
            }
            *consumed += 1;
        }
        Ok(())
    }

    /// The bytes a `[` conversion accepts, and whether it is negated, after
    /// which `position` is past its closing `]`. glibc's reading: a `]` or
    /// `-` first is a member; a later `-` between two characters adds the
    /// characters from the one before it up to the one after it when they
    /// ascend, so `a-b-d` is `a` to `d` without `-`, and is a member itself
    /// otherwise, so `z-a` is `z`, `-` and `a`.
    fn scanset(directives: &[u8], position: &mut usize) -> ([bool; 256], bool) {
        let mut set = [false; 256];
        let negated = directives.get(*position) == Some(&b'^');
        *position += usize::from(negated);
        if let Some(&first @ (b']' | b'-')) = directives.get(*position) {
            set[usize::from(first)] = true;
            *position += 1;
        }
        while let Some(&byte) = directives.get(*position) {
            *position += 1;
            if byte == b']' {
                break;
            }
            // The character before this one; a `-` is never first here.
            let previous = directives[*position - 2];
            match directives.get(*position) {
                | Some(&next) if byte == b'-' && next != b']' && previous <= next =>
                    for member in previous..next {
                        set[usize::from(member)] = true;
                    },
                | _ => set[usize::from(byte)] = true,
            }
        }
        (set, negated)
    }

    /// Takes the next byte into `field` if it satisfies `accept` and the
    /// width allows it.
    fn take(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        memory: &Memory<'_>,
        accept: impl Fn(u8) -> bool,
    ) -> Result<Option<u8>, Fault> {
        if field.used >= field.width {
            return Ok(None);
        }
        match self.next(input, memory)? {
            | Some(byte) if accept(byte) => {
                field.used += 1;
                field.text.push(byte);
                Ok(Some(byte))
            },
            | Some(byte) => {
                self.unread(input, byte);
                Ok(None)
            },
            | None => Ok(None),
        }
    }

    /// Takes the bytes of `word`, case-insensitively, saying whether all
    /// matched.
    fn take_word(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        memory: &Memory<'_>,
        word: &[u8],
    ) -> Result<bool, Fault> {
        for &expected in word {
            if self
                .take(input, field, memory, |byte| {
                    byte.eq_ignore_ascii_case(&expected)
                })?
                .is_none()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// An integer as `strtol` reads it with `base`, within the field width;
    /// `None` (a matching failure) when no digit follows the sign and
    /// prefix, including a `0x` with no hexadecimal digit after it.
    fn scan_integer(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        base: u32,
        memory: &Memory<'_>,
    ) -> Result<Option<u128>, Fault> {
        let negative =
            self.take(input, field, memory, |byte| byte == b'+' || byte == b'-')? == Some(b'-');
        let mut base = base;
        let mut digits = 0_u64;
        if matches!(base, 0 | 16)
            && self
                .take(input, field, memory, |byte| byte == b'0')?
                .is_some()
        {
            if self
                .take(input, field, memory, |byte| byte == b'x' || byte == b'X')?
                .is_some()
            {
                base = 16;
            } else {
                digits = 1;
                if base == 0 {
                    base = 8;
                }
            }
        }
        if base == 0 {
            base = 10;
        }
        let mut value = 0_u128;
        while let Some(byte) = self.take(input, field, memory, |byte: u8| {
            char::from(byte).is_digit(base)
        })? {
            let digit = char::from(byte).to_digit(base).expect("an accepted digit");
            value = value
                .wrapping_mul(u128::from(base))
                .wrapping_add(u128::from(digit));
            digits = digits.saturating_add(1);
        }
        Ok((digits > 0).then(|| {
            if negative {
                value.wrapping_neg()
            } else {
                value
            }
        }))
    }

    /// A floating number as `strtod` reads it, within the field width:
    /// decimal, hexadecimal with an optional binary exponent, `inf`,
    /// `infinity`, or `nan` with an optional `(n-char-sequence)`; the bits
    /// of the nearest binary32 (`single`) or binary64 value, or `None` (a
    /// matching failure) when the field stops inside one of those forms.
    fn scan_float(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        single: bool,
        memory: &Memory<'_>,
    ) -> Result<Option<u64>, Fault> {
        let negative =
            self.take(input, field, memory, |byte| byte == b'+' || byte == b'-')? == Some(b'-');
        let start = field.text.len();
        let (sign_bit, infinity, quiet, payload_bits) = if single {
            (1_u64 << 31, 0x7F80_0000_u64, 0x0040_0000_u64, 22)
        } else {
            (
                1_u64 << 63,
                0x7FF0_0000_0000_0000,
                0x0008_0000_0000_0000,
                51,
            )
        };
        let sign = if negative { sign_bit } else { 0 };
        let first = self.take(input, field, memory, |byte| {
            byte.is_ascii_digit() || matches!(byte, b'.' | b'i' | b'I' | b'n' | b'N')
        })?;
        match first {
            | Some(b'i' | b'I') => {
                if !self.take_word(input, field, memory, b"nf")? {
                    return Ok(None);
                }
                // `infinity` must be spelled out once it continues.
                if self
                    .take(input, field, memory, |byte| {
                        byte.eq_ignore_ascii_case(&b'i')
                    })?
                    .is_some()
                    && !self.take_word(input, field, memory, b"nity")?
                {
                    return Ok(None);
                }
                return Ok(Some(sign | infinity));
            },
            | Some(b'n' | b'N') => {
                if !self.take_word(input, field, memory, b"an")? {
                    return Ok(None);
                }
                let mut payload = 0;
                if self
                    .take(input, field, memory, |byte| byte == b'(')?
                    .is_some()
                {
                    let open = field.text.len();
                    while self
                        .take(input, field, memory, |byte| {
                            byte.is_ascii_alphanumeric() || byte == b'_'
                        })?
                        .is_some()
                    {}
                    let sequence = std::str::from_utf8(&field.text[open..]).unwrap_or("");
                    payload = super::nan_payload(sequence);
                    if self
                        .take(input, field, memory, |byte| byte == b')')?
                        .is_none()
                    {
                        return Ok(None);
                    }
                }
                return Ok(Some(
                    sign | infinity | quiet | (payload & ((1 << payload_bits) - 1)),
                ));
            },
            | None => return Ok(None),
            | Some(_) => {},
        }
        if field.text[start..] == *b"0"
            && self
                .take(input, field, memory, |byte| byte == b'x' || byte == b'X')?
                .is_some()
        {
            return self.scan_hexadecimal(input, field, single, negative, memory);
        }
        let mut digits = usize::from(field.text[start].is_ascii_digit());
        let mut point = field.text[start] == b'.';
        loop {
            match self.take(input, field, memory, |byte| {
                byte.is_ascii_digit() || (!point && byte == b'.')
            })? {
                | Some(b'.') => point = true,
                | Some(_) => digits = digits.saturating_add(1),
                | None => break,
            }
        }
        if digits == 0 {
            return Ok(None);
        }
        if self
            .take(input, field, memory, |byte| byte == b'e' || byte == b'E')?
            .is_some()
            && !self.exponent_digits(input, field, memory)?
        {
            return Ok(None);
        }
        let text = std::str::from_utf8(&field.text[start..]).expect("ASCII digits");
        Ok(Some(if single {
            u64::from(text.parse::<f32>().map_or(0, f32::to_bits)) | sign
        } else {
            text.parse::<f64>().map_or(0, f64::to_bits) | sign
        }))
    }

    /// A sign and at least one decimal digit, after an exponent letter.
    fn exponent_digits(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        memory: &Memory<'_>,
    ) -> Result<bool, Fault> {
        let _ = self.take(input, field, memory, |byte| byte == b'+' || byte == b'-')?;
        let mut digits = 0_u64;
        while self
            .take(input, field, memory, |byte| byte.is_ascii_digit())?
            .is_some()
        {
            digits = digits.saturating_add(1);
        }
        Ok(digits > 0)
    }

    /// The rest of a hexadecimal floating number after its `0x`: digits
    /// with an optional point, then an optional `p` exponent, rounded once.
    fn scan_hexadecimal(
        &mut self,
        input: &mut Input,
        field: &mut Field<'_>,
        single: bool,
        negative: bool,
        memory: &Memory<'_>,
    ) -> Result<Option<u64>, Fault> {
        let (mut magnitude, mut exponent, mut sticky, mut digits) = (0_u128, 0_i64, false, 0);
        let mut point = false;
        while let Some(byte) = self.take(input, field, memory, |byte| {
            byte.is_ascii_hexdigit() || (!point && byte == b'.')
        })? {
            if byte == b'.' {
                point = true;
                continue;
            }
            digits += 1;
            let digit = u128::from(char::from(byte).to_digit(16).expect("a hexadecimal digit"));
            // Keep 120 bits; later digits only move the exponent or set the
            // sticky bit.
            if magnitude >> 120 == 0 {
                magnitude = magnitude << 4 | digit;
                exponent = exponent.saturating_sub(if point { 4 } else { 0 });
            } else {
                sticky |= digit != 0;
                exponent = exponent.saturating_add(if point { 0 } else { 4 });
            }
        }
        if digits == 0 {
            return Ok(None);
        }
        if self
            .take(input, field, memory, |byte| byte == b'p' || byte == b'P')?
            .is_some()
        {
            let start = field.text.len();
            if !self.exponent_digits(input, field, memory)? {
                return Ok(None);
            }
            let text = std::str::from_utf8(&field.text[start..]).expect("ASCII digits");
            // An exponent past i64 saturates; any that large already
            // overflows or underflows, as the clamp below keeps it.
            let written = text.parse::<i64>().unwrap_or(if text.starts_with('-') {
                i64::MIN
            } else {
                i64::MAX
            });
            exponent = exponent.saturating_add(written);
        }
        let exponent = i32::try_from(exponent.clamp(-0x10_0000, 0x10_0000)).expect("clamped");
        let magnitude = (magnitude << 1) | u128::from(sticky);
        Ok(Some(crate::x87::nearest_binary(
            single,
            negative,
            exponent - 1,
            magnitude,
        )))
    }
}

/// One conversion's characters, within its field width.
struct Field<'a> {
    width: u64,
    used:  u64,
    text:  super::ArenaVec<'a, u8>,
}

/// Stores the integer `value` in `size` bytes at `destination`.
fn store_integer(
    memory: &mut Memory<'_>,
    destination: RuntimeValue,
    value: u128,
    size: u32,
) -> Result<(), Fault> {
    let ty = match size {
        | 1 => Type::I8,
        | 2 => Type::I16,
        | 8 => Type::I64,
        | _ => Type::I32,
    };
    memory.store(ty, RuntimeValue::int(ty, value), destination, Align::BYTE)
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}
