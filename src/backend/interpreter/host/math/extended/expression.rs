//! How compiled C code treats `long double` values, for the ports of
//! glibc's C functions: comparisons are false for an unordered pair (a NaN
//! or an invalid encoding), as `fucomi` and the branches GCC emits make
//! them; `(int)` conversion stores the integer indefinite when the value
//! does not fit; and glibc's classification macros compare magnitudes the
//! way GCC expands them for the x87 format.
//! C99: §6.5.8 paragraphs 3-6, p. 85; PDF p. 97; §6.3.1.4 paragraph 1,
//! p. 43; PDF p. 55; §7.12.3, pp. 216-218; PDF pp. 228-230.

use std::cmp::Ordering;

use crate::x87::X87;

/// `a < b`.
pub(super) fn less(a: X87, b: X87) -> bool {
    a.compare(b) == Some(Ordering::Less)
}

/// `a <= b`.
pub(super) fn less_equal(a: X87, b: X87) -> bool {
    a.compare(b).is_some_and(Ordering::is_le)
}

/// `a > b`.
pub(super) fn greater(a: X87, b: X87) -> bool {
    a.compare(b) == Some(Ordering::Greater)
}

/// `a >= b`.
pub(super) fn greater_equal(a: X87, b: X87) -> bool {
    a.compare(b).is_some_and(Ordering::is_ge)
}

/// `a == b`.
pub(super) fn equal(a: X87, b: X87) -> bool {
    a.compare(b) == Some(Ordering::Equal)
}

/// `isfinite`, as GCC expands it: `|x| <= LDBL_MAX`.
pub(super) fn is_finite(x: X87) -> bool {
    less_equal(x.with_sign(false), MAX)
}

/// `isnan`, as GCC expands it: `x` unordered with itself, invalid
/// encodings included.
pub(super) fn is_nan(x: X87) -> bool {
    x.compare(x).is_none()
}

/// `isinf`, as GCC expands it: `|x| > LDBL_MAX`.
pub(super) fn is_infinite(x: X87) -> bool {
    greater(x.with_sign(false), MAX)
}

/// `(long double) n`.
pub(super) fn integer(n: i64) -> X87 {
    X87::from_integer(n < 0, u128::from(n.unsigned_abs()))
}

/// `(int) x`: truncated toward zero, or the integer indefinite `INT_MIN`
/// that `fistp` stores for a NaN, an invalid encoding or a value out of
/// range.
pub(super) fn truncate(x: X87) -> i32 {
    x.to_integer(true, 32).map_or(i32::MIN, |bits| {
        u32::try_from(bits)
            .expect("32-bit integer bits")
            .cast_signed()
    })
}

/// `(long double) d` for a `double` constant, which is exact.
pub(super) fn double(d: f64) -> X87 {
    X87::from_f64(d)
}

/// `1.0L`.
pub(super) const ONE: X87 = X87::from_bits(0x3FFF_8000_0000_0000_0000);

/// `2.0L`.
pub(super) const TWO: X87 = X87::from_bits(0x4000_8000_0000_0000_0000);

/// `0.5L`.
pub(super) const HALF: X87 = X87::from_bits(0x3FFE_8000_0000_0000_0000);

/// `LDBL_MAX`.
pub(super) const MAX: X87 = X87::from_bits(0x7FFE_FFFF_FFFF_FFFF_FFFF);
