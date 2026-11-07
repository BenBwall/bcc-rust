//! Format strings and where their bytes are written. A diagnostic about one
//! conversion points into the string, so each byte of a literal's value is
//! traced back through its tokens' spellings: an escape sequence stands at
//! its backslash, and each concatenated token is counted, since GCC places
//! a diagnostic in the first token and notes one in a later token. A
//! literal a macro wrote, or one whose spelling cannot be traced, has no
//! such places, and its diagnostics stay at the literal's expansion.
//! C99: §6.4.5p4-5, pp. 62-63; PDF pp. 74-75 (escape sequences and
//! concatenation); §5.1.1.2p1 items 5-6, p. 10; PDF p. 22.

use super::super::Analyzer;
use crate::{
    translation_phases::{
        SourcePosition,
        SourceVectors,
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
        preprocessing::{
            LiteralEncoding,
            LiteralId,
            LiteralUnit,
            StringTokenType,
        },
    },
    util::bump::ArenaVec,
};

/// A string literal a format argument comes to.
#[derive(Clone, Copy)]
pub(super) struct Leaf<'tu> {
    /// The string literal.
    pub(super) literal:   &'tu Expression<'tu>,
    pub(super) id:        LiteralId,
    /// The byte the format starts at: `"..." + N` starts later.
    pub(super) offset:    usize,
    /// The expression the compiler read the string from, where it reports
    /// what it says of the whole string.
    pub(super) site:      SourceVectors,
    /// Where GCC reports a conversion it cannot place in the string: the
    /// site, or the outermost operator of an offset into the string.
    pub(super) char_site: SourceVectors,
    /// Whether the string is the call's argument, rather than a constant
    /// object's initializer: Clang's `InFunctionCall`.
    pub(super) in_call:   bool,
    /// Whether GCC reads the string where it is written: the expression
    /// itself, neither parenthesized nor a constant object's initializer.
    pub(super) bare:      bool,
    /// Who wrote the string's first token.
    pub(super) spelled:   Spelled,
    /// The elements of the constant array the string initializes, which
    /// may leave out its null character.
    pub(super) extent:    Option<usize>,
}

/// Who wrote a string literal's first token.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Spelled {
    /// A file, outside any macro invocation.
    File,
    /// A file, as an argument of a macro invocation.
    MacroArgument,
    /// A macro's replacement list.
    MacroBody,
}

/// Where one byte of a literal's value is written.
#[derive(Clone, Copy)]
pub(super) struct BytePosition {
    file:             u32,
    index:            u32,
    line:             u32,
    column:           u32,
    /// The length of the character there.
    width:            u8,
    /// The token that wrote it, from 0, in a concatenation.
    pub(super) token: u32,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// The string literal `e` is, through parentheses, and whether there
    /// were any.
    pub(super) fn string_literal(e: &'tu Expression<'tu>) -> Option<(&'tu Expression<'tu>, bool)> {
        let mut inner = e;
        let mut parenthesized = false;
        while let ExpressionType::Parenthesized { expression } = inner.kind {
            inner = expression;
            parenthesized = true;
        }
        narrow_literal(inner).map(|_| (inner, parenthesized))
    }

    /// A leaf for the narrow string literal `literal`, read from `site`.
    pub(super) fn leaf(
        &self,
        literal: &'tu Expression<'tu>,
        site: &'tu Expression<'tu>,
        offset: usize,
        in_call: bool,
        bare: bool,
    ) -> Option<Leaf<'tu>> {
        let id = narrow_literal(literal)?;
        let source = literal.source_vectors;
        let spelled = if source.length() == 0 || !self.context.produced_by_macro_expansion(source) {
            Spelled::File
        } else if self.context.written_by_macro_body(source) {
            Spelled::MacroBody
        } else {
            Spelled::MacroArgument
        };
        Some(Leaf {
            literal,
            id,
            offset,
            site: site.source_vectors,
            char_site: site.source_vectors,
            in_call,
            bare,
            spelled,
            extent: None,
        })
    }

    /// The bytes of `leaf`'s format, without the terminating null, from its
    /// offset.
    pub(super) fn format_bytes(&self, leaf: &Leaf<'tu>) -> &'tu [u8] {
        let mut bytes = ArenaVec::new_in(self.types.tu);
        for unit in self.context.literal_units(leaf.id) {
            match *unit {
                | LiteralUnit::Character(c) =>
                    bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                | LiteralUnit::Numeric(code) => bytes.push(code.to_le_bytes()[0]),
            }
        }
        let bytes = bytes.leak();
        bytes.get(leaf.offset..).unwrap_or_default()
    }

    /// Where each byte of the literal `literal`'s value is written, or
    /// `None` when its tokens' spellings do not account for its value.
    pub(super) fn byte_positions(
        &self,
        literal: &'tu Expression<'tu>,
    ) -> Option<&'tu [BytePosition]> {
        let id = narrow_literal(literal)?;
        // The spelling's bytes, each with its place.
        let mut spelled = ArenaVec::new_in(self.scratch);
        for vector in self.context.get_source_vectors(literal.source_vectors) {
            let text = self
                .context
                .source_text(vector.source_file_index)?
                .as_bytes();
            let bytes = text.get(vector.index as usize..vector.end())?;
            // Columns count characters, as the lexer counts them.
            let mut column = vector.column;
            for (offset, &byte) in bytes.iter().enumerate() {
                let offset = u32::try_from(offset).ok()?;
                if offset != 0 && byte & 0xC0 != 0x80 {
                    column += 1;
                }
                let width = utf8_length(byte).min(bytes.len() - offset as usize);
                spelled.push((
                    byte,
                    BytePosition {
                        file: vector.source_file_index,
                        index: vector.index + offset,
                        line: vector.line,
                        column,
                        width: u8::try_from(width).unwrap_or(1),
                        token: 0,
                    },
                ));
            }
        }
        // One place for each unit of the value: each token's characters
        // between its quotes, an escape sequence at its backslash.
        let mut places = ArenaVec::new_in(self.scratch);
        let mut token = None::<u32>;
        let mut inside = false;
        let mut i = 0;
        while i < spelled.len() {
            let (byte, mut place) = spelled[i];
            if !inside {
                if byte == b'"' {
                    inside = true;
                    token = Some(token.map_or(0, |token| token + 1));
                }
                i += 1;
                continue;
            }
            place.token = token.unwrap_or_default();
            if byte == b'"' {
                inside = false;
                i += 1;
                continue;
            }
            places.push(place);
            i += if byte == b'\\' {
                1 + escape_length(&spelled[i + 1..])
            } else {
                utf8_length(byte)
            };
        }
        let units = self.context.literal_units(id);
        if places.len() != units.len() {
            return None;
        }
        let mut positions = ArenaVec::new_in(self.types.tu);
        for (unit, &place) in units.iter().zip(places.iter()) {
            let count = match *unit {
                | LiteralUnit::Character(c) => c.len_utf8(),
                | LiteralUnit::Numeric(_) => 1,
            };
            for _ in 0..count {
                positions.push(place);
            }
        }
        Some(positions.leak())
    }

    /// A location for the byte at `position`, one character wide.
    pub(super) fn byte_location(&mut self, position: BytePosition) -> SourceVectors {
        self.context.create_retained_source_vectors(
            SourcePosition {
                index:  position.index as usize,
                line:   position.line,
                column: position.column,
            },
            position.file,
            usize::from(position.width),
        )
    }
}

/// The value of a narrow string literal, plain or UTF-8, which both
/// compilers read as a format; a wide one is none.
/// C11: §6.4.5p3, p. 70; PDF p. 88.
fn narrow_literal(literal: &Expression<'_>) -> Option<LiteralId> {
    match literal.kind {
        | ExpressionType::StringLiteral(
            StringTokenType::String(id) | StringTokenType::EncodedString(id, LiteralEncoding::Utf8),
        ) => Some(id),
        | _ => None,
    }
}

/// The length of the escape sequence whose backslash precedes `rest`.
/// C99: §6.4.4.4p1, p. 59; PDF p. 71.
fn escape_length<T>(rest: &[(u8, T)]) -> usize {
    let byte = |index: usize| rest.get(index).map(|(byte, _)| *byte);
    match byte(0) {
        | Some(b'0'..=b'7') =>
            1 + (1..3)
                .take_while(|&index| matches!(byte(index), Some(b'0'..=b'7')))
                .count(),
        | Some(b'x') =>
            1 + (1..rest.len())
                .take_while(|&index| byte(index).is_some_and(|byte| byte.is_ascii_hexdigit()))
                .count(),
        | Some(b'u') => 5,
        | Some(b'U') => 9,
        | Some(byte) => utf8_length(byte),
        | None => 0,
    }
}

/// The length of the UTF-8 sequence `lead` begins.
const fn utf8_length(lead: u8) -> usize {
    match lead {
        | 0xF0.. => 4,
        | 0xE0.. => 3,
        | 0xC0.. => 2,
        | _ => 1,
    }
}
