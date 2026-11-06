//! Records where macro replacement separates tokens, for a text of phase 4
//! (`-E`): the padding GCC's preprocessor would put before and after each
//! expansion and substituted argument, as [`TokenSpacing`] on the next token
//! read. Only a text pays for it; for any other reader
//! [`Expander::text_spacing`] is `None` and every token keeps
//! [`TokenSpacing::NONE`].
//!
//! Padding comes before each expansion, with the macro's name as its source,
//! and before each substituted argument, with the parameter as its source
//! unless the parameter begins the replacement list. Padding with no source
//! comes after each argument and each expansion. The replacement a `##` or
//! `#` makes is padded as an argument is (libcpp `macro.cc`,
//! `enter_macro_context`, `replace_args`, `_cpp_pop_context` and
//! `paste_all_tokens`).
//!
//! C99: macro replacement rescans with the rest of the source, §6.10.3.4
//! paragraph 1, p. 155; PDF p. 167; how phase 4's tokens are written out is
//! outside the standard.

use super::{
    super::{
        Expander,
        runtime::{
            TokenizerFrame,
            TokenizerFrameType,
        },
    },
    FunctionLikeMacroArgument,
};
use crate::translation_phases::preprocessor_tokenizer::{
    PreprocessorToken,
    PreprocessorTokenType,
    TokenSpacing,
};

/// What the reader knows about spacing since the last token it gave out.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent facts about the last token read, none implying another."
)]
pub(in crate::translation_phases::preprocessing) struct SpacingState {
    /// The padding since the last token given out, including what the tokens
    /// read since then carried.
    pending: TokenSpacing,
    /// White space was read since the last token read.
    white: bool,
    /// The last token read had white space before it, GCC's `PREV_WHITE`.
    token_white: bool,
    /// A replacement list was just entered, so the next token begins it.
    list_start: bool,
    /// The last token read began its replacement list.
    token_first: bool,
    /// The next token given out is a whole replacement of its own, which
    /// padding follows.
    end_after: bool,
    /// The padding left when the last argument was read to its end.
    pub(in crate::translation_phases::preprocessing) argument_end: TokenSpacing,
    /// The spacing of the last token that left phase 4.
    pub(in crate::translation_phases::preprocessing) output: TokenSpacing,
}

impl SpacingState {
    pub(in crate::translation_phases::preprocessing) const fn new() -> Self {
        Self {
            pending:      TokenSpacing::NONE,
            white:        false,
            token_white:  false,
            list_start:   false,
            token_first:  false,
            end_after:    false,
            argument_end: TokenSpacing::NONE,
            output:       TokenSpacing::NONE,
        }
    }

    /// Notes `token` as read from a frame.
    pub(in crate::translation_phases::preprocessing) fn read(&mut self, token: &PreprocessorToken) {
        match token.kind {
            | PreprocessorTokenType::Whitespace => self.white = true,
            // A new line ends what the printer's `source` remembers, though
            // not that padding was seen (`cb_line_change`).
            | PreprocessorTokenType::Newline => {
                self.white = false;
                if self.pending != TokenSpacing::NONE {
                    self.pending = TokenSpacing::END;
                }
            },
            | _ => {
                self.token_white = std::mem::take(&mut self.white) || token.spacing.white();
                self.token_first =
                    std::mem::take(&mut self.list_start) || token.spacing.is_right_operand();
                self.pending = self.pending.then(token.spacing);
            },
        }
    }

    /// Notes white space that the reader dropped.
    pub(in crate::translation_phases::preprocessing) fn dropped_white(&mut self) {
        self.white = true;
    }

    /// Notes that the last token read gave way to a replacement, which stands
    /// where it did.
    pub(in crate::translation_phases::preprocessing) fn pad_before(&mut self) {
        self.pending = self.pending.then(TokenSpacing::before(self.token_white));
    }

    /// As [`Self::pad_before`], for a parameter, which brings no padding of
    /// its own where it begins its replacement list: the macro's name
    /// already speaks for it.
    pub(in crate::translation_phases::preprocessing) fn pad_before_parameter(&mut self) {
        if !self.token_first {
            self.pad_before();
        }
    }

    /// Notes that the next token given out is a whole replacement.
    pub(in crate::translation_phases::preprocessing) fn pad_after_next(&mut self) {
        self.end_after = true;
    }

    /// Notes a replacement that produced no token, as an empty argument does.
    pub(in crate::translation_phases::preprocessing) fn empty_replacement(&mut self) {
        self.pad_before();
        self.frame_ended(TokenSpacing::END);
        self.end_after = false;
    }

    /// Notes that the last token read, a macro's name with white space before
    /// it or not, was replaced by a list that comes next.
    pub(in crate::translation_phases::preprocessing) fn entered_list(&mut self, white: bool) {
        self.pending = self.pending.then(TokenSpacing::before(white));
        self.list_start = true;
        self.end_after = false;
    }

    /// Notes that a replacement list, which brings no padding, comes next.
    pub(in crate::translation_phases::preprocessing) fn begin_list(&mut self) {
        self.list_start = true;
    }

    /// Notes that a frame ended, with padding `end`. White space at the end
    /// of what it held does not separate what comes next.
    pub(in crate::translation_phases::preprocessing) fn frame_ended(&mut self, end: TokenSpacing) {
        self.pending = self.pending.then(end);
        self.white = false;
        self.list_start = false;
    }

    /// Gives `token`, about to leave the reader, the spacing gathered for
    /// it. White space and placemarkers leave the spacing for the next token.
    pub(in crate::translation_phases::preprocessing) fn give(
        &mut self,
        token: &mut PreprocessorToken,
    ) {
        if matches!(
            token.kind,
            PreprocessorTokenType::Whitespace
                | PreprocessorTokenType::Newline
                | PreprocessorTokenType::Placeholder
        ) {
            return;
        }
        token.spacing = self.pending.with_white(self.token_white);
        self.pending = if std::mem::take(&mut self.end_after) {
            TokenSpacing::END
        } else {
            TokenSpacing::NONE
        };
    }

    /// The padding gathered so far, to go with the argument just read.
    pub(in crate::translation_phases::preprocessing) const fn pending(self) -> TokenSpacing {
        self.pending
    }

    /// The last token read had white space before it.
    pub(in crate::translation_phases::preprocessing) const fn token_white(self) -> bool {
        self.token_white
    }
}

/// The padding that the end of a frame of this type brings.
pub(in crate::translation_phases::preprocessing) fn frame_end(
    frame_type: &TokenizerFrameType<'_>,
) -> TokenSpacing {
    match frame_type {
        | TokenizerFrameType::ObjectLikeMacroInvocation { .. }
        | TokenizerFrameType::FunctionLikeMacroInvocation { .. } => TokenSpacing::END,
        | TokenizerFrameType::Rescan { end, .. }
        | TokenizerFrameType::FunctionLikeMacroArgument { end, .. } => *end,
        | TokenizerFrameType::DeferredQuery | TokenizerFrameType::SourceFile { .. } =>
            TokenSpacing::NONE,
    }
}

impl<'x> Expander<'_, '_, '_, 'x> {
    /// The spacing state, for a text of phase 4 only.
    #[inline(always)]
    pub(in crate::translation_phases::preprocessing) fn text_spacing(
        &mut self,
    ) -> Option<&mut SpacingState> {
        self.state.as_text.then_some(&mut self.spacing)
    }

    /// Notes the end of the frame on top of the stack, which is about to be
    /// popped.
    pub(in crate::translation_phases::preprocessing) fn note_frame_end(&mut self) {
        if self.state.as_text
            && let Some(TokenizerFrame { frame_type, .. }) = self.tokenizer_stack.last()
        {
            self.spacing.frame_ended(frame_end(frame_type));
        }
    }

    /// The padding that ends `argument` where it is substituted.
    pub(in crate::translation_phases::preprocessing) fn prescan_end(&self) -> TokenSpacing {
        self.spacing.argument_end.then(TokenSpacing::END)
    }

    /// Pads the substitution of `argument`, which comes next, for the
    /// parameter just read; returns the padding that ends it.
    pub(in crate::translation_phases::preprocessing) fn pad_substitution(
        &mut self,
        argument: &FunctionLikeMacroArgument<'x>,
    ) -> TokenSpacing {
        let prescan = argument
            .expanded
            .get()
            .expect("the argument was prescanned");
        if let Some(spacing) = self.text_spacing() {
            spacing.pad_before_parameter();
        }
        prescan.end
    }
}
