//! Value types.
//!
//! Integers are signless: signedness belongs to the operation (`sdiv` or
//! `udiv`, `slt` or `ult`, `sext` or `zext`). Pointers are opaque.
//! Aggregates are never values; they live in stack slots or globals.

use std::fmt;

/// The type of an SSA value.
///
/// `i1` is the result of comparisons and the type of branch conditions; C's
/// `_Bool` is an `i8` in memory.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub(crate) enum Type {
    I1,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    /// The x87 80-bit extended format.
    F80,
    /// The IEEE 754 binary128 format.
    F128,
    /// An opaque pointer. Every supported target is 64-bit.
    Ptr,
}

impl Type {
    /// Every type, in declaration order.
    pub(crate) const ALL: [Self; 11] = [
        Self::I1,
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::F32,
        Self::F64,
        Self::F80,
        Self::F128,
        Self::Ptr,
    ];

    /// The width of the value in bits.
    pub(crate) const fn bits(self) -> u32 {
        match self {
            | Self::I1 => 1,
            | Self::I8 => 8,
            | Self::I16 => 16,
            | Self::I32 | Self::F32 => 32,
            | Self::I64 | Self::F64 | Self::Ptr => 64,
            | Self::F80 => 80,
            | Self::I128 | Self::F128 => 128,
        }
    }

    /// The bytes the value's bits occupy, rounded up: `i1` takes one and
    /// `f80` ten. Target layouts may pad a stored value further.
    pub(crate) const fn bytes(self) -> u32 {
        self.bits().div_ceil(8)
    }

    pub(crate) const fn is_int(self) -> bool {
        matches!(
            self,
            Self::I1 | Self::I8 | Self::I16 | Self::I32 | Self::I64 | Self::I128
        )
    }

    pub(crate) const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64 | Self::F80 | Self::F128)
    }

    /// The mask of the low [`Type::bits`] bits.
    pub(crate) const fn mask(self) -> u128 {
        u128::MAX >> (128 - self.bits())
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::I1 => "i1",
            | Self::I8 => "i8",
            | Self::I16 => "i16",
            | Self::I32 => "i32",
            | Self::I64 => "i64",
            | Self::I128 => "i128",
            | Self::F32 => "f32",
            | Self::F64 => "f64",
            | Self::F80 => "f80",
            | Self::F128 => "f128",
            | Self::Ptr => "ptr",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|ty| ty.name() == name)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
