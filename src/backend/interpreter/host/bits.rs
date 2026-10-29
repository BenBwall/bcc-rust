//! The functions of C23's `<stdbit.h>`, which glibc 2.39 and later define
//! and its header calls where it does not expand one inline: each of the
//! fourteen operations for `unsigned char`, `unsigned short`, `unsigned
//! int`, `unsigned long` and `unsigned long long`, named `stdc_<operation>`
//! with the suffix `_uc`, `_us`, `_ui`, `_ul` or `_ull`.
//!
//! C23: §7.18.3-§7.18.16, pp. 305-311; PDF pp. 318-324.

use super::{
    Fault,
    HostCall,
    HostReturn,
    RuntimeValue,
    Type,
    integer,
};

/// The operations, as their names spell them after `stdc_`.
pub(super) const OPERATIONS: [&str; 14] = [
    "leading_zeros",
    "leading_ones",
    "trailing_zeros",
    "trailing_ones",
    "first_leading_zero",
    "first_leading_one",
    "first_trailing_zero",
    "first_trailing_one",
    "count_zeros",
    "count_ones",
    "has_single_bit",
    "bit_width",
    "bit_floor",
    "bit_ceil",
];

/// The type suffixes, from `unsigned char` to `unsigned long long`.
pub(super) const SUFFIXES: [&str; 5] = ["uc", "us", "ui", "ul", "ull"];

/// What a function name names: the operation, the suffix's index in
/// [`SUFFIXES`], and the result's kind.
pub(super) fn parse(name: &str) -> Option<(&str, usize, Returns)> {
    let (operation, suffix) = name.strip_prefix("stdc_")?.rsplit_once('_')?;
    let operation = *OPERATIONS.iter().find(|&&known| known == operation)?;
    let suffix = SUFFIXES.iter().position(|&known| known == suffix)?;
    let result = match operation {
        | "has_single_bit" => Returns::Bool,
        | "bit_floor" | "bit_ceil" => Returns::Value,
        | _ => Returns::Count,
    };
    Some((operation, suffix, result))
}

/// The type of a function's result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Returns {
    /// `unsigned int`.
    Count,
    /// `bool`.
    Bool,
    /// The parameter's type.
    Value,
}

/// Runs `call` if it names a `<stdbit.h>` function, on a target whose
/// `unsigned long` is `long_width` bits.
pub(super) fn call(call: HostCall<'_>, long_width: u32) -> Option<Result<HostReturn, Fault>> {
    let (operation, suffix, result) = parse(call.name)?;
    Some((|| {
        let width = [8, 16, 32, long_width, 64][suffix];
        let mask = u128::MAX >> (128 - width);
        let x = integer(call.args.first().copied().unwrap_or(RuntimeValue::Poison))? & mask;
        let leading = |value: u128| (value & mask).leading_zeros() - (128 - width);
        let trailing = |value: u128| (value & mask).trailing_zeros().min(width);
        let first = |count: u32| if count == width { 0 } else { count + 1 };
        let value: u128 = match operation {
            | "leading_zeros" => leading(x).into(),
            | "leading_ones" => leading(!x).into(),
            | "trailing_zeros" => trailing(x).into(),
            | "trailing_ones" => trailing(!x).into(),
            | "first_leading_zero" => first(leading(!x)).into(),
            | "first_leading_one" => first(leading(x)).into(),
            | "first_trailing_zero" => first(trailing(!x)).into(),
            | "first_trailing_one" => first(trailing(x)).into(),
            | "count_zeros" => (width - x.count_ones()).into(),
            | "count_ones" => x.count_ones().into(),
            | "has_single_bit" => u128::from(x.is_power_of_two()),
            | "bit_width" => (width - leading(x)).into(),
            | "bit_floor" if x == 0 => 0,
            | "bit_floor" => 1 << (width - 1 - leading(x)),
            | _ => x
                .max(1)
                .checked_next_power_of_two()
                .filter(|&ceiling| ceiling <= mask)
                .unwrap_or(0),
        };
        let ty = match result {
            | Returns::Count => Type::I32,
            | Returns::Bool => Type::I8,
            | Returns::Value => match width {
                | 8 => Type::I8,
                | 16 => Type::I16,
                | 32 => Type::I32,
                | _ => Type::I64,
            },
        };
        Ok(HostReturn::Value(Some(RuntimeValue::int(ty, value))))
    })())
}
