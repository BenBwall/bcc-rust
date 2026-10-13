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
        parsing::{
            SyntaxOperand,
            declaration_syntax::{
                Declaration,
                Declarator,
                DirectDeclarator,
                Initializer,
                InitializerType,
            },
            syntax::{
                BlockItem,
                Expression,
                ExpressionSlot,
                ExpressionType,
                ForInitializer,
                Statement,
                StatementType,
            },
        },
        semantic_analysis::{
            ArrayBound,
            TypeId,
            TypeKind,
        },
    },
    util::bump::{
        ArenaSet,
        ArenaVec,
    },
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

    /// Labels retain their lexical path, including statement-expression blocks
    /// in operands, bounds and initializers. The work stack carries the parent
    /// at the syntax site, before later declarations extend its VLA lifetime.
    /// C99: §6.2.4p6, p. 32; PDF p. 44; §6.8.6.1p2, p. 137; PDF p. 149.
    /// GNU extension: GCC "Statement Exprs".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    pub(super) fn index_vla_scopes(&mut self) {
        let mut pending = ArenaVec::new_in(self.scratch);
        let mut expressions = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        let mut types = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        pending.push((ScopeNode::Statement(self.record.syntax.body), 0));
        while let Some((node, mut parent)) = pending.pop() {
            match node {
                | ScopeNode::Expression(expression) => {
                    if !expressions.insert(std::ptr::from_ref(expression).addr()) {
                        continue;
                    }
                    let mut push = |e| pending.push((ScopeNode::Expression(e), parent));
                    match expression.kind {
                        | ExpressionType::StatementExpression(s) =>
                            pending.push((ScopeNode::Statement(s), parent)),
                        | ExpressionType::Parenthesized { expression }
                        | ExpressionType::Unary {
                            operand_expression: expression,
                            ..
                        }
                        | ExpressionType::Cast {
                            operand_expression: expression,
                            ..
                        }
                        | ExpressionType::SizeofExpr(expression)
                        | ExpressionType::AlignofExpr(expression)
                        | ExpressionType::DirectMember {
                            base_expression: expression,
                            ..
                        }
                        | ExpressionType::IndirectMember {
                            base_expression: expression,
                            ..
                        } => push(expression),
                        | ExpressionType::Binary {
                            left_expression,
                            right_expression,
                            ..
                        } => {
                            push(left_expression);
                            push(right_expression);
                        },
                        | ExpressionType::Conditional(c)
                        | ExpressionType::OmittedConditional(c) => {
                            push(c.condition_expression);
                            push(c.then_expression);
                            push(c.else_expression);
                        },
                        | ExpressionType::Call {
                            function_expression,
                            arguments,
                        } => {
                            push(function_expression);
                            for &argument in arguments {
                                push(argument);
                            }
                        },
                        | ExpressionType::CompoundLiteral { initializer, .. } =>
                            pending.push((ScopeNode::Initializer(initializer), parent)),
                        | ExpressionType::Generic(g) => {
                            for association in g.associations {
                                push(association.expression);
                            }
                            if let SyntaxOperand::Expression(e) = g.controlling {
                                push(e);
                            }
                        },
                        | ExpressionType::Builtin(b) => {
                            for &operand in b.operands {
                                if let SyntaxOperand::Expression(e) = operand {
                                    push(e);
                                }
                            }
                            for &member in b.members {
                                if let crate::translation_phases::parsing::OffsetMember::Index(e) =
                                    member
                                {
                                    push(e);
                                }
                            }
                        },
                        | _ => {},
                    }
                    // Type operands can contain evaluated VLA bounds too.
                    let name = match expression.kind {
                        | ExpressionType::SizeofType(name)
                        | ExpressionType::AlignofType(name)
                        | ExpressionType::Cast {
                            target_type: name, ..
                        }
                        | ExpressionType::CompoundLiteral {
                            type_name: name, ..
                        } => Some(name),
                        | _ => None,
                    };
                    if let Some(name) = name
                        && let Some(declarator) = name.declarator
                    {
                        pending.push((ScopeNode::Declarator(declarator), parent));
                    }
                },
                | ScopeNode::Declarator(declarator) =>
                    for direct in declarator.kind {
                        match *direct {
                            | DirectDeclarator::Array {
                                assignment_expression: Some(e),
                                ..
                            } => pending.push((ScopeNode::Expression(e), parent)),
                            | DirectDeclarator::Parenthesized(d) =>
                                pending.push((ScopeNode::Declarator(d.declarator), parent)),
                            | _ => {},
                        }
                    },
                | ScopeNode::Initializer(initializer) => match initializer.kind {
                    | InitializerType::AssignmentExpression(e) =>
                        pending.push((ScopeNode::Expression(e), parent)),
                    | InitializerType::InitializerList(list) =>
                        for element in list.elements {
                            pending.push((ScopeNode::Initializer(element.initializer), parent));
                        },
                },
                | ScopeNode::Type(ty) => {
                    if !types.insert(ty) {
                        continue;
                    }
                    match self.unit.sema.types.kind(ty) {
                        | TypeKind::Array(element, bound) => {
                            pending.push((ScopeNode::Type(element), parent));
                            if let ArrayBound::Variable(index) = bound {
                                pending.push((
                                    ScopeNode::Expression(
                                        self.unit.sema.types.runtime_bounds[index],
                                    ),
                                    parent,
                                ));
                            }
                        },
                        | TypeKind::Pointer(element) =>
                            pending.push((ScopeNode::Type(element), parent)),
                        | _ => {},
                    }
                },
                | ScopeNode::Statement(statement) => {
                    if matches!(
                        statement.kind,
                        StatementType::Compound { .. } | StatementType::For(_)
                    ) {
                        let id = std::ptr::from_ref(statement).addr();
                        _ = self.scope_parents.insert(id, parent);
                        parent = id;
                    }
                    match statement.kind {
                        | StatementType::Compound { items } => {
                            let start = pending.len();
                            for item in items {
                                match *item {
                                    | BlockItem::Statement(s) =>
                                        pending.push((ScopeNode::Statement(s), parent)),
                                    | BlockItem::Declaration(d) =>
                                        parent = self.index_declaration(d, parent, &mut pending),
                                    | _ => {},
                                }
                            }
                            // Visit declarations before subsequent uses of a
                            // typedef's shared bound expression.
                            pending[start..].reverse();
                        },
                        | StatementType::Label(label, inner) => {
                            _ = self.label_scopes.insert(label.name, parent);
                            pending.push((ScopeNode::Statement(inner), parent));
                        },
                        | StatementType::If {
                            condition_expression,
                            then_statement,
                            else_statement,
                        } => {
                            push_slot(&mut pending, condition_expression, parent);
                            pending.push((ScopeNode::Statement(then_statement), parent));
                            if let Some(s) = else_statement {
                                pending.push((ScopeNode::Statement(s), parent));
                            }
                        },
                        | StatementType::While {
                            condition_expression,
                            body_statement,
                        }
                        | StatementType::DoWhile {
                            condition_expression,
                            body_statement,
                        }
                        | StatementType::Switch {
                            condition_expression,
                            body_statement,
                        } => {
                            push_slot(&mut pending, condition_expression, parent);
                            pending.push((ScopeNode::Statement(body_statement), parent));
                        },
                        | StatementType::For(clauses) => {
                            match clauses.initializer {
                                | Some(ForInitializer::Declaration(d)) =>
                                    parent = self.index_declaration(d, parent, &mut pending),
                                | Some(ForInitializer::Expression(e)) =>
                                    push_slot(&mut pending, e, parent),
                                | None => {},
                            }
                            for slot in [clauses.condition_expression, clauses.iteration_expression]
                                .into_iter()
                                .flatten()
                            {
                                push_slot(&mut pending, slot, parent);
                            }
                            pending.push((ScopeNode::Statement(clauses.body_statement), parent));
                        },
                        | StatementType::Case(_, inner) | StatementType::Default(inner) =>
                            pending.push((ScopeNode::Statement(inner), parent)),
                        | StatementType::CaseRange(range) =>
                            pending.push((ScopeNode::Statement(range.statement), parent)),
                        | StatementType::Attributed(a) =>
                            pending.push((ScopeNode::Statement(a.statement), parent)),
                        | StatementType::Expression(e) | StatementType::Return(Some(e)) =>
                            push_slot(&mut pending, e, parent),
                        | StatementType::Declaration(d) => {
                            _ = self.index_declaration(d, parent, &mut pending);
                        },
                        | _ => {},
                    }
                },
            }
        }
    }

    fn index_declaration(
        &mut self,
        declaration: &'tu Declaration<'tu>,
        mut parent: usize,
        pending: &mut ArenaVec<'_, (ScopeNode<'tu>, usize)>,
    ) -> usize {
        for init in declaration.init_declarators {
            let Some(binding) = self.unit.declarator_binding(init.declarator) else {
                continue;
            };
            let data = self.unit.sema.bindings[binding];
            pending.push((ScopeNode::Type(data.ty), parent));
            if data.duration == crate::translation_phases::semantic_analysis::Duration::Automatic
                && matches!(self.unit.sema.types.kind(data.ty), TypeKind::Array(..))
                && self.unit.sema.types.layout(data.ty).is_none()
            {
                let id = binding + 1;
                _ = self.scope_parents.insert(id, parent);
                parent = id;
            }
            if let Some(initializer) = init.initializer {
                pending.push((ScopeNode::Initializer(initializer), parent));
            }
        }
        parent
    }
}

pub(super) struct Scope {
    pub(super) id: usize,
    save:          Option<Value>,
    marker:        bool,
}

/// Syntax and retained bound types share one nonrecursive scope-index stack.
enum ScopeNode<'tu> {
    Statement(&'tu Statement<'tu>),
    Expression(&'tu Expression<'tu>),
    Initializer(&'tu Initializer<'tu>),
    Type(TypeId),
    Declarator(Declarator<'tu>),
}

fn push_slot<'tu>(
    pending: &mut ArenaVec<'_, (ScopeNode<'tu>, usize)>,
    slot: ExpressionSlot<'tu>,
    parent: usize,
) {
    if let ExpressionSlot::Parsed(e) = slot {
        pending.push((ScopeNode::Expression(e), parent));
    }
}
