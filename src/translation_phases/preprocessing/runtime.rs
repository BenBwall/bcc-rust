//! Preprocessor working state keeps macro definitions, opened files,
//! conditionals, and reusable literal storage for the translation unit. While
//! macros are active, an expander also holds a frame stack in a temporary
//! arena. Suspend and resume move the source-file cursors between that stack
//! and the long-lived state, so the temporary arena can be reset without losing
//! the current input position.
//!
//! For an object-like macro, the reader pushes a replacement frame and later
//! pops it to return to the source file. Once enough frames have accumulated
//! and only source-file frames remain, the outer loop suspends the expander,
//! resets its arena, and resumes with the same file cursors.
//!
//! Read [`PreprocessorState`], [`Expander::resume`](super::Expander::resume),
//! and [`Expander::suspend`](super::Expander::suspend) first. Then follow
//! [`Expander::push_tokenizer_frame`](super::Expander::push_tokenizer_frame)
//! and [`Expander::next_parser_token`](super::Expander::next_parser_token). The
//! driving loops and their [`Preprocessor`](super::Preprocessor) and
//! [`Expander`](super::Expander) types live in the parent entry file.
//!
//! Files by role:
//!
//! - Construction and arena lifetimes: `runtime/construction.rs`,
//!   `runtime/lifecycle.rs`, `runtime/expansion_state.rs`.
//! - Input frames and output reading: `runtime/frames.rs`, `runtime/reader.rs`.
//! - Locations, builtins, and macro-use diagnostics: `runtime/location.rs`,
//!   `runtime/builtins.rs`, `runtime/diagnostics.rs`.
//!
//! C99: translation phases 4-7, §5.1.1.2 paragraph 1 items 4-7, p. 10; PDF p.
//! 22.
//!
//! C99: macro replacement and rescanning, §6.10.3, pp. 151-153; PDF pp.
//! 163-165; §6.10.3.4, p. 155; PDF p. 167.
//!
//! C99: predefined macros, §6.10.8, pp. 160-161; PDF pp. 172-173;
//! pragma operator, §6.10.9, pp. 161-162; PDF pp. 173-174.
//! The frame storage schedules these operations; it does not analyze C syntax.

// Construction and arena lifetimes.
mod construction;
mod expansion_state;
mod lifecycle;

// Input frames and output reading.
mod frames;
mod reader;

// Locations, builtins, and macro-use diagnostics.
mod builtins;
mod diagnostics;
mod location;

pub(super) use builtins::{
    TranslationTimestamp,
    spell_string_literal,
};
pub(super) use diagnostics::MacroDeprecation;
pub(super) use expansion_state::{
    OutputPurpose,
    QueryExpansion,
    RESET_EXPANSIONS_AFTER_FRAMES,
};
pub(super) use frames::{
    FileFrame,
    TokenizerFrame,
    TokenizerFrameType,
};
pub(super) use lifecycle::Resting;

use super::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    Bump,
    ConditionalGroup,
    LexedFiles,
    LiteralScratch,
    MacroDefinition,
    StringCacheId,
    language_features,
};

/// State kept from the start of preprocessing to its end.
pub(super) struct PreprocessorState<'pp> {
    pub(super) counter:               u64,
    /// Reserved optional-replacement marker, checked by ID on token paths.
    pub(super) va_opt_name:           StringCacheId,
    pub(super) query_depth:           usize,
    pub(super) conditional_queries:   bool,
    /// Directives end at their first new-line (C99 §6.10p2).
    pub(super) in_directive:          bool,
    pub(super) retain_placeholders:   bool,
    pub(super) arena:                 &'pp Bump,
    pub(super) once_set:              ArenaSet<'pp, u32>,
    pub(super) macro_definitions:     ArenaMap<'pp, StringCacheId, MacroDefinition<'pp>>,
    /// Implementation-defined pragma markers (C99 §6.10.6p1).
    pub(super) deprecated_macros:     ArenaMap<'pp, StringCacheId, MacroDeprecation<'pp>>,
    /// Every source file opened, including the main file and headers.
    pub(super) lexed_files:           LexedFiles<'pp>,
    /// Source-file frames, outermost first, while no expansion is active.
    /// During an expansion segment they live on the expander's stack and
    /// this vector stays empty, keeping its capacity.
    pub(super) file_frames:           ArenaVec<'pp, FileFrame<'pp>>,
    pub(super) command_line_file:     Option<u32>,
    /// Provenance of open conditionals is owned because token iteration
    /// compacts temporary preprocessor provenance while groups remain open.
    pub(super) open_conditionals:     ArenaVec<'pp, ConditionalGroup<'pp>>,
    /// Fixed on first use so every `__DATE__` and `__TIME__` agrees.
    pub(super) translation_timestamp: Option<TranslationTimestamp<'pp>>,
    /// Storage that string-literal conversion reuses.
    pub(super) literal_scratch:       LiteralScratch<'pp>,
}
