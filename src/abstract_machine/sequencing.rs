//! The run-time half of C's sequencing rule: between two sequence points
//! an object may be modified at most once, and read then only to compute
//! the value stored. `check_update` refuses the clashes it can see in the
//! source, over variables named directly; this module decides the rest on
//! the objects the accesses actually reach, so `*p++ = *q++` runs and
//! `a[i] = i++` stops at the access that breaks the rule, whichever names
//! the program used to reach the object.
//!
//! Every read and store of a full expression is logged with the
//! expression-tree node that made it. Whether a sequence point separates
//! two of them follows from the tree alone: the first operand of `&&`, `||`
//! and `?:` and the left operand of a comma operator are complete before the
//! rest of their operator is evaluated; a call's arguments are complete
//! before its body runs, and its body before its value is used; an
//! operator's operands are evaluated before the operator computes its
//! value; and every other pair of operands is unsequenced. A store is a
//! side effect, which C completes only by the next sequence point, so a
//! store nested in an operand is not ordered before its enclosing operator.
//!
//! A call's body is separated from the expression that calls it, but C
//! leaves the order of a call and the other operands unspecified (C99
//! §6.5p3): when the call reads or writes an object that another operand
//! also accesses, and one of them writes it, the result depends on an order
//! the machine would have to choose, so it stops instead. Library calls
//! count as writes of standard output and reads of the strings they take.
//!
//! C99: §6.5 paragraphs 2-3, p. 67; PDF p. 79.
//! C99: §6.5.2.2 paragraph 10, p. 72; PDF p. 84.
//! C99: §6.5.16 paragraph 3, p. 91; PDF p. 103.
//! C99: Annex C paragraph 1, p. 439; PDF p. 451.

use super::{
    clauses,
    machine::Machine,
    model::{
        Diagnostic,
        ExpressionNode,
        Program,
        SourceRange,
        Undefined,
    },
};

/// The pseudo-object standard output, written by every library call: no
/// real object has ID 0.
pub(super) const STANDARD_OUTPUT: usize = 0;

/// One access to memory during a full expression: scalars `start` to
/// `start + count` of object `target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Access {
    pub target: usize,
    pub start:  i32,
    pub count:  i32,
    pub write:  bool,
    /// The expression-tree node whose operation made the access.
    pub node:   usize,
    /// Whether a call at `node` made it, in its body.
    pub call:   bool,
}

impl Access {
    const fn overlaps(&self, other: &Self) -> bool {
        self.target == other.target
            && self.start < other.start + other.count
            && other.start < self.start + self.count
    }
}

/// An access a call's body made to an object that existed before the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Touch {
    pub target: usize,
    pub start:  i32,
    pub count:  i32,
    pub write:  bool,
}

/// How two accesses of one full expression are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    /// A sequence point separates them, or one is an operand's value that
    /// its operator uses: C orders them.
    Sequenced,
    /// Nothing orders them.
    Unsequenced,
    /// A store's own operator reads the object it stores into to find
    /// where to store, as in `a[a[0]] = 1` with `a[0]` 0.
    LocatingRead,
}

impl Machine {
    /// Log an access by the innermost activation's operation at `node`, to
    /// scalars `start..start + count` of `target`, and stop if it clashes
    /// with an earlier one of the same full expression. `call` for an
    /// access a library call makes. Each enclosing call also records it,
    /// when the object existed before that call began.
    pub fn access(
        &mut self,
        program: &Program,
        node: Option<usize>,
        (target, start, count): (usize, i32, i32),
        write: bool,
        call: bool,
    ) -> Result<(), Diagnostic> {
        let touch = Touch {
            target,
            start,
            count,
            write,
        };
        for activation in self.activations.iter_mut().skip(1) {
            if target <= activation.watermark && !activation.footprint.contains(&touch) {
                activation.footprint.push(touch);
            }
        }
        let Some(node) = node else {
            return Ok(());
        };
        let access = Access {
            target,
            start,
            count,
            write,
            node,
            call,
        };
        self.log(program, access)
    }

    /// Log `access` in the innermost activation's full expression, after
    /// checking it against every earlier access there.
    pub fn log(&mut self, program: &Program, access: Access) -> Result<(), Diagnostic> {
        let top = self.activations.len() - 1;
        if self.activations[top].evaluation.is_none() {
            return Ok(());
        }
        let nodes = &program.functions[self.activations[top].function].nodes;
        for earlier in &self.activations[top].accesses {
            if !(earlier.write || access.write) || !earlier.overlaps(&access) {
                continue;
            }
            match order(nodes, earlier, &access) {
                | Order::Sequenced => {},
                | Order::Unsequenced if earlier.call || access.call =>
                    return Err(self.unspecified(program, nodes, earlier, &access)),
                | Order::Unsequenced =>
                    return Err(self.unsequenced(program, nodes, earlier, &access)),
                | Order::LocatingRead =>
                    return Err(self.locating(program, nodes, earlier, &access)),
            }
        }
        if !self.activations[top].accesses.contains(&access) {
            self.activations[top].accesses.push(access);
        }
        Ok(())
    }

    /// The source a node of the innermost activation's function covers.
    fn node_text<'p>(program: &'p Program, nodes: &[ExpressionNode], node: usize) -> &'p str {
        nodes
            .get(node)
            .and_then(|node| program.source.get(node.range.start..node.range.end))
            .unwrap_or("")
    }

    /// Where the access at `node` is in the source: for a call's body,
    /// the call. A stop points there rather than at the operation that
    /// found the clash, which for a call is its `return`.
    fn node_range(nodes: &[ExpressionNode], node: usize, fallback: SourceRange) -> SourceRange {
        nodes.get(node).map_or(fallback, |node| node.range)
    }

    /// The object an access reaches, in words.
    fn accessed(&self, access: &Access) -> String {
        if access.target == STANDARD_OUTPUT {
            String::from("standard output")
        } else {
            self.location(access.target, access.start)
        }
    }

    /// Two unsequenced accesses to one object, one a store: undefined.
    fn unsequenced(
        &self,
        program: &Program,
        nodes: &[ExpressionNode],
        earlier: &Access,
        later: &Access,
    ) -> Diagnostic {
        let object = self.accessed(later);
        let plain = object.trim_matches('`');
        let verb = |access: &Access| if access.write { "stores into" } else { "reads" };
        let participle = |access: &Access| if access.write { "stored into" } else { "read" };
        let (later_text, earlier_text) = (
            Self::node_text(program, nodes, later.node),
            Self::node_text(program, nodes, earlier.node),
        );
        // A bare name says only "x"; say what happens to it instead.
        let first = if later_text == plain {
            format!("{object} is {} here", participle(later))
        } else {
            format!("`{later_text}` {} {object}", verb(later))
        };
        let second = if earlier_text == plain {
            format!(
                "it is also {} elsewhere in the expression",
                participle(earlier)
            )
        } else {
            format!("`{earlier_text}` {} it too", verb(earlier))
        };
        let message = format!(
            "{first}, and {second}, with no sequence point between the two: between two sequence \
             points C99 lets an object be modified at most once, and read only to compute the \
             value stored there"
        );
        Diagnostic::ub(
            Self::node_range(nodes, later.node, self.state.source_range),
            &message,
            &clauses::UNSEQUENCED,
            Undefined::Unsequenced,
        )
    }

    /// A call and another operand that access one object, one writing it,
    /// in an order C leaves unspecified.
    fn unspecified(
        &self,
        program: &Program,
        nodes: &[ExpressionNode],
        earlier: &Access,
        later: &Access,
    ) -> Diagnostic {
        // The call first; of two calls, the later.
        let (call, other) = if later.call {
            (later, earlier)
        } else {
            (earlier, later)
        };
        let object = self.accessed(call);
        let verb = |access: &Access| match (access.write, access.target == STANDARD_OUTPUT) {
            | (true, true) => "writes to",
            | (true, false) => "stores into",
            | (false, _) => "reads",
        };
        let call_text = Self::node_text(program, nodes, call.node);
        let other_text = Self::node_text(program, nodes, other.node);
        let second = if other.call && other_text == call_text {
            format!("the other call `{other_text}` {} it too", verb(other))
        } else if other.call {
            format!("the call `{other_text}` {} it too", verb(other))
        } else {
            format!("another operand, `{other_text}`, {} it", verb(other))
        };
        let message = format!(
            "the call `{call_text}` {} {object}, and {second}; C99 leaves unspecified which of \
             the two is evaluated first, and the result depends on it, so compilers may differ: \
             this machine stops rather than choose an order for you",
            verb(call),
        );
        Diagnostic {
            citation: Some(clauses::EVALUATION_ORDER.citation),
            clause: Some(&clauses::EVALUATION_ORDER),
            ..Diagnostic::new(
                "unsupported",
                Self::node_range(nodes, later.node, self.state.source_range),
                &message,
            )
        }
    }

    /// A store whose own operand reads the object stored into, to find
    /// where to store.
    fn locating(
        &self,
        program: &Program,
        nodes: &[ExpressionNode],
        earlier: &Access,
        later: &Access,
    ) -> Diagnostic {
        let (store, read) = if later.write {
            (later, earlier)
        } else {
            (earlier, later)
        };
        let object = self.accessed(store);
        let message = format!(
            "`{}` stores into {object}, and `{}` reads {object} to find where to store: C99 lets \
             the old value be read only to compute the new one (§6.5p2), so this machine stops \
             here rather than guess what that permits",
            Self::node_text(program, nodes, store.node),
            Self::node_text(program, nodes, read.node),
        );
        Diagnostic {
            citation: Some(clauses::UNSEQUENCED.citation),
            clause: Some(&clauses::UNSEQUENCED),
            ..Diagnostic::new(
                "unsupported",
                Self::node_range(nodes, later.node, self.state.source_range),
                &message,
            )
        }
    }

    /// Fold the footprint of the call that just returned into the caller's
    /// full expression, as accesses of the call node `node`.
    pub fn returned_footprint(
        &mut self,
        program: &Program,
        node: usize,
        footprint: &[Touch],
    ) -> Result<(), Diagnostic> {
        for touch in footprint {
            self.log(
                program,
                Access {
                    target: touch.target,
                    start: touch.start,
                    count: touch.count,
                    write: touch.write,
                    node,
                    call: true,
                },
            )?;
        }
        Ok(())
    }
}

/// The parent of `node` in its expression tree.
fn parent(nodes: &[ExpressionNode], node: usize) -> Option<usize> {
    let root = nodes.get(node)?.root;
    (node + 1..=root).find(|&candidate| nodes[candidate].children.contains(&node))
}

/// `node` and its ancestors, up to the root.
fn ancestors(nodes: &[ExpressionNode], node: usize) -> Vec<usize> {
    let mut chain = vec![node];
    while let Some(next) = chain.last().and_then(|&last| parent(nodes, last)) {
        chain.push(next);
    }
    chain
}

/// Whether `node` is an operator whose first operand a sequence point ends:
/// `&&`, `||`, `?:` or a comma (C99 Annex C).
fn sequences_first(node: &ExpressionNode) -> bool {
    matches!(node.label.as_str(), "&&" | "||" | "?:" | ",")
}

/// Whether `node` is a call, whose arguments a sequence point ends.
fn is_call(node: &ExpressionNode) -> bool {
    node.label.ends_with("()")
}

/// Whether `node` stores into the object its first operand designates: an
/// assignment, a compound assignment, `++` or `--`.
fn stores(node: &ExpressionNode) -> bool {
    let label = node.label.as_str();
    label == "++"
        || label == "--"
        || (label.ends_with('=') && !matches!(label, "==" | "!=" | "<=" | ">="))
}

/// How the earlier access `a` and the later access `b` of one full
/// expression are ordered.
fn order(nodes: &[ExpressionNode], a: &Access, b: &Access) -> Order {
    if a.node == b.node {
        // One operation: `++` reads and stores, a call's body is its own.
        return Order::Sequenced;
    }
    let above_a = ancestors(nodes, a.node);
    let above_b = ancestors(nodes, b.node);
    // Two elements of one initializer list: C leaves the order of their
    // side effects unspecified (C99 §6.7.8p23).
    let Some(&common) = above_a.iter().find(|node| above_b.contains(node)) else {
        return Order::Unsequenced;
    };
    let operator = &nodes[common];
    if common == a.node || common == b.node {
        let (outer, inner, path) = if common == a.node {
            (a, b, &above_b)
        } else {
            (b, a, &above_a)
        };
        // A call's arguments come before its body.
        if outer.call {
            return Order::Sequenced;
        }
        // The nodes from the inner access up to the operator, exclusive.
        let below: Vec<usize> = path.iter().copied().take_while(|&n| n != common).collect();
        // A sequence point inside the operand ends the inner access before
        // the operand's value exists: the first operand of `&&`, `||`, `?:`
        // or a comma, an argument of a call, or a call's own body.
        let separated = inner.call
            || below.windows(2).any(|pair| {
                let (child, above) = (pair[0], pair[1]);
                let node = &nodes[above];
                is_call(node) || (sequences_first(node) && node.children.first() == Some(&child))
            });
        if separated {
            return Order::Sequenced;
        }
        // A store nested in the operand is a side effect, which C completes
        // only by the next sequence point, not before the operator's value.
        if inner.write {
            return Order::Unsequenced;
        }
        // The operand's value comes before the operator uses it. A store
        // may read the old value of what it stores into only to compute the
        // new one, not to find where to store.
        let first = below.last().copied();
        if outer.write
            && stores(operator)
            && first.is_some()
            && operator.children.first() == first.as_ref()
        {
            return Order::LocatingRead;
        }
        return Order::Sequenced;
    }
    if sequences_first(operator) {
        // Different operands of `&&`, `||` or a comma are separated; of
        // `?:`, the first is separated from the others, and the second and
        // third are never both evaluated.
        return Order::Sequenced;
    }
    Order::Unsequenced
}
