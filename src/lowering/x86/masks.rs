//! Comparisons into lane masks, the sign-bit masks and tests, selection by
//! a mask, and the loads, stores and gathers a mask guards. A masked-off
//! lane's memory is never accessed, so each guarded access runs in a block
//! of its own (Intel SDM Vol. 2, each instruction's "Operation"; Intel
//! Intrinsics Guide).

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
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::SourceVectors,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Expands the mask builtin `id`, if it is one.
    #[expect(
        clippy::many_single_char_names,
        reason = "the operands and lanes keep the SDM's names"
    )]
    pub(super) fn mask_builtin(
        &mut self,
        id: BuiltinId,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Option<Produced>, LoweringError> {
        use BuiltinId as B;
        let a = operands.get(0);
        let b = operands.values.get(1).copied().unwrap_or(a);
        let produced = match id {
            // VMOVMSKPS, VMOVMSKPD, PMOVMSKB: bit `i` is lane `i`'s sign.
            | B::X86Movmskps256 | B::X86Movmskpd256 | B::X86Pmovmskb256 => {
                let (ty, count) = match id {
                    | B::X86Movmskps256 => (Type::I32, 8),
                    | B::X86Movmskpd256 => (Type::I64, 4),
                    | _ => (Type::I8, 32),
                };
                let mut mask = self.iconst(Type::I32, 0);
                for i in 0..count {
                    let lane = self.lane(a, ty, i);
                    let sign = self.sign_set(lane);
                    let bit = self.extend(sign, false, Type::I32);
                    let position = self.iconst(Type::I32, i128::from(i));
                    let bit = self.bin(Opcode::Shl, bit, position);
                    mask = self.bin(Opcode::Or, mask, bit);
                }
                Produced::Scalar(mask)
            },
            // VPTEST: ZF is set when `a & b` is zero and CF when `!a & b`
            // is; VTESTPS and VTESTPD test only each lane's sign bit. The
            // `z` builtins return ZF, the `c` ones CF, and the `nzc` ones
            // whether both are clear.
            | B::X86Ptestz256
            | B::X86Ptestc256
            | B::X86Ptestnzc256
            | B::X86Vtestzps
            | B::X86Vtestcps
            | B::X86Vtestnzcps
            | B::X86Vtestzps256
            | B::X86Vtestcps256
            | B::X86Vtestnzcps256
            | B::X86Vtestzpd
            | B::X86Vtestcpd
            | B::X86Vtestnzcpd
            | B::X86Vtestzpd256
            | B::X86Vtestcpd256
            | B::X86Vtestnzcpd256 => {
                let (ty, count) = match id {
                    | B::X86Vtestzps | B::X86Vtestcps | B::X86Vtestnzcps => (Type::I32, 4),
                    | B::X86Vtestzps256 | B::X86Vtestcps256 | B::X86Vtestnzcps256 => (Type::I32, 8),
                    | B::X86Vtestzpd | B::X86Vtestcpd | B::X86Vtestnzcpd => (Type::I64, 2),
                    | _ => (Type::I64, 4),
                };
                let signs = !matches!(id, B::X86Ptestz256 | B::X86Ptestc256 | B::X86Ptestnzc256);
                let mut both = self.iconst(ty, 0);
                let mut second = self.iconst(ty, 0);
                for i in 0..count {
                    let (x, y) = (self.lane(a, ty, i), self.lane(b, ty, i));
                    let and = self.bin(Opcode::And, x, y);
                    both = self.bin(Opcode::Or, both, and);
                    let ones = self.iconst(ty, -1);
                    let inverted = self.bin(Opcode::Xor, x, ones);
                    let and_not = self.bin(Opcode::And, inverted, y);
                    second = self.bin(Opcode::Or, second, and_not);
                }
                let zero = self.iconst(ty, 0);
                let (zf, cf) = if signs {
                    let (z, c) = (self.sign_set(both), self.sign_set(second));
                    let yes = self.iconst(Type::I1, 1);
                    (self.bin(Opcode::Xor, z, yes), self.bin(Opcode::Xor, c, yes))
                } else {
                    (
                        self.icmp(IntCC::Eq, both, zero),
                        self.icmp(IntCC::Eq, second, zero),
                    )
                };
                let flag = match id {
                    | B::X86Ptestz256
                    | B::X86Vtestzps
                    | B::X86Vtestzps256
                    | B::X86Vtestzpd
                    | B::X86Vtestzpd256 => zf,
                    | B::X86Ptestc256
                    | B::X86Vtestcps
                    | B::X86Vtestcps256
                    | B::X86Vtestcpd
                    | B::X86Vtestcpd256 => cf,
                    | _ => {
                        let either = self.bin(Opcode::Or, zf, cf);
                        let yes = self.iconst(Type::I1, 1);
                        self.bin(Opcode::Xor, either, yes)
                    },
                };
                Produced::Scalar(self.extend(flag, false, Type::I32))
            },
            // VBLENDVPS, VBLENDVPD, PBLENDVB: lane `i` of the second operand
            // where the mask lane's sign bit is set, else of the first.
            | B::X86Blendvps256 | B::X86Blendvpd256 | B::X86Pblendvb256 => {
                let (ty, count) = match id {
                    | B::X86Blendvps256 => (Type::I32, 8),
                    | B::X86Blendvpd256 => (Type::I64, 4),
                    | _ => (Type::I8, 32),
                };
                let mask = operands.get(2);
                Produced::Vector(self.fill_lanes(ty, count, source, |this, i| {
                    let (x, y, m) = (
                        this.lane(a, ty, i),
                        this.lane(b, ty, i),
                        this.lane(mask, ty, i),
                    );
                    let set = this.sign_set(m);
                    this.choose(set, y, x)
                })?)
            },
            // CMPPS and kin: all ones in each lane where the comparison the
            // immediate's bits 0-3 select holds (bit 4 only chooses whether a
            // quiet NaN signals); the scalar forms compare lane 0 and keep
            // the first operand's other lanes.
            | B::X86Cmpps
            | B::X86Cmpps256
            | B::X86Cmppd
            | B::X86Cmppd256
            | B::X86Cmpss
            | B::X86Cmpsd => {
                let (ty, count) = match id {
                    | B::X86Cmpps | B::X86Cmpss => (Type::F32, 4),
                    | B::X86Cmpps256 => (Type::F32, 8),
                    | B::X86Cmppd | B::X86Cmpsd => (Type::F64, 2),
                    | _ => (Type::F64, 4),
                };
                let scalar = matches!(id, B::X86Cmpss | B::X86Cmpsd);
                let predicate = operands.imm(2)? & 15;
                let int = int_of(u64::from(ty.bytes()));
                Produced::Vector(self.fill_lanes(int, count, source, |this, i| {
                    if scalar && i > 0 {
                        return this.lane(a, int, i);
                    }
                    let (x, y) = (this.lane(a, ty, i), this.lane(b, ty, i));
                    let truth = match predicate {
                        | 11 => this.iconst(Type::I1, 0),
                        | 15 => this.iconst(Type::I1, 1),
                        | _ => this.fcmp(compare_predicate(predicate), x, y),
                    };
                    this.extend(truth, true, int)
                })?)
            },
            // VMASKMOVPS and VPMASKMOVD and kin: the lanes whose mask sign
            // bit is set are loaded, the others zero.
            | B::X86Maskloadd
            | B::X86Maskloadd256
            | B::X86Maskloadpd
            | B::X86Maskloadpd256
            | B::X86Maskloadps
            | B::X86Maskloadps256
            | B::X86Maskloadq
            | B::X86Maskloadq256 => {
                let (ty, count) = mask_shape(id);
                let object = self.fill_lanes(ty, count, source, |this, _| this.iconst(ty, 0))?;
                for i in 0..count {
                    let m = self.lane(b, ty, i);
                    let set = self.sign_set(m);
                    self.guarded(set, |this| {
                        let value = this.lane(a, ty, i);
                        this.set_lane(value, object, i);
                    });
                }
                Produced::Vector(object)
            },
            // Their stores: the lanes whose mask sign bit is set are stored,
            // the others' memory untouched.
            | B::X86Maskstored
            | B::X86Maskstored256
            | B::X86Maskstorepd
            | B::X86Maskstorepd256
            | B::X86Maskstoreps
            | B::X86Maskstoreps256
            | B::X86Maskstoreq
            | B::X86Maskstoreq256 => {
                let (ty, count) = mask_shape(id);
                let values = operands.get(2);
                for i in 0..count {
                    let m = self.lane(b, ty, i);
                    let set = self.sign_set(m);
                    self.guarded(set, |this| {
                        let value = this.lane(values, ty, i);
                        this.set_lane(value, a, i);
                    });
                }
                Produced::Nothing
            },
            // VPGATHERDD and kin: lane `i` is loaded from `base + index[i] *
            // scale` where the mask lane's sign bit is set, else taken from
            // the first operand; lanes past the indices are zero.
            | B::X86GatherdD
            | B::X86GatherdD256
            | B::X86GatherdPd
            | B::X86GatherdPd256
            | B::X86GatherdPs
            | B::X86GatherdPs256
            | B::X86GatherdQ
            | B::X86GatherdQ256
            | B::X86GatherqD
            | B::X86GatherqD256
            | B::X86GatherqPd
            | B::X86GatherqPd256
            | B::X86GatherqPs
            | B::X86GatherqPs256
            | B::X86GatherqQ
            | B::X86GatherqQ256 => {
                let (ty, count, index_ty, indices) = gather_shape(id);
                let (base, index, mask) = (operands.get(1), operands.get(2), operands.get(3));
                let scale = operands.imm(4)?;
                let gathered = count.min(indices);
                let object = self.fill_lanes(ty, count, source, |this, i| {
                    if i < gathered {
                        this.lane(a, ty, i)
                    } else {
                        this.iconst(ty, 0)
                    }
                })?;
                for i in 0..gathered {
                    let m = self.lane(mask, ty, i);
                    let set = self.sign_set(m);
                    self.guarded(set, |this| {
                        let offset = this.lane(index, index_ty, i);
                        let offset = this.extend(offset, true, Type::I64);
                        let scale = this.iconst(Type::I64, i128::from(scale));
                        let offset = this.bin(Opcode::Imul, offset, scale);
                        let address =
                            this.binary_inst(Opcode::PtrAdd, InstFlags::empty(), base, offset);
                        let value = this.lane(address, ty, 0);
                        this.set_lane(value, object, i);
                    });
                }
                Produced::Vector(object)
            },
            | _ => return Ok(None),
        };
        Ok(Some(produced))
    }

    /// Runs `body` only where `condition` holds, in a block of its own.
    fn guarded(&mut self, condition: Value, body: impl FnOnce(&mut Self)) {
        let then = self.draft.create_block(&[]);
        let next = self.draft.create_block(&[]);
        self.draft.brif(condition, then, next);
        self.draft.seal(then);
        self.draft.switch_to(then);
        body(self);
        self.draft.jump(next, &[]);
        self.draft.seal(next);
        self.draft.switch_to(next);
    }
}

/// The comparison of CMPPS's predicates 0-15 other than FALSE (11) and
/// TRUE (15): Intel SDM Vol. 2A, "CMPPS", Table 3-1.
const fn compare_predicate(predicate: u64) -> FloatCC {
    match predicate {
        | 0 => FloatCC::Oeq,
        | 1 => FloatCC::Olt,
        | 2 => FloatCC::Ole,
        | 3 => FloatCC::Uno,
        | 4 => FloatCC::Une,
        | 5 => FloatCC::Uge,
        | 6 => FloatCC::Ugt,
        | 7 => FloatCC::Ord,
        | 8 => FloatCC::Ueq,
        | 9 => FloatCC::Ult,
        | 10 => FloatCC::Ule,
        | 12 => FloatCC::One,
        | 13 => FloatCC::Oge,
        | _ => FloatCC::Ogt,
    }
}

/// The lane type and count of a masked load or store.
const fn mask_shape(id: BuiltinId) -> (Type, u64) {
    use BuiltinId as B;
    match id {
        | B::X86Maskloadd | B::X86Maskloadps | B::X86Maskstored | B::X86Maskstoreps =>
            (Type::I32, 4),
        | B::X86Maskloadd256 | B::X86Maskloadps256 | B::X86Maskstored256 | B::X86Maskstoreps256 =>
            (Type::I32, 8),
        | B::X86Maskloadpd | B::X86Maskloadq | B::X86Maskstorepd | B::X86Maskstoreq =>
            (Type::I64, 2),
        | _ => (Type::I64, 4),
    }
}

/// A gather's result lane type and count, and its index lane type and
/// count.
const fn gather_shape(id: BuiltinId) -> (Type, u64, Type, u64) {
    use BuiltinId as B;
    match id {
        | B::X86GatherdD | B::X86GatherdPs => (Type::I32, 4, Type::I32, 4),
        | B::X86GatherdD256 | B::X86GatherdPs256 => (Type::I32, 8, Type::I32, 8),
        | B::X86GatherdPd | B::X86GatherdQ => (Type::I64, 2, Type::I32, 4),
        | B::X86GatherdPd256 | B::X86GatherdQ256 => (Type::I64, 4, Type::I32, 4),
        | B::X86GatherqD | B::X86GatherqPs => (Type::I32, 4, Type::I64, 2),
        | B::X86GatherqD256 | B::X86GatherqPs256 => (Type::I32, 4, Type::I64, 4),
        | B::X86GatherqPd | B::X86GatherqQ => (Type::I64, 2, Type::I64, 2),
        | _ => (Type::I64, 4, Type::I64, 4),
    }
}
