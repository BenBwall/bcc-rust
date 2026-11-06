//! `-mgeneral-regs-only`: refuses a module's floating code before any back end
//! would have to generate it. GCC refuses a floating result, which its
//! arithmetic library routines return too, and a floating variable argument,
//! in the words of [`DisabledRegister`]. It compiles the rest with general
//! registers: it passes floating parameters on the stack, copies floating
//! values through general registers, and compares them with routines libgcc
//! does not provide on x86-64, so that linking fails. bcc does not generate
//! such code and refuses it by name.
//! GNU extension: GCC manual, "x86 Options".
//!
//! This also refuses a handler with variable arguments, which GCC compiles
//! though nothing can pass them. It runs on every lowered module, since that
//! check does not depend on the option.

use super::{
    Construct,
    LoweringError,
    LoweringErrorKind,
    errors::DisabledRegister,
};
use crate::{
    abi::{
        ResultPlan,
        plan_signature,
    },
    ir::{
        Body,
        Entity,
        EntryConvention,
        InstData,
        Module,
        Opcode,
        SigId,
        Type,
        Value,
    },
    target::Target,
    translation_phases::SourceVectors,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Reports each function of `module` whose code `-mgeneral-regs-only` rules
/// out, or that is a variadic handler, at its name in `sources`.
pub(super) fn check<'ir>(
    module: &Module<'ir>,
    target: Target,
    sources: &[(&'ir str, SourceVectors)],
    scratch: &mut Bump,
    errors: &mut ArenaVec<'_, LoweringError>,
) {
    let source = |name: &str| {
        sources
            .iter()
            .find(|&&(function, _)| function == name)
            .map(|&(_, source)| source)
    };
    for (_, function) in module.functions() {
        let Some(body) = &function.body else { continue };
        let kind = if function.entry == EntryConvention::Interrupt
            && module.signature(function.signature).variadic
        {
            Some(LoweringErrorKind::Unsupported(Construct::VariadicHandler))
        } else if module.general_regs_only() {
            scratch.reset();
            refusal(module, target, function.signature, body, scratch)
        } else {
            None
        };
        if let Some(kind) = kind {
            errors.push(LoweringError {
                kind,
                source: source(function.name),
            });
        }
    }
}

/// Why the function with signature `signature` and `body` needs a register
/// `-mgeneral-regs-only` takes away, if it does.
fn refusal(
    module: &Module<'_>,
    target: Target,
    signature: SigId,
    body: &Body<'_>,
    scratch: &Bump,
) -> Option<LoweringErrorKind> {
    let disabled = LoweringErrorKind::DisabledRegister;
    if let Some(register) = result_register(module, target, signature, scratch) {
        return Some(disabled(register));
    }
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            let data = body.inst(inst);
            let register = match *data {
                | InstData::Call { func, .. } =>
                    result_register(module, target, module.function(func).signature, scratch),
                | InstData::CallIndirect { sig, .. } =>
                    result_register(module, target, sig, scratch),
                | InstData::VaArgAggregate { sig, .. }
                    if module.signature(sig).aggregates.iter().any(|param| {
                        param.abi.layout().is_some_and(|layout| {
                            layout.scalars.iter().any(|scalar| scalar.ty.is_float())
                        })
                    }) =>
                    Some(DisabledRegister::SseArgument),
                | _ if data.opcode() == Opcode::VaArg
                    && module.result_type(data).is_some_and(Type::is_float) =>
                    Some(DisabledRegister::SseArgument),
                // An operation computes through a library routine, which
                // returns its value in the register.
                | _ if !matches!(data.opcode(), Opcode::Load | Opcode::Fconst) =>
                    module.result_type(data).and_then(floating_register),
                | _ => None,
            };
            if let Some(register) = register {
                return Some(disabled(register));
            }
        }
    }
    (0..body.value_count())
        .any(|index| body.value_type(Value::new(index)).is_float())
        .then_some(LoweringErrorKind::Unsupported(
            Construct::FloatingWithoutSse,
        ))
}

/// The register a function of signature `signature` returns a floating value
/// in, if it does: a System V vector result takes a whole SSE register.
fn result_register(
    module: &Module<'_>,
    target: Target,
    signature: SigId,
    scratch: &Bump,
) -> Option<DisabledRegister> {
    let plan = plan_signature(module, target, signature, scratch).ok()?;
    match plan.result {
        | ResultPlan::Registers { registers, .. } => [Some(registers.first), registers.second]
            .into_iter()
            .flatten()
            .find_map(|ty| {
                if ty == Type::I128 {
                    Some(DisabledRegister::SseReturn)
                } else {
                    floating_register(ty)
                }
            }),
        | _ => module
            .signature(signature)
            .result
            .and_then(floating_register),
    }
}

/// The register that holds a value of type `ty`, if it is floating.
const fn floating_register(ty: Type) -> Option<DisabledRegister> {
    match ty {
        | Type::F80 => Some(DisabledRegister::X87Return),
        | _ if ty.is_float() => Some(DisabledRegister::SseReturn),
        | _ => None,
    }
}
