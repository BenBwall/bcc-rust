//! Retained expression effects and sparse effects-only evaluation recipes.
//! Each completed expression combines its immediate children's retained
//! summaries once per context, so production is O(1) per operand edge. A GNU
//! statement expression combines the statements of its body as they complete
//! and closes that summary with the block. Builtin calls take their effect
//! class and operand evaluation modes from the registry instance; generic and
//! choose-expression selections forward only the selected arm. ICE eligibility
//! remains a separate constant constraint, not a synonym for effect freedom.
//!
//! Lowering interprets recipes on its task stack. It never reclassifies
//! operators, volatility or selections to rediscover what must execute.
//!
//! C99: §5.1.2.3p2, p. 13; PDF p. 25 (side effects); §6.7.3p6, p. 109;
//! PDF p. 121 (volatile access); §6.5.3.4p2, p. 80; PDF p. 92 (unevaluated
//! and VLA sizeof operands); §6.5.13p4, p. 89; PDF p. 101; §6.5.14p4,
//! p. 89; PDF p. 101; §6.5.15p4, p. 90; PDF p. 102 (conditional
//! evaluation); §6.5.16p3, p. 91; PDF p. 103; §6.5.2.4p2, p. 75;
//! PDF p. 87 (stores). GNU extensions: GCC Statement Exprs, Typeof and
//! Integer Overflow Builtins.

use rustc_hash::FxBuildHasher;

use super::{
    super::{
        Analyzer,
        ArrayBound,
        ExpressionInfo,
        SemanticTranslationUnit,
        TypeKind,
    },
    control::ExprId,
    resolved::Callee,
};
use crate::{
    builtins::{
        ArgumentEvaluation,
        BuiltinEffectClass,
    },
    translation_phases::parsing::{
        SyntaxOperand,
        declaration_syntax::TypeQualifiers,
        syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionSlot,
            ExpressionType as E,
            ForInitializer,
            Statement,
            StatementType as S,
            UnaryOperator as U,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Summarizes both contexts of a completed expression from its immediate
    /// children. Called once per retained expression, before its ordinal is
    /// pushed; `retain_effect_recipes` follows once constant folding settles.
    pub(in crate::translation_phases::semantic_analysis) fn summarize_effects(
        &mut self,
        info: ExpressionInfo<'tu>,
    ) -> [EffectSet; 2] {
        let block = matches!(info.expression.kind, E::StatementExpression(_))
            .then(|| self.effects.closed.take().unwrap_or(EffectSet::UNKNOWN));
        let designation = self.context_summary(info, EffectContext::Designation, block);
        let value = if info.ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
            designation | EffectSet::VOLATILE
        } else {
            self.context_summary(info, EffectContext::Value, block)
        };
        [value, designation]
    }

    /// Records the recipes after the expression's final ICE state is known
    /// and pushes its dense record at the expression's ordinal.
    pub(in crate::translation_phases::semantic_analysis) fn retain_effect_recipes(
        &mut self,
        info: ExpressionInfo<'tu>,
        [value, designation]: [EffectSet; 2],
    ) -> ExpressionEffects {
        let index = self.expressions.len();
        let effects = ExpressionEffects {
            value:       ContextEffects {
                effects: value,
                recipe:  self.recipe(info, index, EffectContext::Value),
            },
            designation: ContextEffects {
                effects: designation,
                recipe:  self.recipe(info, index, EffectContext::Designation),
            },
        };
        debug_assert_eq!(
            self.effects.summaries.len(),
            index,
            "effect records stay parallel to expression ordinals"
        );
        self.effects.summaries.push(effects);
        effects
    }

    /// The structural summary for one context. Selections and wrappers
    /// forward the context; operands of operators are read or designated as
    /// their operator prescribes.
    fn context_summary(
        &mut self,
        info: ExpressionInfo<'tu>,
        context: EffectContext,
        block: Option<EffectSet>,
    ) -> EffectSet {
        use EffectContext::{
            Designation as D,
            Value as V,
        };
        #[cfg(test)]
        {
            self.effects.work += 1;
        }
        if info.ice && info.integer.is_some() {
            return EffectSet::empty();
        }
        let e = info.expression;
        let summary = match e.kind {
            // GNU extension: GCC "Labels as Values"; `&&label` is an
            // address constant. C23 `true`, `false` and `nullptr` are
            // constants. C23: §6.4.4.6p2-4, p. 66; PDF p. 79.
            | E::Identifier(_)
            | E::Constant(_)
            | E::StringLiteral(_)
            | E::LabelAddress(_)
            | E::Boolean(_)
            | E::Nullptr => EffectSet::empty(),
            | E::Countof(operand) => self.countof_effects(operand),
            | E::SizeofExpr(_) | E::SizeofType(_) | E::AlignofExpr(_) | E::AlignofType(_)
                if info.ice =>
                EffectSet::empty(),
            // C99 §6.5.3.4p2: a VLA operand is evaluated, with its new bounds.
            | E::SizeofExpr(_) | E::SizeofType(_) => self.size_query_effects(e),
            | E::Parenthesized { expression }
            | E::Cast {
                operand_expression: expression,
                ..
            } => self.child_effects(expression, context),
            | E::Unary {
                operator: U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
                operand_expression,
            } => EffectSet::WRITE | self.child_effects(operand_expression, V),
            | E::Binary {
                operator: B::Assignment,
                left_expression,
                right_expression,
            } => {
                let volatile = if self.volatile(left_expression) {
                    EffectSet::VOLATILE
                } else {
                    EffectSet::empty()
                };
                EffectSet::WRITE
                    | volatile
                    | self.child_effects(left_expression, D)
                    | self.child_effects(right_expression, V)
            },
            | E::Binary {
                operator:
                    B::MultiplicationAssignment
                    | B::DivisionAssignment
                    | B::ModuloAssignment
                    | B::AdditionAssignment
                    | B::SubtractionAssignment
                    | B::LeftShiftAssignment
                    | B::RightShiftAssignment
                    | B::BitwiseAndAssignment
                    | B::BitwiseXorAssignment
                    | B::BitwiseOrAssignment,
                left_expression,
                right_expression,
            } =>
                EffectSet::WRITE
                    | self.child_effects(left_expression, V)
                    | self.child_effects(right_expression, V),
            | E::Unary {
                operator,
                operand_expression,
            } => self.child_effects(
                operand_expression,
                if operator == U::AddressOf { D } else { V },
            ),
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => self.child_effects(left_expression, V) | self.child_effects(right_expression, V),
            | E::Conditional(c) | E::OmittedConditional(c) =>
                self.child_effects(c.condition_expression, V)
                    | self.child_effects(c.then_expression, V)
                    | self.child_effects(c.else_expression, V),
            | E::DirectMember {
                base_expression, ..
            } => self.child_effects(base_expression, D),
            | E::IndirectMember {
                base_expression, ..
            } => self.child_effects(base_expression, V),
            | E::Generic(_) | E::Builtin(_) if info.selected_expression.is_some() => info
                .selected_expression
                .map_or(EffectSet::UNKNOWN, |selected| {
                    self.child_effects(selected, context)
                }),
            | E::Call {
                function_expression,
                arguments,
            } => {
                let callee = self.child_effects(function_expression, V);
                match self.resolved.call(self.expressions.len()) {
                    | Some(call) => match call.callee {
                        | Callee::Builtin {
                            instance: Some(instance),
                            ..
                        } => callee | self.builtin_effects(instance),
                        | Callee::Builtin { instance: None, .. } =>
                            callee
                                | EffectSet::UNKNOWN
                                | self.arguments_effects(arguments.iter().copied()),
                        | Callee::Direct { .. } | Callee::Indirect(_) =>
                            callee
                                | EffectSet::CALL
                                | self.arguments_effects(arguments.iter().copied()),
                    },
                    | None =>
                        callee
                            | EffectSet::UNKNOWN
                            | self.arguments_effects(arguments.iter().copied()),
                }
            },
            | E::Builtin(_) => match self.resolved.call(self.expressions.len()) {
                | Some(super::CallFacts {
                    callee:
                        Callee::Builtin {
                            instance: Some(instance),
                            ..
                        },
                    ..
                }) => self.builtin_effects(instance),
                | _ => EffectSet::UNKNOWN,
            },
            | E::StatementExpression(_) => block.unwrap_or(EffectSet::UNKNOWN),
            // The initializer's effects are not summarized separately.
            | E::CompoundLiteral { .. } => EffectSet::WRITE | EffectSet::UNKNOWN,
            | E::AlignofExpr(_) | E::AlignofType(_) | E::Generic(_) | E::Error =>
                EffectSet::UNKNOWN,
        };
        summary | self.action_effects(e)
    }

    /// Cast type-site actions: evaluated typeof operands and new bounds.
    /// GNU extension: GCC Typeof; C99 §6.7.5.2p5, p. 117; PDF p. 129.
    fn action_effects(&self, e: &'tu Expression<'tu>) -> EffectSet {
        let Some(site) = self.expression_actions.get(&identity(e)) else {
            return EffectSet::empty();
        };
        self.site_effects(site.operand_evaluations, site.bounds)
    }

    /// C2y `_Countof` evaluates its operand only when the outermost array
    /// length is variable; otherwise the result is a constant. A type operand
    /// evaluates its newly written bounds. An operand that is not a known
    /// array stays unknown. C2y draft grammar from the GCC primary
    /// implementation (see `language-standards.md`).
    fn countof_effects(&self, operand: SyntaxOperand<'tu>) -> EffectSet {
        let (ty, evaluated) = match operand {
            | SyntaxOperand::Expression(e) => (
                self.expression_indices
                    .get(&identity(e))
                    .map(|&index| self.expressions[index].ty),
                self.child_effects(e, EffectContext::Designation),
            ),
            | SyntaxOperand::Type(name) => {
                let key = super::sizes::type_name_identity(name);
                (
                    self.resolved_type_names.get(&key).copied(),
                    self.type_name_sites
                        .get(&key)
                        .map_or(EffectSet::UNKNOWN, |site| {
                            self.site_effects(site.operand_evaluations, site.bounds)
                        }),
                )
            },
        };
        match ty.map(|ty| self.types.nodes[ty.index]) {
            | Some(TypeKind::Array(_, ArrayBound::Constant(_))) => EffectSet::empty(),
            | Some(TypeKind::Array(_, ArrayBound::Variable(_))) => evaluated,
            | _ => EffectSet::UNKNOWN,
        }
    }

    fn size_query_effects(&self, e: &'tu Expression<'tu>) -> EffectSet {
        let Some(query) = self.size_query_sites.get(&identity(e)) else {
            return EffectSet::UNKNOWN;
        };
        let operand = query.operand.map_or(EffectSet::empty(), |operand| {
            self.child_effects(operand, EffectContext::Designation)
        });
        operand | self.site_effects(query.site.operand_evaluations, query.site.bounds)
    }

    fn site_effects(&self, evaluations: &[&'tu Expression<'tu>], bounds: &[usize]) -> EffectSet {
        let mut effects = EffectSet::empty();
        for &operand in evaluations {
            effects |= self.child_effects(operand, EffectContext::Value);
        }
        for &bound in bounds {
            effects |= self.child_effects(self.types.runtime_bounds[bound], EffectContext::Value);
        }
        effects
    }

    fn arguments_effects(
        &self,
        arguments: impl IntoIterator<Item = &'tu Expression<'tu>>,
    ) -> EffectSet {
        arguments
            .into_iter()
            .fold(EffectSet::empty(), |effects, argument| {
                effects | self.child_effects(argument, EffectContext::Value)
            })
    }

    /// The registry's class plus the operands the checked instance evaluates.
    /// Unevaluated and type operands contribute nothing.
    fn builtin_effects(&self, instance: &super::BuiltinCallFacts<'tu>) -> EffectSet {
        let mut effects = match instance.effects {
            | BuiltinEffectClass::Pure | BuiltinEffectClass::Query => EffectSet::empty(),
            | BuiltinEffectClass::Write => EffectSet::WRITE,
            | BuiltinEffectClass::Control => EffectSet::CONTROL,
            | BuiltinEffectClass::Allocate => EffectSet::LIFETIME,
            | BuiltinEffectClass::Library => EffectSet::CALL,
        };
        for operand in instance.operands {
            let Some(expression) = operand.expression else {
                continue;
            };
            effects |= match operand.evaluation {
                | ArgumentEvaluation::Value | ArgumentEvaluation::EffectsOnly =>
                    self.child_effects(expression, EffectContext::Value),
                | ArgumentEvaluation::StorageAddress | ArgumentEvaluation::AdjustedPointer =>
                    self.child_effects(expression, EffectContext::Designation),
                | ArgumentEvaluation::Unevaluated | ArgumentEvaluation::Type => EffectSet::empty(),
            };
        }
        effects
    }

    fn volatile(&self, e: &'tu Expression<'tu>) -> bool {
        self.expression_indices
            .get(&identity(e))
            .is_some_and(|&index| {
                self.expressions[index]
                    .ty
                    .qualifiers
                    .contains(TypeQualifiers::VOLATILE)
            })
    }

    /// One retained child summary; a child without a record is unknown.
    fn child_effects(&self, e: &'tu Expression<'tu>, context: EffectContext) -> EffectSet {
        self.expression_indices
            .get(&identity(e))
            .and_then(|&index| self.effects.summaries.get(index))
            .map_or(EffectSet::UNKNOWN, |effects| {
                effects.context(context).effects
            })
    }

    pub(in crate::translation_phases::semantic_analysis) fn effects_pure(
        &self,
        e: &'tu Expression<'tu>,
        context: EffectContext,
    ) -> bool {
        self.child_effects(e, context).is_empty()
    }

    /// The effects-only schedule. Only immediate children are consulted, by
    /// their retained recipe kinds; a branch is retained only where its arm
    /// has effects, and an omitted conditional reuses its condition.
    fn recipe(
        &mut self,
        info: ExpressionInfo<'tu>,
        index: usize,
        context: EffectContext,
    ) -> RecipeKind {
        use EffectContext::{
            Designation as D,
            Value as V,
        };
        #[cfg(test)]
        {
            self.effects.work += 1;
        }
        if info.ice {
            return RecipeKind::None;
        }
        if context == V && info.ty.qualifiers.contains(TypeQualifiers::VOLATILE) {
            return RecipeKind::EvaluateValue;
        }
        let e = info.expression;
        match e.kind {
            | E::Parenthesized { expression }
            | E::Cast {
                operand_expression: expression,
                ..
            } => self.sequence(e, index, context, &[(expression, context)]),
            | E::Unary {
                operator: U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
                ..
            }
            | E::Binary {
                operator:
                    B::Assignment
                    | B::MultiplicationAssignment
                    | B::DivisionAssignment
                    | B::ModuloAssignment
                    | B::AdditionAssignment
                    | B::SubtractionAssignment
                    | B::LeftShiftAssignment
                    | B::RightShiftAssignment
                    | B::BitwiseAndAssignment
                    | B::BitwiseXorAssignment
                    | B::BitwiseOrAssignment,
                ..
            }
            | E::Call { .. } => RecipeKind::EvaluateValue,
            | E::Unary {
                operator,
                operand_expression,
            } => self.sequence(
                e,
                index,
                context,
                &[(
                    operand_expression,
                    if operator == U::AddressOf { D } else { V },
                )],
            ),
            | E::Binary {
                operator: operator @ (B::LogicalAnd | B::LogicalOr),
                left_expression,
                right_expression,
            } =>
                if self.effects_pure(right_expression, V) {
                    self.sequence(e, index, context, &[(left_expression, V)])
                } else {
                    let Ok(right) = self.effect_use(right_expression, V) else {
                        return RecipeKind::Unavailable;
                    };
                    let (then, otherwise) = if operator == B::LogicalAnd {
                        (right, None)
                    } else {
                        (None, right)
                    };
                    self.conditional(e, index, context, left_expression, then, otherwise)
                },
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => self.sequence(
                e,
                index,
                context,
                &[(left_expression, V), (right_expression, V)],
            ),
            | E::Conditional(c) | E::OmittedConditional(c) => {
                let omitted = matches!(e.kind, E::OmittedConditional(_));
                if (omitted || self.effects_pure(c.then_expression, V))
                    && self.effects_pure(c.else_expression, V)
                {
                    self.sequence(e, index, context, &[(c.condition_expression, V)])
                } else {
                    let then = if omitted {
                        Ok(None)
                    } else {
                        self.effect_use(c.then_expression, V)
                    };
                    let (Ok(then), Ok(otherwise)) = (then, self.effect_use(c.else_expression, V))
                    else {
                        return RecipeKind::Unavailable;
                    };
                    self.conditional(e, index, context, c.condition_expression, then, otherwise)
                }
            },
            | E::DirectMember {
                base_expression, ..
            } => self.sequence(e, index, context, &[(base_expression, D)]),
            | E::IndirectMember {
                base_expression, ..
            } => self.sequence(e, index, context, &[(base_expression, V)]),
            | E::StatementExpression(_) => RecipeKind::StatementBlock,
            | E::Builtin(_) | E::Generic(_) => match info.selected_expression {
                | Some(selected) => self.sequence(e, index, context, &[(selected, context)]),
                | None => RecipeKind::EvaluateValue,
            },
            | E::Countof(operand) if self.countof_effects(operand).is_empty() => RecipeKind::None,
            | E::Identifier(_)
            | E::Constant(_)
            | E::StringLiteral(_)
            | E::LabelAddress(_)
            | E::Boolean(_)
            | E::Nullptr => RecipeKind::None,
            | _ => RecipeKind::EvaluateValue,
        }
    }

    /// A child use, absent when the child has nothing to execute. A child
    /// without a retained record makes every recipe naming it unavailable.
    fn effect_use(
        &self,
        e: &'tu Expression<'tu>,
        context: EffectContext,
    ) -> Result<Option<EffectUse>, MissingRecord> {
        let &index = self
            .expression_indices
            .get(&identity(e))
            .ok_or(MissingRecord)?;
        let kind = self
            .effects
            .summaries
            .get(index)
            .ok_or(MissingRecord)?
            .context(context)
            .recipe;
        Ok((kind != RecipeKind::None).then_some(EffectUse {
            expression: ExprId(index),
            context,
        }))
    }

    /// The node's own type-site actions run first, then each child use. An
    /// empty sequence without actions executes nothing.
    fn sequence(
        &mut self,
        e: &'tu Expression<'tu>,
        index: usize,
        context: EffectContext,
        children: &[(&'tu Expression<'tu>, EffectContext)],
    ) -> RecipeKind {
        let mut uses = [None; 2];
        for (slot, &(child, context)) in uses.iter_mut().zip(children) {
            let Ok(child) = self.effect_use(child, context) else {
                return RecipeKind::Unavailable;
            };
            *slot = child;
        }
        let actions = self
            .expression_actions
            .get(&identity(e))
            .is_some_and(|site| !site.bounds.is_empty() || !site.operand_evaluations.is_empty());
        if uses.iter().all(Option::is_none) && !actions {
            return RecipeKind::None;
        }
        let uses = self
            .types
            .tu
            .alloc_slice_fill_iter(uses.into_iter().flatten());
        _ = self
            .effects
            .recipes
            .insert(key(index, context), EffectRecipe::Sequence(uses));
        RecipeKind::Retained
    }

    fn conditional(
        &mut self,
        e: &'tu Expression<'tu>,
        index: usize,
        context: EffectContext,
        condition: &'tu Expression<'tu>,
        then: Option<EffectUse>,
        otherwise: Option<EffectUse>,
    ) -> RecipeKind {
        if then.is_none() && otherwise.is_none() {
            return self.sequence(e, index, context, &[(condition, EffectContext::Value)]);
        }
        let Some(&condition) = self.expression_indices.get(&identity(condition)) else {
            return RecipeKind::Unavailable;
        };
        _ = self.effects.recipes.insert(
            key(index, context),
            EffectRecipe::Conditional {
                condition: ExprId(condition),
                then,
                otherwise,
            },
        );
        RecipeKind::Retained
    }

    /// Opens the summary of a GNU statement expression's body.
    pub(in crate::translation_phases::semantic_analysis) fn open_block_effects(&mut self) {
        self.effects.blocks.push(EffectSet::empty());
    }

    /// Closes the innermost body summary for its statement expression.
    pub(in crate::translation_phases::semantic_analysis) fn close_block_effects(&mut self) {
        self.effects.closed = self.effects.blocks.pop();
    }

    /// Declarations and nested definitions in a body are not summarized.
    pub(in crate::translation_phases::semantic_analysis) fn unsummarized_block_item(&mut self) {
        if let Some(block) = self.effects.blocks.last_mut() {
            *block |= EffectSet::UNKNOWN;
        }
    }

    /// Adds one completed statement's own contribution to the innermost open
    /// body; nested statements contributed when they completed. Loops and
    /// jumps may not return normally, so they are control effects.
    /// C99: §6.8.3p2, p. 132; PDF p. 144; §6.8.5-6.8.6, pp. 135-139;
    /// PDF pp. 147-151.
    pub(in crate::translation_phases::semantic_analysis) fn statement_effects(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) {
        if self.effects.blocks.is_empty() {
            return;
        }
        #[cfg(test)]
        {
            self.effects.work += 1;
        }
        let slot = |this: &Self, slot: ExpressionSlot<'tu>| match slot {
            | ExpressionSlot::Parsed(e) => this.child_effects(e, EffectContext::Value),
            | ExpressionSlot::Selection(_) => EffectSet::UNKNOWN,
            | ExpressionSlot::Missing(_) => EffectSet::empty(),
        };
        let effects = match statement.kind {
            | S::Expression(expression) => slot(self, expression),
            | S::If {
                condition_expression,
                ..
            }
            | S::Switch {
                condition_expression,
                ..
            } => slot(self, condition_expression),
            | S::While {
                condition_expression,
                ..
            }
            | S::DoWhile {
                condition_expression,
                ..
            } => EffectSet::CONTROL | slot(self, condition_expression),
            | S::For(f) => {
                let initializer = match f.initializer {
                    | Some(ForInitializer::Expression(expression)) => slot(self, expression),
                    | Some(_) => EffectSet::UNKNOWN,
                    | None => EffectSet::empty(),
                };
                EffectSet::CONTROL
                    | initializer
                    | f.condition_expression
                        .map_or(EffectSet::empty(), |e| slot(self, e))
                    | f.iteration_expression
                        .map_or(EffectSet::empty(), |e| slot(self, e))
            },
            | S::Return(expression) =>
                EffectSet::CONTROL | expression.map_or(EffectSet::empty(), |e| slot(self, e)),
            | S::ComputedGoto(expression) => EffectSet::CONTROL | slot(self, expression),
            | S::Break | S::Continue | S::Goto(_) | S::NamedBreak(_) | S::NamedContinue(_) =>
                EffectSet::CONTROL,
            | S::Declaration(_) => EffectSet::UNKNOWN,
            | S::Asm(_) | S::MsAsm(_) | S::Seh(_) | S::SehLeave =>
                EffectSet::CONTROL | EffectSet::UNKNOWN,
            | S::Compound { .. }
            | S::Label(..)
            | S::Case(..)
            | S::CaseRange(_)
            | S::Default(_)
            | S::Attributed(_)
            | S::LocalLabels(_)
            | S::Null => EffectSet::empty(),
        };
        if let Some(block) = self.effects.blocks.last_mut() {
            *block |= effects;
        }
    }
}

bitflags::bitflags! {
    /// Observable effect classes of one evaluation context. Empty means the
    /// evaluation may be omitted without changing observable behavior; it is
    /// not an optimizer promise that arithmetic cannot trap or be poison.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct EffectSet: u8 {
        /// Stores: assignment, increment/decrement, writing builtins and
        /// compound literal initialization.
        const WRITE = 1 << 0;
        /// A call whose callee may perform any side effect.
        const CALL = 1 << 1;
        /// An access to a volatile object.
        const VOLATILE = 1 << 2;
        /// Control transfer or possible non-termination.
        const CONTROL = 1 << 3;
        /// Dynamic stack lifetime operations such as `alloca`.
        const LIFETIME = 1 << 4;
        /// Unmodeled or unsummarized evaluation, treated as effectful.
        const UNKNOWN = 1 << 5;
    }
}

/// Whether a value is read or only designated (address-only and
/// effects-only uses). C99: §6.3.2.1p2, p. 46; PDF p. 58.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectContext {
    Value,
    Designation,
}

/// Dense per-expression record, parallel to expression ordinals: four bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExpressionEffects {
    pub(crate) value:       ContextEffects,
    pub(crate) designation: ContextEffects,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextEffects {
    pub(crate) effects: EffectSet,
    recipe:             RecipeKind,
}

/// Recipes naming only the expression itself stay in the dense record;
/// sequences and branches are sparse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecipeKind {
    None,
    EvaluateValue,
    StatementBlock,
    Retained,
    /// A child had no record; the query reports the missing fact.
    Unavailable,
}

/// What effects-only evaluation of one expression executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectRecipe<'tu> {
    /// Nothing, not even type-site actions.
    None,
    /// Evaluate the whole expression and discard its value.
    EvaluateValue(ExprId),
    /// The expression's own type-site actions, then each use in order.
    Sequence(&'tu [EffectUse]),
    /// Evaluate the condition once as a branch, then the selected arm's uses.
    Conditional {
        condition: ExprId,
        then:      Option<EffectUse>,
        otherwise: Option<EffectUse>,
    },
    /// Execute the GNU statement expression's body for its effects.
    StatementBlock(ExprId),
}

/// One child recipe applied in its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffectUse {
    pub(crate) expression: ExprId,
    pub(crate) context:    EffectContext,
}

/// Retained summaries and sparse recipes.
#[derive(Debug)]
pub(crate) struct EffectFacts<'tu> {
    pub(crate) summaries: &'tu [ExpressionEffects],
    pub(crate) recipes:   ArenaMap<'tu, usize, EffectRecipe<'tu>>,
    /// Summary, recipe and statement steps performed during production.
    #[cfg(test)]
    pub(crate) work:      usize,
}

pub(in crate::translation_phases::semantic_analysis) struct EffectBuilder<'tu, 's> {
    summaries: ArenaVec<'tu, ExpressionEffects>,
    recipes:   ArenaMap<'tu, usize, EffectRecipe<'tu>>,
    /// Open statement-expression bodies, innermost last.
    blocks:    ArenaVec<'s, EffectSet>,
    closed:    Option<EffectSet>,
    #[cfg(test)]
    work:      usize,
}

impl<'tu> SemanticTranslationUnit<'tu> {
    /// The effect classes of evaluating `expression` in `context`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Retained summary interface inspected by semantic tests; lowering consumes \
                      recipes."
        )
    )]
    pub(crate) fn effect_summary(
        &self,
        expression: &Expression<'tu>,
        context: EffectContext,
    ) -> Option<EffectSet> {
        let index = self.expression_index(expression)?;
        Some(self.effects.summaries.get(index)?.context(context).effects)
    }

    /// The retained effects-only recipe; `None` when the fact is missing.
    pub(crate) fn effect_recipe(
        &self,
        expression: &Expression<'tu>,
        context: EffectContext,
    ) -> Option<EffectRecipe<'tu>> {
        let index = self.expression_index(expression)?;
        match self.effects.summaries.get(index)?.context(context).recipe {
            | RecipeKind::None => Some(EffectRecipe::None),
            | RecipeKind::EvaluateValue => Some(EffectRecipe::EvaluateValue(ExprId(index))),
            | RecipeKind::StatementBlock => Some(EffectRecipe::StatementBlock(ExprId(index))),
            | RecipeKind::Retained => self.effects.recipes.get(&key(index, context)).copied(),
            | RecipeKind::Unavailable => None,
        }
    }
}

impl ExpressionEffects {
    pub(crate) const fn context(self, context: EffectContext) -> ContextEffects {
        match context {
            | EffectContext::Value => self.value,
            | EffectContext::Designation => self.designation,
        }
    }
}

impl<'tu, 's> EffectBuilder<'tu, 's> {
    pub(in crate::translation_phases::semantic_analysis) fn new(
        tu: &'tu Bump,
        scratch: &'s Bump,
    ) -> Self {
        Self {
            summaries:         ArenaVec::new_in(tu),
            recipes:           ArenaMap::with_hasher_in(FxBuildHasher, tu),
            blocks:            ArenaVec::new_in(scratch),
            closed:            None,
            #[cfg(test)]
            work:              0,
        }
    }

    pub(in crate::translation_phases::semantic_analysis) fn is_balanced(&self) -> bool {
        self.blocks.is_empty() && self.closed.is_none()
    }

    pub(in crate::translation_phases::semantic_analysis) fn retain(self) -> EffectFacts<'tu> {
        EffectFacts {
            summaries:         self.summaries.leak(),
            recipes:           self.recipes,
            #[cfg(test)]
            work:              self.work,
        }
    }
}

/// A child that has no retained record; only erroneous input has one.
struct MissingRecord;

const fn key(index: usize, context: EffectContext) -> usize {
    index * 2 + matches!(context, EffectContext::Designation) as usize
}

fn identity<T>(value: &T) -> usize {
    std::ptr::from_ref(value).addr()
}
