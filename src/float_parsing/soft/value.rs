//! Unpacked floating values and the one rounding step every operation ends
//! with: the exact result, rounded to nearest with ties to even, in one of
//! the three binary formats the bridge computes in.

/// A binary floating format: IEEE binary32 and binary64 with a hidden
/// integer bit, and x87 double-extended precision with an explicit one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Format {
    /// Significand bits, counting the integer bit.
    pub(super) precision:    u32,
    /// The exponent of the least normal value.
    pub(super) min_exponent: i32,
    /// The exponent of the greatest finite value.
    pub(super) max_exponent: i32,
    /// Whether the x87 rules choose between NaN operands, rather than the
    /// SSE ones.
    pub(super) x87:          bool,
}

/// C `float` and SSE single-precision arithmetic.
pub(super) const F32: Format = Format {
    precision:    24,
    min_exponent: -126,
    max_exponent: 127,
    x87:          false,
};

/// C `double` and SSE double-precision arithmetic.
pub(super) const F64: Format = Format {
    precision:    53,
    min_exponent: -1022,
    max_exponent: 1023,
    x87:          false,
};

/// The x86-64 `long double`, computed with the x87 precision control set to
/// 64 bits, as the bridge sets it.
pub(super) const X87: Format = Format {
    precision:    64,
    min_exponent: -16382,
    max_exponent: 16383,
    x87:          true,
};

/// A floating value with its sign. Every finite nonzero value is kept
/// normalized, whatever its encoding, so arithmetic sees one form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Soft {
    pub(super) negative: bool,
    pub(super) value:    Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Value {
    Zero,
    /// `significand × 2^(exponent - 63)`, with bit 63 of the significand set.
    /// A subnormal value has an exponent below the format's least.
    Finite {
        exponent:    i32,
        significand: u64,
    },
    Infinity,
    /// The fraction bits aligned below an x87 integer bit: bit 62 is the
    /// quiet bit and the payload follows it.
    Nan {
        fraction: u64,
    },
    /// An x87 encoding the FPU refuses as an operand (a pseudo-NaN,
    /// pseudo-infinity or unnormal), kept to be stored back unchanged.
    Unsupported {
        exponent:    u16,
        significand: u64,
    },
}

/// The quiet bit of [`Value::Nan`]'s fraction.
pub(super) const QUIET: u64 = 1 << 62;

impl Soft {
    /// The default NaN both units produce for an invalid operation: the x87
    /// "real indefinite" and SSE's default quiet NaN, negative with only
    /// the quiet bit set.
    pub(super) const DEFAULT_NAN: Self = Self {
        negative: true,
        value:    Value::Nan { fraction: QUIET },
    };

    pub(super) const fn zero(negative: bool) -> Self {
        Self {
            negative,
            value: Value::Zero,
        }
    }

    pub(super) const fn infinity(negative: bool) -> Self {
        Self {
            negative,
            value: Value::Infinity,
        }
    }

    pub(super) const fn one(negative: bool) -> Self {
        Self {
            negative,
            value: Value::Finite {
                exponent:    0,
                significand: 1 << 63,
            },
        }
    }

    /// Whether comparisons treat the value as unordered: a NaN, or an x87
    /// encoding the FPU refuses.
    pub(super) fn is_nan(self) -> bool {
        matches!(self.value, Value::Nan { .. } | Value::Unsupported { .. })
    }

    pub(super) fn is_infinite(self) -> bool {
        self.value == Value::Infinity
    }

    pub(super) fn is_finite(self) -> bool {
        matches!(self.value, Value::Zero | Value::Finite { .. })
    }

    pub(super) fn is_zero(self) -> bool {
        self.value == Value::Zero
    }

    pub(super) fn negated(self) -> Self {
        Self {
            negative: !self.negative,
            ..self
        }
    }

    pub(super) fn abs(self) -> Self {
        Self {
            negative: false,
            ..self
        }
    }

    /// `copysign(self, sign)`.
    pub(super) fn with_sign_of(self, sign: Self) -> Self {
        Self {
            negative: sign.negative,
            ..self
        }
    }

    /// Converts the value to `format`, rounding a finite value and quieting
    /// a NaN whose payload loses its low bits, as the x87 stores and loads
    /// and SSE conversions do. A refused x87 encoding is an invalid operand.
    pub(super) fn convert(self, format: Format) -> Self {
        match self.value {
            | Value::Finite {
                exponent,
                significand,
            } => round(
                self.negative,
                u128::from(significand),
                exponent - 63,
                false,
                format,
            ),
            | Value::Nan { fraction } => Self {
                negative: self.negative,
                value:    Value::Nan {
                    fraction: (fraction | QUIET) & fraction_mask(format),
                },
            },
            | Value::Unsupported { .. } => Self::DEFAULT_NAN,
            | Value::Zero | Value::Infinity => self,
        }
    }

    /// Decodes a binary32 or binary64 encoding.
    pub(super) fn from_ieee(bits: u64, format: Format) -> Self {
        let fraction_bits = format.precision - 1;
        let exponent_bits = if format == F32 { 8 } else { 11 };
        let negative = bits >> (fraction_bits + exponent_bits) & 1 != 0;
        let biased = (bits >> fraction_bits) & ((1 << exponent_bits) - 1);
        let fraction = bits & ((1 << fraction_bits) - 1);
        let value = if biased == (1 << exponent_bits) - 1 {
            if fraction == 0 {
                Value::Infinity
            } else {
                Value::Nan {
                    fraction: fraction << (64 - format.precision),
                }
            }
        } else if biased == 0 {
            if fraction == 0 {
                Value::Zero
            } else {
                let shift = fraction.leading_zeros();
                Value::Finite {
                    exponent:    format.min_exponent - fraction_bits as i32 + 63 - shift as i32,
                    significand: fraction << shift,
                }
            }
        } else {
            Value::Finite {
                exponent:    biased as i32 + format.min_exponent - 1,
                significand: ((1 << fraction_bits) | fraction) << (64 - format.precision),
            }
        };
        Self { negative, value }
    }

    /// Encodes the value, which must be representable in `format`.
    pub(super) fn to_ieee(self, format: Format) -> u64 {
        let fraction_bits = format.precision - 1;
        let exponent_bits = if format == F32 { 8 } else { 11 };
        let all_ones = (1_u64 << exponent_bits) - 1;
        let (biased, fraction) = match self.value {
            | Value::Zero => (0, 0),
            | Value::Infinity => (all_ones, 0),
            | Value::Nan { fraction } => (all_ones, fraction >> (64 - format.precision)),
            | Value::Unsupported { .. } => return Self::DEFAULT_NAN.to_ieee(format),
            | Value::Finite {
                exponent,
                significand,
            } =>
                if exponent >= format.min_exponent {
                    (
                        (exponent - format.min_exponent + 1) as u64,
                        (significand >> (64 - format.precision)) & ((1 << fraction_bits) - 1),
                    )
                } else {
                    let shift = 64 - format.precision + (format.min_exponent - exponent) as u32;
                    (0, significand.checked_shr(shift).unwrap_or(0))
                },
        };
        (u64::from(self.negative) << (fraction_bits + exponent_bits))
            | (biased << fraction_bits)
            | fraction
    }

    /// Decodes the ten value bytes of an x87 double-extended encoding.
    pub(super) fn from_x87(bytes: [u8; 10]) -> Self {
        let mut low = [0; 8];
        low.copy_from_slice(&bytes[..8]);
        let significand = u64::from_le_bytes(low);
        let top = u16::from_le_bytes([bytes[8], bytes[9]]);
        let negative = top >> 15 != 0;
        let exponent = top & 0x7FFF;
        let integer = significand >> 63 != 0;
        let value = match exponent {
            | 0x7FFF if !integer => Value::Unsupported {
                exponent,
                significand,
            },
            | 0x7FFF if significand << 1 == 0 => Value::Infinity,
            | 0x7FFF => Value::Nan {
                fraction: significand & !(1 << 63),
            },
            | 0 if significand == 0 => Value::Zero,
            // A pseudo-denormal has the least normal exponent's weight.
            | 0 if integer => Value::Finite {
                exponent: X87.min_exponent,
                significand,
            },
            | 0 => {
                let shift = significand.leading_zeros();
                Value::Finite {
                    exponent:    X87.min_exponent - shift as i32,
                    significand: significand << shift,
                }
            },
            | _ if !integer => Value::Unsupported {
                exponent,
                significand,
            },
            | _ => Value::Finite {
                exponent: i32::from(exponent) - 16383,
                significand,
            },
        };
        Self { negative, value }
    }

    /// Encodes the value, which must be representable as an x87
    /// double-extended value, in its ten value bytes.
    pub(super) fn to_x87(self) -> [u8; 10] {
        let (exponent, significand) = match self.value {
            | Value::Zero => (0, 0),
            | Value::Infinity => (0x7FFF, 1 << 63),
            | Value::Nan { fraction } => (0x7FFF, (1 << 63) | fraction),
            | Value::Unsupported {
                exponent,
                significand,
            } => (exponent, significand),
            | Value::Finite {
                exponent,
                significand,
            } =>
                if exponent >= X87.min_exponent {
                    ((exponent + 16383) as u16, significand)
                } else {
                    let shift = (X87.min_exponent - exponent) as u32;
                    (0, significand.checked_shr(shift).unwrap_or(0))
                },
        };
        let mut bytes = [0; 10];
        bytes[..8].copy_from_slice(&significand.to_le_bytes());
        let top = exponent | if self.negative { 0x8000 } else { 0 };
        bytes[8..].copy_from_slice(&top.to_le_bytes());
        bytes
    }
}

/// The fraction bits `format` keeps of a NaN's aligned fraction.
fn fraction_mask(format: Format) -> u64 {
    !(1_u64 << 63) & !((1_u64 << (64 - format.precision)) - 1)
}

/// Rounds `significand × 2^exponent`, plus a nonzero amount below its last
/// bit when `sticky` is set, to nearest with ties to even in `format`,
/// gradually underflowing to subnormals and overflowing to infinity. With
/// `sticky`, the significand must carry at least two bits below the
/// format's precision, so the rounding decision is exact.
/// IEC 60559 rounding: C99 Annex F.7.2, p. 456; PDF p. 468.
pub(super) fn round(
    negative: bool,
    significand: u128,
    exponent: i32,
    sticky: bool,
    format: Format,
) -> Soft {
    if significand == 0 {
        return Soft::zero(negative);
    }
    let precision = format.precision as i32;
    let length = 128 - significand.leading_zeros() as i32;
    let magnitude = exponent + length - 1;
    // The weight of the result's last significand bit.
    let quantum = magnitude.max(format.min_exponent) - (precision - 1);
    let shift = quantum - exponent;
    let mut kept = if shift <= 0 {
        debug_assert!(!sticky || shift >= 2, "rounding lacks guard bits");
        significand << -shift
    } else if shift >= 128 {
        0
    } else {
        significand >> shift
    };
    if shift > 0 {
        let half = if shift > 128 {
            0
        } else {
            1_u128 << (shift - 1)
        };
        let rest = if shift >= 128 {
            significand
        } else {
            significand & ((1_u128 << shift) - 1)
        };
        let up = if shift > 128 {
            false
        } else {
            rest > half || (rest == half && (sticky || kept & 1 != 0))
        };
        kept += u128::from(up);
    }
    if kept == 0 {
        return Soft::zero(negative);
    }
    let length = 128 - kept.leading_zeros() as i32;
    let magnitude = quantum + length - 1;
    if magnitude > format.max_exponent {
        return Soft::infinity(negative);
    }
    Soft {
        negative,
        value: Value::Finite {
            exponent:    magnitude,
            significand: (kept << (128 - length) >> 64) as u64,
        },
    }
}
