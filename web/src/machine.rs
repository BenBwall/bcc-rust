//! The engine Worker's side: run the bcc abstract machine and present each of
//! its states as a [`Snapshot`]. Only the `worker` build links the engine.
//!
//! The engine reports byte offsets into UTF-8 source; the page's editor counts
//! UTF-16 columns, so ranges are converted here. Values are rendered as the
//! short text the page shows (`5`, `&values[1]`), and the current full
//! expression's tree is resolved from the program.

use std::collections::HashMap;

use bcc_rust::abstract_machine::{
    self,
    Operand,
    Program,
    Session,
    SourceRange,
    State,
};

use crate::{
    protocol::{
        Answer,
        Command,
    },
    snapshot::{
        Clause,
        Diagnostic,
        Frame,
        Object,
        Position,
        Range,
        Snapshot,
        StepEvent,
        Tree,
        TreeNode,
        Value,
    },
};

/// One engine session over the most recently loaded source, and the
/// WebAssembly stepper's session over the most recently loaded module.
#[derive(Default)]
pub struct Machine {
    session: Option<Session>,
    source:  String,
    wasm:    crate::wasm::worker::WasmWorker,
}

impl Machine {
    /// Run one request from the page.
    ///
    /// # Errors
    /// Stepping before any program has loaded, or compiling, which is the
    /// compiler Worker's job. Rejected source and C faults are snapshots, not
    /// errors.
    pub fn run(&mut self, command: Command) -> Result<Answer, String> {
        match command {
            | Command::Load(source) => Ok(Answer::Snapshot(Box::new(self.load(source)))),
            | Command::Step => self
                .step()
                .map(|snapshot| Answer::Snapshot(Box::new(snapshot))),
            | Command::WasmLoad(module) => self
                .wasm
                .load(module)
                .map(|program| Answer::WasmLoaded(Box::new(program))),
            | Command::Wasm(control) => self
                .wasm
                .control(control)
                .map(|snapshot| Answer::Wasm(Box::new(snapshot))),
            | Command::Compile { .. } | Command::CompileWasm(_) => Err("The abstract machine's \
                                                                        Worker does not compile; \
                                                                        the compiler Worker does."
                .into()),
        }
    }

    /// Compile `source`, replacing any previous session. Rejected source is a
    /// snapshot with an error status, not an `Err`.
    fn load(&mut self, source: String) -> Snapshot {
        self.session = None;
        self.source = source;
        match abstract_machine::compile(&self.source) {
            | Ok(program) => {
                let session = program.session();
                let snapshot = snapshot(session.state(), session.program(), &self.source);
                self.session = Some(session);
                snapshot
            },
            | Err(diagnostic) => rejected(&diagnostic, &self.source),
        }
    }

    /// Execute one operation. A terminal state is returned unchanged.
    fn step(&mut self) -> Result<Snapshot, String> {
        let session = self
            .session
            .as_mut()
            .ok_or("Load a program before stepping.")?;
        let state = session.step().clone();
        Ok(snapshot(&state, session.program(), &self.source))
    }
}

/// The snapshot for source the engine could not compile.
fn rejected(diagnostic: &abstract_machine::Diagnostic, source: &str) -> Snapshot {
    let location = range(diagnostic.source_range, source);
    Snapshot {
        step: 0,
        status: "error".to_owned(),
        location,
        event: StepEvent {
            kind:           "fault".to_owned(),
            category:       "fault".to_owned(),
            title:          "This program is outside the supported subset".to_owned(),
            explanation:    format!(
                "The abstract machine could not compile it: {}. The compiler pipeline view may \
                 still show what bcc itself makes of it.",
                diagnostic.message
            ),
            clauses:        clause(diagnostic.clause).into_iter().collect(),
            range:          location,
            objects:        Vec::new(),
            sequence_point: false,
        },
        tree: None,
        stack: Vec::new(),
        objects: Vec::new(),
        effects: Vec::new(),
        diagnostic: Some(self::diagnostic(diagnostic)),
        return_value: None,
    }
}

fn snapshot(state: &State, program: &Program, source: &str) -> Snapshot {
    let functions: HashMap<usize, &str> = state
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
    let event = &state.event;
    Snapshot {
        step:         state.step,
        status:       state.status.to_owned(),
        location:     range(
            fault.map_or(state.source_range, |fault| fault.source_range),
            source,
        ),
        event:        StepEvent {
            kind:           event.kind.name().to_owned(),
            category:       event.kind.category().to_owned(),
            title:          event.title.clone(),
            explanation:    event.explanation.clone(),
            clauses:        event
                .clauses
                .iter()
                .map(|entry| clause(Some(entry)).unwrap_or_default())
                .collect(),
            range:          range(event.range, source),
            objects:        event.objects.clone(),
            sequence_point: event.sequence_point,
        },
        tree:         state
            .evaluation
            .as_ref()
            .map(|evaluation| tree(evaluation, program, source, &objects)),
        stack:        state
            .frames
            .iter()
            .rev()
            .map(|frame| Frame {
                id:      frame.id,
                depth:   frame.depth,
                name:    frame.function.clone(),
                waiting: frame
                    .waiting
                    .and_then(|range| source.get(range.start..range.end))
                    .map(str::to_owned),
            })
            .collect(),
        objects:      state
            .objects
            .iter()
            .map(|object| Object {
                id:        object.id,
                name:      object.name.clone(),
                ty:        object.r#type.clone(),
                storage:   object.storage_duration.to_owned(),
                alive:     object.lifetime == "alive",
                frame_id:  object.frame_id,
                function:  functions
                    .get(&object.frame_id)
                    .map(|name| (*name).to_owned())
                    .unwrap_or_default(),
                parameter: object.parameter,
                value:     value(&object.value, &objects),
            })
            .collect(),
        effects:      state.pending_effects.clone(),
        diagnostic:   fault.map(diagnostic),
        return_value: state
            .return_value
            .as_ref()
            .map(|returned| value(returned, &objects)),
    }
}

/// The current full expression's nodes, re-indexed from 0, with the values
/// the engine has recorded for them.
fn tree(
    evaluation: &abstract_machine::Evaluation,
    program: &Program,
    source: &str,
    objects: &HashMap<usize, &abstract_machine::Object>,
) -> Tree {
    let nodes = program.expression(evaluation.function, evaluation.root);
    let first = nodes.first().map_or(0, |node| node.first);
    let values: HashMap<usize, &Operand> = evaluation
        .values
        .iter()
        .map(|(node, operand)| (*node, operand))
        .collect();
    Tree {
        function: program
            .function_name(evaluation.function)
            .unwrap_or_default()
            .to_owned(),
        nodes:    nodes
            .iter()
            .enumerate()
            .map(|(offset, node)| {
                let index = first + offset;
                let operand = values.get(&index);
                let is_name = node.children.is_empty()
                    && node
                        .label
                        .bytes()
                        .next()
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
                TreeNode {
                    label:    node.label.clone(),
                    children: node.children.iter().map(|child| child - first).collect(),
                    value:    operand.map(|operand| operand_text(operand, objects)),
                    lvalue:   matches!(operand, Some(Operand::Address { .. })),
                    active:   evaluation.last == Some(index),
                    object:   is_name
                        .then(|| node.label.split('[').next().unwrap_or_default().to_owned()),
                    range:    range(node.range, source),
                }
            })
            .collect(),
    }
}

/// How the page shows an operand: `5`, `&values[1]`, or the designated
/// object `values[1]` for an lvalue.
fn operand_text(operand: &Operand, objects: &HashMap<usize, &abstract_machine::Object>) -> String {
    match operand {
        | Operand::Value(value) => value_text(value, objects),
        | &Operand::Address { target, offset } => location(target, offset, objects),
    }
}

fn value_text(
    value: &abstract_machine::Value,
    objects: &HashMap<usize, &abstract_machine::Object>,
) -> String {
    match value {
        | abstract_machine::Value::Indeterminate => "?".to_owned(),
        | abstract_machine::Value::Integer(integer) => integer.to_string(),
        | abstract_machine::Value::Pointer { target: None, .. } => "NULL".to_owned(),
        | &abstract_machine::Value::Pointer {
            target: Some(target),
            offset,
        } => format!("&{}", location(target, offset, objects)),
        | abstract_machine::Value::Array(_) => "{…}".to_owned(),
    }
}

fn location(
    target: usize,
    offset: i32,
    objects: &HashMap<usize, &abstract_machine::Object>,
) -> String {
    objects.get(&target).map_or_else(
        || format!("#{target}"),
        |object| match object.value {
            | abstract_machine::Value::Array(_) => format!("{}[{offset}]", object.name),
            | _ if offset == 0 => object.name.clone(),
            | _ => format!("{}+{offset}", object.name),
        },
    )
}

fn clause(clause: Option<&abstract_machine::Clause>) -> Option<Clause> {
    clause.map(|clause| Clause {
        id:       clause.id.to_owned(),
        title:    clause.title.to_owned(),
        gloss:    clause.gloss.to_owned(),
        citation: clause.citation.to_owned(),
    })
}

fn diagnostic(fault: &abstract_machine::Diagnostic) -> Diagnostic {
    let title = match fault.kind {
        | "undefined_behavior" => "Undefined behavior detected",
        | "indeterminate" => "Indeterminate value read",
        | "unsupported" => "Outside the supported C subset",
        | "syntax" => "Source could not be compiled",
        | "limit" => "Engine resource limit reached",
        | _ => "Execution stopped",
    };
    let note = match (fault.clause, fault.kind) {
        | (Some(_), _) => "",
        | (None, "limit") =>
            "An engine resource limit is a prototype restriction, not undefined behavior.",
        | (None, _) =>
            "This diagnostic describes a restriction of the teaching engine; it does not mean the \
             program has undefined behavior.",
    };
    Diagnostic {
        kind:    Some(fault.kind.to_owned()),
        title:   title.to_owned(),
        message: fault.message.clone(),
        clause:  clause(fault.clause),
        note:    note.to_owned(),
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

    fn returned(states: &[Snapshot]) -> Option<&str> {
        integer(states.last()?.return_value.as_ref())
    }

    #[test]
    fn runs_the_tour_through_calls_pointers_and_lifetimes() {
        let mut machine = Machine::default();
        let states = execute(&mut machine, demos::find("tour").unwrap().source);
        let finished = states.last().unwrap();
        assert_eq!(finished.status, "finished", "{:?}", finished.diagnostic);
        assert_eq!(returned(&states), Some("15"));
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
        assert!(
            machine
                .run(Command::Compile {
                    source:   String::new(),
                    optimize: false,
                })
                .is_err(),
            "the compiler Worker compiles, not the abstract machine's"
        );

        let changed = demos::find("tour")
            .unwrap()
            .source
            .replace("{2, 4, 6}", "{4, 6, 8}");
        assert_eq!(returned(&execute(&mut machine, &changed)), Some("21"));
    }

    #[test]
    fn every_lesson_runs_to_its_documented_outcome() {
        let mut machine = Machine::default();
        for (key, status, result) in [
            ("tour", "finished", Some("15")),
            ("pointers", "finished", Some("17")),
            ("recursion", "finished", Some("24")),
            ("promotion", "finished", Some("255")),
            ("overflow", "undefined_behavior", None),
            ("bounds", "undefined_behavior", None),
            ("indeterminate", "error", None),
        ] {
            let demo = demos::find(key).unwrap();
            let states = execute(&mut machine, demo.source);
            let last = states.last().unwrap();
            assert_eq!(last.status, status, "{key}: {:?}", last.diagnostic);
            assert_eq!(returned(&states), result, "{key}");
            assert_eq!(demo.faults, result.is_none(), "{key} is labelled");
            for state in &states {
                assert!(
                    !state.event.title.is_empty(),
                    "{key} step {} has a title",
                    state.step
                );
                assert!(
                    !state.event.clauses.is_empty(),
                    "{key} step {} cites C99",
                    state.step
                );
            }
        }
    }

    #[test]
    fn snapshots_carry_event_ranges_and_expression_trees() {
        let mut machine = Machine::default();
        let source = demos::find("recursion").unwrap().source;
        let states = execute(&mut machine, source);
        let product = states
            .iter()
            .find(|state| state.event.title.starts_with("Compute 2 * 1"))
            .expect("factorial(2) multiplies");
        let range = product.event.range.expect("an exact range");
        assert_eq!(
            (range.start.line, range.start.column),
            (7, 12),
            "n * smaller"
        );
        assert_eq!(range.end.column, 23);
        let tree = product.tree.as_ref().expect("the return expression's tree");
        assert_eq!(tree.function, "factorial");
        let labels: Vec<&str> = tree.nodes.iter().map(|node| node.label.as_str()).collect();
        assert_eq!(labels, ["n", "smaller", "*", "return"]);
        assert_eq!(tree.nodes[2].value.as_deref(), Some("2"));
        assert!(tree.nodes[2].active);
        assert_eq!(tree.nodes[0].object.as_deref(), Some("n"));
        let depth = states.iter().map(|state| state.stack.len()).max();
        assert_eq!(depth, Some(5), "main and four factorial frames");
        assert!(
            states.iter().any(
                |state| state.stack.last().and_then(|main| main.waiting.as_deref())
                    == Some("factorial(4)")
            ),
            "main shows the call it waits on"
        );
    }

    #[test]
    fn stops_the_diagnostic_demos_with_citations() {
        let mut machine = Machine::default();
        for (key, status, kind) in [
            ("indeterminate", "error", "indeterminate"),
            ("bounds", "undefined_behavior", "undefined_behavior"),
            ("overflow", "undefined_behavior", "undefined_behavior"),
        ] {
            let states = execute(&mut machine, demos::find(key).unwrap().source);
            let finished = states.last().unwrap();
            assert_eq!(finished.status, status, "{key}");
            let diagnostic = finished.diagnostic.as_ref().unwrap();
            assert_eq!(diagnostic.kind.as_deref(), Some(kind));
            assert!(
                diagnostic
                    .clause
                    .as_ref()
                    .is_some_and(|clause| clause.id.starts_with('6')),
                "{key} cites the standard"
            );
            assert_eq!(finished.event.category, "fault");
            assert!(finished.location.is_some(), "{key} locates the fault");
        }
    }

    #[test]
    fn the_compiler_lessons_are_outside_the_machines_subset() {
        let mut machine = Machine::default();
        for demo in demos::DEMOS.iter().filter(|demo| demo.stage.is_some()) {
            let rejected = machine.load(demo.source.to_owned());
            assert_eq!(rejected.status, "error", "{}", demo.key);
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
