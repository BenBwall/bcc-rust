//! Hands an analyzed translation unit to the middle end: lowering into an
//! `'ir` arena that the caller creates after semantic analysis, then the
//! optimizer when one is requested, then ABI lowering when the caller wants
//! the module a back end receives.
//!
//! The optimizer works on the pre-ABI module, where a structure passed by
//! value is still one `ptr` parameter and `va_arg` is one operation. ABI
//! lowering then fixes how the target passes each of them, so the LLVM back
//! end and the interpreter see the same calling-convention decisions;
//! `--emit=ir` prints the pre-ABI module and `--emit=ir-abi` the post-ABI
//! one.

use crate::{
    abi::{
        AbiError,
        lower_abi,
    },
    ir::{
        Module,
        Profile,
    },
    lowering::LoweringErrors,
    optimizer::{
        OptimizerOptions,
        optimize,
    },
    translation_phases::{
        Context,
        parsing::ParsedTranslationUnit,
        semantic_analysis::SemanticTranslationUnit,
    },
    util::bump::Bump,
};

/// Lowers the analyzed `syntax` for the configured target into `arena`,
/// optimizes the module with `optimizer`, and, for [`Profile::PostAbi`],
/// lowers its calls for the target's calling convention.
///
/// A unit that any phase reported an error in is refused (`sema.lowerable`)
/// with a single [`UnitHasErrors`](crate::lowering::LoweringErrorKind) error;
/// callers that have already reported those errors check
/// `sema.lowerable(context)` first. Constructs that lowering does not handle
/// yet come back as errors with their provenance, which render through the
/// shared diagnostic interface; a case ABI lowering does not handle names
/// its function.
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
pub(crate) fn lower_translation_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
    profile: Profile,
) -> Result<Module<'ir>, MiddleEndError<'ir>> {
    let mut scratch = Bump::new();
    let target = context.configuration.target();
    let mut module =
        crate::lowering::lower_translation_unit(context, syntax, sema, target, arena, &mut scratch)
            .map_err(MiddleEndError::Lowering)?;
    if let Some(options) = optimizer {
        scratch.reset();
        _ = optimize(&mut module, options, &scratch);
    }
    if profile == Profile::PostAbi {
        scratch.reset();
        lower_abi(&mut module, target, &scratch).map_err(MiddleEndError::Abi)?;
    }
    Ok(module)
}

/// Why the middle end produced no module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MiddleEndError<'ir> {
    /// Lowering refused the unit, with one error per construct.
    Lowering(LoweringErrors<'ir>),
    /// ABI lowering met a case it does not handle.
    Abi(AbiError<'ir>),
}
