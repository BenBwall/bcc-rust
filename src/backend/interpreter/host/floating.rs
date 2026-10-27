//! The text of `printf`'s floating conversions `e E f F g G a A` for
//! `double` and x87 `long double`, digit for digit as glibc prints them.
//!
//! A finite value is `significand × 2^exponent`, so its decimal expansion is
//! finite: the integer part is a big integer, and the fraction `F / 2^shift`
//! yields one digit per multiplication by ten. Rounding to the requested
//! digits looks at the next digit and at whether anything nonzero remains,
//! and rounds halfway cases to even, as glibc does in the default rounding
//! mode. Hexadecimal output takes the digits straight from the
//! representation: `double` normalizes to a leading `1` (or `0` for
//! denormals), while glibc prints the x87 significand's top four bits as the
//! leading digit (`0x8p-3` for 1.0).
//!
//! C99: §7.19.6.1 paragraph 8, pp. 278-279; PDF pp. 290-291 (the `f`, `e`,
//! `g` and `a` conversions); paragraph 13, p. 280; PDF p. 292 (exact
//! conversion when `DECIMAL_DIG` digits suffice, a quality glibc extends to
//! all digits).

use crate::{
    util::bump::ArenaVec,
    x87::{
        Class,
        X87,
    },
};

/// A floating `printf` argument.
#[derive(Clone, Copy, Debug)]
pub(super) enum Floating {
    Double(f64),
    Extended(X87),
}

/// How a conversion shows the sign of a nonnegative value.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum PositiveSign {
    #[default]
    None,
    /// The `+` flag.
    Plus,
    /// The space flag.
    Space,
}

/// A value decoded for formatting.
#[derive(Clone, Copy)]
enum Decoded {
    /// `significand × 2^exponent`; zero has a zero significand.
    Finite {
        significand: u64,
        exponent:    i32,
    },
    Infinite,
    Nan,
}

impl Floating {
    fn decode(self) -> (bool, Decoded) {
        match self {
            | Self::Double(value) => {
                let bits = value.to_bits();
                let biased = i32::try_from((bits >> 52) & 0x7FF).expect("eleven bits");
                let fraction = bits & ((1 << 52) - 1);
                let decoded = match biased {
                    | 0x7FF if fraction == 0 => Decoded::Infinite,
                    | 0x7FF => Decoded::Nan,
                    | 0 => Decoded::Finite {
                        significand: fraction,
                        exponent:    -1074,
                    },
                    | _ => Decoded::Finite {
                        significand: fraction | 1 << 52,
                        exponent:    biased - 1075,
                    },
                };
                (value.is_sign_negative(), decoded)
            },
            | Self::Extended(value) => {
                let decoded = match value.class() {
                    | Class::Zero => Decoded::Finite {
                        significand: 0,
                        exponent:    0,
                    },
                    | Class::Finite {
                        exponent,
                        significand,
                    } => Decoded::Finite {
                        significand,
                        exponent,
                    },
                    | Class::Infinite => Decoded::Infinite,
                    // glibc's `isnanl` counts unnormals and pseudo-NaNs as
                    // NaNs.
                    | Class::Nan { .. } | Class::Unsupported => Decoded::Nan,
                };
                (value.is_negative(), decoded)
            },
        }
    }
}

/// Scratch space for one conversion: the integer part's digits and the
/// significant digits an `e` or `g` conversion keeps.
pub(super) struct Scratch<'a> {
    pub(super) integer:     ArenaVec<'a, u8>,
    pub(super) significant: ArenaVec<'a, u8>,
}

/// Appends the text of `value` under `conversion` (one of `eEfFgGaA`) to
/// `out`, with `precision` digits (`None` for the default) and the `#` flag
/// as `alternate`, and says whether the value was finite.
pub(super) fn format(
    value: Floating,
    conversion: u8,
    precision: Option<usize>,
    alternate: bool,
    sign: PositiveSign,
    out: &mut ArenaVec<'_, u8>,
    scratch: &mut Scratch<'_>,
) -> bool {
    let (negative, decoded) = value.decode();
    let upper = conversion.is_ascii_uppercase();
    if negative {
        out.push(b'-');
    } else if sign == PositiveSign::Plus {
        out.push(b'+');
    } else if sign == PositiveSign::Space {
        out.push(b' ');
    }
    let (significand, exponent) = match decoded {
        | Decoded::Finite {
            significand,
            exponent,
        } => (significand, exponent),
        | Decoded::Infinite => {
            out.extend_from_slice(if upper { b"INF" } else { b"inf" });
            return false;
        },
        | Decoded::Nan => {
            out.extend_from_slice(if upper { b"NAN" } else { b"nan" });
            return false;
        },
    };
    let Scratch {
        integer,
        significant,
    } = scratch;
    match conversion.to_ascii_lowercase() {
        | b'a' => hexadecimal(value, precision, alternate, upper, out),
        | b'f' => {
            let mut expansion = Expansion::new(significand, exponent, integer);
            fixed(&mut expansion, precision.unwrap_or(6), alternate, out);
        },
        | b'e' => {
            let mut expansion = Expansion::new(significand, exponent, integer);
            let power = expansion.significant(precision.unwrap_or(6) + 1, significant);
            scientific(significant, power, alternate, upper, out);
        },
        | _ => {
            // C99 §7.19.6.1p8: `g` chooses `e` style when the exponent is
            // below -4 or at least the precision, and removes trailing zeros
            // unless `#` is given.
            let precision = precision.unwrap_or(6).max(1);
            let mut expansion = Expansion::new(significand, exponent, integer);
            let power = expansion.significant(precision, significant);
            if !alternate {
                let kept = significant.len()
                    - significant[1..]
                        .iter()
                        .rev()
                        .take_while(|&&digit| digit == b'0')
                        .count();
                significant.truncate(kept);
            }
            if power < -4 || power >= i64::try_from(precision).unwrap_or(i64::MAX) {
                scientific(significant, power, alternate, upper, out);
            } else {
                positional(significant, power, alternate, out);
            }
        },
    }
    true
}

/// `d.ddde±xx`: `digits` are the significant digits, `power` the decimal
/// exponent of the first.
fn scientific(digits: &[u8], power: i64, alternate: bool, upper: bool, out: &mut ArenaVec<'_, u8>) {
    out.push(digits[0]);
    if digits.len() > 1 || alternate {
        out.push(b'.');
    }
    out.extend_from_slice(&digits[1..]);
    out.push(if upper { b'E' } else { b'e' });
    out.push(if power < 0 { b'-' } else { b'+' });
    let magnitude = power.unsigned_abs();
    if magnitude < 10 {
        out.push(b'0');
    }
    push_decimal(out, magnitude);
}

/// The `f` style of a `g` conversion: significant `digits` whose first has
/// decimal exponent `power`.
fn positional(digits: &[u8], power: i64, alternate: bool, out: &mut ArenaVec<'_, u8>) {
    let count = |n: i64| usize::try_from(n).unwrap_or(0);
    if power < 0 {
        out.extend_from_slice(b"0.");
        out.extend(std::iter::repeat_n(b'0', count(-power - 1)));
        out.extend_from_slice(digits);
        return;
    }
    let integer = count(power + 1);
    for index in 0..integer {
        out.push(digits.get(index).copied().unwrap_or(b'0'));
    }
    let fraction = digits.get(integer..).unwrap_or_default();
    if !fraction.is_empty() || alternate {
        out.push(b'.');
    }
    out.extend_from_slice(fraction);
}

/// The `f` conversion: all integer digits and `precision` fraction digits,
/// rounded.
fn fixed(
    expansion: &mut Expansion<'_, '_>,
    precision: usize,
    alternate: bool,
    out: &mut ArenaVec<'_, u8>,
) {
    let start = out.len();
    if expansion.integer.is_empty() {
        out.push(b'0');
    } else {
        out.extend_from_slice(expansion.integer);
    }
    for _ in 0..precision {
        let digit = expansion.next_fraction_digit();
        out.push(b'0' + digit);
    }
    let next = expansion.next_fraction_digit();
    let rest = !expansion.fraction_is_zero();
    let last = out[out.len() - 1] - b'0';
    if round_up(last, next, rest) && increment(&mut out[start..]) {
        out.insert(start, b'1');
    }
    if precision > 0 || alternate {
        out.insert(out.len() - precision, b'.');
    }
}

/// Whether a kept last digit `last` rounds up given the `next` digit and
/// whether any nonzero digit follows it: to nearest, ties to even.
const fn round_up(last: u8, next: u8, rest: bool) -> bool {
    next > 5 || (next == 5 && (rest || last % 2 == 1))
}

/// Adds one to the decimal digits, returning whether a carry left the top.
fn increment(digits: &mut [u8]) -> bool {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            return false;
        }
    }
    true
}

fn push_decimal(out: &mut ArenaVec<'_, u8>, value: u64) {
    let mut buffer = [0_u8; 20];
    let mut position = buffer.len();
    let mut value = value;
    loop {
        position -= 1;
        buffer[position] = b'0' + u8::try_from(value % 10).expect("a digit");
        value /= 10;
        if value == 0 {
            break;
        }
    }
    out.extend_from_slice(&buffer[position..]);
}

/// The exact decimal expansion of `significand × 2^exponent`.
struct Expansion<'d, 'a> {
    /// The integer part's digits, most significant first, without leading
    /// zeros: empty for a value below one.
    integer:  &'d ArenaVec<'a, u8>,
    /// The fraction's numerator; the denominator is `2^shift`.
    fraction: Natural,
    shift:    u32,
}

impl<'d, 'a> Expansion<'d, 'a> {
    fn new(significand: u64, exponent: i32, digits: &'d mut ArenaVec<'a, u8>) -> Self {
        digits.clear();
        let mut integer = Natural::default();
        let mut fraction = Natural::default();
        let shift = if exponent >= 0 {
            integer.set_shifted(significand, exponent.unsigned_abs());
            0
        } else {
            let shift = exponent.unsigned_abs();
            if shift < 64 {
                integer.set_shifted(significand >> shift, 0);
                fraction.set_shifted(significand & ((1 << shift) - 1), 0);
            } else {
                fraction.set_shifted(significand, 0);
            }
            shift
        };
        integer.write_decimal(digits);
        Self {
            integer: digits,
            fraction,
            shift,
        }
    }

    /// The next digit of the fraction: zero once it is exhausted.
    fn next_fraction_digit(&mut self) -> u8 {
        if self.fraction.is_zero() {
            return 0;
        }
        self.fraction.multiply_small(10);
        self.fraction.take_above(self.shift)
    }

    fn fraction_is_zero(&self) -> bool {
        self.fraction.is_zero()
    }

    /// Rounds the value to `count` significant digits, written to `kept`,
    /// and returns the decimal exponent of the first; zero has exponent
    /// zero.
    fn significant(&mut self, count: usize, kept: &mut ArenaVec<'_, u8>) -> i64 {
        kept.clear();
        let integer = self.integer;
        let mut power = i64::try_from(integer.len()).expect("a few thousand digits") - 1;
        if integer.is_empty() {
            if self.fraction_is_zero() {
                kept.extend(std::iter::repeat_n(b'0', count));
                return 0;
            }
            // Skip the zeros before the first significant fraction digit.
            loop {
                let digit = self.next_fraction_digit();
                if digit != 0 {
                    kept.push(b'0' + digit);
                    break;
                }
                power -= 1;
            }
        }
        let mut from_integer = integer.iter().copied();
        let mut next_digit = |this: &mut Self| {
            from_integer
                .next()
                .map_or_else(|| this.next_fraction_digit(), |digit| digit - b'0')
        };
        while kept.len() < count {
            kept.push(b'0' + next_digit(self));
        }
        let next = next_digit(self);
        let rest = integer
            .get(count + 1..)
            .is_some_and(|rest| rest.iter().any(|&digit| digit != b'0'))
            || !self.fraction_is_zero();
        let last = kept[count - 1] - b'0';
        if round_up(last, next, rest) && increment(kept) {
            kept[0] = b'1';
            power += 1;
        }
        power
    }
}

/// A natural number of up to `LIMBS` 32-bit limbs, least significant first:
/// enough for the integer part of the largest x87 value (2^16384) and for
/// the fraction of the smallest (2^-16445) times ten.
#[derive(Clone)]
struct Natural {
    limbs: [u32; LIMBS],
    len:   usize,
}

const LIMBS: usize = 520;

impl Default for Natural {
    fn default() -> Self {
        Self {
            limbs: [0; LIMBS],
            len:   0,
        }
    }
}

impl Natural {
    fn set_shifted(&mut self, value: u64, shift: u32) {
        self.limbs = [0; LIMBS];
        let word = (shift / 32) as usize;
        let bit = shift % 32;
        let wide = u128::from(value) << bit;
        for index in 0..4 {
            let limb = u32::try_from((wide >> (32 * index)) & 0xFFFF_FFFF).expect("masked");
            if let Some(slot) = self.limbs.get_mut(word + index) {
                *slot = limb;
            }
        }
        self.len = (word + 4).min(LIMBS);
        self.trim();
    }

    fn trim(&mut self) {
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
    }

    fn is_zero(&self) -> bool {
        self.len == 0
    }

    fn multiply_small(&mut self, factor: u32) {
        let mut carry = 0_u64;
        for limb in &mut self.limbs[..self.len] {
            let product = u64::from(*limb) * u64::from(factor) + carry;
            *limb = u32::try_from(product & 0xFFFF_FFFF).expect("masked");
            carry = product >> 32;
        }
        if carry != 0 && self.len < LIMBS {
            self.limbs[self.len] = u32::try_from(carry).expect("below 2^32");
            self.len += 1;
        }
    }

    /// Removes and returns the bits at and above `shift`, which hold less
    /// than ten.
    fn take_above(&mut self, shift: u32) -> u8 {
        let word = (shift / 32) as usize;
        let bit = shift % 32;
        if word >= self.len {
            return 0;
        }
        // Below ten fits in the two limbs from `word` up.
        let mut above = 0_u64;
        for &limb in self.limbs[word..self.len].iter().rev() {
            above = (above << 32) | u64::from(limb);
        }
        let digit = u8::try_from(above >> bit).expect("a decimal digit");
        self.limbs[word] &= (1_u32 << bit) - 1;
        self.limbs[word + 1..self.len].fill(0);
        self.len = word + 1;
        self.trim();
        digit
    }

    /// Appends the decimal digits, most significant first; nothing for zero.
    fn write_decimal(&self, out: &mut ArenaVec<'_, u8>) {
        let mut value = self.clone();
        let start = out.len();
        while !value.is_zero() {
            // Nine digits at a time, least significant chunk first.
            let mut remainder = 0_u64;
            for limb in value.limbs[..value.len].iter_mut().rev() {
                let current = (remainder << 32) | u64::from(*limb);
                *limb = u32::try_from(current / 1_000_000_000).expect("a quotient limb");
                remainder = current % 1_000_000_000;
            }
            value.trim();
            for _ in 0..9 {
                out.push(b'0' + u8::try_from(remainder % 10).expect("a digit"));
                remainder /= 10;
                if value.is_zero() && remainder == 0 {
                    break;
                }
            }
        }
        out[start..].reverse();
    }
}

/// The `a` conversion.
fn hexadecimal(
    value: Floating,
    precision: Option<usize>,
    alternate: bool,
    upper: bool,
    out: &mut ArenaVec<'_, u8>,
) {
    // The leading digit, the fraction digits after it and the binary
    // exponent of the leading digit's lowest bit.
    let (mut leading, fraction, fraction_digits, mut exponent): (u64, u64, usize, i64) = match value
    {
        | Floating::Double(value) => {
            let bits = value.to_bits();
            let biased = i64::try_from((bits >> 52) & 0x7FF).expect("eleven bits");
            let fraction = bits & ((1 << 52) - 1);
            match (biased, fraction) {
                | (0, 0) => (0, 0, 13, 0),
                | (0, _) => (0, fraction, 13, -1022),
                | _ => (1, fraction, 13, biased - 1023),
            }
        },
        | Floating::Extended(value) => {
            let bits = value.to_bits();
            let significand = u64::try_from(bits & u128::from(u64::MAX)).expect("masked");
            let biased = i64::try_from((bits >> 64) & 0x7FFF).expect("fifteen bits");
            if significand == 0 {
                (0, 0, 15, 0)
            } else {
                (
                    significand >> 60,
                    significand & ((1 << 60) - 1),
                    15,
                    biased.max(1) - 16383 - 3,
                )
            }
        },
    };
    let mut digits = [0_u8; 15];
    for (index, digit) in digits[..fraction_digits].iter_mut().enumerate() {
        *digit = u8::try_from((fraction >> (4 * (fraction_digits - 1 - index))) & 0xF)
            .expect("a nibble");
    }
    let shown = match precision {
        | Some(precision) => precision,
        | None => {
            let mut shown = fraction_digits;
            while shown > 0 && digits[shown - 1] == 0 {
                shown -= 1;
            }
            shown
        },
    };
    if shown < fraction_digits {
        let last = if shown == 0 {
            leading
        } else {
            u64::from(digits[shown - 1])
        };
        let next = digits[shown];
        let rest = digits[shown + 1..fraction_digits]
            .iter()
            .any(|&digit| digit != 0);
        let up = next > 8 || (next == 8 && (rest || last % 2 == 1));
        if up {
            let mut carry = true;
            for digit in digits[..shown].iter_mut().rev() {
                if *digit == 15 {
                    *digit = 0;
                } else {
                    *digit += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                leading += 1;
                if leading > 15 {
                    // glibc restarts the x87 leading digit at 1.
                    leading = 1;
                    exponent += 4;
                }
            }
        }
    }
    let hex = |digit: u64| {
        let digit = u8::try_from(digit).expect("a nibble");
        match (digit, upper) {
            | (0..=9, _) => b'0' + digit,
            | (_, true) => b'A' + digit - 10,
            | (_, false) => b'a' + digit - 10,
        }
    };
    out.extend_from_slice(if upper { b"0X" } else { b"0x" });
    out.push(hex(leading));
    if shown > 0 || alternate {
        out.push(b'.');
    }
    for index in 0..shown {
        out.push(hex(digits.get(index).copied().map_or(0, u64::from)));
    }
    out.push(if upper { b'P' } else { b'p' });
    out.push(if exponent < 0 { b'-' } else { b'+' });
    push_decimal(out, exponent.unsigned_abs());
}
