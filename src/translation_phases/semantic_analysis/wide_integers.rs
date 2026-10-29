//! Exact folding of integer constants of bit-precise types wider than 128
//! bits, up to `BITINT_MAXWIDTH`.
//!
//! An [`Integer`] holds its exact value in `value` whenever that fits
//! `i128` (for an unsigned type of more than 128 bits, below 2^127), so
//! every consumer that reads a small value, a size or a truth value needs
//! nothing else. Any other value of a type wider than 128 bits names, in
//! `Integer::wide`, its two's-complement limbs interned in a
//! [`WideIntegers`] table, with the low 128 bits kept in `value`, which is
//! what a conversion to a type of at most 128 bits reads
//! ([`Integer::cast`]). Equal values of one type share one entry, so
//! `Integer`'s equality stays exact.
//!
//! Every operation on a value of such a type goes through the table:
//! [`WideIntegers::binary`], [`WideIntegers::unary`],
//! [`WideIntegers::convert`], [`WideIntegers::shift`] and the floating
//! conversions compute the exact result in a scratch arena (`limbs`) and
//! then check or reduce it at the result type's width N exactly as the
//! 128-bit arithmetic does: a signed result outside the type's range
//! overflowed and is no constant, an unsigned one wraps modulo 2^N. After
//! analysis the table freezes into [`WideValues`], which
//! `SemanticTranslationUnit` keeps for lowering and inspection; both read
//! values through [`WideStore`].
//!
//! Reading order: [`WideIntegers::binary`], then `arithmetic`, `finish`
//! and `canonical`, then the [`WideStore`] readers.
//!
//! - `wide_integers/limbs.rs`: arithmetic on limb slices.
//!
//! C23: §6.2.5 paragraph 8, p. 39; PDF p. 52 (bit-precise widths);
//! §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56.
//! C99: §6.3.1.3, p. 43; PDF p. 55; §6.5 paragraph 5, p. 67; PDF p. 79;
//! §6.6 paragraph 4, p. 95; PDF p. 107.

mod limbs;

use std::{
    cell::RefCell,
    cmp::Ordering,
    fmt,
    num::NonZeroU32,
};

use rustc_hash::FxBuildHasher;

use super::{
    Integer,
    integer::common,
};
use crate::{
    float_parsing::bits,
    ir::Type,
    translation_phases::parsing::syntax::{
        BinaryOperator,
        UnaryOperator,
    },
    util::{
        bump::Bump,
        dedup_arena::DedupArena,
    },
};

impl WideIntegers<'_> {
    /// The value of `left op right` under the usual arithmetic conversions,
    /// for operands of any integer types, exactly; `None` where it has no
    /// constant value: a signed result outside its type, a division by zero,
    /// or a shift whose count is negative or not below the left operand's
    /// width. Operands of at most 128 bits take [`Integer::binary`].
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57; §6.5.5-§6.5.14,
    /// pp. 82-89; PDF pp. 94-101.
    pub(crate) fn binary(
        &self,
        left: Integer,
        op: BinaryOperator,
        right: Integer,
    ) -> Option<Integer> {
        use BinaryOperator as B;
        if !left.wide() && !right.wide() {
            return left.binary(op, right);
        }
        match op {
            | B::LogicalAnd =>
                return Some(Integer::int(i128::from(
                    !left.is_zero() && !right.is_zero(),
                ))),
            | B::LogicalOr =>
                return Some(Integer::int(i128::from(
                    !left.is_zero() || !right.is_zero(),
                ))),
            | B::LeftShift | B::RightShift => {
                // A shift's type is its left operand's; a wide count is
                // read by its value.
                let count = right.to_i128()?;
                if !left.wide() {
                    return left.binary(
                        op,
                        Integer {
                            value:  count,
                            bits:   128,
                            signed: true,
                            wide:   None,
                        },
                    );
                }
                let count = u32::try_from(count)
                    .ok()
                    .filter(|&count| count < left.bits)?;
                return self.shift(op, left, count).map(|(value, _)| value);
            },
            | _ => {},
        }
        let (bits, signed) = common((left.bits, left.signed), (right.bits, right.signed));
        let (left, right) = (
            self.convert(left, bits, signed),
            self.convert(right, bits, signed),
        );
        let comparison = self.compare(left, right);
        let predicate = match op {
            | B::LessThan => Some(comparison.is_lt()),
            | B::LessThanOrEqual => Some(comparison.is_le()),
            | B::GreaterThan => Some(comparison.is_gt()),
            | B::GreaterThanOrEqual => Some(comparison.is_ge()),
            | B::Equal => Some(comparison.is_eq()),
            | B::NotEqual => Some(comparison.is_ne()),
            | _ => None,
        };
        if let Some(result) = predicate {
            return Some(Integer::int(i128::from(result)));
        }
        if matches!(op, B::Division | B::Modulo) && right.is_zero() {
            return None;
        }
        self.with_scratch(|scratch| self.arithmetic(op, left, right, scratch))
    }

    /// `left op right` for operands converted to their common type, whose
    /// width is above 128 bits: the exact result, checked against or
    /// reduced to that width. Products are schoolbook and quotients
    /// truncate toward zero (C99 §6.5.5p6), with the remainder taking the
    /// dividend's sign; a quotient the type cannot hold (its minimum over
    /// -1) makes the remainder no constant too, as for narrow types.
    /// C99: §6.5.5 paragraphs 3-6, p. 82; PDF p. 94; §6.5.6 paragraph 5,
    /// p. 83; PDF p. 95; §6.5.10-§6.5.12, pp. 87-88; PDF pp. 99-100.
    fn arithmetic(
        &self,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        scratch: &Bump,
    ) -> Option<Integer> {
        use BinaryOperator as B;
        let (bits, signed) = (left.bits, left.signed);
        let length = limb_count(bits) + 1;
        let (a, b) = (
            load(self, left, length, scratch),
            load(self, right, length, scratch),
        );
        let out = zeroed(scratch, length);
        match op {
            | B::Addition => limbs::add(a, b, out),
            | B::Subtraction => limbs::subtract(a, b, out),
            | B::BitwiseAnd | B::BitwiseOr | B::BitwiseXor =>
                for ((out, &a), &b) in out.iter_mut().zip(a.iter()).zip(b.iter()) {
                    *out = match op {
                        | B::BitwiseAnd => a & b,
                        | B::BitwiseOr => a | b,
                        | _ => a ^ b,
                    };
                },
            | B::Multiplication => {
                let negative = magnitude(a) != magnitude(b);
                let (a, b) = (limbs::trimmed(a), limbs::trimmed(b));
                let product = zeroed(scratch, (a.len() + b.len()).max(length) + 1);
                limbs::multiply(a, b, product);
                if negative {
                    limbs::negate(product);
                }
                return self.finish(product, bits, signed, true, scratch);
            },
            | B::Division | B::Modulo => {
                let (dividend_negative, divisor_negative) = (magnitude(a), magnitude(b));
                let quotient = zeroed(scratch, length);
                let remainder = zeroed(scratch, length);
                limbs::divide(a, b, quotient, remainder, scratch);
                if dividend_negative != divisor_negative {
                    limbs::negate(quotient);
                }
                if dividend_negative {
                    limbs::negate(remainder);
                }
                let quotient = self.finish(quotient, bits, signed, true, scratch)?;
                return if op == B::Division {
                    Some(quotient)
                } else {
                    self.finish(remainder, bits, signed, true, scratch)
                };
            },
            | _ => return None,
        }
        self.finish(out, bits, signed, true, scratch)
    }

    /// The unary operator `op` applied to `value`, exactly; `None` where
    /// the result has no constant value (the negated minimum of a signed
    /// type). Values of at most 128 bits take [`Integer::unary`].
    /// C99: §6.5.3.3 paragraphs 1-5, p. 79; PDF p. 91.
    pub(crate) fn unary(&self, value: Integer, op: UnaryOperator) -> Option<Integer> {
        if !value.wide() {
            return value.unary(op);
        }
        match op {
            | UnaryOperator::Plus | UnaryOperator::Extension => Some(value),
            | UnaryOperator::LogicalNot => Some(Integer::int(i128::from(value.is_zero()))),
            | UnaryOperator::Minus | UnaryOperator::BitwiseNot => self.with_scratch(|scratch| {
                let number = load(self, value, limb_count(value.bits) + 1, scratch);
                if op == UnaryOperator::Minus {
                    limbs::negate(number);
                } else {
                    for limb in number.iter_mut() {
                        *limb = !*limb;
                    }
                }
                self.finish(number, value.bits, value.signed, true, scratch)
            }),
            | _ => None,
        }
    }

    /// `value` converted to the integer type of `bits` width and `signed`ness:
    /// the same value if the type holds it, else reduced modulo 2^bits
    /// (and read in two's complement for a signed type, which is
    /// implementation-defined: GCC and Clang wrap).
    /// C99: §6.3.1.3 paragraphs 1-3, p. 43; PDF p. 55.
    pub(crate) fn convert(&self, value: Integer, bits: u32, signed: bool) -> Integer {
        if bits <= 128 {
            return value.cast(bits, signed);
        }
        if value.wide.is_none() {
            let negative = value.signed && value.value < 0;
            let high_half = !value.signed && value.bits <= 128 && value.value < 0;
            // Any other `i128` value lies in the type's range.
            if !high_half && (signed || !negative) {
                return Integer {
                    value: value.value,
                    bits,
                    signed,
                    wide: None,
                };
            }
        }
        self.with_scratch(|scratch| {
            let number = load(self, value, limb_count(bits.max(value.bits)) + 1, scratch);
            self.finish(number, bits, signed, false, scratch)
        })
        .expect("a wrapping conversion has a value")
    }

    /// `left` shifted by `count` (below its width N) at its own bit-precise
    /// type of more than 128 bits, and whether a signed result went into
    /// (not past) the sign bit, where it folds to its two's-complement
    /// value: `None` for a negative value shifted left or a signed result
    /// of 2^N or more. A right shift of a negative value rounds toward
    /// negative infinity, as GCC and Clang shift arithmetically.
    /// C99: §6.5.7 paragraphs 3-5, p. 85; PDF p. 97 (implementation-defined
    /// right shift of a negative value). GNU extension: GCC manual,
    /// "Integers".
    pub(crate) fn shift(
        &self,
        op: BinaryOperator,
        left: Integer,
        count: u32,
    ) -> Option<(Integer, bool)> {
        let (bits, signed) = (left.bits, left.signed);
        self.with_scratch(|scratch| {
            if op == BinaryOperator::RightShift {
                let length = limb_count(bits) + 1;
                let number = load(self, left, length, scratch);
                let out = zeroed(scratch, length);
                limbs::shift_right(number, u64::from(count), out);
                return Some((self.finish(out, bits, signed, true, scratch)?, false));
            }
            if signed && self.negative(left) {
                return None;
            }
            let length = limb_count(bits + count) + 1;
            let number = load(self, left, length, scratch);
            let out = zeroed(scratch, length);
            limbs::shift_left(number, u64::from(count), out);
            let used = limbs::bit_length(out);
            if signed && used > u64::from(bits) {
                return None;
            }
            let sign_bit = signed && used == u64::from(bits);
            Some((self.finish(out, bits, signed, false, scratch)?, sign_bit))
        })
    }

    /// Whether the infinitely precise `left op right` (`+`, `-` or `*`)
    /// lies outside the integer type of `bits` width and `signed`ness, for
    /// an operand or a type wider than 128 bits: what
    /// `__builtin_add_overflow_p` and its siblings answer.
    /// GNU extension: GCC manual, "Integer Overflow Builtins".
    pub(crate) fn overflows(
        &self,
        op: BinaryOperator,
        left: Integer,
        right: Integer,
        bits: u32,
        signed: bool,
    ) -> bool {
        self.with_scratch(|scratch| {
            let length = limb_count(left.bits.max(right.bits).max(bits)) + 1;
            let (a, b) = (
                load(self, left, length, scratch),
                load(self, right, length, scratch),
            );
            let result = zeroed(scratch, 2 * length + 1);
            match op {
                | BinaryOperator::Multiplication => {
                    let negative = magnitude(a) != magnitude(b);
                    limbs::multiply(limbs::trimmed(a), limbs::trimmed(b), result);
                    if negative {
                        limbs::negate(result);
                    }
                },
                | _ => {
                    let (a_wide, b_wide) =
                        (zeroed(scratch, result.len()), zeroed(scratch, result.len()));
                    limbs::extend(a, true, a_wide);
                    limbs::extend(b, true, b_wide);
                    if op == BinaryOperator::Subtraction {
                        limbs::subtract(a_wide, b_wide, result);
                    } else {
                        limbs::add(a_wide, b_wide, result);
                    }
                },
            }
            if signed {
                return self.finish(result, bits, true, true, scratch).is_none();
            }
            limbs::negative(result) || limbs::bit_length(result) > u64::from(bits)
        })
    }

    /// The order of the exact values of `left` and `right`, of any integer
    /// types.
    pub(crate) fn compare(&self, left: Integer, right: Integer) -> Ordering {
        if left.wide.is_none() && right.wide.is_none() {
            // An unsigned 128-bit value with its top bit set exceeds every
            // other, and those order as their patterns do.
            let key = |v: Integer| (!v.signed && v.bits <= 128 && v.value < 0, v.value);
            return key(left).cmp(&key(right));
        }
        self.with_scratch(|scratch| {
            let length = limb_count(left.bits.max(right.bits)) + 1;
            limbs::compare(
                load(self, left, length, scratch),
                load(self, right, length, scratch),
            )
        })
    }

    /// The finite floating value `(-1)^negative * mantissa * 2^shift`
    /// truncated toward zero to the integer type of `bits` (above 128) and
    /// `signed`ness, or `None` when the type cannot hold the truncated
    /// value: the behavior is undefined, and it is no constant.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(crate) fn truncated(
        &self,
        negative: bool,
        mantissa: u128,
        shift: i32,
        bits: u32,
        signed: bool,
    ) -> Option<Integer> {
        let leading = i64::from(128 - mantissa.leading_zeros()) + i64::from(shift);
        if mantissa == 0 || leading <= 0 {
            return Some(Integer {
                value: 0,
                bits,
                signed,
                wide: None,
            });
        }
        if leading > i64::from(bits) {
            return None;
        }
        self.with_scratch(|scratch| {
            let length = limb_count(bits) + 1;
            let source = zeroed(scratch, length);
            source[0] = low(mantissa);
            source[1] = low(mantissa >> 64);
            let number = zeroed(scratch, length);
            if shift >= 0 {
                limbs::shift_left(source, u64::from(shift.unsigned_abs()), number);
            } else {
                limbs::shift_right(source, u64::from(shift.unsigned_abs()), number);
            }
            if negative {
                if !signed {
                    return None;
                }
                limbs::negate(number);
            }
            self.finish(number, bits, signed, true, scratch)
        })
    }

    /// The least (`negative`) or greatest value of the integer type of
    /// `bits` (above 128) and `signed`ness: the value GCC stores for an
    /// out-of-range floating constant.
    /// C99: §6.2.6.2 paragraph 2, p. 39; PDF p. 51.
    pub(crate) fn bound(&self, bits: u32, signed: bool, negative: bool) -> Integer {
        let zero = Integer {
            value: 0,
            bits,
            signed,
            wide: None,
        };
        if !signed {
            return if negative {
                zero
            } else {
                self.convert(Integer::int(-1), bits, false)
            };
        }
        self.with_scratch(|scratch| {
            let length = limb_count(bits) + 1;
            let one = zeroed(scratch, length);
            one[0] = 1;
            let number = zeroed(scratch, length);
            limbs::shift_left(one, u64::from(bits - 1), number);
            if !negative {
                // 2^(N - 1) - 1.
                let maximum = zeroed(scratch, length);
                limbs::subtract(number, one, maximum);
                return self.finish(maximum, bits, signed, true, scratch);
            }
            self.finish(number, bits, signed, false, scratch)
        })
        .unwrap_or(zero)
    }

    /// The exact number `number` (of at least N bits) as a value of the
    /// type of `bits` width N and `signed`ness: `None` if `checked` and the
    /// type is signed but does not hold it, else reduced modulo 2^N and read
    /// in two's complement for a signed type.
    /// C99: §6.3.1.3 paragraphs 2-3, p. 43; PDF p. 55; §6.5 paragraph 5,
    /// p. 67; PDF p. 79.
    fn finish(
        &self,
        number: &[u64],
        bits: u32,
        signed: bool,
        checked: bool,
        scratch: &Bump,
    ) -> Option<Integer> {
        let count = limb_count(bits);
        let out = zeroed(scratch, count);
        limbs::extend(number, true, out);
        let partial = bits % 64;
        if partial != 0 {
            let mask = u64::MAX >> (64 - partial);
            let top = out[count - 1] & mask;
            out[count - 1] = if signed && top >> (partial - 1) & 1 == 1 {
                top | !mask
            } else {
                top
            };
        }
        if checked && signed {
            let fill = limbs::fill(limbs::negative(out));
            let same = number
                .iter()
                .enumerate()
                .all(|(index, &limb)| limb == out.get(index).copied().unwrap_or(fill));
            if !same {
                return None;
            }
        }
        Some(self.canonical(out, bits, signed))
    }

    /// The [`Integer`] of the type of `bits` width and `signed`ness whose
    /// value `limbs` holds, extended by `signed`ness: exact in `value` when
    /// `i128` holds it, else interned.
    fn canonical(&self, value: &[u64], bits: u32, signed: bool) -> Integer {
        let minimal = limbs::minimal(value, signed);
        let negative = signed && limbs::negative(minimal);
        let low = minimal.first().copied().unwrap_or(0);
        let high = minimal.get(1).copied().unwrap_or(limbs::fill(negative));
        let pattern = (u128::from(high) << 64 | u128::from(low)).cast_signed();
        let fits = minimal.len() <= 1 || (minimal.len() == 2 && (signed || high >> 63 == 0));
        let wide = (!fits).then(|| {
            let index = self
                .values
                .borrow_mut()
                .intern_by(minimal, || &*self.arena.alloc_slice_copy(minimal));
            WideId(NonZeroU32::new(index + 1).expect("an interned index below u32::MAX"))
        });
        Integer {
            value: pattern,
            bits,
            signed,
            wide,
        }
    }

    /// Runs `f` on the scratch arena, rewound first: every operation's
    /// working numbers live there, and only results are interned.
    fn with_scratch<R>(&self, f: impl FnOnce(&Bump) -> R) -> R {
        let mut scratch = self.scratch.borrow_mut();
        scratch.reset();
        f(&scratch)
    }
}

impl<'tu> WideIntegers<'tu> {
    pub(super) fn new(arena: &'tu Bump) -> Self {
        Self {
            arena,
            values: RefCell::new(DedupArena::with_hasher(FxBuildHasher, arena)),
            scratch: RefCell::new(Bump::new()),
        }
    }

    /// The frozen table, living as long as the translation unit's arena.
    pub(super) fn into_values(self) -> WideValues<'tu> {
        let values = self.values.borrow();
        WideValues {
            values: self.arena.alloc_slice_copy(&values),
        }
    }
}

/// The interned limbs of the folded integer constants of bit-precise types
/// wider than 128 bits whose values `i128` does not hold, during analysis.
pub(crate) struct WideIntegers<'tu> {
    arena:   &'tu Bump,
    values:  RefCell<DedupArena<'tu, &'tu [u64], FxBuildHasher>>,
    scratch: RefCell<Bump>,
}

/// An entry of a [`WideIntegers`] table: the minimal two's-complement
/// limbs of an exact value, extended by its type's signedness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WideId(NonZeroU32);

/// A frozen [`WideIntegers`] table, as `SemanticTranslationUnit` keeps it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WideValues<'tu> {
    values: &'tu [&'tu [u64]],
}

/// Reads the exact values of folded integer constants from a table.
pub(crate) trait WideStore<'tu> {
    /// The minimal limbs entry `id` holds.
    fn stored(&self, id: WideId) -> &'tu [u64];

    /// Whether `value` is below zero.
    fn negative(&self, value: Integer) -> bool {
        value.signed
            && value
                .wide
                .map_or(value.value < 0, |id| limbs::negative(self.stored(id)))
    }

    /// Writes `value`'s two's complement to `out` as little-endian limbs,
    /// extended (or truncated) to `out`'s length by its signedness.
    /// C23: §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56.
    fn write_limbs(&self, value: Integer, out: &mut [u64]) {
        if let Some(id) = value.wide {
            limbs::extend(self.stored(id), value.signed, out);
            return;
        }
        let pattern = value.value.cast_unsigned();
        let pair = [low(pattern), low(pattern >> 64)];
        let fill = limbs::fill(value.signed && value.value < 0);
        for (index, limb) in out.iter_mut().enumerate() {
            *limb = pair.get(index).copied().unwrap_or(fill);
        }
    }

    /// The bits of `value` converted to the floating format `ty`, rounded
    /// once to nearest even from its exact value: its top 128 bits from the
    /// leading one, with every lower bit folded into a sticky bit
    /// ([`bits::from_scaled`]).
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56.
    fn float_bits(&self, value: Integer, ty: Type) -> Option<u128> {
        let Some(id) = value.wide else {
            return if value.signed {
                bits::from_integer(value.value, ty)
            } else {
                bits::from_unsigned(value.value.cast_unsigned(), ty)
            };
        };
        let number = Magnitude::new(self.stored(id), value.signed);
        let length = number.bit_length();
        if length <= 128 {
            return bits::from_scaled(number.negative, number.bits(0), 0, ty);
        }
        let start = length - 128;
        let sticky = number.trailing_zeros < start;
        let scale = u32::try_from(start).ok()?;
        bits::from_scaled(
            number.negative,
            number.bits(start) | u128::from(sticky),
            scale,
            ty,
        )
    }

    /// `value` in decimal.
    fn display(&self, value: Integer) -> impl fmt::Display {
        let stored = value.wide.map(|id| self.stored(id));
        fmt::from_fn(move |f| {
            let Some(stored) = stored else {
                return fmt::Display::fmt(&value, f);
            };
            let number = Magnitude::new(stored, value.signed);
            // Base 10^19 digits, least significant first, by repeated
            // division of the magnitude.
            let scratch = Bump::new();
            let length = stored.len();
            let rest = zeroed(&scratch, length);
            for (index, limb) in rest.iter_mut().enumerate() {
                *limb = number.limb(index);
            }
            let digits = zeroed(&scratch, length * 64 / 63 + 1);
            let mut count = 0;
            let mut rest = &mut rest[..];
            while !limbs::trimmed(rest).is_empty() {
                let mut remainder = 0_u128;
                for limb in rest.iter_mut().rev() {
                    let number = remainder << 64 | u128::from(*limb);
                    *limb = low(number / 10_000_000_000_000_000_000);
                    remainder = number % 10_000_000_000_000_000_000;
                }
                digits[count] = low(remainder);
                count += 1;
                let used = limbs::trimmed(rest).len();
                rest = &mut rest[..used];
            }
            if number.negative {
                f.write_str("-")?;
            }
            let digits = &digits[..count];
            let (first, rest) = digits.split_last().unwrap_or((&0, &[]));
            write!(f, "{first}")?;
            for digit in rest.iter().rev() {
                write!(f, "{digit:019}")?;
            }
            Ok(())
        })
    }
}

impl<'tu> WideStore<'tu> for WideIntegers<'tu> {
    fn stored(&self, id: WideId) -> &'tu [u64] {
        self.values.borrow()[id.0.get() - 1]
    }
}

impl<'tu> WideStore<'tu> for WideValues<'tu> {
    fn stored(&self, id: WideId) -> &'tu [u64] {
        self.values[id.0.get() as usize - 1]
    }
}

/// The magnitude of a stored number, read limb by limb without copying:
/// for a negative number, `!x + 1`, whose carry passes through exactly the
/// limbs below the lowest set bit.
struct Magnitude<'a> {
    number:         &'a [u64],
    negative:       bool,
    trailing_zeros: u64,
}

impl<'a> Magnitude<'a> {
    fn new(number: &'a [u64], signed: bool) -> Self {
        let trailing_zeros = number
            .iter()
            .position(|&limb| limb != 0)
            .map_or(0, |index| {
                index as u64 * 64 + u64::from(number[index].trailing_zeros())
            });
        Self {
            number,
            negative: signed && limbs::negative(number),
            trailing_zeros,
        }
    }

    /// Limb `index` of the magnitude.
    fn limb(&self, index: usize) -> u64 {
        let limb = self.number.get(index).copied().unwrap_or(0);
        if !self.negative {
            return limb;
        }
        let carry_limb = usize::try_from(self.trailing_zeros / 64).unwrap_or(usize::MAX);
        match index.cmp(&carry_limb) {
            | Ordering::Less => 0,
            | Ordering::Equal => (!limb).wrapping_add(1),
            | Ordering::Greater => !self.number.get(index).copied().unwrap_or(u64::MAX),
        }
    }

    /// The number of bits up to the magnitude's highest set one.
    fn bit_length(&self) -> u64 {
        (0..self.number.len())
            .rev()
            .find_map(|index| {
                let limb = self.limb(index);
                (limb != 0).then(|| index as u64 * 64 + u64::from(64 - limb.leading_zeros()))
            })
            .unwrap_or(0)
    }

    /// The 128 bits of the magnitude from bit `start`.
    fn bits(&self, start: u64) -> u128 {
        let (index, shift) = (
            usize::try_from(start / 64).unwrap_or(usize::MAX),
            start % 64,
        );
        let at = |offset: usize| u128::from(self.limb(index.saturating_add(offset)));
        let window = at(0) | at(1) << 64;
        if shift == 0 {
            window
        } else {
            window >> shift | at(2) << (128 - shift)
        }
    }
}

/// `value`'s exact two's complement as a number of `length` limbs, which
/// leaves its top bit clear for an unsigned value.
fn load<'tu, 'b>(
    store: &impl WideStore<'tu>,
    value: Integer,
    length: usize,
    scratch: &'b Bump,
) -> &'b mut [u64] {
    let out = zeroed(scratch, length);
    store.write_limbs(value, out);
    out
}

/// Negates the number `value` in place if it is negative, leaving its
/// magnitude, and reports whether it was.
fn magnitude(value: &mut [u64]) -> bool {
    let negative = limbs::negative(value);
    if negative {
        limbs::negate(value);
    }
    negative
}

/// A slice of `length` zero limbs in `scratch`.
fn zeroed(scratch: &Bump, length: usize) -> &mut [u64] {
    scratch.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, length))
}

/// The number of 64-bit limbs of a `bits`-bit value.
const fn limb_count(bits: u32) -> usize {
    bits.div_ceil(64) as usize
}

/// The low 64 bits of `value`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "A limb is the low half of a 128-bit value."
)]
const fn low(value: u128) -> u64 {
    value as u64
}
