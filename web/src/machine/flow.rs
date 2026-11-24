//! What one step did, in the two forms the page draws besides the
//! narration: its value flow (the operands it took and the value or object it
//! produced, as chips joined by arrows) and its accesses (which objects it
//! read, wrote, created, ended or only designated), for the memory diagram.

use std::{
    collections::HashMap,
    fmt::Write,
};

use bcc_rust::abstract_machine::{
    self,
    Event,
    EventKind,
    Operand,
    Program,
};

use crate::snapshot::{
    Access,
    FlowPart,
};

type Objects<'a> = HashMap<usize, &'a abstract_machine::Object>;

/// The chips for `event`. `operator` is the operator of the expression node
/// the step evaluated, `exited` whether the step ended the program, and
/// `written` what it wrote to standard output.
pub(super) fn flow(
    event: &Event,
    operator: Option<&str>,
    exited: bool,
    written: &[u8],
    objects: &Objects<'_>,
    program: &Program,
) -> Vec<FlowPart> {
    let chips = Chips { objects, program };
    let first = event.operands.first();
    let second = event.operands.get(1);
    let result = event.result.as_ref();
    let operand = |operand: Option<&Operand>| operand.map(|operand| chips.operand(operand));
    let to = event.result_type.as_deref();
    let parts: Vec<Option<FlowPart>> = match event.kind {
        | EventKind::LvalueConversion => vec![operand(first), Some(arrow("read")), operand(result)],
        | EventKind::IntegerPromotion => chips.conversion(event, "promote", "char", "int"),
        | EventKind::ArithmeticConversion =>
            chips.conversion(event, "convert", "int", "unsigned int"),
        | EventKind::Sizeof => vec![
            program
                .source()
                .get(event.range.start..event.range.end)
                .map(|text| part("label", text)),
            Some(arrow("is")),
            operand(result).map(|chip| typed(chip, "size_t")),
        ],
        | EventKind::ArrayDecay => chips.decay(event),
        | EventKind::FunctionDesignator => chips.designator(event),
        | EventKind::AddressOf => vec![
            result.and_then(|result| chips.pointee(result)),
            Some(arrow("address")),
            operand(result),
        ],
        | EventKind::Dereference => vec![operand(first), Some(arrow("*")), operand(result)],
        | EventKind::Member if first.is_some() =>
            vec![operand(first), Some(arrow("->")), operand(result)],
        | EventKind::Member => vec![
            event.objects.first().map(|&id| chips.whole(id)),
            Some(arrow("member")),
            operand(result),
        ],
        | EventKind::StaticStorage => vec![
            event.objects.first().map(|&id| chips.whole(id)),
            Some(arrow("starts at")),
            operand(result),
        ],
        | EventKind::Subscript => vec![
            operand(first),
            Some(operator_chip("+")),
            operand(second),
            Some(arrow("designates")),
            operand(result),
        ],
        | EventKind::Unary => vec![
            operator.map(operator_chip),
            operand(first),
            Some(arrow("=")),
            operand(result),
        ],
        | EventKind::Arithmetic | EventKind::Comparison | EventKind::PointerArithmetic => {
            let shown = match (event.kind, operator) {
                | (EventKind::PointerArithmetic, Some(operator @ ("+" | "-"))) => operator,
                | (EventKind::PointerArithmetic, _) => "+",
                | (_, operator) => operator.unwrap_or("·"),
            };
            vec![
                operand(first),
                Some(operator_chip(shown)),
                operand(second),
                Some(arrow("=")),
                operand(result),
            ]
        },
        // `char s[] = "hi";` stores a string's characters at once.
        | EventKind::Initialize
            if let Some(Operand::Value(abstract_machine::Value::Array(elements))) =
                event.operands.get(1) =>
            vec![
                Some(part("value", &characters(elements))),
                Some(arrow("store")),
                operand(first),
            ],
        | EventKind::Initialize | EventKind::Assign => chips.store(event),
        | EventKind::Increment => chips.increment(event),
        | EventKind::CompoundAssign => chips.compound(event, operator),
        | EventKind::Cast => chips.cast(event, operator),
        // Reaching a declaration brings the object's name into scope (C99
        // §6.2.1p7); its block's entry created it.
        | EventKind::EnterBlock | EventKind::LeaveBlock => chips.lifetimes(event),
        | EventKind::Declare => vec![
            event.objects.first().map(|&id| chips.whole(id)),
            Some(arrow("in scope, holds")),
            Some(part("value", "?")),
        ],
        | EventKind::Constant => vec![operand(result).map(|chip| typed(chip, to.unwrap_or("int")))],
        | EventKind::Branch if operator == Some("switch") => switched(event, operand(first)),
        | EventKind::Branch => chips.branch(first),
        | EventKind::Logical | EventKind::Conditional | EventKind::Comma => chips.sequenced(event),
        | EventKind::Call => chips.call(event),
        | EventKind::LibraryCall => chips.library_call(event, written),
        | EventKind::Directive => directive(event, program),
        | EventKind::Enumeration => enumeration(event, program),
        | EventKind::Jump => goto(event, program),
        | EventKind::Return => vec![
            operand(first),
            Some(arrow(if exited { "exit status" } else { "returned" })),
        ],
        | _ => Vec::new(),
    };
    parts.into_iter().flatten().collect()
}

/// A goto's jump: the goto, and the label it jumps to, with its line.
/// Other jumps draw nothing.
fn goto(event: &Event, program: &Program) -> Vec<Option<FlowPart>> {
    let Some(label) = program.goto_label(event.range) else {
        return Vec::new();
    };
    let source = program.source();
    vec![
        source
            .get(event.range.start..event.range.end)
            .map(|text| part("label", text.trim_end_matches(';'))),
        Some(arrow("jumps to")),
        source
            .get(label.start..label.end)
            .map(|text| part("value", &format!("{text} line {}", label.line))),
    ]
}

/// A directive the machine ran in translation phase 4: `#include
/// <stdio.h>` declares the output functions, and `#define NAME value`
/// makes the name stand for its replacement.
fn directive(event: &Event, program: &Program) -> Vec<Option<FlowPart>> {
    let text = program
        .source()
        .get(event.range.start..event.range.end)
        .unwrap_or_default();
    let words: Vec<&str> = text.trim_start_matches('#').split_whitespace().collect();
    match words.as_slice() {
        | ["define", name, ..] => {
            let replacement = text
                .trim_start_matches('#')
                .trim_start()
                .trim_start_matches("define")
                .trim_start()
                .trim_start_matches(name)
                .trim();
            vec![
                Some(part("label", &format!("#define {name}"))),
                Some(arrow("stands for")),
                Some(part("value", replacement)),
            ]
        },
        | _ => vec![
            Some(part("label", "#include <stdio.h>")),
            Some(arrow("declares")),
            Some(part("value", "printf, puts, putchar")),
        ],
    }
}

/// An enumeration's declaration: the type, and the constants it declares
/// with their values. The step's range is the declaration's, which tells
/// two enumerations with one name apart, as in sibling blocks; one without
/// a tag or a typedef name has no name to draw.
fn enumeration(event: &Event, program: &Program) -> Vec<Option<FlowPart>> {
    let Some(declared) = program
        .enumerations()
        .iter()
        .find(|known| known.range == event.range && !known.name.is_empty())
    else {
        return Vec::new();
    };
    let constants: Vec<String> = declared
        .constants
        .iter()
        .map(|constant| format!("{} = {}", constant.name, constant.value))
        .collect();
    vec![
        Some(part("label", &declared.name)),
        Some(arrow("declares")),
        Some(part("value", &constants.join(", "))),
    ]
}

/// How `event` used each object it touched.
pub(super) fn access(event: &Event) -> Vec<Access> {
    let at = |operand: Option<&Operand>, kind: &str| match operand {
        | Some(
            &(Operand::Address {
                target,
                offset,
                structure: None,
            }
            | Operand::Value(abstract_machine::Value::Pointer {
                target: Some(target),
                offset,
                structure: None,
                ..
            })),
        ) => vec![Access {
            object: target,
            cell:   usize::try_from(offset).ok(),
            kind:   kind.to_owned(),
        }],
        // A whole structure: every cell of it.
        | Some(
            &(Operand::Address {
                target,
                offset,
                structure: Some(_),
            }
            | Operand::Value(abstract_machine::Value::Pointer {
                target: Some(target),
                offset,
                structure: Some(_),
                ..
            })),
        ) => vec![Access {
            object: target,
            cell:   (offset != 0)
                .then(|| usize::try_from(offset).ok())
                .flatten(),
            kind:   kind.to_owned(),
        }],
        | _ => Vec::new(),
    };
    let whole = |kind: &str| {
        event
            .objects
            .iter()
            .map(|&object| Access {
                object,
                cell: None,
                kind: kind.to_owned(),
            })
            .collect()
    };
    match event.kind {
        | EventKind::LvalueConversion => at(event.operands.first(), "read"),
        | EventKind::Initialize
        | EventKind::Assign
        | EventKind::Increment
        | EventKind::CompoundAssign => at(event.operands.first(), "write"),
        | EventKind::EnterBlock | EventKind::Call | EventKind::StaticStorage => whole("create"),
        | EventKind::Declare => whole("declare"),
        | EventKind::LibraryCall => whole("read"),
        | EventKind::LeaveBlock | EventKind::Return => whole("end"),
        | EventKind::Designate
        | EventKind::Subscript
        | EventKind::Dereference
        | EventKind::Member
        | EventKind::StaticLocal => at(event.result.as_ref(), "designate"),
        | EventKind::AddressOf | EventKind::ArrayDecay => at(event.result.as_ref(), "address"),
        | _ => Vec::new(),
    }
}

/// Builds chips that name objects as the page does (`values[1]`).
struct Chips<'a, 'o> {
    objects: &'a Objects<'o>,
    program: &'a Program,
}

impl Chips<'_, '_> {
    /// A promotion or a conversion: the value in the type it had, and in
    /// the type it has now, as the engine names them (`from` and `to` when
    /// it does not).
    fn conversion(&self, event: &Event, verb: &str, from: &str, to: &str) -> Vec<Option<FlowPart>> {
        let operand = |operand: Option<&Operand>| operand.map(|operand| self.operand(operand));
        vec![
            operand(event.operands.first())
                .map(|chip| typed(chip, event.from_type.as_deref().unwrap_or(from))),
            Some(arrow(verb)),
            operand(event.result.as_ref())
                .map(|chip| typed(chip, event.result_type.as_deref().unwrap_or(to))),
        ]
    }

    /// `i++`: the object, what it held and what it holds now. Inside a
    /// condition the update also yields a value: the old one for a postfix
    /// operator, the new one for a prefix one.
    fn increment(&self, event: &Event) -> Vec<Option<FlowPart>> {
        let operand = |operand: Option<&Operand>| operand.map(|operand| self.operand(operand));
        let mut parts = vec![
            operand(event.operands.first()),
            Some(arrow("was")),
            operand(event.operands.get(1)),
            Some(arrow("now")),
        ];
        if let Some(stored) = event.operands.get(2) {
            parts.extend([
                operand(Some(stored)),
                Some(arrow("yields")),
                operand(event.result.as_ref()),
            ]);
        } else {
            parts.push(operand(event.result.as_ref()));
        }
        parts
    }

    /// A store: the value, converted when the object's type changes it,
    /// into the object.
    fn store(&self, event: &Event) -> Vec<Option<FlowPart>> {
        let (target, value, stored) = (
            event.operands.first(),
            event.operands.get(1),
            event.result.as_ref(),
        );
        let operand = |operand: Option<&Operand>| operand.map(|operand| self.operand(operand));
        let converted = value
            .zip(stored)
            .is_some_and(|(value, stored)| value != stored);
        if converted && let Some(Operand::Address { target: id, .. }) = target {
            let ty = event.result_type.as_deref().unwrap_or_else(|| {
                self.objects
                    .get(id)
                    .map_or("char", |object| object.r#type.as_str())
            });
            vec![
                operand(value).map(|chip| typed(chip, event.from_type.as_deref().unwrap_or("int"))),
                Some(arrow("convert")),
                operand(stored).map(|chip| typed(chip, ty)),
                Some(arrow("store")),
                operand(target),
            ]
        } else {
            vec![operand(value), Some(arrow("store")), operand(target)]
        }
    }

    /// `total += step`: the object, its old value, the operator and the
    /// right operand, and what it holds now.
    fn compound(&self, event: &Event, operator: Option<&str>) -> Vec<Option<FlowPart>> {
        let operand = |index: usize| {
            event
                .operands
                .get(index)
                .map(|operand| self.operand(operand))
        };
        vec![
            operand(0),
            Some(arrow("was")),
            operand(1),
            operator.map(operator_chip),
            operand(2),
            Some(arrow("now")),
            event.result.as_ref().map(|result| self.operand(result)),
        ]
    }

    /// `(char)300`: the operand, the cast, and the value in its new type.
    fn cast(&self, event: &Event, operator: Option<&str>) -> Vec<Option<FlowPart>> {
        let ty = operator.map_or("int", |cast| cast.trim_matches(['(', ')']));
        vec![
            event.operands.first().map(|operand| self.operand(operand)),
            Some(arrow(operator.unwrap_or("cast"))),
            event
                .result
                .as_ref()
                .map(|result| typed(self.operand(result), ty)),
        ]
    }

    /// A condition and whether C counts it as true.
    fn branch(&self, condition: Option<&Operand>) -> Vec<Option<FlowPart>> {
        vec![
            condition.map(|operand| self.operand(operand)),
            Some(arrow("counts as")),
            Some(part(
                "label",
                if truth(condition) { "true" } else { "false" },
            )),
        ]
    }

    /// A step of `&&`, `||`, `?:` or a comma operator: the test of the
    /// first operand, what it counts as and the result when that settles
    /// it (C99 §6.5.13p4, §6.5.14p4, §6.5.15p4); the end of a comma's left
    /// operand, its value discarded (§6.5.17p2); or the value the operator
    /// yields from its last operand.
    fn sequenced(&self, event: &Event) -> Vec<Option<FlowPart>> {
        let first = event.operands.first();
        let result = event.result.as_ref();
        let operand = |operand: Option<&Operand>| operand.map(|operand| self.operand(operand));
        match event.kind {
            | EventKind::Comma if event.sequence_point => vec![
                operand(first),
                Some(arrow(if first.is_some() { "discarded" } else { "done" })),
            ],
            | _ if event.sequence_point => {
                let mut parts = self.branch(first);
                parts.push(Some(arrow("so")));
                parts.push(Some(part(
                    "label",
                    match (event.kind, result) {
                        | (EventKind::Logical, Some(_)) => "skip the right",
                        | (EventKind::Logical, None) => "evaluate the right",
                        | _ if truth(first) => "choose the second",
                        | _ => "choose the third",
                    },
                )));
                if result.is_some() {
                    parts.extend([Some(arrow("result")), operand(result)]);
                }
                parts
            },
            | _ => vec![
                operand(first),
                Some(arrow("yields")),
                operand(result).map(|chip| match event.result_type.as_deref() {
                    | Some(ty) => typed(chip, ty),
                    | None => chip,
                }),
            ],
        }
    }

    /// The objects a block's entry created, indeterminate, or its exit
    /// ended (C99 §6.2.4p5): nothing for a block without objects.
    fn lifetimes(&self, event: &Event) -> Vec<Option<FlowPart>> {
        if event.objects.is_empty() {
            return Vec::new();
        }
        let entry = event.kind == EventKind::EnterBlock;
        let verb = match (entry, event.objects.len() == 1) {
            | (true, true) => "begins, holding",
            | (true, false) => "begin, holding",
            | (false, true) => "ends",
            | (false, false) => "end",
        };
        let mut parts: Vec<Option<FlowPart>> = event
            .objects
            .iter()
            .map(|&id| Some(self.whole(id)))
            .collect();
        parts.push(Some(arrow(verb)));
        if entry {
            parts.push(Some(part("value", "?")));
        }
        parts
    }

    /// An array and the pointer to its first element it decays to.
    fn decay(&self, event: &Event) -> Vec<Option<FlowPart>> {
        let array = event.objects.first().map(|&id| self.whole(id));
        // A const array decays to a pointer to const elements.
        let ty = array
            .as_ref()
            .and_then(|chip| chip.ty.as_deref())
            .unwrap_or("int");
        let element = ty.split('[').next().unwrap_or("int").trim();
        let pointer = format!("{element} *");
        vec![
            array,
            Some(arrow("decays to")),
            event
                .result
                .as_ref()
                .map(|result| typed(self.operand(result), &pointer)),
        ]
    }

    /// A function's name and the pointer to the function it becomes as an
    /// operand.
    ///
    /// C99: §6.3.2.1 paragraph 4, p. 46; PDF p. 58.
    fn designator(&self, event: &Event) -> Vec<Option<FlowPart>> {
        vec![
            self.program
                .source()
                .get(event.range.start..event.range.end)
                .map(|text| part("label", text)),
            Some(arrow("becomes")),
            event.result.as_ref().map(|result| self.operand(result)),
        ]
    }

    /// A library call's arguments and the text it wrote.
    fn library_call(&self, event: &Event, written: &[u8]) -> Vec<Option<FlowPart>> {
        let mut parts = Vec::new();
        for (index, argument) in event.operands.iter().enumerate() {
            if index > 0 {
                parts.push(Some(part("separator", ",")));
            }
            parts.push(Some(self.operand(argument)));
        }
        parts.extend([Some(arrow("writes")), Some(part("value", &quoted(written)))]);
        parts
    }

    /// Each argument copied into its parameter.
    fn call(&self, event: &Event) -> Vec<Option<FlowPart>> {
        let mut parts = Vec::new();
        for (index, (argument, &parameter)) in event.operands.iter().zip(&event.objects).enumerate()
        {
            if index > 0 {
                parts.push(Some(part("separator", ",")));
            }
            parts.extend([
                Some(self.operand(argument)),
                Some(arrow("copied to")),
                Some(self.whole(parameter)),
            ]);
        }
        parts
    }

    fn operand(&self, operand: &Operand) -> FlowPart {
        match operand {
            | Operand::Value(value) => part(
                "value",
                &super::value_text(value, self.objects, self.program),
            ),
            | &Operand::Address {
                target,
                offset,
                structure,
            } => FlowPart {
                object: self.name(target),
                ..part(
                    "object",
                    &super::location(target, offset, structure, self.objects, self.program),
                )
            },
        }
    }

    /// The object a pointer value points to.
    fn pointee(&self, operand: &Operand) -> Option<FlowPart> {
        match operand {
            | &Operand::Value(abstract_machine::Value::Pointer {
                target: Some(target),
                offset,
                structure,
                ..
            }) => Some(self.operand(&Operand::Address {
                target,
                offset,
                structure,
            })),
            | _ => None,
        }
    }

    /// A whole object, with its type: `values: int[3]`.
    fn whole(&self, id: usize) -> FlowPart {
        let object = self.objects.get(&id);
        FlowPart {
            role:   "object".to_owned(),
            text:   object.map_or_else(|| format!("#{id}"), |object| object.name.clone()),
            ty:     object.map(|object| object.r#type.clone()),
            object: self.name(id),
        }
    }

    fn name(&self, id: usize) -> Option<String> {
        self.objects.get(&id).map(|object| object.name.clone())
    }
}

fn part(role: &str, text: &str) -> FlowPart {
    FlowPart {
        role: role.to_owned(),
        text: text.to_owned(),
        ..FlowPart::default()
    }
}

/// Whether C counts `condition` as true: it compares unequal to 0, and a
/// pointer is true unless it is null (C99 §6.8.4.1p2, §6.5.15p4).
fn truth(condition: Option<&Operand>) -> bool {
    condition.is_some_and(|operand| match operand {
        | Operand::Value(abstract_machine::Value::Integer(value)) => *value != 0,
        | Operand::Value(abstract_machine::Value::Floating(real)) => !real.is_zero(),
        | Operand::Value(abstract_machine::Value::Pointer { target, .. }) => target.is_some(),
        // A pointer that points to a function is not null (C99 §6.3.2.3p3).
        | Operand::Value(abstract_machine::Value::Function(_)) => true,
        | _ => false,
    })
}

/// A switch's step: its value, and where it sent control. A switch
/// compares its value with its labels rather than testing it for truth
/// (C99 §6.8.4.2p5), so its chips name the label its title names after
/// "jump to", `default`, or, when it skips its body, the body.
fn switched(event: &Event, value: Option<FlowPart>) -> Vec<Option<FlowPart>> {
    let (verb, target) = match event.title.split_once("jump to ") {
        | Some((_, label)) => ("jumps to", label.trim_matches('`').to_owned()),
        | None => ("skips", "its body".to_owned()),
    };
    vec![value, Some(arrow(verb)), Some(part("label", &target))]
}

fn arrow(verb: &str) -> FlowPart {
    part("arrow", verb)
}

fn operator_chip(operator: &str) -> FlowPart {
    part("operator", operator)
}

/// A char array's elements as character constants: `{'h','i','\\0'}`.
fn characters(elements: &[abstract_machine::Value]) -> String {
    let characters: Vec<String> = elements
        .iter()
        .map(|element| match element {
            | abstract_machine::Value::Integer(value) => super::character(*value),
            | _ => "?".to_owned(),
        })
        .collect();
    format!("{{{}}}", characters.join(","))
}

/// Output as a C string literal spells it: `"Ada\n"`.
fn quoted(bytes: &[u8]) -> String {
    let mut text = String::from("\"");
    for &byte in bytes {
        match byte {
            | b'\n' => text.push_str("\\n"),
            | b'\t' => text.push_str("\\t"),
            | b'"' => text.push_str("\\\""),
            | b'\\' => text.push_str("\\\\"),
            | 0x20..=0x7E => text.push(char::from(byte)),
            | _ => {
                let _ = write!(text, "\\x{byte:02x}");
            },
        }
    }
    text.push('"');
    text
}

fn typed(chip: FlowPart, ty: &str) -> FlowPart {
    FlowPart {
        ty: Some(ty.to_owned()),
        ..chip
    }
}
