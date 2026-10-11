//! ABI lowering turns a pre-ABI module into a post-ABI one: it decides how
//! the target's calling convention passes each structure or union argument
//! and result, rewrites signatures, callers and callees to match, and
//! expands `va_arg` into loads from the target's `va_list`. Both back ends
//! then see the same calling-convention decisions.
//!
//! [`lower_abi`] works in three steps. It first checks every `va_arg` type
//! and plans every interned signature with the target's rules
//! ([`plan_signature`]), so an unsupported case is reported before anything
//! changes. It then gives each function its lowered signature. Last, it
//! rebuilds each defined function's body ([`rewrite::rewrite_body`]): the
//! entry block receives the lowered parameters and rebuilds each aggregate
//! that arrived in registers in a stack slot; each call packs the
//! aggregates it passes in registers from their copies; and each `va_arg`
//! becomes the target's sequence of loads, branches and stores.
//!
//! For `struct pair { long a, b; } swap(struct pair p)` on System V x86-64,
//! the pre-ABI signature is `(ptr result(size 16, ...), ptr aggregate(size
//! 16, ...))`. Both eightbytes of the pair are INTEGER, so the lowered
//! signature is `(ptr registers(i64, i64, align 8), i64, i64)`: the result
//! comes back in RAX and RDX and the argument travels in two general
//! registers. The callee's entry block stores the two `i64` parameters into
//! a 16-byte slot that stands for the old pointer parameter; each caller
//! loads the two halves of its argument copy and passes them. On Win64 the
//! same pair is 16 bytes, which is not 1, 2, 4 or 8, so the result becomes
//! `sret` and the argument stays a pointer to the caller's copy.
//!
//! Read [`lower_abi`], then [`plan_signature`] and the plan types in
//! `plan.rs`, then `rewrite.rs`.
//!
//! - Decisions: `plan.rs` holds the target-independent plan of a signature and
//!   builds the lowered signature from it; `sysv.rs` classifies aggregates into
//!   eightbyte classes, assigns argument registers and expands `va_arg` for
//!   System V x86-64; `win64.rs` does the same for the Microsoft x64
//!   convention.
//! - Rewriting: `rewrite.rs` rebuilds a function body for its plans.
//! - Errors: `errors.rs`. Tests: `tests.rs` and `tests/`.
//!
//! The conventions are those of the System V AMD64 psABI
//! (<https://gitlab.com/x86-psABIs/x86-64-ABI>, version 1.0, §3.2.3
//! "Parameter Passing" and §3.5.7 "Variable Argument Lists") and of
//! Microsoft's x64 calling convention
//! (<https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention>),
//! as Clang implements them for C; the conformance tests link the result
//! with Clang-compiled code.
//!
//! C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83 (arguments are passed by
//! value, so a structure argument is a copy); §6.9.1 paragraph 10, p. 142;
//! PDF p. 154 (each parameter is an object of the callee); §7.15.1.1,
//! pp. 249-250; PDF pp. 261-262 (`va_arg`).

// Decisions
mod plan;
pub(crate) mod sysv;
mod win64;

// Rewriting
mod rewrite;

// Errors
mod errors;

pub(crate) use errors::{
    AbiError,
    AbiErrorKind,
};
pub(crate) use plan::Convention;
use plan::plan_signature;

use crate::{
    ir::{
        Entity,
        InstData,
        Module,
        Opcode,
        Profile,
        SigId,
    },
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Lowers every signature, call, parameter and `va_arg` of a verified
/// pre-ABI `module` for `target`, leaving a post-ABI module that the back
/// ends accept. `scratch` holds the plans and per-function working data.
///
/// An unsupported case (a `va_arg` of a type the target's expansion does not
/// handle, or an aggregate the classification cannot place) is reported
/// before the module changes.
pub(crate) fn lower_abi<'ir>(
    module: &mut Module<'ir>,
    target: Target,
    scratch: &Bump,
) -> Result<(), AbiError<'ir>> {
    for (_, function) in module.functions() {
        let Some(body) = &function.body else {
            continue;
        };
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                if let InstData::Unary {
                    opcode: Opcode::VaArg,
                    ty,
                    ..
                } = *body.inst(inst)
                {
                    rewrite::check_va_arg(target, ty).map_err(|kind| AbiError {
                        function: function.name,
                        kind,
                    })?;
                }
            }
        }
    }
    let count = module.signature_count();
    let mut plans = ArenaVec::with_capacity_in(count, scratch);
    for index in 0..count {
        let sig = SigId::new(index);
        let plan = plan_signature(module, target, sig, scratch).map_err(|kind| AbiError {
            function: signature_user(module, sig),
            kind,
        })?;
        plans.push(plan);
    }
    let mut old_signatures = ArenaVec::with_capacity_in(module.functions().count(), scratch);
    old_signatures.extend(module.functions().map(|(_, function)| function.signature));
    let mut functions = ArenaVec::new_in(scratch);
    functions.extend(module.functions().map(|(func, _)| func));
    for &func in &functions {
        let old = old_signatures[func.index()];
        module.function_mut(func).signature = plans[old.index()].lowered;
    }
    for &func in &functions {
        rewrite::rewrite_body(module, func, target, &plans, &old_signatures, scratch);
    }
    module.set_profile(Profile::PostAbi);
    Ok(())
}

/// A function that uses `sig`, to name in an error about it.
fn signature_user<'ir>(module: &Module<'ir>, sig: SigId) -> &'ir str {
    let declared = module
        .functions()
        .find(|(_, function)| function.signature == sig);
    if let Some((_, function)) = declared {
        return function.name;
    }
    let calling = module.functions().find(|(_, function)| {
        function.body.as_ref().is_some_and(|body| {
            body.blocks().any(|block| {
                body.block_insts(block).iter().any(|&inst| {
                    matches!(*body.inst(inst), InstData::CallIndirect { sig: used, .. } if used == sig)
                })
            })
        })
    });
    calling.map_or("", |(_, function)| function.name)
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
