//! Arena-backed natural-number arithmetic supports exact binary128
//! conversion. Exponent parsing and carrier display accompany that arithmetic.

use std::{
    cmp::Ordering,
    fmt,
};

use super::Binary128;
use crate::{
    float_parsing::{
        FloatRangeError,
        ParseFloatError,
    },
    translation_phases::preprocessing::FloatTokenType,
    util::{
        bump::{
            ArenaVec,
            Bump,
        },
        packed::Packed,
    },
};

/// Nonnegative little-endian base-2^32 integer; storage belongs to scratch.
pub(super) struct Natural<'a>(pub(super) ArenaVec<'a, u32>);

#[expect(
    clippy::cast_possible_truncation,
    reason = "Base-2^32 limb stores discard high bits; arena extents fit usize on the required \
              64-bit host."
)]
impl<'a> Natural<'a> {
    pub(super) fn mul_add(&mut self, multiplier: u32, addend: u32) {
        let mut carry = u64::from(addend);
        for word in &mut self.0 {
            let value = u64::from(*word) * u64::from(multiplier) + carry;
            *word = value as u32;
            carry = value >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    pub(super) fn shift(&mut self, count: i64) {
        if self.0.is_empty() || count == 0 {
            return;
        }
        let words = (count / 32) as usize;
        let bits = (count % 32) as u32;
        let old = self.0.len();
        self.0.resize(old + words + 1, 0);
        self.0.copy_within(..old, words);
        self.0[..words].fill(0);
        let mut carry = 0_u64;
        for word in &mut self.0[words..] {
            let value = (u64::from(*word) << bits) | carry;
            *word = value as u32;
            carry = value >> 32;
        }
        self.trim();
    }

    pub(super) fn subtract(&mut self, other: &Self) {
        let mut borrow = 0_u64;
        for (index, word) in self.0.iter_mut().enumerate() {
            let sub = u64::from(other.0.get(index).copied().unwrap_or(0)) + borrow;
            let value = u64::from(*word);
            *word = value.wrapping_sub(sub) as u32;
            borrow = u64::from(value < sub);
        }
        self.trim();
    }

    pub(super) fn compare(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }

    pub(super) fn halve(&mut self) {
        let mut carry = 0;
        for word in self.0.iter_mut().rev() {
            let next = *word << 31;
            *word = (*word >> 1) | carry;
            carry = next;
        }
        self.trim();
    }

    pub(super) fn bits(&self) -> i64 {
        self.0.last().map_or(0, |word| {
            (self.0.len() as i64 - 1) * 32 + i64::from(32 - word.leading_zeros())
        })
    }

    pub(super) fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            _ = self.0.pop();
        }
    }

    pub(super) fn copy(&self, arena: &'a Bump) -> Self {
        let mut words = ArenaVec::new_in(arena);
        words.extend_from_slice(&self.0);
        Self(words)
    }

    pub(super) fn new(arena: &'a Bump, value: u32) -> Self {
        let mut words = ArenaVec::new_in(arena);
        if value != 0 {
            words.push(value);
        }
        Self(words)
    }
}

pub(super) fn exponent(text: &str) -> Option<i64> {
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut result = 0_i64;
    for digit in digits.bytes() {
        result = result
            .saturating_mul(10)
            .saturating_add(i64::from(digit - b'0'));
    }
    Some(if negative { -result } else { result })
}

pub(super) fn error(range: Option<FloatRangeError>) -> ParseFloatError {
    let bits = if matches!(range, Some(FloatRangeError::Overflow)) {
        0x7FFF_u128 << 112
    } else {
        0
    };
    let value = FloatTokenType::Float128(Binary128(Packed::new(bits)));
    range.map_or(ParseFloatError::Invalid(value), |range| {
        ParseFloatError::OutOfRange(value, range)
    })
}

impl fmt::Display for Binary128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bits = self.0.get();
        let exponent = (bits >> 112) & 0x7FFF;
        let fraction = bits & ((1_u128 << 112) - 1);
        if exponent == 0x7FFF {
            return f.write_str("inf");
        }
        if exponent == 0 && fraction == 0 {
            return f.write_str("0x0p+0");
        }
        let power = if exponent == 0 {
            -16382
        } else {
            exponent as i32 - 16383
        };
        write!(f, "0x{}.{fraction:028x}p{power:+}", u8::from(exponent != 0))
    }
}
