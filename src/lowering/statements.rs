//! Statements, declarations and the function frame: parameters on entry,
//! control flow as blocks, and the return at the closing brace.
//!
//! Each statement pushes its parts as tasks in reverse order of execution.
//! A loop's header is sealed after its back edge, an exit after every
//! `break` is known, a label block at the end of the function, since a
//! `goto` may reach it from anywhere. A jump continues lowering in a block
//! nothing reaches, which emission drops.
//!
//! C99: §6.8, pp. 131-139; PDF pp. 143-151; §6.9.1, pp. 141-142;
//! PDF pp. 153-154.

use super::{
    Construct,
    FunctionLowerer,
    JumpTargets,
    Local,
    LoweringError,
    Task,
    module_items::at,
    ssa::{
        Draft,
        Terminator,
    },
    types::{
        Repr,
        repr,
    },
    work::{
        Item,
        Object,
        Operand,
    },
};
use crate::{
    ir::{
        Block,
        Entity,
        FuncId,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                InitializerType,
            },
            syntax::{
                BlockItem,
                ConstantExpressionSlot,
                Expression,
                ExpressionSlot,
                ForInitializer,
                Statement,
                StatementType,
            },
        },
        semantic_analysis::{
            BindingKind,
            Duration,
            FunctionRecord,
            Linkage,
            TypeKind,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Lowers one statement by pushing its parts.
    pub(super) fn statement(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) -> Result<(), LoweringError> {
        use StatementType as S;
        let source = statement.source_vectors;
        match statement.kind {
            | S::Compound { items } =>
                for item in items.iter().rev() {
                    self.tasks.push(match *item {
                        | BlockItem::Statement(statement) => Task::Statement(statement),
                        | BlockItem::Declaration(declaration) => Task::Declaration(declaration),
                        | BlockItem::FunctionDefinition(definition) =>
                            return Err(LoweringError::unsupported(
                                Construct::NestedFunction,
                                definition.source_vectors,
                            )),
                    });
                },
            | S::Expression(slot) => {
                let expression = expression(slot, source)?;
                self.tasks.push(Task::Discard);
                self.tasks.push(Task::Evaluate(expression));
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                let condition = expression(condition_expression, source)?;
                let then = self.draft.create_block(&[]);
                let join = self.draft.create_block(&[]);
                if let Some(else_statement) = else_statement {
                    let otherwise = self.draft.create_block(&[]);
                    self.push_all(&[
                        Task::Condition {
                            expression: condition,
                            then,
                            otherwise,
                        },
                        Task::Enter {
                            block: then,
                            seal:  true,
                        },
                        Task::Statement(then_statement),
                        Task::JumpTo {
                            block: join,
                            seal:  false,
                        },
                        Task::Enter {
                            block: otherwise,
                            seal:  true,
                        },
                        Task::Statement(else_statement),
                        Task::JumpTo {
                            block: join,
                            seal:  true,
                        },
                        Task::Enter {
                            block: join,
                            seal:  false,
                        },
                    ]);
                } else {
                    self.push_all(&[
                        Task::Condition {
                            expression: condition,
                            then,
                            otherwise: join,
                        },
                        Task::Enter {
                            block: then,
                            seal:  true,
                        },
                        Task::Statement(then_statement),
                        Task::JumpTo {
                            block: join,
                            seal:  true,
                        },
                        Task::Enter {
                            block: join,
                            seal:  false,
                        },
                    ]);
                }
            },
            | S::While {
                condition_expression,
                body_statement,
            } => {
                let condition = expression(condition_expression, source)?;
                let header = self.draft.create_block(&[]);
                let body = self.draft.create_block(&[]);
                let exit = self.draft.create_block(&[]);
                self.push_all(&[
                    Task::JumpTo {
                        block: header,
                        seal:  false,
                    },
                    Task::Enter {
                        block: header,
                        seal:  false,
                    },
                    Task::Condition {
                        expression: condition,
                        then:       body,
                        otherwise:  exit,
                    },
                    Task::Enter {
                        block: body,
                        seal:  true,
                    },
                    Task::PushTargets(JumpTargets {
                        break_to:    exit,
                        continue_to: Some(header),
                    }),
                    Task::Statement(body_statement),
                    Task::PopTargets,
                    Task::JumpTo {
                        block: header,
                        seal:  true,
                    },
                    Task::Enter {
                        block: exit,
                        seal:  true,
                    },
                ]);
            },
            | S::DoWhile {
                condition_expression,
                body_statement,
            } => {
                let condition = expression(condition_expression, source)?;
                let body = self.draft.create_block(&[]);
                let test = self.draft.create_block(&[]);
                let exit = self.draft.create_block(&[]);
                self.push_all(&[
                    Task::JumpTo {
                        block: body,
                        seal:  false,
                    },
                    Task::Enter {
                        block: body,
                        seal:  false,
                    },
                    Task::PushTargets(JumpTargets {
                        break_to:    exit,
                        continue_to: Some(test),
                    }),
                    Task::Statement(body_statement),
                    Task::PopTargets,
                    Task::JumpTo {
                        block: test,
                        seal:  true,
                    },
                    Task::Enter {
                        block: test,
                        seal:  false,
                    },
                    Task::Condition {
                        expression: condition,
                        then:       body,
                        otherwise:  exit,
                    },
                    Task::Seal(body),
                    Task::Enter {
                        block: exit,
                        seal:  true,
                    },
                ]);
            },
            | S::For(clauses) => {
                let header = self.draft.create_block(&[]);
                let body = self.draft.create_block(&[]);
                let step = self.draft.create_block(&[]);
                let exit = self.draft.create_block(&[]);
                let mut tasks = ArenaVec::with_capacity_in(16, self.scratch);
                match clauses.initializer {
                    | Some(ForInitializer::Expression(slot)) => {
                        tasks.push(Task::Evaluate(expression(slot, source)?));
                        tasks.push(Task::Discard);
                    },
                    | Some(ForInitializer::Declaration(declaration)) =>
                        tasks.push(Task::Declaration(declaration)),
                    | None => {},
                }
                tasks.push(Task::JumpTo {
                    block: header,
                    seal:  false,
                });
                tasks.push(Task::Enter {
                    block: header,
                    seal:  false,
                });
                // C99 §6.8.5.3p2: an omitted condition is a nonzero constant.
                match clauses.condition_expression {
                    | Some(slot) => tasks.push(Task::Condition {
                        expression: expression(slot, source)?,
                        then:       body,
                        otherwise:  exit,
                    }),
                    | None => tasks.push(Task::JumpTo {
                        block: body,
                        seal:  false,
                    }),
                }
                tasks.extend_from_slice(&[
                    Task::Enter {
                        block: body,
                        seal:  true,
                    },
                    Task::PushTargets(JumpTargets {
                        break_to:    exit,
                        continue_to: Some(step),
                    }),
                    Task::Statement(clauses.body_statement),
                    Task::PopTargets,
                    Task::JumpTo {
                        block: step,
                        seal:  true,
                    },
                    Task::Enter {
                        block: step,
                        seal:  false,
                    },
                ]);
                if let Some(slot) = clauses.iteration_expression {
                    tasks.push(Task::Evaluate(expression(slot, source)?));
                    tasks.push(Task::Discard);
                }
                tasks.extend_from_slice(&[
                    Task::JumpTo {
                        block: header,
                        seal:  true,
                    },
                    Task::Enter {
                        block: exit,
                        seal:  true,
                    },
                ]);
                self.push_all(&tasks);
            },
            | S::Return(None) => {
                // C89 permits `return;` in a function with a result, whose
                // caller then may not use it.
                let value = match self.repr(self.result, source)?.value_type() {
                    | Some(ty) => Some(self.draft.poison(ty)),
                    | None => None,
                };
                self.draft
                    .terminate_and_continue_dead(Terminator::Return(value));
            },
            | S::Return(Some(slot)) => {
                self.tasks.push(Task::Return);
                self.tasks.push(Task::Evaluate(expression(slot, source)?));
            },
            | S::Break => {
                let target = self
                    .jumps
                    .last()
                    .ok_or_else(|| LoweringError::missing("break outside a loop", source))?
                    .break_to;
                self.jump_away(target);
            },
            | S::Continue => {
                let target = self
                    .jumps
                    .iter()
                    .rev()
                    .find_map(|targets| targets.continue_to)
                    .ok_or_else(|| LoweringError::missing("continue outside a loop", source))?;
                self.jump_away(target);
            },
            | S::Goto(label) => {
                let target = self.label_block(label.name);
                self.jump_away(target);
            },
            | S::Label(label, statement) => {
                let block = self.label_block(label.name);
                self.draft.jump(block, &[]);
                self.draft.switch_to(block);
                self.tasks.push(Task::Statement(statement));
            },
            | S::Case(_, inner) | S::Default(inner) => {
                let block = self
                    .cases
                    .get(&std::ptr::from_ref(statement).addr())
                    .copied()
                    .ok_or_else(|| {
                        LoweringError::missing("a case label is outside a switch", source)
                    })?;
                self.draft.jump(block, &[]);
                self.draft.switch_to(block);
                self.tasks.push(Task::Statement(inner));
            },
            | S::Switch {
                condition_expression,
                body_statement,
            } => {
                let exit = self.draft.create_block(&[]);
                self.push_all(&[
                    Task::Evaluate(expression(condition_expression, source)?),
                    Task::Switch {
                        statement,
                        body: body_statement,
                        exit,
                    },
                    Task::Statement(body_statement),
                    Task::EndSwitch { statement, exit },
                ]);
            },
            | S::Declaration(declaration) => self.tasks.push(Task::Declaration(declaration)),
            | S::Attributed(attributed) => self.tasks.push(Task::Statement(attributed.statement)),
            | S::Null => {},
            | S::CaseRange(_) =>
                return Err(LoweringError::unsupported(Construct::CaseRange, source)),
            | S::Asm(_) | S::MsAsm(_) =>
                return Err(LoweringError::unsupported(
                    Construct::InlineAssembly,
                    source,
                )),
            | S::Seh(_) | S::SehLeave =>
                return Err(LoweringError::unsupported(
                    Construct::StructuredExceptionHandling,
                    source,
                )),
            | S::ComputedGoto(_) =>
                return Err(LoweringError::unsupported(Construct::ComputedGoto, source)),
            | S::LocalLabels(_) =>
                return Err(LoweringError::unsupported(Construct::LocalLabel, source)),
            | S::NamedBreak(_) | S::NamedContinue(_) =>
                return Err(LoweringError::unsupported(Construct::NamedJump, source)),
        }
        Ok(())
    }

    /// Pops the switch's promoted controlling value and dispatches on it:
    /// each `case` and `default` of this switch, but not of a nested one,
    /// gets a block, and an absent `default` goes to the exit.
    /// C99: §6.8.4.2 paragraphs 4-5, p. 134; PDF p. 146.
    pub(super) fn switch(
        &mut self,
        statement: &'tu Statement<'tu>,
        body: &'tu Statement<'tu>,
        exit: Block,
    ) -> Result<(), LoweringError> {
        let source = statement.source_vectors;
        let item = self.pop();
        let value = self.value(item, source)?;
        let Repr::Int { ty, signed } = self.repr(item.ty, source)? else {
            return Err(LoweringError::missing(
                "a switch value is not an integer",
                source,
            ));
        };
        let start = self.switch_blocks.len();
        let mut default = None;
        let mut cases = ArenaVec::new_in(self.scratch);
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push(body);
        while let Some(statement) = pending.pop() {
            match statement.kind {
                | StatementType::Case(label, inner) => {
                    let ConstantExpressionSlot::Parsed(constant) = label else {
                        return Err(LoweringError::missing(
                            "a case has no value",
                            statement.source_vectors,
                        ));
                    };
                    let case = self
                        .info(constant.expression())?
                        .integer
                        .ok_or_else(|| {
                            LoweringError::missing("a case has no value", statement.source_vectors)
                        })?
                        .cast(ty.bits(), signed);
                    let block = self.case_block(statement);
                    cases.push((case.value, self.draft.edge(block, &[])));
                    pending.push(inner);
                },
                | StatementType::Default(inner) => {
                    default = Some(self.case_block(statement));
                    pending.push(inner);
                },
                | StatementType::CaseRange(_) =>
                    return Err(LoweringError::unsupported(
                        Construct::CaseRange,
                        statement.source_vectors,
                    )),
                | StatementType::Compound { items } =>
                    for item in items.iter().rev() {
                        if let BlockItem::Statement(statement) = *item {
                            pending.push(statement);
                        }
                    },
                | StatementType::If {
                    then_statement,
                    else_statement,
                    ..
                } => {
                    pending.extend(else_statement);
                    pending.push(then_statement);
                },
                | StatementType::While { body_statement, .. }
                | StatementType::DoWhile { body_statement, .. } => pending.push(body_statement),
                | StatementType::For(clauses) => pending.push(clauses.body_statement),
                | StatementType::Label(_, inner) => pending.push(inner),
                | StatementType::Attributed(attributed) => pending.push(attributed.statement),
                // A nested switch owns its labels.
                | _ => {},
            }
        }
        let default = self.draft.edge(default.unwrap_or(exit), &[]);
        self.draft.terminate_and_continue_dead(Terminator::Switch {
            value,
            default,
            cases,
        });
        self.switch_starts.push(start);
        self.jumps.push(JumpTargets {
            break_to:    exit,
            continue_to: None,
        });
        Ok(())
    }

    /// Ends a switch body: falls into the exit and seals the case blocks,
    /// whose predecessors are now all known.
    pub(super) fn end_switch(&mut self, _statement: &'tu Statement<'tu>, exit: Block) {
        self.draft.jump(exit, &[]);
        _ = self.jumps.pop();
        let start = self.switch_starts.pop().expect("a switch is open");
        for index in start..self.switch_blocks.len() {
            let block = self.switch_blocks[index];
            self.draft.seal(block);
        }
        self.switch_blocks.truncate(start);
        self.draft.seal(exit);
        self.draft.switch_to(exit);
    }

    fn case_block(&mut self, statement: &'tu Statement<'tu>) -> Block {
        let block = self.draft.create_block(&[]);
        _ = self
            .cases
            .insert(std::ptr::from_ref(statement).addr(), block);
        self.switch_blocks.push(block);
        block
    }

    /// Pops the returned value, converted to the result type, and returns.
    /// C99: §6.8.6.4 paragraph 3, p. 139; PDF p. 151.
    pub(super) fn return_value(&mut self) -> Result<(), LoweringError> {
        let item = self.pop();
        let source = self.record.syntax.source_vectors;
        let value = match self.repr(self.result, source)? {
            | Repr::Void => None,
            | _ => Some(self.value(item, source)?),
        };
        self.draft
            .terminate_and_continue_dead(Terminator::Return(value));
        Ok(())
    }

    /// Jumps to `target`; what follows is unreachable until a label.
    fn jump_away(&mut self, target: Block) {
        let edge = self.draft.edge(target, &[]);
        self.draft
            .terminate_and_continue_dead(Terminator::Jump(edge));
    }

    fn label_block(&mut self, label: crate::util::string_cache::StringCacheId) -> Block {
        if let Some(&block) = self.labels.get(&label) {
            return block;
        }
        let block = self.draft.create_block(&[]);
        _ = self.labels.insert(label, block);
        block
    }

    /// Pushes `tasks` so that they run in slice order.
    fn push_all(&mut self, tasks: &[Task<'tu>]) {
        self.tasks.extend(tasks.iter().rev().copied());
    }
}

// Declarations and initialization

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Allocates the objects a block-scope declaration declares and
    /// schedules their initializers in declaration order. A static local is
    /// a global initialized before the program starts.
    /// C99: §6.7.8 paragraphs 9-10, p. 126; PDF p. 138; §6.8 paragraph 3,
    /// p. 131; PDF p. 143.
    pub(super) fn declaration(
        &mut self,
        declaration: &'tu Declaration<'tu>,
    ) -> Result<(), LoweringError> {
        let mut tasks = ArenaVec::new_in(self.scratch);
        for init_declarator in declaration.init_declarators {
            let source = init_declarator.source_vectors;
            let Some(binding) = self.unit.declarator_binding(init_declarator.declarator) else {
                continue;
            };
            let data = self.unit.sema.bindings[binding];
            if data.kind != BindingKind::Object {
                continue;
            }
            if data.duration == Duration::Static {
                if data.linkage == Linkage::None {
                    _ = self.unit.define_object(
                        binding,
                        Some(self.name),
                        init_declarator.initializer,
                    )?;
                }
                continue;
            }
            let local = self.local(binding, source)?;
            let Some(initializer) = init_declarator.initializer else {
                continue;
            };
            let object = Object::Local(binding);
            match initializer.kind {
                | InitializerType::AssignmentExpression(expression)
                    if self.repr(data.ty, source)?.is_scalar() =>
                {
                    tasks.push(Task::Evaluate(expression));
                    tasks.push(Task::Initialize {
                        object,
                        offset: 0,
                        ty: data.ty,
                    });
                },
                | _ => {
                    if let Local::Slot(slot) = local {
                        self.zero_fill(slot, data.ty, source)?;
                    }
                    tasks.push(Task::InitializeAggregate {
                        object,
                        ty: data.ty,
                        initializer,
                    });
                },
            }
        }
        self.push_all(&tasks);
        Ok(())
    }
}

// The function frame

impl<'u, 'a, 'tu, 'ir, 's> FunctionLowerer<'u, 'a, 'tu, 'ir, 's> {
    /// Starts lowering `record`: the entry block's parameters are the
    /// arguments, converted on entry to the declared parameter types, which
    /// differ for an old-style definition (§6.9.1p10).
    pub(super) fn new(
        unit: &'u mut super::Unit<'a, 'tu, 'ir>,
        record: &'tu FunctionRecord<'tu>,
        func: FuncId,
        scratch: &'s Bump,
    ) -> Result<Self, LoweringError> {
        let source = record.syntax.source_vectors;
        let binding = record.binding.ok_or_else(|| {
            LoweringError::missing("a function definition has no binding", source)
        })?;
        let data = unit.sema.bindings[binding];
        let name = unit.context.string_cache.at(data.name.name);
        let TypeKind::Function { parameters, .. } = unit.sema.types.kind(data.ty) else {
            return Err(LoweringError::missing(
                "a function has no function type",
                source,
            ));
        };
        let signature = unit.module.function_signature(func);
        let draft = Draft::new(scratch, signature.params);
        let mut lowerer = Self {
            unit,
            record,
            name,
            draft,
            scratch,
            tasks: ArenaVec::new_in(scratch),
            operands: ArenaVec::new_in(scratch),
            locals: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            labels: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            cases: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            jumps: ArenaVec::new_in(scratch),
            switch_blocks: ArenaVec::new_in(scratch),
            switch_starts: ArenaVec::new_in(scratch),
            result: record.result.unqualified(),
        };
        let entry = Draft::entry();
        for (index, &parameter) in record.parameters.iter().enumerate() {
            let Some(parameter) = parameter else {
                continue;
            };
            let (Some(&value), Some(&incoming)) = (
                lowerer.draft.block_params(entry).get(index),
                parameters.get(index),
            ) else {
                return Err(LoweringError::missing(
                    "a parameter has no argument",
                    source,
                ));
            };
            let declared = lowerer.unit.sema.bindings[parameter].ty;
            let parameter_source = lowerer.unit.sema.bindings[parameter].name.source_vectors;
            let argument = Item {
                operand: Operand::Value(value),
                ty:      incoming,
            };
            let converted =
                lowerer.convert_value(argument, declared.unqualified(), parameter_source)?;
            lowerer.operands.push(converted);
            lowerer.initialize(Object::Local(parameter), 0, declared)?;
        }
        lowerer.tasks.push(Task::Statement(record.syntax.body));
        Ok(lowerer)
    }

    /// Ends the body at its closing brace: `main` returns 0, any other
    /// function returns nothing or an indeterminate value, which its caller
    /// may not use (§6.9.1p12). Label blocks are sealed now that every
    /// `goto` is known.
    /// C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25.
    pub(super) fn finish_body(&mut self) -> Result<(), LoweringError> {
        let source = self.record.syntax.source_vectors;
        let value =
            match repr(&self.unit.sema.types, self.result).map_err(|kind| at(kind, source))? {
                | Repr::Void => None,
                | representation => {
                    let ty = representation.value_type().ok_or_else(|| {
                        LoweringError::missing("a result has no value type", source)
                    })?;
                    Some(if self.name == "main" {
                        self.iconst(ty, 0)
                    } else {
                        self.draft.poison(ty)
                    })
                },
            };
        self.draft
            .terminate_and_continue_dead(Terminator::Return(value));
        for index in 0..self.draft.blocks.len() {
            self.draft.seal(Block::new(index));
        }
        Ok(())
    }
}

/// The expression a statement holds.
fn expression(
    slot: ExpressionSlot<'_>,
    source: SourceVectors,
) -> Result<&Expression<'_>, LoweringError> {
    match slot {
        | ExpressionSlot::Parsed(expression) => Ok(expression),
        | ExpressionSlot::Selection(_) => Err(LoweringError::unsupported(
            Construct::SelectionDeclaration,
            source,
        )),
        | ExpressionSlot::Missing(_) => Err(LoweringError::missing(
            "a statement has no expression",
            source,
        )),
    }
}
