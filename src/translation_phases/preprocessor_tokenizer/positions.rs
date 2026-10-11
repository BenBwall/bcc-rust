//! Physical source positions after translation phases 1 and 2.
//! [`PositionTracker::advance`] walks increasing offsets through the cleaned
//! buffer and its remaps. Deleted splices and trigraphs still contribute their
//! physical bytes, lines, and columns. This module tracks positions; it does
//! not choose preprocessing-token boundaries.
//!
//! C99: §5.1.1.2 paragraph 1 (phases 1-2), pp. 9-10; PDF pp. 21-22;
//! trigraph replacement §5.2.1.1 paragraph 1, p. 18; PDF p. 30.

use super::{
    SourcePosition,
    splicing::{
        Remap,
        RemapKind,
    },
};
use crate::util::byte_scan;

impl PositionTracker<'_> {
    /// Advances to spliced offset `to` and returns the position there. A splice
    /// deleted immediately before `to` is not yet skipped, because the
    /// token starts before the splice it reads through.
    pub(super) fn advance(&mut self, to: usize) -> SourcePosition {
        debug_assert!(to >= self.clean, "positions are queried in order");
        while let Some(&Remap { clean, kind }) = self.remaps.get(self.next_remap)
            && clean < to
        {
            self.advance_plain(clean);
            match kind {
                | RemapKind::Deleted(length) => self.skip_deleted(length),
                | RemapKind::Trigraph => {
                    self.clean += 1;
                    self.position.index += 3;
                    self.add_columns(3);
                },
                | RemapKind::CarriageReturnLineFeed => {
                    self.clean += 1;
                    self.position.index += 2;
                    self.add_lines(1);
                    self.position.column = 1;
                },
            }
            self.next_remap += 1;
        }
        self.advance_plain(to);
        self.position
    }

    /// Like [`Self::advance`], but also skips splices deleted immediately
    /// before `to`, giving where the character at `to` is spelled.
    pub(super) fn advance_past_deletions(&mut self, to: usize) -> SourcePosition {
        _ = self.advance(to);
        while let Some(&Remap {
            clean,
            kind: RemapKind::Deleted(length),
        }) = self.remaps.get(self.next_remap)
            && clean == to
        {
            self.skip_deleted(length);
            self.next_remap += 1;
        }
        self.position
    }
}

/// Maps monotonically increasing offsets in spliced text back to original
/// source positions.
#[derive(Clone, Debug)]
pub(super) struct PositionTracker<'a> {
    text:                &'a [u8],
    remaps:              &'a [Remap],
    next_remap:          usize,
    clean:               usize,
    pub(super) position: SourcePosition,
}

impl<'a> PositionTracker<'a> {
    fn advance_plain(&mut self, to: usize) {
        let bytes = &self.text[self.clean..to];
        if bytes.is_empty() {
            return;
        }
        if byte_scan::find_line_feed(bytes) == bytes.len() {
            let count = u32::try_from(byte_scan::count_chars(bytes))
                .expect("source column exceeds u32::MAX");
            self.add_columns(count);
        } else {
            let last = bytes
                .iter()
                .rposition(|&byte| byte == b'\n')
                .expect("a line feed was found");
            let lines = u32::try_from(byte_scan::count_line_feeds(bytes))
                .expect("source line exceeds u32::MAX");
            self.add_lines(lines);
            let columns = u32::try_from(byte_scan::count_chars(&bytes[last + 1..]))
                .expect("source column exceeds u32::MAX");
            self.position.column = 1_u32
                .checked_add(columns)
                .expect("source column exceeds u32::MAX");
        }
        self.position.index += bytes.len();
        self.clean = to;
    }

    fn skip_deleted(&mut self, length: usize) {
        self.position.index += length;
        self.add_lines(1);
        self.position.column = 1;
    }

    fn add_lines(&mut self, count: u32) {
        self.position.line = self
            .position
            .line
            .checked_add(count)
            .expect("source line exceeds u32::MAX");
    }

    fn add_columns(&mut self, count: u32) {
        self.position.column = self
            .position
            .column
            .checked_add(count)
            .expect("source column exceeds u32::MAX");
    }

    pub(super) fn new(text: &'a [u8], remaps: &'a [Remap]) -> Self {
        Self {
            text,
            remaps,
            next_remap: 0,
            clean: 0,
            position: SourcePosition::default(),
        }
    }

    /// Whether the byte at the tracker's offset replaced a trigraph.
    pub(super) fn at_trigraph(&self) -> bool {
        matches!(
            self.remaps.get(self.next_remap),
            Some(&Remap {
                clean,
                kind: RemapKind::Trigraph,
            }) if clean == self.clean
        )
    }
}

/// The position of source offset `to`, found by reading forward from `from`
/// in `source`.
pub(crate) fn position_after(source: &str, from: SourcePosition, to: usize) -> SourcePosition {
    let mut position = from;
    let mut characters = source[from.index..to].chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            | '\r' | '\n' => {
                if character == '\r' && characters.peek() == Some(&'\n') {
                    _ = characters.next();
                }
                position.line += 1;
                position.column = 1;
            },
            | _ => position.column += 1,
        }
    }
    position.index = to;
    position
}
