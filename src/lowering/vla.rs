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
        Block,
        InstData,
        InstFlags,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            BlockItem,
            Statement,
            StatementType,
        },
        semantic_analysis::{
            ArrayBound,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// C99: §6.7.5.2p5, pp. 116-117; PDF pp. 128-129.
    pub(super) fn prepare_type_tasks(
        &mut self,
        mut ty: TypeId,
        tasks: &mut ArenaVec<'_, Task<'tu>>,
    ) {
        loop {
            match self.unit.sema.types.kind(ty) {
                | TypeKind::Array(element, bound) => {
                    if let ArrayBound::Variable(index) = bound
                        && !self.bounds.contains_key(&index)
                    {
                        let var = self.draft.declare_var(Type::I64);
                        _ = self.bounds.insert(index, var);
                        tasks.push(Task::Evaluate(self.unit.sema.types.runtime_bounds[index]));
                        tasks.push(Task::FinishBound(index));
                    }
                    ty = element;
                },
                | TypeKind::Pointer(element) => ty = element,
                | _ => break,
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

    pub(super) fn enter_scope(&mut self, statement: &'tu Statement<'tu>) {
        let id = std::ptr::from_ref(statement).addr();
        self.scopes.push(Scope {
            id,
            save: None,
            marker: true,
        });
        self.tasks.push(Task::ExitScope);
    }

    /// The lifetime starts at the declaration, including backward goto
    /// cleanup to a label before it in the same compound statement.
    /// C99: §6.2.4p6, p. 32; PDF p. 44.
    pub(super) fn enter_vla(&mut self, binding: usize) {
        let id = binding + 1;
        let parent = self.scopes.last().map_or(0, |scope| scope.id);
        _ = self.scope_parents.insert(id, parent);
        let save = self.nullary(Opcode::StackSave);
        self.scopes.push(Scope {
            id,
            save: Some(save),
            marker: false,
        });
    }

    pub(super) fn exit_scope(&mut self) {
        loop {
            let scope = self.scopes.pop().expect("lexical scope is open");
            if let Some(save) = scope.save {
                self.restore_stack(save);
            }
            if scope.marker {
                break;
            }
        }
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

    pub(super) fn cleanup_to(&mut self, target: usize) {
        let mut keep = self.scopes.len();
        while keep > 0 && !self.is_scope_ancestor(self.scopes[keep - 1].id, target) {
            keep -= 1;
        }
        for index in (keep..self.scopes.len()).rev() {
            if let Some(save) = self.scopes[index].save {
                self.restore_stack(save);
            }
        }
    }

    pub(super) fn remember_target(&mut self, block: Block) {
        _ = self
            .block_scopes
            .insert(block, self.scopes.last().map_or(0, |scope| scope.id));
    }

    fn is_scope_ancestor(&self, ancestor: usize, mut target: usize) -> bool {
        while target != 0 {
            if ancestor == target {
                return true;
            }
            target = self.scope_parents.get(&target).copied().unwrap_or(0);
        }
        false
    }

    fn index_declaration(
        &mut self,
        declaration: &crate::translation_phases::parsing::declaration_syntax::Declaration<'tu>,
        mut parent: usize,
    ) -> usize {
        for init in declaration.init_declarators {
            let Some(binding) = self.unit.declarator_binding(init.declarator) else {
                continue;
            };
            let data = self.unit.sema.bindings[binding];
            if data.duration == crate::translation_phases::semantic_analysis::Duration::Automatic
                && matches!(self.unit.sema.types.kind(data.ty), TypeKind::Array(..))
                && self.unit.sema.types.layout(data.ty).is_none()
            {
                let id = binding + 1;
                _ = self.scope_parents.insert(id, parent);
                parent = id;
            }
        }
        parent
    }

    /// Labels retain their lexical path, including forward goto targets.
    pub(super) fn index_vla_scopes(&mut self) {
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push((self.record.syntax.body, 0));
        while let Some((statement, parent)) = pending.pop() {
            let mut parent = parent;
            if matches!(
                statement.kind,
                StatementType::Compound { .. } | StatementType::For(_)
            ) {
                let id = std::ptr::from_ref(statement).addr();
                _ = self.scope_parents.insert(id, parent);
                parent = id;
            }
            match statement.kind {
                | StatementType::Compound { items } =>
                    for item in items {
                        match *item {
                            | BlockItem::Statement(s) => pending.push((s, parent)),
                            | BlockItem::Declaration(d) =>
                                parent = self.index_declaration(d, parent),
                            | _ => {},
                        }
                    },
                | StatementType::Label(label, inner) => {
                    _ = self.label_scopes.insert(label.name, parent);
                    pending.push((inner, parent));
                },
                | StatementType::If {
                    then_statement,
                    else_statement,
                    ..
                } => {
                    pending.push((then_statement, parent));
                    if let Some(s) = else_statement {
                        pending.push((s, parent));
                    }
                },
                | StatementType::While { body_statement, .. }
                | StatementType::DoWhile { body_statement, .. }
                | StatementType::Switch { body_statement, .. } =>
                    pending.push((body_statement, parent)),
                | StatementType::For(clauses) => {
                    if let Some(
                        crate::translation_phases::parsing::syntax::ForInitializer::Declaration(d),
                    ) = clauses.initializer
                    {
                        parent = self.index_declaration(d, parent);
                    }
                    pending.push((clauses.body_statement, parent));
                },
                | StatementType::Case(_, inner) | StatementType::Default(inner) =>
                    pending.push((inner, parent)),
                | StatementType::CaseRange(range) => pending.push((range.statement, parent)),
                | StatementType::Attributed(a) => pending.push((a.statement, parent)),
                | _ => {},
            }
        }
    }
}

pub(super) struct Scope {
    pub(super) id: usize,
    save:          Option<Value>,
    marker:        bool,
}
