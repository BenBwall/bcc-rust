//! The `/O` option, whose letters clang-cl and `cl` read one at a time, so
//! that `/O2s` asks for speed and then size.
//!
//! clang-cl reads every argument `/O` or `-O` starts as this option (`-O3`
//! and even `-ObjC` too), and turns its letters into Clang options in order
//! (`TranslateOptArg` in Clang's `MSVC.cpp`): `s` is `-Os`, `t` is `-O3`, `i`
//! and `i-` are `-fbuiltin` and `-fno-builtin`, `b0` is `-fno-inline`, `b1`
//! to `b3` ask for inlining bcc decides itself, and `g` and `y` (on x86-64)
//! do nothing. Only the last `1`, `2`, `x` or `d` on the whole command line
//! sets a level: `1` is `-Os`, `2` and `x` are `-fbuiltin -O3`, and `d` is
//! `-O0`; the others do nothing. Clang's last level wins, so `/O2s` is
//! `-Os` and `/Os2` is `-O3`. An argument no letter of which does anything,
//! like `/O3`, is unused, which clang-cl's driver warns about. bcc's `-O2`
//! stands for Clang's `-O3`, as for clang-cl's `/O2`.
//!
//! `cl` reads combined letters as the options they name: "/Odi is the same
//! as /Od /Oi" (Microsoft Learn, "/O options (Optimize code)").

use std::ffi::OsString;

/// Where the letter that sets clang-cl's level is: the last `1`, `2`, `x`
/// or `d` among the letters of the `/O` options in `values`, by the index
/// each comes with and the letter's byte offset. A letter right after `b`
/// is `b`'s value, never a level, as clang-cl skips it.
pub(super) fn clang_cl_level_letter<'a>(
    values: impl IntoIterator<Item = (usize, &'a str)>,
) -> Option<(usize, usize)> {
    let mut last = None;
    for (index, letters) in values {
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
/// if this argument holds it. Returns whether any letter does anything;
/// clang-cl warns about the argument otherwise.
pub(super) fn clang_cl_options(
    letters: &str,
    level: Option<usize>,
    out: &mut Vec<OsString>,
) -> bool {
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
                    used |= digit <= b'3';
                    if digit == b'0' {
                        out.push("-fno-inline".into());
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

/// The options `cl`'s `/O` with `letters` combines, each as the letters
/// after `O` (`2s` is `2` and `s`), or `None` if a letter is none of
/// `cl`'s: `1`, `2`, `d`, `g`, `s`, `t` and `x` alone, `i` and `y` with an
/// optional `-`, and `b` with a digit from 0 to 3.
pub(super) fn cl_options(letters: &str) -> Option<Vec<&str>> {
    let bytes = letters.as_bytes();
    let mut options = Vec::new();
    let mut start = 0;
    while let Some(&letter) = bytes.get(start) {
        let length = match (letter, bytes.get(start + 1)) {
            | (b'1' | b'2' | b'd' | b'g' | b's' | b't' | b'x', _) => 1,
            | (b'i' | b'y', Some(b'-')) => 2,
            | (b'i' | b'y', _) => 1,
            | (b'b', Some(b'0'..=b'3')) => 2,
            | _ => return None,
        };
        options.push(&letters[start..start + length]);
        start += length;
    }
    Some(options)
}
