//! Named target instructions: the x86 SIMD instructions whose result no
//! generic IR computes exactly, or computes only at great length. Lowering
//! expands most `__builtin_ia32_*` builtins into scalar lane code
//! (`lowering/x86.rs`); the rest become one `intrinsic` instruction naming
//! the instruction, as Clang's `CGBuiltin.cpp` emits an LLVM intrinsic for
//! them.
//!
//! An intrinsic reads each vector operand from memory, through a `ptr`
//! operand, and writes a vector result through a destination `ptr`, its
//! first operand, since vectors have no IR type and live in memory
//! (`lowering/vectors.rs`); a scalar operand or result is a value. An
//! immediate operand is a constant in the record, as the instruction
//! encodes it. The textual form writes the name, the immediate if the
//! intrinsic has one, then the operands:
//!
//! ```text
//! intrinsic x86.pclmulqdq 17, v0, v1, v2
//! v5 = intrinsic x86.pcmpistri128 12, v3, v4
//! ```
//!
//! The LLVM back end calls the LLVM intrinsic of [`IntrinsicSignature::llvm`]
//! on the loaded vectors; the interpreter computes the instructions it
//! models (`backend/interpreter/intrinsics.rs`); the native and WebAssembly
//! back ends refuse the instruction by name.

use super::types::Type;

/// An operand or result of an intrinsic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntrinsicOperand {
    /// A vector of `count` lanes of type `lane`, in memory: a `ptr`
    /// operand.
    Vector(Type, u8),
    /// A value of this type.
    Scalar(Type),
}

impl IntrinsicOperand {
    /// The IR type of the operand's value: a vector's address.
    pub(crate) const fn value_type(self) -> Type {
        match self {
            | Self::Vector(..) => Type::Ptr,
            | Self::Scalar(ty) => ty,
        }
    }

    /// The bytes a vector operand reads or a vector result writes.
    pub(crate) const fn vector_bytes(self) -> Option<u32> {
        match self {
            | Self::Vector(lane, count) => Some(lane.bytes() * count as u32),
            | Self::Scalar(_) => None,
        }
    }
}

/// What an intrinsic takes and gives, and the LLVM intrinsic it is.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IntrinsicSignature {
    /// `None` for an instruction with no result.
    pub(crate) result:     Option<IntrinsicOperand>,
    pub(crate) operands:   &'static [IntrinsicOperand],
    /// The type of the immediate operand LLVM's intrinsic takes last, if
    /// the instruction has one.
    pub(crate) immediate:  Option<Type>,
    pub(crate) llvm:       &'static str,
    /// Whether the LLVM intrinsic is overloaded on its result type.
    pub(crate) overloaded: bool,
}

impl IntrinsicSignature {
    /// The value the instruction defines: a scalar result. A vector result
    /// is written through the destination operand.
    pub(crate) const fn result_type(&self) -> Option<Type> {
        match self.result {
            | Some(IntrinsicOperand::Scalar(ty)) => Some(ty),
            | _ => None,
        }
    }

    /// The vector result's destination, which comes first in the operand
    /// list.
    pub(crate) const fn has_destination(&self) -> bool {
        matches!(self.result, Some(IntrinsicOperand::Vector(..)))
    }

    /// The number of IR operands: the destination, if any, and the
    /// operands.
    pub(crate) const fn arity(&self) -> usize {
        self.operands.len() + self.has_destination() as usize
    }
}

use IntrinsicOperand::{
    Scalar as S,
    Vector as V,
};

const I8X16: IntrinsicOperand = V(Type::I8, 16);
const I32X4: IntrinsicOperand = V(Type::I32, 4);
const I32X8: IntrinsicOperand = V(Type::I32, 8);
const I64X2: IntrinsicOperand = V(Type::I64, 2);
const F32X4: IntrinsicOperand = V(Type::F32, 4);
const F32X8: IntrinsicOperand = V(Type::F32, 8);
const F64X2: IntrinsicOperand = V(Type::F64, 2);
const F64X4: IntrinsicOperand = V(Type::F64, 4);
const I32: IntrinsicOperand = S(Type::I32);

const fn sig(
    result: Option<IntrinsicOperand>,
    operands: &'static [IntrinsicOperand],
    immediate: Option<Type>,
    llvm: &'static str,
) -> IntrinsicSignature {
    IntrinsicSignature {
        result,
        operands,
        immediate,
        llvm,
        overloaded: false,
    }
}

/// An intrinsic whose LLVM intrinsic is overloaded on its result type.
const fn overloaded(signature: IntrinsicSignature) -> IntrinsicSignature {
    IntrinsicSignature {
        overloaded: true,
        ..signature
    }
}

/// Defines [`Intrinsic`] with each variant's text name and signature.
macro_rules! intrinsics {
    ($($(#[$meta:meta])* $variant:ident = $text:literal => $signature:expr,)+) => {
        /// A named target instruction.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[repr(u8)]
        pub(crate) enum Intrinsic {
            $($(#[$meta])* $variant,)+
        }

        impl Intrinsic {
            /// Every intrinsic, in declaration order.
            pub(crate) const ALL: &[Self] = &[$(Self::$variant,)+];

            /// The name the textual form uses.
            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            pub(crate) const fn signature(self) -> IntrinsicSignature {
                match self {
                    $(Self::$variant => $signature,)+
                }
            }
        }
    };
}

intrinsics! {
    /// The correctly rounded square root (`sqrtss`, `sqrtsd`), lane by lane
    /// for Clang's `__builtin_elementwise_sqrt`. Intel SDM Vol. 2B,
    /// "SQRTSS"; IEEE 754-2008 §5.4.1.
    SqrtF32 = "x86.sqrtss"
        => overloaded(sig(Some(S(Type::F32)), &[S(Type::F32)], None, "llvm.sqrt")),
    SqrtF64 = "x86.sqrtsd"
        => overloaded(sig(Some(S(Type::F64)), &[S(Type::F64)], None, "llvm.sqrt")),
    /// One AES encryption round (`aesenc`): `ShiftRows`, `SubBytes`,
    /// `MixColumns`, then the round key added by exclusive or. Intel SDM
    /// Vol. 2A, "AESENC"; Clang `BuiltinsX86.td`, `__builtin_ia32_aesenc128`.
    AesEnc = "x86.aesenc" => sig(Some(I64X2), &[I64X2, I64X2], None, "llvm.x86.aesni.aesenc"),
    /// The last AES encryption round, without `MixColumns` (`aesenclast`).
    AesEncLast = "x86.aesenclast"
        => sig(Some(I64X2), &[I64X2, I64X2], None, "llvm.x86.aesni.aesenclast"),
    /// One AES decryption round (`aesdec`): `InvShiftRows`, `InvSubBytes`,
    /// `InvMixColumns`, then the round key added by exclusive or.
    AesDec = "x86.aesdec" => sig(Some(I64X2), &[I64X2, I64X2], None, "llvm.x86.aesni.aesdec"),
    /// The last AES decryption round, without `InvMixColumns` (`aesdeclast`).
    AesDecLast = "x86.aesdeclast"
        => sig(Some(I64X2), &[I64X2, I64X2], None, "llvm.x86.aesni.aesdeclast"),
    /// `InvMixColumns` of a round key (`aesimc`).
    AesImc = "x86.aesimc" => sig(Some(I64X2), &[I64X2], None, "llvm.x86.aesni.aesimc"),
    /// The AES key-schedule step (`aeskeygenassist`), with the round
    /// constant as its immediate.
    AesKeygenAssist = "x86.aeskeygenassist"
        => sig(Some(I64X2), &[I64X2], Some(Type::I8), "llvm.x86.aesni.aeskeygenassist"),
    /// The carry-less product of one quadword of each operand, which bits 0
    /// and 4 of the immediate select (`pclmulqdq`). Intel SDM Vol. 2B,
    /// "PCLMULQDQ".
    Pclmulqdq = "x86.pclmulqdq"
        => sig(Some(I64X2), &[I64X2, I64X2], Some(Type::I8), "llvm.x86.pclmulqdq"),
    /// Approximate reciprocals, with a relative error of at most
    /// 1.5 * 2^-12 that the processor chooses (`vrcpps`). Intel SDM
    /// Vol. 2B, "RCPPS".
    RcpPs256 = "x86.rcpps256" => sig(Some(F32X8), &[F32X8], None, "llvm.x86.avx.rcp.ps.256"),
    /// Approximate reciprocal square roots, as `vrcpps` approximates
    /// (`vrsqrtps`). Intel SDM Vol. 2B, "RSQRTPS".
    RsqrtPs256 = "x86.rsqrtps256"
        => sig(Some(F32X8), &[F32X8], None, "llvm.x86.avx.rsqrt.ps.256"),
    /// Each lane rounded to an integral value in the mode the immediate's
    /// bits 0-2 select (`roundps`). Intel SDM Vol. 2B, "ROUNDPS".
    RoundPs = "x86.roundps" => sig(Some(F32X4), &[F32X4], Some(Type::I32), "llvm.x86.sse41.round.ps"),
    RoundPd = "x86.roundpd" => sig(Some(F64X2), &[F64X2], Some(Type::I32), "llvm.x86.sse41.round.pd"),
    RoundPs256 = "x86.roundps256"
        => sig(Some(F32X8), &[F32X8], Some(Type::I32), "llvm.x86.avx.round.ps.256"),
    RoundPd256 = "x86.roundpd256"
        => sig(Some(F64X4), &[F64X4], Some(Type::I32), "llvm.x86.avx.round.pd.256"),
    /// The first operand with lane 0 replaced by the second's rounded
    /// (`roundss`).
    RoundSs = "x86.roundss"
        => sig(Some(F32X4), &[F32X4, F32X4], Some(Type::I32), "llvm.x86.sse41.round.ss"),
    RoundSd = "x86.roundsd"
        => sig(Some(F64X2), &[F64X2, F64X2], Some(Type::I32), "llvm.x86.sse41.round.sd"),
    /// Each lane converted to `i32` in the rounding mode of MXCSR, which is
    /// to nearest, ties to even; a lane out of range or NaN gives
    /// `0x80000000` (`vcvtps2dq`). Intel SDM Vol. 2A, "CVTPS2DQ".
    CvtPs2dq256 = "x86.cvtps2dq256"
        => sig(Some(I32X8), &[F32X8], None, "llvm.x86.avx.cvt.ps2dq.256"),
    CvtPd2dq256 = "x86.cvtpd2dq256"
        => sig(Some(I32X4), &[F64X4], None, "llvm.x86.avx.cvt.pd2dq.256"),
    /// As `cvtps2dq256`, truncating (`vcvttps2dq`). Intel SDM Vol. 2A,
    /// "CVTTPS2DQ".
    CvttPs2dq256 = "x86.cvttps2dq256"
        => sig(Some(I32X8), &[F32X8], None, "llvm.x86.avx.cvtt.ps2dq.256"),
    CvttPd2dq256 = "x86.cvttpd2dq256"
        => sig(Some(I32X4), &[F64X4], None, "llvm.x86.avx.cvtt.pd2dq.256"),
    /// Zeroes the upper halves of the YMM registers (`vzeroupper`), which
    /// holds no value of the program. Intel SDM Vol. 2C, "VZEROUPPER".
    Vzeroupper = "x86.vzeroupper" => sig(None, &[], None, "llvm.x86.avx.vzeroupper"),
    /// Zeroes the YMM registers (`vzeroall`).
    Vzeroall = "x86.vzeroall" => sig(None, &[], None, "llvm.x86.avx.vzeroall"),
    /// SSE4.2 string comparisons of implicit length, which end at the first
    /// zero element (`pcmpistri`, `pcmpistrm`): the immediate selects the
    /// element type, the aggregation and the polarity, and each builtin
    /// returns the index, the mask or one flag. Intel SDM Vol. 1, §12.10.2,
    /// "Implicit Length and Explicit Length String Instructions", and
    /// Vol. 2B, "PCMPISTRI".
    Pcmpistri128 = "x86.pcmpistri128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistri128"),
    Pcmpistrm128 = "x86.pcmpistrm128"
        => sig(Some(I8X16), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistrm128"),
    Pcmpistria128 = "x86.pcmpistria128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistria128"),
    Pcmpistric128 = "x86.pcmpistric128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistric128"),
    Pcmpistrio128 = "x86.pcmpistrio128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistrio128"),
    Pcmpistris128 = "x86.pcmpistris128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistris128"),
    Pcmpistriz128 = "x86.pcmpistriz128"
        => sig(Some(I32), &[I8X16, I8X16], Some(Type::I8), "llvm.x86.sse42.pcmpistriz128"),
    /// The explicit-length forms, whose lengths are the `i32` operands
    /// (`pcmpestri`, `pcmpestrm`).
    Pcmpestri128 = "x86.pcmpestri128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestri128"),
    Pcmpestrm128 = "x86.pcmpestrm128"
        => sig(Some(I8X16), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestrm128"),
    Pcmpestria128 = "x86.pcmpestria128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestria128"),
    Pcmpestric128 = "x86.pcmpestric128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestric128"),
    Pcmpestrio128 = "x86.pcmpestrio128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestrio128"),
    Pcmpestris128 = "x86.pcmpestris128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestris128"),
    Pcmpestriz128 = "x86.pcmpestriz128"
        => sig(Some(I32), &[I8X16, I32, I8X16, I32], Some(Type::I8), "llvm.x86.sse42.pcmpestriz128"),
}

impl Intrinsic {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|item| item.name() == name)
    }
}

impl core::fmt::Display for Intrinsic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
