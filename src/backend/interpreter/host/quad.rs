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
//!
//! The functions set `errno` as glibc's `_Float128` wrappers do (glibc 2.39
//! and 2.43 agree): `sqrtf128` of a negative number and `fmodf128` of an
//! infinity or by zero (neither a NaN) are domain errors (`EDOM`);
//! `ldexpf128` and `scalbnf128` of a finite nonzero number that overflows
//! or underflows to zero, `fdimf128` that overflows, and `strtof128` of a
//! value that overflows, underflows to zero or is subnormal and inexact are
//! range errors (`ERANGE`). The others set none.
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
    binary128::{
        Binary128,
        Class,
    },
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
        let mut range_error = false;
        let value = match call.name {
            | "strtof128" => {
                let (value, out_of_range) = self.strtof128(args[0], args[1], memory)?;
                range_error = out_of_range;
                value
            },
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
                Binary128::nan(true, u128::from(payload))
            },
            | _ => unreachable!("{} is not a binary128 function", call.name),
        };
        let code = if range_error {
            super::errno::ERANGE
        } else {
            errno(call.name, quad(0).ok(), quad(1).ok(), value)
        };
        if code != 0 {
            self.set_errno(memory, code)?;
        }
        Ok(HostReturn::Value(Some(RuntimeValue::F128(value.to_bits()))))
    }

    /// `strtof128(nptr, endptr)`: the value of the longest prefix of `nptr`
    /// that forms a floating number, storing where it ends in `*endptr`
    /// (or `nptr` itself when there is none) unless `endptr` is null, and
    /// whether it is a range error.
    /// C99: §7.20.1.3 paragraphs 2-10, pp. 307-309; PDF pp. 319-321.
    fn strtof128(
        &mut self,
        string: RuntimeValue,
        end: RuntimeValue,
        memory: &mut Memory<'_>,
    ) -> Result<(Binary128, bool), Fault> {
        self.text.clear();
        memory.read_c_string(string, &mut self.text)?;
        self.scratch.clear();
        let (value, consumed, out_of_range) = scan(&self.text, &mut self.scratch, self.allocator);
        if end != RuntimeValue::NULL {
            let stop = memory.ptr_add(string, RuntimeValue::Int(consumed as u128), true);
            memory.store(Type::Ptr, stop, end, Align::BYTE)?;
        }
        Ok((value, out_of_range))
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

/// The `fmaxf128` or `fminf128` of two values, as glibc's generic code
/// computes it: the ordered choice (the first of equal values), else, when
/// either is a signaling NaN, `x + y`, which quiets it, else the operand
/// that is not a NaN (the first when both are quiet NaNs).
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
        | None if x.is_signaling() || y.is_signaling() => x.add(y),
        | None if y.is_nan() => x,
        | None => y,
    }
}

/// The `errno` glibc's binary128 function `name` sets for the arguments
/// `x` and `y` (where it has them) and its `result`, or zero.
/// C99: §7.12.1 paragraphs 2-4, pp. 214-215; PDF pp. 226-227.
fn errno(name: &str, x: Option<Binary128>, y: Option<Binary128>, result: Binary128) -> i32 {
    use super::errno::{
        EDOM,
        ERANGE,
    };
    let finite = |value: Binary128| matches!(value.class(), Class::Zero | Class::Finite { .. });
    let infinite = |value: Binary128| value.class() == Class::Infinite;
    let less_than_zero = |value: Binary128| {
        value
            .compare(Binary128::zero(false))
            .is_some_and(Ordering::is_lt)
    };
    match (name, x, y) {
        | ("sqrtf128", Some(x), _) if less_than_zero(x) => EDOM,
        | ("fmodf128", Some(x), Some(y))
            if (infinite(x) || y.is_zero()) && !x.is_nan() && !y.is_nan() =>
            EDOM,
        | ("ldexpf128" | "scalbnf128", Some(x), _)
            if finite(x) && !x.is_zero() && (!finite(result) || result.is_zero()) =>
            ERANGE,
        | ("fdimf128", Some(x), Some(y)) if infinite(result) && !infinite(x) && !infinite(y) =>
            ERANGE,
        | _ => 0,
    }
}

/// The value of the longest prefix of `text` that `strtod` reads as a
/// floating number, how many bytes it takes (zero when there is none), and
/// whether glibc reports it as a range error: an overflow, an underflow to
/// zero, or a subnormal result that is not exact. `spelling` is scratch
/// space for the significand and exponent.
fn scan(
    text: &[u8],
    spelling: &mut crate::util::bump::ArenaVec<'_, u8>,
    arena: &crate::util::bump::Bump,
) -> (Binary128, usize, bool) {
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
        return (Binary128::infinity(negative), index + 8, false);
    }
    if starts_with(b"inf") {
        return (Binary128::infinity(negative), index + 3, false);
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
        return (
            Binary128::nan(true, u128::from(payload)).with_sign(negative),
            end,
            false,
        );
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
        return (Binary128::zero(false), 0, false);
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
    let (value, out_of_range) = match crate::binary128::parse(spelling, arena) {
        | Ok(value) => (value, false),
        | Err(ParseFloatError::OutOfRange(FloatTokenType::Float128(value), _)) => (value, true),
        | Err(_) => (Binary128::zero(false), false),
    };
    let subnormal = matches!(value.class(), Class::Finite { exponent, significand }
        if exponent == SUBNORMAL_EXPONENT && significand >> 112 == 0);
    let inexact = subnormal && !exact_hexadecimal(spelling.as_bytes());
    (value.with_sign(negative), end, out_of_range || inexact)
}

/// The exponent of a binary128 subnormal's lowest significand bit,
/// `1 - 16383 - 112`.
const SUBNORMAL_EXPONENT: i32 = -16494;

/// Whether the spelling `0x<digits>[.<digits>]p<exponent>q` names a value
/// whose lowest set bit is at least the subnormal unit, so that a subnormal
/// result holds it exactly; a decimal spelling of a subnormal never does
/// (its exact value has thousands of digits).
fn exact_hexadecimal(spelling: &[u8]) -> bool {
    let Some(digits) = spelling.strip_prefix(b"0x") else {
        return false;
    };
    let marker = digits
        .iter()
        .position(|&byte| byte == b'p')
        .unwrap_or(digits.len());
    let (significand, exponent) = digits.split_at(marker);
    let exponent = exponent
        .get(1..)
        .map(|text| text.strip_suffix(b"q").unwrap_or(text))
        .and_then(|text| std::str::from_utf8(text).ok())
        .and_then(|text| text.parse::<i64>().ok())
        .unwrap_or(0);
    let fraction = significand
        .iter()
        .position(|&byte| byte == b'.')
        .map_or(0, |point| significand.len() - point - 1);
    let mut trailing = 0_i64;
    for &byte in significand.iter().rev().filter(|&&byte| byte != b'.') {
        let digit = char::from(byte).to_digit(16).unwrap_or(0);
        if digit != 0 {
            trailing += i64::from(digit.trailing_zeros());
            break;
        }
        trailing += 4;
    }
    let lowest = exponent
        .saturating_sub(4 * i64::try_from(fraction).unwrap_or(i64::MAX))
        .saturating_add(trailing);
    lowest >= i64::from(SUBNORMAL_EXPONENT)
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
