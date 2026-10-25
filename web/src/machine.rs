//! The engine Worker's side: run the bcc abstract machine and present each of
//! its states as a [`Snapshot`]. Only the `worker` build links the engine.
//!
//! The engine reports byte offsets into UTF-8 source; the page's editor and
//! line view count UTF-16 columns, so ranges are converted here.

use std::collections::HashMap;

use bcc_rust::abstract_machine::{
    self,
    Session,
    SourceRange,
    State,
};

use crate::{
    protocol::Command,
    snapshot::{
        Diagnostic,
        Frame,
        Object,
        Position,
        Range,
        Snapshot,
        Value,
    },
};

/// One engine session over the most recently loaded source.
#[derive(Default)]
pub struct Machine {
    session: Option<Session>,
    source:  String,
}

impl Machine {
    /// Run one request from the page.
    ///
    /// # Errors
    /// Stepping before any program has loaded. Rejected source and C faults
    /// are snapshots, not errors.
    pub fn run(&mut self, command: Command) -> Result<Snapshot, String> {
        match command {
            | Command::Load(source) => Ok(self.load(source)),
            | Command::Step => self.step(),
        }
    }

    /// Compile `source`, replacing any previous session. Rejected source is a
    /// snapshot with an error status, not an `Err`.
    fn load(&mut self, source: String) -> Snapshot {
        self.session = None;
        let state = match abstract_machine::compile(&source) {
            | Ok(program) => {
                let session = program.session();
                let state = session.state().clone();
                self.session = Some(session);
                state
            },
            | Err(diagnostic) => State {
                step:            0,
                status:          "error",
                source_range:    diagnostic.source_range,
                event:           String::new(),
                frames:          Vec::new(),
                objects:         Vec::new(),
                pending_effects: Vec::new(),
                diagnostics:     vec![diagnostic],
                return_value:    None,
            },
        };
        self.source = source;
        snapshot(&state, &self.source)
    }

    /// Execute one operation. A terminal state is returned unchanged.
    fn step(&mut self) -> Result<Snapshot, String> {
        let session = self
            .session
            .as_mut()
            .ok_or("Load a program before stepping.")?;
        Ok(snapshot(session.step(), &self.source))
    }
}

const SEQUENCING: &str = "Stores commit atomically. Pending effects describe the next store; this \
                          prototype does not model deferred or unsequenced writes. Operand order \
                          is left to right.";

fn snapshot(state: &State, source: &str) -> Snapshot {
    let frames: HashMap<usize, &str> = state
        .frames
        .iter()
        .map(|frame| (frame.id, frame.function.as_str()))
        .collect();
    let objects: HashMap<usize, &abstract_machine::Object> = state
        .objects
        .iter()
        .map(|object| (object.id, object))
        .collect();
    let fault = state.diagnostics.first();
    let event = if state.event.is_empty() && state.status == "ready" {
        "Program loaded. No operations have executed.".to_owned()
    } else {
        state.event.clone()
    };
    Snapshot {
        step: state.step,
        status: state.status.to_owned(),
        location: range(
            fault.map_or(state.source_range, |fault| fault.source_range),
            source,
        ),
        event,
        stack: state
            .frames
            .iter()
            .rev()
            .map(|frame| Frame {
                depth: frame.depth,
                name:  frame.function.clone(),
                label: format!("activation #{}", frame.id),
            })
            .collect(),
        objects: state
            .objects
            .iter()
            .map(|object| Object {
                id:      object.id,
                name:    object.name.clone(),
                ty:      object.r#type.clone(),
                storage: object.storage_duration.to_owned(),
                alive:   object.lifetime == "alive",
                scope:   format!(
                    "{} · frame #{}",
                    frames
                        .get(&object.frame_id)
                        .copied()
                        .unwrap_or("ended activation"),
                    object.frame_id
                ),
                value:   value(&object.value, &objects),
            })
            .collect(),
        effects: state.pending_effects.clone(),
        sequence_explanation: SEQUENCING.to_owned(),
        diagnostic: fault.map(|fault| diagnostic(fault, &state.diagnostics[1..])),
        return_value: state
            .return_value
            .as_ref()
            .map(|returned| value(returned, &objects)),
    }
}

fn diagnostic(
    fault: &abstract_machine::Diagnostic,
    rest: &[abstract_machine::Diagnostic],
) -> Diagnostic {
    let title = match fault.kind {
        | "undefined_behavior" => "Undefined behavior detected",
        | "indeterminate" => "Indeterminate value read",
        | "unsupported" => "Outside the supported C subset",
        | "syntax" => "Source could not be compiled",
        | "limit" => "Engine resource limit reached",
        | _ => "Execution stopped",
    };
    let citation = fault.citation.unwrap_or(if fault.kind == "limit" {
        "An engine resource limit is a prototype restriction, not undefined behavior."
    } else {
        "This diagnostic describes a prototype restriction; it does not establish undefined \
         behavior."
    });
    Diagnostic {
        kind:     Some(fault.kind.to_owned()),
        title:    title.to_owned(),
        message:  fault.message.clone(),
        citation: citation.to_owned(),
        detail:   rest
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn value(
    value: &abstract_machine::Value,
    objects: &HashMap<usize, &abstract_machine::Object>,
) -> Value {
    match value {
        | abstract_machine::Value::Indeterminate => Value::Indeterminate,
        | abstract_machine::Value::Integer(number) => Value::Integer {
            text: number.to_string(),
        },
        | abstract_machine::Value::Array(elements) => Value::Array {
            elements: elements
                .iter()
                .map(|element| self::value(element, objects))
                .collect(),
        },
        | &abstract_machine::Value::Pointer { target, offset } => {
            let object = target.and_then(|target| objects.get(&target));
            let length = object.map(|object| match &object.value {
                | abstract_machine::Value::Array(elements) => elements.len(),
                | _ => 1,
            });
            Value::Pointer {
                target,
                offset,
                one_past: length.is_some_and(|length| usize::try_from(offset) == Ok(length)),
                ended: object.is_some_and(|object| object.lifetime == "ended"),
                text: target.is_none().then(|| "null pointer".to_owned()),
            }
        },
    }
}

/// The engine's byte range as lines and UTF-16 columns; `None` when empty.
fn range(range: SourceRange, source: &str) -> Option<Range> {
    (range.end > range.start).then(|| Range {
        start: position(source, range.start),
        end:   position(source, range.end),
    })
}

fn position(source: &str, offset: usize) -> Position {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
    let line_start = prefix.rfind('\n').map_or(0, |newline| newline + 1);
    Position {
        line:   prefix.matches('\n').count() + 1,
        column: prefix[line_start..].encode_utf16().count() + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demos;

    /// Load `source` and step until the engine stops; every snapshot in order.
    fn execute(machine: &mut Machine, source: &str) -> Vec<Snapshot> {
        let mut states = vec![machine.load(source.to_owned())];
        while states.last().is_some_and(Snapshot::can_step) {
            assert!(
                states.len() < 2200,
                "the engine terminates or reports its step limit"
            );
            states.push(machine.step().expect("a loaded program steps"));
        }
        states
    }

    fn integer(value: Option<&Value>) -> Option<&str> {
        match value {
            | Some(Value::Integer { text }) => Some(text),
            | _ => None,
        }
    }

    #[test]
    fn runs_the_tour_through_calls_pointers_and_lifetimes() {
        let mut machine = Machine::default();
        let states = execute(&mut machine, demos::find("tour").unwrap().source);
        let finished = states.last().unwrap();
        assert_eq!(finished.status, "finished", "{:?}", finished.diagnostic);
        assert_eq!(integer(finished.return_value.as_ref()), Some("15"));
        assert!(
            states.iter().any(|state| state.stack.len() == 2),
            "bump is called"
        );
        let any_value = |test: fn(&Value) -> bool| {
            states
                .iter()
                .any(|state| state.objects.iter().any(|object| test(&object.value)))
        };
        assert!(any_value(|value| matches!(value, Value::Indeterminate)));
        assert!(any_value(|value| matches!(
            value,
            Value::Pointer {
                target: Some(_),
                ..
            }
        )));
        assert!(states.iter().any(|state| !state.effects.is_empty()));
        assert!(finished.objects.iter().any(|object| {
            match &object.value {
                | Value::Array { elements } =>
                    elements
                        .iter()
                        .map(|element| integer(Some(element)))
                        .collect::<Vec<_>>()
                        == [Some("3"), Some("5"), Some("7")],
                | _ => false,
            }
        }));
        assert!(finished.objects.iter().any(|object| !object.alive));
        assert_eq!(
            machine.step(),
            Ok(finished.clone()),
            "a terminal state stays put"
        );

        let changed = demos::find("tour")
            .unwrap()
            .source
            .replace("{2, 4, 6}", "{4, 6, 8}");
        let states = execute(&mut machine, &changed);
        assert_eq!(
            integer(states.last().unwrap().return_value.as_ref()),
            Some("21")
        );
    }

    #[test]
    fn stops_the_diagnostic_demos_with_citations() {
        let mut machine = Machine::default();
        for (key, status, kind) in [
            ("indeterminate", "error", "indeterminate"),
            ("bounds", "undefined_behavior", "undefined_behavior"),
        ] {
            let states = execute(&mut machine, demos::find(key).unwrap().source);
            let finished = states.last().unwrap();
            assert_eq!(finished.status, status, "{key}");
            let diagnostic = finished.diagnostic.as_ref().unwrap();
            assert_eq!(diagnostic.kind.as_deref(), Some(kind));
            assert!(
                diagnostic.citation.contains("6."),
                "{key} cites the standard"
            );
            assert!(finished.location.is_some(), "{key} locates the fault");
        }
    }

    #[test]
    fn rejects_unsupported_source_and_recovers() {
        let mut machine = Machine::default();
        assert_eq!(
            machine.step(),
            Err("Load a program before stepping.".into())
        );
        for source in [
            "#include <stdio.h>\nint main(void) { return 0; }\n",
            "int main(void) { int a[2] = {3, 7}; int *p = a; *p++; return a[0]; }",
            "int main(void) { return later(); } int later(void) { return 7; }",
        ] {
            let rejected = machine.load(source.to_owned());
            assert_eq!(rejected.status, "error", "{source}");
            assert_ne!(rejected.diagnostic.unwrap().message, "");
            assert_eq!(
                machine.step(),
                Err("Load a program before stepping.".into())
            );
        }
        let states = execute(&mut machine, "int main(void) { return 42; }\n");
        assert_eq!(
            integer(states.last().unwrap().return_value.as_ref()),
            Some("42")
        );
        let limited = execute(&mut machine, "int main(void) { while (1) {} }\n");
        let limited = limited.last().unwrap();
        assert_eq!(limited.status, "limit");
        assert_eq!(
            limited.diagnostic.as_ref().unwrap().kind.as_deref(),
            Some("limit")
        );
        let oversized = machine.load(" ".repeat(32 * 1024 + 1));
        assert_eq!(oversized.status, "error");
    }

    #[test]
    fn maps_byte_ranges_to_utf16_columns() {
        let source = "/* π 🧭 */ int main(void) { return 42; }\n";
        let state = Machine::default().load(source.to_owned());
        let brace = source[..source.find('{').unwrap()].encode_utf16().count();
        assert_eq!(
            state.location.unwrap().start,
            Position {
                line:   1,
                column: brace + 1,
            }
        );
        assert_eq!(
            position("a\nπb", 5),
            Position {
                line:   2,
                column: 3,
            }
        );
        assert_eq!(
            position("π", 1),
            Position {
                line:   1,
                column: 1,
            },
            "inside a character"
        );
    }
}
