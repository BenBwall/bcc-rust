//! Constant automatic aggregate initialization bypasses Clang's aggregate
//! expression visitor. Ordinary aggregate values, including member bases,
//! call arguments and return values, emit both conditional arms.
//! C99: §6.7.8p11-22, pp. 126-127; PDF pp. 138-139. CodeGen policy follows
//! Clang.

use super::super::{
    Analyzer,
    ConstantClass,
    Duration,
    InitOperation,
    TypeKind,
};
use crate::{
    translation_phases::parsing::{
        declaration_syntax::Initializer,
        syntax::{
            BinaryOperator,
            Expression,
            ExpressionType,
            StatementType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn record_aggregate_initializer_arm(
        &mut self,
        owner: &'tu Expression<'tu>,
    ) {
        let Some(binding) = self.functions.initializer else {
            return;
        };
        let data = self.bindings[binding];
        if data.duration != Duration::Automatic || !self.aggregate(data.ty) {
            return;
        }
        self.functions
            .emission
            .aggregate_initializer_arms
            .entry(binding)
            .or_insert_with(|| ArenaVec::new_in(self.scratch))
            .push(owner);
    }

    /// Decide only after the whole initializer has been typed and planned:
    /// one dynamic member prevents the automatic constant-store shortcut.
    /// C99: §6.7.8p13, p. 126; PDF p. 138.
    pub(in crate::translation_phases::semantic_analysis) fn finish_aggregate_initializer_emission(
        &mut self,
        binding: usize,
        initializer: &'tu Initializer<'tu>,
    ) {
        let Some(arms) = self
            .functions
            .emission
            .aggregate_initializer_arms
            .remove(&binding)
        else {
            return;
        };
        let root = Node::Initializer(initializer);
        let constant = self.constant_aggregate_initializer(root, false);
        if !constant && !self.constant_aggregate_initializer(root, true) {
            return;
        }
        for &owner in arms.iter() {
            _ = self
                .functions
                .emission
                .potential_aggregate_arms
                .insert(std::ptr::from_ref(owner).addr(), ());
            if constant {
                _ = self
                    .functions
                    .emission
                    .constant_aggregate_arms
                    .insert(std::ptr::from_ref(owner).addr(), ());
                self.complete_conditional_elision(owner);
            }
        }
    }

    /// A bounded capability scan over typed values and retained plans, not
    /// an integer evaluator. Potential mode accepts a pure unmodeled choice
    /// with a constant-capable arm, solely to limit optional storage. Only
    /// proven mode permits elision. Both modes memoize shared object plans.
    fn constant_aggregate_initializer(&mut self, root: Node<'tu>, potential: bool) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        let mut children = ArenaVec::new_in(self.scratch);
        work.push((root, false));
        while let Some((node, done)) = work.pop() {
            let key = (node.key(), potential);
            if self
                .functions
                .emission
                .aggregate_constants
                .get(&key)
                .is_some_and(Option::is_some)
            {
                continue;
            }
            if !done
                && self
                    .functions
                    .emission
                    .aggregate_constants
                    .contains_key(&key)
            {
                _ = self
                    .functions
                    .emission
                    .aggregate_constants
                    .insert(key, Some(false));
                continue;
            }
            children.clear();
            let (leaf, alternatives) =
                self.aggregate_constant_children(node, potential, &mut children);
            if let Some(value) = leaf {
                _ = self
                    .functions
                    .emission
                    .aggregate_constants
                    .insert(key, Some(value));
            } else if done {
                let eligible = |child: &Node<'tu>| {
                    self.functions
                        .emission
                        .aggregate_constants
                        .get(&(child.key(), potential))
                        == Some(&Some(true))
                };
                let value = if alternatives {
                    children.iter().any(eligible)
                } else {
                    children.iter().all(eligible)
                };
                _ = self
                    .functions
                    .emission
                    .aggregate_constants
                    .insert(key, Some(value));
            } else {
                _ = self
                    .functions
                    .emission
                    .aggregate_constants
                    .insert(key, None);
                work.push((node, true));
                work.extend(children.iter().rev().map(|&child| (child, false)));
            }
        }
        self.functions
            .emission
            .aggregate_constants
            .get(&(root.key(), potential))
            == Some(&Some(true))
    }

    fn aggregate_constant_children(
        &mut self,
        node: Node<'tu>,
        potential: bool,
        children: &mut ArenaVec<'_, Node<'tu>>,
    ) -> (Option<bool>, bool) {
        let Node::Expression(e) = node else {
            let Node::Initializer(initializer) = node else {
                unreachable!()
            };
            let Some(&plan) = self.initializers.plan(initializer) else {
                return (Some(false), false);
            };
            if plan
                .operations
                .iter()
                .any(|op| matches!(op, InitOperation::Unmodeled { .. }))
            {
                return (Some(false), false);
            }
            children.extend(plan.values.iter().map(|v| Node::Expression(v.expression)));
            return (None, false);
        };
        let info = self.expression_info(e);
        if !self.aggregate(info.ty) {
            return (
                Some(
                    (potential && self.potential_constant_condition(e))
                        || info.constant == ConstantClass::Address
                        || (info.constant == ConstantClass::Arithmetic
                            && (info.integer.is_some()
                                || info.floating.is_some()
                                || info.unfolded_binary128))
                        || (info.static_address
                            && matches!(
                                self.types.nodes[info.ty.index],
                                TypeKind::Array(..) | TypeKind::Function { .. }
                            )),
                ),
                false,
            );
        }
        match e.kind {
            | ExpressionType::Parenthesized { expression } =>
                children.push(Node::Expression(expression)),
            | ExpressionType::CompoundLiteral { initializer, .. } =>
                children.push(Node::Initializer(initializer)),
            | ExpressionType::Conditional(c) | ExpressionType::OmittedConditional(c) => {
                let Some(value) = self.folded_condition(c.condition_expression) else {
                    if potential && self.potential_constant_condition(c.condition_expression) {
                        children.extend([
                            Node::Expression(c.then_expression),
                            Node::Expression(c.else_expression),
                        ]);
                        return (None, true);
                    }
                    return (Some(false), false);
                };
                children.push(Node::Expression(if value {
                    c.then_expression
                } else {
                    c.else_expression
                }));
            },
            | ExpressionType::Binary {
                operator: BinaryOperator::Comma,
                left_expression,
                right_expression,
            } => {
                if !self.effects_pure(left_expression, super::super::EffectContext::Value) {
                    return (Some(false), false);
                }
                children.push(Node::Expression(right_expression));
            },
            | ExpressionType::Generic(_) | ExpressionType::Builtin(_) => {
                let Some(selected) = info.selected_expression else {
                    return (Some(false), false);
                };
                children.push(Node::Expression(selected));
            },
            | ExpressionType::StatementExpression(block) => {
                // A transparent value tail can carry the initializer; other
                // block execution needs the ordinary aggregate visitor.
                if !matches!(block.kind, StatementType::Compound { items } if items.len() == 1) {
                    return (Some(false), false);
                }
                let Some((_, tail)) = self.statement_expression_tail(block) else {
                    return (Some(false), false);
                };
                children.push(Node::Expression(tail));
            },
            | ExpressionType::Identifier(_) => {
                let initializer = info
                    .binding
                    .and_then(|b| self.functions.emission.constant_aggregates.get(&b))
                    .copied();
                let Some(initializer) = initializer else {
                    return (Some(false), false);
                };
                children.push(Node::Initializer(initializer));
            },
            | _ => return (Some(false), false),
        }
        (None, false)
    }
}

#[derive(Clone, Copy)]
enum Node<'tu> {
    Expression(&'tu Expression<'tu>),
    Initializer(&'tu Initializer<'tu>),
}
impl Node<'_> {
    fn key(self) -> usize {
        match self {
            | Self::Expression(e) => std::ptr::from_ref(e).addr(),
            | Self::Initializer(i) => std::ptr::from_ref(i).addr(),
        }
    }
}
