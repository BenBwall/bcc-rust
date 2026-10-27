//! The front of the compiler, phase by phase: what translation phases 1-2
//! (logical lines), 3 (preprocessing tokens) and 4 (the preprocessor) made
//! of a program, as the page shows it. The compiler Worker fills these from
//! bcc's own output for each phase (`explorer.rs`); this module only
//! arranges it: the line labels of the logical lines, the links between
//! each view's lines and the source, and the preprocessed text with each
//! header's contents folded into one line.

use serde::{
    Deserialize,
    Serialize,
};

use super::SourceSpan;

/// One logical line: the physical lines phase 2 spliced into it and what
/// phases 1 and 2 changed in it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalLine {
    /// The physical lines, both inclusive.
    pub source:  (usize, usize),
    pub changes: Vec<SourceChange>,
}

/// One change phases 1 and 2 made, or a UTF-8 character they kept.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceChange {
    /// `splice`, `trigraph`, `crlf` or `extended`.
    pub kind:        String,
    /// The UTF-16 column in the logical line where it happened.
    pub column:      usize,
    /// Where the source spelled it, in the editor's coordinates; the end
    /// column is just past it.
    pub source:      SourceSpan,
    pub spelling:    String,
    pub replacement: String,
}

impl SourceChange {
    /// A sentence on what happened here.
    pub fn describe(&self) -> String {
        match self.kind.as_str() {
            | "splice" => format!(
                "Line {} ends in a backslash, so phase 2 deleted it with the new-line and line {} \
                 continues this one.",
                self.source.line,
                self.source.line + 1
            ),
            | "trigraph" => format!(
                "The trigraph {} on line {} became {}.",
                self.spelling, self.source.line, self.replacement
            ),
            | "crlf" => format!(
                "Line {} ends in a carriage return and a line feed, read as one new-line.",
                self.source.line
            ),
            | _ => {
                let bytes = self.spelling.len();
                let characters = self.spelling.chars().count();
                format!(
                    "{} on line {} is {} {} of UTF-8 but {} source {}.",
                    self.spelling,
                    self.source.line,
                    bytes,
                    if bytes == 1 { "byte" } else { "bytes" },
                    characters,
                    if characters == 1 {
                        "character"
                    } else {
                        "characters"
                    }
                )
            },
        }
    }
}

/// What phase 4 did, for the source file only: headers' own expansions
/// and conditionals are left out.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preprocessing {
    pub expansions:   Vec<Expansion>,
    pub conditionals: Vec<Conditional>,
    /// The lines of each skipped group, both inclusive.
    pub skipped:      Vec<(usize, usize)>,
    pub includes:     Vec<Include>,
    pub definitions:  Vec<Definition>,
}

/// A macro invocation in the source and what it became.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expansion {
    pub range:      SourceSpan,
    pub invocation: String,
    /// The tokens it produced; empty for one a directive read, such as the
    /// name in `#if VERBOSE`.
    pub result:     String,
    pub steps:      Vec<ExpansionStep>,
}

/// One replacement, `#` or `##` of an expansion.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpansionStep {
    /// `replace`, `stringify` or `paste`.
    pub kind:       String,
    pub macro_name: String,
    /// Replacements around a replaced name: 0 for the name in the source.
    pub depth:      usize,
    /// Each parameter and its argument, as written.
    pub arguments:  Vec<(String, String)>,
    /// `##`'s operands; empty for a placemarker.
    pub operands:   Vec<String>,
    pub result:     String,
}

impl ExpansionStep {
    /// The step in one line, such as `x = 1 + 2` or `#x → "a"`.
    pub fn describe(&self) -> String {
        match self.kind.as_str() {
            | "stringify" => format!("# made the string literal {}", self.result),
            | "paste" => {
                let operand = |text: &str| {
                    if text.is_empty() {
                        "(nothing)".to_owned()
                    } else {
                        text.to_owned()
                    }
                };
                let left = self
                    .operands
                    .first()
                    .map_or_else(String::new, |o| operand(o));
                let right = self
                    .operands
                    .get(1)
                    .map_or_else(String::new, |o| operand(o));
                format!("{left} ## {right} pasted into {}", operand(&self.result))
            },
            | _ if self.arguments.is_empty() => format!("{} replaced", self.macro_name),
            | _ => {
                let arguments: Vec<String> = self
                    .arguments
                    .iter()
                    .map(|(parameter, argument)| {
                        if argument.is_empty() {
                            format!("{parameter} = (empty)")
                        } else {
                            format!("{parameter} = {argument}")
                        }
                    })
                    .collect();
                format!("{} replaced with {}", self.macro_name, arguments.join(", "))
            },
        }
    }
}

/// A conditional directive in the source and whether it kept its group.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conditional {
    pub line:      usize,
    pub directive: String,
    pub taken:     bool,
}

/// An `#include` in the source.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Include {
    pub line:   usize,
    pub header: String,
    pub angled: bool,
    /// Whether a header was found; the in-browser compiler has only bcc's
    /// own freestanding headers.
    pub found:  bool,
    pub system: bool,
    /// How many lines the header holds.
    pub lines:  usize,
}

/// A `#define` or `#undef` in the source.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub line:      usize,
    pub name:      String,
    /// `object`, `function` or `undef`.
    pub kind:      String,
    pub directive: String,
}

/// The gutter label of each logical line: its physical line, or the first
/// and last of the lines a splice joined.
pub fn line_labels(lines: &[LogicalLine]) -> Vec<String> {
    lines
        .iter()
        .map(|line| match line.source {
            | (first, last) if first == last => first.to_string(),
            | (first, last) => format!("{first}–{last}"),
        })
        .collect()
}

/// A line marker, `# 12 "<input>" 2`: the line the next line has in a file,
/// the file, and whether it enters (1) or returns to (2) it.
fn line_marker(line: &str) -> Option<(usize, &str, Option<u8>)> {
    let rest = line.strip_prefix("# ")?;
    let (number, rest) = rest.split_once(' ')?;
    let number = number.parse().ok()?;
    let rest = rest.strip_prefix('"')?;
    let (file, flags) = rest.split_once('"')?;
    let flag = flags
        .split_whitespace()
        .next()
        .and_then(|flag| flag.parse().ok());
    Some((number, file, flag))
}

/// The preprocessed text as the view shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreprocessedView {
    pub text:   String,
    /// Each line's source lines (index 0 unused): its own, or the
    /// `#include` that brought in a header's line.
    pub links:  Vec<Option<(usize, usize)>>,
    /// Each line's gutter label: the source line it came from, empty for a
    /// line marker or a header's own line.
    pub labels: Vec<String>,
}

/// The preprocessed text as the view shows it. With `fold`, every header
/// the source includes is folded into one line that says what it held:
/// header text is mostly declarations a lesson never reads, and the full
/// text stays a switch away.
pub fn preprocessed_view(
    text: &str,
    source_lines: usize,
    includes: &[Include],
    fold: bool,
) -> PreprocessedView {
    let links = preprocessed_lines(text, source_lines);
    let label = |line: &str, link: Option<(usize, usize)>, in_header: bool| match link {
        | Some((first, _)) if !in_header && line_marker(line).is_none() => first.to_string(),
        | _ => String::new(),
    };
    if !fold {
        let mut depth = 0usize;
        let labels = text
            .lines()
            .enumerate()
            .map(|(index, line)| {
                match line_marker(line) {
                    | Some((_, _, Some(1))) => depth += 1,
                    | Some((_, _, Some(2))) => depth = depth.saturating_sub(1),
                    | _ => {},
                }
                label(line, links.get(index + 1).copied().flatten(), depth > 0)
            })
            .collect();
        return PreprocessedView {
            text: text.to_owned(),
            links,
            labels,
        };
    }
    let mut folded = String::new();
    let mut folded_links = vec![None];
    let mut labels = Vec::new();
    let mut depth = 0usize;
    let mut held = 0usize;
    let mut found = includes.iter().filter(|include| include.found);
    let mut current: Option<&Include> = None;
    let mut include_link = None;
    for (index, line) in text.lines().enumerate() {
        let link = links.get(index + 1).copied().flatten();
        let marker = line_marker(line);
        match marker {
            | Some((_, _, Some(1))) => {
                if depth == 0 {
                    current = found.next();
                    held = 0;
                    include_link = link;
                }
                depth += 1;
                continue;
            },
            | Some((_, _, Some(2))) if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    folded.push_str(&summary(current, held));
                    folded.push('\n');
                    folded_links.push(include_link);
                    labels
                        .push(include_link.map_or_else(String::new, |(line, _)| line.to_string()));
                }
                continue;
            },
            | _ => {},
        }
        if depth > 0 {
            if marker.is_none() && !line.trim().is_empty() {
                held += 1;
            }
            continue;
        }
        folded.push_str(line);
        folded.push('\n');
        folded_links.push(link);
        labels.push(label(line, link, false));
    }
    PreprocessedView {
        text: folded,
        links: folded_links,
        labels,
    }
}

/// Whether `line` of the view is a folded header's summary.
pub fn is_folded(line: &str) -> bool {
    line.starts_with("/* ")
        && (line.ends_with(FOLDED_DECLARATIONS) || line.ends_with(FOLDED_MACROS))
}

const FOLDED_DECLARATIONS: &str = "of declarations, folded (Show headers to read them) */";
const FOLDED_MACROS: &str = "only macro definitions, which the preprocessor kept for itself */";

/// The line that stands for a folded header.
fn summary(include: Option<&Include>, held: usize) -> String {
    let name = include.map_or_else(
        || "a header".to_owned(),
        |include| {
            if include.angled {
                format!("<{}>", include.header)
            } else {
                format!("\"{}\"", include.header)
            }
        },
    );
    if held == 0 {
        format!("/* {name}: {FOLDED_MACROS}")
    } else {
        format!(
            "/* {name}: {held} {} {FOLDED_DECLARATIONS}",
            if held == 1 { "line" } else { "lines" }
        )
    }
}

/// For each line of the full preprocessed text (index 0 unused), the source
/// lines it came from: its own line in the source, or for text from a
/// header, the `#include` line that brought it in. A header's line markers
/// link like its text.
fn preprocessed_lines(text: &str, source_lines: usize) -> Vec<Option<(usize, usize)>> {
    let input = crate::INPUT_NAME;
    let mut lines = vec![None];
    // The next line's place in the source, or `None` in a header, and the
    // header lines waiting for the line their `#include` is on.
    let mut next = Some(1usize);
    let mut depth = 0usize;
    let mut waiting = Vec::new();
    for line in text.lines() {
        let Some((number, file, flag)) = line_marker(line) else {
            match next {
                | Some(number) => {
                    lines.push((number <= source_lines).then_some((number, number)));
                    next = Some(number + 1);
                },
                | None => {
                    waiting.push(lines.len());
                    lines.push(None);
                },
            }
            continue;
        };
        match flag {
            | Some(1) => depth += 1,
            | Some(2) => depth = depth.saturating_sub(1),
            | _ => {},
        }
        if depth == 0 && file == input {
            let link = if flag == Some(2) {
                // Back from a header: the line before is its `#include`.
                let include = number.saturating_sub(1).max(1);
                for index in waiting.drain(..) {
                    lines[index] = Some((include, include));
                }
                Some((include, include))
            } else {
                None
            };
            lines.push(link);
            next = Some(number);
        } else {
            next = None;
            waiting.push(lines.len());
            lines.push(None);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# 1 \"<input>\"\nint a;\n# 1 \"/bcc/stdint.h\" 1 3\ntypedef long \
                        int64_t;\n\ntypedef int int32_t;\n# 3 \"<input>\" 2\nint b;\n";

    fn include() -> Include {
        Include {
            line:   2,
            header: "stdint.h".into(),
            angled: true,
            found:  true,
            system: true,
            lines:  40,
        }
    }

    #[test]
    fn headers_fold_into_one_line_that_counts_their_declarations() {
        let folded = preprocessed_view(TEXT, 3, &[include()], true).text;
        assert!(is_folded(folded.lines().nth(2).unwrap()));
        assert_eq!(
            folded,
            "# 1 \"<input>\"\nint a;\n/* <stdint.h>: 2 lines of declarations, folded (Show \
             headers to read them) */\nint b;\n"
        );
        let macros = preprocessed_view(
            "# 1 \"<input>\"\n# 1 \"/l.h\" 1 3\n\n# 2 \"<input>\" 2\nx\n",
            2,
            &[include()],
            true,
        )
        .text;
        assert!(macros.contains("only macro definitions"), "{macros}");
        assert!(is_folded(macros.lines().nth(1).unwrap()), "{macros}");
    }

    #[test]
    fn preprocessed_lines_link_to_their_source_line_or_include() {
        // The marker, `int a;`, the header's marker and lines (linked to its
        // `#include`), the return marker, `int b;`.
        let full = preprocessed_view(TEXT, 3, &[include()], false);
        assert_eq!(full.labels, ["", "1", "", "", "", "", "", "3"]);
        assert_eq!(
            full.links,
            [
                None,
                None,
                Some((1, 1)),
                Some((2, 2)),
                Some((2, 2)),
                Some((2, 2)),
                Some((2, 2)),
                Some((2, 2)),
                Some((3, 3))
            ]
        );
        let folded = preprocessed_view(TEXT, 3, &[include()], true);
        assert_eq!(
            folded.links,
            [None, None, Some((1, 1)), Some((2, 2)), Some((3, 3))]
        );
        assert_eq!(folded.labels, ["", "1", "2", "3"]);
    }

    #[test]
    fn spliced_lines_are_labelled_with_every_physical_line() {
        let lines = [
            LogicalLine {
                source: (1, 1),
                ..LogicalLine::default()
            },
            LogicalLine {
                source: (2, 4),
                ..LogicalLine::default()
            },
        ];
        assert_eq!(line_labels(&lines), ["1", "2–4"]);
    }
}
