//! Passes a finished parsed translation unit to semantic analysis.

use super::{
    Context,
    ParsedTranslationUnit,
};

/// The semantic half of phase 7 starts after syntax parsing has completed.
/// Inspection callers may stop at `parse_translation_unit` to keep syntax modes
/// unchanged. C99: §5.1.1.2p1, p. 10; PDF p. 22.
pub(crate) fn analyze_translation_unit<'tu>(
    context: &mut Context<'tu>,
    unit: &ParsedTranslationUnit<'tu>,
) -> crate::translation_phases::semantic_analysis::SemanticTranslationUnit<'tu> {
    crate::translation_phases::semantic_analysis::analyze(context, unit)
}
