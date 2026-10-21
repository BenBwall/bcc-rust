//! ASCII subset tokenization with exact ranges; no preprocessing is attempted.

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
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        let byte = bytes[offset];
        if byte.is_ascii_whitespace() {
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
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            offset += 1;
            while offset < bytes.len()
                && (bytes[offset].is_ascii_alphanumeric() || bytes[offset] == b'_')
            {
                offset += 1;
            }
        } else if b"{}()[];,+-*/%<>=!&".contains(&byte) {
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
                )
            {
                offset += 1;
            }
        } else {
            let end = start + source[start..].chars().next().map_or(1, char::len_utf8);
            return Err(Diagnostic::new(
                "unsupported",
                range(source, start, end),
                "unsupported character; preprocessing, strings, and non-ASCII syntax are outside \
                 this subset",
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
