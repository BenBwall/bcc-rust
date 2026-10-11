//! Builds the lexer state for phase-3 token formation and final-newline
//! tracking. Token scanning and diagnostic replay belong to separate modules.
//!
//! C99: §5.1.1.2 paragraph 1 (phase 3), p. 10; PDF p. 22;
//! preprocessing-token categories §6.4 paragraphs 1-3, p. 49; PDF p. 61.

use super::{
    Lexer,
    LexingFile,
    positions::PositionTracker,
    splicing::Remap,
};
use crate::{
    translation_phases::{
        context::Context,
        provenance::SourcePosition,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

impl<'a, 'tu, 'arena, 's> Lexer<'a, 'tu, 'arena, 's> {
    pub(super) fn new(
        context: &'a mut Context<'tu>,
        arena: &'arena Bump,
        scratch: &'s Bump,
        text: &'a str,
        remaps: &'a [Remap],
        physically_empty: bool,
    ) -> Self {
        let bytes = text.as_bytes();
        let lacks_final_newline = !physically_empty && bytes.last() != Some(&b'\n');
        Self {
            ascii: [None; 128],
            context,
            text,
            bytes,
            tracker: PositionTracker::new(bytes, remaps),
            pos: 0,
            current_line_high: 0,
            lacks_final_newline,
            virtual_newline: lacks_final_newline,
            reached_eof: false,
            terminal_splice: false,
            splice_armed: true,
            read_end: false,
            read_final_newline: false,
            scratch,
            pending: ArenaVec::new_in(scratch),
            file: LexingFile {
                entries: arena.tail_vec(),
                line_high_starts: ArenaVec::new_in(scratch),
                end_of_tokens: SourcePosition::default(),
                eof: SourcePosition::default(),
                diagnostics: ArenaVec::new_in(scratch),
                other_locations: ArenaVec::new_in(scratch),
                final_newline_entry: None,
                final_newline_readers: ArenaVec::new_in(scratch),
                lacks_final_newline,
                end_readers: ArenaVec::new_in(scratch),
            },
        }
    }

    pub(super) fn with_terminal_splice(mut self, terminal_splice: bool) -> Self {
        self.terminal_splice = terminal_splice;
        self
    }
}
