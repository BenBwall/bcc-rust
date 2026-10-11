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

pub(crate) mod semantic_analysis;

mod errors;

mod interface;

use std::{
    fmt::Debug,
    hash::Hash,
};

pub(crate) use context::Context;
pub(crate) use errors::{
    ErrorSeverity,
    GetSeverity,
    TranslationError,
};
pub(crate) use extension::{
    DiagnosticPolicy,
    policy_severity,
};
use interface::StrExt;
pub(crate) use interface::{
    GetPosition,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    TranslationPhase,
};
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
