//! Engine observations in the shape the page presents, as posted by the engine
//! Worker (`machine.rs` builds them from the engine's states). No C semantics
//! or computed program values live in the page.

use serde::{
    Deserialize,
    Serialize,
};

/// One complete engine observation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub step:         usize,
    pub status:       String,
    /// The next operation, or the faulting one.
    pub location:     Option<Range>,
    /// What the last completed operation did.
    pub event:        StepEvent,
    /// The full expression being evaluated, with the values known so far.
    pub tree:         Option<Tree>,
    /// Active calls, innermost first.
    pub stack:        Vec<Frame>,
    pub objects:      Vec<Object>,
    pub effects:      Vec<String>,
    pub diagnostic:   Option<Diagnostic>,
    pub return_value: Option<Value>,
}

impl Snapshot {
    /// Whether the engine can evaluate another operation from this state.
    pub fn can_step(&self) -> bool {
        matches!(self.status.as_str(), "ready" | "running")
    }
}

/// The last completed operation, explained.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    /// The engine's rule name, such as `lvalue_conversion`.
    pub kind:           String,
    /// `evaluate`, `convert`, `memory`, `lifetime`, `control`, `call` or
    /// `fault`; it selects the colour.
    pub category:       String,
    pub title:          String,
    /// One plain sentence: what happened and why. The card leads with it;
    /// the explanation is the detail behind it.
    pub summary:        String,
    pub explanation:    String,
    /// Why the rule matters, given the first time a run needs it; empty
    /// otherwise.
    pub note:           String,
    /// What went in and what came out, as chips the page lines up with an
    /// arrow: `values[1]` → `4`.
    pub flow:           Vec<FlowPart>,
    /// How the operation used each object, for the memory diagram.
    pub access:         Vec<Access>,
    pub clauses:        Vec<Clause>,
    /// The exact source the operation evaluated.
    pub range:          Option<Range>,
    /// Objects the operation created, read, wrote or ended.
    pub objects:        Vec<usize>,
    pub sequence_point: bool,
}

/// One chip of a step's value flow.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct FlowPart {
    /// `object` (a designated object), `value`, `operator`, or `arrow` (with
    /// the verb that connects its neighbours as `text`).
    pub role:   String,
    pub text:   String,
    /// The C type, where the step makes it matter (`char` → `int`).
    pub ty:     Option<String>,
    /// The variable whose colour the chip takes.
    pub object: Option<String>,
}

/// How an operation used one object, or one element of an array.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Access {
    pub object: usize,
    /// The element of an array, or 0 for another object; `None` when the
    /// operation used the whole object.
    pub cell:   Option<usize>,
    /// `read`, `write`, `create`, `end`, `designate`, or `address` (`&` or
    /// an array's decay took its address).
    pub kind:   String,
}

/// A cited paragraph of the C standard.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Clause {
    /// Clause and paragraph, such as `6.5.16.1p2`.
    pub id:       String,
    pub title:    String,
    pub gloss:    String,
    pub citation: String,
}

/// A full expression's tree. Nodes are listed children first; the root is
/// last.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Tree {
    pub function: String,
    pub nodes:    Vec<TreeNode>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TreeNode {
    pub label:    String,
    /// Indices into [`Tree::nodes`].
    pub children: Vec<usize>,
    /// The value or designated object, once evaluated.
    pub value:    Option<String>,
    /// Whether `value` names a designated object (an lvalue).
    pub lvalue:   bool,
    /// The last completed operation evaluated this node.
    pub active:   bool,
    /// The object an identifier names, for colour coding.
    pub object:   Option<String>,
    pub range:    Option<Range>,
}

/// A source range in 1-based lines and UTF-16 editor columns.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Range {
    pub start: Position,
    pub end:   Position,
}

impl Range {
    /// Whether this range covers `line`. A range ending at column 1 of a later
    /// line does not include that line.
    pub fn contains_line(&self, line: usize) -> bool {
        line >= self.start.line
            && line <= self.end.line
            && !(line == self.end.line && self.end.column == 1 && line != self.start.line)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Position {
    pub line:   usize,
    pub column: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Frame {
    pub id:      usize,
    pub depth:   usize,
    pub name:    String,
    /// The source of the call this frame is waiting on.
    pub waiting: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Object {
    pub id:        usize,
    pub name:      String,
    #[serde(rename = "type")]
    pub ty:        String,
    pub storage:   String,
    pub alive:     bool,
    pub frame_id:  usize,
    /// The owning function, or empty once its frame has returned.
    pub function:  String,
    pub parameter: bool,
    pub value:     Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Value {
    Indeterminate,
    Integer {
        text: String,
    },
    Array {
        elements: Vec<Value>,
    },
    #[serde(rename_all = "camelCase")]
    Pointer {
        target:   Option<usize>,
        offset:   i32,
        one_past: bool,
        ended:    bool,
    },
}

/// A terminal diagnostic, or a site-level error shown in the same place.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Diagnostic {
    pub kind:    Option<String>,
    pub title:   String,
    pub message: String,
    pub clause:  Option<Clause>,
    /// A note when no clause applies (a prototype limit, a site error).
    pub note:    String,
}

impl Clause {
    /// The paragraph's page in N1256, the C99 draft with its corrigenda
    /// that every citation was checked against, as a link that opens the
    /// PDF there. The citation names it as `PDF p. 58` or `PDF pp. 54-55`.
    pub fn link(&self) -> Option<String> {
        let (_, pages) = self.citation.split_once("PDF p")?;
        let page: String = pages
            .trim_start_matches('p')
            .trim_start_matches(['.', ' '])
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        (!page.is_empty()).then(|| format!("{N1256_URL}#page={page}"))
    }
}

/// The C99 standard with its three technical corrigenda, as published by
/// the committee.
pub const N1256_URL: &str = "https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1256.pdf";

impl Diagnostic {
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_deref() {
            | Some("undefined_behavior") => "Undefined behavior",
            | Some("indeterminate") => "Indeterminate value",
            | Some("unsupported") => "Outside the supported subset",
            | Some("syntax") => "Compilation diagnostic",
            | Some("limit") => "Resource limit",
            | _ => "Program diagnostic",
        }
    }
}

pub fn status_label(status: &str) -> &str {
    match status {
        | "ready" => "Ready",
        | "running" => "Paused",
        | "finished" => "Finished",
        | "undefined_behavior" => "Undefined behavior",
        | "error" => "Stopped",
        | "limit" => "Resource limit",
        | other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::Clause;

    #[test]
    fn clauses_link_to_their_page_of_the_standard() {
        let clause = |citation: &str| Clause {
            citation: citation.to_owned(),
            ..Clause::default()
        };
        assert_eq!(
            clause("C99 §6.3.2.1p2, p. 46; PDF p. 58").link().as_deref(),
            Some("https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1256.pdf#page=58")
        );
        assert!(
            clause("C99 §6.3.1.1p2, pp. 42-43; PDF pp. 54-55")
                .link()
                .is_some_and(|link| link.ends_with("#page=54"))
        );
        assert_eq!(clause("no pages").link(), None);
    }
}
