//! Shared translation-phase concepts: diagnostics plumbing, provenance,
//! the translation [`Context`], and the [`TranslationPhase`] interface of
//! the preprocessing-token sources.
//!
//! C99: translation phases 1-7, §5.1.1.2, pp. 9-10; PDF pp. 21-22. This
//! interface also carries the diagnostics required by §5.1.1.3 paragraph 1,
//! p. 11; PDF p. 23.

mod context;
mod extension;
pub(crate) mod initial_processing;
pub(crate) mod parsing;
pub(crate) mod preprocessing;
pub(crate) mod preprocessor_tokenizer;
mod provenance;

use std::{
    fmt::Debug,
    hash::Hash,
};

pub(crate) use context::Context;
pub(crate) use provenance::{
    SourcePosition,
    SourceVector,
    SourceVectors,
};
use thiserror::Error;

use self::{
    initial_processing::InitialProcessorError,
    parsing::ParserError,
    preprocessing::PreprocessorError,
    preprocessor_tokenizer::PreprocessorTokenizerError,
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
            | Self::Extension(error) => visit(&mut error.source_vectors),
        }
    }
}

impl GetSeverity for TranslationError<'_> {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::InitialProcessing(error) => error.severity(),
            | Self::PreprocessorTokenizining(error) => error.severity(),
            | Self::Preprocessing(error) => error.severity(),
            | Self::Parsing(error) => error.severity(),
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
        match self {
            | Self::InitialProcessing(error) => error.diagnostic_in(context, source, arena),
            | Self::PreprocessorTokenizining(error) => error.diagnostic_in(context, source, arena),
            | Self::Preprocessing(error) => error.diagnostic_in(context, source, arena),
            | Self::Parsing(error) => error.diagnostic_in(context, source, arena),
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
            | Self::Extension(error) => error.source_vectors(context),
        }
    }
}

trait StrExt {
    /// Returns the character at the given index,
    /// or `None` if the index is out of bounds or is in the middle of a
    /// character.
    fn char_at(&self, index: usize) -> Option<char>;
}

impl StrExt for str {
    fn char_at(&self, index: usize) -> Option<char> {
        self.get(index..)?.chars().next()
    }
}

#[expect(dead_code, reason = "We aren't using the Note variant yet")]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
    Note,
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

pub(crate) trait GetSourceFileIndex {
    fn source_file_index(&self) -> u32;
}

pub(crate) trait GetPosition {
    fn position(&self, context: &Context<'_>) -> SourcePosition;
    #[inline(always)]
    fn index(&self, context: &Context<'_>) -> usize {
        self.position(context).index
    }
    #[inline(always)]
    fn column(&self, context: &Context<'_>) -> u32 {
        self.position(context).column
    }
}

pub(crate) trait GetSourceVectors {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors;
}

/// Moves a reader to a source position. The derived setters read the rest
/// of the current position through [`GetPosition`].
pub(crate) trait SetPosition: GetPosition {
    fn set_position(&mut self, position: SourcePosition);
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Position setters are retained for translation-phase implementations."
        )
    )]
    #[inline(always)]
    fn set_line(&mut self, context: &Context<'_>, line: u32) {
        self.set_position(SourcePosition {
            index: self.index(context),
            line,
            column: self.column(context),
        });
    }
}

pub(crate) trait SetSourceFileIndex {
    fn set_source_file_index(&mut self, source_file_index: u32);
}

/// Reads one stage of the conceptual translation sequence.
/// C99: §5.1.1.2p1-7, pp. 9-10; PDF pp. 21-22.
pub(crate) trait TranslationPhase<'tu>:
    GetPosition + SetPosition + GetSourceFileIndex + SetSourceFileIndex
{
    type Item;
    fn next_item(&mut self, context: &mut Context<'tu>) -> Option<Self::Item>;
}
