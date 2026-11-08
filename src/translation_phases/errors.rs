//! Diagnostics from lexing, preprocessing, parsing, semantic analysis, and
//! extension policy share one error enum. Each variant delegates severity,
//! source ranges, and rendering to its phase error.
//!
//! C99: required diagnostics, §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! This module carries errors; each phase enforces its own rules.

use std::{
    fmt::Debug,
    hash::Hash,
};

use thiserror::Error;

use super::{
    Context,
    GetSourceVectors,
    SourceVectors,
    extension,
    initial_processing::InitialProcessorError,
    parsing::ParserError,
    preprocessing::PreprocessorError,
    preprocessor_tokenizer::PreprocessorTokenizerError,
    semantic_analysis,
};
use crate::{
    diagnostics::{
        Diagnostic,
        ToDiagnostic,
    },
    util::bump::Bump,
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
    #[error(transparent)]
    Classified(ClassifiedError<'tu>),
}

/// A diagnostic a `#pragma GCC diagnostic` or `#pragma clang diagnostic`
/// classified where it arose, which the command line's settings cannot tell
/// when it is reported. Only a unit with such a pragma has any.
/// C99: §6.10.6 paragraph 1, p. 159; PDF p. 171.
#[derive(Error, Debug)]
#[error("{error}")]
pub(crate) struct ClassifiedError<'tu> {
    pub(crate) error:    &'tu mut TranslationError<'tu>,
    pub(crate) severity: ErrorSeverity,
    /// Clang's `#pragma clang diagnostic fatal`, after which nothing else is
    /// reported.
    pub(crate) fatal:    bool,
}

impl TranslationError<'_> {
    /// The diagnostic itself, without a pragma's classification around it.
    /// Whoever asks what a queued diagnostic is reads it through this.
    pub(crate) fn unclassified(&self) -> &Self {
        match self {
            | Self::Classified(classified) => classified.error,
            | error => error,
        }
    }

    /// Whether `__extension__` or a withdrawal has taken this diagnostic back.
    pub(crate) fn is_suppressed(&self) -> bool {
        matches!(self.unclassified(), Self::Extension(extension) if extension.suppressed.get())
    }

    /// A warning-group diagnostic takes its group's configured severity,
    /// which already includes global -Werror and the group's own overrides.
    /// Other warnings from every phase share the global -Werror control.
    pub(crate) fn configured_severity(&self, context: &Context<'_>) -> ErrorSeverity {
        if let Self::Classified(classified) = self {
            classified.severity
        } else if let Self::Parsing(error) = self
            && error.severity_is_final
        {
            error.severity
        } else if let Some(group) = self.group() {
            context
                .configuration
                .diagnostic_severity(group)
                .unwrap_or_else(|| self.severity())
        } else {
            context.configuration.warning_severity(self.severity())
        }
    }

    /// The warning group that controls this diagnostic, if any.
    pub(crate) fn group(&self) -> Option<crate::configuration::DiagnosticGroup> {
        use super::preprocessing::PreprocessorErrorType as P;
        use crate::configuration::DiagnosticGroup as G;
        match self {
            | Self::Classified(classified) => classified.error.group(),
            | Self::InitialProcessing(_) => Some(G::NewlineEof),
            | Self::Preprocessing(error) => match error.error_type {
                | P::UndefinedIdentifierInPreprocessorExpression(_) => Some(G::Undef),
                | P::UnknownPragmaDirective | P::IgnoredPragma(_) => Some(G::UnknownPragmas),
                | P::WarningDirective(_) => Some(G::Cpp),
                | P::PragmaMessage { error: false, .. } => Some(G::PragmaMessages),
                | P::StorageOrderPragmaOperand { .. } => Some(G::Pragmas),
                | P::PragmaDiagnostic(problem) => problem.group(),
                | P::ExtraTokensAfterPragmaOnce(..)
                | P::ExtraTokensAfterPragmaOperator
                | P::ExtraTokensAfterIncludeDirective
                | P::ExtraTokensAfterConditionalDirective(_)
                | P::ExtraTokensAfterIfdefDirective(_)
                | P::ExtraTokensAfterIfndefDirective(_) => Some(G::ExtraTokens),
                | P::DeprecatedMacro(..) => Some(G::DeprecatedPragma),
                | P::PragmaOnceInNonHeader => Some(G::PragmaOnceOutsideHeader),
                | P::SystemHeaderPragmaInMainFile => Some(G::PragmaSystemHeaderOutsideHeader),
                | P::IncludeNextInPrimarySource(..) => Some(G::IncludeNextOutsideHeader),
                | P::IncludeNextWithoutSearchEntry(..) => Some(G::IncludeNextAbsolutePath),
                | P::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(..)
                | P::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(..)
                | P::MacroRedefinedWithDifferentDefinition(..) => Some(G::MacroRedefined),
                | P::UnknownEscapeSequence(_) => Some(G::UnknownEscapeSequence),
                | P::IllegalCharacterEncoding {
                    prefixed: false, ..
                } => Some(G::InvalidSourceEncoding),
                | P::UnknownPragmaSTDCArgument(_) => Some(G::UnknownStdcPragma),
                | P::ForcedSignedPromotion { .. } | P::ForcedUnsignedPromotion { .. } =>
                    Some(G::ConstantPromotion),
                | _ => None,
            },
            | Self::Semantic(error) => match error.kind {
                | semantic_analysis::SemanticErrorKind::PointerConstraint(_)
                | semantic_analysis::SemanticErrorKind::Unused(_)
                | semantic_analysis::SemanticErrorKind::Main(_)
                | semantic_analysis::SemanticErrorKind::SignCompare
                | semantic_analysis::SemanticErrorKind::TypeLimits(_)
                | semantic_analysis::SemanticErrorKind::ShadowOrBody(_)
                | semantic_analysis::SemanticErrorKind::Parentheses(_)
                | semantic_analysis::SemanticErrorKind::KnrPromotedParameter
                | semantic_analysis::SemanticErrorKind::SwitchCases(_)
                | semantic_analysis::SemanticErrorKind::ReturnFlow(_)
                | semantic_analysis::SemanticErrorKind::MissingFieldInitializer
                | semantic_analysis::SemanticErrorKind::ImplicitFunctionDeclaration
                | semantic_analysis::SemanticErrorKind::ImplicitLibraryFunction(_)
                | semantic_analysis::SemanticErrorKind::ImplicitInt
                | semantic_analysis::SemanticErrorKind::ImplicitIntReturn
                | semantic_analysis::SemanticErrorKind::ImplicitIntParameter
                | semantic_analysis::SemanticErrorKind::ImplicitIntTypeName
                | semantic_analysis::SemanticErrorKind::ReturnMismatch
                | semantic_analysis::SemanticErrorKind::VoidReturnMismatch
                | semantic_analysis::SemanticErrorKind::DeclarationMissingParameterType
                | semantic_analysis::SemanticErrorKind::UndefinedInternal(_)
                | semantic_analysis::SemanticErrorKind::Format(..) => error.kind.group(),
                | semantic_analysis::SemanticErrorKind::CallAttributeWarning =>
                    Some(G::AttributeWarning),
                | semantic_analysis::SemanticErrorKind::PredefinedOutsideFunction =>
                    Some(G::PredefinedOutsideFunction),
                | semantic_analysis::SemanticErrorKind::UnusedStaticFunction =>
                    Some(G::UnusedFunction),
                | semantic_analysis::SemanticErrorKind::InitializerOverride
                | semantic_analysis::SemanticErrorKind::PartialInitializerOverride =>
                    Some(G::InitializerOverrides),
                | semantic_analysis::SemanticErrorKind::ConstantConversion(_) =>
                    Some(G::ConstantConversion),
                | semantic_analysis::SemanticErrorKind::ExcessString
                | semantic_analysis::SemanticErrorKind::ExcessElements(_) =>
                    Some(G::ExcessInitializers),
                | semantic_analysis::SemanticErrorKind::FloatOverflow(_) => Some(G::Overflow),
                | semantic_analysis::SemanticErrorKind::ExceptionalConstant(group, _) =>
                    Some(group),
                | semantic_analysis::SemanticErrorKind::PedanticOverflow =>
                    Some(G::PedanticOverflow),
                | semantic_analysis::SemanticErrorKind::LiteralConversion(_) =>
                    Some(G::LiteralConversion),
                | semantic_analysis::SemanticErrorKind::ImmediateOutOfRange { .. } =>
                    Some(G::ArgumentOutsideRange),
                | _ => None,
            },
            | Self::PreprocessorTokenizining(error) => error.group(),
            | _ => None,
        }
    }

    /// Visits every provenance range this diagnostic reads from a context
    /// arena. Owned source vectors are not visited.
    pub(crate) fn for_each_source_vectors_mut(
        &mut self,
        visit: &mut impl FnMut(&mut SourceVectors),
    ) {
        match self {
            | Self::Classified(classified) => classified.error.for_each_source_vectors_mut(visit),
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
            | Self::Classified(classified) => classified.severity,
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
        let mut diagnostic = match self {
            | Self::Classified(classified) => {
                let mut diagnostic = classified.error.diagnostic_in(context, source, arena);
                diagnostic.fatal = classified.fatal;
                diagnostic
            },
            | Self::InitialProcessing(error) => error.diagnostic_in(context, source, arena),
            | Self::PreprocessorTokenizining(error) => error.diagnostic_in(context, source, arena),
            | Self::Preprocessing(error) => error.diagnostic_in(context, source, arena),
            | Self::Parsing(error) => error.diagnostic_in(context, source, arena),
            | Self::Semantic(error) => error.diagnostic_in(context, source, arena),
            | Self::Extension(error) => error.diagnostic_in(context, source, arena),
        };
        diagnostic.severity = self.configured_severity(context);
        diagnostic
    }
}

impl GetSourceVectors for TranslationError<'_> {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors {
        match self {
            | Self::Classified(classified) => classified.error.source_vectors(context),
            | Self::InitialProcessing(error) => error.source_vectors(context),
            | Self::PreprocessorTokenizining(error) => error.source_vectors(context),
            | Self::Preprocessing(error) => error.source_vectors(context),
            | Self::Parsing(error) => error.source_vectors(context),
            | Self::Semantic(error) => error.source_vectors(context),
            | Self::Extension(error) => error.source_vectors(context),
        }
    }
}
