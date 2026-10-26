//! `scanf`'s format interpretation for `sscanf`, `fscanf` and `scanf`:
//! white space, ordinary characters, and the conversions `d i u o x X c s
//! [ n e f g a %` (the floating ones decimal only, into `float` or
//! `double`), with assignment suppression, a field width and the length
//! modifiers `hh h l ll j z t`. Input comes from a string in memory or a
//! stream, with one character of pushback.
//!
//! C99: §7.19.6.2 paragraphs 3-14, pp. 282-288; PDF pp. 294-300.

use super::{
    ArgumentClass,
    DefaultHost,
    FormatArguments,
    integer,
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

/// How a conversion ended the directive loop, if it did.
enum Stop {
    /// The input ended (or failed) before the directive matched.
    Input,
    /// The input did not match the directive.
    Matching,
}

impl DefaultHost<'_> {
    /// Runs `format` against `input`, storing through `args`, and returns
    /// the number of assigned conversions, or `EOF` (-1) when the input
    /// ended before the first conversion.
    pub(super) fn scanf(
        &mut self,
        mut input: Input,
        format: RuntimeValue,
        args: &mut FormatArguments<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<i32, Fault> {
        let mut directives = super::ArenaVec::new_in(self.allocator);
        memory.read_c_string(format, &mut directives)?;
        let mut assigned = 0;
        let mut converted_any = false;
        let mut consumed: u64 = 0;
        let mut position = 0;
        let stop = loop {
            let Some(&directive) = directives.get(position) else {
                break None;
            };
            position += 1;
            if directive.is_ascii_whitespace() {
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
                        break Some(Stop::Matching);
                    },
                    | None => break Some(Stop::Input),
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
            let mut size = 4;
            let mut ells = 0;
            loop {
                match directives.get(position) {
                    | Some(b'h') => size /= 2,
                    | Some(b'l') => {
                        ells += 1;
                        size = if ells == 1 { self.long_width / 8 } else { 8 };
                    },
                    | Some(b'j' | b'z' | b't' | b'q') => size = 8,
                    | Some(b'L') => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
                    | _ => break,
                }
                position += 1;
            }
            let Some(&conversion) = directives.get(position) else {
                return Err(Fault::Unsupported(Unsupported::FormatConversion));
            };
            position += 1;
            if !matches!(conversion, b'[' | b'c' | b'n') {
                self.skip_space(&mut input, &mut consumed, memory)?;
            }
            let destination = if suppress {
                RuntimeValue::NULL
            } else {
                args.next(ArgumentClass::Pointer, memory)?
            };
            let store_int = |memory: &mut Memory<'_>, value: u128, size: u32| {
                let ty = match size {
                    | 1 => Type::I8,
                    | 2 => Type::I16,
                    | 8 => Type::I64,
                    | _ => Type::I32,
                };
                memory.store(ty, RuntimeValue::int(ty, value), destination, Align::BYTE)
            };
            match conversion {
                | b'n' => {
                    if !suppress {
                        store_int(memory, u128::from(consumed), size)?;
                    }
                    continue;
                },
                | b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                    let base = match conversion {
                        | b'd' | b'u' => 10,
                        | b'i' => 0,
                        | b'o' => 8,
                        | _ => 16,
                    };
                    let Some(value) =
                        self.scan_integer(&mut input, &mut consumed, width, base, memory)?
                    else {
                        break Some(if self.at_end(&mut input, memory)? {
                            Stop::Input
                        } else {
                            Stop::Matching
                        });
                    };
                    if !suppress {
                        store_int(memory, value, size)?;
                    }
                },
                | b'c' | b's' | b'[' => {
                    let (scanset, negated) = if conversion == b'[' {
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
                    while count < width {
                        let Some(byte) = self.next(&mut input, memory)? else {
                            break;
                        };
                        let accepted = match conversion {
                            | b'c' => true,
                            | b's' => !byte.is_ascii_whitespace(),
                            | _ => scanset[usize::from(byte)] != negated,
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
                        consumed += 1;
                    }
                    if count == 0 || (conversion == b'c' && count < width) {
                        break Some(if count == 0 && self.at_end(&mut input, memory)? {
                            Stop::Input
                        } else {
                            Stop::Matching
                        });
                    }
                    if conversion != b'c' && !suppress {
                        let at =
                            memory.ptr_add(destination, RuntimeValue::Int(u128::from(count)), true);
                        memory.store(Type::I8, RuntimeValue::Int(0), at, Align::BYTE)?;
                    }
                },
                | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                    let Some(value) = self.scan_float(&mut input, &mut consumed, width, memory)?
                    else {
                        break Some(if self.at_end(&mut input, memory)? {
                            Stop::Input
                        } else {
                            Stop::Matching
                        });
                    };
                    if !suppress {
                        if size == 4 {
                            #[expect(
                                clippy::cast_possible_truncation,
                                reason = "`%f` stores a float."
                            )]
                            let single = value as f32;
                            memory.store(
                                Type::F32,
                                RuntimeValue::F32(single),
                                destination,
                                Align::BYTE,
                            )?;
                        } else {
                            memory.store(
                                Type::F64,
                                RuntimeValue::F64(value),
                                destination,
                                Align::BYTE,
                            )?;
                        }
                    }
                },
                | _ => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
            }
            converted_any = true;
            assigned += i32::from(!suppress);
        };
        Ok(match stop {
            | Some(Stop::Input) if !converted_any && assigned == 0 => -1,
            | _ => assigned,
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
            if !byte.is_ascii_whitespace() {
                self.unread(input, byte);
                break;
            }
            *consumed += 1;
        }
        Ok(())
    }

    /// The bytes a `[` conversion accepts, and whether it is negated, after
    /// which `position` is past its closing `]`.
    fn scanset(directives: &[u8], position: &mut usize) -> ([bool; 256], bool) {
        let mut set = [false; 256];
        let negated = directives.get(*position) == Some(&b'^');
        *position += usize::from(negated);
        let mut first = true;
        while let Some(&byte) = directives.get(*position) {
            if byte == b']' && !first {
                *position += 1;
                break;
            }
            // A `-` between two characters is a range, as glibc reads it.
            if let (Some(b'-'), Some(&end)) =
                (directives.get(*position + 1), directives.get(*position + 2))
                && end != b']'
            {
                for member in byte..=end {
                    set[usize::from(member)] = true;
                }
                *position += 3;
            } else {
                set[usize::from(byte)] = true;
                *position += 1;
            }
            first = false;
        }
        (set, negated)
    }

    /// An integer as `strtol` reads it with `base`, at most `width` bytes;
    /// `None` when no digit matched.
    fn scan_integer(
        &mut self,
        input: &mut Input,
        consumed: &mut u64,
        width: u64,
        base: u32,
        memory: &Memory<'_>,
    ) -> Result<Option<u128>, Fault> {
        let mut used = 0;
        let mut take = |this: &mut Self,
                        input: &mut Input,
                        accept: &dyn Fn(u8) -> bool|
         -> Result<Option<u8>, Fault> {
            if used >= width {
                return Ok(None);
            }
            match this.next(input, memory)? {
                | Some(byte) if accept(byte) => {
                    used += 1;
                    *consumed += 1;
                    Ok(Some(byte))
                },
                | Some(byte) => {
                    this.unread(input, byte);
                    Ok(None)
                },
                | None => Ok(None),
            }
        };
        let negative = take(self, input, &|byte| byte == b'+' || byte == b'-')? == Some(b'-');
        let mut base = base;
        let mut digits = 0;
        let mut value = 0_u128;
        if matches!(base, 0 | 16) && take(self, input, &|byte| byte == b'0')?.is_some() {
            digits = 1;
            if take(self, input, &|byte| byte == b'x' || byte == b'X')?.is_some() {
                base = 16;
                digits = 0;
            } else if base == 0 {
                base = 8;
            }
        }
        if base == 0 {
            base = 10;
        }
        while let Some(byte) = take(self, input, &|byte: u8| char::from(byte).is_digit(base))? {
            let digit = char::from(byte).to_digit(base).expect("an accepted digit");
            value = value
                .wrapping_mul(u128::from(base))
                .wrapping_add(u128::from(digit));
            digits += 1;
        }
        Ok((digits > 0).then(|| {
            if negative {
                value.wrapping_neg()
            } else {
                value
            }
        }))
    }

    /// A decimal floating number, infinity or NaN as `strtod` reads it, at
    /// most `width` bytes; `None` when none matched. Hexadecimal input is
    /// unsupported.
    fn scan_float(
        &mut self,
        input: &mut Input,
        consumed: &mut u64,
        width: u64,
        memory: &Memory<'_>,
    ) -> Result<Option<f64>, Fault> {
        let mut text = super::ArenaVec::new_in(self.allocator);
        let mut accept = |this: &mut Self,
                          input: &mut Input,
                          text: &mut super::ArenaVec<'_, u8>,
                          allowed: &dyn Fn(u8) -> bool|
         -> Result<bool, Fault> {
            if (text.len() as u64) >= width {
                return Ok(false);
            }
            match this.next(input, memory)? {
                | Some(byte) if allowed(byte) => {
                    text.push(byte);
                    *consumed += 1;
                    Ok(true)
                },
                | Some(byte) => {
                    this.unread(input, byte);
                    Ok(false)
                },
                | None => Ok(false),
            }
        };
        let _ = accept(self, input, &mut text, &|byte| byte == b'+' || byte == b'-')?;
        let mut letters = 0;
        while accept(self, input, &mut text, &|byte| {
            b"infinityINFINITYaA".contains(&byte)
        })? {
            letters += 1;
        }
        if letters > 0 {
            let word = text
                .iter()
                .filter(|byte| byte.is_ascii_alphabetic())
                .count();
            if !(word == 3 || word == 8) {
                return Ok(None);
            }
        } else {
            let mut digits = 0;
            while accept(self, input, &mut text, &|byte| byte.is_ascii_digit())? {
                digits += 1;
            }
            if accept(self, input, &mut text, &|byte| byte == b'.')? {
                while accept(self, input, &mut text, &|byte| byte.is_ascii_digit())? {
                    digits += 1;
                }
            }
            if digits == 0 {
                return Ok(None);
            }
            if matches!(text.as_slice(), [b'0'] | [_, b'0'])
                && accept(self, input, &mut text, &|byte| byte == b'x' || byte == b'X')?
            {
                return Err(Fault::Unsupported(Unsupported::FormatConversion));
            }
            if accept(self, input, &mut text, &|byte| byte == b'e' || byte == b'E')? {
                let _ = accept(self, input, &mut text, &|byte| byte == b'+' || byte == b'-')?;
                let mut exponent = 0;
                while accept(self, input, &mut text, &|byte| byte.is_ascii_digit())? {
                    exponent += 1;
                }
                if exponent == 0 {
                    return Ok(None);
                }
            }
        }
        Ok(std::str::from_utf8(&text)
            .ok()
            .and_then(|text| text.parse::<f64>().ok()))
    }
}

#[expect(clippy::cast_possible_truncation, reason = "Truncation is the intent.")]
const fn truncate_u8(bits: u128) -> u8 {
    bits as u8
}
