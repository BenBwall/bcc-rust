//! The integer types and the rules that convert between them: each type's
//! width and range on x86-64 Linux, the integer conversion rank, the
//! integer promotions, the usual arithmetic conversions, conversion to a
//! type that cannot hold a value, and the type of an integer constant.
//!
//! C sets only minimum ranges (an int must hold -32767 to 32767); the widths
//! here are the ones bcc, GCC and Clang give on x86-64 Linux (LP64): char 8
//! bits, short 16, int 32, long and long long 64. Plain char is signed.
//!
//! C99: §5.2.4.2.1 paragraph 1, pp. 21-23; PDF pp. 33-35 (minimum ranges).
//! C99: §6.2.5 paragraphs 4, 6 and 9, pp. 33-34; PDF pp. 45-46 (signed and
//! unsigned integer types; unsigned arithmetic is reduced modulo 2^N).
//! C99: §6.3.1.1 paragraphs 1-2, pp. 42-43; PDF pp. 54-55 (rank and the
//! integer promotions).
//! C99: §6.3.1.3 paragraphs 1-3, p. 43; PDF p. 55 (integer conversions).
//! C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57 (usual arithmetic
//! conversions).
//! C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68 (constant types).

/// An integer type the machine models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Integer {
    /// Plain `char`, signed on x86-64 (implementation-defined; C99
    /// §6.2.5p15).
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    UnsignedShort,
    Int,
    UnsignedInt,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
}

/// Why the usual arithmetic conversions chose their common type, for the
/// narration (C99 §6.3.1.8p1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Common {
    /// Both operands already have the type.
    Same,
    /// Both signed or both unsigned: the lower rank converts to the higher.
    Rank,
    /// The unsigned operand's rank is at least the signed one's: the
    /// signed operand converts to the unsigned type.
    Unsigned,
    /// The signed type can hold every value of the unsigned one, which
    /// converts to it.
    Signed,
    /// The signed type is higher in rank but no wider: both convert to its
    /// unsigned version.
    BothUnsigned,
}

impl Integer {
    /// Width in bits on x86-64.
    pub(super) const fn bits(self) -> u32 {
        match self {
            | Self::Char | Self::SignedChar | Self::UnsignedChar => 8,
            | Self::Short | Self::UnsignedShort => 16,
            | Self::Int | Self::UnsignedInt => 32,
            | Self::Long | Self::UnsignedLong | Self::LongLong | Self::UnsignedLongLong => 64,
        }
    }

    /// Size in bytes on x86-64; each type is aligned to its size.
    pub(super) const fn bytes(self) -> usize {
        match self.bits() {
            | 8 => 1,
            | 16 => 2,
            | 32 => 4,
            | _ => 8,
        }
    }

    /// Whether the type is signed. Plain char is, on x86-64.
    pub(super) const fn signed(self) -> bool {
        matches!(
            self,
            Self::Char | Self::SignedChar | Self::Short | Self::Int | Self::Long | Self::LongLong
        )
    }

    /// The integer conversion rank: char types 1, short 2, int 3, long 4,
    /// long long 5; an unsigned type ranks with its signed one.
    ///
    /// C99: §6.3.1.1 paragraph 1, p. 42; PDF p. 54.
    pub(super) const fn rank(self) -> u8 {
        match self {
            | Self::Char | Self::SignedChar | Self::UnsignedChar => 1,
            | Self::Short | Self::UnsignedShort => 2,
            | Self::Int | Self::UnsignedInt => 3,
            | Self::Long | Self::UnsignedLong => 4,
            | Self::LongLong | Self::UnsignedLongLong => 5,
        }
    }

    /// The smallest value of the type.
    pub(super) const fn min(self) -> i128 {
        if self.signed() {
            -(1 << (self.bits() - 1))
        } else {
            0
        }
    }

    /// The largest value of the type.
    pub(super) const fn max(self) -> i128 {
        if self.signed() {
            (1 << (self.bits() - 1)) - 1
        } else {
            (1 << self.bits()) - 1
        }
    }

    /// Whether the type can represent `value`.
    pub(super) const fn fits(self, value: i128) -> bool {
        value >= self.min() && value <= self.max()
    }

    /// The type as C spells it: `unsigned long`.
    pub(super) const fn spelling(self) -> &'static str {
        match self {
            | Self::Char => "char",
            | Self::SignedChar => "signed char",
            | Self::UnsignedChar => "unsigned char",
            | Self::Short => "short",
            | Self::UnsignedShort => "unsigned short",
            | Self::Int => "int",
            | Self::UnsignedInt => "unsigned int",
            | Self::Long => "long",
            | Self::UnsignedLong => "unsigned long",
            | Self::LongLong => "long long",
            | Self::UnsignedLongLong => "unsigned long long",
        }
    }

    /// The range C requires of the type at least, which every
    /// implementation may widen.
    ///
    /// C99: §5.2.4.2.1 paragraph 1, pp. 21-23; PDF pp. 33-35.
    pub(super) const fn minimum(self) -> &'static str {
        match self {
            | Self::Char | Self::SignedChar => "-127 to 127",
            | Self::UnsignedChar => "0 to 255",
            | Self::Short | Self::Int => "-32767 to 32767",
            | Self::UnsignedShort | Self::UnsignedInt => "0 to 65535",
            | Self::Long => "-2147483647 to 2147483647",
            | Self::UnsignedLong => "0 to 4294967295",
            | Self::LongLong => "-9223372036854775807 to 9223372036854775807",
            | Self::UnsignedLongLong => "0 to 18446744073709551615",
        }
    }

    /// Whether this is one of the three character types, whose values the
    /// page shows with their character.
    pub(super) const fn is_char(self) -> bool {
        matches!(self, Self::Char | Self::SignedChar | Self::UnsignedChar)
    }

    /// The type the integer promotions give a value of this type: int for
    /// every type of lower rank than int, whose values all fit in an int on
    /// x86-64, and the type itself otherwise.
    ///
    /// C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55.
    pub(super) const fn promoted(self) -> Self {
        if self.rank() < Self::Int.rank() {
            Self::Int
        } else {
            self
        }
    }

    /// The unsigned type of the same rank.
    pub(super) const fn to_unsigned(self) -> Self {
        match self {
            | Self::Char | Self::SignedChar | Self::UnsignedChar => Self::UnsignedChar,
            | Self::Short | Self::UnsignedShort => Self::UnsignedShort,
            | Self::Int | Self::UnsignedInt => Self::UnsignedInt,
            | Self::Long | Self::UnsignedLong => Self::UnsignedLong,
            | Self::LongLong | Self::UnsignedLongLong => Self::UnsignedLongLong,
        }
    }

    /// The signed type of the same rank (plain char's is signed char).
    pub(super) const fn to_signed(self) -> Self {
        match self {
            | Self::Char | Self::SignedChar | Self::UnsignedChar => Self::SignedChar,
            | Self::Short | Self::UnsignedShort => Self::Short,
            | Self::Int | Self::UnsignedInt => Self::Int,
            | Self::Long | Self::UnsignedLong => Self::Long,
            | Self::LongLong | Self::UnsignedLongLong => Self::LongLong,
        }
    }

    /// `value` converted to this type. A value the type can hold is kept
    /// (§6.3.1.3p1). Otherwise, for an unsigned type, 2^N is added or
    /// subtracted until it fits (§6.3.1.3p2); for a signed type the result
    /// is implementation-defined (§6.3.1.3p3), and bcc, GCC and Clang wrap
    /// it the same way, keeping the low N bits as two's complement.
    ///
    /// C99: §6.3.1.3 paragraphs 1-3, p. 43; PDF p. 55.
    pub(super) const fn convert(self, value: i128) -> i128 {
        if self.fits(value) {
            return value;
        }
        let modulus = 1_i128 << self.bits();
        let reduced = value.rem_euclid(modulus);
        if reduced > self.max() {
            reduced - modulus
        } else {
            reduced
        }
    }

    /// The common type of operands whose promoted types are `self` and
    /// `other`, and why.
    ///
    /// C99: §6.3.1.8 paragraph 1, pp. 44-45; PDF pp. 56-57.
    pub(super) const fn common(self, other: Self) -> (Self, Common) {
        if self as u8 == other as u8 {
            return (self, Common::Same);
        }
        if self.signed() == other.signed() {
            let higher = if self.rank() >= other.rank() {
                self
            } else {
                other
            };
            return (higher, Common::Rank);
        }
        let (unsigned, signed) = if self.signed() {
            (other, self)
        } else {
            (self, other)
        };
        if unsigned.rank() >= signed.rank() {
            (unsigned, Common::Unsigned)
        } else if signed.bits() > unsigned.bits() {
            (signed, Common::Signed)
        } else {
            (signed.to_unsigned(), Common::BothUnsigned)
        }
    }

    /// The type of a decimal integer constant with `suffix` and `value`:
    /// the first of its list that can represent the value.
    ///
    /// C99: §6.4.4.1 paragraph 5, pp. 55-56; PDF pp. 67-68.
    pub(super) fn constant(value: i128, suffix: Suffix) -> Option<Self> {
        let list: &[Self] = match (suffix.unsigned, suffix.long) {
            | (false, 0) => &[Self::Int, Self::Long, Self::LongLong],
            | (true, 0) => &[
                Self::UnsignedInt,
                Self::UnsignedLong,
                Self::UnsignedLongLong,
            ],
            | (false, 1) => &[Self::Long, Self::LongLong],
            | (true, 1) => &[Self::UnsignedLong, Self::UnsignedLongLong],
            | (false, _) => &[Self::LongLong],
            | (true, _) => &[Self::UnsignedLongLong],
        };
        list.iter().copied().find(|ty| ty.fits(value))
    }
}

/// An integer constant's suffix: `u`, `l`, `ll` and their combinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Suffix {
    pub unsigned: bool,
    /// 0 for none, 1 for `l`, 2 for `ll`.
    pub long:     u8,
}

impl Suffix {
    /// The suffix `text` spells, if it is one (C99 §6.4.4.1p1): `u` or `U`,
    /// `l` or `L`, `ll` or `LL`, and `u` with either, in either order.
    pub(super) fn parse(text: &str) -> Option<Self> {
        let lower = text.to_ascii_lowercase();
        let (unsigned, rest) = match lower.strip_prefix('u') {
            | Some(rest) => (true, rest),
            | None => (false, lower.as_str()),
        };
        let (long, rest) = if let Some(rest) = rest.strip_prefix("ll") {
            // `lL` and `Ll` are not suffixes: both letters share a case.
            if text.contains("lL") || text.contains("Ll") {
                return None;
            }
            (2, rest)
        } else if let Some(rest) = rest.strip_prefix('l') {
            (1, rest)
        } else {
            (0, rest)
        };
        match (rest, unsigned) {
            | ("", _) => Some(Self { unsigned, long }),
            | ("u", false) if long > 0 => Some(Self {
                unsigned: true,
                long,
            }),
            | _ => None,
        }
    }
}

/// The text of a conversion by reduction modulo 2^N: `-1 + 2^32 =
/// 4294967295`, `300 - 256 = 44`, or `-5000000000 + 2 × 2^32 = 3589934592`.
pub(super) fn modulo_text(value: i128, converted: i128, bits: u32) -> String {
    let modulus = 1_i128 << bits;
    let times = (converted - value) / modulus;
    let power = if bits <= 8 {
        format!("{modulus}")
    } else {
        format!("2^{bits}")
    };
    let step = match times {
        | 1 => format!(" + {power}"),
        | -1 => format!(" - {power}"),
        | times if times > 0 => format!(" + {times} × {power}"),
        | times => format!(" - {} × {power}", -times),
    };
    format!("{value}{step} = {converted}")
}

/// `2^N` as the narration writes it, with its value: `2^32 (4294967296)`.
pub(super) fn power_text(bits: u32) -> String {
    format!("2^{bits} ({})", 1_i128 << bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_types_follow_rank_and_signedness() {
        use Integer::*;
        assert_eq!(Int.common(UnsignedInt), (UnsignedInt, Common::Unsigned));
        assert_eq!(Long.common(UnsignedInt), (Long, Common::Signed));
        assert_eq!(
            LongLong.common(UnsignedLong),
            (UnsignedLongLong, Common::BothUnsigned)
        );
        assert_eq!(Int.common(Long), (Long, Common::Rank));
        assert_eq!(UnsignedInt.common(UnsignedInt), (UnsignedInt, Common::Same));
    }

    #[test]
    fn conversions_reduce_modulo_two_to_the_width() {
        assert_eq!(Integer::UnsignedInt.convert(-1), 4_294_967_295);
        assert_eq!(Integer::Char.convert(300), 44);
        assert_eq!(Integer::Char.convert(200), -56);
        assert_eq!(Integer::UnsignedChar.convert(-1), 255);
        assert_eq!(Integer::Int.convert(4_294_967_295), -1);
        assert_eq!(
            Integer::UnsignedLongLong.convert(-1),
            18_446_744_073_709_551_615
        );
        assert_eq!(modulo_text(-1, 4_294_967_295, 32), "-1 + 2^32 = 4294967295");
        assert_eq!(modulo_text(300, 44, 8), "300 - 256 = 44");
    }

    #[test]
    fn constants_take_the_first_type_that_holds_them() {
        let none = Suffix::default();
        let unsigned = Suffix::parse("u").unwrap();
        assert_eq!(Integer::constant(1, none), Some(Integer::Int));
        assert_eq!(Integer::constant(2_147_483_648, none), Some(Integer::Long));
        assert_eq!(Integer::constant(1, unsigned), Some(Integer::UnsignedInt));
        assert_eq!(
            Integer::constant(4_294_967_296, unsigned),
            Some(Integer::UnsignedLong)
        );
        assert_eq!(Suffix::parse("ULL").map(|s| s.long), Some(2));
        assert_eq!(Suffix::parse("lu").map(|s| s.unsigned), Some(true));
        assert_eq!(Suffix::parse("lL"), None);
        assert_eq!(Suffix::parse("x"), None);
    }
}
