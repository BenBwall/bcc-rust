//! LLVM-C back end for the post-ABI IR. `build_module` creates globals,
//! declarations, blocks and phis, then instructions and phi incoming edges.
//! `emit_module` prints that in-memory module; `compile_module` verifies it,
//! runs the selected new-pass-manager pipeline and emits an object in memory.
//! Only final executable linking starts the bundled clang/LLD driver.
//!
//! Read `function.rs` for instruction mapping, `global.rs` for relocations,
//! `symbols.rs` for linkage, aliases and the constructor/destructor arrays,
//! `api.rs` for scoped LLVM ownership and `driver.rs` for object/link output.
//! LLVM allocates its own C++ storage; all Rust compilation scratch is arena
//! allocated. Owners live on the stack and are never placed in an arena.

// LLVM construction
mod api;
mod function;
mod global;
mod symbols;
mod target;

// Host output/link tooling
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Host tooling reads environment, builds paths and captures the link driver output."
)]
mod driver;

use std::fmt;

use api::{
    Session,
    Val,
};
#[cfg(test)]
pub(crate) use driver::ClangError;
pub(crate) use driver::{
    LlvmOptions,
    OptLevel,
    OutputKind,
    TemporaryObject,
    Toolchain,
    compile_module,
};
#[cfg(test)]
use target::data_layout;

use crate::{
    ir::{
        Extension,
        InstData,
        Module,
        SigId,
    },
    target::{
        CodeOptions,
        Target,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Prints the LLVM-C module, after checking the ABI and target contracts,
/// with the default code options.
pub(crate) fn emit_module<'m>(
    module: &'m Module<'_>,
    target: Target,
    out: &mut dyn fmt::Write,
) -> Result<(), EmitError<'m>> {
    emit_module_with(module, target, CodeOptions::default(), out)
}

/// Prints the LLVM-C module with the relocation and TLS models of `code`.
pub(crate) fn emit_module_with<'m>(
    module: &'m Module<'_>,
    target: Target,
    code: CodeOptions,
    out: &mut dyn fmt::Write,
) -> Result<(), EmitError<'m>> {
    let arena = Bump::new();
    Session::with_code(&arena, target, code, |api| {
        build_module(module, target, api, &arena)?;
        out.write_str(api.print().text())?;
        Ok(())
    })
}

/// The assembly LLVM generates for the module with the relocation and TLS
/// models of `code`, at `opt`, as `compile_module` would assemble it.
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "Tests compare the assembly as an owned string."
)]
pub(crate) fn emit_assembly(
    module: &Module<'_>,
    target: Target,
    code: CodeOptions,
    opt: Option<OptLevel>,
) -> String {
    let arena = Bump::new();
    Session::with_code(&arena, target, code, |api| {
        build_module(module, target, api, &arena).unwrap();
        let buffer = api.emit(target, opt, false, true).unwrap();
        String::from_utf8(buffer.bytes().to_vec()).unwrap()
    })
}

/// Validates the IR references before inspecting or emitting call signatures.
/// C99: §6.5.2.2 paragraph 2, p. 71; PDF p. 83 (prototype argument types);
/// validity of dense signature references is an additional IR contract.
fn build_module<'m>(
    module: &'m Module<'_>,
    target: Target,
    api: &Session<'_>,
    arena: &Bump,
) -> Result<(), EmitError<'m>> {
    if !module.triple().is_empty() && Target::parse(module.triple()) != Some(target) {
        return Err(EmitError::TargetMismatch {
            module:    module.triple(),
            requested: target.triple(),
        });
    }
    // The C API's builders require well-typed operands. Validate the arena IR
    // at this safe boundary before making any construction call into LLVM.
    // Handwritten scalar test modules may retain a native LLVM va_arg, which
    // belongs to the pre-ABI profile; aggregate/extension signatures below
    // must still already be lowered.
    let errors = crate::ir::verify_module(module, crate::ir::Profile::PostAbi, arena);
    if !errors.is_empty()
        && !crate::ir::verify_module(module, crate::ir::Profile::PreAbi, arena).is_empty()
    {
        return Err(EmitError::InvalidIr);
    }
    if uses_pre_abi_signature(module) {
        return Err(EmitError::NotAbiLowered);
    }
    let (functions, globals) = symbols::declare(module, api, target, arena)?;
    symbols::special_arrays(module, api, target, arena, &functions, &globals);
    let emitter = Emitter {
        module,
        api,
        arena,
        functions: &functions,
        globals: &globals,
        labels: &[],
        indirect_returns_twice: module.returns_twice_escapes(),
    };
    // Bodies first: a relocation may take the address of a labeled block,
    // which exists once its function is built.
    let mut labels = ArenaVec::new_in(arena);
    for (id, _) in module.functions() {
        labels.push(emitter.function(id)?);
    }
    let emitter = Emitter {
        labels: &labels,
        ..emitter
    };
    for (id, _) in module.globals() {
        emitter.global(id)?;
    }
    api.verify()?;
    Ok(())
}

struct Emitter<'a, 'id, 'm, 'ir> {
    module:                 &'m Module<'ir>,
    api:                    &'a Session<'id>,
    arena:                  &'a Bump,
    functions:              &'a [Val<'id>],
    globals:                &'a [Val<'id>],
    /// The LLVM block of each label of each function, by `FuncId`.
    labels:                 &'a [&'a [api::Block<'id>]],
    /// A returns-twice function's address escapes, so every indirect call
    /// is marked `returns_twice` ([`Module::returns_twice_escapes`]).
    indirect_returns_twice: bool,
}

fn uses_pre_abi_signature(module: &Module<'_>) -> bool {
    let pre_abi = |sig: SigId| {
        let signature = module.signature(sig);
        signature
            .aggregates
            .iter()
            .any(|param| param.abi.is_pre_abi())
            || signature
                .extensions
                .iter()
                .any(|param| param.extension.is_pre_abi())
            || signature
                .result_extension
                .is_some_and(Extension::is_pre_abi)
    };
    module.functions().any(|(_, function)| {
        pre_abi(function.signature)
            || function.body.as_ref().is_some_and(|body| {
                body.blocks().any(|block| {
                    body.block_insts(block).iter().any(|&inst| {
                        matches!(*body.inst(inst), InstData::CallIndirect { sig, .. } if pre_abi(sig))
                    })
                })
            })
    })
}

/// Why a module could not be printed as LLVM IR.
#[derive(PartialEq, Eq, Debug, thiserror::Error)]
pub(crate) enum EmitError<'m> {
    #[error("the output refused the LLVM IR text")]
    Format(#[from] fmt::Error),
    #[error("LLVM: {0}")]
    Llvm(#[from] api::Message),
    #[error("the module targets {module}, but LLVM IR was requested for {requested}")]
    TargetMismatch {
        module:    &'m str,
        requested: &'static str,
    },
    /// A signature still passes structures, unions or narrow integers in
    /// the pre-ABI way, which only ABI lowering turns into the target's
    /// convention.
    #[error("the module passes structures or narrow integers as before ABI lowering")]
    NotAbiLowered,
    #[error("the input module does not verify as bcc IR")]
    InvalidIr,
    /// Two relocations of a global share bytes; the verifier only checks
    /// that each fits.
    #[error("@{global}: the relocations at offsets {first} and {second} overlap")]
    OverlappingRelocations {
        global: &'m str,
        first:  u64,
        second: u64,
    },
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
