//! The `/O` option, whose letters clang-cl and `cl` read one at a time, so
//! that `/O2s` asks for speed and then size.
//!
//! clang-cl reads every argument `/O` or `-O` starts as this option (`-O3`
//! and even `-ObjC` too), and turns its letters into Clang options in order
//! (`TranslateOptArg` in Clang's `MSVC.cpp`): `s` is `-Os`, `t` is `-O3`, `i`
//! and `i-` are `-fbuiltin` and `-fno-builtin`, `b0` is `-fno-inline`, `b1`
//! is `-finline-hint-functions`, `b2` and `b3` are `-finline-functions`
//! (which [`ClangInlining`] reads), and `g` and `y` (on x86-64) do nothing.
//! Only the last `1`, `2`, `x` or `d` on the whole command line sets a level:
//! `1` is `-Os`, `2` and `x` are `-fbuiltin -O3`, and `d` is `-O0`; the others
//! do nothing. Clang's last level wins, so `/O2s` is `-Os` and `/Os2` is `-O3`.
//! An argument no letter of which does anything, like `/O3`, is unused, which
//! clang-cl's driver warns about. bcc's `-O2` stands for Clang's `-O3`, as for
//! clang-cl's `/O2`.
//!
//! icx-cl also has `/O3`, which clang-cl does not: Intel documents `/O[n]`
//! with "1, 2, or 3", and "The last O option specified on the command line
//! takes precedence over any others" (oneAPI DPC++/C++ Compiler Developer
//! Guide and Reference 2026.2, "O"). So icx-cl reads `/O3` and `-O3` as a
//! level, Clang's `-O3`, which stands among the level letters.
//!
//! `cl` reads combined letters as the options they name: "/Odi is the same
//! as /Od /Oi" (Microsoft Learn, "/O options (Optimize code)"). `cl` 19.44
//! takes the letters it knows from an argument that also holds others, and
//! warns about each other letter as an unknown option (`/O2-` is `/O2`, with
//! warning D9002 about `/O-`); it warns that `/Og`, `/Og-` and `/O` alone are
//! deprecated (D9035).

use std::ffi::OsString;

use super::ClangInlining;

/// Where the letter that sets clang-cl's level is: the last `1`, `2`, `x`
/// or `d` among the letters of the `/O` options in `values`, by the index
/// each comes with and the letter's byte offset. A letter right after `b`
/// is `b`'s value, never a level, as clang-cl skips it. For icx-cl
/// (`intel`), `/O3` is a level too.
pub(super) fn clang_cl_level_letter<'a>(
    values: impl IntoIterator<Item = (usize, &'a str)>,
    intel: bool,
) -> Option<(usize, usize)> {
    let mut last = None;
    for (index, letters) in values {
        if is_intel_level(letters, intel) {
            last = Some((index, 0));
            continue;
        }
        let bytes = letters.as_bytes();
        for (offset, letter) in bytes.iter().enumerate() {
            let after_b = offset > 0 && bytes[offset - 1] == b'b';
            if !after_b && matches!(letter, b'1' | b'2' | b'x' | b'd') {
                last = Some((index, offset));
            }
        }
    }
    last
}

/// Appends to `out` the bcc options clang-cl's `/O` with `letters` gives,
/// in order, where `level` is the offset of the letter that sets the level
/// if this argument holds it, and gives `inlining` the inlining options
/// `b` stands for. icx-cl's `/O3` (`intel`) is `-O3` if it sets the level.
/// Returns whether any letter does anything; clang-cl warns about the
/// argument otherwise.
pub(super) fn clang_cl_options(
    letters: &str,
    level: Option<usize>,
    intel: bool,
    inlining: &mut ClangInlining,
    out: &mut Vec<OsString>,
) -> bool {
    if is_intel_level(letters, intel) {
        if level == Some(0) {
            out.push("-O3".into());
        }
        return true;
    }
    let bytes = letters.as_bytes();
    let mut used = false;
    let mut offset = 0;
    while let Some(&letter) = bytes.get(offset) {
        let next = bytes.get(offset + 1).copied();
        match letter {
            | b'1' | b'2' | b'x' | b'd' => {
                used = true;
                if level == Some(offset) {
                    match letter {
                        | b'd' => out.push("-O0".into()),
                        | b'1' => out.push("-Os".into()),
                        | _ => out.extend(["-fbuiltin", "-O2"].map(OsString::from)),
                    }
                }
            },
            // `b` takes the digit after it, which only 0 to 3 make useful.
            | b'b' =>
                if let Some(digit) = next.filter(u8::is_ascii_digit) {
                    offset += 1;
                    let option = match digit {
                        | b'0' => Some("-fno-inline"),
                        | b'1' => Some("-finline-hint-functions"),
                        | b'2' | b'3' => Some("-finline-functions"),
                        | _ => None,
                    };
                    if let Some(option) = option {
                        used = true;
                        _ = inlining.read(option);
                    }
                },
            | b'i' => {
                used = true;
                if next == Some(b'-') {
                    offset += 1;
                    out.push("-fno-builtin".into());
                } else {
                    out.push("-fbuiltin".into());
                }
            },
            | b's' => {
                used = true;
                out.push("-Os".into());
            },
            | b't' => {
                used = true;
                out.push("-O2".into());
            },
            | b'g' => used = true,
            // Frame-pointer omission, which x86-64 always allows.
            | b'y' => {
                used = true;
                if next == Some(b'-') {
                    offset += 1;
                }
            },
            | _ => {},
        }
        offset += 1;
    }
    used
}

/// Whether `letters` are those of icx-cl's `/O3` under `intel`.
fn is_intel_level(letters: &str, intel: bool) -> bool {
    intel && letters == "3"
}

/// One of the options `cl`'s `/O` combines ([`cl_letters`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClLetters<'a> {
    /// The letters of an option `cl` knows, after `O` (`2`, `i-`, `b1`).
    Option(&'a str),
    /// Letters `cl` warns about as an unknown option (D9002) and skips.
    Unknown(&'a str),
}

/// The options `cl`'s `/O` with `letters` combines, each as the letters
/// after `O` (`2s` is `2` and `s`): `1`, `2`, `d`, `s`, `t` and `x` alone,
/// `g`, `i` and `y` with an optional `-`, and `b` with a digit from 0 to 3.
/// Any other letter is an unknown option of its own, as is `b` with any
/// other digit or with none, which `cl` 19.44 skips with its warning while
/// it takes the rest (`/O2-` is `/O2` and warns about `/O-`).
pub(super) fn cl_letters(letters: &str) -> Vec<ClLetters<'_>> {
    let bytes = letters.as_bytes();
    let mut options = Vec::new();
    let mut start = 0;
    while let Some(&letter) = bytes.get(start) {
        let next = bytes.get(start + 1).copied();
        let (length, known) = match (letter, next) {
            | (b'g' | b'i' | b'y', Some(b'-')) | (b'b', Some(b'0'..=b'3')) => (2, true),
            | (b'b', Some(b'4'..=b'9')) => (2, false),
            | (b'1' | b'2' | b'd' | b'g' | b'i' | b's' | b't' | b'x' | b'y', _) => (1, true),
            // Any other character, whole.
            | _ => (
                letters[start..].chars().next().map_or(1, char::len_utf8),
                false,
            ),
        };
        let piece = &letters[start..start + length];
        options.push(if known {
            ClLetters::Option(piece)
        } else {
            ClLetters::Unknown(piece)
        });
        start += length;
    }
    options
}

#[cfg(test)]
mod tests {
    use super::{
        ClLetters::{
            Option,
            Unknown,
        },
        cl_letters,
    };

    /// Each row was checked against `cl` 19.44's warnings.
    #[test]
    fn cl_reads_the_letters_it_knows_and_skips_the_others() {
        for (letters, expected) in [
            ("2s", &[Option("2"), Option("s")][..]),
            ("i-g-y-", &[Option("i-"), Option("g-"), Option("y-")]),
            ("b2x", &[Option("b2"), Option("x")]),
            ("2-", &[Option("2"), Unknown("-")]),
            ("2zz", &[Option("2"), Unknown("z"), Unknown("z")]),
            ("b4", &[Unknown("b4")]),
            ("bx", &[Unknown("b"), Option("x")]),
            ("b-", &[Unknown("b"), Unknown("-")]),
            ("x-", &[Option("x"), Unknown("-")]),
            ("3", &[Unknown("3")]),
            ("\u{e9}", &[Unknown("\u{e9}")]),
            ("", &[]),
        ] {
            assert_eq!(cl_letters(letters), expected, "{letters}");
        }
    }
}
