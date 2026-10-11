//! Push and pop source-file and macro-replacement frames.
//!
//! C99: translation phase 4, §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22;
//! includes, §6.10.2 paragraphs 2-3, pp. 149-150; PDF pp. 161-162;
//! macro invocations, §6.10.3 paragraphs 9-11, p. 152; PDF p. 164;
//! rescanning, §6.10.3.4 paragraph 1, p. 155; PDF p. 167.
//! Directive handlers and replacement operators decide which frames to push.

use std::{
    fmt::Debug,
    mem::take,
};

use super::super::{
    Expander,
    PreprocessorError,
    PreprocessorErrorType,
    macro_expansion::{
        FunctionLikeMacroArgument,
        MacroArguments,
        MacroDefinition,
    },
};
use crate::{
    translation_phases::{
        SourceVector,
        preprocessor_tokenizer::TokenSource,
    },
    util::string_cache::StringCacheId,
};

impl<'x> Expander<'_, '_, '_, 'x> {
    pub(in crate::translation_phases::preprocessing) fn push_tokenizer_frame(
        &mut self,
        frame: TokenizerFrame<'x>,
    ) {
        self.tokenizer_stack.last_mut().unwrap().tokenizer = take(&mut self.tokenizer);
        self.tokenizer = frame.tokenizer.clone();
        self.tokenizer_stack.push(frame);
        self.pushed_frames += 1;
    }

    /// Pops the innermost frame. Popping a source file reports the
    /// conditionals it left open: each file passes through phase 4 on its
    /// own, so its `if-section`s close within it, as GCC and Clang require.
    ///
    /// C99: §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22, and the
    /// `if-section` grammar of §6.10 paragraph 1, p. 145; PDF p. 157.
    pub(in crate::translation_phases::preprocessing) fn pop_tokenizer_frame(&mut self) {
        let frame = self.tokenizer_stack.pop();
        if let Some(TokenizerFrame {
            frame_type:
                TokenizerFrameType::SourceFile {
                    conditional_base, ..
                },
            ..
        }) = frame
        {
            // File-local openings are owned copies, so their provenance
            // survives preprocessor-arena compaction. Macro frame pops
            // leave conditional state untouched.
            let base = conditional_base.min(self.state.open_conditionals.len());
            for vectors in self.state.open_conditionals.split_off(base) {
                let source_vectors = self.context.push_source_vectors(vectors.source);
                self.context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::MoreIfDirectivesThanEndifDirectives,
                    source_vectors,
                });
            }
        }
        if let Some(last) = self.tokenizer_stack.last() {
            self.tokenizer = last.tokenizer.clone();
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(in crate::translation_phases::preprocessing) struct TokenizerFrame<'a> {
    pub(in crate::translation_phases::preprocessing) frame_type: TokenizerFrameType<'a>,
    pub(in crate::translation_phases::preprocessing) tokenizer:  TokenSource<'a>,
}

/// What a frame of the tokenizer stack reads.
#[derive(Debug, PartialEq, Clone)]
pub(in crate::translation_phases::preprocessing) enum TokenizerFrameType<'a> {
    /// Tokens replayed ahead of the frame below: the remainder of a
    /// boundary-crossing macro call, rejected lookahead, or the output of a
    /// builtin query or `#embed`, or a macro-replaced argument substituted
    /// for its parameter.
    Rescan {
        /// Whether the tokens are a substituted argument.
        ///
        /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
        argument: bool,
    },
    /// Already collected query arguments, replayed without macro replacement
    /// until their enclosing embed parameter is evaluated.
    /// C23: §6.10.4.2p3, p. 174; PDF p. 187.
    DeferredQuery,
    /// A source file, the main file or one named by `#include`.
    ///
    /// C99: §6.10.2 paragraphs 2-3, pp. 149-150; PDF pp. 161-162.
    SourceFile {
        /// Caller groups below this depth cannot be modified by this file.
        conditional_base:           usize,
        /// Physical file identity, unaffected by #line.
        physical_source_file_index: u32,
        /// Configured search entry for this opening; local, absolute and main
        /// files have none. GNU `#include_next` continues from this origin.
        include_search_index:       Option<usize>,
    },
    /// The replacement list of an object-like macro being rescanned.
    ///
    /// C99: §6.10.3 paragraph 9, p. 152; PDF p. 164.
    ObjectLikeMacroInvocation {
        name:           StringCacheId,
        invocation:     SourceVector,
        invocation_end: SourceVector,
        /// Spelling before replacement, for nested expansion notes.
        spelling:       SourceVector,
    },
    /// The replacement list of a function-like macro, with the arguments
    /// that its parameters stand for.
    ///
    /// C99: §6.10.3 paragraphs 10-11, p. 152; PDF p. 164.
    FunctionLikeMacroInvocation {
        invocation:     SourceVector,
        invocation_end: SourceVector,
        name:           StringCacheId,
        arguments:      MacroArguments<'a>,
        is_variadic:    bool,
        /// Spelling before replacement, for nested expansion notes.
        spelling:       SourceVector,
    },
    /// The tokens of one argument, read where its parameter is substituted.
    ///
    /// C99: §6.10.3.1 paragraph 1, p. 153; PDF p. 165.
    FunctionLikeMacroArgument {
        argument:            &'a FunctionLikeMacroArgument<'a>,
        /// The parenthesis depth within an argument read from its invocation,
        /// or `None` for a replayed operand, which ends with its tokens.
        paren_depth:         Option<usize>,
        has_generated_token: bool,
    },
}

// Frames, the arguments they share, and macro definitions live in arenas,
// which run no destructors, so none of them may own other memory.
const _: () = {
    assert!(
        !std::mem::needs_drop::<TokenizerFrame<'static>>(),
        "tokenizer frames must not own memory outside their arena"
    );
    assert!(
        !std::mem::needs_drop::<MacroDefinition<'static>>(),
        "macro definitions must not own memory outside their arena"
    );
};

/// A source-file frame kept while no expansion is active.
#[derive(Debug)]
pub(in crate::translation_phases::preprocessing) struct FileFrame<'pp> {
    pub(in crate::translation_phases::preprocessing) conditional_base:           usize,
    pub(in crate::translation_phases::preprocessing) physical_source_file_index: u32,
    pub(in crate::translation_phases::preprocessing) include_search_index:       Option<usize>,
    pub(in crate::translation_phases::preprocessing) tokenizer:                  TokenSource<'pp>,
}
