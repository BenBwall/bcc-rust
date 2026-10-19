//! Finalization of typed runtime bounds and saved-extent dependencies.
//! C99: §6.7.5.2p1,5, pp. 116-117; PDF pp. 128-129;
//! §6.5.3.4p2, p. 80; PDF p. 92; §6.5.6p8, p. 83; PDF p. 95.

use super::{
    super::{
        Analyzer,
        BinaryOperator,
        FxBuildHasher,
        TypeId,
        TypeKind,
        UnaryOperator,
    },
    BoundEvaluationSite,
    RuntimeBound,
    SizeExtent,
    SizeFacts,
    SizeFormula,
};
use crate::{
    translation_phases::parsing::syntax::ExpressionType,
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Finalize typed bound records after size formulas and expression records
    /// exist. Retain the checked integer type and `size_t` conversion; lowering
    /// projects the conversion to its extent representation.
    /// C99: §6.7.5.2p1,5, pp. 116-117; PDF pp. 128-129.
    pub(in crate::translation_phases::semantic_analysis) fn retain_runtime_bounds(
        &mut self,
        sizes: &SizeFacts<'tu>,
    ) {
        for query in self.size_query_sites.values_mut() {
            query.formula = sizes.formula(query.site.ty);
        }
        for (index, &expression) in self.types.runtime_bounds.iter().enumerate() {
            let mut dependencies = ArenaVec::new_in(self.scratch);
            let mut seen_bounds = ArenaSet::with_hasher_in(FxBuildHasher, self.scratch);
            let mut seen_formulas = ArenaSet::with_hasher_in(FxBuildHasher, self.scratch);
            let mut seen = ArenaMap::with_hasher_in(FxBuildHasher, self.scratch);
            let mut work = ArenaVec::new_in(self.scratch);
            work.push(expression);
            while let Some(expression) = work.pop() {
                if seen
                    .insert(std::ptr::from_ref(expression).addr(), ())
                    .is_some()
                {
                    continue;
                }
                // The producer follows immediate checked expression children.
                // Size queries and pointer operations consume saved extents.
                let info = self.expression_info(expression);
                let mut include_type = |ty: TypeId| {
                    if let Some(mut formula) = sizes.formula(ty) {
                        while seen_formulas.insert(formula) {
                            let SizeFormula::Product { inner, extent } = sizes.formulas[formula.0]
                            else {
                                break;
                            };
                            if let SizeExtent::Bound(bound) = extent
                                && bound.0 < index
                                && seen_bounds.insert(bound)
                            {
                                dependencies.push(bound);
                            }
                            formula = inner;
                        }
                    }
                };
                if let Some(query) = self
                    .size_query_sites
                    .get(&std::ptr::from_ref(expression).addr())
                {
                    include_type(query.site.ty);
                    if let Some(operand) = query.operand {
                        work.push(operand);
                    }
                    work.extend_from_slice(query.site.operand_evaluations);
                } else {
                    if let Some(selected) = info.selected_expression {
                        work.push(selected);
                    }
                    if let Some(site) = self
                        .expression_actions
                        .get(&std::ptr::from_ref(expression).addr())
                    {
                        work.extend_from_slice(site.operand_evaluations);
                        for &bound in site.bounds {
                            if bound < index {
                                work.push(self.types.runtime_bounds[bound]);
                            }
                        }
                    }
                    match expression.kind {
                        | ExpressionType::StatementExpression(_) => {
                            if let (Some(&start), Some(&end)) = (
                                self.bound_expression_starts.get(
                                    &std::ptr::from_ref(self.types.runtime_bounds[index]).addr(),
                                ),
                                self.expression_indices
                                    .get(&std::ptr::from_ref(expression).addr()),
                            ) {
                                work.extend(
                                    self.expressions[start..end]
                                        .iter()
                                        .map(|info| info.expression),
                                );
                            }
                        },
                        | ExpressionType::Parenthesized { expression }
                        | ExpressionType::Cast {
                            operand_expression: expression,
                            ..
                        }
                        | ExpressionType::DirectMember {
                            base_expression: expression,
                            ..
                        }
                        | ExpressionType::IndirectMember {
                            base_expression: expression,
                            ..
                        } => work.push(expression),
                        | ExpressionType::Unary {
                            operator,
                            operand_expression,
                        } => {
                            if matches!(
                                operator,
                                UnaryOperator::PreIncrement
                                    | UnaryOperator::PreDecrement
                                    | UnaryOperator::PostIncrement
                                    | UnaryOperator::PostDecrement
                            ) && let TypeKind::Pointer(element) =
                                self.types.nodes[self.expression_info(operand_expression).ty.index]
                            {
                                include_type(element);
                            }
                            work.push(operand_expression);
                        },
                        | ExpressionType::Binary {
                            operator,
                            left_expression,
                            right_expression,
                        } => {
                            if matches!(
                                operator,
                                BinaryOperator::Addition
                                    | BinaryOperator::Subtraction
                                    | BinaryOperator::Subscript
                                    | BinaryOperator::AdditionAssignment
                                    | BinaryOperator::SubtractionAssignment
                            ) {
                                for child in [left_expression, right_expression] {
                                    if let TypeKind::Pointer(element)
                                    | TypeKind::Array(element, _) =
                                        self.types.nodes[self.expression_info(child).ty.index]
                                    {
                                        include_type(element);
                                    }
                                }
                            }
                            work.push(left_expression);
                            work.push(right_expression);
                        },
                        | ExpressionType::Conditional(c)
                        | ExpressionType::OmittedConditional(c) => {
                            work.push(c.condition_expression);
                            work.push(c.then_expression);
                            work.push(c.else_expression);
                        },
                        | ExpressionType::Call {
                            function_expression,
                            arguments,
                        } => {
                            work.push(function_expression);
                            work.extend_from_slice(arguments.as_slice());
                        },
                        | _ => {},
                    }
                }
            }
            self.runtime_bounds.push(RuntimeBound {
                expression,
                ty: self.expression_info(expression).ty,
                conversion: self.types.target.size_t,
                site: self
                    .bound_origins
                    .get(&index)
                    .copied()
                    .unwrap_or(BoundEvaluationSite::Unevaluated),
                dependencies: self.types.tu.alloc_slice_copy(&dependencies),
            });
        }
    }
}
