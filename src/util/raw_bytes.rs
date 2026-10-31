//! Source bytes that are not UTF-8, carried through `str` source text.
//!
//! Source text is held as `str`, so
//! [`crate::util::bump::Bump::read_to_str_lossy`] reads each byte of malformed
//! UTF-8 as a stand-in character of its own: byte `b` (always 0x80-0xFF, since
//! ASCII is valid) becomes U+10FF00 + `b`, from the end of Supplementary
//! Private Use Area-B. Phase 5 turns a stand-in in a narrow or UTF-8 literal
//! back into its byte, as GCC and Clang keep such bytes; `-E` writes the byte
//! back, and diagnostics show it as `<FF>`, as Clang does.
//!
//! The stand-ins stand for bytes only in a translation unit that read
//! malformed UTF-8 (`Context::raw_source_bytes`). Elsewhere they are the
//! private-use characters they spell; a unit that has both keeps the bytes'
//! meaning, the only spelling the reader could not tell apart.
//!
//! C99: the mapping of physical source bytes to the source character set is
//! implementation-defined, phase 1, §5.1.1.2 paragraph 1, p. 10; PDF p. 22.

use std::{
    io::{
        self,
        Write,
    },
    path::Path,
};

use crate::util::bump::{
    ArenaString,
    ArenaVec,
    Bump,
};

/// The stand-in for the malformed-UTF-8 byte `byte`.
pub(crate) const fn stand_in(byte: u8) -> char {
    char::from_u32(BASE + byte as u32).expect("U+10FF80-U+10FFFF are characters")
}

/// The byte `c` stands for, if it is a stand-in.
pub(crate) fn raw_byte(c: char) -> Option<u8> {
    u32::from(c)
        .checked_sub(BASE)
        .and_then(|byte| u8::try_from(byte).ok())
        .filter(|byte| *byte >= 0x80)
}

/// Writes `text` with each stand-in replaced by its byte when `raw_bytes`,
/// as the translation unit read them.
pub(crate) fn write(
    out: &mut (impl Write + ?Sized),
    text: &str,
    raw_bytes: bool,
) -> io::Result<()> {
    if !raw_bytes {
        return out.write_all(text.as_bytes());
    }
    let mut rest = text;
    // Every stand-in is encoded as F4 8F BE or BF and one more byte.
    while let Some(lead) = rest.bytes().position(|byte| byte == 0xF4) {
        out.write_all(&rest.as_bytes()[..lead])?;
        let c = rest[lead..]
            .chars()
            .next()
            .expect("a lead byte starts a character");
        match raw_byte(c) {
            | Some(byte) => out.write_all(&[byte])?,
            | None => out.write_all(c.encode_utf8(&mut [0; 4]).as_bytes())?,
        }
        rest = &rest[lead + c.len_utf8()..];
    }
    out.write_all(rest.as_bytes())
}

/// The path a name in source text spells: where paths are bytes, each
/// stand-in is its byte again, so `#include "h<FF>.h"` opens the file whose
/// name holds that byte, as GCC and Clang open it.
pub(crate) fn path<'a>(arena: &'a Bump, text: &'a str) -> &'a Path {
    #[cfg(unix)]
    if text.chars().any(|c| raw_byte(c).is_some()) {
        use std::os::unix::ffi::OsStrExt as _;
        let mut bytes = ArenaVec::with_capacity_in(text.len(), arena);
        for c in text.chars() {
            match raw_byte(c) {
                | Some(byte) => bytes.push(byte),
                | None => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            }
        }
        return Path::new(std::ffi::OsStr::from_bytes(arena.alloc_slice_copy(&bytes)));
    }
    #[cfg(not(unix))]
    let _ = arena;
    Path::new(text)
}

/// The text of a path whose bytes are not UTF-8, each byte of malformed
/// UTF-8 a stand-in, as `__FILE__` spells it; `None` for a UTF-8 path or
/// where paths are not bytes.
pub(crate) fn path_text<'a>(arena: &'a Bump, path: &Path) -> Option<&'a str> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let bytes = path.as_os_str().as_bytes();
        if std::str::from_utf8(bytes).is_ok() {
            return None;
        }
        let mut text = ArenaString::new_in(arena);
        for chunk in bytes.utf8_chunks() {
            text.push_str(chunk.valid());
            for &byte in chunk.invalid() {
                text.push(stand_in(byte));
            }
        }
        Some(text.into_str())
    }
    #[cfg(not(unix))]
    {
        let _ = (arena, path);
        None
    }
}

/// The first stand-in, for byte 0x00; only 0x80-0xFF are used.
const BASE: u32 = 0x0010_FF00;

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build expected bytes with std types; the arena rule covers the compiler."
)]
mod tests {
    use super::*;

    #[test]
    fn stand_ins_round_trip_high_bytes_only() {
        for byte in 0x80..=0xFF {
            assert_eq!(raw_byte(stand_in(byte)), Some(byte));
        }
        assert_eq!(raw_byte('\u{10FF7F}'), None);
        assert_eq!(raw_byte('a'), None);
        assert_eq!(raw_byte('\u{FFFD}'), None);
    }

    #[test]
    fn write_restores_bytes_only_when_asked() {
        let text = format!("a{}b\u{10FFFF}", stand_in(0xE2));
        let mut out = Vec::new();
        write(&mut out, &text, true).unwrap();
        assert_eq!(out, b"a\xE2b\xFF");
        out.clear();
        write(&mut out, "\u{1F600}x", true).unwrap();
        assert_eq!(out, "\u{1F600}x".as_bytes());
        out.clear();
        write(&mut out, &text, false).unwrap();
        assert_eq!(out, text.as_bytes());
    }
}
