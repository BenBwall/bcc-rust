//! The x86 builtins bcc's intrinsic headers call (`src/headers/`), and
//! Clang's elementwise vector builtins (`elementwise.rs`).
//!
//! A vector has no IR type: it lives in memory, and its value is the
//! address of an object holding it (`vectors.rs`). Most builtins are
//! expanded here into scalar code lane by lane, as Clang's `CGBuiltin.cpp`
//! expands them into generic LLVM IR, so every back end runs them and
//! LLVM's SLP vectorizer may put the lanes back together at `-O2`. Each
//! builtin reads the lanes of its operands' objects as integers of the lane
//! width, computes each result lane with scalar instructions, and stores it
//! into a fresh object, the result. Pure lane movement (shuffles, blends,
//! inserts and extracts) copies lanes as integers so that every bit of a
//! NaN survives.
//!
//! The instructions whose result no generic code computes exactly, or only
//! at great length, become one `intrinsic` instruction naming them
//! (`ir/intrinsics.rs`): AES rounds, the carry-less product, rounding,
//! conversions in MXCSR's rounding mode, the approximate reciprocals,
//! `vzeroupper` and `vzeroall`, and SSE4.2's string comparisons.
//!
//! For `_mm256_add_epi16`'s neighbour `_mm256_mulhi_epi16(a, b)`, which
//! calls `__builtin_ia32_pmulhw256(a, b)`, lowering makes a 32-byte stack
//! object and, for each of the 16 lanes, loads the `i16` lane of `a` and of
//! `b`, sign-extends both to `i32`, multiplies, shifts right by 16, and
//! stores the truncated product into the object.
//!
//! Each builtin cites the instruction it is (Intel SDM; Intel Intrinsics
//! Guide) and its signature in Clang's `BuiltinsX86.td`, which
//! `builtins/x86.rs` records.
//!
//! - `x86/permutes.rs`: lane movement selected by an immediate or a vector of
//!   indices.
//! - `x86/arithmetic.rs`: lane arithmetic, packs, shifts and the scalar carry
//!   chains.
//! - `x86/masks.rs`: comparisons, sign-bit masks and tests, and the masked
//!   loads, stores and gathers.

use super::{
    FunctionLowerer,
    LoweringError,
    alignment::at_offset,
    types::Repr,
    vectors::vector_address,
    work::{
        Item,
        Operand,
        Task,
    },
};
use crate::{
    builtins::BuiltinId,
    ir::{
        Align,
        InstData,
        InstFlags,
        Intrinsic,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            Callee,
            TypeId,
        },
    },
    util::bump::ArenaVec,
};

mod arithmetic;
mod masks;
mod permutes;

/// What a target builtin produced.
pub(super) enum Produced {
    /// The address of the result vector's object.
    Vector(Value),
    Scalar(Value),
    Nothing,
}

/// The operands of a target builtin call, after prototype conversion: each
/// vector's address or each scalar's value, and the value of each operand
/// that is an integer constant expression, converted to its parameter's
/// type (`None` for the others).
pub(super) struct Operands<'s> {
    pub(super) values:    ArenaVec<'s, Value>,
    pub(super) constants: ArenaVec<'s, Option<i128>>,
    pub(super) source:    SourceVectors,
}

impl Operands<'_> {
    /// Operand `index`'s address or value.
    pub(super) fn get(&self, index: usize) -> Value {
        self.values[index]
    }

    /// The immediate operand `index`, as the instruction's 8-bit field
    /// encodes it.
    pub(super) fn imm(&self, index: usize) -> Result<u64, LoweringError> {
        self.byte(index).map(u64::from)
    }

    /// The low byte of the immediate operand `index`. Semantic analysis
    /// refuses a call whose immediate is no integer constant expression, so
    /// a missing value is an internal error, never an immediate of zero.
    pub(super) fn byte(&self, index: usize) -> Result<u8, LoweringError> {
        let value = self.constants[index].ok_or_else(|| {
            LoweringError::missing("an immediate operand has no constant value", self.source)
        })?;
        Ok(value.to_le_bytes()[0])
    }
}

/// The integer type of a lane of `bytes` bytes.
pub(super) const fn int_of(bytes: u64) -> Type {
    match bytes {
        | 1 => Type::I8,
        | 2 => Type::I16,
        | 4 => Type::I32,
        | 8 => Type::I64,
        | _ => Type::I128,
    }
}

impl<'tu, 's> FunctionLowerer<'_, '_, 'tu, '_, 's> {
    /// The builtin `e` calls, if lowering expands it here.
    fn target_builtin(&self, e: &'tu Expression<'tu>) -> Option<BuiltinId> {
        let call = self.unit.sema.call_facts(e)?;
        let Callee::Builtin {
            binding: Some(binding),
            ..
        } = call.callee
        else {
            return None;
        };
        let id = self.unit.sema.bindings[binding].origin.descriptor()?.id;
        (expands(id) || super::elementwise::expands(id)).then_some(id)
    }

    /// Schedules a call of a target builtin lowering expands: each of its
    /// operands, then the builtin.
    pub(super) fn schedule_target_call(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> Result<bool, LoweringError> {
        if self.target_builtin(e).is_none() {
            return Ok(false);
        }
        let arguments = self.call_arguments(e)?;
        self.tasks.push(Task::FinishTargetBuiltin(e));
        for &argument in arguments.iter().rev() {
            self.tasks.push(Task::Evaluate(argument));
        }
        Ok(true)
    }

    /// Completes a target builtin whose operands are on the stack.
    pub(super) fn finish_target_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = e.source_vectors;
        let ty = self.info(e)?.ty;
        let id = self
            .target_builtin(e)
            .ok_or_else(|| LoweringError::missing("a target builtin has no callee", source))?;
        let arguments = self.call_arguments(e)?;
        let mut items = ArenaVec::with_capacity_in(arguments.len(), self.scratch);
        for _ in 0..arguments.len() {
            items.push(self.pop());
        }
        items.reverse();
        if super::elementwise::expands(id) {
            let item = self.elementwise(id, &items, ty, source)?;
            self.operands.push(item);
            return Ok(());
        }
        let facts = self
            .unit
            .sema
            .call_facts(e)
            .ok_or_else(|| LoweringError::missing("call facts are not retained", source))?;
        let mut operands = Operands {
            values: ArenaVec::with_capacity_in(items.len(), self.scratch),
            constants: ArenaVec::with_capacity_in(items.len(), self.scratch),
            source,
        };
        for ((&item, &argument), fact) in items.iter().zip(arguments.iter()).zip(facts.arguments) {
            let value = match self.repr(item.ty, source)? {
                | Repr::Aggregate => vector_address(item, source)?,
                | _ => self.value(item, source)?,
            };
            operands.values.push(value);
            let constant = self.converted_constant(argument, fact.ty, source)?;
            operands.constants.push(constant);
        }
        let operand = match self.x86_builtin(id, &operands, source)? {
            | Produced::Vector(object) => Operand::Aggregate(object),
            | Produced::Scalar(value) => Operand::Value(value),
            | Produced::Nothing => Operand::Void,
        };
        self.operands.push(Item { operand, ty });
        Ok(())
    }

    /// The value of `argument`, if it is an integer constant expression,
    /// after its conversion to the parameter type `ty`: the value whose
    /// limits semantic analysis checked, so `0x8000000000000101ULL` passed to
    /// an `int` parameter is 257 (C99 §6.5.2.2p7, p. 72; PDF p. 84). Under
    /// GCC's identity semantic analysis checks the value as GCC's `int`
    /// parameter receives it instead, which has the same low byte, the only
    /// one an immediate encodes, because no immediate's parameter type is
    /// narrower than a byte (C99 §6.3.1.3p2-3, p. 43; PDF p. 55).
    fn converted_constant(
        &self,
        argument: &'tu Expression<'tu>,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Option<i128>, LoweringError> {
        let info = self.info(argument)?;
        let Some(integer) = info.integer.filter(|_| info.ice) else {
            return Ok(None);
        };
        Ok(match self.repr(ty.unqualified(), source)? {
            | Repr::Int { signed, width, .. } => Some(integer.cast(u32::from(width), signed).value),
            | _ => None,
        })
    }

    /// The argument expressions of the call `e`.
    pub(super) fn call_arguments(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Result<ArenaVec<'s, &'tu Expression<'tu>>, LoweringError> {
        let source = e.source_vectors;
        let call = self
            .unit
            .sema
            .call_facts(e)
            .ok_or_else(|| LoweringError::missing("call facts are not retained", source))?;
        let mut arguments = ArenaVec::with_capacity_in(call.arguments.len(), self.scratch);
        for argument in call.arguments {
            let expression = self
                .unit
                .sema
                .expressions
                .get(argument.expression.0)
                .ok_or_else(|| LoweringError::missing("a call argument is not retained", source))?
                .expression;
            arguments.push(expression);
        }
        Ok(arguments)
    }

    /// Expands the x86 builtin `id` on `operands`.
    fn x86_builtin(
        &mut self,
        id: BuiltinId,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Produced, LoweringError> {
        use BuiltinId as B;
        let named = |intrinsic| Ok(Named(intrinsic));
        let expanded: Result<Named, LoweringError> = match id {
            | B::X86Aesenc128 => named(Intrinsic::AesEnc),
            | B::X86Aesenclast128 => named(Intrinsic::AesEncLast),
            | B::X86Aesdec128 => named(Intrinsic::AesDec),
            | B::X86Aesdeclast128 => named(Intrinsic::AesDecLast),
            | B::X86Aesimc128 => named(Intrinsic::AesImc),
            | B::X86Aeskeygenassist128 => named(Intrinsic::AesKeygenAssist),
            | B::X86Pclmulqdq128 => named(Intrinsic::Pclmulqdq),
            | B::X86Rcpps256 => named(Intrinsic::RcpPs256),
            | B::X86Rsqrtps256 => named(Intrinsic::RsqrtPs256),
            | B::X86Roundps => named(Intrinsic::RoundPs),
            | B::X86Roundpd => named(Intrinsic::RoundPd),
            | B::X86Roundps256 => named(Intrinsic::RoundPs256),
            | B::X86Roundpd256 => named(Intrinsic::RoundPd256),
            | B::X86Roundss => named(Intrinsic::RoundSs),
            | B::X86Roundsd => named(Intrinsic::RoundSd),
            | B::X86Cvtps2dq256 => named(Intrinsic::CvtPs2dq256),
            | B::X86Cvtpd2dq256 => named(Intrinsic::CvtPd2dq256),
            | B::X86Cvttps2dq256 => named(Intrinsic::CvttPs2dq256),
            | B::X86Cvttpd2dq256 => named(Intrinsic::CvttPd2dq256),
            | B::X86Vzeroupper => named(Intrinsic::Vzeroupper),
            | B::X86Xgetbv => named(Intrinsic::Xgetbv),
            | B::X86Vzeroall => named(Intrinsic::Vzeroall),
            | B::X86Pcmpistri128 => named(Intrinsic::Pcmpistri128),
            | B::X86Pcmpistrm128 => named(Intrinsic::Pcmpistrm128),
            | B::X86Pcmpistria128 => named(Intrinsic::Pcmpistria128),
            | B::X86Pcmpistric128 => named(Intrinsic::Pcmpistric128),
            | B::X86Pcmpistrio128 => named(Intrinsic::Pcmpistrio128),
            | B::X86Pcmpistris128 => named(Intrinsic::Pcmpistris128),
            | B::X86Pcmpistriz128 => named(Intrinsic::Pcmpistriz128),
            | B::X86Pcmpestri128 => named(Intrinsic::Pcmpestri128),
            | B::X86Pcmpestrm128 => named(Intrinsic::Pcmpestrm128),
            | B::X86Pcmpestria128 => named(Intrinsic::Pcmpestria128),
            | B::X86Pcmpestric128 => named(Intrinsic::Pcmpestric128),
            | B::X86Pcmpestrio128 => named(Intrinsic::Pcmpestrio128),
            | B::X86Pcmpestris128 => named(Intrinsic::Pcmpestris128),
            | B::X86Pcmpestriz128 => named(Intrinsic::Pcmpestriz128),
            | _ => return self.expand_x86(id, operands, source),
        };
        let Named(intrinsic) = expanded?;
        self.named_intrinsic(intrinsic, operands, source)
    }

    /// One `intrinsic` instruction: the destination of a vector result,
    /// then the operands its signature lists; the immediate, if it takes
    /// one, is the call's last operand.
    fn named_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Produced, LoweringError> {
        let signature = intrinsic.signature();
        let count = signature.operands.len();
        let immediate = if signature.immediate.is_some() {
            operands.byte(count)?
        } else {
            0
        };
        let destination = match signature.result {
            | Some(result) => match result.vector_bytes() {
                | Some(bytes) => Some(self.target_vector(u64::from(bytes), source)?),
                | None => None,
            },
            | None => None,
        };
        let mut values = ArenaVec::with_capacity_in(count + 1, self.scratch);
        values.extend(destination);
        values.extend_from_slice(&operands.values[..count]);
        let values = self.scratch.alloc_slice_copy(&values);
        let result = self.draft.push(
            super::ssa::Op::Intrinsic(intrinsic, immediate, values),
            signature.result_type(),
        );
        Ok(match (destination, result) {
            | (Some(object), _) => Produced::Vector(object),
            | (None, Some(value)) => Produced::Scalar(value),
            | (None, None) => Produced::Nothing,
        })
    }

    /// The builtins expanded into lane code.
    fn expand_x86(
        &mut self,
        id: BuiltinId,
        operands: &Operands<'_>,
        source: SourceVectors,
    ) -> Result<Produced, LoweringError> {
        // An unspecified vector, here zero.
        if id == BuiltinId::X86Undef256 {
            let object =
                self.fill_lanes(Type::I64, 4, source, |this, _| this.iconst(Type::I64, 0))?;
            return Ok(Produced::Vector(object));
        }
        if let Some(produced) = self.permute(id, operands, source)? {
            return Ok(produced);
        }
        if let Some(produced) = self.lane_arithmetic(id, operands, source)? {
            return Ok(produced);
        }
        if let Some(produced) = self.mask_builtin(id, operands, source)? {
            return Ok(produced);
        }
        Err(LoweringError::missing(
            "an x86 builtin has no expansion",
            source,
        ))
    }
}

/// An intrinsic chosen for a builtin.
struct Named(Intrinsic);

/// Whether lowering expands the x86 builtin `id`: those bcc's intrinsic
/// headers call. Lowering refuses the other x86 builtins by name.
pub(crate) const fn expands(id: BuiltinId) -> bool {
    use BuiltinId as B;
    matches!(
        id,
        B::X86AddcarryxU32
            | B::X86AddcarryxU64
            | B::X86SubborrowU32
            | B::X86SubborrowU64
            | B::X86Addsubpd256
            | B::X86Addsubps256
            | B::X86Aesdec128
            | B::X86Aesdeclast128
            | B::X86Aesenc128
            | B::X86Aesenclast128
            | B::X86Aesimc128
            | B::X86Aeskeygenassist128
            | B::X86Blendpd
            | B::X86Blendpd256
            | B::X86Blendps
            | B::X86Blendps256
            | B::X86Blendvpd256
            | B::X86Blendvps256
            | B::X86Cmppd
            | B::X86Cmppd256
            | B::X86Cmpps
            | B::X86Cmpps256
            | B::X86Cmpsd
            | B::X86Cmpss
            | B::X86Cvtpd2dq256
            | B::X86Cvtpd2ps256
            | B::X86Cvtps2dq256
            | B::X86Cvttpd2dq256
            | B::X86Cvttps2dq256
            | B::X86Dppd
            | B::X86Dpps
            | B::X86Dpps256
            | B::X86Extract128i256
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
            | B::X86GatherqQ256
            | B::X86Haddpd256
            | B::X86Haddps256
            | B::X86Hsubpd256
            | B::X86Hsubps256
            | B::X86Insert128i256
            | B::X86Insertps128
            | B::X86Lddqu256
            | B::X86Maskloadd
            | B::X86Maskloadd256
            | B::X86Maskloadpd
            | B::X86Maskloadpd256
            | B::X86Maskloadps
            | B::X86Maskloadps256
            | B::X86Maskloadq
            | B::X86Maskloadq256
            | B::X86Maskstored
            | B::X86Maskstored256
            | B::X86Maskstorepd
            | B::X86Maskstorepd256
            | B::X86Maskstoreps
            | B::X86Maskstoreps256
            | B::X86Maskstoreq
            | B::X86Maskstoreq256
            | B::X86Maxpd256
            | B::X86Maxps256
            | B::X86Minpd256
            | B::X86Minps256
            | B::X86Movmskpd256
            | B::X86Movmskps256
            | B::X86Mpsadbw128
            | B::X86Mpsadbw256
            | B::X86Packssdw256
            | B::X86Packsswb256
            | B::X86Packusdw256
            | B::X86Packuswb256
            | B::X86Palignr128
            | B::X86Palignr256
            | B::X86Pavgb256
            | B::X86Pavgw256
            | B::X86Pblendd128
            | B::X86Pblendd256
            | B::X86Pblendvb256
            | B::X86Pblendw128
            | B::X86Pblendw256
            | B::X86Pclmulqdq128
            | B::X86Pcmpestri128
            | B::X86Pcmpestria128
            | B::X86Pcmpestric128
            | B::X86Pcmpestrio128
            | B::X86Pcmpestris128
            | B::X86Pcmpestriz128
            | B::X86Pcmpestrm128
            | B::X86Pcmpistri128
            | B::X86Pcmpistria128
            | B::X86Pcmpistric128
            | B::X86Pcmpistrio128
            | B::X86Pcmpistris128
            | B::X86Pcmpistriz128
            | B::X86Pcmpistrm128
            | B::X86Permdf256
            | B::X86Permdi256
            | B::X86Permti256
            | B::X86Permvarsf256
            | B::X86Permvarsi256
            | B::X86Phaddd256
            | B::X86Phaddsw256
            | B::X86Phaddw256
            | B::X86Phsubd256
            | B::X86Phsubsw256
            | B::X86Phsubw256
            | B::X86Pmaddubsw256
            | B::X86Pmaddwd256
            | B::X86Pmovmskb256
            | B::X86Pmuldq256
            | B::X86Pmulhrsw256
            | B::X86Pmulhuw256
            | B::X86Pmulhw256
            | B::X86Pmuludq256
            | B::X86Psadbw256
            | B::X86Pshufb256
            | B::X86Pshufd
            | B::X86Pshufd256
            | B::X86Pshufhw
            | B::X86Pshufhw256
            | B::X86Pshuflw
            | B::X86Pshuflw256
            | B::X86Psignb256
            | B::X86Psignd256
            | B::X86Psignw256
            | B::X86Pslld256
            | B::X86Pslldi256
            | B::X86Pslldqi128Byteshift
            | B::X86Pslldqi256Byteshift
            | B::X86Psllq256
            | B::X86Psllqi256
            | B::X86Psllv2di
            | B::X86Psllv4di
            | B::X86Psllv4si
            | B::X86Psllv8si
            | B::X86Psllw256
            | B::X86Psllwi256
            | B::X86Psrad256
            | B::X86Psradi256
            | B::X86Psrav4si
            | B::X86Psrav8si
            | B::X86Psraw256
            | B::X86Psrawi256
            | B::X86Psrld256
            | B::X86Psrldi256
            | B::X86Psrldqi128Byteshift
            | B::X86Psrldqi256Byteshift
            | B::X86Psrlq256
            | B::X86Psrlqi256
            | B::X86Psrlv2di
            | B::X86Psrlv4di
            | B::X86Psrlv4si
            | B::X86Psrlv8si
            | B::X86Psrlw256
            | B::X86Psrlwi256
            | B::X86Ptestc256
            | B::X86Ptestnzc256
            | B::X86Ptestz256
            | B::X86Rcpps256
            | B::X86Roundpd
            | B::X86Roundpd256
            | B::X86Roundps
            | B::X86Roundps256
            | B::X86Roundsd
            | B::X86Roundss
            | B::X86Rsqrtps256
            | B::X86Shufpd
            | B::X86Shufpd256
            | B::X86Shufps
            | B::X86Shufps256
            | B::X86Undef256
            | B::X86VecExtV16hi
            | B::X86VecExtV16qi
            | B::X86VecExtV2di
            | B::X86VecExtV32qi
            | B::X86VecExtV4di
            | B::X86VecExtV4hi
            | B::X86VecExtV4sf
            | B::X86VecExtV4si
            | B::X86VecExtV8hi
            | B::X86VecExtV8si
            | B::X86VecSetV16hi
            | B::X86VecSetV16qi
            | B::X86VecSetV2di
            | B::X86VecSetV32qi
            | B::X86VecSetV4di
            | B::X86VecSetV4hi
            | B::X86VecSetV4si
            | B::X86VecSetV8hi
            | B::X86VecSetV8si
            | B::X86Vextractf128Pd256
            | B::X86Vextractf128Ps256
            | B::X86Vextractf128Si256
            | B::X86Vinsertf128Pd256
            | B::X86Vinsertf128Ps256
            | B::X86Vinsertf128Si256
            | B::X86Vperm2f128Pd256
            | B::X86Vperm2f128Ps256
            | B::X86Vperm2f128Si256
            | B::X86Vpermilpd
            | B::X86Vpermilpd256
            | B::X86Vpermilps
            | B::X86Vpermilps256
            | B::X86Vpermilvarpd
            | B::X86Vpermilvarpd256
            | B::X86Vpermilvarps
            | B::X86Vpermilvarps256
            | B::X86Vtestcpd
            | B::X86Vtestcpd256
            | B::X86Vtestcps
            | B::X86Vtestcps256
            | B::X86Vtestnzcpd
            | B::X86Vtestnzcpd256
            | B::X86Vtestnzcps
            | B::X86Vtestnzcps256
            | B::X86Vtestzpd
            | B::X86Vtestzpd256
            | B::X86Vtestzps
            | B::X86Vtestzps256
            | B::X86Vzeroall
            | B::X86Vzeroupper
            | B::X86Xgetbv
    )
}

// Lanes

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// A fresh object for a result vector of `bytes` bytes, aligned as a
    /// vector of that size.
    pub(super) fn target_vector(
        &mut self,
        bytes: u64,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let align = Align::from_bytes(bytes.clamp(1, 32)).expect("a power of two");
        let address = self.fresh_object(bytes, align, source)?;
        self.retain_alignment(address, align);
        Ok(address)
    }

    /// Lane `index` of type `ty` of the vector at `base`.
    pub(super) fn lane(&mut self, base: Value, ty: Type, index: u64) -> Value {
        let offset = index * u64::from(ty.bytes());
        let addr = self.offset_address(base, offset);
        let align = at_offset(self.place_alignment(base, Align::BYTE), u128::from(offset));
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

    /// Stores `value` as lane `index`, of its type, of the vector at `base`.
    pub(super) fn set_lane(&mut self, value: Value, base: Value, index: u64) {
        let ty = self.draft.value_type(value);
        let offset = index * u64::from(ty.bytes());
        let align = at_offset(self.place_alignment(base, Align::BYTE), u128::from(offset));
        let addr = self.offset_address(base, offset);
        self.retain_alignment(addr, align);
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty,
                flags: InstFlags::empty(),
                align,
                tag: None,
                args: [value, addr],
            }),
            None,
        );
    }

    /// A fresh vector of `count` lanes of type `ty`, lane `i` computed by
    /// `lane(self, i)`.
    pub(super) fn fill_lanes(
        &mut self,
        ty: Type,
        count: u64,
        source: SourceVectors,
        mut lane: impl FnMut(&mut Self, u64) -> Value,
    ) -> Result<Value, LoweringError> {
        let object = self.target_vector(u64::from(ty.bytes()) * count, source)?;
        for index in 0..count {
            let value = lane(self, index);
            self.set_lane(value, object, index);
        }
        Ok(object)
    }

    /// `a op b` without flags.
    pub(super) fn bin(&mut self, opcode: Opcode, a: Value, b: Value) -> Value {
        self.binary_inst(opcode, InstFlags::empty(), a, b)
    }

    /// Whether the integer `value`'s sign bit is set, as an `i1`.
    pub(super) fn sign_set(&mut self, value: Value) -> Value {
        let ty = self.draft.value_type(value);
        let zero = self.iconst(ty, 0);
        self.icmp(crate::ir::IntCC::Slt, value, zero)
    }
}
