//! Shared translation-phase concepts: diagnostics plumbing, provenance,
//! the translation [`Context`], and the [`TranslationPhase`] interface of
//! the preprocessing-token sources.

mod context;
pub(crate) mod initial_processing;
pub(crate) mod parsing;
pub(crate) mod preprocessing;
pub(crate) mod preprocessor_tokenizer;
mod provenance;

use std::{
    convert::Infallible,
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
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
        }
    }
}

impl GetPosition for TranslationError<'_> {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        match self {
            | Self::InitialProcessing(error) => error.position(context),
            | Self::PreprocessorTokenizining(error) => error.position(context),
            | Self::Preprocessing(error) => error.position(context),
            | Self::Parsing(error) => error.position(context),
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

#[doc(hidden)]
#[macro_export]
macro_rules! bail {
    ($e:expr $(,)?) => {
        match $e {
            | Ok(v) => v,
            | Err(e) => return Some(Err(e.into())),
        }
    };
}

#[expect(dead_code, reason = "We aren't using the Note variant yet")]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum ErrorSeverity {
    Warning,
    Error,
    Note,
}

impl Display for ErrorSeverity {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | Self::Warning => f.write_str("warning"),
            | Self::Error => f.write_str("error"),
            | Self::Note => f.write_str("note"),
        }
    }
}

pub(crate) trait GetSeverity {
    fn severity(&self) -> ErrorSeverity;
}

impl GetSeverity for ErrorSeverity {
    fn severity(&self) -> ErrorSeverity {
        *self
    }
}

impl GetSeverity for Infallible {
    fn severity(&self) -> ErrorSeverity {
        match *self {}
    }
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
    #[inline(always)]
    fn line(&self, context: &Context<'_>) -> u32 {
        self.position(context).line
    }
}

pub(crate) trait GetSourceVectors {
    fn source_vectors(&self, context: &mut Context<'_>) -> SourceVectors;
}

/// Moves a reader to a source position. The derived setters read the rest
/// of the current position through [`GetPosition`].
pub(crate) trait SetPosition: GetPosition {
    fn set_position(&mut self, position: SourcePosition);
    #[expect(
        dead_code,
        reason = "Position setters are retained for translation-phase implementations."
    )]
    #[inline(always)]
    fn set_index(&mut self, context: &Context<'_>, index: usize) {
        self.set_position(SourcePosition {
            index,
            line: self.line(context),
            column: self.column(context),
        });
    }
    #[expect(
        dead_code,
        reason = "Position setters are retained for translation-phase implementations."
    )]
    #[inline(always)]
    fn set_column(&mut self, context: &Context<'_>, column: u32) {
        self.set_position(SourcePosition {
            index: self.index(context),
            line: self.line(context),
            column,
        });
    }
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

impl GetPosition for Infallible {
    #[inline(always)]
    fn position(&self, _context: &Context<'_>) -> SourcePosition {
        match *self {}
    }
}

pub(crate) trait TranslationPhase<'tu>:
    GetPosition + SetPosition + GetSourceFileIndex + SetSourceFileIndex
{
    type Item;
    fn next_item(&mut self, context: &mut Context<'tu>) -> Option<Self::Item>;
}
