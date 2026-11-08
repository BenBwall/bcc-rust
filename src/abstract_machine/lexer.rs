//! ASCII subset tokenization with exact ranges. A line that starts with `#`
//! is one token, the whole directive, for the parser to accept or reject;
//! character constants and string literals are single tokens too, decoded
//! by [`literal_bytes`].
//!
//! C99: §6.4.4.4 paragraphs 2-10, pp. 59-61; PDF pp. 71-73 (character
//! constants and escape sequences).
//! C99: §6.4.5 paragraph 2, p. 62; PDF p. 74 (string literals).
//! C99: §6.10 paragraph 2, p. 146; PDF p. 158 (a directive starts a line).

use super::model::{
    Diagnostic,
    MAX_SOURCE,
    SourceRange,
};

pub(super) fn lex(source: &str) -> Result<Vec<Token<'_>>, Diagnostic> {
    if source.len() > MAX_SOURCE {
        return Err(Diagnostic::limit(
            range(source, 0, 0),
            "source exceeds 32 KiB",
        ));
    }
    // Phases 1-2 are deliberately outside this standalone parser. Reject
    // transformations even in comments rather than silently change meaning.
    if let Some(offset) = source
        .as_bytes()
        .windows(2)
        .position(|pair| pair == b"\\\n" || pair == b"\\\r")
    {
        return Err(Diagnostic::new(
            "unsupported",
            range(source, offset, offset + 2),
            "line splicing is outside this subset",
        ));
    }
    if let Some(offset) = source
        .as_bytes()
        .windows(3)
        .position(|triple| triple.starts_with(b"??") && b"=/(')!<>-".contains(&triple[2]))
    {
        return Err(Diagnostic::new(
            "unsupported",
            range(source, offset, offset + 3),
            "trigraph processing is outside this subset",
        ));
    }
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut offset = 0;
    // Whether only white space and comments precede `offset` on its line.
    let mut line_start = true;
    while offset < bytes.len() {
        let start = offset;
        let byte = bytes[offset];
        if byte.is_ascii_whitespace() {
            line_start |= byte == b'\n';
            offset += 1;
            continue;
        }
        if bytes[offset..].starts_with(b"//") {
            while offset < bytes.len() && bytes[offset] != b'\n' {
                offset += 1;
            }
            continue;
        }
        if bytes[offset..].starts_with(b"/*") {
            offset += 2;
            while offset < bytes.len() && !bytes[offset..].starts_with(b"*/") {
                offset += 1;
            }
            if offset == bytes.len() {
                return Err(Diagnostic::new(
                    "syntax",
                    range(source, start, offset),
                    "unterminated comment",
                ));
            }
            offset += 2;
            continue;
        }
        let first_on_line = std::mem::replace(&mut line_start, false);
        if byte == b'#' && first_on_line {
            offset = directive_end(source, start)?;
        } else if byte == b'"' || byte == b'\'' {
            offset = literal_end(source, start)?;
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            offset += 1;
            while offset < bytes.len()
                && (bytes[offset].is_ascii_alphanumeric() || bytes[offset] == b'_')
            {
                offset += 1;
            }
            if matches!(&source[start..offset], "L" | "u" | "U" | "u8")
                && matches!(bytes.get(offset), Some(b'"' | b'\''))
            {
                return Err(Diagnostic::new(
                    "unsupported",
                    range(source, start, offset + 1),
                    "wide and Unicode literals are outside this subset; the machine models char \
                     strings",
                ));
            }
            if byte.is_ascii_digit() && bytes.get(offset) == Some(&b'.') {
                return Err(Diagnostic::new(
                    "unsupported",
                    range(source, start, offset + 1),
                    "floating point is outside this subset; the machine models int and char",
                ));
            }
        } else if b"{}()[];,+-*/%<>=!&.".contains(&byte) {
            offset += 1;
            if offset < bytes.len()
                && matches!(
                    &bytes[start..=offset],
                    b"++"
                        | b"--"
                        | b"=="
                        | b"!="
                        | b"<="
                        | b">="
                        | b"+="
                        | b"-="
                        | b"*="
                        | b"/="
                        | b"&&"
                        | b"->"
                )
            {
                offset += 1;
            }
        } else {
            let end = start + source[start..].chars().next().map_or(1, char::len_utf8);
            return Err(Diagnostic::new(
                "unsupported",
                range(source, start, end),
                "unsupported character; the machine reads ASCII C, and #, other than to start a \
                 directive line, is outside this subset",
            ));
        }
        tokens.push(Token {
            text:  &source[start..offset],
            range: range(source, start, offset),
        });
    }
    tokens.push(Token {
        text:  "",
        range: range(source, source.len(), source.len()),
    });
    Ok(tokens)
}

/// The end of the directive starting with the `#` at `start`: the end of
/// its line, without trailing white space or a trailing comment.
fn directive_end(source: &str, start: usize) -> Result<usize, Diagnostic> {
    let line = source[start..]
        .find('\n')
        .map_or(source.len(), |end| start + end);
    let mut end = line;
    for marker in ["//", "/*"] {
        if let Some(comment) = source[start..line].find(marker) {
            end = end.min(start + comment);
        }
    }
    if source[end..line].starts_with("/*") && !source[end..line].trim_end().ends_with("*/") {
        return Err(Diagnostic::new(
            "unsupported",
            range(source, end, line),
            "a comment that continues past its directive's line is outside this subset",
        ));
    }
    Ok(start + source[start..end].trim_end().len())
}

/// The end of the character constant or string literal opening at `start`.
fn literal_end(source: &str, start: usize) -> Result<usize, Diagnostic> {
    let bytes = source.as_bytes();
    let quote = bytes[start];
    let mut at = start + 1;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            | b'\n' => break,
            | b'\\' => at += 2,
            | _ if byte == quote => return Ok(at + 1),
            | _ if !byte.is_ascii() => {
                let end = at + source[at..].chars().next().map_or(1, char::len_utf8);
                return Err(Diagnostic::new(
                    "unsupported",
                    range(source, at, end),
                    "non-ASCII characters in literals are outside this subset; their values \
                     depend on the execution character set (C99 §6.4.5p5)",
                ));
            },
            | _ => at += 1,
        }
    }
    Err(Diagnostic::new(
        "syntax",
        range(source, start, at.min(source.len())),
        if quote == b'"' {
            "unterminated string literal: it must end with \" on the same line"
        } else {
            "unterminated character constant: it must end with ' on the same line"
        },
    ))
}

/// The bytes a character constant or string literal token stands for,
/// escape sequences replaced, without the quotes and without the null a
/// string literal gets in translation phase 7.
///
/// C99: §6.4.4.4 paragraph 9, p. 61; PDF p. 73 (an escape's value fits in
/// unsigned char).
pub(super) fn literal_bytes(source: &str, token: Token<'_>) -> Result<Vec<u8>, Diagnostic> {
    let text = token.text.as_bytes();
    let body = &text[1..text.len() - 1];
    let base = token.range.start + 1;
    let error = |at: usize, length: usize, message: &str| {
        Diagnostic::new(
            "syntax",
            range(source, base + at, (base + at + length).min(source.len())),
            message,
        )
    };
    let mut bytes = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let byte = body[at];
        if byte != b'\\' {
            bytes.push(byte);
            at += 1;
            continue;
        }
        let Some(&escape) = body.get(at + 1) else {
            return Err(error(at, 1, "a backslash must start an escape sequence"));
        };
        let simple = match escape {
            | b'\'' | b'"' | b'?' | b'\\' => Some(escape),
            | b'a' => Some(0x07),
            | b'b' => Some(0x08),
            | b'f' => Some(0x0C),
            | b'n' => Some(b'\n'),
            | b'r' => Some(b'\r'),
            | b't' => Some(b'\t'),
            | b'v' => Some(0x0B),
            | _ => None,
        };
        if let Some(value) = simple {
            bytes.push(value);
            at += 2;
            continue;
        }
        let (digits, radix, skip) = if escape == b'x' {
            let count = body[at + 2..]
                .iter()
                .take_while(|digit| digit.is_ascii_hexdigit())
                .count();
            (count, 16, 2)
        } else {
            let count = body[at + 1..]
                .iter()
                .take(3)
                .take_while(|digit| (b'0'..=b'7').contains(digit))
                .count();
            (count, 8, 1)
        };
        if digits == 0 {
            return Err(error(
                at,
                2,
                "unknown escape sequence; C99 has \\n, \\t, \\0, \\x41 and the other escapes of \
                 §6.4.4.4",
            ));
        }
        let spelled = std::str::from_utf8(&body[at + skip..at + skip + digits]).unwrap_or("");
        let Some(value) = u32::from_str_radix(spelled, radix)
            .ok()
            .and_then(|value| u8::try_from(value).ok())
        else {
            return Err(error(
                at,
                skip + digits,
                "the escape's value does not fit in an unsigned char (C99 §6.4.4.4p9)",
            ));
        };
        bytes.push(value);
        at += skip + digits;
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Token<'a> {
    pub text:  &'a str,
    pub range: SourceRange,
}

pub(super) fn range(source: &str, start: usize, end: usize) -> SourceRange {
    let position = |offset| {
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
        let column = prefix
            .rfind('\n')
            .map_or(offset + 1, |newline| offset - newline);
        (line, column)
    };
    let (line, column) = position(start);
    let (end_line, end_column) = position(end);
    SourceRange {
        start,
        end,
        line,
        column,
        end_line,
        end_column,
    }
}
