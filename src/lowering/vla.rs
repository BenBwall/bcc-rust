//! Runtime array bounds and lexical cleanup: VLA stack release and GNU
//! cleanup calls, innermost first, on every region exit. Bound expressions run
//! on the ordinary explicit task stack; saved extents are SSA variables so loop
//! invocations evaluate new bounds without reevaluating a typedef's bound.
//! C99: §6.7.5.2p5, p. 117; PDF p. 129; §6.5.3.4p2, p. 80;
//! PDF p. 92; §6.2.4p6, p. 32; PDF p. 44.

use super::{
    FunctionLowerer,
    LoweringError,
    Task,
    ssa::Op,
};
use crate::{
    configuration::RuleSet,
    ir::{
        InstData,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::Statement,
        semantic_analysis::{
            CleanupKind,
            ControlTarget,
            SizeExtent,
            SizeFormula,
            Transition,
            TypeId,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Consume the site's semantic operand evaluations before new bounds.
    /// GNU extension: GCC Typeof; C99: §6.5.3.4p2, p. 80; PDF p. 92.
    pub(super) fn prepare_site_tasks(
        site: crate::translation_phases::semantic_analysis::BoundSiteFacts<'tu>,
        tasks: &mut ArenaVec<'_, Task<'tu>>,
    ) {
        for &expression in site.operand_evaluations {
            tasks.push(Task::EvaluateRaw(expression));
            tasks.push(Task::Discard);
        }
    }

    pub(super) fn prepare_expression_actions(
        &mut self,
        expression: &'tu crate::translation_phases::parsing::syntax::Expression<'tu>,
    ) -> Result<(), LoweringError> {
        if let Some(site) = self.unit.sema.expression_actions(expression) {
            let mut tasks = ArenaVec::new_in(self.scratch);
            Self::prepare_site_tasks(site, &mut tasks);
            self.prepare_bound_tasks(site.bounds, &mut tasks, expression.source_vectors)?;
            self.push_all(&tasks);
        } else if matches!(
            expression.kind,
            crate::translation_phases::parsing::syntax::ExpressionType::Cast { .. }
        ) {
            return Err(LoweringError::missing(
                "expression type-site actions are not retained",
                expression.source_vectors,
            ));
        }
        Ok(())
    }

    pub(super) fn prepare_bound_tasks(
        &mut self,
        bounds: &[usize],
        tasks: &mut ArenaVec<'_, Task<'tu>>,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        for &index in bounds {
            let fact = self.unit.sema.runtime_bounds.get(index).ok_or_else(|| {
                LoweringError::missing("runtime bound record is not retained", source)
            })?;
            if fact.site
                == crate::translation_phases::semantic_analysis::BoundEvaluationSite::Unevaluated
            {
                return Err(LoweringError::missing(
                    "runtime bound evaluation site is not retained",
                    source,
                ));
            }
            if fact
                .dependencies
                .iter()
                .any(|dependency| self.unit.sema.runtime_bounds.get(dependency.0).is_none())
            {
                return Err(LoweringError::missing(
                    "runtime bound dependency is not retained",
                    source,
                ));
            }
            if !self.bounds.contains_key(&index) {
                let var = self.draft.declare_var(Type::I64);
                _ = self.bounds.insert(index, var);
                tasks.push(Task::Evaluate(
                    self.unit.sema.runtime_bounds[index].expression,
                ));
                tasks.push(Task::FinishBound(index));
            }
        }
        Ok(())
    }

    pub(super) fn finish_bound(&mut self, index: usize) -> Result<(), LoweringError> {
        let mut item = self.pop();
        let bound = self.unit.sema.runtime_bounds[index];
        let source = bound.expression.source_vectors;
        if let super::Operand::Place(place) = item.operand {
            item = self.read(place, item.ty, item.ty, source)?;
        }
        let signed = self.repr(bound.ty, source)?.signed();
        let (bits, converted_signed) = self
            .unit
            .sema
            .types
            .target
            .integer(bound.conversion)
            .ok_or_else(|| {
                LoweringError::missing("bound extent conversion is not retained", source)
            })?;
        let value = self.value(item, source)?;
        let value = self.extend(value, signed, super::types::int_type(bits));
        let value = self.extend(value, converted_signed, Type::I64);
        self.draft.def_var(self.bounds[&index], value);
        Ok(())
    }

    /// C99: §6.5.3.4p2, p. 80; PDF p. 92; §6.7.5.2p5,
    /// p. 117; PDF p. 129. A saved bound is never reevaluated.
    pub(super) fn runtime_size(
        &mut self,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let facts = &self.unit.sema.sizes;
        if let Some(size) = facts.constant_size(ty) {
            return Ok(self.iconst(Type::I64, i128::from(size)));
        }
        let formula = facts
            .formula(ty)
            .ok_or_else(|| LoweringError::missing("type size formula is not retained", source))?;
        self.emit_size_formula(formula, source)
    }

    pub(super) fn emit_size_formula(
        &mut self,
        mut formula: crate::translation_phases::semantic_analysis::SizeFormulaId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let facts = &self.unit.sema.sizes;
        let mut products = ArenaVec::new_in(self.scratch);
        let size = loop {
            match facts.formulas.get(formula.0).copied().ok_or_else(|| {
                LoweringError::missing("size formula node is not retained", source)
            })? {
                | SizeFormula::Constant(size) => break size,
                | SizeFormula::Product { inner, extent } => {
                    products.push(extent);
                    formula = inner;
                },
            }
        };
        // Keep the existing scalar instruction order while interpreting the
        // semantic DAG. No type walk or factor discovery occurs here.
        let mut factors = ArenaVec::new_in(self.scratch);
        for &extent in &products {
            let value = match extent {
                | SizeExtent::Constant(size) => self.iconst(Type::I64, i128::from(size)),
                | SizeExtent::Bound(id) => {
                    let var = self.bounds.get(&id.0).copied().ok_or_else(|| {
                        LoweringError::missing("VLA bound was not evaluated", source)
                    })?;
                    self.draft.use_var(var)
                },
            };
            factors.push(value);
        }
        let mut size = self.iconst(Type::I64, i128::from(size));
        for &factor in factors.iter().rev() {
            size = self.binary_inst(Opcode::Imul, InstFlags::empty(), size, factor);
        }
        Ok(size)
    }

    /// Stack tokens are lowering SSA variables keyed by semantic region.
    /// C99: §6.2.4p6, p. 32; PDF p. 44.
    pub(super) fn enter_vla(
        &mut self,
        binding: usize,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let region =
            self.unit.sema.control.vla_region(binding).ok_or_else(|| {
                LoweringError::missing("VLA lifetime region is not retained", source)
            })?;
        let save = self.nullary(Opcode::StackSave);
        let var = self.draft.declare_var(Type::Ptr);
        self.draft.def_var(var, save);
        _ = self.vla_tokens.insert(region, var);
        Ok(())
    }

    /// Under GCC's rules, keeps an `alloca` block until the function
    /// returns: leaving a VLA scope entered before it restores the stack to
    /// where it is now, below the block, so the scope's arrays stay too, as
    /// GCC keeps a scope's stack once it calls `alloca`. Each token is an
    /// SSA variable, so an exit no `alloca` reached still releases
    /// everything. Clang releases the block with the scope's arrays.
    /// GNU extension: GCC manual, "Other Builtins" (`__builtin_alloca`).
    pub(super) fn keep_stack(&mut self) {
        if self.vla_tokens.is_empty()
            || self.unit.context.configuration.alloca_scope_rules() == RuleSet::Clang
        {
            return;
        }
        let save = self.nullary(Opcode::StackSave);
        for &var in self.vla_tokens.values() {
            self.draft.def_var(var, save);
        }
    }

    pub(super) fn cleanup_transition(
        &mut self,
        transition: Transition,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let regions = self.unit.sema.control.regions;
        let mut cleanup = regions[transition.left.first.0].nearest_cleanup;
        let common = regions[transition.left.stop_before.0];
        while let Some(region) = cleanup {
            let fact = regions[region.0];
            if fact.preorder <= common.preorder {
                break;
            }
            match fact.cleanup {
                | CleanupKind::AutomaticVla(_) => {
                    let token = self.vla_tokens.get(&region).copied().ok_or_else(|| {
                        LoweringError::missing("VLA region has no stack token", source)
                    })?;
                    let save = self.draft.use_var(token);
                    self.restore_stack(save);
                },
                | CleanupKind::Call { variable, function } =>
                    self.call_cleanup(variable.0, function.0, source)?,
                | CleanupKind::None => {
                    return Err(LoweringError::missing(
                        "a cleanup link selects a region without cleanup",
                        source,
                    ));
                },
            }
            let parent = fact.parent.ok_or_else(|| {
                LoweringError::missing("transition leaves its function root", source)
            })?;
            cleanup = regions[parent.0].nearest_cleanup;
        }
        Ok(())
    }

    pub(super) fn cleanup_normal(
        &mut self,
        statement: &Statement<'_>,
    ) -> Result<(), LoweringError> {
        let transition = self
            .unit
            .sema
            .control
            .statement(statement)
            .ok_or_else(|| {
                LoweringError::missing(
                    "statement control facts are not retained",
                    statement.source_vectors,
                )
            })?
            .exit;
        self.cleanup_transition(transition, statement.source_vectors)
    }

    pub(super) fn cleanup_jump(
        &mut self,
        statement: &Statement<'_>,
    ) -> Result<ControlTarget, LoweringError> {
        let facts = self
            .unit
            .sema
            .control
            .statement(statement)
            .and_then(|s| s.jump)
            .map(|id| self.unit.sema.control.jumps[id.0])
            .ok_or_else(|| {
                LoweringError::missing(
                    "jump control facts are not retained",
                    statement.source_vectors,
                )
            })?;
        self.cleanup_transition(facts.transition, facts.provenance)?;
        Ok(facts.target)
    }

    pub(super) fn transition_has_cleanup(&self, transition: Transition) -> bool {
        let regions = self.unit.sema.control.regions;
        regions[transition.left.first.0]
            .nearest_cleanup
            .is_some_and(|cleanup| {
                regions[cleanup.0].preorder > regions[transition.left.stop_before.0].preorder
            })
    }

    fn restore_stack(&mut self, save: Value) {
        _ = self.draft.push(
            Op::Inst(InstData::Unary {
                opcode: Opcode::StackRestore,
                ty:     Type::Ptr,
                arg:    save,
            }),
            None,
        );
    }
}
