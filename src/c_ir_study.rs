//! An isolated executable C IR, compiled only by the library test harness.
//!
//! `lower` snapshots integer types, logical objects, expression relations and
//! statement control from retained phase-7 facts. `evaluate` executes that
//! snapshot without the syntax tree, scalar IR, optimizer or target ABI.
//! Neither path participates in ordinary compilation.
//!
//! Read `lower`, then `lowering::Lowerer::run` and `eval::evaluate`.
//! `model` owns the contract, `arithmetic` its integer implementation,
//! `lowering` the explicit syntax work stacks, and `tests` differential and
//! semantic litmus cases. The scalar/LLVM boundary migration is later work.
//!
//! C99: §6.2.4p5, p. 32; PDF p. 44 (automatic lifetime);
//! §6.3.1.3, p. 43; PDF p. 55 (integer conversion);
//! §6.5p2,5, p. 67; PDF p. 79 (sequencing and undefined operations);
//! §6.8, pp. 131-139; PDF pp. 143-151 (control flow).

mod arithmetic;
mod eval;
mod lowering;
mod model;
mod verify;

use lowering::Lowerer;
use model::{
    Error,
    Failure,
    Module,
    Origin,
    Signature,
};

use crate::{
    configuration::CStandard,
    translation_phases::{
        Context,
        semantic_analysis::{
            CleanupKind,
            SemanticTranslationUnit,
        },
    },
    util::bump::{
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// Copies the supported C99 subset into `arena`; no syntax references escape.
fn lower<'tu, 'ir>(
    context: &Context<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
) -> Result<Module<'ir>, Error> {
    if !sema.lowerable(context) || context.configuration.standard() != CStandard::C99 {
        return Err(Error::new(
            Failure::Unsupported("requires accepted C99"),
            None,
        ));
    }
    if !sema.symbols.init_functions.is_empty()
        || !sema.symbols.aliases.is_empty()
        || !sema.symbols.refusals.is_empty()
    {
        return Err(Error::new(
            Failure::Unsupported("module symbol extensions"),
            None,
        ));
    }
    if sema
        .control
        .regions
        .iter()
        .any(|r| !matches!(r.cleanup, CleanupKind::None))
    {
        return Err(Error::new(
            Failure::Unsupported("VLA or cleanup lifetime"),
            None,
        ));
    }
    let mut signatures = ArenaVec::new_in(arena);
    for record in sema.functions {
        signatures.push(Signature::snapshot(context, sema, record, arena)?);
    }
    if signatures.is_empty() {
        return Err(Error::new(
            Failure::Unsupported("no function definitions"),
            None,
        ));
    }
    let mut functions = ArenaVec::new_in(arena);
    for (record, signature) in sema.functions.iter().zip(&signatures) {
        functions.push(Lowerer::run(
            context,
            sema,
            record,
            *signature,
            &signatures,
            arena,
        )?);
    }
    let mut origins = ArenaVec::new_in(arena);
    let mut seen = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for function in &functions {
        let handles = std::iter::once(function.signature.origin)
            .chain(function.locals.iter().map(|local| local.origin))
            .chain(function.expressions.iter().map(|e| e.origin))
            .chain(function.code.iter().map(|op| op.origin));
        for handle in handles {
            if seen.insert(handle) {
                origins.push(Origin {
                    handle,
                    spans: arena
                        .alloc_slice_fill_iter(context.get_source_vectors(handle).iter().cloned()),
                });
            }
        }
    }
    let module = Module {
        functions: arena.alloc_slice_copy(&functions),
        origins:   arena.alloc_slice_copy(&origins),
    };
    module.verify()?;
    Ok(module)
}

#[cfg(test)]
mod tests;
