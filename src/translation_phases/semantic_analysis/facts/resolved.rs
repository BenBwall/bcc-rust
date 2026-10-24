//! Bottom-up expression designations, call sites, member identities and GNU
//! block values. Each wrapper copies one child summary; statement completion
//! forwards one immediate labeled child. Queries never follow syntax chains.
//! C99: §6.5.1p5, p. 69; PDF p. 81; §6.5.2.2p6-7, pp. 71-72;
//! PDF pp. 83-84; §6.5.2.3p3-4, p. 73; PDF p. 85; §6.8.1p4,
//! p. 132; PDF p. 144. Generic selections and statement expressions are
//! C11/GNU extensions; builtin checking remains in the existing family
//! handlers.

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        Analyzer,
        BindingKind,
        ConversionKind,
        ExpressionInfo,
        SemanticTranslationUnit,
        TypeId,
        TypeKind,
    },
    control::ExprId,
    correspondence::EntityId,
};
use crate::{
    translation_phases::parsing::syntax::{
        BlockItem,
        Expression,
        ExpressionSlot,
        ExpressionType,
        Statement,
        StatementType,
        UnaryOperator,
    },
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// Dense designations and sparse operation records share expression ordinals.
#[derive(Debug)]
pub(crate) struct ResolvedFacts<'tu> {
    pub(crate) designations:          &'tu [Designation<'tu>],
    pub(crate) calls:                 ArenaMap<'tu, usize, CallFacts<'tu>>,
    pub(crate) members:               ArenaMap<'tu, usize, MemberFacts>,
    pub(crate) statement_expressions: ArenaMap<'tu, usize, StatementExpressionFacts<'tu>>,
    branch_operands:                  ArenaMap<'tu, usize, &'tu Expression<'tu>>,
    #[cfg(test)]
    pub(crate) work:                  [usize; 2],
}

/// The selected leaf and binding, excluding conversions on this outer node.
/// C99: §6.5.1p5, p. 69; PDF p. 81. C11/GNU selections preserve designation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Designation<'tu> {
    pub(crate) leaf:            &'tu Expression<'tu>,
    pub(crate) binding:         Option<usize>,
    pub(crate) converted_child: bool,
}

/// The checked callee kind, including the retained M4 operation instance.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Callee<'tu> {
    Direct {
        entity:  EntityId,
        binding: usize,
    },
    Indirect(ExprId),
    Builtin {
        binding:  Option<usize>,
        direct:   Option<EntityId>,
        /// None only for the explicit temporary M6/unknown-reserved bridge.
        instance: Option<&'tu super::builtin_instances::BuiltinCallFacts<'tu>>,
    },
}

/// Point-of-use shape, independent of later composite binding types.
/// C99: §6.5.2.2p6-7, pp. 71-72; PDF pp. 83-84.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallFacts<'tu> {
    pub(crate) callee:        Callee<'tu>,
    pub(crate) function_type: Option<TypeId>,
    pub(crate) fixed:         usize,
    pub(crate) prototype:     bool,
    pub(crate) variadic:      bool,
    pub(crate) arguments:     &'tu [ArgumentFacts],
}

/// An expression ordinal references its retained contextual conversion run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArgumentFacts {
    pub(crate) expression: ExprId,
    pub(crate) ty:         TypeId,
}

/// Owning record identity and checked access facts, without layout
/// recomputation. C99: §6.5.2.3p3-4, p. 73; PDF p. 85.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MemberFacts {
    pub(crate) tag:         usize,
    pub(crate) field:       usize,
    pub(crate) access_type: TypeId,
    pub(crate) alignment:   u32,
    pub(crate) bit_field:   Option<u32>,
}

/// The outer labeled tail executes before its converted value is consumed.
/// A nested compound final statement has no value tail. GNU extension: GCC
/// "Statement Exprs"; C99: §6.8.1p4, p. 132; PDF p. 144 (label execution).
#[derive(Debug, Clone, Copy)]
pub(crate) struct StatementExpressionFacts<'tu> {
    pub(crate) block:       &'tu Statement<'tu>,
    pub(crate) outer_tail:  Option<&'tu Statement<'tu>>,
    pub(crate) value_tail:  Option<&'tu Expression<'tu>>,
    pub(crate) prefix:      usize,
    pub(crate) result_type: TypeId,
}

pub(in crate::translation_phases::semantic_analysis) struct ResolvedBuilder<'tu, 's> {
    designations: ArenaVec<'tu, Designation<'tu>>,
    calls: ArenaMap<'tu, usize, CallFacts<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) members: ArenaMap<'tu, usize, MemberFacts>,
    statement_expressions: ArenaMap<'tu, usize, StatementExpressionFacts<'tu>>,
    branch_operands: ArenaMap<'tu, usize, &'tu Expression<'tu>>,
    tails: ArenaMap<'s, usize, Option<&'tu Expression<'tu>>>,
    conversions: ArenaMap<'s, usize, ConversionSummary>,
    implicit_callees: ArenaSet<'s, usize>,
    #[cfg(test)]
    work: [usize; 2],
}

#[derive(Clone, Copy)]
struct ConversionSummary {
    count: usize,
    kind:  ConversionKind,
    ty:    TypeId,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Retain the sole operand left by a branch-context simplification.
    /// C99: §6.5.13-§6.5.14, pp. 89-90; PDF pp. 101-102.
    pub(in crate::translation_phases::semantic_analysis) fn retain_branch_operand(
        &mut self,
        expression: &'tu Expression<'tu>,
        operand: &'tu Expression<'tu>,
    ) {
        _ = self
            .resolved
            .branch_operands
            .insert(identity(expression), operand);
    }

    /// C89/GNU implicit declarations must be installed before the identifier
    /// is typed. Forward call context through immediate transparent children
    /// during ordinary discovery; selection results are not available yet.
    /// C89: §3.3.2.2, p. 41; PDF p. 55; C99: §6.5.1p5, p. 69; PDF p. 81.
    pub(in crate::translation_phases::semantic_analysis) fn mark_implicit_callee(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) {
        _ = self.resolved.implicit_callees.insert(identity(expression));
    }

    pub(in crate::translation_phases::semantic_analysis) fn discover_implicit_callee(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) {
        if !self.resolved.implicit_callees.remove(&identity(expression)) {
            return;
        }
        match expression.kind {
            | ExpressionType::Parenthesized { expression } => self.mark_implicit_callee(expression),
            | ExpressionType::Unary {
                operator: UnaryOperator::Extension,
                operand_expression,
            } => self.mark_implicit_callee(operand_expression),
            | _ => self.implicit_function(expression),
        }
    }

    /// One child lookup per transparent edge; no suffix walks.
    /// C99: §6.5.1p5, p. 69; PDF p. 81.
    pub(in crate::translation_phases::semantic_analysis) fn retain_designation(
        &mut self,
        info: ExpressionInfo<'tu>,
    ) {
        let child = match info.expression.kind {
            | ExpressionType::Parenthesized { expression } => Some(expression),
            | ExpressionType::Unary {
                operator: UnaryOperator::Extension,
                operand_expression,
            } => Some(operand_expression),
            | _ => info.selected_expression,
        };
        let designation = child.map_or(
            Designation {
                leaf:            info.expression,
                binding:         info.binding,
                converted_child: false,
            },
            |child| {
                let mut result = self.designation(child);
                result.converted_child |= self.resolved.conversions.contains_key(&identity(child));
                result
            },
        );
        #[cfg(test)]
        {
            self.resolved.work[0] += 1 + usize::from(child.is_some());
        }
        self.resolved.designations.push(designation);
        if let ExpressionType::Call {
            function_expression,
            arguments,
        } = info.expression.kind
        {
            self.retain_call(info, function_expression, arguments.as_slice());
        } else if let ExpressionType::Builtin(syntax) = info.expression.kind {
            self.retain_keyword_call(info, syntax);
        }
    }

    /// Keyword forms join the same sparse call table without a named binding.
    pub(in crate::translation_phases::semantic_analysis) fn retain_keyword_call(
        &mut self,
        info: ExpressionInfo<'tu>,
        syntax: &'tu crate::translation_phases::parsing::Builtin<'tu>,
    ) {
        let Some(descriptor) = super::super::builtins::registry::keyword_descriptor(syntax.keyword)
            .filter(|descriptor| descriptor.operation.retains_instance())
        else {
            return;
        };
        let instance = self.checked_keyword_builtin(info, descriptor, syntax);
        _ = self.resolved.calls.insert(
            self.expressions.len(),
            CallFacts {
                callee:        Callee::Builtin {
                    binding:  None,
                    direct:   None,
                    instance: Some(instance),
                },
                function_type: None,
                fixed:         0,
                prototype:     false,
                variadic:      false,
                arguments:     &[],
            },
        );
    }

    pub(in crate::translation_phases::semantic_analysis) fn designation(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Designation<'tu> {
        self.expression_indices
            .get(&identity(expression))
            .and_then(|&index| self.resolved.designations.get(index))
            .copied()
            .unwrap_or(Designation {
                leaf:            expression,
                binding:         None,
                converted_child: false,
            })
    }

    /// Semantic-private selection query; consumers use retained operation
    /// facts.
    pub(in crate::translation_phases::semantic_analysis) fn selected_designator(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> &'tu Expression<'tu> {
        self.designation(expression).leaf
    }

    fn retain_call(
        &mut self,
        info: ExpressionInfo<'tu>,
        expression: &'tu Expression<'tu>,
        arguments: &[&'tu Expression<'tu>],
    ) {
        let mut designation = self.designation(expression);
        let selected = self.function_extensions.call(info.expression);
        if let Some(selected) = selected {
            designation.binding = Some(selected.binding);
        }
        let conversion = self.resolved.conversions.get(&identity(expression));
        let direct = conversion
            .is_some_and(|c| c.count == 1 && c.kind == ConversionKind::FunctionDecay)
            && !designation.converted_child
            && matches!(designation.leaf.kind, ExpressionType::Identifier(_))
            && designation
                .binding
                .is_some_and(|binding| self.bindings[binding].kind == BindingKind::Function);
        let callee = match designation.binding {
            | Some(binding) if self.bindings[binding].origin.is_intrinsic() => Callee::Builtin {
                binding:  Some(binding),
                direct:   direct
                    .then(|| self.correspondence.entity(binding, self.bindings[binding])),
                instance: self.bindings[binding]
                    .origin
                    .descriptor()
                    .filter(|descriptor| descriptor.operation.retains_instance())
                    .map(|descriptor| self.checked_named_builtin(info, descriptor, arguments)),
            },
            | Some(binding) if direct => Callee::Direct {
                entity: self.correspondence.entity(binding, self.bindings[binding]),
                binding,
            },
            | _ => Callee::Indirect(ExprId(self.expression_indices[&identity(expression)])),
        };
        let ty = selected.map_or_else(
            || self.expression_info(expression).ty,
            |selected| self.bindings[selected.binding].ty,
        );
        let function_type = match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Function { .. } => Some(ty),
            | TypeKind::Pointer(target)
                if matches!(self.types.nodes[target.index], TypeKind::Function { .. }) =>
                Some(target),
            | _ => None,
        };
        let (fixed, prototype, variadic) = match function_type.map(|ty| self.types.nodes[ty.index])
        {
            | Some(TypeKind::Function {
                parameters,
                prototype,
                variadic,
                ..
            }) => (
                if prototype {
                    parameters.len().min(arguments.len())
                } else {
                    arguments.len()
                },
                prototype,
                variadic,
            ),
            | _ => (0, false, false),
        };
        let arguments = self
            .types
            .tu
            .alloc_slice_fill_iter(arguments.iter().map(|&argument| {
                ArgumentFacts {
                    expression: ExprId(self.expression_indices[&identity(argument)]),
                    ty:         self
                        .resolved
                        .conversions
                        .get(&identity(argument))
                        .map_or(self.expression_info(argument).ty, |c| c.ty),
                }
            }));
        _ = self.resolved.calls.insert(
            self.expressions.len(),
            CallFacts {
                callee,
                function_type,
                fixed,
                prototype,
                variadic,
                arguments,
            },
        );
    }

    /// Forward only the immediate label's completed value tail; compound
    /// statements never forward their last item's value to an enclosing block.
    /// C99: §6.8.1p4, p. 132; PDF p. 144; GNU statement expressions.
    pub(in crate::translation_phases::semantic_analysis) fn retain_statement_tail(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        let tail = match statement.kind {
            | StatementType::Label(_, child)
            | StatementType::Case(_, child)
            | StatementType::Default(child) => self.statement_tail(child),
            | StatementType::CaseRange(range) => self.statement_tail(range.statement),
            | StatementType::Attributed(attributed) => self.statement_tail(attributed.statement),
            | StatementType::Expression(ExpressionSlot::Parsed(expression)) => Some(expression),
            | _ => None,
        };
        #[cfg(test)]
        {
            self.resolved.work[1] += 1;
        }
        _ = self.resolved.tails.insert(identity(statement), tail);
    }

    fn statement_tail(&self, statement: &'tu Statement<'tu>) -> Option<&'tu Expression<'tu>> {
        self.resolved
            .tails
            .get(&identity(statement))
            .copied()
            .flatten()
    }

    /// Block completion reads one immediate tail identity, regardless of
    /// labels.
    pub(in crate::translation_phases::semantic_analysis) fn statement_expression_tail(
        &self,
        block: &'tu Statement<'tu>,
    ) -> Option<(&'tu Statement<'tu>, &'tu Expression<'tu>)> {
        let StatementType::Compound { items } = block.kind else {
            return None;
        };
        let BlockItem::Statement(outer) = *items.last()? else {
            return None;
        };
        self.statement_tail(outer)
            .map(|expression| (outer, expression))
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain_statement_expression(
        &mut self,
        expression: &'tu Expression<'tu>,
        block: &'tu Statement<'tu>,
        result_type: TypeId,
    ) {
        let tail = self.statement_expression_tail(block);
        let count = match block.kind {
            | StatementType::Compound { items } => items.len(),
            | _ => 0,
        };
        _ = self.resolved.statement_expressions.insert(
            identity(expression),
            StatementExpressionFacts {
                block,
                outer_tail: tail.map(|(outer, _)| outer),
                value_tail: tail.map(|(_, value)| value),
                prefix: count - usize::from(tail.is_some()),
                result_type,
            },
        );
    }
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn branch_operand(
        &self,
        expression: &Expression<'tu>,
    ) -> Option<&'tu Expression<'tu>> {
        self.resolved
            .branch_operands
            .get(&identity(expression))
            .copied()
    }

    pub(crate) fn designation(&self, expression: &Expression<'tu>) -> Option<Designation<'tu>> {
        self.resolved
            .designations
            .get(self.expression_index(expression)?)
            .copied()
    }

    pub(crate) fn call_facts(&self, expression: &Expression<'tu>) -> Option<CallFacts<'tu>> {
        self.resolved
            .calls
            .get(&self.expression_index(expression)?)
            .copied()
    }

    pub(crate) fn member_facts(&self, expression: &Expression<'tu>) -> Option<MemberFacts> {
        let leaf = self.designation(expression)?.leaf;
        self.resolved
            .members
            .get(&self.expression_index(leaf)?)
            .copied()
    }

    pub(crate) fn statement_expression_facts(
        &self,
        expression: &Expression<'tu>,
    ) -> Option<StatementExpressionFacts<'tu>> {
        self.resolved
            .statement_expressions
            .get(&identity(expression))
            .copied()
    }
}

impl<'tu, 's> ResolvedBuilder<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) fn new(
        tu: &'tu Bump,
        scratch: &'s Bump,
    ) -> Self {
        Self {
            designations:          ArenaVec::new_in(tu),
            calls:                 ArenaMap::with_hasher_in(FxBuildHasher, tu),
            members:               ArenaMap::with_hasher_in(FxBuildHasher, tu),
            statement_expressions: ArenaMap::with_hasher_in(FxBuildHasher, tu),
            branch_operands:       ArenaMap::with_hasher_in(FxBuildHasher, tu),
            tails:                 ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            conversions:           ArenaMap::with_hasher_in(FxBuildHasher, scratch),
            implicit_callees:      ArenaSet::with_hasher_in(FxBuildHasher, scratch),
            #[cfg(test)]
            work:                  [0; 2],
        }
    }

    /// The call facts retained for expression ordinal `index`, if any.
    pub(in crate::translation_phases::semantic_analysis) fn call(
        &self,
        index: usize,
    ) -> Option<CallFacts<'tu>> {
        self.calls.get(&index).copied()
    }

    pub(in crate::translation_phases::semantic_analysis) fn argument_type(
        &self,
        expression: &Expression<'tu>,
        original: TypeId,
    ) -> TypeId {
        self.conversions
            .get(&identity(expression))
            .map_or(original, |c| c.ty)
    }

    pub(in crate::translation_phases::semantic_analysis) fn conversion(
        &mut self,
        expression: &Expression<'tu>,
        ty: TypeId,
        kind: ConversionKind,
    ) {
        let summary = self
            .conversions
            .entry(identity(expression))
            .or_insert(ConversionSummary { count: 0, ty, kind });
        summary.count += 1;
        summary.ty = ty;
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(self) -> ResolvedFacts<'tu> {
        ResolvedFacts {
            designations:          self.designations.leak(),
            calls:                 self.calls,
            members:               self.members,
            statement_expressions: self.statement_expressions,
            branch_operands:       self.branch_operands,
            #[cfg(test)]
            work:                  self.work,
        }
    }
}

fn identity<T>(value: &T) -> usize {
    std::ptr::from_ref(value).addr()
}
