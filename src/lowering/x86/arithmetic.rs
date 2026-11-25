//! Lane arithmetic: alternating, horizontal and dot-product sums, minimum
//! and maximum, multiplications keeping the high half or widening, averages,
//! sums of absolute differences, packs with saturation, sign transfer, the
//! shifts, and the carry chains of ADCX and SBB. Integer lanes wrap unless
//! the instruction saturates. Unless said otherwise, a 256-bit form applies
//! the 128-bit rule to each 128-bit half (Intel SDM Vol. 2, each
//! instruction's "Operation"; Intel Intrinsics Guide).

use super::{
    super::{
        FunctionLowerer,
        LoweringError,
    },
    Operands,
    Produced,
    int_of,
};
use crate::{
    builtins::BuiltinId,
    ir::{
        FloatCC,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::SourceVectors,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Expands the arithmetic builtin `id`, if it is one.
    #[expect(
        clippy::many_single_char_names,
        reason = "the operands and lanes keep the SDM's names"
    )]
    pub(super) fn lane_arithmetic(
        &mut self,
        id: BuiltinId,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Option<Produced>, LoweringError> {
        use BuiltinId as B;
        let a = operands.get(0);
        let b = operands.values.get(1).copied().unwrap_or(a);
        let vector = |object: Result<Value, LoweringError>| object.map(Produced::Vector);
        let produced = match id {
            // VADDSUBPS, VADDSUBPD: even lanes subtract, odd lanes add.
            | B::X86Addsubps256 | B::X86Addsubpd256 => {
                let (ty, count) = if id == B::X86Addsubps256 {
                    (Type::F32, 8)
                } else {
                    (Type::F64, 4)
                };
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let (x, y) = (this.lane(a, ty, i), this.lane(b, ty, i));
                    let opcode = if i % 2 == 0 {
                        Opcode::Fsub
                    } else {
                        Opcode::Fadd
                    };
                    this.bin(opcode, x, y)
                }))?
            },
            // VHADDPS, VHSUBPS, VHADDPD, VHSUBPD and PHADDW, PHADDD, PHSUBW,
            // PHSUBD, PHADDSW, PHSUBSW: the first half of each 128-bit half
            // combines adjacent pairs of the first operand, the second half
            // those of the second.
            | B::X86Haddps256
            | B::X86Hsubps256
            | B::X86Haddpd256
            | B::X86Hsubpd256
            | B::X86Phaddw256
            | B::X86Phaddd256
            | B::X86Phaddsw256
            | B::X86Phsubw256
            | B::X86Phsubd256
            | B::X86Phsubsw256 => {
                let (ty, count) = match id {
                    | B::X86Haddps256 | B::X86Hsubps256 => (Type::F32, 8),
                    | B::X86Haddpd256 | B::X86Hsubpd256 => (Type::F64, 4),
                    | B::X86Phaddd256 | B::X86Phsubd256 => (Type::I32, 8),
                    | _ => (Type::I16, 16),
                };
                let subtract = matches!(
                    id,
                    B::X86Hsubps256
                        | B::X86Hsubpd256
                        | B::X86Phsubw256
                        | B::X86Phsubd256
                        | B::X86Phsubsw256
                );
                let saturate = matches!(id, B::X86Phaddsw256 | B::X86Phsubsw256);
                let per_half = 16 / u64::from(ty.bytes());
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let (half, k) = (i - i % per_half, i % per_half);
                    let (from, pair) = if k < per_half / 2 {
                        (a, k)
                    } else {
                        (b, k - per_half / 2)
                    };
                    let x = this.lane(from, ty, half + 2 * pair);
                    let y = this.lane(from, ty, half + 2 * pair + 1);
                    if ty.is_float() {
                        this.bin(if subtract { Opcode::Fsub } else { Opcode::Fadd }, x, y)
                    } else if saturate {
                        let (x, y) = (
                            this.extend(x, true, Type::I32),
                            this.extend(y, true, Type::I32),
                        );
                        let sum =
                            this.bin(if subtract { Opcode::Isub } else { Opcode::Iadd }, x, y);
                        this.saturate(sum, -32768, 32767, Type::I16)
                    } else {
                        this.bin(if subtract { Opcode::Isub } else { Opcode::Iadd }, x, y)
                    }
                }))?
            },
            // VMINPS and kin: the first operand if it is less (greater) than
            // the second, else the second, which a NaN or equal zeros give.
            | B::X86Minps256 | B::X86Minpd256 | B::X86Maxps256 | B::X86Maxpd256 => {
                let (ty, count) = if matches!(id, B::X86Minps256 | B::X86Maxps256) {
                    (Type::F32, 8)
                } else {
                    (Type::F64, 4)
                };
                let cc = if matches!(id, B::X86Minps256 | B::X86Minpd256) {
                    FloatCC::Olt
                } else {
                    FloatCC::Ogt
                };
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let (x, y) = (this.lane(a, ty, i), this.lane(b, ty, i));
                    let first = this.fcmp(cc, x, y);
                    this.choose(first, x, y)
                }))?
            },
            // VCVTPD2PS: each lane rounded to `float` in MXCSR's default
            // mode, as `fptrunc` rounds.
            | B::X86Cvtpd2ps256 => vector(self.fill_lanes(Type::F32, 4, source, |this, i| {
                let x = this.lane(a, Type::F64, i);
                this.convert(Opcode::Fptrunc, Type::F32, x)
            }))?,
            // PACKSSWB, PACKUSWB, PACKSSDW, PACKUSDW: the lanes of each half of
            // the first operand, then of the second, narrowed with signed or
            // unsigned saturation.
            | B::X86Packsswb256 | B::X86Packuswb256 | B::X86Packssdw256 | B::X86Packusdw256 => {
                let (from, to) = if matches!(id, B::X86Packsswb256 | B::X86Packuswb256) {
                    (Type::I16, Type::I8)
                } else {
                    (Type::I32, Type::I16)
                };
                let unsigned = matches!(id, B::X86Packuswb256 | B::X86Packusdw256);
                let bits = to.bits();
                let (low, high) = if unsigned {
                    (0, (1_i128 << bits) - 1)
                } else {
                    (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
                };
                let per_half = 16 / u64::from(to.bytes());
                let inputs = per_half / 2;
                vector(
                    self.fill_lanes(to, 32 / u64::from(to.bytes()), source, |this, i| {
                        let (half, k) = (i / per_half, i % per_half);
                        let (vector, k) = if k < inputs { (a, k) } else { (b, k - inputs) };
                        let x = this.lane(vector, from, half * inputs + k);
                        this.saturate(x, low, high, to)
                    }),
                )?
            },
            // PAVGB, PAVGW: the unsigned average rounded up.
            | B::X86Pavgb256 | B::X86Pavgw256 => {
                let (ty, count) = if id == B::X86Pavgb256 {
                    (Type::I8, 32)
                } else {
                    (Type::I16, 16)
                };
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let x = this.lane(a, ty, i);
                    let y = this.lane(b, ty, i);
                    let (x, y) = (
                        this.extend(x, false, Type::I32),
                        this.extend(y, false, Type::I32),
                    );
                    let sum = this.bin(Opcode::Iadd, x, y);
                    let one = this.iconst(Type::I32, 1);
                    let sum = this.bin(Opcode::Iadd, sum, one);
                    let average = this.bin(Opcode::Lshr, sum, one);
                    this.extend(average, false, ty)
                }))?
            },
            // PMADDWD: adjacent signed word products summed into a doubleword,
            // which wraps only for four -32768s.
            | B::X86Pmaddwd256 => vector(self.fill_lanes(Type::I32, 8, source, |this, i| {
                let mut sum = this.iconst(Type::I32, 0);
                for k in 0..2 {
                    let x = this.lane(a, Type::I16, 2 * i + k);
                    let y = this.lane(b, Type::I16, 2 * i + k);
                    let (x, y) = (
                        this.extend(x, true, Type::I32),
                        this.extend(y, true, Type::I32),
                    );
                    let product = this.bin(Opcode::Imul, x, y);
                    sum = this.bin(Opcode::Iadd, sum, product);
                }
                sum
            }))?,
            // PMADDUBSW: unsigned bytes of the first operand times signed
            // bytes of the second, adjacent products summed with signed
            // saturation.
            | B::X86Pmaddubsw256 => vector(self.fill_lanes(Type::I16, 16, source, |this, i| {
                let mut sum = this.iconst(Type::I32, 0);
                for k in 0..2 {
                    let x = this.lane(a, Type::I8, 2 * i + k);
                    let y = this.lane(b, Type::I8, 2 * i + k);
                    let (x, y) = (
                        this.extend(x, false, Type::I32),
                        this.extend(y, true, Type::I32),
                    );
                    let product = this.bin(Opcode::Imul, x, y);
                    sum = this.bin(Opcode::Iadd, sum, product);
                }
                this.saturate(sum, -32768, 32767, Type::I16)
            }))?,
            // PMULHW, PMULHUW: the high word of the signed (unsigned)
            // product; PMULHRSW: the product scaled by 2^-15 and rounded.
            | B::X86Pmulhw256 | B::X86Pmulhuw256 | B::X86Pmulhrsw256 => {
                let signed = id != B::X86Pmulhuw256;
                vector(self.fill_lanes(Type::I16, 16, source, |this, i| {
                    let x = this.lane(a, Type::I16, i);
                    let y = this.lane(b, Type::I16, i);
                    let (x, y) = (
                        this.extend(x, signed, Type::I32),
                        this.extend(y, signed, Type::I32),
                    );
                    let product = this.bin(Opcode::Imul, x, y);
                    let high = if id == B::X86Pmulhrsw256 {
                        let fourteen = this.iconst(Type::I32, 14);
                        let scaled = this.bin(Opcode::Ashr, product, fourteen);
                        let one = this.iconst(Type::I32, 1);
                        let rounded = this.bin(Opcode::Iadd, scaled, one);
                        this.bin(Opcode::Ashr, rounded, one)
                    } else {
                        let sixteen = this.iconst(Type::I32, 16);
                        this.bin(
                            if signed { Opcode::Ashr } else { Opcode::Lshr },
                            product,
                            sixteen,
                        )
                    };
                    this.extend(high, false, Type::I16)
                }))?
            },
            // PMULDQ, PMULUDQ: the even doublewords widened and multiplied.
            | B::X86Pmuldq256 | B::X86Pmuludq256 => {
                let signed = id == B::X86Pmuldq256;
                vector(self.fill_lanes(Type::I64, 4, source, |this, i| {
                    let x = this.lane(a, Type::I32, 2 * i);
                    let y = this.lane(b, Type::I32, 2 * i);
                    let (x, y) = (
                        this.extend(x, signed, Type::I64),
                        this.extend(y, signed, Type::I64),
                    );
                    this.bin(Opcode::Imul, x, y)
                }))?
            },
            // PSADBW: each quadword the sum of the eight absolute differences
            // of its unsigned bytes.
            | B::X86Psadbw256 => vector(self.fill_lanes(Type::I64, 4, source, |this, i| {
                let mut sum = this.iconst(Type::I64, 0);
                for k in 0..8 {
                    let difference = this.byte_difference(a, 8 * i + k, b, 8 * i + k);
                    let difference = this.extend(difference, false, Type::I64);
                    sum = this.bin(Opcode::Iadd, sum, difference);
                }
                sum
            }))?,
            // MPSADBW: word `i` sums the absolute differences of the four
            // bytes of the first operand from byte `4 * imm[2] + i` and the
            // four of the second from byte `4 * imm[1:0]`; the 256-bit form's
            // high half takes bits 3-5.
            | B::X86Mpsadbw128 | B::X86Mpsadbw256 => {
                let imm = operands.imm(2)?;
                let count = if id == B::X86Mpsadbw128 { 8 } else { 16 };
                vector(self.fill_lanes(Type::I16, count, source, |this, i| {
                    let (half, k) = (i / 8, i % 8);
                    let control = imm >> (3 * half);
                    let first = 16 * half + 4 * (control >> 2 & 1) + k;
                    let second = 16 * half + 4 * (control & 3);
                    let mut sum = this.iconst(Type::I32, 0);
                    for j in 0..4 {
                        let difference = this.byte_difference(a, first + j, b, second + j);
                        sum = this.bin(Opcode::Iadd, sum, difference);
                    }
                    this.extend(sum, false, Type::I16)
                }))?
            },
            // PSIGNB, PSIGNW, PSIGND: the first operand's lane negated where
            // the second's is negative and zeroed where it is zero.
            | B::X86Psignb256 | B::X86Psignw256 | B::X86Psignd256 => {
                let (ty, count) = match id {
                    | B::X86Psignb256 => (Type::I8, 32),
                    | B::X86Psignw256 => (Type::I16, 16),
                    | _ => (Type::I32, 8),
                };
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let (x, y) = (this.lane(a, ty, i), this.lane(b, ty, i));
                    let zero = this.iconst(ty, 0);
                    let negated = this.bin(Opcode::Isub, zero, x);
                    let negative = this.sign_set(y);
                    let signed = this.choose(negative, negated, x);
                    let is_zero = this.icmp(IntCC::Eq, y, zero);
                    this.choose(is_zero, zero, signed)
                }))?
            },
            // PSLLW and kin by the low quadword of a vector, and PSLLWI and
            // kin by an `int` read as unsigned: a count past the lane width
            // clears the lane, or fills it with its sign.
            | B::X86Psllw256
            | B::X86Pslld256
            | B::X86Psllq256
            | B::X86Psrlw256
            | B::X86Psrld256
            | B::X86Psrlq256
            | B::X86Psraw256
            | B::X86Psrad256
            | B::X86Psllwi256
            | B::X86Pslldi256
            | B::X86Psllqi256
            | B::X86Psrlwi256
            | B::X86Psrldi256
            | B::X86Psrlqi256
            | B::X86Psrawi256
            | B::X86Psradi256 => {
                let (ty, opcode) = match id {
                    | B::X86Psllw256 | B::X86Psllwi256 => (Type::I16, Opcode::Shl),
                    | B::X86Pslld256 | B::X86Pslldi256 => (Type::I32, Opcode::Shl),
                    | B::X86Psllq256 | B::X86Psllqi256 => (Type::I64, Opcode::Shl),
                    | B::X86Psrlw256 | B::X86Psrlwi256 => (Type::I16, Opcode::Lshr),
                    | B::X86Psrld256 | B::X86Psrldi256 => (Type::I32, Opcode::Lshr),
                    | B::X86Psrlq256 | B::X86Psrlqi256 => (Type::I64, Opcode::Lshr),
                    | B::X86Psraw256 | B::X86Psrawi256 => (Type::I16, Opcode::Ashr),
                    | _ => (Type::I32, Opcode::Ashr),
                };
                let by_vector = matches!(
                    id,
                    B::X86Psllw256
                        | B::X86Pslld256
                        | B::X86Psllq256
                        | B::X86Psrlw256
                        | B::X86Psrld256
                        | B::X86Psrlq256
                        | B::X86Psraw256
                        | B::X86Psrad256
                );
                let count = if by_vector {
                    self.lane(b, Type::I64, 0)
                } else {
                    self.extend(b, false, Type::I64)
                };
                vector(
                    self.fill_lanes(ty, 32 / u64::from(ty.bytes()), source, |this, i| {
                        let x = this.lane(a, ty, i);
                        this.shift_lane(opcode, x, count)
                    }),
                )?
            },
            // VPSLLVD and kin: each lane by the count in the same lane.
            | B::X86Psllv2di
            | B::X86Psllv4di
            | B::X86Psllv4si
            | B::X86Psllv8si
            | B::X86Psrlv2di
            | B::X86Psrlv4di
            | B::X86Psrlv4si
            | B::X86Psrlv8si
            | B::X86Psrav4si
            | B::X86Psrav8si => {
                let (ty, count) = match id {
                    | B::X86Psllv2di | B::X86Psrlv2di => (Type::I64, 2),
                    | B::X86Psllv4di | B::X86Psrlv4di => (Type::I64, 4),
                    | B::X86Psllv4si | B::X86Psrlv4si | B::X86Psrav4si => (Type::I32, 4),
                    | _ => (Type::I32, 8),
                };
                let opcode = match id {
                    | B::X86Psllv2di | B::X86Psllv4di | B::X86Psllv4si | B::X86Psllv8si =>
                        Opcode::Shl,
                    | B::X86Psrav4si | B::X86Psrav8si => Opcode::Ashr,
                    | _ => Opcode::Lshr,
                };
                vector(self.fill_lanes(ty, count, source, |this, i| {
                    let x = this.lane(a, ty, i);
                    let y = this.lane(b, ty, i);
                    let y = this.extend(y, false, Type::I64);
                    this.shift_lane(opcode, x, y)
                }))?
            },
            // DPPS, DPPD: the products the immediate's high bits select (the
            // others +0.0) summed pairwise, then stored in the lanes its low
            // bits select (the others +0.0).
            | B::X86Dpps | B::X86Dpps256 | B::X86Dppd => {
                let imm = operands.imm(2)?;
                let (ty, per_half) = if id == B::X86Dppd {
                    (Type::F64, 2)
                } else {
                    (Type::F32, 4)
                };
                let halves = if id == B::X86Dpps256 { 2 } else { 1 };
                let object = self.target_vector(16 * halves, source)?;
                for half in 0..halves {
                    let base = half * per_half;
                    let zero = self.fconst(ty, 0);
                    let mut lanes = [zero; 4];
                    for (k, product) in (0..per_half).zip(lanes.iter_mut()) {
                        if imm >> (4 + k) & 1 != 0 {
                            let x = self.lane(a, ty, base + k);
                            let y = self.lane(b, ty, base + k);
                            *product = self.bin(Opcode::Fmul, x, y);
                        }
                    }
                    let first = self.bin(Opcode::Fadd, lanes[0], lanes[1]);
                    let sum = if per_half == 4 {
                        let second = self.bin(Opcode::Fadd, lanes[2], lanes[3]);
                        self.bin(Opcode::Fadd, first, second)
                    } else {
                        first
                    };
                    for k in 0..per_half {
                        let value = if imm >> k & 1 != 0 { sum } else { zero };
                        self.set_lane(value, object, base + k);
                    }
                }
                Produced::Vector(object)
            },
            // ADCX and SBB: `a + b + carry` (`a - b - borrow`) with the
            // carry-in any nonzero `c`; the low half is stored and the carry
            // or borrow out returned.
            | B::X86AddcarryxU32 | B::X86AddcarryxU64 | B::X86SubborrowU32 | B::X86SubborrowU64 => {
                let (c, x, y, out) = (
                    operands.get(0),
                    operands.get(1),
                    operands.get(2),
                    operands.get(3),
                );
                let ty = self.draft.value_type(x);
                let wide = int_of(2 * u64::from(ty.bytes()));
                let zero = self.iconst(Type::I8, 0);
                let carry = self.icmp(IntCC::Ne, c, zero);
                let carry = self.extend(carry, false, wide);
                let (x, y) = (self.extend(x, false, wide), self.extend(y, false, wide));
                let total = if matches!(id, B::X86AddcarryxU32 | B::X86AddcarryxU64) {
                    let sum = self.bin(Opcode::Iadd, x, y);
                    self.bin(Opcode::Iadd, sum, carry)
                } else {
                    let difference = self.bin(Opcode::Isub, x, y);
                    self.bin(Opcode::Isub, difference, carry)
                };
                let low = self.extend(total, false, ty);
                self.set_lane(low, out, 0);
                let width = self.iconst(wide, i128::from(ty.bits()));
                let high = self.bin(Opcode::Lshr, total, width);
                let one = self.iconst(wide, 1);
                let bit = self.bin(Opcode::And, high, one);
                Produced::Scalar(self.extend(bit, false, Type::I8))
            },
            | _ => return Ok(None),
        };
        Ok(Some(produced))
    }

    /// `value`, a signed integer, clamped to `[low, high]` and truncated to
    /// `to`.
    pub(in crate::lowering) fn saturate(
        &mut self,
        value: Value,
        low: i128,
        high: i128,
        to: Type,
    ) -> Value {
        let ty = self.draft.value_type(value);
        let (low, high) = (self.iconst(ty, low), self.iconst(ty, high));
        let below = self.icmp(IntCC::Slt, value, low);
        let value = self.choose(below, low, value);
        let above = self.icmp(IntCC::Sgt, value, high);
        let value = self.choose(above, high, value);
        self.extend(value, true, to)
    }

    /// `x` shifted by the unsigned `i64` `count`: a count of the lane width
    /// or more gives zero, or for an arithmetic shift the sign in every bit.
    fn shift_lane(&mut self, opcode: Opcode, x: Value, count: Value) -> Value {
        let ty = self.draft.value_type(x);
        let last = self.iconst(Type::I64, i128::from(ty.bits() - 1));
        let beyond = self.icmp(IntCC::Ugt, count, last);
        let amount = if opcode == Opcode::Ashr {
            self.choose(beyond, last, count)
        } else {
            self.bin(Opcode::And, count, last)
        };
        let amount = self.extend(amount, false, ty);
        let shifted = self.bin(opcode, x, amount);
        if opcode == Opcode::Ashr {
            shifted
        } else {
            let zero = self.iconst(ty, 0);
            self.choose(beyond, zero, shifted)
        }
    }

    /// `|a[i] - b[j]|` of unsigned bytes, as an `i32`.
    #[expect(
        clippy::many_single_char_names,
        reason = "the operands and lanes keep the SDM's names"
    )]
    fn byte_difference(&mut self, a: Value, i: u64, b: Value, j: u64) -> Value {
        let x = self.lane(a, Type::I8, i);
        let y = self.lane(b, Type::I8, j);
        let (x, y) = (
            self.extend(x, false, Type::I32),
            self.extend(y, false, Type::I32),
        );
        let difference = self.bin(Opcode::Isub, x, y);
        let negated = self.bin(Opcode::Isub, y, x);
        let negative = self.sign_set(difference);
        self.choose(negative, negated, difference)
    }
}
