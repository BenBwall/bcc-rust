//! Runtime array bounds and lexical stack cleanup. Bound expressions run on
//! the ordinary explicit task stack; saved extents are SSA variables so loop
//! invocations evaluate new bounds without reevaluating a typedef's bound.
//! C99: §6.7.5.2p5, pp. 116-117; PDF pp. 128-129; §6.5.3.4p2,
//! p. 80; PDF p. 92; §6.2.4p6, p. 32; PDF p. 44.

use super::{
    FunctionLowerer,
    LoweringError,
    Task,
    ssa::Op,
};
use crate::{
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
            ArrayBound,
            ControlTarget,
            Transition,
            TypeId,
            TypeKind,
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
            self.prepare_bound_tasks(site.bounds, &mut tasks);
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
    ) {
        for &index in bounds {
            if !self.bounds.contains_key(&index) {
                let var = self.draft.declare_var(Type::I64);
                _ = self.bounds.insert(index, var);
                tasks.push(Task::Evaluate(self.unit.sema.types.runtime_bounds[index]));
                tasks.push(Task::FinishBound(index));
            }
        }
    }

    pub(super) fn finish_bound(&mut self, index: usize) -> Result<(), LoweringError> {
        let mut item = self.pop();
        let source = self.unit.sema.types.runtime_bounds[index].source_vectors;
        if let super::Operand::Place(place) = item.operand {
            item = self.read(place, item.ty, item.ty, source)?;
        }
        let signed = self.repr(item.ty, source)?.signed();
        let value = self.value(item, source)?;
        let value = self.extend(value, signed, Type::I64);
        self.draft.def_var(self.bounds[&index], value);
        Ok(())
    }

    /// C99: §6.5.3.4p2, p. 80; PDF p. 92; §6.7.5.2p5,
    /// pp. 116-117; PDF pp. 128-129. A saved bound is never reevaluated.
    pub(super) fn runtime_size(
        &mut self,
        mut ty: TypeId,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        if let Some(layout) = self.unit.sema.types.layout(ty) {
            return Ok(self.iconst(Type::I64, i128::from(layout.size)));
        }
        let mut factors = ArenaVec::new_in(self.scratch);
        while let TypeKind::Array(element, bound) = self.unit.sema.types.kind(ty) {
            let factor = match bound {
                | ArrayBound::Constant(n) => self.iconst(Type::I64, i128::from(n)),
                | ArrayBound::Variable(index) => {
                    let var = self.bounds.get(&index).copied().ok_or_else(|| {
                        LoweringError::missing("VLA bound was not evaluated", source)
                    })?;
                    self.draft.use_var(var)
                },
                | _ =>
                    return Err(LoweringError::missing(
                        "array has no runtime extent",
                        source,
                    )),
            };
            factors.push(factor);
            ty = element;
        }
        let layout = super::types::layout(&self.unit.sema.types, ty)
            .map_err(|kind| super::module_items::at(kind, source))?;
        let mut size = self.iconst(Type::I64, i128::from(layout.size));
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
            debug_assert!(
                matches!(
                    fact.cleanup,
                    crate::translation_phases::semantic_analysis::CleanupKind::AutomaticVla(_)
                ),
                "cleanup links select automatic VLA regions"
            );
            if fact.preorder <= common.preorder {
                break;
            }
            let token =
                self.vla_tokens.get(&region).copied().ok_or_else(|| {
                    LoweringError::missing("VLA region has no stack token", source)
                })?;
            let save = self.draft.use_var(token);
            self.restore_stack(save);
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
