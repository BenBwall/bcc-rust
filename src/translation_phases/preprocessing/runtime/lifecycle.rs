//! Suspend and resume preprocessing around expansion-arena resets.
//!
//! C99: translation phase 4, §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22;
//! replacement rescanning, §6.10.3.4 paragraph 1, p. 155; PDF p. 167.
//! This storage boundary preserves input cursors; it does not change
//! replacement rules.

use std::fmt::Debug;

use super::{
    super::{
        Expander,
        Preprocessor,
        Token,
        expression::PreprocessorExpressionParser,
    },
    FileFrame,
    OutputPurpose,
    PreprocessorState,
    QueryExpansion,
    TokenizerFrame,
    TokenizerFrameType,
};
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        preprocessor_tokenizer::TokenSource,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

impl<'c, 'tu, 'pp, 'x> Expander<'c, 'tu, 'pp, 'x> {
    /// Continues reading the resting source files with `context`, taking
    /// expansion memory from `scratch`.
    pub(in crate::translation_phases::preprocessing) fn resume(
        resting: Resting<'tu, 'pp>,
        context: &'c mut Context<'tu>,
        scratch: &'x Bump,
    ) -> Self {
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
                include_search_index:       frame.include_search_index,
            },
            tokenizer:  frame.tokenizer,
        }));
        Self {
            context,
            state,
            tokenizer_stack,
            tokenizer,
            scratch,
            hash_hash_stack: ArenaVec::new_in(scratch),
            current_is_newline,
            output_purpose,
            last_was_newline,
            generate_placeholders: false,
            query_expansion: QueryExpansion::Evaluate,
            operand_fence: 0,
            expansion_fence: 0,
            verbatim_fence: 0,
            expression_parser,
            pending_parser_token,
            pending_parser_errors,
            source_segment_limit,
            pushed_frames: 0,
        }
    }

    /// Returns the state to keep while the expansion arena is reset. Only
    /// valid between expansions. Source cursors return to the `'pp`
    /// lifetime through the registry of opened files.
    pub(in crate::translation_phases::preprocessing) fn suspend(self) -> Resting<'tu, 'pp> {
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
                include_search_index,
            } = frame.frame_type
            else {
                unreachable!("only source-file frames remain between expansions");
            };
            let tokenizer = state.lexed_files.persist(&frame.tokenizer);
            state.file_frames.push(FileFrame {
                conditional_base,
                physical_source_file_index,
                include_search_index,
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

    /// Whether only source files are being read, so nothing refers to the
    /// expansion arena except the frame stack itself.
    pub(in crate::translation_phases::preprocessing) fn is_between_expansions(&self) -> bool {
        self.operand_fence == 0
            && self.expansion_fence == 0
            && self.verbatim_fence == 0
            && self.hash_hash_stack.is_empty()
            && self
                .tokenizer_stack
                .iter()
                .all(|frame| matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }))
    }
}

/// What the preprocessor keeps while no macro expansion is active.
#[derive(Debug)]
pub(in crate::translation_phases::preprocessing) struct Resting<'tu, 'pp> {
    pub(in crate::translation_phases::preprocessing) state:                 PreprocessorState<'pp>,
    /// The innermost source file's cursor.
    pub(in crate::translation_phases::preprocessing) tokenizer:             TokenSource<'pp>,
    pub(in crate::translation_phases::preprocessing) current_is_newline:    bool,
    pub(in crate::translation_phases::preprocessing) last_was_newline:      bool,
    pub(in crate::translation_phases::preprocessing) output_purpose:        OutputPurpose,
    pub(in crate::translation_phases::preprocessing) expression_parser:
        PreprocessorExpressionParser<'pp>,
    pub(in crate::translation_phases::preprocessing) pending_parser_token:  Option<Token>,
    pub(in crate::translation_phases::preprocessing) pending_parser_errors:
        ArenaVec<'pp, TranslationError<'tu>>,
    pub(in crate::translation_phases::preprocessing) source_segment_limit:  usize,
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

impl Debug for Preprocessor<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preprocessor")
            .field("resting", &self.resting)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}
