//! Diagnostics from lexing, preprocessing, parsing, semantic analysis, and
//! extension policy share one error enum. Each variant delegates severity,
//! source ranges, and rendering to its phase error.
//!
//! C99: required diagnostics, §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! This module carries errors; each phase enforces its own rules.

use super::{
    Bump,
    Context,
    Debug,
    Diagnostic,
    Error,
    GetSourceVectors,
    Hash,
    InitialProcessorError,
    ParserError,
    PreprocessorError,
    PreprocessorTokenizerError,
    SourceVectors,
    ToDiagnostic,
    extension,
    semantic_analysis,
};

/// Diagnostics emitted while translating one preprocessing translation unit.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Error, Debug)]
pub(crate) enum TranslationError<'tu> {
    #[error(transparent)]
    InitialProcessing(InitialProcessorError),
    #[error(transparent)]
    PreprocessorTokenizining(PreprocessorTokenizerError),
    #[error(transparent)]
    Preprocessing(PreprocessorError<'tu>),
    #[error(transparent)]
    Parsing(ParserError<'tu>),
    #[error(transparent)]
    Semantic(semantic_analysis::SemanticError),
    #[error(transparent)]
    Extension(extension::ExtensionDiagnostic<'tu>),
}

impl TranslationError<'_> {
    /// Visits every provenance range this diagnostic reads from a context
    /// arena. Owned source vectors are not visited.
    pub(crate) fn for_each_source_vectors_mut(
        &mut self,
        visit: &mut impl FnMut(&mut SourceVectors),
    ) {
        match self {
            | Self::InitialProcessing(_) | Self::PreprocessorTokenizining(_) => {},
            | Self::Preprocessing(error) => visit(&mut error.source_vectors),
            | Self::Parsing(error) => error.for_each_source_vectors_mut(visit),
            | Self::Semantic(error) => {
                visit(&mut error.source_vectors);
                if let Some(previous) = &mut error.previous {
                    visit(previous);
                }
            },
            | Self::Extension(error) => visit(&mut error.source_vectors),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

impl GetSeverity for TranslationError<'_> {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::InitialProcessing(error) => error.severity(),
            | Self::PreprocessorTokenizining(error) => error.severity(),
            | Self::Preprocessing(error) => error.severity(),
            | Self::Parsing(error) => error.severity(),
            | Self::Semantic(error) => error.severity(),
            | Self::Extension(error) => error.severity(),
        }
    }
}

impl ToDiagnostic for TranslationError<'_> {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        // C99 §5.1.1.3 and footnote 8: every primary label has a location.
        _ = context.diagnostic_position(source);
        match self {
            | Self::InitialProcessing(error) => error.diagnostic_in(context, source, arena),
            | Self::PreprocessorTokenizining(error) => error.diagnostic_in(context, source, arena),
            | Self::Preprocessing(error) => error.diagnostic_in(context, source, arena),
            | Self::Parsing(error) => error.diagnostic_in(context, source, arena),
            | Self::Semantic(error) => error.diagnostic_in(context, source, arena),
            | Self::Extension(error) => error.diagnostic_in(context, source, arena),
        }
    }
}

impl GetSourceVectors for TranslationError<'_> {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors {
        match self {
            | Self::InitialProcessing(error) => error.source_vectors(context),
            | Self::PreprocessorTokenizining(error) => error.source_vectors(context),
            | Self::Preprocessing(error) => error.source_vectors(context),
            | Self::Parsing(error) => error.source_vectors(context),
            | Self::Semantic(error) => error.source_vectors(context),
            | Self::Extension(error) => error.source_vectors(context),
        }
    }
}
