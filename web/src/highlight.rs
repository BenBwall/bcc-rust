//! Rust-side syntax colouring for the prerendered code views. Monaco takes
//! over in the browser; until then (and without JavaScript) these spans show
//! C with the same token classes the editor themes use.
//!
//! Program text is always rendered as text nodes, never as HTML.

use leptos::prelude::*;

/// A code view's text as numbered lines, coloured when it is C.
pub fn fallback(text: &str, language: &str) -> AnyView {
    let lines = if language == "c" {
        lines(text)
    } else {
        plain_lines(text)
    };
    lines
        .into_iter()
        .map(|line| {
            let number = line.number;
            view! {
                <span class="fallback-line">
                    <span class="fallback-number" aria-hidden="true">{number}</span>
                    <span class="fallback-code">{segments(&line)}</span>
                </span>
            }
        })
        .collect_view()
        .into_any()
}

/// One source line with its syntax tokens.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Line {
    number: usize,
    text:   String,
    /// Syntax tokens as byte ranges within `text`.
    tokens: Vec<(usize, usize, Kind)>,
}

/// Split `source` into lines carrying their syntax tokens.
fn lines(source: &str) -> Vec<Line> {
    let tokens = syntax_tokens(source);
    let mut offset = 0;
    source
        .split('\n')
        .enumerate()
        .map(|(index, text)| {
            let end = offset + text.len();
            let line_tokens = tokens
                .iter()
                .filter(|token| token.0 < end && token.1 > offset)
                .map(|&(start, stop, kind)| {
                    (start.max(offset) - offset, stop.min(end) - offset, kind)
                })
                .collect();
            offset = end + 1;
            Line {
                number: index + 1,
                text:   text.to_owned(),
                tokens: line_tokens,
            }
        })
        .collect()
}

fn plain_lines(text: &str) -> Vec<Line> {
    text.split('\n')
        .enumerate()
        .map(|(index, text)| Line {
            number: index + 1,
            text:   text.to_owned(),
            tokens: Vec::new(),
        })
        .collect()
}

fn segments(line: &Line) -> AnyView {
    let text = line.text.as_str();
    if text.is_empty() {
        return " ".into_any();
    }
    let mut points = vec![0, text.len()];
    for &(start, end, _) in &line.tokens {
        points.extend([start, end]);
    }
    points.sort_unstable();
    points.dedup();
    points
        .windows(2)
        .map(|pair| {
            let (from, to) = (pair[0], pair[1]);
            let class = line
                .tokens
                .iter()
                .find(|token| token.0 <= from && token.1 > from)
                .map_or("", |token| token.2.class());
            let piece = text[from..to].to_owned();
            view! { <span class=class>{piece}</span> }
        })
        .collect_view()
        .into_any()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Comment,
    String,
    Keyword,
    Type,
    Number,
}

impl Kind {
    fn class(self) -> &'static str {
        match self {
            | Self::Comment => "syntax-comment",
            | Self::String => "syntax-string",
            | Self::Keyword => "syntax-keyword",
            | Self::Type => "syntax-type",
            | Self::Number => "syntax-number",
        }
    }
}

const TYPES: [&str; 7] = ["int", "void", "char", "short", "long", "unsigned", "signed"];

const KEYWORDS: [&str; 22] = [
    "const", "volatile", "static", "extern", "auto", "register", "return", "for", "while", "do",
    "if", "else", "break", "continue", "sizeof", "struct", "union", "enum", "typedef", "switch",
    "case", "default",
];

/// Colouring tokens as byte ranges: comments, string and character literals,
/// keywords, and numbers. Unterminated literals and comments stay plain.
fn syntax_tokens(source: &str) -> Vec<(usize, usize, Kind)> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        let token = if rest.starts_with(b"//") {
            let end = rest
                .iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(rest.len());
            Some((end, Kind::Comment))
        } else if rest.starts_with(b"/*") {
            rest.windows(2)
                .skip(2)
                .position(|pair| pair == b"*/")
                .map(|end| (end + 4, Kind::Comment))
        } else if matches!(rest[0], b'"' | b'\'') {
            literal_length(rest).map(|end| (end, Kind::String))
        } else if is_word(rest[0]) && (index == 0 || !is_word(bytes[index - 1])) {
            let word = rest
                .iter()
                .position(|&byte| !is_word(byte))
                .unwrap_or(rest.len());
            let text = &source[index..index + word];
            if TYPES.contains(&text) {
                Some((word, Kind::Type))
            } else if KEYWORDS.contains(&text) {
                Some((word, Kind::Keyword))
            } else if let Some(length) = rest[0]
                .is_ascii_digit()
                .then(|| number_length(rest))
                .flatten()
            {
                Some((length, Kind::Number))
            } else {
                // No token starts inside a word.
                index += word;
                continue;
            }
        } else {
            None
        };
        match token {
            | Some((length, kind)) => {
                tokens.push((index, index + length, kind));
                index += length;
            },
            | None => index += 1,
        }
    }
    tokens
}

/// The length of a quoted literal starting at `rest[0]`. An escaped line break
/// or a missing closing quote leaves the text uncoloured.
fn literal_length(rest: &[u8]) -> Option<usize> {
    let quote = rest[0];
    let mut index = 1;
    while index < rest.len() {
        match rest[index] {
            | b'\\' if matches!(rest.get(index + 1), None | Some(b'\n' | b'\r')) => return None,
            | b'\\' => index += 2,
            | byte if byte == quote => return Some(index + 1),
            | _ => index += 1,
        }
    }
    None
}

/// The length of a hexadecimal or decimal number with optional suffixes,
/// which must end at a word boundary.
fn number_length(rest: &[u8]) -> Option<usize> {
    let run = |from: usize, accept: fn(&u8) -> bool| {
        from + rest[from..].iter().take_while(|byte| accept(byte)).count()
    };
    let suffixed = |end: usize| {
        let end = run(end, |byte| b"uUlLfF".contains(byte));
        (!rest.get(end).copied().is_some_and(is_word)).then_some(end)
    };
    if rest.len() > 2
        && rest[0] == b'0'
        && matches!(rest[1], b'x' | b'X')
        && rest[2].is_ascii_hexdigit()
        && let Some(end) = suffixed(run(2, u8::is_ascii_hexdigit))
    {
        return Some(end);
    }
    let digits = run(0, u8::is_ascii_digit);
    if rest.get(digits) == Some(&b'.')
        && rest.get(digits + 1).is_some_and(u8::is_ascii_digit)
        && let Some(end) = suffixed(run(digits + 1, u8::is_ascii_digit))
    {
        return Some(end);
    }
    suffixed(digits)
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(&str, Kind)> {
        syntax_tokens(source)
            .into_iter()
            .map(|(start, end, kind)| (&source[start..end], kind))
            .collect()
    }

    #[test]
    fn colours_comments_literals_keywords_and_numbers() {
        assert_eq!(
            kinds("int x = 0x1Fu; /* a\nb */ return 'c' + \"s\\\"\" + 1.5; // end"),
            [
                ("int", Kind::Type),
                ("0x1Fu", Kind::Number),
                ("/* a\nb */", Kind::Comment),
                ("return", Kind::Keyword),
                ("'c'", Kind::String),
                ("\"s\\\"\"", Kind::String),
                ("1.5", Kind::Number),
                ("// end", Kind::Comment),
            ]
        );
    }

    #[test]
    fn ignores_words_containing_keywords_and_unterminated_text() {
        assert_eq!(
            kinds("integer x1 1x 1.5x \"open /* open"),
            [("1", Kind::Number)]
        );
        assert_eq!(kinds("\"a\\\nb\""), []);
    }

    #[test]
    fn splits_block_comment_tokens_across_lines() {
        let lines = lines("a /* b\nc */ d");
        assert_eq!(lines[0].tokens, [(2, 6, Kind::Comment)]);
        assert_eq!(lines[1].tokens, [(0, 4, Kind::Comment)]);
    }
}
