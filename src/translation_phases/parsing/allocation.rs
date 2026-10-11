//! Syntax allocation and propagation of expression recovery status.
//!
//! [`Parser::alloc_syntax`] and [`Parser::alloc_syntax_list`] retain immutable
//! nodes in the translation-unit arena and count them against the parser's
//! limits. [`Parser::store_expression`] records whether a child already needed
//! recovery. Allocation does not decide expression types or constant values.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: expression grammar, §6.5, pp. 67-94; PDF pp. 79-106. Retaining repaired
//! syntax serves the diagnostics of §5.1.1.3, p. 11; PDF p. 23.

use super::{
    Parser,
    syntax::{
        ConditionalExpression,
        Expression,
        ExpressionType,
    },
    syntax_log::TreeNode,
};
use crate::{
    translation_phases::SourceVectors,
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu> Parser<'_, 'tu, '_> {
    /// Allocates one node in the translation-unit arena and counts it toward
    /// the node limit.
    pub(super) fn alloc_syntax<T: TreeNode<'tu>>(&mut self, node: T) -> &'tu T {
        let node = &*self.tree.alloc(node);
        self.syntax_nodes = self
            .syntax_nodes
            .checked_add(1)
            .expect("syntax node count overflows usize");
        #[cfg(test)]
        self.syntax.record(node);
        node
    }

    /// Copies frame-retained nodes into one length-prefixed list in the
    /// translation-unit arena, counts them, and empties `nodes` for reuse.
    /// An empty list allocates nothing.
    pub(super) fn alloc_syntax_list<T: TreeNode<'tu> + Copy>(
        &mut self,
        nodes: &mut ArenaVec<'_, T>,
    ) -> ArenaList<'tu, T> {
        let list = ArenaList::copy_from_slice(self.tree, nodes);
        nodes.clear();
        self.syntax_nodes = self
            .syntax_nodes
            .checked_add(list.len())
            .expect("syntax node count overflows usize");
        #[cfg(test)]
        for node in list {
            self.syntax.record(node);
        }
        list
    }

    /// Stores the out-of-line part of a node, such as a conditional's three
    /// operands, in the translation-unit arena. It belongs to its node, so
    /// it is not counted as a node of its own.
    pub(super) fn alloc_syntax_part<T: Copy>(&self, part: T) -> &'tu T {
        self.tree.alloc(part)
    }

    pub(super) fn store_expression(
        &mut self,
        kind: ExpressionType<'tu>,
        source_vectors: SourceVectors,
        operator_source_vectors: Option<SourceVectors>,
        recovered: bool,
    ) -> &'tu Expression<'tu> {
        let recovered = recovered || Self::expression_children_recovered(&kind);
        self.alloc_syntax(Expression {
            kind,
            source_vectors,
            operator_source_vectors,
            recovered,
        })
    }

    /// Marks `expression` as recovered. Tree nodes never change, so a copy
    /// that says so takes its place; the copy is not a new node.
    pub(super) fn mark_expression_recovered(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> &'tu Expression<'tu> {
        if expression.recovered {
            return expression;
        }
        let marked = &*self.tree.alloc(Expression {
            recovered: true,
            ..*expression
        });
        #[cfg(test)]
        self.syntax.replace_expression(expression, marked);
        marked
    }

    fn expression_children_recovered(kind: &ExpressionType<'tu>) -> bool {
        let expression_recovered = |expression: &Expression<'_>| expression.recovered;
        match kind {
            | ExpressionType::StatementExpression(x) => x.recovered,
            | ExpressionType::Builtin(x) => x.recovered,
            | ExpressionType::OmittedConditional(x) =>
                x.condition_expression.recovered || x.else_expression.recovered,
            | ExpressionType::Parenthesized { expression }
            | ExpressionType::Unary {
                operand_expression: expression,
                ..
            }
            | ExpressionType::AlignofExpr(expression)
            | ExpressionType::SizeofExpr(expression) => expression_recovered(expression),
            | ExpressionType::Conditional(ConditionalExpression {
                condition_expression,
                then_expression,
                else_expression,
            }) =>
                expression_recovered(condition_expression)
                    || expression_recovered(then_expression)
                    || expression_recovered(else_expression),
            | ExpressionType::Binary {
                left_expression,
                right_expression,
                ..
            } => expression_recovered(left_expression) || expression_recovered(right_expression),
            | ExpressionType::Call {
                function_expression,
                arguments,
            } =>
                expression_recovered(function_expression)
                    || arguments
                        .iter()
                        .any(|argument| expression_recovered(argument)),
            | ExpressionType::DirectMember {
                base_expression, ..
            }
            | ExpressionType::IndirectMember {
                base_expression, ..
            } => expression_recovered(base_expression),
            | ExpressionType::CompoundLiteral {
                type_name,
                initializer,
            } => type_name.recovered || initializer.recovered,
            | ExpressionType::SizeofType(type_name) | ExpressionType::AlignofType(type_name) =>
                type_name.recovered,
            | ExpressionType::Countof(operand) => match operand {
                | super::modern::SyntaxOperand::Type(x) => x.recovered,
                | super::modern::SyntaxOperand::Expression(x) => x.recovered,
            },
            | ExpressionType::Generic(selection) =>
                (match selection.controlling {
                    | super::modern::SyntaxOperand::Type(x) => x.recovered,
                    | super::modern::SyntaxOperand::Expression(x) => x.recovered,
                }) || selection
                    .associations
                    .iter()
                    .any(|x| x.expression.recovered || x.type_name.is_some_and(|x| x.recovered)),
            | ExpressionType::Cast {
                target_type,
                operand_expression,
            } => target_type.recovered || expression_recovered(operand_expression),
            | ExpressionType::Error => true,
            | ExpressionType::LabelAddress(_)
            | ExpressionType::Boolean(_)
            | ExpressionType::Nullptr
            | ExpressionType::Identifier(..)
            | ExpressionType::Constant(..)
            | ExpressionType::StringLiteral(..) => false,
        }
    }
}
