mod construction;

mod lifecycle;

mod frames;

mod reader;

mod location;

mod builtins;

mod diagnostics;

pub(super) use builtins::{
    TranslationTimestamp,
    spell_string_literal,
};
pub(super) use frames::{
    TokenizerFrame,
    TokenizerFrameType,
};

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

mod expansion_state;
pub(super) use diagnostics::MacroDeprecation;
pub(super) use expansion_state::{
    OutputPurpose,
    QueryExpansion,
    RESET_EXPANSIONS_AFTER_FRAMES,
};
pub(super) use frames::FileFrame;
pub(super) use lifecycle::Resting;

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
