//! Hands an analyzed translation unit to the middle end: lowering into an
//! `'ir` arena that the caller creates after semantic analysis, then the
//! optimizer when one is requested.
//!
//! There is no ABI lowering yet, so the pre-ABI module this returns is what
//! the back ends receive. That is sound for what lowering accepts today:
//! calls pass and return only scalars and pointers (lowering refuses
//! structures and unions passed or returned by value), and the LLVM back end
//! prints `va_arg` as LLVM's own `va_arg`.

use super::{
    Bump,
    Context,
    ParsedTranslationUnit,
};
use crate::{
    ir::Module,
    lowering::LoweringErrors,
    optimizer::{
        OptimizerOptions,
        optimize,
    },
    translation_phases::semantic_analysis::SemanticTranslationUnit,
};

/// Lowers the analyzed `syntax` for the configured target into `arena` and,
/// with `optimizer`, optimizes the module.
///
/// A unit that any phase reported an error in is refused (`sema.lowerable`)
/// with a single [`UnitHasErrors`](crate::lowering::LoweringErrorKind) error;
/// callers that have already reported those errors check
/// `sema.lowerable(context)` first. Constructs that lowering does not handle
/// yet come back as errors with their provenance, which render through the
/// shared diagnostic interface.
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
pub(crate) fn lower_translation_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    arena: &'ir Bump,
    optimizer: Option<&OptimizerOptions<'_>>,
) -> Result<Module<'ir>, LoweringErrors<'ir>> {
    let mut scratch = Bump::new();
    let target = context.configuration.target();
    let mut module = crate::lowering::lower_translation_unit(
        context,
        syntax,
        sema,
        target,
        arena,
        &mut scratch,
    )?;
    if let Some(options) = optimizer {
        scratch.reset();
        _ = optimize(&mut module, options, &scratch);
    }
    Ok(module)
}
