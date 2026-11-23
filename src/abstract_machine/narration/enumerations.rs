//! Enumerations and `switch`: declaring an enumeration's constants, which
//! creates nothing, using a constant, which reads nothing, a switch's jump
//! to the label that matches its value, and `break`.
//!
//! C99: §6.4.4.3 paragraph 2, p. 59; PDF p. 71.
//! C99: §6.7.2.2 paragraphs 3-4, p. 105; PDF p. 117.
//! C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146.
//! C99: §6.8.6.3 paragraph 2, p. 138; PDF p. 150.

use std::fmt::Write;

use super::{
    Topic,
    explained,
    noted,
    result,
    snippet,
};
use crate::abstract_machine::{
    clauses,
    integers::Integer,
    machine::Machine,
    model::{
        Case,
        Enumeration,
        Event,
        EventKind,
        Operand,
        Program,
        Skipped,
        SourceRange,
        Value,
    },
};

/// The constants `names`, listed: `RED`, `RED and GREEN`, `RED, GREEN and
/// BLUE`.
fn listed(names: &[String]) -> String {
    match names {
        | [] => String::new(),
        | [only] => only.clone(),
        | [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A count in words, for the small numbers an enumeration has.
fn count(number: usize) -> String {
    const WORDS: [&str; 11] = [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    WORDS
        .get(number)
        .map_or_else(|| format!("{number}"), |word| String::from(*word))
}

/// How a sentence names `enumeration`'s type: its spelling, `enum color`,
/// or for an enumeration with neither a tag nor a typedef name, "the
/// enumeration's type".
fn type_name(enumeration: &Enumeration) -> &str {
    if enumeration.name.is_empty() {
        "the enumeration's type"
    } else {
        &enumeration.name
    }
}

/// Which integer type `enumeration` is compatible with, and how GCC chose
/// it: "compatible with unsigned int: C lets …".
pub(super) fn compatible_text(enumeration: &Enumeration) -> String {
    let negative = enumeration.integer == Integer::Int;
    format!(
        "compatible with {}: C lets each implementation choose an integer type that holds every \
         constant, and GCC and Clang choose int when a constant is negative{}",
        enumeration.integer.spelling(),
        if negative {
            ", as one is here, and unsigned int otherwise"
        } else {
            " and unsigned int otherwise, as here, where none is"
        },
    )
}

/// What C promises of the range of `enumeration`'s type: only that it can
/// represent the values of its constants, "0 to 6" (C99 §6.7.2.2p4). An
/// object of the type may hold any other value of that type.
pub(super) fn promised_range(enumeration: &Enumeration) -> String {
    let values = enumeration.constants.iter().map(|constant| constant.value);
    match (values.clone().min(), values.max()) {
        | (Some(low), Some(high)) if low == high =>
            format!("C promises only that its type can represent its constant's value, {low}"),
        | (Some(low), Some(high)) => format!(
            "C promises only that its type can represent its constants' values, so at least {low} \
             to {high}"
        ),
        | _ => String::from("C promises only that its type can represent its constants' values"),
    }
}

/// How the constant `index` of `enumeration` got its value.
fn origin(enumeration: &Enumeration, index: usize) -> String {
    let constant = &enumeration.constants[index];
    match (&constant.written, index.checked_sub(1)) {
        | (Some(written), _) if written.trim() == format!("{}", constant.value) => format!(
            "{} = {} gives it the value {}",
            constant.name, written, constant.value
        ),
        | (Some(written), _) => format!(
            "{} = {written} gives it the value of that constant expression, {}",
            constant.name, constant.value
        ),
        | (None, Some(previous)) => format!(
            "{} has no =, so it is the previous constant, {}, plus 1: {}",
            constant.name, enumeration.constants[previous].name, constant.value
        ),
        | (None, None) => format!(
            "{} is the first constant and has no =, so it is 0",
            constant.name
        ),
    }
}

impl Machine {
    /// The declaration of enumeration `index`, at file scope before `main`
    /// or where execution reaches it in a block: it declares the
    /// constants, and creates and stores nothing.
    pub(in crate::abstract_machine) fn enumeration(
        &self,
        program: &Program,
        index: usize,
    ) -> Event {
        let enumeration = &self.enumerations[index];
        let range = enumeration.range;
        let constants: Vec<String> = enumeration
            .constants
            .iter()
            .map(|constant| format!("{} = {}", constant.name, constant.value))
            .collect();
        let names: Vec<String> = enumeration
            .constants
            .iter()
            .map(|constant| constant.name.clone())
            .collect();
        let typedef = snippet(&program.source, range).starts_with("typedef");
        let tagged = enumeration.tag.is_some();
        let subject = if tagged || typedef {
            format!("`{}`", enumeration.name)
        } else {
            String::from("This enumeration")
        };
        let number = enumeration.constants.len();
        let summary = format!(
            "{subject} declares {} constant{}, {}: names for int values, which are not objects \
             and take no storage.",
            count(number),
            if number == 1 { "" } else { "s" },
            listed(&constants),
        );
        let mut text = String::from(
            "An enumeration declares a type and a list of named constants, each an int. ",
        );
        let origins: Vec<String> = (0..number)
            .map(|constant| origin(enumeration, constant))
            .collect();
        let _ = write!(text, "{}. ", capitalized(&origins.join("; ")));
        let _ = write!(
            text,
            "The constants are not objects: nothing is created or stored here, they have no \
             address, and wherever the program writes {} the compiler puts the value instead. ",
            if number == 1 {
                String::from("the constant's name")
            } else {
                String::from("one of their names")
            }
        );
        if enumeration.function.is_some() {
            text.push_str(
                "Declared in a block, the constants can be used from here to the block's closing \
                 brace. ",
            );
        } else {
            text.push_str(
                "Declared at file scope, the constants can be used from here to the end of the \
                 file, in every function after it; the machine shows the declaration before main \
                 runs, since the compiler has finished with it by then. ",
            );
        }
        let _ = write!(
            text,
            "{} itself is {}, so an object of it holds {} bytes.",
            if enumeration.name.is_empty() {
                String::from("The enumeration's type")
            } else {
                format!("The type {}", enumeration.name)
            },
            compatible_text(enumeration),
            enumeration.integer.bytes()
        );
        let mut cited = vec![
            clauses::ENUMERATOR_VALUES,
            clauses::ENUMERATION_CONSTANT,
            clauses::ENUMERATED_TYPE,
            clauses::SCOPE_START,
        ];
        if typedef {
            let _ = write!(
                text,
                " The same declaration makes `{}` a typedef name for the enumeration; a typedef \
                 names a type and creates no object either.",
                program
                    .aliases
                    .iter()
                    .find(|alias| alias.line >= range.line && alias.line <= range.end_line)
                    .map_or(enumeration.name.as_str(), |alias| alias.name.as_str())
            );
            cited.push(clauses::TYPEDEF_NAME);
        }
        let title = if tagged || typedef {
            format!("Declare the constants of `{}`", enumeration.name)
        } else {
            format!("Declare the constants {}", listed(&names))
        };
        noted(
            explained(
                Event::new(EventKind::Enumeration, range, title),
                &summary,
                &text,
                &cited,
            ),
            Topic::Enumeration,
        )
    }

    /// The enumeration constant `constant` of enumeration `enumeration`
    /// evaluated to its value: no object is read.
    pub(in crate::abstract_machine) fn enumerator(
        &self,
        range: SourceRange,
        (enumeration, constant): (usize, usize),
        value: i128,
    ) -> Event {
        let declared = &self.enumerations[enumeration];
        let known = &declared.constants[constant];
        let (name, number) = (&known.name, known.value);
        let of = if declared.tag.is_some() || !declared.name.is_empty() {
            format!(" of {}", declared.name)
        } else {
            String::new()
        };
        let summary = format!(
            "`{name}` is an enumeration constant{of}: it stands for the int value {number}, so \
             nothing is read from memory."
        );
        let mut text = format!(
            "An enumeration constant is not an object, so there is nothing to read: the compiler \
             puts {name}'s value, {number}, here. It was fixed when the enumeration was declared \
             on line {}: {}. Its type is int, as every enumeration constant's is",
            declared.range.line,
            origin(declared, constant),
        );
        if declared.integer == Integer::Int {
            text.push('.');
        } else {
            let _ = write!(
                text,
                ", even though {} itself is compatible with {}.",
                type_name(declared),
                declared.integer.spelling()
            );
        }
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Constant,
                    range,
                    format!("Evaluate the enumeration constant {name}"),
                ),
                &summary,
                &text,
                &[clauses::ENUMERATION_CONSTANT, clauses::ENUMERATOR_VALUES],
            ),
            Topic::Enumeration,
        );
        event.result_type = Some(String::from("int"));
        result(event, Operand::Value(Value::Integer(value)))
    }

    /// A switch compared its controlling value, `promoted` after the
    /// integer promotions, which keep its value, with its case labels and
    /// jumped to `matched`, to its `default` label, or past its body.
    pub(in crate::abstract_machine) fn switched(
        program: &Program,
        range: SourceRange,
        promoted: i128,
        matched: Option<&Case>,
        default: bool,
    ) -> Event {
        let written = snippet(&program.source, range);
        let label = matched.map(|case| snippet(&program.source, case.range));
        let (title, short, outcome) = match (&label, default) {
            | (Some(label), _) => (
                format!("switch on {promoted}: jump to `{label}`"),
                format!("`{label}` matches, so control jumps to the statement after that label"),
                format!(
                    "The label `{label}`, on line {}, has the value {promoted}, which matches, so \
                     control jumps straight to the statement after it; everything before it in \
                     the body is skipped. From there execution runs on through the statements \
                     that follow, past any later labels, until a break leaves the switch or its \
                     body ends.",
                    matched.map_or(0, |case| case.range.line)
                ),
            ),
            | (None, true) => (
                format!("switch on {promoted}: no case matches, jump to `default`"),
                String::from("no case label matches, so control jumps to the `default` label"),
                format!(
                    "No case label has the value {promoted}, so control jumps to the statement \
                     after `default:`, and runs on from there until a break or the end of the \
                     body."
                ),
            ),
            | (None, false) => (
                format!("switch on {promoted}: no case matches, skip the body"),
                String::from(
                    "no case label matches and there is no default, so none of the body runs",
                ),
                format!(
                    "No case label has the value {promoted}, and the switch has no default label, \
                     so no part of its body runs: control continues after the switch."
                ),
            ),
        };
        // The range runs from `switch` to the closing parenthesis.
        let summary = format!("`{written}` has the value {promoted}: {short}.");
        let mut text = String::from(
            "A switch evaluates its controlling expression once and compares the value with the \
             constant of each of its case labels, each converted to the expression's promoted \
             type. ",
        );
        text.push_str(&outcome);
        let mut event = noted(
            explained(
                Event::new(EventKind::Branch, range, title),
                &summary,
                &text,
                &[
                    clauses::SWITCH_JUMP,
                    clauses::SWITCH_STATEMENT,
                    clauses::FULL_EXPRESSION,
                ],
            ),
            Topic::Switch,
        );
        // The controlling expression is a full expression (C99 §6.8p4).
        event.sequence_point = true;
        let value = Operand::Value(Value::Integer(promoted));
        event.operands.push(value.clone());
        result(event, value)
    }
}

/// `break`: control leaves the innermost switch or loop, `statement`,
/// skipping, in a switch, what its body holds after the break.
pub(in crate::abstract_machine) fn break_statement(
    range: SourceRange,
    statement: &str,
    skipped: Skipped,
) -> Event {
    let (title, summary, text, topic) = if statement == "switch" {
        let rest = match skipped {
            | Skipped::Labels(1) =>
                "and the statements after the break, including those under the later label, do not \
                 run. Without the break, execution would run on past that label into its \
                 statements.",
            | Skipped::Labels(_) =>
                "and the statements after the break, including those under the later labels, do \
                 not run. Without the break, execution would run on past the next label into its \
                 statements.",
            | Skipped::Statements =>
                "and the statements after the break, to the end of the body, do not run.",
            | Skipped::Nothing =>
                "Nothing in the body follows the break, so it skips nothing: reaching the body's \
                 closing brace would leave the switch the same way.",
        };
        (
            String::from("break: leave the switch"),
            String::from(
                "`break` ends the switch around it, so control continues just after the switch.",
            ),
            format!(
                "A break statement ends the smallest switch or loop that contains it. Here that \
                 is the switch: control jumps to just after its body{} {rest}",
                if skipped == Skipped::Nothing {
                    "."
                } else {
                    ","
                }
            ),
            Topic::Switch,
        )
    } else {
        (
            String::from("break: leave the loop"),
            format!(
                "`break` ends the {statement} loop around it, so control continues just after the \
                 loop."
            ),
            format!(
                "A break statement ends the smallest switch or loop that contains it. Here that \
                 is the {statement} loop: control jumps to just after it, without finishing this \
                 pass through the body and without testing the condition again."
            ),
            Topic::Condition,
        )
    };
    noted(
        explained(
            Event::new(EventKind::Jump, range, title),
            &summary,
            &text,
            &[clauses::BREAK],
        ),
        topic,
    )
}

/// Execution reached the `count` labels at `range` from the statement
/// before them, or, when `first` in a block, from the block's entry, so it
/// runs on past them: a label only marks a place a switch can jump to.
///
/// C99: §6.8.1 paragraph 4, p. 132; PDF p. 144.
pub(in crate::abstract_machine) fn past_labels(
    range: SourceRange,
    source: &str,
    count: usize,
    first: bool,
) -> Event {
    let labels = snippet(source, range);
    let (phrase, pronoun) = if count == 1 {
        ("the label", "it")
    } else {
        ("the labels", "them")
    };
    let before = if first {
        format!(
            "Execution has entered the block that `{labels}` begins, and no break left the switch"
        )
    } else {
        format!("The statement before `{labels}` has finished, and no break left the switch")
    };
    noted(
        explained(
            Event::new(EventKind::Jump, range, format!("Run on past `{labels}`")),
            &format!(
                "{before}, so execution runs on past {phrase} into the statements after {pronoun}."
            ),
            &format!(
                "A case or default label only marks a place the switch can jump to; it does not \
                 alter the flow of control. Execution that reaches {phrase} from {}, as here, \
                 continues across {pronoun} into the statements that follow, whatever value the \
                 switch compared. This is called falling through. A `break;` before {phrase} \
                 would have left the switch instead.",
                if first {
                    "the start of its block"
                } else {
                    "the statement before"
                }
            ),
            &[clauses::LABELS_FLOW, clauses::SWITCH_JUMP],
        ),
        Topic::Switch,
    )
}

/// `text` with its first letter in upper case.
fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}
