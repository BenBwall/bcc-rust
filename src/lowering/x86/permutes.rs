//! Lane movement: shuffles, blends, alignments, byte shifts, inserts and
//! extracts selected by an immediate, and the permutes whose indices are
//! another vector's lanes. Lanes move as integers of their width. Unless
//! said otherwise, a 256-bit form applies the 128-bit rule to each 128-bit
//! half, with the same immediate (Intel SDM Vol. 2, each instruction's
//! "Operation"; Intel Intrinsics Guide).

use super::{
    super::{
        FunctionLowerer,
        LoweringError,
    },
    Operands,
    Produced,
};
use crate::{
    builtins::BuiltinId,
    ir::{
        Align,
        InstData,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::SourceVectors,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Expands the lane-movement builtin `id`, if it is one.
    pub(super) fn permute(
        &mut self,
        id: BuiltinId,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Option<Produced>, LoweringError> {
        use BuiltinId as B;
        let a = operands.get(0);
        let sources = [a, operands.values.get(1).copied().unwrap_or(a)];
        let picked = |this: &mut Self, ty: Type, count: u64, pick: &dyn Fn(u64) -> Pick| {
            this.picked_lanes(&sources, ty, count, source, pick)
                .map(Produced::Vector)
        };
        let produced = match id {
            // BLENDPS, BLENDPD, PBLENDD, PBLENDW: bit `i` of the immediate
            // takes lane `i` from the second operand; PBLENDW's 256-bit form
            // reuses the 8 bits in each half.
            | B::X86Blendpd
            | B::X86Blendpd256
            | B::X86Blendps
            | B::X86Blendps256
            | B::X86Pblendd128
            | B::X86Pblendd256
            | B::X86Pblendw128
            | B::X86Pblendw256 => {
                let (ty, count) = match id {
                    | B::X86Blendpd => (Type::I64, 2),
                    | B::X86Blendpd256 => (Type::I64, 4),
                    | B::X86Blendps | B::X86Pblendd128 => (Type::I32, 4),
                    | B::X86Blendps256 | B::X86Pblendd256 => (Type::I32, 8),
                    | B::X86Pblendw128 => (Type::I16, 8),
                    | _ => (Type::I16, 16),
                };
                let imm = operands.imm(2)?;
                picked(self, ty, count, &|i| {
                    Pick::Lane(usize::from(imm >> (i % 8) & 1 != 0), i)
                })?
            },
            // SHUFPS: lanes 0-1 of each half from the first operand, 2-3
            // from the second, each chosen by two bits of the immediate.
            | B::X86Shufps | B::X86Shufps256 => {
                let imm = operands.imm(2)?;
                let count = if id == B::X86Shufps { 4 } else { 8 };
                picked(self, Type::I32, count, &|i| {
                    let k = i % 4;
                    Pick::Lane(usize::from(k >= 2), i - k + (imm >> (2 * k) & 3))
                })?
            },
            // SHUFPD: even lanes from the first operand, odd from the
            // second, each chosen within its half by one bit.
            | B::X86Shufpd | B::X86Shufpd256 => {
                let imm = operands.imm(2)?;
                let count = if id == B::X86Shufpd { 2 } else { 4 };
                picked(self, Type::I64, count, &|i| {
                    Pick::Lane(usize::from(i % 2 == 1), i - i % 2 + (imm >> i & 1))
                })?
            },
            // PSHUFD and VPERMILPS with an immediate: each lane of a half
            // chosen by two bits.
            | B::X86Pshufd | B::X86Pshufd256 | B::X86Vpermilps | B::X86Vpermilps256 => {
                let imm = operands.imm(1)?;
                let count = if matches!(id, B::X86Pshufd | B::X86Vpermilps) {
                    4
                } else {
                    8
                };
                picked(self, Type::I32, count, &|i| {
                    let k = i % 4;
                    Pick::Lane(0, i - k + (imm >> (2 * k) & 3))
                })?
            },
            // PSHUFLW and PSHUFHW: the low (high) four words of each half
            // chosen by two bits each, the others kept.
            | B::X86Pshuflw | B::X86Pshuflw256 | B::X86Pshufhw | B::X86Pshufhw256 => {
                let imm = operands.imm(1)?;
                let count = if matches!(id, B::X86Pshuflw | B::X86Pshufhw) {
                    8
                } else {
                    16
                };
                let high = matches!(id, B::X86Pshufhw | B::X86Pshufhw256);
                picked(self, Type::I16, count, &|i| {
                    let k = i % 8;
                    let quarter = if high { 4 } else { 0 };
                    if (k >= 4) == high {
                        Pick::Lane(0, i - k + quarter + (imm >> (2 * (k - quarter)) & 3))
                    } else {
                        Pick::Lane(0, i)
                    }
                })?
            },
            // VPERMILPD with an immediate: bit `i` chooses lane `i` within
            // its half.
            | B::X86Vpermilpd | B::X86Vpermilpd256 => {
                let imm = operands.imm(1)?;
                let count = if id == B::X86Vpermilpd { 2 } else { 4 };
                picked(self, Type::I64, count, &|i| {
                    Pick::Lane(0, i - i % 2 + (imm >> i & 1))
                })?
            },
            // PALIGNR: each half of the first operand above the same half of
            // the second, shifted right by the immediate's bytes.
            | B::X86Palignr128 | B::X86Palignr256 => {
                let imm = operands.imm(2)?;
                let count = if id == B::X86Palignr128 { 16 } else { 32 };
                picked(self, Type::I8, count, &|i| {
                    let (half, k) = (i - i % 16, i % 16 + imm);
                    match k {
                        | 0..16 => Pick::Lane(1, half + k),
                        | 16..32 => Pick::Lane(0, half + k - 16),
                        | _ => Pick::Zero,
                    }
                })?
            },
            // PSLLDQ and PSRLDQ: each half shifted by the immediate's bytes,
            // zeros shifted in.
            | B::X86Pslldqi128Byteshift | B::X86Pslldqi256Byteshift => {
                let imm = operands.imm(1)?;
                let count = if id == B::X86Pslldqi128Byteshift {
                    16
                } else {
                    32
                };
                picked(self, Type::I8, count, &|i| {
                    if i % 16 >= imm {
                        Pick::Lane(0, i - imm)
                    } else {
                        Pick::Zero
                    }
                })?
            },
            | B::X86Psrldqi128Byteshift | B::X86Psrldqi256Byteshift => {
                let imm = operands.imm(1)?;
                let count = if id == B::X86Psrldqi128Byteshift {
                    16
                } else {
                    32
                };
                picked(self, Type::I8, count, &|i| {
                    if i % 16 + imm < 16 {
                        Pick::Lane(0, i + imm)
                    } else {
                        Pick::Zero
                    }
                })?
            },
            // VPERM2F128 and VPERM2I128: each half of the result is the half
            // of either operand that four bits of the immediate name, or
            // zero when the fourth is set.
            | B::X86Vperm2f128Pd256
            | B::X86Vperm2f128Ps256
            | B::X86Vperm2f128Si256
            | B::X86Permti256 => {
                let imm = operands.imm(2)?;
                picked(self, Type::I64, 4, &|i| {
                    let control = imm >> (4 * (i / 2)) & 0xF;
                    if control & 8 != 0 {
                        Pick::Zero
                    } else {
                        Pick::Lane(usize::from(control & 2 != 0), (control & 1) * 2 + i % 2)
                    }
                })?
            },
            // VPERMQ and VPERMPD: each quadword chosen from all four by two
            // bits.
            | B::X86Permdi256 | B::X86Permdf256 => {
                let imm = operands.imm(1)?;
                picked(self, Type::I64, 4, &|i| Pick::Lane(0, imm >> (2 * i) & 3))?
            },
            // VEXTRACTF128 and VEXTRACTI128: the half bit 0 names.
            | B::X86Extract128i256
            | B::X86Vextractf128Pd256
            | B::X86Vextractf128Ps256
            | B::X86Vextractf128Si256 => {
                let imm = operands.imm(1)?;
                picked(self, Type::I64, 2, &|i| Pick::Lane(0, (imm & 1) * 2 + i))?
            },
            // VINSERTF128 and VINSERTI128: the first operand with the half
            // bit 0 names replaced by the second.
            | B::X86Insert128i256
            | B::X86Vinsertf128Pd256
            | B::X86Vinsertf128Ps256
            | B::X86Vinsertf128Si256 => {
                let imm = operands.imm(2)?;
                picked(self, Type::I64, 4, &|i| {
                    if i / 2 == imm & 1 {
                        Pick::Lane(1, i % 2)
                    } else {
                        Pick::Lane(0, i)
                    }
                })?
            },
            // INSERTPS: lane `imm[7:6]` of the second operand into lane
            // `imm[5:4]` of the first, then the lanes `imm[3:0]` names
            // zeroed.
            | B::X86Insertps128 => {
                let imm = operands.imm(2)?;
                picked(self, Type::I32, 4, &|i| {
                    if imm >> i & 1 != 0 {
                        Pick::Zero
                    } else if i == imm >> 4 & 3 {
                        Pick::Lane(1, imm >> 6 & 3)
                    } else {
                        Pick::Lane(0, i)
                    }
                })?
            },
            | B::X86VecExtV16hi
            | B::X86VecExtV16qi
            | B::X86VecExtV2di
            | B::X86VecExtV32qi
            | B::X86VecExtV4di
            | B::X86VecExtV4hi
            | B::X86VecExtV4sf
            | B::X86VecExtV4si
            | B::X86VecExtV8hi
            | B::X86VecExtV8si => {
                let ty = match id {
                    | B::X86VecExtV16qi | B::X86VecExtV32qi => Type::I8,
                    | B::X86VecExtV16hi | B::X86VecExtV4hi | B::X86VecExtV8hi => Type::I16,
                    | B::X86VecExtV4si | B::X86VecExtV8si => Type::I32,
                    | B::X86VecExtV4sf => Type::F32,
                    | _ => Type::I64,
                };
                Produced::Scalar(self.lane(a, ty, operands.imm(1)?))
            },
            | B::X86VecSetV16hi
            | B::X86VecSetV16qi
            | B::X86VecSetV2di
            | B::X86VecSetV32qi
            | B::X86VecSetV4di
            | B::X86VecSetV4hi
            | B::X86VecSetV4si
            | B::X86VecSetV8hi
            | B::X86VecSetV8si => {
                let (ty, count) = match id {
                    | B::X86VecSetV16qi => (Type::I8, 16),
                    | B::X86VecSetV32qi => (Type::I8, 32),
                    | B::X86VecSetV4hi => (Type::I16, 4),
                    | B::X86VecSetV8hi => (Type::I16, 8),
                    | B::X86VecSetV16hi => (Type::I16, 16),
                    | B::X86VecSetV4si => (Type::I32, 4),
                    | B::X86VecSetV8si => (Type::I32, 8),
                    | B::X86VecSetV2di => (Type::I64, 2),
                    | _ => (Type::I64, 4),
                };
                let (value, at) = (operands.get(1), operands.imm(2)?);
                Produced::Vector(self.fill_lanes(ty, count, source, |this, i| {
                    if i == at { value } else { this.lane(a, ty, i) }
                })?)
            },
            // VLDDQU: an unaligned load.
            | B::X86Lddqu256 => picked(self, Type::I64, 4, &|i| Pick::Lane(0, i))?,
            // PSHUFB: byte `i` of each half is zero when the control byte's
            // bit 7 is set, else the half's byte its bits 0-3 name.
            | B::X86Pshufb256 => {
                let control = sources[1];
                Produced::Vector(self.fill_lanes(Type::I8, 32, source, |this, i| {
                    let selector = this.lane(control, Type::I8, i);
                    let mask = this.iconst(Type::I8, 15);
                    let index = this.bin(Opcode::And, selector, mask);
                    let index = this.extend(index, false, Type::I64);
                    let half = this.iconst(Type::I64, i128::from(i - i % 16));
                    let index = this.bin(Opcode::Iadd, index, half);
                    let byte = this.indexed_lane(a, Type::I8, index);
                    let zero = this.iconst(Type::I8, 0);
                    let clear = this.sign_set(selector);
                    this.choose(clear, zero, byte)
                })?)
            },
            // VPERMD and VPERMPS: each lane any of the eight, by bits 0-2 of
            // the index lane.
            | B::X86Permvarsi256 | B::X86Permvarsf256 => {
                let control = sources[1];
                Produced::Vector(self.fill_lanes(Type::I32, 8, source, |this, i| {
                    let selector = this.lane(control, Type::I32, i);
                    let index = this.masked_index(selector, 0, 7, 0);
                    this.indexed_lane(a, Type::I32, index)
                })?)
            },
            // VPERMILPS: each lane from its half by bits 0-1 of the index
            // lane; VPERMILPD by bit 1.
            | B::X86Vpermilvarps | B::X86Vpermilvarps256 => {
                let control = sources[1];
                let count = if id == B::X86Vpermilvarps { 4 } else { 8 };
                Produced::Vector(self.fill_lanes(Type::I32, count, source, |this, i| {
                    let selector = this.lane(control, Type::I32, i);
                    let index = this.masked_index(selector, 0, 3, i - i % 4);
                    this.indexed_lane(a, Type::I32, index)
                })?)
            },
            | B::X86Vpermilvarpd | B::X86Vpermilvarpd256 => {
                let control = sources[1];
                let count = if id == B::X86Vpermilvarpd { 2 } else { 4 };
                Produced::Vector(self.fill_lanes(Type::I64, count, source, |this, i| {
                    let selector = this.lane(control, Type::I64, i);
                    let index = this.masked_index(selector, 1, 1, i - i % 2);
                    this.indexed_lane(a, Type::I64, index)
                })?)
            },
            | _ => return Ok(None),
        };
        Ok(Some(produced))
    }

    /// A fresh vector of `count` lanes of type `ty`, each the lane of one of
    /// `sources` that `pick` names, or zero.
    fn picked_lanes(
        &mut self,
        sources: &[Value],
        ty: Type,
        count: u64,
        source: SourceVectors,
        pick: &dyn Fn(u64) -> Pick,
    ) -> Result<Value, LoweringError> {
        self.fill_lanes(ty, count, source, |this, i| match pick(i) {
            | Pick::Lane(operand, lane) => this.lane(sources[operand], ty, lane),
            | Pick::Zero => this.iconst(ty, 0),
        })
    }

    /// `(selector >> shift & mask) + base` as an `i64` lane index.
    fn masked_index(&mut self, selector: Value, shift: u32, mask: i128, base: u64) -> Value {
        let ty = self.draft.value_type(selector);
        let shift = self.iconst(ty, i128::from(shift));
        let shifted = self.bin(Opcode::Lshr, selector, shift);
        let mask = self.iconst(ty, mask);
        let index = self.bin(Opcode::And, shifted, mask);
        let index = self.extend(index, false, Type::I64);
        let base = self.iconst(Type::I64, i128::from(base));
        self.bin(Opcode::Iadd, index, base)
    }

    /// The lane of type `ty` at the run-time lane index `index` (an `i64`)
    /// of the vector at `base`.
    pub(super) fn indexed_lane(&mut self, base: Value, ty: Type, index: Value) -> Value {
        let size = self.iconst(Type::I64, i128::from(ty.bytes()));
        let offset = self.bin(Opcode::Imul, index, size);
        let addr = self.ptr_add(base, offset);
        let align = super::super::alignment::at_offset(
            self.place_alignment(base, Align::BYTE),
            u128::from(ty.bytes()),
        );
        self.inst(
            InstData::Load {
                ty,
                flags: InstFlags::empty(),
                align,
                tag: None,
                addr,
            },
            ty,
        )
    }
}

/// Where a result lane comes from.
enum Pick {
    /// Operand `.0`'s lane `.1`.
    Lane(usize, u64),
    Zero,
}
