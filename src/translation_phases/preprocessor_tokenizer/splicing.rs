//! Whole-buffer translation phases 1 and 2. [`splice`] replaces enabled
//! trigraphs, maps physical line endings to LF, and deletes line splices.
//! [`Remap`] records where cleaned offsets stop matching physical source
//! offsets; [`logical_characters`] reconstructs characters in an existing
//! source span. Token formation and diagnostic replay belong to the lexer and
//! token source.
//!
//! C99: §5.1.1.2 paragraph 1 (phases 1-2), pp. 9-10; PDF pp. 21-22;
//! trigraph replacement §5.2.1.1 paragraph 1, p. 18; PDF p. 30.
//! Accepting LF, CRLF, and CR is an implementation-defined phase-1 mapping.

use std::ops::Range;

use crate::util::{
    bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
    byte_scan,
};

/// Applies translation phases 1 and 2 to a whole buffer: trigraphs are
/// replaced, line endings become `'\n'`, and line splices are deleted. The
/// returned remaps record every place where spliced and original offsets stop
/// advancing together. A buffer that needs changes is copied into `scratch`.
/// C99: §5.1.1.2p1-2, pp. 9-10; PDF pp. 21-22; trigraph mapping §5.2.1.1p1,
/// p. 18; PDF p. 30.
pub(super) fn splice<'a>(
    source: &'a str,
    scratch: &'a Bump,
    trigraphs: bool,
) -> (&'a str, &'a [Remap]) {
    let bytes = source.as_bytes();
    let mut special = byte_scan::find_phase2_special(bytes);
    if special == bytes.len() {
        return (source, &[]);
    }
    // Phases 1 and 2 only shorten the text, so its buffer never grows; the
    // remaps after it are the arena's latest block and grow in place.
    let mut text = ArenaString::with_capacity_in(bytes.len(), scratch);
    let mut remaps = ArenaVec::new_in(scratch);
    let mut copied = 0;
    while special < bytes.len() {
        // Every special byte is ASCII, so these are character boundaries.
        text.push_str(&source[copied..special]);
        let rest = &bytes[special..];
        special += match rest {
            | [b'\r', b'\n', ..] => {
                remaps.push(Remap {
                    clean: text.len(),
                    kind:  RemapKind::CarriageReturnLineFeed,
                });
                text.push('\n');
                2
            },
            | [b'\r', ..] => {
                text.push('\n');
                1
            },
            | [b'\\', after @ ..] => match line_ending_length(after) {
                | Some(ending) => {
                    remaps.push(Remap {
                        clean: text.len(),
                        kind:  RemapKind::Deleted(1 + ending),
                    });
                    1 + ending
                },
                | None => {
                    text.push('\\');
                    1
                },
            },
            | [b'?', b'?', third, after @ ..]
                if trigraphs && let Some(replacement) = trigraph_replacement(*third) =>
                match line_ending_length(after) {
                    // `??/` splices like a backslash.
                    | Some(ending) if replacement == '\\' => {
                        remaps.push(Remap {
                            clean: text.len(),
                            kind:  RemapKind::Deleted(3 + ending),
                        });
                        3 + ending
                    },
                    | _ => {
                        remaps.push(Remap {
                            clean: text.len(),
                            kind:  RemapKind::Trigraph,
                        });
                        text.push(replacement);
                        3
                    },
                },
            | _ => {
                text.push('?');
                1
            },
        };
        copied = special;
        special += byte_scan::find_phase2_special(&bytes[special..]);
    }
    text.push_str(&source[copied..]);
    (text.into_str(), remaps.leak())
}

/// How one byte of spliced text, or the gap before it, maps to the original
/// source when the mapping is not one byte to one byte.
#[derive(Clone, Copy, Debug)]
pub(super) enum RemapKind {
    /// A line splice of this many original bytes was deleted before the byte.
    Deleted(usize),
    /// The byte replaced a three-character trigraph.
    Trigraph,
    /// The line feed replaced a carriage return and line feed.
    CarriageReturnLineFeed,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Remap {
    /// Offset in the spliced text.
    pub(super) clean: usize,
    pub(super) kind:  RemapKind,
}

/// The characters that `source[range]` spells once trigraphs are replaced
/// and line splices deleted. The range must not end inside a trigraph or
/// line splice; token spans never do. The characters are collected in `arena`.
/// C99: §5.1.1.2p1, p. 9; PDF p. 21; §5.1.1.2p2, p. 10; PDF p. 22.
pub(crate) fn logical_characters<'a>(
    arena: &'a Bump,
    source: &str,
    range: Range<usize>,
    trigraphs: bool,
) -> ArenaVec<'a, LogicalCharacter> {
    let bytes = &source.as_bytes()[..range.end];
    let mut characters = ArenaVec::new_in(arena);
    let mut index = range.start;
    while index < range.end {
        let rest = &bytes[index..];
        let splice = match rest {
            | [b'\\', after @ ..] => line_ending_length(after).map(|ending| 1 + ending),
            | [b'?', b'?', b'/', after @ ..] if trigraphs =>
                line_ending_length(after).map(|ending| 3 + ending),
            | _ => None,
        };
        if let Some(length) = splice {
            index += length;
            continue;
        }
        let (character, length) = match rest {
            | [b'?', b'?', third, ..]
                if trigraphs && let Some(replacement) = trigraph_replacement(*third) =>
                (replacement, 3),
            | _ => {
                let character = source[index..]
                    .chars()
                    .next()
                    .expect("ranges start at character boundaries");
                (character, character.len_utf8())
            },
        };
        characters.push(LogicalCharacter {
            character,
            index,
            length,
        });
        index += length;
    }
    characters
}

/// One character after translation phases 1 and 2, with the source bytes
/// that spell it.
/// C99: §5.1.1.2p1-2, pp. 9-10; PDF pp. 21-22.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogicalCharacter {
    pub(crate) character: char,
    /// Offset of its spelling in the source.
    pub(crate) index:     usize,
    /// Length of its spelling in bytes: 3 for a trigraph.
    pub(crate) length:    usize,
}

/// Accepts LF, CRLF, and CR as physical end-of-line indicators.
/// C99: phase-1 mapping is implementation-defined, §5.1.1.2p1, p. 9; PDF p. 21;
/// source new-lines §5.2.1p3, p. 17; PDF p. 29.
fn line_ending_length(bytes: &[u8]) -> Option<usize> {
    match bytes {
        | [b'\r', b'\n', ..] => Some(2),
        | [b'\n' | b'\r', ..] => Some(1),
        | _ => None,
    }
}

/// Maps the nine trigraph suffixes to their source characters.
/// C99: §5.2.1.1p1, p. 18; PDF p. 30.
fn trigraph_replacement(byte: u8) -> Option<char> {
    Some(match byte {
        | b'=' => '#',
        | b')' => ']',
        | b'!' => '|',
        | b'(' => '[',
        | b'\'' => '^',
        | b'>' => '}',
        | b'/' => '\\',
        | b'<' => '{',
        | b'-' => '~',
        | _ => return None,
    })
}
