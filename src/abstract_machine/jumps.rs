//! `do`, `continue`, `goto` and the labels statements carry. A `do`
//! statement runs its body before its first test; `continue` ends a pass
//! through a loop's body early; `goto` jumps to a named label anywhere in
//! the function. A jump that leaves blocks ends the lifetimes of their
//! objects, as their closing braces would, and one that enters blocks
//! begins them, with values that stay indeterminate until something is
//! stored: the jump passes their initializers.
//!
//! C99: §6.8.5.2 paragraph 1, p. 136; PDF p. 148.
//! C99: §6.8.6.2 paragraphs 1-2, p. 138; PDF p. 150.
//! C99: §6.8.6.1 paragraphs 1-2, p. 137; PDF p. 149.
//! C99: §6.8.1 paragraph 3, p. 132; PDF p. 144.
//! C99: §6.2.1 paragraph 3, p. 29; PDF p. 41.
//! C99: §6.2.4 paragraph 5, p. 32; PDF p. 44.

use super::{
    clauses,
    expressions::combined,
    model::{
        Diagnostic,
        Entry,
        Goto,
        Landing,
        Op,
        SourceRange,
    },
    parser::{
        LabelSite,
        Parser,
        is_identifier,
    },
};

impl Parser<'_> {
    /// `do statement while (expression);`: the body first, then the test,
    /// whose branch jumps back to the body while the value is nonzero.
    pub(super) fn do_statement(&mut self) -> Result<(), Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        self.emit(Op::Do, keyword.range)?;
        let head = self.code.len();
        self.loop_body = true;
        self.open_breakable("do");
        self.control_body()?;
        if !self.take("while") {
            return Err(self.error("expected `while` and a condition after the body of a do"));
        }
        let test = self.code.len();
        let (range, node) = self.condition("do")?;
        self.emit_node(Op::Branch(head, "do"), range, node)?;
        self.expect(";")?;
        self.close_loop(test, Some(range));
        Ok(())
    }

    /// `continue;`: end this pass through the innermost loop's body. Each
    /// block opened inside the body, and the body itself, ends first, as
    /// at their closing braces; then control goes to the loop's
    /// continuation, which `close_loop` patches in.
    pub(super) fn continue_statement(&mut self) -> Result<(), Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        let Some(index) = self
            .breakables
            .iter()
            .rposition(|breakable| breakable.statement != "switch")
        else {
            return Err(Diagnostic::constraint(
                keyword.range,
                "a continue belongs inside a loop, whose current pass it ends (C99 §6.8.6.2p1)",
                &clauses::CONTINUE_PLACE,
            ));
        };
        let (open, statement) = (
            self.breakables[index].blocks,
            self.breakables[index].statement,
        );
        for _ in open..self.blocks.len() {
            self.emit(Op::Leave, keyword.range)?;
        }
        let jump = self.code.len();
        self.emit(
            Op::Continue {
                target: 0,
                statement,
                next: None,
            },
            keyword.range,
        )?;
        self.breakables[index].continues.push(jump);
        self.expect(";")
    }

    /// End the innermost loop: each `continue` in it jumps to
    /// `continuation`, its condition, third clause or test, written at
    /// `next`, and each `break` just past it.
    pub(super) fn close_loop(&mut self, continuation: usize, next: Option<SourceRange>) {
        if let Some(breakable) = self.breakables.last() {
            for &jump in &breakable.continues {
                if let Op::Continue {
                    target,
                    next: written,
                    ..
                } = &mut self.code[jump].op
                {
                    *target = continuation;
                    *written = next;
                }
            }
        }
        self.close_breakable();
    }

    /// Whether a named label, `name:`, starts here.
    pub(super) fn at_named_label(&self) -> bool {
        is_identifier(self.current().text) && self.peek(1) == ":"
    }

    /// A run of labels, `again:`, `case 2:` or `default:`, and the
    /// statement they label. Execution that reaches the run from the
    /// statement before it in a switch's body runs on past its case labels
    /// in a step of its own.
    pub(super) fn labeled_statement(&mut self) -> Result<(), Diagnostic> {
        let falls = self.switch_labelled();
        let mut past = None;
        let mut cases = false;
        loop {
            if self.at("case") || self.at("default") {
                self.case_label(falls, &mut past)?;
                cases = true;
            } else if self.at_named_label() {
                self.named_label()?;
            } else {
                break;
            }
        }
        if self.at("}") || self.at("") {
            // `break;` is advice only where a case label shows a switch
            // around it; elsewhere it may be a constraint violation itself
            // (C99 §6.8.6.3p1).
            return Err(Diagnostic::new(
                "syntax",
                self.current().range,
                if cases {
                    "a label is followed by a statement; write `break;` (or an empty statement, \
                     `;`) after the last label"
                } else {
                    "a label is followed by a statement; write an empty statement, `;`, after the \
                     last label"
                },
            ));
        }
        if self.at_declaration() {
            return Err(self.labelled_declaration());
        }
        self.statement()
    }

    /// `name:`: a label, whose name has function scope and names the
    /// statement after it for a `goto` anywhere in the function, before
    /// or after it (C99 §6.2.1p3).
    fn named_label(&mut self) -> Result<(), Diagnostic> {
        let name = self.current();
        self.position += 2;
        let range = combined(name.range, self.tokens[self.position - 1].range);
        if let Some(earlier) = self
            .labels
            .iter()
            .find(|label| label.name.text == name.text)
        {
            return Err(Diagnostic::constraint(
                name.range,
                &format!(
                    "the label `{}` is already on line {}: label names are unique within a \
                     function (C99 §6.8.1p3)",
                    name.text, earlier.range.line
                ),
                &clauses::LABEL_UNIQUE,
            ));
        }
        let path = self.landing(0).enter;
        self.labels.push(LabelSite {
            name,
            range,
            target: self.code.len(),
            path,
        });
        Ok(())
    }

    /// `goto name;`. Which blocks it leaves and enters depends on where
    /// the label is, which may come later, so `resolve_gotos` fills them
    /// in at the end of the function.
    pub(super) fn goto_statement(&mut self) -> Result<(), Diagnostic> {
        let keyword = self.current();
        self.position += 1;
        let name = self.current();
        if !is_identifier(name.text) {
            return Err(self.error("expected the name of a label after goto"));
        }
        self.position += 1;
        self.expect(";")?;
        let range = combined(keyword.range, self.tokens[self.position - 1].range);
        self.gotos.push((self.code.len(), name));
        self.emit(
            Op::Goto(Box::new(Goto {
                label:    String::from(name.text),
                at:       name.range,
                from:     self.blocks.clone(),
                common:   0,
                declared: 0,
                landing:  Landing {
                    target: 0,
                    enter:  Vec::new(),
                },
            })),
            range,
        )
    }

    /// Point each `goto` of the function just compiled at its label: the
    /// blocks open at both stay open, the goto leaves the others it is in
    /// and enters the others the label is in. A goto to a label that runs
    /// on into case labels lands past the step that shows falling through
    /// them, since it did not fall through.
    pub(super) fn resolve_gotos(&mut self) -> Result<(), Diagnostic> {
        let gotos = std::mem::take(&mut self.gotos);
        let labels = std::mem::take(&mut self.labels);
        for (index, name) in gotos {
            let Some(label) = labels.iter().find(|label| label.name.text == name.text) else {
                return Err(Diagnostic::constraint(
                    name.range,
                    &format!(
                        "this function has no label `{}`: a goto names a label of the function it \
                         is in (C99 §6.8.6.1p1)",
                        name.text
                    ),
                    &clauses::GOTO_CONSTRAINT,
                ));
            };
            let mut target = label.target;
            if matches!(
                self.code.get(target).map(|instruction| &instruction.op),
                Some(Op::PastLabels(_))
            ) {
                target += 1;
            }
            if let Op::Goto(goto) = &mut self.code[index].op {
                let common = goto
                    .from
                    .iter()
                    .zip(&label.path)
                    .take_while(|(block, entry)| **block == entry.block)
                    .count();
                goto.at = label.range;
                goto.common = common;
                goto.declared = common
                    .checked_sub(1)
                    .and_then(|innermost| label.path.get(innermost))
                    .map_or(0, |entry| entry.declared);
                goto.landing = Landing {
                    target,
                    enter: label.path[common..].to_vec(),
                };
            }
        }
        Ok(())
    }

    /// Where a jump to the next instruction lands from a place with
    /// `blocks` blocks open: the blocks open here beyond those, each with
    /// how many of its objects are declared so far.
    pub(super) fn landing(&self, blocks: usize) -> Landing {
        let enter = self
            .blocks
            .iter()
            .skip(blocks)
            .map(|&block| Entry {
                block,
                declared: match &self.code[block].op {
                    | Op::Enter { objects, .. } => objects.len(),
                    | _ => 0,
                },
            })
            .collect();
        Landing {
            target: self.code.len(),
            enter,
        }
    }
}
