//! Dependency-free, escaped JSON encoding for browser snapshots and recorded
//! traces.

use std::fmt::Write;

use super::{
    machine::Machine,
    model::{
        Diagnostic,
        Object,
        Program,
        SourceRange,
        State,
        Value,
    },
};

impl State {
    /// Encodes this observation as schema version 1 JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        state(&mut output, self);
        output
    }
}

impl Diagnostic {
    /// Encodes a compilation or execution diagnostic as JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        diagnostic(&mut output, self);
        output
    }
}

pub(super) fn trace(program: &Program, history: &[Machine]) -> String {
    let mut output = String::from("{\"schema_version\":1,\"source\":");
    string(&mut output, &program.source);
    output.push_str(",\"states\":[");
    for (index, machine) in history.iter().enumerate() {
        comma(&mut output, index);
        state(&mut output, &machine.state);
    }
    output.push_str("]}");
    output
}

fn state(output: &mut String, state: &State) {
    write!(
        output,
        "{{\"schema_version\":1,\"engine\":\"bounded-c-subset\",\"step\":{},\"status\":",
        state.step
    )
    .expect("String write");
    string(output, state.status);
    output.push_str(",\"source_range\":");
    range(output, state.source_range);
    output.push_str(",\"event\":");
    string(output, &state.event);
    output.push_str(",\"frames\":[");
    for (index, frame) in state.frames.iter().enumerate() {
        comma(output, index);
        write!(
            output,
            "{{\"id\":{},\"depth\":{},\"function\":",
            frame.id, frame.depth
        )
        .expect("String write");
        string(output, &frame.function);
        output.push('}');
    }
    output.push_str("],\"objects\":[");
    for (index, object) in state.objects.iter().enumerate() {
        comma(output, index);
        object_json(output, object);
    }
    output.push_str("],\"pending_effects\":[");
    for (index, effect) in state.pending_effects.iter().enumerate() {
        comma(output, index);
        string(output, effect);
    }
    output.push_str("],\"diagnostics\":[");
    for (index, entry) in state.diagnostics.iter().enumerate() {
        comma(output, index);
        diagnostic(output, entry);
    }
    output.push_str("],\"return_value\":");
    if let Some(result) = &state.return_value {
        value(output, result);
    } else {
        output.push_str("null");
    }
    output.push('}');
}

fn object_json(output: &mut String, object: &Object) {
    write!(
        output,
        "{{\"id\":{},\"frame_id\":{},\"name\":",
        object.id, object.frame_id
    )
    .expect("String write");
    string(output, &object.name);
    output.push_str(",\"type\":");
    string(output, &object.r#type);
    output.push_str(",\"storage_duration\":");
    string(output, object.storage_duration);
    output.push_str(",\"lifetime\":");
    string(output, object.lifetime);
    output.push_str(",\"value\":");
    value(output, &object.value);
    output.push('}');
}

fn value(output: &mut String, input: &Value) {
    match input {
        | Value::Indeterminate => output.push_str("{\"kind\":\"indeterminate\"}"),
        | Value::Integer(integer) =>
            write!(output, "{{\"kind\":\"integer\",\"value\":{integer}}}").expect("String write"),
        | Value::Pointer { target, offset } => {
            output.push_str("{\"kind\":\"pointer\",\"target\":");
            if let Some(target) = target {
                write!(output, "{target}").expect("String write");
            } else {
                output.push_str("null");
            }
            write!(output, ",\"offset\":{offset}}}").expect("String write");
        },
        | Value::Array(elements) => {
            output.push_str("{\"kind\":\"array\",\"elements\":[");
            for (index, element) in elements.iter().enumerate() {
                comma(output, index);
                value(output, element);
            }
            output.push_str("]}");
        },
    }
}

fn diagnostic(output: &mut String, diagnostic: &Diagnostic) {
    output.push_str("{\"kind\":");
    string(output, diagnostic.kind);
    output.push_str(",\"message\":");
    string(output, &diagnostic.message);
    output.push_str(",\"source_range\":");
    range(output, diagnostic.source_range);
    output.push_str(",\"citation\":");
    if let Some(citation) = diagnostic.citation {
        string(output, citation);
    } else {
        output.push_str("null");
    }
    output.push('}');
}

fn range(output: &mut String, range: SourceRange) {
    write!(
        output,
        "{{\"start\":{},\"end\":{},\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{}}}",
        range.start, range.end, range.line, range.column, range.end_line, range.end_column
    )
    .expect("String write");
}

fn comma(output: &mut String, index: usize) {
    if index > 0 {
        output.push(',');
    }
}

fn string(output: &mut String, input: &str) {
    output.push('"');
    for character in input.chars() {
        match character {
            | '"' => output.push_str("\\\""),
            | '\\' => output.push_str("\\\\"),
            | '\n' => output.push_str("\\n"),
            | '\r' => output.push_str("\\r"),
            | '\t' => output.push_str("\\t"),
            | '\u{0}'..='\u{1F}' =>
                write!(output, "\\u{:04x}", u32::from(character)).expect("String write"),
            | character => output.push(character),
        }
    }
    output.push('"');
}
