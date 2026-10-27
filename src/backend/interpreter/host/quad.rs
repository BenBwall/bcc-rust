//! glibc's `_Float128` functions for the default host: `strtof128`,
//! `strfromf128`, and the `<math.h>` functions whose binary128 results are
//! exact or correctly rounded, computed by `crate::binary128` bit for bit as
//! glibc computes them. `printf` has no binary128 conversion, so programs
//! print through `strfromf128`, whose digits [`floating::format`] gives
//! exactly, as glibc does.
//!
//! `strtof128` reads the longest prefix glibc's `strtod` family accepts:
//! leading white space, a sign, then `inf`/`infinity`, `nan` with an
//! optional `(n-char-sequence)`, or a decimal or hexadecimal significand
//! with an optional exponent, and rounds it once to nearest.
//! C99: §7.20.1.3, pp. 307-310; PDF pp. 319-322 (the `strtod` family, which
//! TS 18661-3 extends to `strtof128`); §7.12.6-§7.12.12, pp. 223-238;
//! PDF pp. 235-250. `strfromf128` is TS 18661-3's (C23 §7.24.1.3).

use std::cmp::Ordering;

use super::{
    DefaultHost,
    Fault,
    HostCall,
    HostReturn,
    Memory,
    RuntimeValue,
    Type,
    UbKind,
    Unsupported,
    floating::{
        self,
        Floating,
        PositiveSign,
    },
    integer,
};
use crate::{
    binary128::Binary128,
    float_parsing::ParseFloatError,
    ir::Align,
    translation_phases::preprocessing::FloatTokenType,
    x87::Integral,
};

/// The functions this file implements.
pub(super) const FUNCTIONS: &[&str] = &[
    "strtof128",
    "strfromf128",
    "fabsf128",
    "copysignf128",
    "sqrtf128",
    "ceilf128",
    "floorf128",
    "truncf128",
    "roundf128",
    "rintf128",
    "nearbyintf128",
    "fmodf128",
    "fmaxf128",
    "fminf128",
    "fdimf128",
    "ldexpf128",
    "scalbnf128",
    "frexpf128",
    "nanf128",
];

impl DefaultHost<'_> {
    /// Runs `call` if it names one of [`FUNCTIONS`]; the caller has checked
    /// its signature.
    pub(super) fn quad(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Option<Result<HostReturn, Fault>> {
        FUNCTIONS
            .contains(&call.name)
            .then(|| self.run_quad(call, memory))
    }

    fn run_quad(
        &mut self,
        call: HostCall<'_>,
        memory: &mut Memory<'_>,
    ) -> Result<HostReturn, Fault> {
        let args = call.args;
        if args.iter().any(|value| value.is_poison()) {
            return Err(UbKind::PoisonInHostCall.into());
        }
        let quad = |index: usize| match args.get(index) {
            | Some(RuntimeValue::F128(bits)) => Ok(Binary128::from_bits(*bits)),
            | _ => Err(Fault::from(UbKind::SignatureMismatch)),
        };
        let value = match call.name {
            | "strtof128" => self.strtof128(args[0], args[1], memory)?,
            | "strfromf128" => {
                let count = self.strfromf128(args, quad(3)?, memory)?;
                return Ok(HostReturn::Value(Some(RuntimeValue::int(
                    Type::I32,
                    u128::from(count.cast_unsigned()),
                ))));
            },
            | "fabsf128" => quad(0)?.with_sign(false),
            | "copysignf128" => quad(0)?.with_sign(quad(1)?.is_negative()),
            | "sqrtf128" => quad(0)?.sqrt(),
            | "ceilf128" => quad(0)?.round_integral(Integral::Ceil),
            | "floorf128" => quad(0)?.round_integral(Integral::Floor),
            | "truncf128" => quad(0)?.round_integral(Integral::Truncate),
            | "roundf128" => quad(0)?.round_half_away(),
            | "rintf128" | "nearbyintf128" => quad(0)?.round_integral(Integral::HalfEven),
            | "fmodf128" => quad(0)?.rem(quad(1)?),
            | "fmaxf128" | "fminf128" =>
                minimum_maximum(call.name == "fminf128", quad(0)?, quad(1)?),
            | "fdimf128" => {
                let (x, y) = (quad(0)?, quad(1)?);
                if x.compare(y).is_some_and(Ordering::is_le) {
                    Binary128::zero(false)
                } else {
                    x.sub(y)
                }
            },
            | "ldexpf128" | "scalbnf128" => {
                let power = integer(args[1])?;
                let power = i64::from(u32::try_from(power).map_or(0, u32::cast_signed));
                quad(0)?.scale(power)
            },
            | "frexpf128" => {
                let (fraction, exponent) = quad(0)?.decompose();
                memory.store(
                    Type::I32,
                    RuntimeValue::int(Type::I32, u128::from(exponent.cast_unsigned())),
                    args[1],
                    Align::BYTE,
                )?;
                fraction
            },
            | "nanf128" => {
                self.text.clear();
                memory.read_c_string(args[0], &mut self.text)?;
                let payload = super::nan_payload(std::str::from_utf8(&self.text).unwrap_or(""));
                Binary128::nan(true, payload)
            },
            | _ => unreachable!("{} is not a binary128 function", call.name),
        };
        Ok(HostReturn::Value(Some(RuntimeValue::F128(value.to_bits()))))
    }

    /// `strtof128(nptr, endptr)`: the value of the longest prefix of `nptr`
    /// that forms a floating number, storing where it ends in `*endptr`
    /// (or `nptr` itself when there is none) unless `endptr` is null.
    /// C99: §7.20.1.3 paragraphs 2-10, pp. 307-309; PDF pp. 319-321.
    fn strtof128(
        &mut self,
        string: RuntimeValue,
        end: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<Binary128, Fault> {
        self.text.clear();
        memory.read_c_string(string, &mut self.text)?;
        self.scratch.clear();
        let (value, consumed) = scan(&self.text, &mut self.scratch, self.allocator);
        if end != RuntimeValue::NULL {
            let stop = memory.ptr_add(string, RuntimeValue::Int(consumed as u128), true);
            memory.store(Type::Ptr, stop, end, Align::BYTE)?;
        }
        Ok(value)
    }

    /// `strfromf128(str, n, format, fp)`: `fp` formatted by `format`, which
    /// is `%`, an optional precision and one of `aAeEfFgG`, written as
    /// `snprintf` writes it; returns the length of the whole text.
    /// TS 18661-3 §12.4.2 (C23 §7.24.1.3 paragraphs 2-4).
    fn strfromf128(
        &mut self,
        args: &[RuntimeValue],
        value: Binary128,
        memory: &mut Memory<'_>,
    ) -> Result<i32, Fault> {
        self.scratch.clear();
        memory.read_c_string(args[2], &mut self.scratch)?;
        let (precision, conversion) = match self.scratch.as_slice() {
            | [b'%', b'.', digits @ .., conversion] if digits.iter().all(u8::is_ascii_digit) => {
                let precision = digits.iter().try_fold(0_usize, |total, digit| {
                    total
                        .checked_mul(10)?
                        .checked_add(usize::from(digit - b'0'))
                        .filter(|&total| total <= super::printf::MAX_FORMAT_WIDTH)
                });
                (
                    Some(precision.ok_or(Fault::Unsupported(Unsupported::FormatWidth))?),
                    *conversion,
                )
            },
            | [b'%', conversion] => (None, *conversion),
            | _ => return Err(Fault::Unsupported(Unsupported::FormatConversion)),
        };
        if !b"aAeEfFgG".contains(&conversion) {
            return Err(Fault::Unsupported(Unsupported::FormatConversion));
        }
        let limit = integer(args[1])?;
        let before = self.output.len();
        self.text.clear();
        _ = floating::format(
            Floating::Quad(value),
            conversion,
            precision,
            false,
            PositiveSign::None,
            &mut self.text,
            &mut self.floating,
        );
        self.output.extend_from_slice(&self.text);
        let count = self.output.len() - before;
        let result = self.store_formatted(before, args[0], limit, memory);
        self.output.truncate(before);
        result?;
        Ok(i32::try_from(count).unwrap_or(i32::MAX))
    }
}

/// The `fmaxf128` or `fminf128` of two values: a NaN operand gives the other
/// operand, and of equal values the first, as glibc's generic code gives.
/// C99: §F.9.9.2 paragraph 2, p. 466; PDF p. 478.
fn minimum_maximum(minimum: bool, x: Binary128, y: Binary128) -> Binary128 {
    match x.compare(y) {
        | Some(order) => {
            let take_x = if minimum {
                order.is_le()
            } else {
                order.is_ge()
            };
            if take_x { x } else { y }
        },
        // Two NaNs, or a signaling one, give a NaN computed from both.
        | None if x.is_nan() && y.is_nan() => x.add(y),
        | None if x.is_nan() => y,
        | None => x,
    }
}

/// The value of the longest prefix of `text` that `strtod` reads as a
/// floating number, and how many bytes it takes (zero when there is
/// none). `spelling` is scratch space for the significand and exponent.
fn scan(
    text: &[u8],
    spelling: &mut crate::util::bump::ArenaVec<'_, u8>,
    arena: &crate::util::bump::Bump,
) -> (Binary128, usize) {
    let mut index = text
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r'))
        .count();
    let negative = text.get(index) == Some(&b'-');
    if matches!(text.get(index), Some(b'-' | b'+')) {
        index += 1;
    }
    let rest = &text[index..];
    let starts_with = |prefix: &[u8]| {
        rest.len() >= prefix.len() && rest[..prefix.len()].eq_ignore_ascii_case(prefix)
    };
    if starts_with(b"infinity") {
        return (Binary128::infinity(negative), index + 8);
    }
    if starts_with(b"inf") {
        return (Binary128::infinity(negative), index + 3);
    }
    if starts_with(b"nan") {
        let mut end = index + 3;
        let mut payload = "";
        if text.get(end) == Some(&b'(') {
            let sequence = text[end + 1..]
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                .count();
            if text.get(end + 1 + sequence) == Some(&b')') {
                payload = std::str::from_utf8(&text[end + 1..end + 1 + sequence]).unwrap_or("");
                end += sequence + 2;
            }
        }
        let payload = super::nan_payload(payload);
        return (Binary128::nan(true, payload).with_sign(negative), end);
    }
    let hex = (starts_with(b"0x"))
        && rest
            .get(2..)
            .is_some_and(|digits| significand_length(digits, true) > 0);
    let (start, radix_hex) = if hex {
        (index + 2, true)
    } else {
        (index, false)
    };
    let length = significand_length(&text[start..], radix_hex);
    if length == 0 {
        return (Binary128::zero(false), 0);
    }
    let mut end = start + length;
    spelling.extend_from_slice(if hex { b"0x" } else { b"" });
    spelling.extend_from_slice(&text[start..end]);
    let marker: &[u8] = if hex { b"pP" } else { b"eE" };
    let exponent = text
        .get(end)
        .is_some_and(|byte| marker.contains(byte))
        .then(|| {
            let sign = usize::from(matches!(text.get(end + 1), Some(b'+' | b'-')));
            let digits = text[end + 1 + sign..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            (digits > 0).then_some(1 + sign + digits)
        });
    if let Some(Some(length)) = exponent {
        spelling.extend_from_slice(&text[end..end + length]);
        end += length;
    } else {
        // A floating constant needs an exponent or a point; zero changes
        // nothing.
        spelling.extend_from_slice(if hex { b"p0" } else { b"e0" });
    }
    spelling.push(b'q');
    let spelling = std::str::from_utf8(spelling).unwrap_or("");
    let value = match crate::binary128::parse(spelling, arena) {
        | Ok(value) | Err(ParseFloatError::OutOfRange(FloatTokenType::Float128(value), _)) => value,
        | Err(_) => Binary128::zero(false),
    };
    (value.with_sign(negative), end)
}

/// The length of the significand at the start of `text`: digits of the
/// radix with at most one point, holding at least one digit.
fn significand_length(text: &[u8], hex: bool) -> usize {
    let digit = |byte: &u8| {
        if hex {
            byte.is_ascii_hexdigit()
        } else {
            byte.is_ascii_digit()
        }
    };
    let whole = text.iter().take_while(|byte| digit(byte)).count();
    let fraction = if text.get(whole) == Some(&b'.') {
        text[whole + 1..]
            .iter()
            .take_while(|byte| digit(byte))
            .count()
    } else {
        0
    };
    match (whole, fraction, text.get(whole) == Some(&b'.')) {
        | (0, 0, _) => 0,
        | (_, _, true) => whole + 1 + fraction,
        | _ => whole,
    }
}
