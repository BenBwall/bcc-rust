//! The engine Worker's side: run the bcc abstract machine and present each of
//! its states as a [`Snapshot`]. Only the `worker` build links the engine.
//!
//! The engine reports byte offsets into UTF-8 source; the page's editor counts
//! UTF-16 columns, so ranges are converted here. Values are rendered as the
//! short text the page shows (`5`, `&values[1]`), and the current full
//! expression's tree is resolved from the program.

mod flow;

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
        MemberValue,
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

/// Why the abstract machine's Worker answers no compile request.
const NOT_A_COMPILER: &str =
    "The abstract machine's Worker does not compile; the compiler Worker does.";

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
            | Command::Compile { .. } | Command::CompileWasm(_) => Err(NOT_A_COMPILER.into()),
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
            | Err(diagnostic) => match first_directive(&self.source) {
                | Some(directive) => before_the_machine(directive, &self.source),
                | None => rejected(&diagnostic, &self.source),
            },
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

/// The byte range of the first preprocessing directive in `source`, up to the
/// end of its physical line: a `#` that is the first token on its line.
/// Comments count as white space there, having become spaces in translation
/// phase 3, and a `#` inside a comment or a literal is not a directive.
///
/// C99: §5.1.1.2 paragraph 1 item 3, p. 10; PDF p. 22.
/// C99: §6.10 paragraph 2, p. 146; PDF p. 158.
fn first_directive(source: &str) -> Option<SourceRange> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut starts_line = true;
    while let Some(&byte) = bytes.get(at) {
        let rest = &bytes[at + 1..];
        match byte {
            | b'\n' => {
                starts_line = true;
                at += 1;
            },
            | b'\\' if rest.starts_with(b"\n") => at += 2,
            | b'\\' if rest.starts_with(b"\r\n") => at += 3,
            | b'/' if rest.starts_with(b"*") => {
                let body = &source[at + 2..];
                at += 2 + body.find("*/").map_or(body.len(), |end| end + 2);
            },
            | b'/' if rest.starts_with(b"/") => at = end_of_logical_line(bytes, at),
            | b'#' if starts_line => {
                let line = source[at..].lines().next().unwrap_or_default();
                return Some(SourceRange {
                    start: at,
                    end: at + line.trim_end().len(),
                    ..SourceRange::default()
                });
            },
            | b'"' | b'\'' => {
                starts_line = false;
                at = end_of_literal(bytes, at);
            },
            | b' ' | b'\t' | b'\r' | 0x0B | 0x0C => at += 1,
            | _ => {
                starts_line = false;
                at += 1;
            },
        }
    }
    None
}

/// The offset of the new-line ending the line at `start`, past any line
/// splices, or the end of `bytes`.
fn end_of_logical_line(bytes: &[u8], start: usize) -> usize {
    let mut at = start;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            | b'\n' => return at,
            | b'\\' if bytes[at + 1..].starts_with(b"\r\n") => at += 3,
            | b'\\' => at += 2,
            | _ => at += 1,
        }
    }
    bytes.len()
}

/// The offset just past the string or character literal opening at `start`, or
/// the new-line ending it if it is unterminated.
fn end_of_literal(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut at = start + 1;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            | b'\n' => return at,
            | b'\\' if bytes[at + 1..].starts_with(b"\r\n") => at += 3,
            | b'\\' => at += 2,
            | _ if byte == quote => return at + 1,
            | _ => at += 1,
        }
    }
    bytes.len()
}

/// The snapshot for a program with preprocessing directives. The machine reads
/// source as written, so it never gets to run one, and stopping here says
/// nothing about the program: directives are executed in translation phase 4,
/// long before any evaluation.
///
/// C99: §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22.
fn before_the_machine(directive: SourceRange, source: &str) -> Snapshot {
    let location = range(directive, source);
    Snapshot {
        step: 0,
        status: "error".to_owned(),
        location,
        event: StepEvent {
            kind:           "fault".to_owned(),
            category:       "fault".to_owned(),
            title:          "The machine starts after the preprocessor".to_owned(),
            summary:        "Lines starting with # are handled before the program runs, and this \
                             machine reads source only as it is written, so it stops here. \
                             Stopping here does not mean the program is wrong."
                .to_owned(),
            explanation:    "C processes a file in translation phases: directives are executed \
                             and macros expanded in phase 4, and evaluation only happens once the \
                             program runs. To see what these lines become, open the compiler \
                             pipeline's Preprocessor stage. To step through the code here, remove \
                             the # lines and write out what they stood for."
                .to_owned(),
            note:           String::new(),
            flow:           Vec::new(),
            access:         Vec::new(),
            clauses:        vec![Clause {
                id:       "5.1.1.2p1".to_owned(),
                title:    "Translation phases".to_owned(),
                gloss:    "Phase 4 executes preprocessing directives and expands macro \
                           invocations."
                    .to_owned(),
                citation: "C99 §5.1.1.2p1, p. 10; PDF p. 22".to_owned(),
            }],
            range:          location,
            objects:        Vec::new(),
            sequence_point: false,
        },
        tree: None,
        stack: Vec::new(),
        objects: Vec::new(),
        effects: Vec::new(),
        diagnostic: None,
        return_value: None,
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
            summary:        "The abstract machine runs only a small subset of C, and this \
                             program uses something outside it."
                .to_owned(),
            explanation:    format!(
                "The abstract machine could not compile it: {}. The compiler pipeline view may \
                 still show what bcc itself makes of it.",
                diagnostic.message
            ),
            note:           String::new(),
            flow:           Vec::new(),
            access:         Vec::new(),
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
    // The operator of the expression node the step evaluated, for the flow.
    let operator = state.evaluation.as_ref().and_then(|evaluation| {
        let nodes = program.expression(evaluation.function, evaluation.root);
        let first = nodes.first()?.first;
        nodes
            .get(evaluation.last?.checked_sub(first)?)
            .map(|node| node.label.as_str())
    });
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
            summary:        event.summary.clone(),
            explanation:    event.explanation.clone(),
            note:           event.note.clone(),
            flow:           flow::flow(
                event,
                operator,
                state.status == "finished",
                &objects,
                program,
            ),
            access:         flow::access(event),
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
                scope:     object.function.clone(),
                parameter: object.parameter,
                value:     value(&object.value, &objects, program),
            })
            .collect(),
        effects:      state.pending_effects.clone(),
        diagnostic:   fault.map(diagnostic),
        return_value: state
            .return_value
            .as_ref()
            .map(|returned| value(returned, &objects, program)),
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
                    value:    operand.map(|operand| operand_text(operand, objects, program)),
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

/// How the schematic labels an operand on a wire: `5`, `&values[1]`, the
/// designated object `values[1]` for an lvalue, or `{…}` for a structure,
/// whose members would not fit there.
fn operand_text(
    operand: &Operand,
    objects: &HashMap<usize, &abstract_machine::Object>,
    program: &Program,
) -> String {
    match operand {
        | Operand::Value(abstract_machine::Value::Struct { .. }) => "{…}".to_owned(),
        | Operand::Value(value) => value_text(value, objects, program),
        | &Operand::Address {
            target,
            offset,
            structure,
        } => location(target, offset, structure, objects, program),
    }
}

fn value_text(
    value: &abstract_machine::Value,
    objects: &HashMap<usize, &abstract_machine::Object>,
    program: &Program,
) -> String {
    match value {
        | abstract_machine::Value::Indeterminate => "?".to_owned(),
        | abstract_machine::Value::Integer(integer) => integer.to_string(),
        | abstract_machine::Value::Pointer { target: None, .. } => "NULL".to_owned(),
        | &abstract_machine::Value::Pointer {
            target: Some(target),
            offset,
            structure,
            ..
        } => format!("&{}", location(target, offset, structure, objects, program)),
        | abstract_machine::Value::Array(_) => "{…}".to_owned(),
        | abstract_machine::Value::Struct { members, .. } => format!(
            "{{{}}}",
            members
                .iter()
                .map(|member| value_text(member, objects, program))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// How the source names the place at scalar `offset` of `target` (or the
/// whole structure `structure` there).
fn location(
    target: usize,
    offset: i32,
    structure: Option<usize>,
    objects: &HashMap<usize, &abstract_machine::Object>,
    program: &Program,
) -> String {
    objects.get(&target).map_or_else(
        || format!("#{target}"),
        |object| program.path(object, offset, structure),
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
    program: &Program,
) -> Value {
    match value {
        | abstract_machine::Value::Indeterminate => Value::Indeterminate,
        | abstract_machine::Value::Integer(number) => Value::Integer {
            text: number.to_string(),
        },
        | abstract_machine::Value::Array(elements) => Value::Array {
            elements: elements
                .iter()
                .map(|element| self::value(element, objects, program))
                .collect(),
        },
        | abstract_machine::Value::Struct { structure, members } => {
            let layout = program.structures().get(*structure);
            Value::Struct {
                name:    layout.map_or_else(String::new, |layout| format!("struct {}", layout.name)),
                size:    layout.map_or(0, |layout| layout.size),
                members: members
                    .iter()
                    .enumerate()
                    .map(|(index, member)| {
                        let declared = layout.and_then(|layout| layout.members.get(index));
                        MemberValue {
                            name:    declared.map(|member| member.name.clone()).unwrap_or_default(),
                            ty:      declared
                                .map(|member| member.r#type.clone())
                                .unwrap_or_default(),
                            offset:  declared.map_or(0, |member| member.offset),
                            size:    declared.map_or(0, |member| member.size),
                            padding: declared.map_or(0, |member| member.padding),
                            value:   self::value(member, objects, program),
                        }
                    })
                    .collect(),
            }
        },
        | &abstract_machine::Value::Pointer {
            target,
            offset,
            first,
            length,
            structure,
        } => {
            let object = target.and_then(|target| objects.get(&target));
            let one_past = target.is_some() && offset == first + length;
            let to = object.map_or_else(String::new, |object| {
                if one_past {
                    let array = program.path(object, first, None);
                    let array = array.strip_suffix("[0]").unwrap_or(&array);
                    format!("{array}+{length}")
                } else {
                    program.path(object, offset, structure)
                }
            });
            Value::Pointer {
                target,
                offset,
                one_past,
                ended: object.is_some_and(|object| object.lifetime == "ended"),
                to,
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

    #[test]
    fn finds_a_directive_only_where_one_can_stand() {
        let found = |source| first_directive(source).map(|range| range.start);
        assert_eq!(found("#define X\n"), Some(0));
        assert_eq!(found("  /* note */ # define X\r\nint y;\n"), Some(13));
        assert_eq!(found("int x;\n#if 0\n"), Some(7));
        assert_eq!(found("int x; #if 0\n"), None);
        assert_eq!(found("/* #a\n#b */\n"), None);
        assert_eq!(found("// #a\\\n#b\n"), None);
        assert_eq!(found("char *s = \"\\\n#a\";\n"), None);
        assert_eq!(found("int x; /* a\n*/ #b\n"), None);
        assert_eq!(found("char c = '#';\n"), None);
        assert_eq!(found("int x\\\n#a\n"), None);
    }

    #[test]
    fn a_directive_stops_the_machine_without_blaming_the_program() {
        let mut machine = Machine::default();
        let state =
            machine.load("// note\n  #define X 1\nint main(void) { return X; }\n".to_owned());
        assert_eq!(
            state.event.title,
            "The machine starts after the preprocessor"
        );
        assert!(state.diagnostic.is_none());
        assert!(!state.can_step());
        assert_eq!(state.location.map(|range| range.start.line), Some(2));
        assert_eq!(
            machine.step(),
            Err("Load a program before stepping.".into())
        );

        for source in [
            "int main(void) { return 1.5; }\n",
            "/*\n#define X 1\n*/\nint main(void) { return 1.5; }\n",
            "// a line \\\n#define X 1\nint main(void) { return 1.5; }\n",
            "int a = 1.5; int s = \"\\\n#define X\";\nint main(void) { return 0; }\n",
            "int a = 1.5 /* a\n*/ #define\n;",
        ] {
            let state = machine.load(source.to_owned());
            assert_eq!(
                state.event.title, "This program is outside the supported subset",
                "{source}"
            );
            assert!(state.diagnostic.is_some(), "{source}");
        }
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
    fn steps_show_their_value_flow_and_the_objects_they_touch() {
        let mut machine = Machine::default();
        let states = execute(&mut machine, demos::find("promotion").unwrap().source);
        let event = |kind: &str| {
            states
                .iter()
                .map(|state| &state.event)
                .find(|event| event.kind == kind)
                .unwrap_or_else(|| panic!("a {kind} step"))
        };
        let shape = |event: &StepEvent| {
            event
                .flow
                .iter()
                .map(|part| format!("{}:{}", part.role, part.text))
                .collect::<Vec<_>>()
        };
        let read = event("lvalue_conversion");
        assert_eq!(shape(read), ["object:small", "arrow:read", "value:100"]);
        assert_eq!(read.access.len(), 1);
        assert_eq!(
            (read.access[0].cell, read.access[0].kind.as_str()),
            (Some(0), "read")
        );
        assert_eq!(
            shape(event("integer_promotion")),
            ["value:100", "arrow:promote", "value:100"]
        );
        assert_eq!(
            shape(event("arithmetic")),
            [
                "value:100",
                "operator:+",
                "value:100",
                "arrow:=",
                "value:200"
            ]
        );
        // 200 does not fit in a char: the store shows the conversion.
        let narrow = states
            .iter()
            .map(|state| &state.event)
            .find(|event| event.title.starts_with("Initialize `narrow`"))
            .expect("narrow is initialized");
        assert_eq!(
            shape(narrow),
            [
                "value:200",
                "arrow:convert",
                "value:-56",
                "arrow:store",
                "object:narrow"
            ]
        );
        assert_eq!(narrow.flow[2].ty.as_deref(), Some("char"));
        assert_eq!(narrow.access[0].kind, "write");
        assert_eq!(event("declare").access[0].cell, None);
        // The first step of a run explains why the abstract machine matters.
        assert!(
            states[0].event.note.contains("abstract machine"),
            "{}",
            states[0].event.note
        );
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
            ("path", "finished", Some("16")),
            ("pointers", "finished", Some("17")),
            ("recursion", "finished", Some("24")),
            ("structs", "finished", Some("31")),
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
                assert!(
                    state.event.summary.ends_with('.')
                        && state.event.summary.len() < state.event.explanation.len().max(171),
                    "{key} step {} leads with one sentence: {:?}",
                    state.step,
                    state.event.summary
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
        // The whole-path lesson opens in the pipeline but runs here too.
        for demo in demos::DEMOS
            .iter()
            .filter(|demo| demo.stage.is_some() && demo.path.is_empty())
        {
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
    fn structures_and_static_objects_reach_the_page() {
        let mut machine = Machine::default();
        let states = execute(&mut machine, demos::find("structs").unwrap().source);
        // Before main: static storage, no frame.
        let first = &states[1];
        assert_eq!(first.event.kind, "static_storage");
        assert_eq!(first.stack, []);
        assert_eq!(first.objects[0].storage, "static");
        assert_eq!(first.objects[0].frame_id, 0);
        assert_eq!(first.event.access[0].kind, "create");
        let total = states
            .iter()
            .flat_map(|state| &state.objects)
            .find(|object| object.name == "total")
            .expect("the static local");
        assert_eq!(total.scope.as_deref(), Some("moved"));
        // A structure lists its members with their layout.
        let last = states.last().unwrap();
        let entry = last
            .objects
            .iter()
            .find(|object| object.name == "e")
            .expect("e");
        let Value::Struct { name, size, members } = &entry.value else {
            panic!("e is a structure: {:?}", entry.value);
        };
        assert_eq!((name.as_str(), *size), ("struct entry", 8));
        let layout: Vec<(&str, &str, usize, usize, usize)> = members
            .iter()
            .map(|member| {
                (
                    member.name.as_str(),
                    member.ty.as_str(),
                    member.offset,
                    member.size,
                    member.padding,
                )
            })
            .collect();
        assert_eq!(
            layout,
            [("tag", "char", 0, 1, 3), ("count", "int", 4, 4, 0)]
        );
        assert_eq!(entry.value.cells(), 2);
        // Member steps name the member, in the flow and the expression tree.
        let arrow = states
            .iter()
            .find(|state| state.event.kind == "member" && state.event.title.starts_with("`at->y`"))
            .expect("at->y");
        let flow: Vec<String> = arrow
            .event
            .flow
            .iter()
            .map(|part| format!("{}:{}", part.role, part.text))
            .collect();
        assert_eq!(flow, ["value:&b", "arrow:->", "object:b.y"]);
        assert_eq!(arrow.event.access[0].cell, Some(1));
        // A copy shows its members in the flow, `{…}` on the schematic's wires.
        let copy = states
            .iter()
            .find(|state| state.event.title.starts_with("Copy"))
            .expect("b = moved(b, 4) copies");
        assert_eq!(copy.event.flow[0].text, "{12, 12}");
        let tree = copy.tree.as_ref().expect("the assignment's tree");
        assert_eq!(tree.nodes.last().and_then(|node| node.value.as_deref()), Some("{…}"));
        let px = last
            .objects
            .iter()
            .find(|object| object.name == "px")
            .map(|object| &object.value);
        assert!(
            matches!(px, Some(Value::Pointer { to, .. }) if to == "b.x"),
            "{px:?}"
        );
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
