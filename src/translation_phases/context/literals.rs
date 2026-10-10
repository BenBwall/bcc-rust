//! Literal identity, code-unit access, decoding, and diagnostic spelling.
//!
//! Implements string encoding in phase 7 (C99 §6.4.5p5, pp. 62-63;
//! PDF pp. 74-75); original source units remain available for diagnostics.

use super::Context;
use crate::{
    translation_phases::preprocessing::{
        LiteralId,
        LiteralUnit,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl Context<'_> {
    pub(crate) fn intern_literal(&mut self, units: &[LiteralUnit]) -> LiteralId {
        let tu = self.tu;
        LiteralId(
            self.literal_values
                .intern_by(units, || tu.alloc_slice_copy(units)),
        )
    }

    pub(crate) fn literal_units(&self, id: LiteralId) -> &[LiteralUnit] {
        self.literal_values[id.0]
    }

    /// Execution code units of an L-prefixed string, excluding its terminator.
    /// Original source units remain available for diagnostics and inspection.
    /// C99: implementation-defined encoding §6.4.5p5, pp. 62-63; PDF pp. 74-75.
    pub(crate) fn wide_literal_units(&self, id: LiteralId) -> impl Iterator<Item = u32> + '_ {
        let utf16 = self.configuration.target().layout().wide_utf16();
        self.literal_units(id).iter().flat_map(move |unit| {
            let mut units = [0; 2];
            let count = match *unit {
                | LiteralUnit::Character(c) if utf16 => {
                    let mut encoded = [0; 2];
                    let n = c.encode_utf16(&mut encoded).len();
                    units = encoded.map(u32::from);
                    n
                },
                | LiteralUnit::Character(c) => {
                    units[0] = u32::from(c);
                    1
                },
                | LiteralUnit::Numeric(code) => {
                    units[0] = code;
                    1
                },
            };
            units.into_iter().take(count)
        })
    }

    /// The literal's characters spelled in `arena`, if they are text.
    /// Text-only consumers (filenames and tests) must reject non-UTF-8
    /// values.
    pub(crate) fn literal_text_in<'a>(
        &self,
        arena: &'a Bump,
        id: LiteralId,
        wide: bool,
    ) -> Option<&'a str> {
        let text = self.decode_literal_text(arena, id, wide)?.leak();
        // SAFETY: decoding checked that these bytes are UTF-8.
        Some(unsafe { std::str::from_utf8_unchecked(text) })
    }

    /// The literal's characters as UTF-8 in `arena`, if they are text.
    fn decode_literal_text<'a>(
        &self,
        arena: &'a Bump,
        id: LiteralId,
        wide: bool,
    ) -> Option<ArenaVec<'a, u8>> {
        let mut text = ArenaVec::new_in(arena);
        for unit in self.literal_units(id) {
            let character = match *unit {
                | LiteralUnit::Character(c) => c,
                | LiteralUnit::Numeric(code) if wide => char::from_u32(code)?,
                | LiteralUnit::Numeric(code) => {
                    text.push(u8::try_from(code).expect("narrow escape checked during decoding"));
                    continue;
                },
            };
            text.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
        }
        std::str::from_utf8(&text).is_ok().then_some(text)
    }

    /// The literal as a C string literal in `arena`: quoted and escaped when
    /// its characters are text, and as numeric escapes otherwise. The text is
    /// decoded in `scratch` and taken back, unless something else is
    /// allocated there meanwhile.
    /// `prefix` selects byte decoding for ordinary/UTF-8 strings and full
    /// code-unit decoding for `L`, `u`, and `U` strings.
    ///
    /// C11: numeric escapes use the corresponding character type,
    /// §6.4.4.4 paragraph 9, p. 69; PDF p. 87; string element types are
    /// specified by §6.4.5 paragraph 6, p. 71; PDF p. 89.
    pub(crate) fn literal_spelling_in<'a>(
        &self,
        arena: &'a Bump,
        scratch: &Bump,
        id: LiteralId,
        prefix: &str,
    ) -> &'a str {
        let wide = matches!(prefix, "L" | "u" | "U");
        let mut spelling = ArenaString::new_in(arena);
        let text = self.decode_literal_text(scratch, id, wide);
        let text = text
            .as_deref()
            .and_then(|text| std::str::from_utf8(text).ok());
        self.write_literal_spelling(&mut spelling, text, id, prefix)
            .expect("arena formatting cannot fail");
        spelling.into_str()
    }

    /// Writes the literal as a C string literal: quoted and escaped when
    /// `text` holds its characters, and as numeric escapes otherwise.
    fn write_literal_spelling(
        &self,
        out: &mut impl std::fmt::Write,
        text: Option<&str>,
        id: LiteralId,
        prefix: &str,
    ) -> std::fmt::Result {
        let wide = matches!(prefix, "L" | "u" | "U");
        if let Some(text) = text {
            return crate::diagnostics::write_c_quoted(out, prefix, '"', text);
        }
        out.write_str(prefix)?;
        out.write_char('"')?;
        for unit in self.literal_units(id) {
            match *unit {
                | LiteralUnit::Character(c) if wide => write!(out, "\\x{:x}", u32::from(c))?,
                | LiteralUnit::Numeric(code) if wide => write!(out, "\\x{code:x}")?,
                | LiteralUnit::Character(c) =>
                    for byte in c.encode_utf8(&mut [0; 4]).bytes() {
                        write!(out, "\\{byte:03o}")?;
                    },
                | LiteralUnit::Numeric(code) => write!(
                    out,
                    "\\{:03o}",
                    u8::try_from(code).expect("narrow escape checked during decoding")
                )?,
            }
        }
        out.write_char('"')
    }
}
