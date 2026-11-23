//! `switch`, its `case` and `default` labels, and `break`. A switch
//! evaluates its controlling expression once, promotes it, and jumps to the
//! label whose constant matches it; execution then runs on through the
//! statements after that label, past any later labels, until a `break` or
//! the end of the body. The subset compiles a switch whose body is a block;
//! its labels may be anywhere inside it, nested in its blocks and
//! statements, and a jump to a nested one enters the blocks around it.
//!
//! C99: §6.8.4.2 paragraphs 1-5, p. 134; PDF p. 146.
//! C99: §6.8.6.3 paragraphs 1-2, p. 138; PDF p. 150.
//! C99: §6.8.1 paragraph 2, p. 131; PDF p. 143.

use super::{
    clauses,
    expressions::combined,
    model::{
        Case,
        Diagnostic,
        Op,
        Skipped,
        Type,
    },
    parser::{
        Breakable,
        Parser,
        SwitchLabels,
    },
};

impl Parser<'_> {
    /// `switch (expression) { labels and statements }`.
    pub(super) fn switch_statement(&mut self) -> Result<(), Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        self.expect("(")?;
        let expression = self.expression(0)?;
        let Type::Integer(integer) = self.expressions[expression].ty else {
            return Err(Diagnostic::constraint(
                self.expressions[expression].range,
                "a switch's controlling expression must have an integer type: its case labels are \
                 integers to compare it with (C99 §6.8.4.2p1)",
                &clauses::SWITCH_CONSTRAINT,
            ));
        };
        self.check_update(expression)?;
        let value = self.emit_expression(expression, false)?;
        self.expect(")")?;
        let range = combined(keyword.range, self.tokens[self.position - 1].range);
        let node = self.tree("switch", range, vec![value])?;
        let promoted = integer.promoted();
        let dispatch = self.code.len();
        self.emit_node(
            Op::Switch {
                cases: Vec::new(),
                default: None,
                end: 0,
                promoted,
            },
            range,
            node,
        )?;
        if !self.at("{") {
            return Err(self.error(
                "a switch body in this subset is a block, `{ … }`, with its case labels inside it",
            ));
        }
        // The body is a block, but one that declares nothing directly
        // (below), so entering it creates no object: no step enters it.
        self.position += 1;
        self.scope += 1;
        self.open_breakable("switch");
        self.switches.push(SwitchLabels {
            cases: Vec::new(),
            default: None,
            promoted,
            blocks: self.blocks.len(),
        });
        while !self.at("}") {
            if self.at("") {
                return Err(self.error("expected closing brace"));
            }
            if self.current().text.starts_with('#') {
                return Err(self.error(
                    "a directive inside a function is outside this subset; the machine runs \
                     #include <stdio.h> at file scope",
                ));
            }
            if self.at_declaration() && !self.at_named_label() {
                return Err(self.switch_declaration());
            }
            self.statement()?;
        }
        self.position += 1;
        let end = self.code.len();
        let labels = self.switches.pop().unwrap_or(SwitchLabels {
            cases: Vec::new(),
            default: None,
            promoted,
            blocks: 0,
        });
        let targets: Vec<usize> = labels
            .cases
            .iter()
            .map(|case| case.landing.target)
            .chain(labels.default.as_ref().map(|landing| landing.target))
            .collect();
        self.mark_skipped(&targets, end);
        self.close_breakable();
        self.prune_scope(self.scope);
        self.scope -= 1;
        if let Op::Switch {
            cases: slots,
            default: slot,
            end: exit,
            ..
        } = &mut self.code[dispatch].op
        {
            *slots = labels.cases;
            *slot = labels.default;
            *exit = end;
        }
        Ok(())
    }

    /// The error for a declaration directly in a switch's block, which
    /// the switch's jump passes.
    fn switch_declaration(&self) -> Diagnostic {
        self.error(
            "declare objects in a block of their own inside the switch, as in `case 1: { int x = \
             0; … }`: a switch jumps past declarations written directly in its body",
        )
    }

    /// The `case` or `default` label here, of the innermost switch, which
    /// a run of labels starting with `falls` follows: when execution falls
    /// to the run from the statement before it, its first case or default
    /// label adds the instruction `past` that shows it running on past
    /// them. A label anywhere in the switch's body, however deeply nested,
    /// is the switch's (C99 §6.8.4.2p2-3); its jump enters the blocks
    /// between.
    pub(super) fn case_label(
        &mut self,
        falls: bool,
        past: &mut Option<usize>,
    ) -> Result<(), Diagnostic> {
        let label = self.current();
        let Some(blocks) = self.switches.last().map(|switch| switch.blocks) else {
            return Err(Diagnostic::constraint(
                label.range,
                "a case or default label belongs inside a switch statement (C99 §6.8.1p2)",
                &clauses::LABEL_PLACE,
            ));
        };
        if falls && past.is_none() {
            *past = Some(self.code.len());
            self.emit(Op::PastLabels(0), label.range)?;
        }
        self.position += 1;
        let case = if label.text == "case" {
            let constant = self.expression(0)?;
            let written = self.expressions[constant].range;
            let value = self.constant_integer(constant, "a case label", &clauses::CASE_LABELS)?;
            Some((value, combined(label.range, written)))
        } else {
            None
        };
        let Some(switch) = self.switches.last() else {
            return Ok(());
        };
        match case {
            | Some((value, range)) => {
                // C99 §6.8.4.2p5: the constant converts to the promoted
                // type of the controlling expression.
                let value = switch.promoted.convert(value);
                if let Some(earlier) = switch.cases.iter().find(|case| case.value == value) {
                    return Err(Diagnostic::constraint(
                        range,
                        &format!(
                            "this case label has the value {value}, like the case label on line \
                             {}: no two case labels of one switch may have the same value (C99 \
                             §6.8.4.2p3)",
                            earlier.range.line
                        ),
                        &clauses::CASE_LABELS,
                    ));
                }
                self.expect(":")?;
                self.extend_past_labels(*past);
                let landing = self.landing(blocks);
                if let Some(switch) = self.switches.last_mut() {
                    switch.cases.push(Case {
                        value,
                        range,
                        landing,
                    });
                }
            },
            | None => {
                if switch.default.is_some() {
                    return Err(Diagnostic::constraint(
                        label.range,
                        "a switch has at most one default label (C99 §6.8.4.2p3)",
                        &clauses::CASE_LABELS,
                    ));
                }
                self.expect(":")?;
                self.extend_past_labels(*past);
                let landing = self.landing(blocks);
                if let Some(switch) = self.switches.last_mut() {
                    switch.default = Some(landing);
                }
            },
        }
        Ok(())
    }

    /// Whether the innermost switch has a label already, so that
    /// execution can reach a later one from the statement before it.
    pub(super) fn switch_labelled(&self) -> bool {
        self.switches
            .last()
            .is_some_and(|switch| !switch.cases.is_empty() || switch.default.is_some())
    }

    /// The error for a declaration right after a run of labels, where C
    /// wants a statement.
    pub(super) fn labelled_declaration(&self) -> Diagnostic {
        if self
            .switches
            .last()
            .is_some_and(|switch| switch.blocks == self.blocks.len())
        {
            return self.switch_declaration();
        }
        Diagnostic::new(
            "syntax",
            self.current().range,
            "a label is followed by a statement, not a declaration: put the declaration in a \
             block of its own, `{ … }`, or write an empty statement, `;`, after the label (C99 \
             §6.8.1p1)",
        )
    }

    /// Count the label just read in the run of labels' instruction `past`,
    /// and stretch its range to the label's `:`.
    fn extend_past_labels(&mut self, past: Option<usize>) {
        if let Some(past) = past {
            let colon = self.tokens[self.position - 1].range;
            let instruction = &mut self.code[past];
            instruction.range = combined(instruction.range, colon);
            if let Op::PastLabels(count) = &mut instruction.op {
                *count += 1;
            }
        }
    }

    /// Record in each `break` of the switch being closed, whose body ends
    /// at instruction `end` and whose labels are at `targets`, what it
    /// skips: the later labels, other statements, or nothing.
    fn mark_skipped(&mut self, targets: &[usize], end: usize) {
        let Some(breakable) = self.breakables.last_mut() else {
            return;
        };
        let breaks = std::mem::take(&mut breakable.breaks);
        for &jump in &breaks {
            // What runs next without the break: past the ends of the blocks
            // it is in, and past an else branch that an if's first branch
            // jumps over.
            let mut next = jump + 1;
            while next < end {
                match self.code[next].op {
                    | Op::Leave => next += 1,
                    | Op::Jump(over) if over > next => next = over,
                    | _ => break,
                }
            }
            let later = targets.iter().filter(|&&target| target > jump).count();
            let skipped = if later > 0 {
                Skipped::Labels(later)
            } else if next < end {
                Skipped::Statements
            } else {
                Skipped::Nothing
            };
            if let Op::Break(_, _, slot) = &mut self.code[jump].op {
                *slot = skipped;
            }
        }
        if let Some(breakable) = self.breakables.last_mut() {
            breakable.breaks = breaks;
        }
    }

    /// `break;`: leave the innermost switch or loop. Each block opened
    /// inside it since its body began ends first, so its objects' lifetimes
    /// end as they would at its closing brace.
    pub(super) fn break_statement(&mut self) -> Result<(), Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        let Some(breakable) = self.breakables.last() else {
            return Err(Diagnostic::constraint(
                keyword.range,
                "a break belongs inside a switch or a loop, which it leaves (C99 §6.8.6.3p1)",
                &clauses::BREAK_PLACE,
            ));
        };
        let (open, statement) = (breakable.blocks, breakable.statement);
        for _ in open..self.blocks.len() {
            self.emit(Op::Leave, keyword.range)?;
        }
        let jump = self.code.len();
        self.emit(Op::Break(0, statement, Skipped::Nothing), keyword.range)?;
        if let Some(breakable) = self.breakables.last_mut() {
            breakable.breaks.push(jump);
        }
        self.expect(";")
    }

    /// Begin a switch's or a loop's body, which a `break` leaves.
    pub(super) fn open_breakable(&mut self, statement: &'static str) {
        self.breakables.push(Breakable {
            statement,
            blocks: self.blocks.len(),
            breaks: Vec::new(),
            continues: Vec::new(),
        });
    }

    /// End the innermost switch's or loop's body: each `break` in it jumps
    /// here, just past the statement.
    pub(super) fn close_breakable(&mut self) {
        let end = self.code.len();
        if let Some(breakable) = self.breakables.pop() {
            for jump in breakable.breaks {
                if let Op::Break(target, ..) = &mut self.code[jump].op {
                    *target = end;
                }
            }
        }
    }
}
