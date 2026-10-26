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
    pub step:                 usize,
    pub status:               String,
    pub location:             Option<Range>,
    #[serde(default)]
    pub event:                String,
    pub stack:                Vec<Frame>,
    pub objects:              Vec<Object>,
    pub effects:              Vec<String>,
    #[serde(default)]
    pub sequence_explanation: String,
    pub diagnostic:           Option<Diagnostic>,
    pub return_value:         Option<Value>,
}

impl Snapshot {
    /// Whether the engine can evaluate another operation from this state.
    pub fn can_step(&self) -> bool {
        matches!(self.status.as_str(), "ready" | "running")
    }
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
    pub depth: usize,
    pub name:  String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Object {
    pub id:      usize,
    pub name:    String,
    #[serde(rename = "type", default)]
    pub ty:      String,
    #[serde(default)]
    pub storage: String,
    pub alive:   bool,
    #[serde(default)]
    pub scope:   String,
    pub value:   Value,
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
        #[serde(default)]
        one_past: bool,
        #[serde(default)]
        ended:    bool,
        text:     Option<String>,
    },
}

/// A terminal diagnostic, or a site-level error shown in the same place.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Diagnostic {
    pub kind:     Option<String>,
    #[serde(default)]
    pub title:    String,
    #[serde(default)]
    pub message:  String,
    #[serde(default)]
    pub citation: String,
    #[serde(default)]
    pub detail:   String,
}

impl Diagnostic {
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_deref() {
            | Some("undefined_behavior") => "UNDEFINED BEHAVIOR · EXECUTION STOPPED",
            | Some("indeterminate") => "INDETERMINATE VALUE · POLICY STOP",
            | Some("unsupported") => "OUTSIDE THE SUPPORTED SUBSET",
            | Some("syntax") => "COMPILATION DIAGNOSTIC",
            | Some("limit") => "RESOURCE LIMIT · EXECUTION STOPPED",
            | _ => "PROGRAM DIAGNOSTIC",
        }
    }
}

pub fn status_label(status: &str) -> String {
    match status {
        | "ready" => "Ready",
        | "running" => "Paused",
        | "finished" => "Finished",
        | "undefined_behavior" => "Undefined behavior",
        | "error" => "Stopped",
        | "limit" => "Resource limit",
        | other => other,
    }
    .to_owned()
}
