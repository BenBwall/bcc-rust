//! The runtime of the wide bit-precise integers: the C routines in
//! `runtime.c`, which the middle end compiles with bcc for the program's
//! target whenever a module calls one (`pipeline::middle_end`), and the
//! linking that moves the routines a module calls into it as internal
//! functions. Only the routines reached from the module's calls are kept.
//! Both modules live in one arena, so a body moves as it is; its calls to
//! other routines are renumbered for the module. The routines refer to no
//! global, function address or assembly, which [`link`] checks.

use crate::{
    diagnostics::format_in,
    ir::{
        Body,
        FuncId,
        InstData,
        Linkage,
        Module,
        Symbol,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The runtime's C source.
pub(crate) const SOURCE: &str = include_str!("runtime.c");

/// The prefix of every runtime routine's name in `runtime.c`.
const PREFIX: &str = "__bcc_bitint_";

/// The prefix of the routines' names in a module that calls them. No C
/// identifier contains a `.`, so a program's own function, even one named
/// like a routine of `runtime.c`, never takes one of these names.
const MODULE_PREFIX: &str = "bcc.bitint.";

/// Whether `module` declares a runtime routine without defining it.
pub(crate) fn needed(module: &Module<'_>) -> bool {
    module
        .functions()
        .any(|(_, function)| declared(function.name, function.body.as_ref(), function.linkage))
}

/// Defines each runtime routine `module` declares with the body `runtime`
/// gives it, and the routines those call, moving the bodies out of
/// `runtime`. The error names what a routine needed that linking cannot
/// carry, a bug of bcc.
pub(crate) fn link<'ir>(
    module: &mut Module<'ir>,
    runtime: &mut Module<'ir>,
    scratch: &Bump,
) -> Result<(), &'static str> {
    let mut pending = ArenaVec::new_in(scratch);
    pending.extend(
        module
            .functions()
            .filter(|(_, function)| {
                declared(function.name, function.body.as_ref(), function.linkage)
            })
            .map(|(id, _)| id),
    );
    while let Some(target) = pending.pop() {
        let name = module.function(target).name;
        let Some(Symbol::Function(source)) = name
            .strip_prefix(MODULE_PREFIX)
            .and_then(|routine| runtime.symbol(format_in!(scratch, "{PREFIX}{routine}")))
        else {
            return Err("a _BitInt runtime routine is missing");
        };
        let Some(mut body) = runtime.function_mut(source).body.take() else {
            return Err("a _BitInt runtime routine has no body");
        };
        // The routines this one calls, as (runtime, module) pairs.
        let mut callees: ArenaVec<'_, (FuncId, FuncId)> = ArenaVec::new_in(scratch);
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                match *body.inst(inst) {
                    | InstData::Call { func, .. } => {
                        if callees.iter().any(|&(from, _)| from == func) {
                            continue;
                        }
                        let callee = runtime.function(func);
                        let signature = module.intern(runtime.signature(callee.signature));
                        let defined = callee.body.is_some();
                        let callee_name = match callee.name.strip_prefix(PREFIX) {
                            | Some(routine) => &*format_in!(scratch, "{MODULE_PREFIX}{routine}"),
                            | None => callee.name,
                        };
                        let to = match module.symbol(callee_name) {
                            | Some(Symbol::Function(to)) => to,
                            | Some(Symbol::Global(_)) =>
                                return Err("a _BitInt runtime routine's callee names a global"),
                            | None => {
                                let to = module.declare_function(
                                    callee_name,
                                    signature,
                                    if defined {
                                        Linkage::Internal
                                    } else {
                                        Linkage::External
                                    },
                                );
                                if defined {
                                    pending.push(to);
                                }
                                to
                            },
                        };
                        callees.push((func, to));
                    },
                    | InstData::GlobalAddr { .. }
                    | InstData::FuncAddr { .. }
                    | InstData::CallIndirect { .. }
                    | InstData::VaArgAggregate { .. }
                    | InstData::InlineAsm { .. } =>
                        return Err("a _BitInt runtime routine refers to a module item"),
                    | _ => {},
                }
            }
        }
        retarget(&mut body, &callees);
        let signature = module.intern(runtime.signature(runtime.function(source).signature));
        let function = module.function_mut(target);
        function.signature = signature;
        function.linkage = Linkage::Internal;
        function.body = Some(body);
    }
    Ok(())
}

/// Whether a function is a runtime routine lowering declared and nothing
/// has defined yet.
fn declared(name: &str, body: Option<&Body<'_>>, linkage: Linkage) -> bool {
    body.is_none() && linkage == Linkage::Internal && name.starts_with(MODULE_PREFIX)
}

/// Renumbers the direct calls of `body` from runtime functions to the
/// module's.
fn retarget(body: &mut Body<'_>, callees: &[(FuncId, FuncId)]) {
    body.retarget_calls(|func| {
        callees
            .iter()
            .find(|&&(from, _)| from == func)
            .map(|&(_, to)| to)
    });
}
