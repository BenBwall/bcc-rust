//! Arithmetic on little-endian 64-bit limbs, the working form of folded
//! `_BitInt` constants wider than 128 bits ([`super::WideIntegers`]).
//!
//! A *number* is a slice holding a two's-complement value, sign-extended
//! through its length; a *magnitude* is a slice holding an unsigned value,
//! zero-extended. Callers size every output; nothing here allocates except
//! [`divide`]'s normalized copies, which come from the scratch arena it is
//! given. Multiplication is schoolbook and division is Knuth's algorithm D
//! (TAOCP vol. 2, §4.3.1), both quadratic in the limb count.
//! C23: §6.2.6.2 paragraphs 1-2, pp. 42-43; PDF pp. 55-56.

use std::cmp::Ordering;

use crate::util::bump::Bump;

/// Writes `value`, read as signed or unsigned, to `out` extended or
/// truncated to `out`'s length.
pub(super) fn extend(value: &[u64], signed: bool, out: &mut [u64]) {
    let fill = fill(signed && negative(value));
    for (index, limb) in out.iter_mut().enumerate() {
        *limb = value.get(index).copied().unwrap_or(fill);
    }
}

/// Whether the number `value` is negative.
pub(super) fn negative(value: &[u64]) -> bool {
    value.last().is_some_and(|&limb| limb >> 63 == 1)
}

/// The limb that extends a number of the given sign.
pub(super) const fn fill(negative: bool) -> u64 {
    if negative { u64::MAX } else { 0 }
}

/// `out = left + right` modulo 2^(64 * len); all three have one length.
pub(super) fn add(left: &[u64], right: &[u64], out: &mut [u64]) {
    let mut carry = false;
    for ((out, &left), &right) in out.iter_mut().zip(left).zip(right) {
        let (sum, first) = left.overflowing_add(right);
        let (sum, second) = sum.overflowing_add(u64::from(carry));
        *out = sum;
        carry = first || second;
    }
}

/// `out = left - right` modulo 2^(64 * len); all three have one length.
pub(super) fn subtract(left: &[u64], right: &[u64], out: &mut [u64]) {
    let mut borrow = false;
    for ((out, &left), &right) in out.iter_mut().zip(left).zip(right) {
        let (difference, first) = left.overflowing_sub(right);
        let (difference, second) = difference.overflowing_sub(u64::from(borrow));
        *out = difference;
        borrow = first || second;
    }
}

/// Negates the number `value` in place, modulo 2^(64 * len).
pub(super) fn negate(value: &mut [u64]) {
    let mut carry = true;
    for limb in value {
        let (sum, overflow) = (!*limb).overflowing_add(u64::from(carry));
        *limb = sum;
        carry = overflow;
    }
}

/// Orders two numbers of one length.
pub(super) fn compare(left: &[u64], right: &[u64]) -> Ordering {
    match (negative(left), negative(right)) {
        | (true, false) => return Ordering::Less,
        | (false, true) => return Ordering::Greater,
        | _ => {},
    }
    // Of one sign, two's-complement limbs order as unsigned ones.
    left.iter().rev().cmp(right.iter().rev())
}

/// `value` without its high zero limbs, as a magnitude.
pub(super) fn trimmed(value: &[u64]) -> &[u64] {
    let length = value
        .iter()
        .rposition(|&limb| limb != 0)
        .map_or(0, |i| i + 1);
    &value[..length]
}

/// `value` without the high limbs that only extend it by `signed`ness:
/// the shortest slice that [`extend`] reads back as the same value.
pub(super) fn minimal(value: &[u64], signed: bool) -> &[u64] {
    let mut length = value.len();
    if !signed {
        return trimmed(value);
    }
    while length > 1 {
        let (top, next) = (value[length - 1], value[length - 2]);
        if top != fill(next >> 63 == 1) {
            break;
        }
        length -= 1;
    }
    if length == 1 && value[0] == 0 {
        length = 0;
    }
    &value[..length]
}

/// The number of bits of the magnitude `value` up to its highest set one.
pub(super) fn bit_length(value: &[u64]) -> u64 {
    let value = trimmed(value);
    value.last().map_or(0, |&top| {
        64 * (value.len() as u64 - 1) + u64::from(64 - top.leading_zeros())
    })
}

/// `out += left * right` for magnitudes; `out` holds at least
/// `left.len() + right.len()` limbs, and the sum fits it.
pub(super) fn multiply(left: &[u64], right: &[u64], out: &mut [u64]) {
    for (i, &a) in left.iter().enumerate() {
        if a == 0 {
            continue;
        }
        let mut carry = 0_u128;
        for (j, &b) in right.iter().enumerate() {
            // (2^64 - 1)^2 + 2 (2^64 - 1) is 2^128 - 1: no overflow.
            let sum = u128::from(a) * u128::from(b) + u128::from(out[i + j]) + carry;
            out[i + j] = low(sum);
            carry = sum >> 64;
        }
        for limb in &mut out[i + right.len()..] {
            if carry == 0 {
                break;
            }
            let sum = u128::from(*limb) + carry;
            *limb = low(sum);
            carry = sum >> 64;
        }
    }
}

/// Divides the magnitude `dividend` by the nonzero magnitude `divisor`,
/// writing the quotient (at least `dividend.len()` limbs) and the remainder
/// (at least `divisor.len()` limbs), with Knuth's algorithm D: each step
/// estimates a quotient limb from the leading two limbs of the normalized
/// remainder over the divisor's leading limb, corrects it with the next
/// limb, so that it is at most one too large, and adds back once if the
/// multiply-and-subtract went negative.
pub(super) fn divide(
    dividend: &[u64],
    divisor: &[u64],
    quotient: &mut [u64],
    remainder: &mut [u64],
    scratch: &Bump,
) {
    quotient.fill(0);
    remainder.fill(0);
    let (u, v) = (trimmed(dividend), trimmed(divisor));
    let n = v.len();
    assert!(n > 0, "division by a zero magnitude");
    if u.len() < n {
        remainder[..u.len()].copy_from_slice(u);
        return;
    }
    if n == 1 {
        let d = u128::from(v[0]);
        let mut rest = 0_u128;
        for i in (0..u.len()).rev() {
            let number = rest << 64 | u128::from(u[i]);
            quotient[i] = low(number / d);
            rest = number % d;
        }
        remainder[0] = low(rest);
        return;
    }
    let m = u.len() - n;
    let shift = v[n - 1].leading_zeros();
    let vn = scratch.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, n));
    let un = scratch.alloc_slice_fill_iter(std::iter::repeat_n(0_u64, u.len() + 1));
    shift_into(v, shift, vn);
    shift_into(u, shift, un);
    let top = u128::from(vn[n - 1]);
    let next = u128::from(vn[n - 2]);
    for j in (0..=m).rev() {
        let number = u128::from(un[j + n]) << 64 | u128::from(un[j + n - 1]);
        let mut estimate = number / top;
        let mut rest = number % top;
        while estimate >> 64 != 0 || estimate * next > (rest << 64 | u128::from(un[j + n - 2])) {
            estimate -= 1;
            rest += top;
            if rest >> 64 != 0 {
                break;
            }
        }
        let mut borrow = 0_u64;
        let mut carry = 0_u128;
        for i in 0..n {
            let product = estimate * u128::from(vn[i]) + carry;
            carry = product >> 64;
            let (difference, first) = un[i + j].overflowing_sub(low(product));
            let (difference, second) = difference.overflowing_sub(borrow);
            un[i + j] = difference;
            borrow = u64::from(first) + u64::from(second);
        }
        let (difference, first) = un[j + n].overflowing_sub(low(carry));
        let (difference, second) = difference.overflowing_sub(borrow);
        un[j + n] = difference;
        if first || second {
            // The estimate was one too large: add the divisor back.
            estimate -= 1;
            let mut carry = 0_u128;
            for i in 0..n {
                let sum = u128::from(un[i + j]) + u128::from(vn[i]) + carry;
                un[i + j] = low(sum);
                carry = sum >> 64;
            }
            un[j + n] = un[j + n].wrapping_add(low(carry));
        }
        quotient[j] = low(estimate);
    }
    for i in 0..n {
        remainder[i] = if shift == 0 {
            un[i]
        } else {
            un[i] >> shift | un[i + 1] << (64 - shift)
        };
    }
}

/// Writes the magnitude `value` shifted left by `shift` (below 64) to
/// `out`, which holds the limb the shift carries out of `value` too.
fn shift_into(value: &[u64], shift: u32, out: &mut [u64]) {
    let mut carry = 0_u64;
    for (i, &limb) in value.iter().enumerate() {
        out[i] = limb << shift | carry;
        carry = if shift == 0 { 0 } else { limb >> (64 - shift) };
    }
    if let Some(last) = out.get_mut(value.len()) {
        *last = carry;
    }
}

/// `out = value << count` for a number, modulo 2^(64 * `out.len()`).
pub(super) fn shift_left(value: &[u64], count: u64, out: &mut [u64]) {
    let extension = fill(negative(value));
    let (limbs, bits) = (count / 64, (count % 64) as u32);
    for (index, out) in out.iter_mut().enumerate() {
        let index = index as u64;
        let at = |i: u64| -> u64 {
            if i < limbs {
                0
            } else {
                usize::try_from(i - limbs)
                    .ok()
                    .and_then(|i| value.get(i).copied())
                    .unwrap_or(extension)
            }
        };
        *out = if bits == 0 {
            at(index)
        } else {
            let below = if index == 0 { 0 } else { at(index - 1) };
            at(index) << bits | below >> (64 - bits)
        };
    }
}

/// `out = value >> count` for a number, rounding toward negative infinity
/// (an arithmetic shift), extended through `out`.
pub(super) fn shift_right(value: &[u64], count: u64, out: &mut [u64]) {
    let extension = fill(negative(value));
    let (limbs, bits) = (count / 64, (count % 64) as u32);
    let at = |i: u64| -> u64 {
        usize::try_from(i + limbs)
            .ok()
            .and_then(|i| value.get(i).copied())
            .unwrap_or(extension)
    };
    for (index, out) in out.iter_mut().enumerate() {
        let index = index as u64;
        *out = if bits == 0 {
            at(index)
        } else {
            at(index) >> bits | at(index + 1) << (64 - bits)
        };
    }
}

/// The low 64 bits of `value`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "A limb is the low half of a double-limb intermediate."
)]
const fn low(value: u128) -> u64 {
    value as u64
}
