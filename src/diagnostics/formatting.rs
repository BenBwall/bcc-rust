use std::fmt::Write as _;

use super::{
    ArenaString,
    Bump,
    Display,
    fmt,
};

/// The function behind [`format_in!`].
pub(crate) fn format_arguments_in<'d>(arena: &'d Bump, arguments: fmt::Arguments<'_>) -> &'d str {
    if let Some(text) = arguments.as_str() {
        return text;
    }
    let mut text = ArenaString::new_in(arena);
    let _ = text.write_fmt(arguments);
    text.into_str()
}

/// Writes `value` as C source text inside the given quotes, escaping
/// characters that would otherwise be invisible or ambiguous.
pub(crate) fn write_c_quoted(
    out: &mut impl fmt::Write,
    prefix: &str,
    quote: char,
    value: &str,
) -> fmt::Result {
    out.write_str(prefix)?;
    out.write_char(quote)?;
    for c in value.chars() {
        match c {
            | '\\' => out.write_str("\\\\")?,
            | '\n' => out.write_str("\\n")?,
            | '\t' => out.write_str("\\t")?,
            | '\r' => out.write_str("\\r")?,
            | '\x07' => out.write_str("\\a")?,
            | '\x08' => out.write_str("\\b")?,
            | '\x0B' => out.write_str("\\v")?,
            | '\x0C' => out.write_str("\\f")?,
            // A fixed-width escape cannot absorb a following octal digit.
            | '\0' => out.write_str("\\000")?,
            | c if c == quote => {
                out.write_char('\\')?;
                out.write_char(c)?;
            },
            | c if u32::from(c) < 0x20 || c == '\x7F' => write!(out, "\\{:03o}", u32::from(c))?,
            | c if c.is_control() => write!(out, "\\u{:04X}", u32::from(c))?,
            | c => out.write_char(c)?,
        }
    }
    out.write_char(quote)
}

/// Quotes a token spelling for use inside a message, shortening long ones.
pub(crate) fn quote_spelling(spelling: &str) -> impl Display {
    let spelling = spelling.trim_end_matches('\0');
    fmt::from_fn(move |f| {
        match spelling.char_indices().nth(MAX_QUOTED_SPELLING - 1) {
            // Longer than the limit: keep the characters before the last one
            // that would fit.
            | Some((cut, _)) if spelling.chars().nth(MAX_QUOTED_SPELLING).is_some() =>
                write!(f, "`{}…`", &spelling[..cut]),
            | _ => write!(f, "`{spelling}`"),
        }
    })
}

/// Formats `count noun`, pluralizing the noun with `s` when needed.
pub(crate) fn count_of(count: usize, noun: &str) -> impl Display {
    fmt::from_fn(move |f| {
        if count == 1 {
            write!(f, "{count} {noun}")
        } else {
            write!(f, "{count} {noun}s")
        }
    })
}

/// Returns the candidate closest to `word` when it is a plausible typo
/// (at most two single-character edits, and fewer than the word's length).
/// The comparison works in `scratch`.
pub(crate) fn closest_match<'a>(
    scratch: &Bump,
    word: &str,
    candidates: &[&'a str],
) -> Option<&'a str> {
    let distance = |a: &str, b: &str| {
        let b = scratch.alloc_slice_fill_iter(b.chars());
        let row = scratch.alloc_slice_fill_iter(0..=b.len());
        for (i, ca) in a.chars().enumerate() {
            let mut diagonal = row[0];
            row[0] = i + 1;
            for (j, &cb) in b.iter().enumerate() {
                let above = row[j + 1];
                row[j + 1] = (above + 1)
                    .min(row[j] + 1)
                    .min(diagonal + usize::from(ca != cb));
                diagonal = above;
            }
        }
        row[b.len()]
    };
    candidates
        .iter()
        .map(|&candidate| (distance(word, candidate), candidate))
        .filter(|&(distance, _)| distance > 0 && distance <= 2 && distance < word.chars().count())
        .min_by_key(|&(distance, _)| distance)
        .map(|(_, candidate)| candidate)
}

/// Longest token spelling quoted inline in a message before it is shortened.
pub(super) const MAX_QUOTED_SPELLING: usize = 40;

/// Formats text into an arena, like `format!` into a `String`; the result
/// borrows the arena. Text without arguments is returned without copying.
macro_rules! format_in {
    ($arena:expr, $($arguments:tt)*) => {
        $crate::diagnostics::format_arguments_in($arena, format_args!($($arguments)*))
    };
}

pub(crate) use format_in;
