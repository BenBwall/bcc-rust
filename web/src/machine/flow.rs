//! What one step did, in the two forms the page draws besides the
//! narration: its value flow (the operands it took and the value or object it
//! produced, as chips joined by arrows) and its accesses (which objects it
//! read, wrote, created, ended or only designated), for the memory diagram.

use std::collections::HashMap;

use bcc_rust::abstract_machine::{
    self,
    Event,
    EventKind,
    Operand,
};

use crate::snapshot::{
    Access,
    FlowPart,
};

type Objects<'a> = HashMap<usize, &'a abstract_machine::Object>;

/// The chips for `event`. `operator` is the operator of the expression node
/// the step evaluated, and `exited` whether the step ended the program.
pub(super) fn flow(
    event: &Event,
    operator: Option<&str>,
    exited: bool,
    objects: &Objects<'_>,
) -> Vec<FlowPart> {
    let chips = Chips { objects };
    let first = event.operands.first();
    let second = event.operands.get(1);
    let result = event.result.as_ref();
    let operand = |operand: Option<&Operand>| operand.map(|operand| chips.operand(operand));
    let parts: Vec<Option<FlowPart>> = match event.kind {
        | EventKind::LvalueConversion => vec![operand(first), Some(arrow("read")), operand(result)],
        | EventKind::IntegerPromotion => vec![
            operand(first).map(|chip| typed(chip, "char")),
            Some(arrow("promote")),
            operand(result).map(|chip| typed(chip, "int")),
        ],
        | EventKind::ArrayDecay => vec![
            event.objects.first().map(|&id| chips.whole(id)),
            Some(arrow("decays to")),
            operand(result).map(|chip| typed(chip, "int *")),
        ],
        | EventKind::AddressOf => vec![
            result.and_then(|result| chips.pointee(result)),
            Some(arrow("address")),
            operand(result),
        ],
        | EventKind::Dereference => vec![operand(first), Some(arrow("*")), operand(result)],
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
        | EventKind::Initialize | EventKind::Assign => chips.store(event),
        | EventKind::Increment => vec![
            operand(first),
            Some(arrow("was")),
            operand(second),
            Some(arrow("now")),
            operand(result),
        ],
        | EventKind::Declare => vec![
            event.objects.first().map(|&id| chips.whole(id)),
            Some(arrow("holds")),
            Some(part("value", "?")),
        ],
        | EventKind::Constant => vec![operand(result).map(|chip| typed(chip, "int"))],
        | EventKind::Branch => chips.branch(first),
        | EventKind::Call => chips.call(event),
        | EventKind::Return => vec![
            operand(first),
            Some(arrow(if exited { "exit status" } else { "returned" })),
        ],
        | _ => Vec::new(),
    };
    parts.into_iter().flatten().collect()
}

/// How `event` used each object it touched.
pub(super) fn access(event: &Event) -> Vec<Access> {
    let at = |operand: Option<&Operand>, kind: &str| match operand {
        | Some(
            &(Operand::Address { target, offset }
            | Operand::Value(abstract_machine::Value::Pointer {
                target: Some(target),
                offset,
            })),
        ) => vec![Access {
            object: target,
            cell:   usize::try_from(offset).ok(),
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
        | EventKind::Initialize | EventKind::Assign | EventKind::Increment =>
            at(event.operands.first(), "write"),
        | EventKind::Declare | EventKind::Call => whole("create"),
        | EventKind::LeaveBlock | EventKind::Return => whole("end"),
        | EventKind::Designate | EventKind::Subscript | EventKind::Dereference =>
            at(event.result.as_ref(), "designate"),
        | EventKind::AddressOf | EventKind::ArrayDecay => at(event.result.as_ref(), "address"),
        | _ => Vec::new(),
    }
}

/// Builds chips that name objects as the page does (`values[1]`).
struct Chips<'a, 'o> {
    objects: &'a Objects<'o>,
}

impl Chips<'_, '_> {
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
            let ty = self
                .objects
                .get(id)
                .map_or("char", |object| object.r#type.as_str());
            vec![
                operand(value).map(|chip| typed(chip, "int")),
                Some(arrow("convert")),
                operand(stored).map(|chip| typed(chip, ty)),
                Some(arrow("store")),
                operand(target),
            ]
        } else {
            vec![operand(value), Some(arrow("store")), operand(target)]
        }
    }

    /// A condition and whether C counts it as true.
    fn branch(&self, condition: Option<&Operand>) -> Vec<Option<FlowPart>> {
        let truth = condition.is_some_and(|operand| match operand {
            | Operand::Value(abstract_machine::Value::Integer(value)) => *value != 0,
            | Operand::Value(abstract_machine::Value::Pointer { target, .. }) => target.is_some(),
            | _ => false,
        });
        vec![
            condition.map(|operand| self.operand(operand)),
            Some(arrow("counts as")),
            Some(part("label", if truth { "true" } else { "false" })),
        ]
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
            | Operand::Value(value) => part("value", &super::value_text(value, self.objects)),
            | &Operand::Address { target, offset } => FlowPart {
                object: self.name(target),
                ..part("object", &super::location(target, offset, self.objects))
            },
        }
    }

    /// The object a pointer value points to.
    fn pointee(&self, operand: &Operand) -> Option<FlowPart> {
        match operand {
            | &Operand::Value(abstract_machine::Value::Pointer {
                target: Some(target),
                offset,
            }) => Some(self.operand(&Operand::Address { target, offset })),
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

fn arrow(verb: &str) -> FlowPart {
    part("arrow", verb)
}

fn operator_chip(operator: &str) -> FlowPart {
    part("operator", operator)
}

fn typed(chip: FlowPart, ty: &str) -> FlowPart {
    FlowPart {
        ty: Some(ty.to_owned()),
        ..chip
    }
}
