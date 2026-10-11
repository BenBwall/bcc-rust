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

/// Plans the pre-ABI signature `sig` for `target` and interns the post-ABI
/// signature it gives, whose id the plan records.
pub(crate) fn plan_signature<'ir: 's, 's>(
    module: &mut Module<'ir>,
    target: Target,
    sig: SigId,
    scratch: &'s Bump,
) -> Result<SignaturePlan<'s>, AbiErrorKind> {
    let signature = module.signature(sig);
    let convention = Convention::of(target);
    let mut args = ArenaVec::with_capacity_in(signature.params.len(), scratch);
    let mut registers = sysv::ArgumentRegisters::default();
    let result = match signature.param_abi(0) {
        | Some(ParamAbi::Result(layout)) => match convention {
            | Convention::SystemV => sysv::result(layout, &mut registers)?,
            | Convention::Win64 => win64::result(layout),
        },
        | _ => ResultPlan::Scalar,
    };
    let first = usize::from(!matches!(result, ResultPlan::Scalar));
    for (index, &ty) in signature.params.iter().enumerate().skip(first) {
        let plan = match signature.param_abi(index) {
            | Some(ParamAbi::Aggregate(layout)) => match convention {
                | Convention::SystemV => sysv::argument(layout, &mut registers)?,
                | Convention::Win64 => win64::argument(layout),
            },
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
    let lowered = lowered_signature(signature, &result, &args, scratch);
    Ok(SignaturePlan {
        result,
        args: scratch.alloc_slice_copy(&args),
        lowered: module.intern(lowered),
    })
}

/// The plan of one signature: how its result and each parameter after the
/// result's are passed, and the lowered signature.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SignaturePlan<'s> {
    pub(crate) result:  ResultPlan<'s>,
    /// One plan per pre-ABI parameter, without the aggregate result's
    /// parameter 0.
    pub(crate) args:    &'s [ArgPlan<'s>],
    pub(crate) lowered: SigId,
}

impl SignaturePlan<'_> {
    /// How many pre-ABI parameters come before the arguments: 1 for an
    /// aggregate result, else 0.
    pub(crate) const fn first_arg(&self) -> usize {
        match self.result {
            | ResultPlan::Scalar => 0,
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
}

/// The post-ABI signature of a plan: aggregates in registers become their
/// parts, ignored ones disappear, and the others keep a `ptr` described by
/// the target's decision without the scalars only classification needed.
fn lowered_signature<'s>(
    signature: Signature<'_>,
    result: &ResultPlan<'s>,
    args: &[ArgPlan<'s>],
    scratch: &'s Bump,
) -> Signature<'s> {
    let mut params = ArenaVec::with_capacity_in(signature.params.len() + 1, scratch);
    let mut aggregates = ArenaVec::new_in(scratch);
    let mut describe = |params: &ArenaVec<'_, Type>, abi| {
        let index = u32::try_from(params.len()).expect("a signature has few parameters");
        aggregates.push(AggregateParam { index, abi });
    };
    match *result {
        | ResultPlan::Scalar | ResultPlan::Ignore(_) => {},
        | ResultPlan::Memory(layout) => {
            describe(&params, ParamAbi::Sret(without_scalars(layout)));
            params.push(Type::Ptr);
        },
        | ResultPlan::Registers { registers, .. } => {
            describe(&params, ParamAbi::Registers(registers));
            params.push(Type::Ptr);
        },
    }
    let first = usize::from(!matches!(result, ResultPlan::Scalar));
    for (&ty, plan) in signature.params[first..].iter().zip(args) {
        match *plan {
            | ArgPlan::Keep => params.push(ty),
            | ArgPlan::Registers { parts, .. } => params.extend_from_slice(parts.types()),
            | ArgPlan::Stack { layout, align } => {
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
        params:     scratch.alloc_slice_copy(&params),
        result:     signature.result,
        variadic:   signature.variadic,
        aggregates: scratch.alloc_slice_copy(&aggregates),
    }
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
