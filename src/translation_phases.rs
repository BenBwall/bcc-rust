//! The compiler passes source through lexing, preprocessing, syntax parsing,
//! and semantic analysis. [`crate::pipeline`] drives these stages in batches:
//! each file is lexed when opened, the whole translation unit is preprocessed,
//! and parsing completes before semantic analysis starts. Shared [`Context`]
//! storage preserves source locations and diagnostics across the stages.
//!
//! For `#define N 2` followed by `int x = N;`, lexing records tokens and source
//! positions. Preprocessing expands `N` to `2`. Parsing builds the declaration,
//! and semantic analysis resolves the declared type and initializer.
//!
//! Read [`crate::pipeline::parse_translation_unit`], then
//! [`crate::pipeline::analyze_translation_unit`]. Follow the phase modules in
//! order, then read [`Context`] and [`TranslationPhase`] for their shared state
//! and token-source interface.
//!
//! Files by role:
//! - Phases 1-3: `initial_processing.rs` maps characters and splices lines;
//!   `preprocessor_tokenizer.rs` and its directory form and replay tokens.
//! - Phases 4-6: `preprocessing.rs` and its directory execute directives,
//!   expand macros, convert literals, and concatenate adjacent string literals.
//! - Phase 7: `parsing.rs` and its directory build syntax;
//!   `semantic_analysis.rs` and its directory resolve types and enforce
//!   semantic constraints.
//! - Shared storage: `context.rs`, `context/`, `provenance.rs`.
//! - Shared interfaces and diagnostics: `interface.rs`, `errors.rs`,
//!   `extension.rs`.
//!
//! C99: §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22. Required diagnostics:
//! §5.1.1.3 paragraph 1, p. 11; PDF p. 23. Code generation and phase-8 linking
//! are outside this module.

// Phases 1-3: source characters and preprocessing tokens
pub(crate) mod initial_processing;
pub(crate) mod preprocessor_tokenizer;

// Phases 4-6: preprocessing
pub(crate) mod preprocessing;

// Phase 7: syntax and semantics
pub(crate) mod parsing;
pub(crate) mod semantic_analysis;

// Shared state and provenance
mod context;
mod provenance;

// Interfaces and diagnostics
mod errors;
mod extension;
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
