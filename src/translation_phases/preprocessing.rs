//! C99 preprocessing (translation phase 4) and the token-level work that
//! follows it: escape-sequence evaluation, adjacent string-literal
//! concatenation, and conversion of preprocessing tokens into parser tokens.

mod conditional;
mod directives;
mod driver;
mod errors;
mod expression;
mod macro_expansion;
#[cfg(test)]
mod tests;
mod token;
mod token_conversion;

use std::{
    fmt::Debug,
    ops::ControlFlow,
    path::{
        Path,
        PathBuf,
    },
};

use conditional::ConditionalGroup;
use driver::{
    TokenizerFrame,
    TokenizerFrameType,
    TranslationTimestamp,
};
pub(crate) use errors::{
    PreprocessorError,
    PreprocessorErrorType,
};
use expression::PreprocessorExpressionParser;
pub(crate) use macro_expansion::HashHash;
use macro_expansion::MacroDefinition;
use rustc_hash::FxBuildHasher;
pub(crate) use token::{
    CharacterTokenType,
    FloatTokenType,
    IntegerTokenType,
    KeywordTokenType,
    LiteralId,
    LiteralUnit,
    OperatorTokenType,
    StringTokenType,
    Token,
    TokenType,
};

#[cfg(test)]
use crate::util::shared::SharedVec;
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SetPosition,
        SetSourceFileIndex,
        SourcePosition,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::{
            LexedFiles,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaSet,
            ArenaVec,
            Bump,
            RegionVec,
        },
        string_cache::StringCacheId,
    },
};

const PREDEFINED_MACRO_NAMES: [&str; 9] = [
    "__LINE__",
    "__FILE__",
    "__DATE__",
    "__TIME__",
    "_Pragma",
    "__STDC__",
    "__STDC_VERSION__",
    "__STDC_HOSTED__",
    "__STDC_MB_MIGHT_NEQ_WC__",
];

/// Frames pushed after which the expansion arena is reset, at the next point
/// where no expansion is active. A reset copies the source-file frames, so it
/// is not worth doing after every top-level token.
const RESET_EXPANSIONS_AFTER_FRAMES: usize = 256;

/// Parser-only diagnostics require invocation metadata; standalone token
/// production does not retain that side information.
#[derive(Debug, Clone, Copy)]
enum OutputPurpose {
    Preprocessing,
    Parsing,
}

/// State kept from the start of preprocessing to its end.
struct PreprocessorState<'pp> {
    arena:                 &'pp Bump,
    once_set:              ArenaSet<'pp, u32>,
    macro_definitions:     ArenaMap<'pp, StringCacheId, MacroDefinition<'pp>>,
    /// Every source file opened, including the main file and headers.
    lexed_files:           LexedFiles<'pp>,
    /// Source-file frames, outermost first, while no expansion is active.
    /// During an expansion segment they live on the expander's stack and
    /// this vector stays empty, keeping its capacity.
    file_frames:           ArenaVec<'pp, FileFrame<'pp>>,
    /// Provenance of open conditionals is owned because token iteration
    /// compacts temporary preprocessor provenance while groups remain open.
    open_conditionals:     ArenaVec<'pp, ConditionalGroup<'pp>>,
    /// Fixed on first use so every `__DATE__` and `__TIME__` agrees.
    translation_timestamp: Option<TranslationTimestamp<'pp>>,
}

impl Debug for PreprocessorState<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreprocessorState")
            .field("once_set", &self.once_set)
            .field("macro_definitions", &self.macro_definitions)
            .field("lexed_files", &self.lexed_files)
            .field("file_frames", &self.file_frames)
            .field("open_conditionals", &self.open_conditionals)
            .field("translation_timestamp", &self.translation_timestamp)
            .finish()
    }
}

/// A source-file frame kept while no expansion is active.
#[derive(Debug)]
struct FileFrame<'pp> {
    conditional_base:           usize,
    physical_source_file_index: u32,
    tokenizer:                  TokenSource<'pp>,
}

/// What the preprocessor keeps while no macro expansion is active.
#[derive(Debug)]
struct Resting<'tu, 'pp> {
    state:                 PreprocessorState<'pp>,
    /// The innermost source file's cursor.
    tokenizer:             TokenSource<'pp>,
    current_is_newline:    bool,
    last_was_newline:      bool,
    output_purpose:        OutputPurpose,
    expression_parser:     PreprocessorExpressionParser<'pp>,
    pending_parser_token:  Option<Token>,
    pending_parser_errors: ArenaVec<'pp, TranslationError<'tu>>,
    source_segment_limit:  usize,
}

/// Translation phases 4 through 6 over one translation unit.
///
/// Macro expansion working memory comes from an expansion arena. It is reset
/// at points where no expansion is active, so per-invocation data does not
/// accumulate in the preprocessing arena for the whole phase. This is the
/// chunking seam's expansion arena, at the granularity of top-level
/// expansions.
pub(crate) struct Preprocessor<'tu, 'pp> {
    /// `None` only after a caller stopped preprocessing inside an expansion.
    resting:   Option<Resting<'tu, 'pp>>,
    expansion: Bump,
    /// Where reading stopped, and the presumed source file there.
    end:       (SourcePosition, u32),
}

impl Debug for Preprocessor<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preprocessor")
            .field("resting", &self.resting)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

/// The preprocessor while it reads input: its long-lived state plus the
/// expansion state, whose memory comes from the expansion arena `'x`.
pub(crate) struct Expander<'tu, 'pp: 'x, 'x> {
    state:                 PreprocessorState<'pp>,
    /// Source and include frames remain between expansions. Macro frames
    /// share this stack until the current expansion finishes.
    tokenizer_stack:       ArenaVec<'x, TokenizerFrame<'x>>,
    pub(crate) tokenizer:  TokenSource<'x>,
    /// Expansion working memory, reset between top-level expansions.
    scratch:               &'x Bump,
    hash_hash_stack:       ArenaVec<'x, HashHash>,
    current_is_newline:    bool,
    /// Collect use-site hint metadata only when a language parser will consume
    /// the output.
    output_purpose:        OutputPurpose,
    last_was_newline:      bool,
    generate_placeholders: bool,
    /// The tokenizer-stack depth of the `#` or `##` operand being replaced,
    /// at which reading stops when that operand ends, or 0 outside such
    /// replacement.
    operand_fence:         usize,
    /// Argument prescan stops here without suppressing expansion within it.
    expansion_fence:       usize,
    expression_parser:     PreprocessorExpressionParser<'pp>,
    pending_parser_token:  Option<Token>,
    pending_parser_errors: ArenaVec<'pp, TranslationError<'tu>>,
    /// Parser-supplied provenance budget, checked within string concatenation
    /// as well as between completed output tokens.
    source_segment_limit:  usize,
    /// Frames pushed since the expansion arena was last reset.
    pushed_frames:         usize,
}

impl GetPosition for Expander<'_, '_, '_> {
    #[inline(always)]
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.tokenizer.position(context)
    }
}

impl SetPosition for Expander<'_, '_, '_> {
    #[inline(always)]
    fn set_position(&mut self, context: &mut Context<'_>, position: SourcePosition) {
        self.tokenizer.set_position(context, position);
    }
}

impl GetSourceFileIndex for Expander<'_, '_, '_> {
    #[inline(always)]
    fn source_file_index(&self) -> u32 {
        self.tokenizer.source_file_index()
    }
}

impl SetSourceFileIndex for Expander<'_, '_, '_> {
    fn set_source_file_index(&mut self, context: &mut Context<'_>, source_file_index: u32) {
        self.tokenizer
            .set_source_file_index(context, source_file_index);
    }
}

impl<'tu, 'pp> Preprocessor<'tu, 'pp> {
    pub(crate) fn prepare_for_parsing(&mut self) {
        self.resting_mut().output_purpose = OutputPurpose::Parsing;
    }

    #[cfg(test)]
    pub(crate) fn new(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: Box<Path>,
        source: &str,
        quote_include_directories: SharedVec<PathBuf>,
        system_include_directories: SharedVec<PathBuf>,
    ) -> Self {
        let source_name = source_name.into_path_buf();
        let preprocessor = Self::new_inner(
            pp,
            context,
            &source_name,
            source,
            None,
            &quote_include_directories,
            &system_include_directories,
        );
        drop(quote_include_directories);
        drop(system_include_directories);
        preprocessor
    }

    pub(crate) fn new_with_arena_source(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: &Path,
        source: &'tu str,
        quote_include_directories: &[PathBuf],
        system_include_directories: &[PathBuf],
    ) -> Self {
        Self::new_inner(
            pp,
            context,
            source_name,
            source,
            Some(source),
            quote_include_directories,
            system_include_directories,
        )
    }

    fn new_inner(
        pp: &'pp Bump,
        context: &mut Context<'tu>,
        source_name: &Path,
        source: &str,
        arena_source: Option<&'tu str>,
        quote_include_directories: &[PathBuf],
        system_include_directories: &[PathBuf],
    ) -> Self {
        context.set_include_directories(quote_include_directories, system_include_directories);
        let mut macro_definitions = ArenaMap::with_hasher_in(FxBuildHasher, pp);
        for name in PREDEFINED_MACRO_NAMES {
            _ = macro_definitions
                .insert(context.string_cache.intern(name), MacroDefinition::BuiltIn);
        }
        let source_file_index = context.intern_source_file(source_name);
        let mut lexed_files = LexedFiles::new_in(pp);
        let tokenizer = lexed_files.open(context, source_file_index, source);
        let mut file_frames = ArenaVec::new_in(pp);
        file_frames.push(FileFrame {
            conditional_base:           0,
            physical_source_file_index: source_file_index,
            tokenizer:                  tokenizer.clone(),
        });
        if let Some(source) = arena_source {
            context.record_arena_source_text(source_file_index, source);
        } else {
            context.record_source_text(source_file_index, source);
        }
        let end = (tokenizer.position(context), source_file_index);
        Self {
            resting: Some(Resting {
                state: PreprocessorState {
                    arena: pp,
                    once_set: ArenaSet::with_hasher_in(FxBuildHasher, pp),
                    macro_definitions,
                    lexed_files,
                    file_frames,
                    open_conditionals: ArenaVec::new_in(pp),
                    translation_timestamp: None,
                },
                tokenizer,
                last_was_newline: true,
                current_is_newline: true,
                output_purpose: OutputPurpose::Preprocessing,
                expression_parser: PreprocessorExpressionParser::new(pp),
                pending_parser_token: None,
                pending_parser_errors: ArenaVec::new_in(pp),
                source_segment_limit: usize::MAX,
            }),
            expansion: Bump::new(),
            end,
        }
    }

    fn resting_mut(&mut self) -> &mut Resting<'tu, 'pp> {
        self.resting
            .as_mut()
            .expect("preprocessing does not resume after stopping inside an expansion")
    }

    /// Where reading stopped, for end-of-input diagnostics.
    pub(crate) fn end_position(&self) -> SourcePosition {
        self.end.0
    }

    /// The presumed source file where reading stopped.
    pub(crate) fn end_source_file_index(&self) -> u32 {
        self.end.1
    }

    /// Calls `step` until it breaks. Between calls, once enough frames were
    /// pushed and no expansion is active, the expansion arena is reset.
    ///
    /// When `step` breaks inside an expansion, that expansion's state is
    /// released with its arena and preprocessing cannot resume.
    pub(crate) fn run<B>(
        &mut self,
        context: &mut Context<'tu>,
        mut step: impl for<'x> FnMut(&mut Expander<'tu, 'pp, 'x>, &mut Context<'tu>) -> ControlFlow<B>,
    ) -> B {
        loop {
            // Nothing borrows the expansion arena between segments: the
            // previous expander was suspended or dropped.
            self.expansion.reset();
            let resting = self
                .resting
                .take()
                .expect("preprocessing does not resume after stopping inside an expansion");
            let mut expander = Expander::resume(resting, &self.expansion);
            let stopped = loop {
                if let ControlFlow::Break(value) = step(&mut expander, context) {
                    break Some(value);
                }
                if expander.pushed_frames >= RESET_EXPANSIONS_AFTER_FRAMES
                    && expander.is_between_expansions()
                {
                    break None;
                }
            };
            self.end = (expander.position(context), expander.source_file_index());
            if expander.is_between_expansions() {
                self.resting = Some(expander.suspend());
            }
            if let Some(value) = stopped {
                return value;
            }
        }
    }

    /// Calls `visit` with each phase-6 token as [`Expander::next_item`]
    /// produces it, without compacting provenance.
    #[cfg(test)]
    pub(crate) fn for_each_item(
        &mut self,
        context: &mut Context<'tu>,
        mut visit: impl FnMut(&mut Context<'tu>, Token),
    ) {
        self.run(context, |preprocessor, context| {
            match preprocessor.next_item(context) {
                | Some(token) => {
                    visit(context, token);
                    ControlFlow::Continue(())
                },
                | None => ControlFlow::Break(()),
            }
        });
    }

    /// Calls `visit` with each phase-6 token. Preprocessor provenance is
    /// compacted before each token is produced, so `visit` must retain the
    /// provenance it keeps.
    pub(crate) fn for_each_iterator_item(
        &mut self,
        context: &mut Context<'tu>,
        mut visit: impl FnMut(&mut Context<'tu>, Token),
    ) {
        self.run(context, |preprocessor, context| {
            match preprocessor.next_iterator_item(context) {
                | Some(token) => {
                    visit(context, token);
                    ControlFlow::Continue(())
                },
                | None => ControlFlow::Break(()),
            }
        });
    }

    /// Runs translation phases 4 through 6 without the parser's resource
    /// budget, for direct preprocessing tests.
    #[cfg(test)]
    pub(crate) fn preprocess_all(
        &mut self,
        context: &mut Context<'tu>,
        _tok: &Bump,
    ) -> RegionVec<Token> {
        let mut tokens = RegionVec::new_in(Bump::new());
        let _ = self.preprocess_into_arena(context, usize::MAX, &mut tokens);
        tokens
    }

    /// Appends phase-6 output to the caller's token buffer before parsing
    /// starts. Diagnostics stay pending in `context`; retained provenance
    /// survives compaction of preprocessor working storage between tokens.
    pub(crate) fn preprocess_into_arena(
        &mut self,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
        tokens: &mut RegionVec<Token>,
    ) -> Option<Token> {
        self.collect_with_limit(context, source_segment_limit, |token| tokens.push(token))
    }

    /// Stops after the first token that exceeds the configured provenance
    /// budget, letting the parser report the existing resource diagnostic.
    fn collect_with_limit(
        &mut self,
        context: &mut Context<'tu>,
        source_segment_limit: usize,
        mut push: impl FnMut(Token),
    ) -> Option<Token> {
        self.resting_mut().source_segment_limit = source_segment_limit;
        self.run(context, |preprocessor, context| {
            let Some(mut token) = preprocessor.next_iterator_item(context) else {
                return ControlFlow::Break(None);
            };
            token.source_vectors = context.retain_token_source(token.source_vectors);
            push(token);
            if context.source_segment_count() > source_segment_limit {
                let mut limit_token = preprocessor.pending_parser_token.unwrap_or(token);
                limit_token.source_vectors =
                    context.retain_token_source(limit_token.source_vectors);
                context.append_pending_errors(preprocessor.pending_parser_errors.drain(..));
                return ControlFlow::Break(Some(limit_token));
            }
            ControlFlow::Continue(())
        })
    }
}

impl<'tu, 'pp, 'x> Expander<'tu, 'pp, 'x> {
    /// Continues reading the resting source files, taking expansion memory
    /// from `scratch`.
    fn resume(resting: Resting<'tu, 'pp>, scratch: &'x Bump) -> Self {
        let Resting {
            mut state,
            tokenizer,
            current_is_newline,
            last_was_newline,
            output_purpose,
            expression_parser,
            pending_parser_token,
            pending_parser_errors,
            source_segment_limit,
        } = resting;
        let mut tokenizer_stack = ArenaVec::with_capacity_in(state.file_frames.len(), scratch);
        tokenizer_stack.extend(state.file_frames.drain(..).map(|frame| TokenizerFrame {
            frame_type: TokenizerFrameType::SourceFile {
                conditional_base:           frame.conditional_base,
                physical_source_file_index: frame.physical_source_file_index,
            },
            tokenizer:  frame.tokenizer,
        }));
        Self {
            state,
            tokenizer_stack,
            tokenizer,
            scratch,
            hash_hash_stack: ArenaVec::new_in(scratch),
            current_is_newline,
            output_purpose,
            last_was_newline,
            generate_placeholders: false,
            operand_fence: 0,
            expansion_fence: 0,
            expression_parser,
            pending_parser_token,
            pending_parser_errors,
            source_segment_limit,
            pushed_frames: 0,
        }
    }

    /// Whether only source files are being read, so nothing refers to the
    /// expansion arena except the frame stack itself.
    fn is_between_expansions(&self) -> bool {
        self.operand_fence == 0
            && self.expansion_fence == 0
            && self.hash_hash_stack.is_empty()
            && self
                .tokenizer_stack
                .iter()
                .all(|frame| matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }))
    }

    /// Returns the state to keep while the expansion arena is reset. Only
    /// valid between expansions. Source cursors return to the `'pp`
    /// lifetime through the registry of opened files.
    fn suspend(self) -> Resting<'tu, 'pp> {
        debug_assert!(
            self.is_between_expansions(),
            "only source-file frames may outlive the expansion arena"
        );
        let Self {
            mut state,
            mut tokenizer_stack,
            tokenizer,
            current_is_newline,
            output_purpose,
            last_was_newline,
            expression_parser,
            pending_parser_token,
            pending_parser_errors,
            source_segment_limit,
            ..
        } = self;
        for frame in tokenizer_stack.drain(..) {
            let TokenizerFrameType::SourceFile {
                conditional_base,
                physical_source_file_index,
            } = frame.frame_type
            else {
                unreachable!("only source-file frames remain between expansions");
            };
            let tokenizer = state.lexed_files.persist(&frame.tokenizer);
            state.file_frames.push(FileFrame {
                conditional_base,
                physical_source_file_index,
                tokenizer,
            });
        }
        let tokenizer = state.lexed_files.persist(&tokenizer);
        Resting {
            state,
            tokenizer,
            current_is_newline,
            last_was_newline,
            output_purpose,
            expression_parser,
            pending_parser_token,
            pending_parser_errors,
            source_segment_limit,
        }
    }

    fn next_parser_token(&mut self, context: &mut Context<'tu>) -> Option<Token> {
        context.append_pending_errors(self.pending_parser_errors.drain(..));
        if let Some(token) = self.pending_parser_token.take() {
            return Some(token);
        }
        loop {
            let Some(token) = self.next_preprocessor_token::<true>(context) else {
                for vectors in self.state.open_conditionals.drain(..) {
                    let source_vectors = context.push_source_vectors(vectors.source);
                    context.preprocessor_error(PreprocessorError {
                        error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                        source_vectors,
                    });
                }
                return None;
            };

            if let Some(result) = self.map_preprocessor_token(context, token) {
                if matches!(self.output_purpose, OutputPurpose::Parsing)
                    && let Some(site) = self.expansion_end()
                {
                    context.record_expansion_end(result.source_vectors, site);
                }
                return Some(result);
            }
        }
    }

    /// Produces the next iterator item while keeping buffered provenance alive.
    ///
    /// Adjacent-string concatenation may already have mapped a later token or
    /// EOF diagnostic. Source-vector compaction therefore belongs to the
    /// producer that owns that buffered work, not to each iterator consumer.
    pub(crate) fn next_iterator_item(&mut self, context: &mut Context<'tu>) -> Option<Token> {
        if self.next_iterator_item_compacts() {
            context.compact_preprocessor_vectors();
        }
        self.next_item(context)
    }

    /// Whether the next [`Self::next_iterator_item`] call discards the
    /// preprocessor provenance arena. Consumers retaining provenance across
    /// calls, such as a deferred diagnostic, must resolve it first.
    ///
    /// A `##` operand held while the other operand's argument expands still
    /// refers to the arena, so compaction waits until every paste completes.
    pub(crate) fn next_iterator_item_compacts(&self) -> bool {
        self.pending_parser_token.is_none()
            && self.pending_parser_errors.is_empty()
            && self
                .hash_hash_stack
                .iter()
                .all(|operand| matches!(operand, HashHash::Empty))
    }
}

impl<'tu> TranslationPhase<'tu> for Expander<'tu, '_, '_> {
    type Item = Token;

    fn next_item(&mut self, context: &mut Context<'tu>) -> Option<Self::Item> {
        let token = self.next_parser_token(context)?;
        Some(self.concatenate_adjacent_strings(context, token))
    }
}
