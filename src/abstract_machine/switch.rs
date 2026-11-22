//! `switch`, its `case` and `default` labels, and `break`. A switch
//! evaluates its controlling expression once, promotes it, and jumps to the
//! label whose constant matches it; execution then runs on through the
//! statements after that label, past any later labels, until a `break` or
//! the end of the body. The subset compiles a switch whose body is a block
//! with its labels directly inside it, the form nearly every program uses.
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
        Type,
    },
    parser::{
        Breakable,
        Parser,
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
                "a switch body in this subset is a block, `{ … }`, with its case labels directly \
                 inside it",
            ));
        }
        // The body is a block, but one that declares nothing directly
        // (below), so entering it creates no object: no step enters it.
        self.position += 1;
        self.scope += 1;
        self.open_breakable("switch");
        let mut cases: Vec<Case> = Vec::new();
        let mut default = None;
        let mut labelled = false;
        while !self.at("}") {
            if self.at("") {
                return Err(self.error("expected closing brace"));
            }
            if self.at("case") {
                let label = self.current();
                self.position += 1;
                let constant = self.expression(0)?;
                let written = self.expressions[constant].range;
                let value =
                    self.constant_integer(constant, "a case label", &clauses::CASE_LABELS)?;
                // C99 §6.8.4.2p5: the constant converts to the promoted
                // type of the controlling expression.
                let value = promoted.convert(value);
                if let Some(earlier) = cases.iter().find(|case| case.value == value) {
                    return Err(Diagnostic::constraint(
                        combined(label.range, written),
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
                cases.push(Case {
                    value,
                    range: combined(label.range, written),
                    target: self.code.len(),
                });
                labelled = true;
                continue;
            }
            if self.at("default") {
                let label = self.current();
                self.position += 1;
                if default.is_some() {
                    return Err(Diagnostic::constraint(
                        label.range,
                        "a switch has at most one default label (C99 §6.8.4.2p3)",
                        &clauses::CASE_LABELS,
                    ));
                }
                self.expect(":")?;
                default = Some(self.code.len());
                labelled = true;
                continue;
            }
            if labelled && self.at("}") {
                break;
            }
            if self.current().text.starts_with('#') {
                return Err(self.error(
                    "a directive inside a function is outside this subset; the machine runs \
                     #include <stdio.h> at file scope",
                ));
            }
            if self.at_declaration() {
                return Err(self.error(
                    "declare objects in a block of their own inside the switch, as in `case 1: { \
                     int x = 0; … }`: a switch jumps past declarations written directly in its \
                     body",
                ));
            }
            self.statement()?;
            labelled = false;
        }
        if labelled {
            return Err(Diagnostic::new(
                "syntax",
                self.current().range,
                "a label is followed by a statement; write `break;` (or an empty statement, `;`) \
                 after the last label",
            ));
        }
        self.position += 1;
        let end = self.code.len();
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
            *slots = cases;
            *slot = default;
            *exit = end;
        }
        Ok(())
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
        self.emit(Op::Break(0, statement), keyword.range)?;
        if let Some(breakable) = self.breakables.last_mut() {
            breakable.breaks.push(jump);
        }
        self.expect(";")
    }

    /// The error for a `case` or `default` label anywhere but directly in
    /// a switch's body.
    pub(super) fn misplaced_label(&self) -> Diagnostic {
        if self
            .breakables
            .iter()
            .any(|breakable| breakable.statement == "switch")
        {
            return self.error(
                "a case or default label nested inside another statement of the switch's body is \
                 outside this subset; write each label directly in the switch's block",
            );
        }
        Diagnostic::constraint(
            self.current().range,
            "a case or default label belongs inside a switch statement (C99 §6.8.1p2)",
            &clauses::LABEL_PLACE,
        )
    }

    /// Begin a switch's or a loop's body, which a `break` leaves.
    pub(super) fn open_breakable(&mut self, statement: &'static str) {
        self.breakables.push(Breakable {
            statement,
            blocks: self.blocks.len(),
            breaks: Vec::new(),
        });
    }

    /// End the innermost switch's or loop's body: each `break` in it jumps
    /// here, just past the statement.
    pub(super) fn close_breakable(&mut self) {
        let end = self.code.len();
        if let Some(breakable) = self.breakables.pop() {
            for jump in breakable.breaks {
                if let Op::Break(target, _) = &mut self.code[jump].op {
                    *target = end;
                }
            }
        }
    }
}
