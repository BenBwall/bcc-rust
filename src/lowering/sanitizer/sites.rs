//! Where a report points, as the compiler bcc answers as would say.
//!
//! Clang reports an operation where its expression is (`Expr::getExprLoc`):
//! an operator's token, a member's name; anything else, a subscript and an
//! implicitly converted operand included, where it starts.
//!
//! GCC instruments most operations after gimplification, where an operation
//! whose value is the whole right side of an assignment to, or the
//! initialization of, a register (a local scalar whose address is never
//! taken), or a call's argument, is part of that statement and reports the
//! statement's location: the `=`, the declarator's name, or the call. Any
//! other operation is a temporary's statement at the operator's token, a
//! member access's `.` or `->` and a subscript's `[`. GCC's front end checks
//! shifts, divisions, array bounds and variable length array bounds as it
//! parses them, at the operator. Measured with GCC 13.3 and Clang 23.1.

use super::super::FunctionLowerer;
use crate::translation_phases::{
    SourceVectors,
    parsing::syntax::{
        BinaryOperator,
        Expression,
        ExpressionType,
    },
};

/// What a GCC statement an expression's value belongs to stores it in,
/// which decides whether a check of the expression reports there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::lowering) enum Root {
    /// A register, which the statement computes the value into: every
    /// operation reports at the statement.
    Register,
    /// Memory, stored from a temporary, which an operation reports at its
    /// operator: only a name, which has no location, reports at the
    /// statement.
    Memory,
    /// A call's argument, a temporary that the call's location is given:
    /// a call there reports at it too.
    Argument,
}

/// What encloses an expression where that changes how it is checked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::lowering) enum Parent<'tu> {
    /// The operand of `&`: a subscript may point one past the end.
    AddressOf,
    /// The left operand of an assignment.
    Assigned,
    /// The operand of `++` or `--`.
    Stepped,
    /// An operand of a subscript.
    Subscripted,
    /// The operand of a cast, which reports at the cast.
    Cast(&'tu Expression<'tu>),
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Where a check of the operation of `expression` reports.
    pub(in crate::lowering) fn report_site(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> SourceVectors {
        if !self.gcc_reports() {
            return clang_site(expression);
        }
        match self.ub.roots.get(&address(expression)) {
            | Some(&(root, Root::Register | Root::Argument)) => root,
            // A name has no location of its own in GCC: it reports at the
            // statement wherever the value goes.
            | Some(&(root, Root::Memory))
                if matches!(expression.kind, ExpressionType::Identifier(_)) =>
                root,
            | _ => operator_site(expression),
        }
    }

    /// Where a check of the operation lowering is completing reports, or
    /// `fallback` outside any expression.
    pub(in crate::lowering) fn current_site(&self, fallback: SourceVectors) -> SourceVectors {
        self.ub
            .site
            .map_or(fallback, |expression| self.report_site(expression))
    }

    /// Where a check GCC's front end makes as it parses the operation
    /// lowering is completing reports: at its operator, whatever statement
    /// it is part of.
    pub(in crate::lowering) fn front_end_site(&self, fallback: SourceVectors) -> SourceVectors {
        match self.ub.site {
            | Some(expression) if self.gcc_reports() => operator_site(expression),
            | Some(expression) => clang_site(expression),
            | None => fallback,
        }
    }

    /// GCC: an operation whose value `expression` gives, through parentheses
    /// and casts, reports at `site`, the statement it is the value of, as
    /// `root` says.
    pub(in crate::lowering) fn note_statement_site(
        &mut self,
        expression: &'tu Expression<'tu>,
        site: SourceVectors,
        root: Root,
    ) {
        if self.unit.sanitizer.options.checks.is_empty() || !self.gcc_reports() {
            return;
        }
        _ = self
            .ub
            .roots
            .insert(address(stripped(expression)), (site, root));
    }

    /// GCC: whether the object `binding` is a register of GCC's, which an
    /// assignment or initialization computes its value into directly: an
    /// automatic scalar whose address is never taken and that is not
    /// volatile. Any other object is stored from a temporary, which reports
    /// at the operator.
    pub(in crate::lowering) fn gcc_register(&self, binding: usize) -> bool {
        use crate::translation_phases::semantic_analysis::{
            BindingKind,
            Duration,
        };
        let data = self.unit.sema.bindings[binding];
        matches!(data.kind, BindingKind::Object | BindingKind::Parameter)
            && data.duration == Duration::Automatic
            && !self.unit.address_taken.contains(&binding)
            && !data.ty.qualifiers.contains(
                crate::translation_phases::parsing::declaration_syntax::TypeQualifiers::VOLATILE,
            )
            && self
                .repr(data.ty, data.name.source_vectors)
                .is_ok_and(super::super::types::Repr::is_scalar)
    }

    /// GCC: whether the assignment's left operand `target`, through
    /// parentheses, names a register ([`Self::gcc_register`]), or is a
    /// record, which GCC copies in one statement wherever it is.
    pub(in crate::lowering) fn assigns_register(&self, target: &'tu Expression<'tu>) -> bool {
        if self.info(target).is_ok_and(|info| {
            self.repr(info.ty, target.source_vectors)
                .is_ok_and(|representation| representation == super::super::types::Repr::Aggregate)
        }) {
            return true;
        }
        let mut target = target;
        while let ExpressionType::Parenthesized { expression } = target.kind {
            target = expression;
        }
        matches!(target.kind, ExpressionType::Identifier(_))
            && self
                .info(target)
                .ok()
                .and_then(|info| info.binding)
                .is_some_and(|binding| self.gcc_register(binding))
    }

    /// Clang: where an operand such as a builtin's or a call's argument
    /// reports. One that is implicitly converted is an implicit cast in
    /// Clang, which is where it starts.
    pub(in crate::lowering) fn operand_site(&self, operand: &'tu Expression<'tu>) -> SourceVectors {
        if self.conversions(operand).is_empty() {
            self.report_site(operand)
        } else {
            operand.source_vectors
        }
    }

    /// Where a check of the call `call` itself reports: GCC's call statement
    /// keeps the call's location whatever it assigns, except as another
    /// call's argument; Clang's is where the call starts too.
    pub(in crate::lowering) fn call_site(&self, call: &'tu Expression<'tu>) -> SourceVectors {
        match self.ub.roots.get(&address(call)) {
            | Some(&(site, Root::Argument)) if self.gcc_reports() => site,
            | _ => call.source_vectors,
        }
    }

    /// Records what encloses `expression`, through parentheses.
    pub(in crate::lowering) fn note_parent(
        &mut self,
        expression: &'tu Expression<'tu>,
        parent: Parent<'tu>,
    ) {
        if self.unit.sanitizer.options.checks.is_empty() {
            return;
        }
        // A cast's own operand converts; `&` and `=` see through
        // parentheses to the lvalue.
        let mut expression = expression;
        while !matches!(parent, Parent::Cast(_))
            && let ExpressionType::Parenthesized { expression: inner } = expression.kind
        {
            expression = inner;
        }
        _ = self.ub.parents.insert(address(expression), parent);
    }

    /// Whether a cast to `void` discards the value of `expression`, which is
    /// then never read: neither compiler checks a read it does not make.
    pub(in crate::lowering) fn discarded(&self, expression: &'tu Expression<'tu>) -> bool {
        matches!(self.parent(expression), Some(Parent::Cast(cast))
        if self.info(cast).is_ok_and(|info| {
            matches!(
                self.unit.sema.types.kind(info.ty),
                crate::translation_phases::semantic_analysis::TypeKind::Scalar(
                    crate::target::Scalar::Void
                )
            )
        }))
    }

    /// What encloses `expression`, if that was recorded.
    pub(in crate::lowering) fn parent(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Option<Parent<'tu>> {
        self.ub.parents.get(&address(expression)).copied()
    }
}

/// `expression` without the parentheses and casts around its operation.
fn stripped<'tu>(mut expression: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    loop {
        match expression.kind {
            | ExpressionType::Parenthesized { expression: inner }
            | ExpressionType::Cast {
                operand_expression: inner,
                ..
            } => expression = inner,
            | _ => return expression,
        }
    }
}

/// Clang's location of an expression.
fn clang_site(expression: &Expression<'_>) -> SourceVectors {
    match expression.kind {
        | ExpressionType::Binary {
            operator: BinaryOperator::Subscript,
            ..
        } => expression.source_vectors,
        | ExpressionType::Binary { .. } | ExpressionType::Unary { .. } => operator_site(expression),
        | ExpressionType::DirectMember { member, .. }
        | ExpressionType::IndirectMember { member, .. } => member.source_vectors,
        | _ => expression.source_vectors,
    }
}

/// An operator's token, or where the expression starts.
pub(in crate::lowering) fn operator_site(expression: &Expression<'_>) -> SourceVectors {
    match expression.kind {
        | ExpressionType::Binary { .. }
        | ExpressionType::Unary { .. }
        | ExpressionType::DirectMember { .. }
        | ExpressionType::IndirectMember { .. } => expression
            .operator_source_vectors
            .unwrap_or(expression.source_vectors),
        | _ => expression.source_vectors,
    }
}

/// The identity of a syntax node.
fn address(expression: &Expression<'_>) -> usize {
    std::ptr::from_ref(expression).addr()
}
