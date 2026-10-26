//! Dependency-free, escaped JSON encoding for browser snapshots and recorded
//! traces.

use std::fmt::Write;

use super::{
    clauses::Clause,
    machine::Machine,
    model::{
        Diagnostic,
        Event,
        Object,
        Operand,
        Program,
        SourceRange,
        State,
        Value,
    },
};

impl State {
    /// Encodes this observation as schema version 2 JSON.
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
    let mut output = String::from("{\"schema_version\":2,\"source\":");
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
        "{{\"schema_version\":2,\"engine\":\"bounded-c-subset\",\"step\":{},\"status\":",
        state.step
    )
    .expect("String write");
    string(output, state.status);
    output.push_str(",\"source_range\":");
    range(output, state.source_range);
    output.push_str(",\"event\":");
    event(output, &state.event);
    output.push_str(",\"evaluation\":");
    if let Some(evaluation) = &state.evaluation {
        write!(
            output,
            "{{\"function\":{},\"root\":{},\"values\":[",
            evaluation.function, evaluation.root
        )
        .expect("String write");
        for (index, (node, entry)) in evaluation.values.iter().enumerate() {
            comma(output, index);
            write!(output, "{{\"node\":{node},\"value\":").expect("String write");
            operand(output, entry);
            output.push('}');
        }
        output.push_str("],\"last\":");
        optional(output, evaluation.last);
        output.push_str(",\"calling\":");
        optional(output, evaluation.calling);
        output.push('}');
    } else {
        output.push_str("null");
    }
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
        output.push_str(",\"waiting\":");
        if let Some(waiting) = frame.waiting {
            range(output, waiting);
        } else {
            output.push_str("null");
        }
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

fn event(output: &mut String, event: &Event) {
    output.push_str("{\"kind\":");
    string(output, event.kind.name());
    output.push_str(",\"category\":");
    string(output, event.kind.category());
    output.push_str(",\"range\":");
    range(output, event.range);
    output.push_str(",\"title\":");
    string(output, &event.title);
    output.push_str(",\"explanation\":");
    string(output, &event.explanation);
    output.push_str(",\"clauses\":[");
    for (index, entry) in event.clauses.iter().enumerate() {
        comma(output, index);
        clause(output, entry);
    }
    output.push_str("],\"operands\":[");
    for (index, entry) in event.operands.iter().enumerate() {
        comma(output, index);
        operand(output, entry);
    }
    output.push_str("],\"result\":");
    if let Some(result) = &event.result {
        operand(output, result);
    } else {
        output.push_str("null");
    }
    output.push_str(",\"objects\":[");
    for (index, id) in event.objects.iter().enumerate() {
        comma(output, index);
        write!(output, "{id}").expect("String write");
    }
    write!(output, "],\"sequence_point\":{}}}", event.sequence_point).expect("String write");
}

fn clause(output: &mut String, clause: &Clause) {
    output.push_str("{\"id\":");
    string(output, clause.id);
    output.push_str(",\"title\":");
    string(output, clause.title);
    output.push_str(",\"gloss\":");
    string(output, clause.gloss);
    output.push_str(",\"citation\":");
    string(output, clause.citation);
    output.push('}');
}

fn operand(output: &mut String, operand: &Operand) {
    match operand {
        | Operand::Value(entry) => value(output, entry),
        | Operand::Address { target, offset } => write!(
            output,
            "{{\"kind\":\"lvalue\",\"target\":{target},\"offset\":{offset}}}"
        )
        .expect("String write"),
    }
}

fn optional(output: &mut String, entry: Option<usize>) {
    if let Some(entry) = entry {
        write!(output, "{entry}").expect("String write");
    } else {
        output.push_str("null");
    }
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
    write!(output, ",\"parameter\":{}", object.parameter).expect("String write");
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
