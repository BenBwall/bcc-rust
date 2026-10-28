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

use super::{
    LineLinks,
    SourceSpan,
};

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
    pub expansions:         Vec<Expansion>,
    pub conditionals:       Vec<Conditional>,
    /// The lines of each skipped group, both inclusive.
    pub skipped:            Vec<(usize, usize)>,
    pub includes:           Vec<Include>,
    pub definitions:        Vec<Definition>,
    /// Every directive executed in the source.
    pub directives:         Vec<Directive>,
    /// Expansions beyond the most the page receives.
    pub omitted_expansions: usize,
    /// Whether the compiler's record of phase 4 stopped early, as it does
    /// for a macro expansion that multiplies its input.
    pub truncated:          bool,
}

/// A macro invocation in the source and what it became.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expansion {
    pub range:         SourceSpan,
    /// The place `#line` gives it, when that differs.
    pub presumed:      Option<String>,
    pub invocation:    String,
    /// Whether a directive read it, as `#if` reads its condition.
    pub directive:     bool,
    /// The tokens it produced; empty for one a directive read, such as the
    /// name in `#if VERBOSE`.
    pub result:        String,
    pub steps:         Vec<ExpansionStep>,
    /// Whether the compiler's record stopped inside this expansion, so its
    /// result and steps may lack the end.
    pub partial:       bool,
    /// Steps beyond the most the page receives.
    pub omitted_steps: usize,
}

/// One replacement, `#` or `##` of an expansion.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpansionStep {
    /// `replace`, `builtin`, `stringify` or `paste`.
    pub kind:       String,
    /// Whether the name came from an argument, replaced before it is
    /// substituted.
    pub prescan:    bool,
    pub macro_name: String,
    /// How deep in the tree of replacements the step is: 0 for the name in
    /// the source.
    pub depth:      usize,
    /// The step, among the expansion's, that this one happened in.
    pub parent:     Option<usize>,
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
            | "builtin" => format!(
                "{} is predefined: it became {}",
                self.macro_name, self.result
            ),
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
            | _ if self.arguments.is_empty() && self.prescan => format!(
                "{} replaced in an argument, before it is substituted",
                self.macro_name
            ),
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
                let place = if self.prescan {
                    " in an argument, before it is substituted,"
                } else {
                    ""
                };
                format!(
                    "{} replaced{place} with {}",
                    self.macro_name,
                    arguments.join(", ")
                )
            },
        }
    }
}

/// A conditional directive in the source and whether it kept its group.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conditional {
    pub line:      usize,
    pub presumed:  Option<String>,
    pub directive: String,
    pub taken:     bool,
    /// Whether its condition was evaluated: an arm after a kept group is
    /// skipped without it.
    pub evaluated: bool,
}

/// A directive executed in the source.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Directive {
    pub line:     usize,
    pub presumed: Option<String>,
    /// Its name, such as `pragma`.
    pub name:     String,
    /// The whole directive as written.
    pub text:     String,
}

/// Where one line of the preprocessed text came from.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLine {
    /// Whether its text comes from the source rather than a header.
    pub input:   bool,
    /// The line of its file its text comes from, if it holds text.
    pub line:    Option<usize>,
    /// For a header's line, the source line whose `#include` brought it in.
    pub include: Option<usize>,
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

/// The preprocessed text as the view shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreprocessedView {
    pub text:   String,
    /// Each line's source lines (index 0 unused): its own, or the
    /// `#include` that brought in a header's line.
    pub links:  LineLinks,
    /// Each line's gutter label: the source line it came from, empty for a
    /// line marker or a header's own line.
    pub labels: Vec<String>,
}

/// The preprocessed text as the view shows it, given where each of its
/// lines came from (`origins`, from the compiler). With `fold`, the lines
/// each `#include` of the source brought in, line markers too, become one
/// line that says what they held: header text is mostly declarations a
/// lesson never reads, and the full text stays a switch away.
pub fn preprocessed_view(
    text: &str,
    origins: &[OutputLine],
    includes: &[Include],
    fold: bool,
) -> PreprocessedView {
    let link = |origin: &OutputLine| match (origin.include, origin.line) {
        | (Some(include), _) => Some((include, include)),
        | (None, Some(line)) if origin.input => Some((line, line)),
        | _ => None,
    };
    let label = |origin: &OutputLine| match origin {
        | OutputLine {
            input: true,
            line: Some(number),
            ..
        } => number.to_string(),
        | _ => String::new(),
    };
    let mut view = PreprocessedView {
        links: vec![None],
        ..PreprocessedView::default()
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let origin = origins.get(index).copied().unwrap_or_default();
        let Some(include) = origin.include.filter(|_| fold) else {
            view.text.push_str(lines[index]);
            view.text.push('\n');
            view.links.push(link(&origin));
            view.labels.push(label(&origin));
            index += 1;
            continue;
        };
        // One `#include`'s lines, folded.
        let end = (index..lines.len())
            .find(|&at| origins.get(at).and_then(|origin| origin.include) != Some(include))
            .unwrap_or(lines.len());
        let held = (index..end)
            .filter(|&at| origins[at].line.is_some() && !lines[at].trim().is_empty())
            .count();
        let header = includes.iter().find(|candidate| candidate.line == include);
        view.text.push_str(&summary(header, held));
        view.text.push('\n');
        view.links.push(Some((include, include)));
        view.labels.push(include.to_string());
        index = end;
    }
    view
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Text as bcc writes it, and where each line came from.
    const TEXT: &str = "# 1 \"<input>\"\nint a;\n# 1 \"/bcc/stdint.h\" 3\ntypedef long \
                        int64_t;\n\ntypedef int int32_t;\n# 3 \"<input>\"\nint b;\n";

    fn origins() -> Vec<OutputLine> {
        let input = |line| OutputLine {
            input: true,
            line,
            include: None,
        };
        let header = |line| OutputLine {
            input: false,
            line,
            include: Some(2),
        };
        [
            input(None),
            input(Some(1)),
            header(None),
            header(Some(10)),
            header(Some(11)),
            header(Some(12)),
            input(None),
            input(Some(3)),
        ]
        .into()
    }

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
        let folded = preprocessed_view(TEXT, &origins(), &[include()], true).text;
        assert!(is_folded(folded.lines().nth(2).unwrap()));
        assert_eq!(
            folded,
            "# 1 \"<input>\"\nint a;\n/* <stdint.h>: 2 lines of declarations, folded (Show \
             headers to read them) */\n# 3 \"<input>\"\nint b;\n"
        );
        // A header that leaves only a line marker held only macros.
        let macros = preprocessed_view(
            "# 1 \"<input>\"\n# 1 \"/l.h\" 3\n# 2 \"<input>\"\nx\n",
            &[
                OutputLine {
                    input: true,
                    ..OutputLine::default()
                },
                OutputLine {
                    include: Some(1),
                    ..OutputLine::default()
                },
                OutputLine {
                    input: true,
                    ..OutputLine::default()
                },
                OutputLine {
                    input:   true,
                    line:    Some(2),
                    include: None,
                },
            ],
            &[include()],
            true,
        )
        .text;
        assert!(macros.contains("only macro definitions"), "{macros}");
    }

    #[test]
    fn preprocessed_lines_link_to_their_source_line_or_include() {
        let full = preprocessed_view(TEXT, &origins(), &[include()], false);
        assert_eq!(full.text, TEXT);
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
                None,
                Some((3, 3))
            ]
        );
        let folded = preprocessed_view(TEXT, &origins(), &[include()], true);
        assert_eq!(
            folded.links,
            [None, None, Some((1, 1)), Some((2, 2)), None, Some((3, 3))]
        );
        assert_eq!(folded.labels, ["", "1", "2", "", "3"]);
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
