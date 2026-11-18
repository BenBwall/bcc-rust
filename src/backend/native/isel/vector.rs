//! The named x86 instructions of the IR (`ir/intrinsics.rs`): each becomes
//! the instruction itself.
//!
//! A vector operand is the address of its bytes and a vector result goes
//! through a destination address, so the sequence loads each vector operand
//! into a precolored SSE register (`movups`, any alignment), runs the
//! instruction on the registers, and stores the result: no virtual register
//! holds a vector. The first operand goes to `%xmm0` and the second to
//! `%xmm1`, both caller-saved under either convention; a string comparison
//! takes `%xmm1` and `%xmm2`, since its mask arrives in `%xmm0`. A function
//! whose features include AVX gets the VEX forms, as GCC and Clang emit
//! there; a 32-byte vector needs them, and a `vzeroupper` follows, since
//! nothing else here keeps a value in the YMM registers' upper halves and
//! the SSE code after it would otherwise pay for the transition. The scalar
//! square roots compute on the values' own registers, and `xgetbv` reads its
//! index from `%ecx` and its result from `%edx:%eax`.
//!
//! Intel SDM Vol. 2A and 2B name each instruction ("AESENC", "PCLMULQDQ",
//! "ROUNDPS", "CVTPS2DQ", "RCPPS", "SQRTSS", "PCMPISTRI", "VZEROUPPER",
//! "XGETBV").

use super::Selector;
use crate::{
    backend::native::mir::{
        AluOp,
        Cond,
        MInst,
        Operand,
        PReg,
        Reg,
        ShiftOp,
        StringOp,
        VectorForm,
        VectorOp,
        Width,
    },
    ir::{
        Intrinsic,
        Value,
    },
};

impl Selector<'_, '_, '_> {
    /// `intrinsic` with `immediate` on `args`, the destination first when
    /// the result is a vector, defining `result` when it is a scalar.
    pub(super) fn intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        immediate: u8,
        args: &[Value],
        result: Option<Value>,
    ) {
        let signature = intrinsic.signature();
        let bytes = signature
            .operands
            .iter()
            .chain(&signature.result)
            .filter_map(|operand| operand.vector_bytes())
            .max()
            .unwrap_or(0);
        let form = if bytes > 16 {
            VectorForm::Vex256
        } else if self.avx {
            VectorForm::Vex128
        } else {
            VectorForm::Sse
        };
        let (destination, operands) = if signature.has_destination() {
            let (&destination, operands) =
                args.split_first().expect("the verifier checked the arity");
            (Some(destination), operands)
        } else {
            (None, args)
        };
        match intrinsic {
            | Intrinsic::SqrtF32 | Intrinsic::SqrtF64 => {
                let op = if intrinsic == Intrinsic::SqrtF32 {
                    VectorOp::SqrtSs
                } else {
                    VectorOp::SqrtSd
                };
                let src = self.reg(operands[0]);
                let dst = self.result_reg(result);
                self.emit(MInst::Vector {
                    op,
                    form: VectorForm::Sse,
                    imm: 0,
                    dst,
                    src,
                });
            },
            | Intrinsic::Vzeroupper | Intrinsic::Vzeroall => self.emit(MInst::VZero {
                all: intrinsic == Intrinsic::Vzeroall,
            }),
            | Intrinsic::Xgetbv => self.xgetbv(operands[0], result),
            | _ => match string_comparison(intrinsic) {
                | Some((op, flag)) =>
                    self.string_comparison(op, flag, form, immediate, operands, destination, result),
                | None => {
                    let op = vector_op(intrinsic);
                    self.vector_operation(op, form, immediate, operands, destination);
                },
            },
        }
        if form == VectorForm::Vex256 {
            self.emit(MInst::VZero { all: false });
        }
    }

    /// `op` on the vectors `operands` address, the result stored through
    /// `destination`.
    fn vector_operation(
        &mut self,
        op: VectorOp,
        form: VectorForm,
        immediate: u8,
        operands: &[Value],
        destination: Option<Value>,
    ) {
        let first = Reg::Phys(PReg::Xmm0);
        let second = Reg::Phys(PReg::Xmm1);
        self.load_vector(form, first, operands[0]);
        let src = if op.reads_destination() {
            self.load_vector(form, second, operands[1]);
            second
        } else {
            first
        };
        self.emit(MInst::Vector {
            op,
            form,
            imm: immediate,
            dst: first,
            src,
        });
        let stored = if op.narrows() {
            VectorForm::Vex128
        } else {
            form
        };
        let destination = destination.expect("a vector operation has a vector result");
        self.store_vector(stored, first, destination);
    }

    /// A string comparison of the vectors `operands` address (with the
    /// lengths after each, for the explicit forms): the mask stored through
    /// `destination`, or the index or the flag `flag` names as `result`.
    #[expect(
        clippy::too_many_arguments,
        reason = "The instruction's operands, immediate and both kinds of result."
    )]
    fn string_comparison(
        &mut self,
        op: StringOp,
        flag: Option<Cond>,
        form: VectorForm,
        immediate: u8,
        operands: &[Value],
        destination: Option<Value>,
        result: Option<Value>,
    ) {
        let lhs = Reg::Phys(PReg::Xmm1);
        let rhs = Reg::Phys(PReg::Xmm2);
        let (left, right) = if op.explicit() {
            (operands[0], operands[2])
        } else {
            (operands[0], operands[1])
        };
        self.load_vector(form, lhs, left);
        self.load_vector(form, rhs, right);
        if op.explicit() {
            for (length, register) in [(operands[1], PReg::Rax), (operands[3], PReg::Rdx)] {
                let src = self.operand(length, Width::B32);
                self.emit(MInst::Mov {
                    width: Width::B32,
                    dst: Reg::Phys(register),
                    src,
                });
            }
        }
        self.emit(MInst::StringCompare {
            op,
            form,
            imm: immediate,
            lhs,
            rhs,
        });
        if op.mask() {
            let destination = destination.expect("a mask comparison has a vector result");
            self.store_vector(form, Reg::Phys(PReg::Xmm0), destination);
            return;
        }
        let dst = self.result_reg(result);
        match flag {
            | Some(cond) => self.set_condition(cond, dst),
            | None => self.emit(MInst::Mov {
                width: Width::B32,
                dst,
                src: Operand::Reg(Reg::Phys(PReg::Rcx)),
            }),
        }
    }

    /// `result ←` the extended control register `index` selects, from
    /// `%edx:%eax`, whose 32-bit writes clear the upper halves.
    fn xgetbv(&mut self, index: Value, result: Option<Value>) {
        let src = self.operand(index, Width::B32);
        self.emit(MInst::Mov {
            width: Width::B32,
            dst: Reg::Phys(PReg::Rcx),
            src,
        });
        self.emit(MInst::Xgetbv);
        let high = self.vreg();
        self.copy(high, Reg::Phys(PReg::Rdx));
        let dst = self.result_reg(result);
        self.copy(dst, Reg::Phys(PReg::Rax));
        self.shift_imm(ShiftOp::Shl, Width::B64, high, 32);
        self.emit(MInst::Alu {
            op: AluOp::Or,
            width: Width::B64,
            dst,
            src: Operand::Reg(high),
        });
    }

    /// `reg ←` the vector `address` points to.
    fn load_vector(&mut self, form: VectorForm, reg: Reg, address: Value) {
        let mem = self.address(address);
        self.emit(MInst::VectorMove {
            form,
            load: true,
            reg,
            mem,
        });
    }

    /// The vector in `reg` stored where `address` points.
    fn store_vector(&mut self, form: VectorForm, reg: Reg, address: Value) {
        let mem = self.address(address);
        self.emit(MInst::VectorMove {
            form,
            load: false,
            reg,
            mem,
        });
    }
}

/// The instruction of a vector intrinsic other than a string comparison.
fn vector_op(intrinsic: Intrinsic) -> VectorOp {
    match intrinsic {
        | Intrinsic::AesEnc => VectorOp::AesEnc,
        | Intrinsic::AesEncLast => VectorOp::AesEncLast,
        | Intrinsic::AesDec => VectorOp::AesDec,
        | Intrinsic::AesDecLast => VectorOp::AesDecLast,
        | Intrinsic::AesImc => VectorOp::AesImc,
        | Intrinsic::AesKeygenAssist => VectorOp::AesKeygenAssist,
        | Intrinsic::Pclmulqdq => VectorOp::Pclmulqdq,
        | Intrinsic::RcpPs256 => VectorOp::RcpPs,
        | Intrinsic::RsqrtPs256 => VectorOp::RsqrtPs,
        | Intrinsic::RoundPs | Intrinsic::RoundPs256 => VectorOp::RoundPs,
        | Intrinsic::RoundPd | Intrinsic::RoundPd256 => VectorOp::RoundPd,
        | Intrinsic::RoundSs => VectorOp::RoundSs,
        | Intrinsic::RoundSd => VectorOp::RoundSd,
        | Intrinsic::CvtPs2dq256 => VectorOp::CvtPs2dq,
        | Intrinsic::CvtPd2dq256 => VectorOp::CvtPd2dq,
        | Intrinsic::CvttPs2dq256 => VectorOp::CvttPs2dq,
        | Intrinsic::CvttPd2dq256 => VectorOp::CvttPd2dq,
        | other => unreachable!("{other} is no vector operation"),
    }
}

/// The comparison of a string intrinsic, and the flag it returns, if any:
/// `a` above (CF and ZF clear), `c` CF, `o` OF, `s` SF and `z` ZF.
const fn string_comparison(intrinsic: Intrinsic) -> Option<(StringOp, Option<Cond>)> {
    use StringOp::{
        ExplicitIndex,
        ExplicitMask,
        ImplicitIndex,
        ImplicitMask,
    };
    Some(match intrinsic {
        | Intrinsic::Pcmpistri128 => (ImplicitIndex, None),
        | Intrinsic::Pcmpistrm128 => (ImplicitMask, None),
        | Intrinsic::Pcmpistria128 => (ImplicitIndex, Some(Cond::A)),
        | Intrinsic::Pcmpistric128 => (ImplicitIndex, Some(Cond::B)),
        | Intrinsic::Pcmpistrio128 => (ImplicitIndex, Some(Cond::O)),
        | Intrinsic::Pcmpistris128 => (ImplicitIndex, Some(Cond::S)),
        | Intrinsic::Pcmpistriz128 => (ImplicitIndex, Some(Cond::E)),
        | Intrinsic::Pcmpestri128 => (ExplicitIndex, None),
        | Intrinsic::Pcmpestrm128 => (ExplicitMask, None),
        | Intrinsic::Pcmpestria128 => (ExplicitIndex, Some(Cond::A)),
        | Intrinsic::Pcmpestric128 => (ExplicitIndex, Some(Cond::B)),
        | Intrinsic::Pcmpestrio128 => (ExplicitIndex, Some(Cond::O)),
        | Intrinsic::Pcmpestris128 => (ExplicitIndex, Some(Cond::S)),
        | Intrinsic::Pcmpestriz128 => (ExplicitIndex, Some(Cond::E)),
        | _ => return None,
    })
}
