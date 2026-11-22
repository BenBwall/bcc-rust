//! `do`, `continue` and `goto`: a do statement entering its body before
//! any test, a continue ending a pass through a loop's body, a goto's jump
//! to its label, and what a jump does to the lifetimes and scopes of the
//! objects of the blocks it leaves, enters and passes.
//!
//! C99: §6.8.5.2 paragraph 1, p. 136; PDF p. 148.
//! C99: §6.8.6.2 paragraph 2, p. 138; PDF p. 150.
//! C99: §6.8.6.1 paragraph 2, p. 137; PDF p. 149.
//! C99: §6.2.4 paragraph 5, p. 32; PDF p. 44.
//! C99: §6.8 paragraph 3, p. 131; PDF p. 143.

use std::fmt::Write;

use super::{
    Topic,
    explained,
    noted,
    snippet,
};
use crate::abstract_machine::{
    clauses,
    machine::Machine,
    model::{
        Entry,
        Event,
        EventKind,
        Goto,
        Instruction,
        Op,
        Program,
        SourceRange,
        Value,
    },
};

/// `one` when `count` is 1, else `many`.
const fn number<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

impl Machine {
    /// `do`: control enters the body without testing anything; the test
    /// comes after it.
    pub(in crate::abstract_machine) fn do_statement(
        &self,
        program: &Program,
        instruction: &Instruction,
    ) -> Event {
        // The test is the do statement's branch back to the body, which
        // starts just after this instruction.
        let head = self.activations.last().map_or(0, |frame| frame.pc);
        let test = self.activations.last().and_then(|frame| {
            program.functions[frame.function].code.iter().find(
                |instruction| matches!(instruction.op, Op::Branch(target, "do") if target == head),
            )
        });
        let condition = test.map_or_else(String::new, |test| {
            snippet(&program.source, test.range).into_owned()
        });
        noted(
            explained(
                Event::new(
                    EventKind::Jump,
                    instruction.range,
                    String::from("do: run the body before any test"),
                ),
                &format!(
                    "A do statement runs its body first and tests `{condition}` only after it, so \
                     the body runs at least once."
                ),
                &format!(
                    "A while or for statement tests its condition before each pass through its \
                     body, so its body may never run. A do statement evaluates its controlling \
                     expression after each pass instead: control goes straight into the body now, \
                     and `{condition}`, tested after the body, decides whether it runs again."
                ),
                &[clauses::DO_STATEMENT, clauses::ITERATION],
            ),
            Topic::Loops,
        )
    }

    /// `continue`: control left the body of the `statement` loop, `left`
    /// blocks in all, in the steps before, and now goes on to its
    /// continuation, written at `next`.
    pub(in crate::abstract_machine) fn continued(
        &self,
        program: &Program,
        range: SourceRange,
        statement: &str,
        next: Option<SourceRange>,
        left: usize,
    ) -> Event {
        let next = next.map(|next| snippet(&program.source, next).into_owned());
        let then = match (statement, &next) {
            | ("for", Some(clause)) if !self.continues_to_condition(program) => format!(
                "goes to the for statement's third clause, `{clause}`, and then to the condition"
            ),
            | ("do", Some(test)) => format!("goes to the test `{test}` after the body"),
            | (_, Some(condition)) => format!("goes straight to the condition `{condition}`"),
            | (_, None) => String::from(
                "starts the next pass: the for statement has no condition, which counts as true",
            ),
        };
        let blocks = if left == 1 {
            String::from(
                "The step before this one left the loop's body, as its closing brace would, \
                 ending the lifetimes of the objects declared in it; the next pass creates fresh \
                 ones.",
            )
        } else {
            format!(
                "The {left} steps before this one left the blocks the continue is in, innermost \
                 first and the loop's body last, as their closing braces would, ending the \
                 lifetimes of the objects declared in them; the next pass creates fresh ones."
            )
        };
        noted(
            explained(
                Event::new(
                    EventKind::Jump,
                    range,
                    String::from("continue: end this pass through the body"),
                ),
                &format!(
                    "`continue` skips the rest of this pass through the {statement} loop's body, \
                     so control {then}."
                ),
                &format!(
                    "A continue statement jumps to the end of the smallest loop body around it, \
                     as if to a label just before the body's closing brace: the rest of this pass \
                     is skipped, but unlike a break it does not leave the loop. {blocks}"
                ),
                &[clauses::CONTINUE, clauses::LOOP_BODY],
            ),
            Topic::Loops,
        )
    }

    /// Whether the `continue` that just ran jumped to a for statement's
    /// condition rather than to a third clause: the instruction it moved
    /// to is the condition's own branch target, the loop head.
    fn continues_to_condition(&self, program: &Program) -> bool {
        self.activations.last().is_some_and(|frame| {
            program.functions[frame.function]
                .code
                .iter()
                .any(|instruction| matches!(instruction.op, Op::Jump(head) if head == frame.pc))
        })
    }

    /// A step that left a block because a `goto` jumps out of it: the
    /// leave's own explanation, said of the goto. `block` is the left
    /// block's `Enter` instruction.
    pub(in crate::abstract_machine) fn left_by_goto(
        &self,
        event: Event,
        program: &Program,
        goto: &Goto,
        block: Option<usize>,
    ) -> Event {
        let which = block
            .and_then(|block| {
                self.activations
                    .last()
                    .and_then(|frame| program.functions[frame.function].code.get(block))
            })
            .map_or_else(
                || String::from("the block"),
                |enter| match program.source.get(enter.range.start..enter.range.end) {
                    | Some("{") => format!("the block that begins on line {}", enter.range.line),
                    | Some("for") => format!("the for statement on line {}", enter.range.line),
                    | _ => format!(
                        "the block of the statement on line {}, a block though it has no braces",
                        enter.range.line
                    ),
                },
            );
        let goto_text = format!("`goto {};`", goto.label);
        left_by(
            event,
            &format!("{goto_text} leaves {which}"),
            &format!(
                "The label `{}` is not inside {which}, so {goto_text} leaves it before it jumps, \
                 one block at a time, innermost first",
                goto.label
            ),
            clauses::GOTO,
        )
    }

    /// A goto's jump to its label, after leaving the blocks the label is
    /// not in: it entered the blocks around the label (`entered` their
    /// objects), passed the declarations of `passed` and went back before
    /// those of `unscoped`, in the innermost block that stayed open.
    pub(in crate::abstract_machine) fn goto_jump(
        &self,
        program: &Program,
        instruction: &Instruction,
        goto: &Goto,
        entered: &[usize],
        passed: &[usize],
        unscoped: &[usize],
    ) -> Event {
        let label = &goto.label;
        let line = goto.at.line;
        let backward = goto.at.start < instruction.range.start;
        let (direction, when) = if backward {
            ("back", "earlier")
        } else {
            ("ahead", "later")
        };
        let function = self.activations.last().map_or_else(String::new, |frame| {
            program.functions[frame.function].name.clone()
        });
        let mut text = format!(
            "A goto statement jumps unconditionally to the statement its label names. A label's \
             name belongs to the whole function, so a goto may jump to a label anywhere in \
             `{function}`, earlier or later: here it jumps {direction}, and execution goes on \
             from the statement after `{label}:`."
        );
        let mut cited = vec![clauses::GOTO, clauses::LABEL_SCOPE];
        let left = goto.from.len().saturating_sub(goto.common);
        if left > 0 {
            let _ = write!(
                text,
                " The {} before this one left the {} the label is not in, ending {} objects' \
                 lifetimes as {} closing {} would.",
                number(left, "step", &format!("{left} steps")),
                number(left, "block", "blocks"),
                number(left, "its", "their"),
                number(left, "its", "their"),
                number(left, "brace", "braces"),
            );
        }
        if !goto.landing.enter.is_empty() {
            text.push(' ');
            text.push_str(&self.entering_text(program, &goto.landing.enter, entered, "the label"));
            cited.push(clauses::AUTOMATIC_LIFETIME);
            cited.push(clauses::DECLARATION_REACHED);
        }
        for &id in passed {
            let object = &self.state.objects[id - 1];
            let name = &object.name;
            let at = self.declaration_line(id);
            let held = match object.value {
                | Value::Indeterminate => String::from("its value is still indeterminate"),
                | Value::Array(_) | Value::Struct { .. } => String::from("it keeps what it held"),
                | ref value => format!("it keeps the value it held, {}", self.short_value(value)),
            };
            let _ = write!(
                text,
                " The jump passes the declaration of `{name}`{at}: `{name}` already exists, since \
                 entry into its block created it, and its name is in scope at the label, but no \
                 initializer of that declaration runs, so {held}."
            );
        }
        for &id in unscoped {
            let name = &self.state.objects[id - 1].name;
            let at = self.declaration_line(id);
            let _ = write!(
                text,
                " The jump goes back before the declaration of `{name}`{at}. The object lives on, \
                 since its block has not ended, but its name is out of scope again until \
                 execution reaches that declaration, which then stores its initializer's value, \
                 or an indeterminate value if it has none, once more."
            );
        }
        if !passed.is_empty() || !unscoped.is_empty() {
            cited.push(clauses::DECLARATION_REACHED);
            cited.push(clauses::SCOPE_START);
        }
        cited.dedup();
        let mut event = noted(
            explained(
                Event::new(
                    EventKind::Jump,
                    instruction.range,
                    format!("goto `{label}`: jump {direction} to line {line}"),
                ),
                &format!(
                    "`goto {label};` jumps to the statement labeled `{label}:` on line {line}, \
                     {when} in `{function}`; execution goes on from there."
                ),
                &text,
                &cited,
            ),
            Topic::Jump,
        );
        event.objects = entered
            .iter()
            .chain(passed)
            .chain(unscoped)
            .copied()
            .collect();
        event
    }

    /// A switch's jump to a label nested in blocks of its body: its
    /// explanation, with what entering those blocks, `enter`, did to
    /// `entered`.
    pub(in crate::abstract_machine) fn entered_by_jump(
        &self,
        mut event: Event,
        program: &Program,
        enter: &[Entry],
        entered: &[usize],
    ) -> Event {
        event.explanation.push(' ');
        event
            .explanation
            .push_str(&self.entering_text(program, enter, entered, "the label"));
        event.clauses.push(clauses::AUTOMATIC_LIFETIME);
        event.clauses.push(clauses::DECLARATION_REACHED);
        event.objects.extend_from_slice(entered);
        event
    }

    /// What a jump into the blocks `enter`, to `target`, did: it began
    /// the lifetimes of `entered`, which hold indeterminate values, since
    /// the jump passed their initializers.
    fn entering_text(
        &self,
        program: &Program,
        enter: &[Entry],
        entered: &[usize],
        target: &str,
    ) -> String {
        let lines: Vec<String> = self.activations.last().map_or_else(Vec::new, |frame| {
            enter
                .iter()
                .filter_map(|entry| program.functions[frame.function].code.get(entry.block))
                .map(|block| format!("{}", block.range.line))
                .collect()
        });
        let blocks = match lines.as_slice() {
            | [line] => format!("the block that begins on line {line}"),
            | [rest @ .., last] => format!(
                "{} blocks, beginning on lines {} and {last}",
                lines.len(),
                rest.join(", ")
            ),
            | [] => String::from("a block"),
        };
        let mut text = format!(
            "{} is inside {blocks}, which the jump enters from outside, just as if execution had \
             come in at {} opening brace.",
            super::sentence_case(target),
            number(enter.len(), "its", "their"),
        );
        if entered.is_empty() {
            text.push_str(" No objects are declared there, so no lifetimes begin.");
            return text;
        }
        let names = self.names(entered);
        let declared: Vec<usize> = entered
            .iter()
            .copied()
            .filter(|&id| self.state.objects[id - 1].declared)
            .collect();
        let _ = write!(
            text,
            " Entering a block begins the lifetimes of its objects however execution gets in, so \
             {names} {} to exist here, indeterminate: the jump passed {} declaration{}, so no \
             initializer ran. C forbids such a jump only into the scope of a variable-length \
             array, which this machine leaves out.",
            number(entered.len(), "begins", "begin"),
            number(entered.len(), "its", "their"),
            number(entered.len(), "", "s"),
        );
        if !declared.is_empty() {
            let _ = write!(
                text,
                " {} declared before {target}, so {} in scope there and can be assigned; reading \
                 {} first would read an indeterminate value.",
                sentence(&self.names(&declared)),
                number(declared.len(), "its name is", "their names are"),
                number(declared.len(), "it", "one"),
            );
        }
        text
    }

    /// " (line 5)": where the declaration of object `id` names it.
    fn declaration_line(&self, id: usize) -> String {
        match self.metadata[id - 1].line {
            | 0 => String::new(),
            | line => format!(" on line {line}"),
        }
    }

    /// Reaching the declaration of `name` again, after a goto jumped back
    /// over it while its block went on: the same object, which now holds
    /// `before`, gets its initializer's value again or an indeterminate
    /// value (C99 §6.2.4p5).
    pub(in crate::abstract_machine) fn declared_again(
        &self,
        mut event: Event,
        name: &str,
        before: &Value,
        initialized: bool,
    ) -> Event {
        let held = match before {
            | Value::Indeterminate => String::from("it held no value yet"),
            | Value::Array(_) | Value::Struct { .. } =>
                String::from("it held what was stored in it"),
            | value => format!("it held {}", self.short_value(value)),
        };
        let (then, result) = if initialized {
            (
                "its initializer stores a value again",
                "so its initializer runs again and replaces what it held",
            )
        } else {
            (
                "with no initializer its value becomes indeterminate again",
                "and without one it stores an indeterminate value each time, so the value it held \
                 is gone",
            )
        };
        event.summary = format!(
            "A goto went back over this declaration, so `{name}` is declared here again: its name \
             can be used from now on, and {then}."
        );
        let _ = write!(
            event.explanation,
            " Execution reached this declaration before, in this same run of its block, and a \
             goto jumped back over it: `{name}` is still the same object, alive since its block \
             began, and {held}. C performs a declaration's initialization each time execution \
             reaches it, {result}."
        );
        event
    }
}

/// `text` as the start of a sentence: capitalized, followed by " is" or
/// " are" by the names' number.
fn sentence(names: &str) -> String {
    let verb = if names.contains(" and ") || names.contains("objects named") {
        "are"
    } else {
        "is"
    };
    format!("{names} {verb}")
}

/// A `continue` left a block: the leave's own explanation, said of the
/// statement.
pub(in crate::abstract_machine) fn left_by_continue(event: Event) -> Event {
    left_by(
        event,
        "`continue` leaves the block",
        "The continue leaves this block on its way to the end of the loop's body",
        clauses::CONTINUE,
    )
}

/// A leave's explanation, `event`, said of the jump that left the block:
/// `lead` replaces "The block ends" and `intro` "The block finishes".
fn left_by(
    mut event: Event,
    lead: &str,
    intro: &str,
    clause: crate::abstract_machine::Clause,
) -> Event {
    event.summary = event.summary.replacen("The block ends", lead, 1);
    event.explanation = if event.explanation.starts_with("The block finishes, so ") {
        event.explanation.replacen(
            "The block finishes, so ",
            &format!("{intro}. With the block left, "),
            1,
        )
    } else {
        event.explanation.replacen("The block finishes", intro, 1)
    };
    if !event.clauses.contains(&clause) {
        event.clauses.push(clause);
    }
    event
}

impl Machine {
    /// `event`, a name's designation of object `id`, with which of the
    /// objects of that name the use reaches and why, when more than one is
    /// alive here: an inner declaration hides an outer one for the rest of
    /// its block (C99 §6.2.1p4), and an inner object its block has created
    /// hides nothing until its declaration, where its scope begins (C99
    /// §6.2.1p7).
    pub(in crate::abstract_machine) fn hiding(&self, mut event: Event, id: usize) -> Event {
        let chosen = &self.state.objects[id - 1];
        let Some(frame) = self.activations.last() else {
            return event;
        };
        // The other objects of the name this use could mean: the call's
        // own, and the file's.
        let others: Vec<&crate::abstract_machine::Object> = self
            .state
            .objects
            .iter()
            .filter(|object| {
                object.id != id
                    && object.name == chosen.name
                    && object.lifetime == "alive"
                    && !object.literal
                    && (object.frame_id == frame.id
                        || (object.frame_id == 0 && object.function.is_none()))
            })
            .collect();
        if others.is_empty() {
            return event;
        }
        let name = &chosen.name;
        let here = self.where_declared(id);
        let mut text = String::new();
        let hidden: Vec<_> = others.iter().filter(|object| object.declared).collect();
        let pending: Vec<_> = others.iter().filter(|object| !object.declared).collect();
        for object in &hidden {
            let there = self.where_declared(object.id);
            let held = match object.value {
                | Value::Array(_) | Value::Struct { .. } => String::new(),
                | ref value => format!(", which still holds {}", self.short_value(value)),
            };
            let _ = write!(
                text,
                " Another object named {name} is alive here too, the {name} declared {there}. \
                 This `{name}` is the one declared {here}, in a block inside that one's scope: an \
                 inner declaration hides an outer one for the rest of its block, so this use \
                 reaches the inner object and not the outer one{held}."
            );
        }
        for object in &pending {
            let there = self.where_declared(object.id);
            let _ = write!(
                text,
                " The block's own {name}, declared {there}, already exists, since entering its \
                 block created it, but its name's scope begins only at its declaration, which \
                 execution has not reached: until then `{name}` still means the one declared \
                 {here}."
            );
        }
        event.explanation.push_str(&text);
        if !hidden.is_empty() {
            event.clauses.push(clauses::NAME_HIDING);
        }
        if !pending.is_empty() {
            event.clauses.push(clauses::SCOPE_START);
        }
        event.clauses.dedup();
        noted(event, Topic::Hiding)
    }

    /// Where object `id`'s declaration is, for telling objects of one
    /// name apart: "on line 4", "as a parameter of `f`", "at file scope,
    /// on line 1".
    fn where_declared(&self, id: usize) -> String {
        let object = &self.state.objects[id - 1];
        if object.parameter {
            return format!(
                "as a parameter of `{}`",
                self.function_name(object.frame_id)
            );
        }
        match (self.metadata[id - 1].line, object.frame_id) {
            | (0, _) => String::from("earlier"),
            | (line, 0) if object.function.is_none() => format!("at file scope, on line {line}"),
            | (line, _) => format!("on line {line}"),
        }
    }

    /// `event`, the return of `function`, when the pointer it returns
    /// points to `target`, one of the call's own automatic objects, which
    /// ended with it: the caller receives an indeterminate value.
    pub(in crate::abstract_machine) fn returned_dangling(
        &self,
        mut event: Event,
        function: &str,
        target: usize,
    ) -> Event {
        let name = &self.state.objects[target - 1].name;
        let _ = write!(
            event.explanation,
            " The pointer it returns points to `{name}`, an automatic object of this call of \
             `{function}`, and `{name}`'s lifetime ends now, with the call. When an object's \
             lifetime ends, the value of every pointer to it becomes indeterminate: the caller \
             receives a dangling pointer, and using it in any way, even storing or comparing it, \
             is undefined behavior, so the machine stops at its first use. Return a pointer to an \
             object that outlives the call instead: a static one, or one the caller passed in."
        );
        event.summary = format!(
            "`{function}` returns a pointer to its own `{name}`, which ends with the call, so the \
             caller receives a dangling pointer."
        );
        event.clauses.push(clauses::LIFETIME);
        event.objects.push(target);
        noted(event, Topic::Lifetime)
    }
}
