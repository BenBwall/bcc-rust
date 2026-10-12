//! The plan of a signature: how the target passes each parameter and the
//! result, independent of which target decided it, and the post-ABI
//! signature it gives.

use super::{
    AbiErrorKind,
    sysv,
    win64,
};
use crate::{
    ir::{
        AggregateLayout,
        AggregateParam,
        Align,
        ExtendedParam,
        Extension,
        Module,
        ParamAbi,
        ResultRegisters,
        SigId,
        Signature,
        Type,
    },
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Plans the pre-ABI signature `sig` for `target` without changing the
/// module. Checks storage and parameter-count limits before rewriting.
/// C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83; §6.8.6.4 paragraph 3,
/// p. 139; PDF p. 151. `__int128` is an extension; Win64 passes it through
/// caller-owned storage and returns its bits in XMM0, as bundled Clang does.
pub(crate) fn plan_signature<'ir: 's, 's>(
    module: &Module<'ir>,
    target: Target,
    sig: SigId,
    scratch: &'s Bump,
) -> Result<SignaturePlan<'s>, AbiErrorKind> {
    let signature = module.signature(sig);
    _ = parameter_count(signature.params.len())?;
    for param in signature.aggregates {
        if let Some(layout) = param.abi.layout() {
            _ = stack_slot_size(layout.size)?;
        }
    }
    let convention = Convention::of(target);
    let mut args = ArenaVec::with_capacity_in(signature.params.len(), scratch);
    let mut registers = sysv::ArgumentRegisters::default();
    let result = match signature.param_abi(0) {
        | Some(ParamAbi::Result(layout)) => match convention {
            | Convention::SystemV => sysv::result(layout, &mut registers)?,
            | Convention::Win64 => win64::result(layout),
        },
        | _ if convention == Convention::Win64 && signature.result == Some(Type::I128) =>
            ResultPlan::VectorI128,
        | _ => ResultPlan::Scalar,
    };
    let first = usize::from(!matches!(
        result,
        ResultPlan::Scalar | ResultPlan::VectorI128
    ));
    for (index, &ty) in signature.params.iter().enumerate().skip(first) {
        let plan = match signature.param_abi(index) {
            | Some(ParamAbi::Aggregate(layout)) => match convention {
                | Convention::SystemV => sysv::argument(layout, &mut registers)?,
                | Convention::Win64 => win64::argument(layout),
            },
            | _ if convention == Convention::Win64 && ty == Type::I128 => ArgPlan::IndirectI128,
            | _ => {
                // A scalar takes the registers it needs if they are free,
                // else it goes on the stack and leaves them for later
                // arguments.
                _ = registers.take_scalar(ty);
                ArgPlan::Keep
            },
        };
        args.push(plan);
    }
    let mut count = u32::from(matches!(
        result,
        ResultPlan::Memory(_) | ResultPlan::Registers { .. } | ResultPlan::VectorI128
    ));
    for arg in &args {
        let added = match arg {
            | ArgPlan::Ignore(_) => 0,
            | ArgPlan::Registers { parts, .. } => u32::from(parts.count),
            | _ => 1,
        };
        count = count
            .checked_add(added)
            .ok_or(AbiErrorKind::TooManyParameters)?;
    }
    Ok(SignaturePlan {
        result,
        args: scratch.alloc_slice_copy(&args),
    })
}

/// The plan of one signature: how its result and each parameter after the
/// result's are passed. The lowered signature is interned after all plans
/// succeed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SignaturePlan<'s> {
    pub(crate) result: ResultPlan<'s>,
    /// One plan per pre-ABI parameter, without the aggregate result's
    /// parameter 0.
    pub(crate) args:   &'s [ArgPlan<'s>],
}

impl SignaturePlan<'_> {
    /// How many pre-ABI parameters come before the arguments: 1 for an
    /// aggregate result, else 0.
    pub(crate) const fn first_arg(&self) -> usize {
        match self.result {
            | ResultPlan::Scalar | ResultPlan::VectorI128 => 0,
            | _ => 1,
        }
    }
}

/// How a pre-ABI parameter is passed after ABI lowering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ArgPlan<'s> {
    /// Unchanged: a scalar, or, for an aggregate, the pointer to the
    /// caller's copy (Win64 passes larger aggregates by reference).
    Keep,
    /// Win64 `__int128`: a pointer to a caller-owned, 16-aligned copy.
    IndirectI128,
    /// In registers: one scalar parameter per part, part `i` holding bytes
    /// `8 * i` onwards of the aggregate.
    Registers {
        layout: AggregateLayout<'s>,
        parts:  Parts,
    },
    /// In the stack's argument area, a copy of the aggregate (`byval`) in a
    /// stack slot aligned to `align`, which may exceed the aggregate's.
    Stack {
        layout: AggregateLayout<'s>,
        align:  Align,
    },
    /// Not passed at all, as for an empty structure on System V.
    Ignore(AggregateLayout<'s>),
}

/// How an aggregate result comes back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ResultPlan<'s> {
    /// The signature has no aggregate result.
    Scalar,
    /// Win64 scalar `__int128`: a 16-byte vector in XMM0. The post-ABI IR
    /// uses a register return area, with `i128` as its memory carrier.
    VectorI128,
    /// In memory at the address the caller passes (`sret`).
    Memory(AggregateLayout<'s>),
    /// In registers loaded from the return area.
    Registers {
        layout:    AggregateLayout<'s>,
        registers: ResultRegisters,
    },
    /// Not returned at all, as for an empty structure on System V.
    Ignore(AggregateLayout<'s>),
}

/// The scalar parts of an aggregate passed in registers, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Parts {
    types: [Type; 2],
    count: u8,
}

impl Parts {
    pub(crate) const fn one(ty: Type) -> Self {
        Self {
            types: [ty, ty],
            count: 1,
        }
    }

    pub(crate) const fn two(first: Type, second: Type) -> Self {
        Self {
            types: [first, second],
            count: 2,
        }
    }

    /// The parts' types; part `i` starts at byte `8 * i`.
    pub(crate) fn types(&self) -> &[Type] {
        &self.types[..usize::from(self.count)]
    }

    /// The bytes the parts cover, from the start of the aggregate.
    pub(crate) fn bytes(self) -> u64 {
        let last = self.types()[self.types().len() - 1];
        8 * (u64::from(self.count) - 1) + u64::from(last.bytes())
    }
}

/// The calling-convention family of a target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Convention {
    /// The System V AMD64 psABI, used by the Linux targets.
    SystemV,
    /// The Microsoft x64 convention, used by the Windows targets with either
    /// C library.
    Win64,
}

impl Convention {
    pub(crate) const fn of(target: Target) -> Self {
        match target {
            | Target::LinuxGnu | Target::LinuxMusl => Self::SystemV,
            | Target::WindowsGnu | Target::WindowsMsvc => Self::Win64,
        }
    }

    /// How a narrow integer parameter or result with the pre-ABI
    /// `extension` travels, if the convention widens it.
    const fn extension(self, extension: Extension) -> Option<Extension> {
        match self {
            | Self::SystemV => Some(sysv::extension(extension)),
            | Self::Win64 => win64::extension(extension),
        }
    }
}

/// The post-ABI signature of a plan: aggregates in registers become their
/// parts, ignored ones disappear, and the others keep a `ptr` described by
/// the target's decision without the scalars only classification needed.
/// Narrow integers keep their place with the extension `convention` gives
/// them.
pub(super) fn lowered_signature<'s>(
    signature: Signature<'_>,
    convention: Convention,
    result: &ResultPlan<'s>,
    args: &[ArgPlan<'s>],
    scratch: &'s Bump,
) -> Signature<'s> {
    let mut params = ArenaVec::with_capacity_in(signature.params.len() + 1, scratch);
    let mut aggregates = ArenaVec::new_in(scratch);
    let mut extensions = ArenaVec::new_in(scratch);
    let mut noundef = ArenaVec::new_in(scratch);
    let mut describe = |params: &ArenaVec<'_, Type>, abi| {
        let index = parameter_count(params.len()).expect("ABI planning checked parameter counts");
        aggregates.push(AggregateParam { index, abi });
    };
    match *result {
        | ResultPlan::Scalar | ResultPlan::Ignore(_) => {},
        | ResultPlan::VectorI128 => {
            describe(
                &params,
                ParamAbi::Registers(ResultRegisters {
                    first:  Type::I128,
                    second: None,
                    align:  Align::from_bytes(16).unwrap(),
                }),
            );
            params.push(Type::Ptr);
        },
        | ResultPlan::Memory(layout) => {
            // This is still the original return-area address. C lowering
            // leaves it unmarked, but explicit IR pointer contracts survive.
            if signature.param_noundef(0) {
                noundef.push(0);
            }
            describe(&params, ParamAbi::Sret(without_scalars(layout)));
            params.push(Type::Ptr);
        },
        | ResultPlan::Registers { registers, .. } => {
            describe(&params, ParamAbi::Registers(registers));
            params.push(Type::Ptr);
        },
    }
    let first = usize::from(!matches!(
        result,
        ResultPlan::Scalar | ResultPlan::VectorI128
    ));
    for ((index, &ty), plan) in signature.params.iter().enumerate().skip(first).zip(args) {
        match *plan {
            | ArgPlan::Keep => {
                if signature.param_noundef(index) {
                    noundef.push(
                        parameter_count(params.len())
                            .expect("ABI planning checked parameter counts"),
                    );
                }
                if let Some(extension) = signature
                    .param_extension(index)
                    .and_then(|extension| convention.extension(extension))
                {
                    extensions.push(ExtendedParam {
                        index: parameter_count(params.len())
                            .expect("ABI planning checked parameter counts"),
                        extension,
                    });
                }
                params.push(ty);
            },
            | ArgPlan::IndirectI128 => {
                if signature.param_noundef(index) {
                    noundef.push(
                        parameter_count(params.len())
                            .expect("ABI planning checked parameter counts"),
                    );
                }
                params.push(Type::Ptr);
            },
            | ArgPlan::Registers { parts, .. } => params.extend_from_slice(parts.types()),
            | ArgPlan::Stack { layout, align } => {
                // Clang marks the byval pointer, not its object bytes.
                // C99 §6.5.2.2p4: parameters receive argument values.
                if signature.param_noundef(index) {
                    noundef.push(
                        parameter_count(params.len())
                            .expect("ABI planning checked parameter counts"),
                    );
                }
                describe(
                    &params,
                    ParamAbi::ByVal(raised(without_scalars(layout), align)),
                );
                params.push(Type::Ptr);
            },
            | ArgPlan::Ignore(_) => {},
        }
    }
    Signature {
        params:           scratch.alloc_slice_copy(&params),
        result:           if matches!(result, ResultPlan::VectorI128) {
            None
        } else {
            signature.result
        },
        variadic:         signature.variadic,
        aggregates:       scratch.alloc_slice_copy(&aggregates),
        extensions:       scratch.alloc_slice_copy(&extensions),
        result_extension: signature
            .result_extension
            .and_then(|extension| convention.extension(extension)),
        noundef:          scratch.alloc_slice_copy(&noundef),
        result_noundef:   signature.result_noundef,
        variadic_noundef: signature.variadic_noundef,
    }
}

/// Stack slots use `u32` byte sizes. Both ABI families share this limit
/// for aggregate parameters and results, including caller-owned storage.
/// Wider layouts remain valid IR; ABI planning refuses them before mutation.
pub(super) fn stack_slot_size(size: u64) -> Result<u32, AbiErrorKind> {
    u32::try_from(size).map_err(|_| AbiErrorKind::AggregateTooLarge(size))
}

/// LLVM and IR parameter indices use `u32`; check input and expanded counts.
fn parameter_count(count: usize) -> Result<u32, AbiErrorKind> {
    u32::try_from(count).map_err(|_| AbiErrorKind::TooManyParameters)
}

/// `layout` without its scalars, which only classification reads.
pub(crate) fn without_scalars(layout: AggregateLayout<'_>) -> AggregateLayout<'static> {
    AggregateLayout {
        size:    layout.size,
        align:   layout.align,
        flags:   layout.flags,
        scalars: &[],
    }
}

/// `layout` aligned to at least `align`, as a stack slot may need to be.
pub(crate) fn raised(layout: AggregateLayout<'_>, align: Align) -> AggregateLayout<'_> {
    AggregateLayout {
        align: if align.bytes() > layout.align.bytes() {
            align
        } else {
            layout.align
        },
        ..layout
    }
}
