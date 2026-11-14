//! The real floating types and their arithmetic, as x86-64 Linux gives
//! them: `float` is IEEE 754 binary32, `double` binary64, and `long
//! double` the x87 80-bit extended format, stored in 16 bytes. C99 Annex
//! F is what makes the first two IEEE: an implementation that defines
//! `__STDC_IEC_559__`, as bcc does, binds C's floating types and
//! operators to IEC 60559 (IEEE 754).
//!
//! Each value is its encoding ([`Real`]); the machine computes in software,
//! the same on every host: an operation forms its exact result with
//! [`Big`] integers and rounds it once, to the nearest value of the type,
//! ties to even, the rounding mode every program starts in. Each type
//! computes in itself (`FLT_EVAL_METHOD` 0, SSE for float and double, the
//! x87 at full precision for long double). Invalid operations give the
//! default NaN of x86-64, whose sign bit is set, as the hardware does at
//! run time: glibc prints it as `-nan`.
//!
//! C99: §6.2.5 paragraph 10, p. 34; PDF p. 46 (the real floating types).
//! C99: §5.2.4.2.2 paragraph 8, pp. 24-25; PDF pp. 36-37 (evaluation
//! format).
//! C99: §F.2 paragraph 1, p. 444; PDF p. 456 (IEC 60559 formats).
//! C99: §F.3 paragraph 1, pp. 445-447; PDF pp. 457-459 (IEC 60559
//! operations).
//! C99: §F.7.3 paragraph 1, p. 449; PDF p. 461 (round to nearest at
//! startup).
//! C99: §6.3.1.4 paragraphs 1-2, pp. 43-44; PDF pp. 55-56 (conversions
//! between floating and integer types).
//! C99: §6.3.1.5 paragraphs 1-2, p. 44; PDF p. 56 (conversions between
//! floating types).
//! C99: §6.4.4.2 paragraphs 2-4, p. 58; PDF p. 70 (floating constants).

use std::cmp::Ordering;

use super::{
    decimal::{
        self,
        Big,
    },
    integers::Integer,
};

/// A real floating type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Floating {
    /// `float`: IEEE 754 binary32.
    Float,
    /// `double`: IEEE 754 binary64.
    Double,
    /// `long double`: the x87 80-bit extended format.
    LongDouble,
}

impl Floating {
    /// Bits of precision, the leading one included.
    pub(super) const fn precision(self) -> u32 {
        match self {
            | Self::Float => 24,
            | Self::Double => 53,
            | Self::LongDouble => 64,
        }
    }

    /// Width of the exponent field in bits.
    pub(super) const fn exponent_bits(self) -> u32 {
        match self {
            | Self::Float => 8,
            | Self::Double => 11,
            | Self::LongDouble => 15,
        }
    }

    /// The bias added to the exponent in its field.
    pub(super) const fn bias(self) -> i32 {
        (1 << (self.exponent_bits() - 1)) - 1
    }

    /// The exponent of the smallest normal value, 1 - bias.
    const fn minimum_exponent(self) -> i32 {
        1 - self.bias()
    }

    /// Size in bytes on x86-64, padding included; each type is aligned to
    /// its size.
    pub(super) const fn bytes(self) -> usize {
        match self {
            | Self::Float => 4,
            | Self::Double => 8,
            | Self::LongDouble => 16,
        }
    }

    /// The type as C spells it.
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            | Self::Float => "float",
            | Self::Double => "double",
            | Self::LongDouble => "long double",
        }
    }

    /// The format's name.
    pub(super) const fn format_name(self) -> &'static str {
        match self {
            | Self::Float => "IEEE 754 binary32 (single precision)",
            | Self::Double => "IEEE 754 binary64 (double precision)",
            | Self::LongDouble => "the x87 80-bit extended format",
        }
    }

    /// Decimal digits that always survive a round trip through the type
    /// (`FLT_DIG`, `DBL_DIG`, `LDBL_DIG`).
    pub(super) const fn digits(self) -> u32 {
        match self {
            | Self::Float => 6,
            | Self::Double => 15,
            | Self::LongDouble => 18,
        }
    }

    /// Decimal digits that tell every value of the type apart, the most
    /// the shortest round-trip form needs.
    const fn distinct_digits(self) -> usize {
        match self {
            | Self::Float => 9,
            | Self::Double => 17,
            | Self::LongDouble => 21,
        }
    }

    /// The largest finite value, as `%g` would round it.
    pub(super) const fn maximum_text(self) -> &'static str {
        match self {
            | Self::Float => "3.40282e+38",
            | Self::Double => "1.79769e+308",
            | Self::LongDouble => "1.18973e+4932",
        }
    }

    /// The common real type of operands of these types: the wider one
    /// (C99 §6.3.1.8p1).
    pub(super) fn common(self, other: Self) -> Self {
        self.max(other)
    }
}

/// A value of a floating type: the type and its encoding, the low bits of
/// `bits` (32, 64 or 80 of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Real {
    format: Floating,
    bits:   u128,
}

/// A value taken apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decoded {
    /// `significand` × 2^`exponent`, zero included.
    Finite {
        negative:    bool,
        significand: u64,
        exponent:    i32,
    },
    Infinite {
        negative: bool,
    },
    /// Not a number; `payload` holds the fraction bits below the quiet bit,
    /// aligned to bit 63.
    NotANumber {
        negative: bool,
        payload:  u64,
    },
}

/// A value rounded to a type, and whether no rounding was needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rounded {
    pub value: Real,
    pub exact: bool,
}

/// Why a floating value does not convert to an integer type (C99
/// §6.3.1.4p1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unrepresentable {
    /// An infinity or a NaN, which has no integral part.
    NotFinite,
    /// The integral part, written in decimal, is outside the type's range.
    OutOfRange(String),
}

/// The fields of a stored value, for the bit view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    /// The sign bit.
    pub sign:        bool,
    /// The exponent field, most significant bit first.
    pub exponent:    String,
    /// The biased exponent, the field's value.
    pub biased:      u32,
    /// The bias subtracted from it.
    pub bias:        i32,
    /// The x87 format's explicit integer bit; `None` for float and double,
    /// whose leading 1 is implied.
    pub integer_bit: Option<bool>,
    /// The fraction field, most significant bit first.
    pub fraction:    String,
    /// The power of two the significand is scaled by: the biased exponent
    /// minus the bias, or 1 minus the bias below the normal range.
    pub scale:       i32,
    /// The significand's exact value,
    /// `1.6000000000000000888178419700125232338905334472656250` for 0.1: the
    /// leading bit, then the fraction, as a binary fraction.
    pub significand: String,
    /// `zero`, `subnormal`, `normal`, `infinity` or `NaN`.
    pub class:       &'static str,
}

impl Real {
    /// The type of the value.
    #[must_use]
    pub const fn format(self) -> Floating {
        self.format
    }

    /// Zero of `format`, negative when `negative`.
    pub(super) fn zero(format: Floating, negative: bool) -> Self {
        Self::encode(format, negative, 0, 0)
    }

    /// The default NaN x86-64 produces for an invalid operation: the sign
    /// bit set, quiet, no payload.
    pub(super) fn default_nan(format: Floating) -> Self {
        Self::nan(format, true, 0)
    }

    fn infinity(format: Floating, negative: bool) -> Self {
        let sign = u128::from(negative) << (format.storage() - 1);
        let exponent = ((1_u128 << format.exponent_bits()) - 1) << format.fraction_field();
        let integer = if format == Floating::LongDouble {
            1_u128 << 63
        } else {
            0
        };
        Self {
            format,
            bits: sign | exponent | integer,
        }
    }

    fn nan(format: Floating, negative: bool, payload: u64) -> Self {
        let infinity = Self::infinity(format, negative).bits;
        let below = format.quiet_bit();
        let quiet = 1_u128 << below;
        let payload = u128::from(payload >> (64 - below)) & ((1 << below) - 1);
        Self {
            format,
            bits: infinity | quiet | payload,
        }
    }

    /// Take the encoding apart.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "The fields are masked to at most 64 bits, and a biased exponent to 15."
    )]
    pub(super) fn decode(self) -> Decoded {
        let format = self.format;
        let negative = self.bits >> (format.storage() - 1) & 1 == 1;
        let fraction_bits = format.fraction_field();
        let biased = (self.bits >> fraction_bits) & ((1 << format.exponent_bits()) - 1);
        let field = self.bits & ((1 << fraction_bits) - 1);
        let maximum = (1 << format.exponent_bits()) - 1;
        let lowest = format.minimum_exponent() - (format.precision() as i32 - 1);
        if biased == maximum {
            // The x87 format keeps its integer bit set in an infinity and
            // a NaN; only the bits below it tell them apart.
            let fraction = if format == Floating::LongDouble {
                field & ((1 << 63) - 1)
            } else {
                field
            };
            if fraction == 0 {
                return Decoded::Infinite { negative };
            }
            let below = format.quiet_bit();
            let payload = (fraction & ((1 << below) - 1)) << (64 - below);
            return Decoded::NotANumber {
                negative,
                payload: payload as u64,
            };
        }
        let (significand, exponent) = match format {
            | Floating::LongDouble if biased == 0 => (field, lowest),
            | Floating::LongDouble => (field, biased as i32 - format.bias() - 63),
            | _ if biased == 0 => (field, lowest),
            | _ => (
                field | (1 << fraction_bits),
                biased as i32 - format.bias() - fraction_bits as i32,
            ),
        };
        Decoded::Finite {
            negative,
            significand: significand as u64,
            exponent,
        }
    }

    /// Encode `significand` × 2^`exponent`, where `significand` < 2^p and
    /// `exponent` is the one its rounding chose.
    fn encode(format: Floating, negative: bool, significand: u128, exponent: i32) -> Self {
        let sign = u128::from(negative) << (format.storage() - 1);
        let p = format.precision();
        if significand == 0 {
            return Self { format, bits: sign };
        }
        let biased = if significand >> (p - 1) == 1 {
            (exponent + (p as i32 - 1) + format.bias()) as u128
        } else {
            0
        };
        let field = if format == Floating::LongDouble {
            significand
        } else {
            significand & ((1 << (p - 1)) - 1)
        };
        Self {
            format,
            bits: sign | (biased << format.fraction_field()) | field,
        }
    }

    /// The nearest value of `format` to (`magnitude` + `sticky`·ε) ×
    /// 2^`exponent`, ties to even: `sticky` says nonzero bits lie below
    /// `magnitude`, which then holds at least p + 2 bits. A result beyond
    /// the largest finite value rounds to infinity.
    pub(super) fn round(
        format: Floating,
        negative: bool,
        magnitude: &Big,
        exponent: i32,
        sticky: bool,
    ) -> Rounded {
        if magnitude.is_zero() {
            return Rounded {
                value: Self::zero(format, negative),
                exact: !sticky,
            };
        }
        let p = format.precision() as i32;
        let leading = magnitude.bits() as i32 - 1 + exponent;
        let lowest = format.minimum_exponent() - (p - 1);
        let mut quantum = (leading - (p - 1)).max(lowest);
        let shift = quantum - exponent;
        let (mut significand, exact) = if shift <= 0 {
            let whole = magnitude.shl(shift.unsigned_abs()).to_u128().unwrap_or(0);
            (whole, !sticky)
        } else {
            let kept = magnitude.shr(shift.unsigned_abs()).to_u128().unwrap_or(0);
            let below = shift.unsigned_abs() - 1;
            let half = magnitude.bit(below);
            let lower = sticky || magnitude.any_below(below);
            let up = half && (lower || kept & 1 == 1);
            (kept + u128::from(up), !half && !lower)
        };
        if significand >> p != 0 {
            significand >>= 1;
            quantum += 1;
        }
        let maximum = format.bias();
        if significand >> (p - 1) != 0 && quantum + (p - 1) > maximum {
            return Rounded {
                value: Self::infinity(format, negative),
                exact: false,
            };
        }
        Rounded {
            value: Self::encode(format, negative, significand, quantum),
            exact,
        }
    }

    /// The integer `value` converted to `format`: kept when the type holds
    /// it exactly, otherwise the nearest value, ties to even, the choice C
    /// leaves to the implementation and IEC 60559 makes (C99 §6.3.1.4p2,
    /// §F.3).
    pub(super) fn from_integer(value: i128, format: Floating) -> Rounded {
        Self::round(
            format,
            value < 0,
            &Big::from_u128(value.unsigned_abs()),
            0,
            false,
        )
    }

    /// The value converted to the integer type `ty`: the fractional part
    /// discarded, truncating toward zero, provided the integral part fits
    /// (C99 §6.3.1.4p1).
    pub(super) fn to_integer(self, ty: Integer) -> Result<i128, Unrepresentable> {
        let Decoded::Finite {
            negative,
            significand,
            exponent,
        } = self.decode()
        else {
            return Err(Unrepresentable::NotFinite);
        };
        let magnitude = Big::from_u128(u128::from(significand));
        let whole = if exponent >= 0 {
            magnitude.shl(exponent.unsigned_abs())
        } else {
            magnitude.shr(exponent.unsigned_abs())
        };
        let value = whole
            .to_u128()
            .and_then(|whole| i128::try_from(whole).ok())
            .map(|whole| if negative { -whole } else { whole });
        match value {
            | Some(value) if ty.fits(value) => Ok(value),
            | _ => {
                let sign = if negative && !whole.is_zero() {
                    "-"
                } else {
                    ""
                };
                Err(Unrepresentable::OutOfRange(format!(
                    "{sign}{}",
                    whole.decimal()
                )))
            },
        }
    }

    /// The value converted to `format`: unchanged when it widens (C99
    /// §6.3.1.5p1), rounded to the nearest when it narrows, and to an
    /// infinity beyond the narrower range, as IEC 60559 converts (C99
    /// §6.3.1.5p2, §F.3).
    pub(super) fn convert(self, format: Floating) -> Rounded {
        match self.decode() {
            | Decoded::Finite {
                negative,
                significand,
                exponent,
            } => Self::round(
                format,
                negative,
                &Big::from_u128(u128::from(significand)),
                exponent,
                false,
            ),
            | Decoded::Infinite { negative } => Rounded {
                value: Self::infinity(format, negative),
                exact: true,
            },
            | Decoded::NotANumber { negative, payload } => Rounded {
                value: Self::nan(format, negative, payload),
                exact: true,
            },
        }
    }

    /// `self operator other`, both of one type, for `+ - * /`: the exact
    /// result rounded to the type (C99 §F.3).
    #[expect(
        clippy::many_single_char_names,
        reason = "Each operand's sign, significand and exponent, as the formulas write them."
    )]
    pub(super) fn arithmetic(self, operator: &str, other: Self) -> Rounded {
        let format = self.format;
        let invalid = Rounded {
            value: Self::default_nan(format),
            exact: false,
        };
        let (a, b) = (self.decode(), other.decode());
        // A NaN operand gives a NaN; the left one's comes first, as SSE
        // returns its first source operand's.
        if let Decoded::NotANumber { negative, payload } = a {
            return Rounded {
                value: Self::nan(format, negative, payload),
                exact: true,
            };
        }
        if let Decoded::NotANumber { negative, payload } = b {
            return Rounded {
                value: Self::nan(format, negative, payload),
                exact: true,
            };
        }
        let b = if operator == "-" { negated(b) } else { b };
        let exact = |value| Rounded { value, exact: true };
        match (operator, a, b) {
            | ("+" | "-", Decoded::Infinite { negative: x }, Decoded::Infinite { negative: y }) =>
                if x == y {
                    exact(Self::infinity(format, x))
                } else {
                    invalid
                },
            | ("+" | "-", Decoded::Infinite { negative }, _)
            | ("+" | "-", _, Decoded::Infinite { negative }) =>
                exact(Self::infinity(format, negative)),
            | (
                "+" | "-",
                Decoded::Finite {
                    negative: x,
                    significand: m,
                    exponent: e,
                },
                Decoded::Finite {
                    negative: y,
                    significand: n,
                    exponent: f,
                },
            ) => Self::sum(format, (x, m, e), (y, n, f)),
            | ("*", _, _) => {
                let negative = sign(a) != sign(b);
                match (a, b) {
                    | (Decoded::Infinite { .. }, b) if is_zero(b) => invalid,
                    | (a, Decoded::Infinite { .. }) if is_zero(a) => invalid,
                    | (Decoded::Infinite { .. }, _) | (_, Decoded::Infinite { .. }) =>
                        exact(Self::infinity(format, negative)),
                    | (
                        Decoded::Finite {
                            significand: m,
                            exponent: e,
                            ..
                        },
                        Decoded::Finite {
                            significand: n,
                            exponent: f,
                            ..
                        },
                    ) => Self::round(
                        format,
                        negative,
                        &Big::from_u128(u128::from(m)).mul(&Big::from_u128(u128::from(n))),
                        e + f,
                        false,
                    ),
                    | _ => invalid,
                }
            },
            | (_, _, _) => {
                let negative = sign(a) != sign(b);
                match (a, b) {
                    | (Decoded::Infinite { .. }, Decoded::Infinite { .. }) => invalid,
                    | (Decoded::Infinite { .. }, _) => exact(Self::infinity(format, negative)),
                    | (_, Decoded::Infinite { .. }) => exact(Self::zero(format, negative)),
                    | (a, b) if is_zero(a) && is_zero(b) => invalid,
                    | (_, b) if is_zero(b) => exact(Self::infinity(format, negative)),
                    | (a, _) if is_zero(a) => exact(Self::zero(format, negative)),
                    | (
                        Decoded::Finite {
                            significand: m,
                            exponent: e,
                            ..
                        },
                        Decoded::Finite {
                            significand: n,
                            exponent: f,
                            ..
                        },
                    ) => {
                        let (m, n) = (Big::from_u128(u128::from(m)), Big::from_u128(u128::from(n)));
                        let shift = (format.precision() + 3 + n.bits()).saturating_sub(m.bits());
                        let (quotient, remainder) = m.shl(shift).div_rem(&n);
                        Self::round(
                            format,
                            negative,
                            &quotient,
                            e - f - shift as i32,
                            !remainder.is_zero(),
                        )
                    },
                    | _ => invalid,
                }
            },
        }
    }

    /// The rounded sum of two finite values, each its sign, significand
    /// and exponent. An exact zero sum is +0, unless both are -0.
    #[expect(
        clippy::many_single_char_names,
        reason = "Each operand's sign, significand and exponent, as the formulas write them."
    )]
    fn sum(format: Floating, a: (bool, u64, i32), b: (bool, u64, i32)) -> Rounded {
        let (x, m, e) = a;
        let (y, n, f) = b;
        let low = e.min(f);
        let left = Big::from_u128(u128::from(m)).shl((e - low).unsigned_abs());
        let right = Big::from_u128(u128::from(n)).shl((f - low).unsigned_abs());
        if x == y {
            return Self::round(format, x, &left.add(&right), low, false);
        }
        match left.cmp(&right) {
            | Ordering::Greater => Self::round(format, x, &left.sub(&right), low, false),
            | Ordering::Less => Self::round(format, y, &right.sub(&left), low, false),
            | Ordering::Equal => Rounded {
                value: Self::zero(format, false),
                exact: true,
            },
        }
    }

    /// The exact result of `self operator other` for `+ - *`, before any
    /// rounding, as its decimal text; `None` for a division or a
    /// non-finite operand.
    #[expect(
        clippy::many_single_char_names,
        reason = "Each operand's sign, significand and exponent, as the formulas write them."
    )]
    pub(super) fn exact_result(self, operator: &str, other: Self) -> Option<String> {
        let (
            Decoded::Finite {
                negative: x,
                significand: m,
                exponent: e,
            },
            Decoded::Finite {
                negative: y,
                significand: n,
                exponent: f,
            },
        ) = (self.decode(), other.decode())
        else {
            return None;
        };
        let y = if operator == "-" { !y } else { y };
        let (negative, magnitude, exponent) = match operator {
            | "*" => (
                x != y,
                Big::from_u128(u128::from(m)).mul(&Big::from_u128(u128::from(n))),
                e + f,
            ),
            | "+" | "-" => {
                let low = e.min(f);
                let left = Big::from_u128(u128::from(m)).shl((e - low).unsigned_abs());
                let right = Big::from_u128(u128::from(n)).shl((f - low).unsigned_abs());
                if x == y {
                    (x, left.add(&right), low)
                } else if left >= right {
                    (x, left.sub(&right), low)
                } else {
                    (y, right.sub(&left), low)
                }
            },
            | _ => return None,
        };
        let (digits, scale) = big_expansion(&magnitude, exponent);
        let sign = if negative && !magnitude.is_zero() {
            "-"
        } else {
            ""
        };
        Some(format!("{sign}{}", decimal::exact_text(&digits, scale)))
    }

    /// How the two values compare, `None` when either is a NaN, which is
    /// unordered with everything, itself included. -0 equals +0.
    #[expect(
        clippy::many_single_char_names,
        reason = "Each operand's sign, significand and exponent, as the formulas write them."
    )]
    pub(super) fn compare(self, other: Self) -> Option<Ordering> {
        let key = |decoded: Decoded| match decoded {
            | Decoded::NotANumber { .. } => None,
            | Decoded::Infinite { negative } => Some((if negative { -1 } else { 1 }, None)),
            | Decoded::Finite {
                negative,
                significand,
                exponent,
            } if significand != 0 => Some((0, Some((negative, significand, exponent)))),
            | Decoded::Finite { .. } => Some((0, None)),
        };
        let (a, b) = (key(self.decode())?, key(other.decode())?);
        Some(match (a, b) {
            | ((x, _), (y, _)) if x != y => x.cmp(&y),
            | ((_, None), (_, None)) => Ordering::Equal,
            | ((_, Some((negative, ..))), (_, None)) =>
                if negative {
                    Ordering::Less
                } else {
                    Ordering::Greater
                },
            | ((_, None), (_, Some((negative, ..)))) =>
                if negative {
                    Ordering::Greater
                } else {
                    Ordering::Less
                },
            | ((_, Some((x, m, e))), (_, Some((y, n, f)))) => {
                if x != y {
                    return Some(if x { Ordering::Less } else { Ordering::Greater });
                }
                let low = e.min(f);
                let left = Big::from_u128(u128::from(m)).shl((e - low).unsigned_abs());
                let right = Big::from_u128(u128::from(n)).shl((f - low).unsigned_abs());
                let order = left.cmp(&right);
                if x { order.reverse() } else { order }
            },
        })
    }

    /// The value with its sign flipped: what unary minus does, to a zero
    /// and a NaN too (C99 §F.3).
    pub(super) fn negate(self) -> Self {
        Self {
            format: self.format,
            bits:   self.bits ^ (1 << (self.format.storage() - 1)),
        }
    }

    /// Whether the value is +0 or -0.
    #[must_use]
    pub fn is_zero(self) -> bool {
        matches!(self.decode(), Decoded::Finite { significand: 0, .. })
    }

    /// Whether the value is a NaN.
    #[must_use]
    pub fn is_nan(self) -> bool {
        matches!(self.decode(), Decoded::NotANumber { .. })
    }

    /// Whether the value is an infinity.
    pub(super) fn is_infinite(self) -> bool {
        matches!(self.decode(), Decoded::Infinite { .. })
    }

    /// Whether the sign bit is set.
    pub(super) fn is_negative(self) -> bool {
        sign(self.decode())
    }

    /// The exact decimal value:
    /// `0.1000000000000000055511151231257827021181583404541015625`,
    /// `-3`, `inf`, `-nan`.
    #[must_use]
    pub fn exact(self) -> String {
        match self.decode() {
            | Decoded::Finite {
                negative,
                significand,
                exponent,
            } => {
                let (digits, scale) = decimal::expansion(u128::from(significand), exponent);
                let sign = if negative { "-" } else { "" };
                format!("{sign}{}", decimal::exact_text(&digits, scale))
            },
            | decoded => special(decoded, false),
        }
    }

    /// The shortest decimal that converts back to exactly this value, as
    /// the page shows it: `0.1`, `0.30000000000000004`, `3.0`, `1e+100`.
    #[must_use]
    pub fn short(self) -> String {
        let Decoded::Finite {
            negative,
            significand,
            exponent,
        } = self.decode()
        else {
            return special(self.decode(), false);
        };
        let sign = if negative { "-" } else { "" };
        if significand == 0 {
            return format!("{sign}0.0");
        }
        let (digits, scale) = decimal::expansion(u128::from(significand), exponent);
        for count in 1..=self.format.distinct_digits() {
            let text = decimal::scientific(&digits, scale, count - 1, false);
            let Some((mantissa, power)) = text.split_once('e') else {
                continue;
            };
            let power: i32 = power.parse().unwrap_or(0);
            let mantissa: String = mantissa.chars().filter(char::is_ascii_digit).collect();
            let shift = power - (i32::try_from(count).unwrap_or(0) - 1);
            let back = Self::from_decimal(false, &Big::from_decimal(&mantissa), shift, self.format);
            if back.value == self.negate_if(negative) {
                return format!("{sign}{}", positional(&mantissa, power));
            }
        }
        format!("{sign}{}", decimal::exact_text(&digits, scale))
    }

    fn negate_if(self, negative: bool) -> Self {
        if negative { self.negate() } else { self }
    }

    /// The bytes in memory, lowest address first; long double's six
    /// padding bytes after its ten are shown as zero.
    #[must_use]
    pub fn bytes(self) -> Vec<u8> {
        let mut bytes = self.bits.to_le_bytes().to_vec();
        bytes.truncate(self.format.bytes());
        bytes
    }

    /// The fields of the encoding.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "The fields are masked to at most 64 bits, and a biased exponent to 15."
    )]
    pub fn fields(self) -> Fields {
        let format = self.format;
        let fraction_bits = format.fraction_field();
        let biased = ((self.bits >> fraction_bits) & ((1 << format.exponent_bits()) - 1)) as u32;
        let field = self.bits & ((1 << fraction_bits) - 1);
        let (integer_bit, fraction, width) = if format == Floating::LongDouble {
            (Some(field >> 63 == 1), field & ((1 << 63) - 1), 63)
        } else {
            (None, field, fraction_bits)
        };
        let class = match self.decode() {
            | Decoded::Finite { significand: 0, .. } => "zero",
            | Decoded::Finite { .. } if biased == 0 => "subnormal",
            | Decoded::Finite { .. } => "normal",
            | Decoded::Infinite { .. } => "infinity",
            | Decoded::NotANumber { .. } => "NaN",
        };
        let leading = match format {
            | Floating::LongDouble => field,
            | _ if biased == 0 => field,
            | _ => field | (1 << fraction_bits),
        };
        let point = format.precision() - 1;
        let (digits, scale) = decimal::expansion(leading, -(point as i32));
        Fields {
            sign: self.is_negative(),
            scale: if biased == 0 {
                format.minimum_exponent()
            } else {
                biased as i32 - format.bias()
            },
            significand: decimal::exact_text(&digits, scale),
            exponent: format!("{biased:0width$b}", width = format.exponent_bits() as usize),
            biased,
            bias: format.bias(),
            integer_bit,
            fraction: format!("{fraction:0width$b}", width = width as usize),
            class,
        }
    }

    /// The value of a decimal floating constant's digits, `digits` ×
    /// 10^`exponent`, rounded to `format`.
    pub(super) fn from_decimal(
        negative: bool,
        digits: &Big,
        exponent: i32,
        format: Floating,
    ) -> Rounded {
        if digits.is_zero() {
            return Self::round(format, negative, digits, 0, false);
        }
        // Past these, every constant of at most 256 digits is beyond the
        // range of long double, or below half its smallest value.
        if exponent > 5000 {
            return Self::round(format, negative, &Big::from_u128(1), 20000, false);
        }
        if exponent < -5500 {
            return Rounded {
                value: Self::zero(format, negative),
                exact: false,
            };
        }
        if exponent >= 0 {
            let whole = digits.mul(&Big::power(10, exponent.unsigned_abs()));
            return Self::round(format, negative, &whole, 0, false);
        }
        let denominator = Big::power(10, exponent.unsigned_abs());
        let shift = (format.precision() + 3 + denominator.bits()).saturating_sub(digits.bits());
        let (quotient, remainder) = digits.shl(shift).div_rem(&denominator);
        Self::round(
            format,
            negative,
            &quotient,
            -(shift as i32),
            !remainder.is_zero(),
        )
    }

    /// The `%f`, `%e`, `%g` or `%a` text of the value (upper case for
    /// `%F %E %G %A`) with `precision`, sign included: glibc's output,
    /// `inf` and `nan` for the special values (C99 §7.19.6.1p8).
    pub(super) fn printf(self, conversion: u8, precision: Option<usize>) -> String {
        let upper = conversion.is_ascii_uppercase();
        let Decoded::Finite {
            negative,
            significand,
            exponent,
        } = self.decode()
        else {
            return special(self.decode(), upper);
        };
        let sign = if negative { "-" } else { "" };
        let body = if conversion.eq_ignore_ascii_case(&b'a') {
            self.hexadecimal_text(significand, exponent, precision, upper)
        } else {
            let (digits, scale) = decimal::expansion(u128::from(significand), exponent);
            let precision = precision.unwrap_or(6);
            match conversion.to_ascii_lowercase() {
                | b'f' => decimal::fixed(&digits, scale, precision),
                | b'e' => decimal::scientific(&digits, scale, precision, upper),
                | _ => decimal::general(&digits, scale, precision, upper),
            }
        };
        format!("{sign}{body}")
    }

    /// `%a` of a finite value as glibc writes it: a double with its
    /// leading 1 (0 below the normal range) and 13 hexadecimal digits, a
    /// long double with the four leading bits of its 64-bit significand
    /// and 15 digits after the point.
    fn hexadecimal_text(
        self,
        significand: u64,
        exponent: i32,
        precision: Option<usize>,
        upper: bool,
    ) -> String {
        if significand == 0 {
            return decimal::hexadecimal(0, 4, 0, precision, false, upper);
        }
        match self.format {
            | Floating::LongDouble =>
                decimal::hexadecimal(significand, 60, exponent + 60, precision, true, upper),
            | _ => {
                // Float arguments arrive as doubles.
                let Decoded::Finite {
                    significand,
                    exponent,
                    ..
                } = self.convert(Floating::Double).value.decode()
                else {
                    return String::new();
                };
                let normal = significand >> 52 == 1;
                let shown = if normal { exponent + 52 } else { -1022 };
                decimal::hexadecimal(significand, 52, shown, precision, false, upper)
            },
        }
    }

    /// Parse the floating constant `text` (C99 §6.4.4.2p1): decimal digits
    /// with a period or an exponent, or a hexadecimal significand with a
    /// binary exponent, and an optional `f` or `l` suffix. Returns its
    /// value and whether the type holds it exactly, or what is wrong.
    pub(super) fn parse_constant(text: &str) -> Result<Rounded, ConstantError> {
        let (body, format) = match text.as_bytes().last() {
            | Some(b'f' | b'F') if !is_hexadecimal(text) || has_binary_exponent(text) =>
                (&text[..text.len() - 1], Floating::Float),
            | Some(b'l' | b'L') => (&text[..text.len() - 1], Floating::LongDouble),
            | _ => (text, Floating::Double),
        };
        if body.len() > 256 {
            return Err(ConstantError::TooLong);
        }
        let rounded = if is_hexadecimal(body) {
            parse_hexadecimal(&body[2..], format)?
        } else {
            parse_decimal(body, format)?
        };
        if rounded.value.is_infinite() {
            return Err(ConstantError::OutOfRange(format));
        }
        Ok(rounded)
    }
}

/// What is wrong with a floating constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConstantError {
    /// Not a floating constant's spelling.
    Malformed,
    /// Longer than the machine reads.
    TooLong,
    /// Beyond the largest finite value of its type (C99 §6.4.4p2).
    OutOfRange(Floating),
}

impl Floating {
    /// Bits in the encoding: 32, 64 or 80.
    const fn storage(self) -> u32 {
        match self {
            | Self::Float => 32,
            | Self::Double => 64,
            | Self::LongDouble => 80,
        }
    }

    /// The fraction bit that makes a NaN quiet: the highest, below the x87
    /// format's integer bit.
    const fn quiet_bit(self) -> u32 {
        match self {
            | Self::Float => 22,
            | Self::Double => 51,
            | Self::LongDouble => 62,
        }
    }

    /// Bits below the exponent field: the fraction, and the x87 format's
    /// explicit integer bit.
    const fn fraction_field(self) -> u32 {
        match self {
            | Self::Float => 23,
            | Self::Double => 52,
            | Self::LongDouble => 64,
        }
    }
}

const fn sign(decoded: Decoded) -> bool {
    match decoded {
        | Decoded::Finite { negative, .. }
        | Decoded::Infinite { negative }
        | Decoded::NotANumber { negative, .. } => negative,
    }
}

const fn is_zero(decoded: Decoded) -> bool {
    matches!(decoded, Decoded::Finite { significand: 0, .. })
}

const fn negated(decoded: Decoded) -> Decoded {
    match decoded {
        | Decoded::Finite {
            negative,
            significand,
            exponent,
        } => Decoded::Finite {
            negative: !negative,
            significand,
            exponent,
        },
        | Decoded::Infinite { negative } => Decoded::Infinite {
            negative: !negative,
        },
        | Decoded::NotANumber { negative, payload } => Decoded::NotANumber {
            negative: !negative,
            payload,
        },
    }
}

/// `inf`, `-inf`, `nan` or `-nan`, as glibc's printf writes them.
fn special(decoded: Decoded, upper: bool) -> String {
    let (negative, word) = match decoded {
        | Decoded::Infinite { negative } => (negative, "inf"),
        | Decoded::NotANumber { negative, .. } => (negative, "nan"),
        | Decoded::Finite { .. } => (false, ""),
    };
    let sign = if negative { "-" } else { "" };
    let word = if upper {
        word.to_ascii_uppercase()
    } else {
        String::from(word)
    };
    format!("{sign}{word}")
}

/// The decimal expansion of `magnitude` × 2^`exponent`.
fn big_expansion(magnitude: &Big, exponent: i32) -> (String, usize) {
    if exponent >= 0 {
        return (magnitude.shl(exponent.unsigned_abs()).decimal(), 0);
    }
    let k = exponent.unsigned_abs();
    (magnitude.mul(&Big::power(5, k)).decimal(), k as usize)
}

/// The significant `digits` with decimal exponent `power`, positional when
/// the exponent is modest (`0.1`, `3.0`, `120.5`), scientific otherwise
/// (`1e+100`, `2.5e-07`).
fn positional(digits: &str, power: i32) -> String {
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if (-5..21).contains(&power) {
        if power < 0 {
            return format!(
                "0.{}{digits}",
                "0".repeat((-power - 1).unsigned_abs() as usize)
            );
        }
        if power + 1 >= count {
            let zeros = (power + 1 - count).unsigned_abs() as usize;
            return format!("{digits}{}.0", "0".repeat(zeros));
        }
        let (whole, fraction) = digits.split_at((power + 1).unsigned_abs() as usize);
        return format!("{whole}.{fraction}");
    }
    let (first, rest) = digits.split_at(1);
    let point = if rest.is_empty() { "" } else { "." };
    let sign = if power < 0 { '-' } else { '+' };
    format!("{first}{point}{rest}e{sign}{:02}", power.unsigned_abs())
}

fn is_hexadecimal(text: &str) -> bool {
    text.starts_with("0x") || text.starts_with("0X")
}

fn has_binary_exponent(text: &str) -> bool {
    text.contains(['p', 'P'])
}

/// `digits.digits` with an optional `e±digits`, rounded to `format`.
fn parse_decimal(body: &str, format: Floating) -> Result<Rounded, ConstantError> {
    let (significand, exponent) = match body.find(['e', 'E']) {
        | Some(at) => (&body[..at], Some(&body[at + 1..])),
        | None => (body, None),
    };
    let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole)
        || !digits(fraction)
        || (whole.is_empty() && fraction.is_empty())
        || (exponent.is_none() && !significand.contains('.'))
    {
        return Err(ConstantError::Malformed);
    }
    let scale = match exponent {
        | Some(exponent) => parse_exponent(exponent)?,
        | None => 0,
    };
    let all = format!("{whole}{fraction}");
    let shift = scale - i32::try_from(fraction.len()).unwrap_or(i32::MAX);
    Ok(Real::from_decimal(
        false,
        &Big::from_decimal(&all),
        shift,
        format,
    ))
}

/// The hexadecimal significand and binary exponent after `0x`.
fn parse_hexadecimal(body: &str, format: Floating) -> Result<Rounded, ConstantError> {
    let Some(at) = body.find(['p', 'P']) else {
        return Err(ConstantError::Malformed);
    };
    let (significand, exponent) = (&body[..at], &body[at + 1..]);
    let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !digits(whole) || !digits(fraction) || (whole.is_empty() && fraction.is_empty()) {
        return Err(ConstantError::Malformed);
    }
    let mut magnitude = Big::default();
    for digit in whole.bytes().chain(fraction.bytes()) {
        let value = char::from(digit).to_digit(16).unwrap_or(0);
        magnitude = magnitude.shl(4).add(&Big::from_u128(u128::from(value)));
    }
    let exponent =
        parse_exponent(exponent)? - 4 * i32::try_from(fraction.len()).unwrap_or(i32::MAX);
    if magnitude.is_zero() {
        return Ok(Real::round(format, false, &magnitude, 0, false));
    }
    let leading = i64::from(magnitude.bits()) + i64::from(exponent);
    if leading > 20000 {
        return Ok(Real::round(format, false, &Big::from_u128(1), 20000, false));
    }
    if leading < -20000 {
        return Ok(Rounded {
            value: Real::zero(format, false),
            exact: false,
        });
    }
    Ok(Real::round(format, false, &magnitude, exponent, false))
}

/// An exponent's optionally signed decimal digits, clamped far beyond any
/// type's range.
fn parse_exponent(text: &str) -> Result<i32, ConstantError> {
    let (negative, digits) = match text.as_bytes().first() {
        | Some(b'-') => (true, &text[1..]),
        | Some(b'+') => (false, &text[1..]),
        | _ => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ConstantError::Malformed);
    }
    let value = digits
        .trim_start_matches('0')
        .parse::<i32>()
        .unwrap_or(if digits.trim_start_matches('0').is_empty() {
            0
        } else {
            1_000_000
        })
        .min(1_000_000);
    Ok(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn double(value: f64) -> Real {
        Real {
            format: Floating::Double,
            bits:   u128::from(value.to_bits()),
        }
    }

    fn float(value: f32) -> Real {
        Real {
            format: Floating::Float,
            bits:   u128::from(value.to_bits()),
        }
    }

    fn host(real: Real) -> f64 {
        f64::from_bits(u64::try_from(real.convert(Floating::Double).value.bits).unwrap_or(0))
    }

    /// A deterministic spread of doubles: small and large, integral and
    /// not, subnormal, and the ends of the range.
    fn samples() -> Vec<f64> {
        let mut values = vec![
            0.0,
            -0.0,
            0.1,
            0.2,
            0.3,
            1.0,
            -1.5,
            3.0,
            1e-310,
            5e-324,
            f64::MAX,
            f64::MIN_POSITIVE,
            1e300,
            123_456.789,
            -2.5e-8,
            9_007_199_254_740_993.0,
        ];
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        for _ in 0..400 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let candidate = f64::from_bits(state);
            if candidate.is_finite() {
                values.push(candidate);
            }
            values.push(f64::from(u32::try_from(state >> 40).unwrap_or(0)) / 1024.0 - 8000.0);
        }
        values
    }

    #[test]
    fn double_arithmetic_matches_the_host() {
        let values = samples();
        for (index, &a) in values.iter().enumerate() {
            let b = values[(index * 7 + 3) % values.len()];
            for (operator, expected) in [("+", a + b), ("-", a - b), ("*", a * b), ("/", a / b)] {
                let got = double(a).arithmetic(operator, double(b)).value;
                if expected.is_nan() {
                    assert!(got.is_nan(), "{a} {operator} {b}");
                } else {
                    assert_eq!(
                        got.bits,
                        u128::from(expected.to_bits()),
                        "{a:e} {operator} {b:e}"
                    );
                }
            }
            assert_eq!(double(a).compare(double(b)), a.partial_cmp(&b));
        }
    }

    #[test]
    fn float_arithmetic_and_conversions_match_the_host() {
        for &a in &samples() {
            #[expect(clippy::cast_possible_truncation, reason = "The host's own rounding.")]
            let narrow = a as f32;
            let converted = double(a).convert(Floating::Float).value;
            assert_eq!(converted.bits, u128::from(narrow.to_bits()), "{a:e}");
            assert_eq!(host(converted), f64::from(narrow));
            let other = narrow * 0.75 + 1.0;
            assert_eq!(
                float(narrow).arithmetic("*", float(0.75)).value.bits,
                u128::from((narrow * 0.75).to_bits())
            );
            assert_eq!(
                float(narrow * 0.75).arithmetic("+", float(1.0)).value.bits,
                u128::from(other.to_bits())
            );
        }
    }

    #[test]
    fn constants_round_to_the_nearest() {
        let tenth = Real::parse_constant("0.1").unwrap();
        assert_eq!(tenth.value, double(0.1));
        assert!(!tenth.exact);
        assert!(Real::parse_constant("0.5").unwrap().exact);
        assert_eq!(
            Real::parse_constant("1e-310").unwrap().value,
            double(1e-310)
        );
        assert_eq!(Real::parse_constant(".5e1").unwrap().value, double(5.0));
        assert_eq!(Real::parse_constant("2.").unwrap().value, double(2.0));
        assert_eq!(Real::parse_constant("0x1.8p1").unwrap().value, double(3.0));
        assert_eq!(
            Real::parse_constant("0x1p-1074").unwrap().value,
            double(5e-324)
        );
        assert_eq!(Real::parse_constant("0.1f").unwrap().value, float(0.1));
        assert_eq!(
            Real::parse_constant("1e400"),
            Err(ConstantError::OutOfRange(Floating::Double))
        );
        assert_eq!(
            Real::parse_constant("1e400L").unwrap().value.format(),
            Floating::LongDouble
        );
        assert_eq!(Real::parse_constant("1e"), Err(ConstantError::Malformed));
        assert_eq!(Real::parse_constant("0x1.8"), Err(ConstantError::Malformed));
        for &value in &samples() {
            let text = format!("{:e}", value.abs());
            assert_eq!(
                Real::parse_constant(&text).unwrap().value,
                double(value.abs()),
                "{text}"
            );
        }
    }

    #[test]
    fn integers_convert_both_ways() {
        let big = Real::from_integer(9_007_199_254_740_993, Floating::Double);
        assert_eq!(big.value, double(9_007_199_254_740_992.0));
        assert!(!big.exact);
        assert_eq!(double(2.99).to_integer(Integer::Int), Ok(2));
        assert_eq!(double(-2.99).to_integer(Integer::Int), Ok(-2));
        assert_eq!(double(-0.5).to_integer(Integer::UnsignedInt), Ok(0));
        assert_eq!(
            double(3e9).to_integer(Integer::Int),
            Err(Unrepresentable::OutOfRange(String::from("3000000000")))
        );
        assert_eq!(
            double(f64::NAN).to_integer(Integer::Int),
            Err(Unrepresentable::NotFinite)
        );
    }

    #[test]
    fn long_double_has_sixty_four_bits_of_precision() {
        let tenth = Real::parse_constant("0.1L").unwrap().value;
        assert_eq!(tenth.bits, 0x3FFB_CCCC_CCCC_CCCC_CCCD);
        assert_eq!(tenth.printf(b'a', None), "0xc.ccccccccccccccdp-7");
        assert_eq!(tenth.printf(b'f', Some(25)), "0.1000000000000000000013553");
        let one = Real::from_integer(1, Floating::LongDouble).value;
        assert_eq!(one.printf(b'a', None), "0x8p-3");
        let third = one.arithmetic("/", Real::from_integer(3, Floating::LongDouble).value);
        assert_eq!(third.value.bits, 0x3FFD_AAAA_AAAA_AAAA_AAAB);
        assert_eq!(
            Real::default_nan(Floating::LongDouble).printf(b'f', None),
            "-nan"
        );
    }

    #[test]
    fn printf_matches_glibc() {
        assert_eq!(double(0.1).printf(b'f', None), "0.100000");
        assert_eq!(double(0.1).printf(b'g', Some(17)), "0.10000000000000001");
        assert_eq!(double(0.1).printf(b'a', None), "0x1.999999999999ap-4");
        assert_eq!(double(0.0).printf(b'a', None), "0x0p+0");
        assert_eq!(double(5e-324).printf(b'a', None), "0x0.0000000000001p-1022");
        assert_eq!(double(-0.0).printf(b'g', None), "-0");
        assert_eq!(double(1e-10).printf(b'G', None), "1E-10");
        assert_eq!(double(f64::INFINITY).printf(b'F', None), "INF");
        assert_eq!(float(0.1).printf(b'a', None), "0x1.99999ap-4");
        assert_eq!(double(1.0005).printf(b'f', Some(3)), "1.000");
        assert_eq!(double(2.0005).printf(b'f', Some(3)), "2.001");
        assert_eq!(double(1.125).printf(b'e', Some(2)), "1.12e+00");
    }

    #[test]
    fn short_forms_round_trip() {
        assert_eq!(double(0.1).short(), "0.1");
        assert_eq!(double(0.1 + 0.2).short(), "0.30000000000000004");
        assert_eq!(double(3.0).short(), "3.0");
        assert_eq!(double(-0.0).short(), "-0.0");
        assert_eq!(double(1e100).short(), "1e+100");
        assert_eq!(double(2.5e-7).short(), "2.5e-07");
        assert_eq!(float(0.1).short(), "0.1");
        assert_eq!(double(f64::from(0.1_f32)).short(), "0.10000000149011612");
        assert_eq!(Real::default_nan(Floating::Double).short(), "-nan");
        assert_eq!(
            double(0.1).exact(),
            "0.1000000000000000055511151231257827021181583404541015625"
        );
    }

    #[test]
    fn fields_split_the_encoding() {
        let fields = double(0.1).fields();
        assert!(!fields.sign);
        assert_eq!(fields.exponent, "01111111011");
        assert_eq!(fields.biased, 1019);
        assert_eq!(fields.fraction.len(), 52);
        assert_eq!(fields.class, "normal");
        let long = Real::from_integer(1, Floating::LongDouble).value.fields();
        assert_eq!(long.integer_bit, Some(true));
        assert_eq!(long.fraction.len(), 63);
        assert_eq!(double(0.1).bytes(), 0.1_f64.to_le_bytes());
    }
}
