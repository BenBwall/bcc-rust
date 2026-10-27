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
    LoweringError,
    module_items::at,
    ssa::{
        Draft,
        Terminator,
    },
    types::{
        self,
        Repr,
        repr,
    },
    work::{
        Item,
        JumpTargets,
        Local,
        Object,
        Operand,
        StatementMode,
        Task,
    },
};
use crate::{
    ir::{
        Block,
        Entity,
        FuncId,
        InstData,
        InstFlags,
        Opcode,
        Type,
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
                Expression,
                ExpressionSlot,
                ForInitializer,
                Statement,
                StatementType,
            },
        },
        semantic_analysis::{
            BindingKind,
            ControlTarget,
            Duration,
            FunctionRecord,
            LabelId,
            Linkage,
            Transition,
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
        mode: StatementMode,
    ) -> Result<(), LoweringError> {
        use StatementType as S;
        let source = statement.source_vectors;
        let control = self
            .unit
            .sema
            .control
            .statement(statement)
            .copied()
            .ok_or_else(|| {
                LoweringError::missing("statement control facts are not retained", source)
            })?;
        // Share semantic traversal's whole-statement decision with static
        // reachability, including references inside the omitted statement.
        // C99 §6.8.4.1p2, pp. 133-134; PDF pp. 145-146.
        if control.elided {
            return Ok(());
        }
        if !matches!(statement.kind, S::For(_)) || !self.transition_has_cleanup(control.exit) {
            self.tasks.push(Task::CleanupNormal(statement));
        }
        debug_assert!(
            std::ptr::eq(control.syntax, statement),
            "statement lookup preserves syntax identity"
        );
        // C99 §6.8.1p4, p. 132; PDF p. 144: labels preserve their statement.
        // GNU statement-expression tails enter the same targets without
        // discarding the final expression's converted value. Overflow type
        // operands carry effect-only evaluation through the same targets.
        // GNU extension: GCC "Integer Overflow Builtins".
        // <https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html>
        let labeled_child = |inner| match mode {
            | StatementMode::Value => Task::StatementValue(inner),
            | StatementMode::Effects => Task::StatementEffects(inner),
            | StatementMode::Discard => Task::Statement(inner),
        };
        let child = |inner| {
            if mode == StatementMode::Effects {
                Task::StatementEffects(inner)
            } else {
                Task::Statement(inner)
            }
        };
        match statement.kind {
            | S::Compound { items } =>
                for item in items.iter().rev() {
                    self.tasks.push(match *item {
                        | BlockItem::Statement(statement) => child(statement),
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
                if mode == StatementMode::Effects {
                    self.tasks.push(Task::BuiltinEffects(
                        expression,
                        crate::translation_phases::semantic_analysis::EffectContext::Value,
                    ));
                } else {
                    if mode == StatementMode::Discard {
                        self.tasks.push(Task::Discard);
                    }
                    self.tasks.push(Task::Evaluate(expression));
                }
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                if let crate::translation_phases::semantic_analysis::GeneratedControl::If(taken) =
                    control.generated
                {
                    if let Some(selected) = if taken {
                        Some(then_statement)
                    } else {
                        else_statement
                    } {
                        self.tasks.push(child(selected));
                    }
                    return Ok(());
                }
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
                        child(then_statement),
                        Task::JumpTo {
                            block: join,
                            seal:  false,
                        },
                        Task::Enter {
                            block: otherwise,
                            seal:  true,
                        },
                        child(else_statement),
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
                        child(then_statement),
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
                _ = self.targets.insert(
                    control.id,
                    JumpTargets {
                        break_to:    exit,
                        continue_to: Some(header),
                    },
                );
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
                    child(body_statement),
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
                _ = self.targets.insert(
                    control.id,
                    JumpTargets {
                        break_to:    exit,
                        continue_to: Some(test),
                    },
                );
                self.push_all(&[
                    Task::JumpTo {
                        block: body,
                        seal:  false,
                    },
                    Task::Enter {
                        block: body,
                        seal:  false,
                    },
                    child(body_statement),
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
                let cleanup_exit = self.transition_has_cleanup(control.exit);
                let break_exit = if cleanup_exit {
                    self.draft.create_block(&[])
                } else {
                    exit
                };
                _ = self.targets.insert(
                    control.id,
                    JumpTargets {
                        break_to:    break_exit,
                        continue_to: Some(step),
                    },
                );
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
                    child(clauses.body_statement),
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
                if cleanup_exit {
                    tasks.push(Task::CleanupNormal(statement));
                    tasks.push(Task::JumpTo {
                        block: break_exit,
                        seal:  true,
                    });
                    tasks.push(Task::Enter {
                        block: break_exit,
                        seal:  false,
                    });
                }
                self.push_all(&tasks);
            },
            | S::Return(None) => {
                // C89 permits `return;` in a function with a result, whose
                // caller then may not use it.
                let value = match self.repr(self.result, source)?.value_type() {
                    | Some(ty) => Some(self.draft.poison(ty)),
                    | None => None,
                };
                _ = self.cleanup_jump(statement)?;
                self.draft
                    .terminate_and_continue_dead(Terminator::Return(value));
            },
            | S::Return(Some(slot)) => {
                self.tasks.push(Task::Return(statement));
                self.tasks.push(Task::Evaluate(expression(slot, source)?));
            },
            | S::Break | S::Continue | S::Goto(_) => {
                let target = self.cleanup_jump(statement)?;
                let block = match target {
                    | ControlTarget::Label(label) => self.label_block(label),
                    | ControlTarget::LoopBreak(id) | ControlTarget::SwitchBreak(id) => self
                        .targets
                        .get(&id)
                        .map(|targets| targets.break_to)
                        .ok_or_else(|| {
                            LoweringError::missing("resolved break target has no IR block", source)
                        })?,
                    | ControlTarget::LoopContinue(id) => self
                        .targets
                        .get(&id)
                        .and_then(|targets| targets.continue_to)
                        .ok_or_else(|| {
                            LoweringError::missing(
                                "resolved continue target has no IR block",
                                source,
                            )
                        })?,
                    | ControlTarget::FunctionExit(_) | ControlTarget::IndirectGoto(_) =>
                        return Err(LoweringError::missing(
                            "a direct jump resolves to a function exit or an indirect goto",
                            source,
                        )),
                };
                self.jump_away(block);
            },
            | S::Label(_, statement) => {
                let label = control.label.ok_or_else(|| {
                    LoweringError::missing("label identity is not retained", source)
                })?;
                let block = self.label_block(label);
                self.draft.jump(block, &[]);
                self.draft.switch_to(block);
                self.tasks.push(labeled_child(statement));
            },
            | S::Case(_, inner) | S::Default(inner) => {
                let label = control.label.ok_or_else(|| {
                    LoweringError::missing("case identity is not retained", source)
                })?;
                let block = self.labels.get(&label).copied().ok_or_else(|| {
                    LoweringError::missing("a case label has no dispatch block", source)
                })?;
                self.draft.jump(block, &[]);
                self.draft.switch_to(block);
                self.tasks.push(labeled_child(inner));
            },
            | S::Switch {
                condition_expression,
                body_statement,
            } => {
                let exit = self.draft.create_block(&[]);
                _ = self.targets.insert(
                    control.id,
                    JumpTargets {
                        break_to:    exit,
                        continue_to: None,
                    },
                );
                if matches!(
                    control.generated,
                    crate::translation_phases::semantic_analysis::GeneratedControl::Runtime
                ) {
                    self.tasks.push(Task::EndSwitch { statement, exit });
                    self.tasks.push(child(body_statement));
                    self.tasks.push(Task::Switch { statement, exit });
                    self.tasks
                        .push(Task::Evaluate(expression(condition_expression, source)?));
                    return Ok(());
                }
                self.push_all(&[
                    Task::Switch { statement, exit },
                    child(body_statement),
                    Task::EndSwitch { statement, exit },
                ]);
            },
            | S::Declaration(declaration) => self.tasks.push(Task::Declaration(declaration)),
            | S::Attributed(attributed) => self.tasks.push(labeled_child(attributed.statement)),
            // GNU local labels only scope label names, which semantic
            // analysis already resolved to label identities. Jumps to them
            // from another function need nested functions, which lowering
            // refuses, so every use here is an ordinary jump.
            // GNU extension: GCC "Local Labels".
            | S::Null | S::LocalLabels(_) => {},
            | S::CaseRange(range) => {
                let label = control.label.ok_or_else(|| {
                    LoweringError::missing("case range identity is not retained", source)
                })?;
                let block = self.labels.get(&label).copied().ok_or_else(|| {
                    LoweringError::missing("a case range has no dispatch block", source)
                })?;
                self.draft.jump(block, &[]);
                self.draft.switch_to(block);
                self.tasks.push(labeled_child(range.statement));
            },
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
            | S::ComputedGoto(slot) => {
                self.tasks.push(Task::IndirectJump(statement));
                self.tasks.push(Task::Evaluate(expression(slot, source)?));
            },
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
        exit: Block,
    ) -> Result<(), LoweringError> {
        let source = statement.source_vectors;
        let generated = self
            .unit
            .sema
            .control
            .statement(statement)
            .ok_or_else(|| {
                LoweringError::missing("switch emission facts are not retained", source)
            })?
            .generated;
        let folded = if let crate::translation_phases::semantic_analysis::GeneratedControl::Switch(
            selected,
        ) = generated
        {
            Some(selected)
        } else {
            None
        };
        let value = if folded.is_none() {
            let item = self.pop();
            Some(self.value(item, source)?)
        } else {
            None
        };
        let facts = self
            .unit
            .sema
            .control
            .statement(statement)
            .and_then(|s| s.switch)
            .map(|id| self.unit.sema.control.switches[id.0])
            .ok_or_else(|| {
                LoweringError::missing("switch dispatch facts are not retained", source)
            })?;
        let Repr::Int { ty, .. } = self.repr(facts.ty, source)? else {
            return Err(LoweringError::missing(
                "a switch value is not an integer",
                source,
            ));
        };
        let start = self.switch_blocks.len();
        let mut default = None;
        let mut cases = ArenaVec::new_in(self.scratch);
        let mut ranges = ArenaVec::new_in(self.scratch);
        let mut selected_target = None;
        for (ordinal, case) in facts.cases.iter().enumerate() {
            debug_assert_eq!(case.ordinal, ordinal, "case facts retain source order");
            if let Some(label) = facts.default
                && label.0 < case.label.0
                && default.is_none()
            {
                let block = self.case_block(label);
                default = Some(self.dispatch_target(
                    block,
                    facts.default_dispatch.ok_or_else(|| {
                        LoweringError::missing("default transition is not retained", source)
                    })?,
                    source,
                )?);
            }
            let block = self.case_block(case.label);
            let Some(dispatch) = case.dispatch else {
                continue;
            };
            let target = self.dispatch_target(block, dispatch, case.statement.source_vectors)?;
            if let Some(selected) = folded {
                if selected.is_some_and(|label| std::ptr::eq(label, case.statement)) {
                    selected_target = Some(target);
                }
                continue;
            }
            if case.range {
                ranges.push((case.lower.value, case.upper.value, target));
            } else {
                cases.push((case.lower.value, self.draft.edge(target, &[])));
            }
        }
        if let Some(label) = facts.default
            && default.is_none()
        {
            let block = self.case_block(label);
            default = Some(self.dispatch_target(
                block,
                facts.default_dispatch.ok_or_else(|| {
                    LoweringError::missing("default transition is not retained", source)
                })?,
                source,
            )?);
        }
        if default.is_none() {
            default = Some(self.dispatch_target(exit, facts.no_match, source)?);
        }
        if let Some(selected) = folded {
            let target = if selected.is_none() {
                exit
            } else {
                selected_target.or(default).ok_or_else(|| {
                    LoweringError::missing("folded switch selected label is not retained", source)
                })?
            };
            let edge = self.draft.edge(target, &[]);
            self.draft
                .terminate_and_continue_dead(Terminator::Jump(edge));
            self.switch_starts.push(start);
            return Ok(());
        }
        let value = value.expect("runtime switch evaluated its condition");
        // GNU inclusive ranges use one unsigned interval comparison each,
        // never an expansion proportional to the number of covered values.
        // GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>.
        let default = default.expect("default or no-match transition has a block");
        if ranges.is_empty() {
            let default = self.draft.edge(default, &[]);
            self.draft.terminate_and_continue_dead(Terminator::Switch {
                value,
                default,
                cases,
            });
        } else {
            let first = self.draft.create_block(&[]);
            let edge = self.draft.edge(first, &[]);
            self.draft.terminate(Terminator::Switch {
                value,
                default: edge,
                cases,
            });
            self.draft.seal(first);
            self.draft.switch_to(first);
            for (lower, upper, target) in ranges {
                let next = self.draft.create_block(&[]);
                let low = self.iconst(ty, lower);
                let span = self.iconst(ty, upper.wrapping_sub(lower));
                let offset = self.binary_inst(Opcode::Isub, InstFlags::empty(), value, low);
                let condition = self.icmp(crate::ir::IntCC::Ule, offset, span);
                self.draft.brif(condition, target, next);
                self.draft.seal(next);
                self.draft.switch_to(next);
            }
            let edge = self.draft.edge(default, &[]);
            self.draft
                .terminate_and_continue_dead(Terminator::Jump(edge));
        }
        self.switch_starts.push(start);
        Ok(())
    }

    /// Ends a switch body: falls into the exit and seals the case blocks,
    /// whose predecessors are now all known.
    pub(super) fn end_switch(&mut self, _statement: &'tu Statement<'tu>, exit: Block) {
        self.draft.jump(exit, &[]);
        let start = self.switch_starts.pop().expect("a switch is open");
        for index in start..self.switch_blocks.len() {
            let block = self.switch_blocks[index];
            self.draft.seal(block);
        }
        self.switch_blocks.truncate(start);
        self.draft.seal(exit);
        self.draft.switch_to(exit);
    }

    fn case_block(&mut self, label: LabelId) -> Block {
        let block = self.label_block(label);
        self.switch_blocks.push(block);
        block
    }

    fn dispatch_target(
        &mut self,
        target: Block,
        transition: Transition,
        source: SourceVectors,
    ) -> Result<Block, LoweringError> {
        if !self.transition_has_cleanup(transition) {
            return Ok(target);
        }
        let current = self.draft.current();
        let trampoline = self.draft.create_block(&[]);
        self.draft.switch_to(trampoline);
        self.cleanup_transition(transition, source)?;
        self.draft.jump(target, &[]);
        self.draft.switch_to(current);
        Ok(trampoline)
    }

    /// Pops the returned value, converted to the result type, and returns.
    /// A structure or union result is copied into the object the caller
    /// passed as parameter 0.
    /// C99: §6.8.6.4 paragraph 3, p. 139; PDF p. 151.
    pub(super) fn return_value(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) -> Result<(), LoweringError> {
        let item = self.pop();
        let source = self.record.syntax.source_vectors;
        let value = match self.repr(self.result, source)? {
            | Repr::Void => None,
            | Repr::Complex { .. } => {
                let to = self.draft.block_params(Draft::entry())[0];
                self.store_complex(to, item, source)?;
                None
            },
            | Repr::Aggregate => {
                let Operand::Aggregate(from) = item.operand else {
                    return Err(LoweringError::missing(
                        "a returned structure has no address",
                        source,
                    ));
                };
                let to = *self
                    .draft
                    .block_params(Draft::entry())
                    .first()
                    .ok_or_else(|| LoweringError::missing("a result has no object", source))?;
                let layout = types::layout(&self.unit.sema.types, self.result)
                    .map_err(|kind| at(kind, source))?;
                let size = self.iconst(Type::I64, i128::from(layout.size));
                _ = self.draft.push(
                    self.memory_op(InstData::MemoryRange {
                        opcode: Opcode::Copy,
                        flags:  InstFlags::empty(),
                        align:  self.place_alignment(from, crate::ir::Align::BYTE),
                        args:   [to, from, size],
                    }),
                    None,
                );
                None
            },
            | _ => Some(self.value(item, source)?),
        };
        _ = self.cleanup_jump(statement)?;
        self.draft
            .terminate_and_continue_dead(Terminator::Return(value));
        Ok(())
    }

    /// Pops a computed goto's `const void *` operand and passes it to the
    /// function's one indirect-branch block, as Clang does, so each label
    /// block has one indirect predecessor however many gotos there are. Like
    /// Clang, the jump releases no variable length array storage.
    /// GNU extension: GCC "Labels as Values".
    pub(super) fn indirect_jump(
        &mut self,
        statement: &'tu Statement<'tu>,
    ) -> Result<(), LoweringError> {
        let source = statement.source_vectors;
        let item = self.pop();
        let address = self.value(item, source)?;
        if !matches!(
            self.cleanup_jump(statement)?,
            ControlTarget::IndirectGoto(_)
        ) {
            return Err(LoweringError::missing(
                "a computed goto has no indirect jump facts",
                source,
            ));
        }
        let dispatch = self.indirect_block();
        let edge = self.draft.edge(dispatch, &[address]);
        self.draft
            .terminate_and_continue_dead(Terminator::Jump(edge));
        Ok(())
    }

    /// The block every computed goto passes its address to, ending in an
    /// indirect branch to each address-taken label, made on first use.
    fn indirect_block(&mut self) -> Block {
        if let Some(block) = self.indirect {
            return block;
        }
        let current = self.draft.current();
        let block = self.draft.create_block(&[Type::Ptr]);
        let address = self.draft.block_params(block)[0];
        let mut edges = ArenaVec::with_capacity_in(self.draft.labels.len(), self.scratch);
        for index in 0..self.draft.labels.len() {
            edges.push(self.draft.edge(self.draft.labels[index], &[]));
        }
        self.draft.switch_to(block);
        self.draft.terminate(Terminator::IndirectBr(address, edges));
        self.draft.switch_to(current);
        self.indirect = Some(block);
        block
    }

    /// The IR label of `label`, an address-taken label of this function.
    /// A label of another function, which only a GNU nested function could
    /// name, is unsupported. GNU extension: GCC "Labels as Values".
    pub(super) fn block_label(
        &self,
        label: LabelId,
        source: SourceVectors,
    ) -> Result<crate::ir::BlockLabel, LoweringError> {
        let facts = self.unit.sema.control.labels[label.0];
        if facts.function != self.record.control.id {
            return Err(LoweringError::unsupported(Construct::LabelAddress, source));
        }
        let index = facts.address.ok_or_else(|| {
            LoweringError::missing("an addressed label has no address index", source)
        })?;
        Ok(crate::ir::BlockLabel::new(index as usize))
    }

    /// Jumps to `target`; what follows is unreachable until a label.
    fn jump_away(&mut self, target: Block) {
        let edge = self.draft.edge(target, &[]);
        self.draft
            .terminate_and_continue_dead(Terminator::Jump(edge));
    }

    fn label_block(&mut self, label: LabelId) -> Block {
        if let Some(&block) = self.labels.get(&label) {
            return block;
        }
        let facts = &self.unit.sema.control.labels[label.0];
        debug_assert_eq!(facts.id, label, "label lookup preserves resolved identity");
        debug_assert_eq!(
            facts.function, self.record.control.id,
            "ordinary labels stay within their function"
        );
        let block = self.draft.create_block(&[]);
        _ = self.labels.insert(label, block);
        block
    }

    /// Pushes `tasks` so that they run in slice order.
    pub(super) fn push_all(&mut self, tasks: &[Task<'tu>]) {
        self.tasks.extend(tasks.iter().rev().copied());
    }
}

// Declarations and initialization

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Allocates the objects a block-scope declaration declares and
    /// schedules their initializers in declaration order. A static local is
    /// a global initialized before the program starts.
    /// C99: §6.7.8 paragraphs 9-10, p. 126; PDF p. 138; §6.8 paragraph 3,
    /// p. 131; PDF p. 143; §6.2.4 paragraph 5, p. 32; PDF p. 44.
    pub(super) fn declaration(
        &mut self,
        declaration: &'tu Declaration<'tu>,
    ) -> Result<(), LoweringError> {
        let mut tasks = ArenaVec::new_in(self.scratch);
        let site = self
            .unit
            .sema
            .declaration_site(declaration)
            .ok_or_else(|| {
                LoweringError::missing(
                    "declaration type-site facts are not retained",
                    declaration.source_vectors,
                )
            })?;
        Self::prepare_site_tasks(site, &mut tasks);
        self.prepare_bound_tasks(site.bounds, &mut tasks, declaration.source_vectors)?;
        for init in declaration.init_declarators {
            let fact = self
                .unit
                .sema
                .correspondence
                .declaration(init.declarator)
                .ok_or_else(|| {
                    LoweringError::missing(
                        "declaration occurrence is not retained",
                        init.source_vectors,
                    )
                })?;
            if init.initializer.is_some() && fact.initializer.is_none() {
                return Err(LoweringError::missing(
                    "declaration initializer association is not retained",
                    init.source_vectors,
                ));
            }
            let binding = fact.binding.ok_or_else(|| {
                LoweringError::missing(
                    "declaration occurrence has no accepted binding",
                    init.source_vectors,
                )
            })?;
            let data = self.unit.sema.bindings[binding];
            if data.kind == BindingKind::Typedef || data.duration == Duration::Automatic {
                self.prepare_bound_tasks(fact.site.bounds, &mut tasks, init.source_vectors)?;
            }
            tasks.push(Task::DeclareBinding(
                binding,
                fact.initializer.filter(|_| !fact.initializer_elided),
                init.source_vectors,
            ));
        }
        self.push_all(&tasks);
        Ok(())
    }

    pub(super) fn declare_binding(
        &mut self,
        binding: usize,
        initializer: Option<
            &'tu crate::translation_phases::parsing::declaration_syntax::Initializer<'tu>,
        >,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let data = self.unit.sema.bindings[binding];
        if data.kind != BindingKind::Object {
            return Ok(());
        }
        if data.duration.is_static_or_thread() {
            // The module planner owns which static initializers are emitted;
            // retained locals also keep their initializer's referents alive.
            // C99 §6.2.4p3, p. 32; PDF p. 44; §6.9p3-5, p. 140; PDF p. 152.
            if data.linkage == Linkage::None && self.unit.reachable.contains(&binding) {
                _ = self
                    .unit
                    .define_object(binding, Some(self.name), initializer)?;
            }
            return Ok(());
        }
        _ = self.local(binding, source)?;
        let Some(initializer) = initializer else {
            return Ok(());
        };
        let object = Object::Local(binding);
        match initializer.kind {
            | InitializerType::AssignmentExpression(expression)
                if self.repr(data.ty, source)?.is_scalar() =>
            {
                self.tasks.push(Task::Initialize {
                    object,
                    offset: 0,
                    ty: data.ty,
                    bit_field: None,
                });
                self.tasks.push(Task::Evaluate(expression));
            },
            | _ => self.tasks.push(Task::InitializeAggregate {
                object,
                ty: data.ty,
                initializer,
            }),
        }
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
        // A structure or union result comes first, before the C parameters.
        let first = usize::from(signature.result_param().is_some());
        if record.parameter_facts.len() != parameters.len() {
            return Err(LoweringError::missing(
                "parameter correspondence is not retained",
                source,
            ));
        }
        let mut lowerer = Self {
            unit,
            record,
            name,
            draft,
            scratch,
            #[cfg(feature = "recursive-lowering-study")]
            recursion: super::recursive::State::new(false, false),
            tasks: ArenaVec::new_in(scratch),
            operands: ArenaVec::new_in(scratch),
            complex_values: ArenaVec::new_in(scratch),
            initializer_values: ArenaVec::new_in(scratch),
            alignments: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            locals: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            incoming_sizes: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            labels: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            targets: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            switch_blocks: ArenaVec::new_in(scratch),
            switch_starts: ArenaVec::new_in(scratch),
            result: record.result.unqualified(),
            bounds: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            vla_tokens: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            indirect: None,
        };
        // Address-taken labels become IR labels in their semantic address
        // order, so a label's address index is its IR label number, also in
        // static initializers lowered before this body.
        // GNU extension: GCC "Labels as Values".
        for &label in lowerer.unit.sema.control.address_taken(record.control.id) {
            let block = lowerer.label_block(label);
            lowerer.draft.labels.push(block);
        }
        let entry = Draft::entry();
        for (index, &value) in lowerer.draft.block_params(entry).iter().enumerate() {
            if let Some(layout) = signature
                .param_abi(index)
                .and_then(crate::ir::ParamAbi::layout)
            {
                _ = lowerer.alignments.insert(value, layout.align);
            }
        }
        let extension = lowerer.unit.sema.function_extensions.function(binding);
        let mut ir_index = first;
        for (index, facts) in record.parameter_facts.iter().enumerate() {
            let argument_index = ir_index;
            ir_index += 1;
            if extension.is_some_and(|extension| {
                extension
                    .sizes
                    .iter()
                    .any(|size| size.index as usize == index)
            }) {
                let size = *lowerer
                    .draft
                    .block_params(entry)
                    .get(ir_index)
                    .ok_or_else(|| {
                        LoweringError::missing("hidden size argument is not retained", source)
                    })?;
                ir_index += 1;
                if let Some(body) = facts.body_binding {
                    _ = lowerer.incoming_sizes.insert(body, size);
                }
                if let Some(prototype) = facts.prototype_binding {
                    _ = lowerer.incoming_sizes.insert(prototype, size);
                }
            }
            let Some(parameter) = facts.body_binding else {
                continue;
            };
            let Some(&value) = lowerer.draft.block_params(entry).get(argument_index) else {
                return Err(LoweringError::missing(
                    "a parameter has no argument",
                    source,
                ));
            };
            let declared = facts.declared;
            let incoming = facts.promoted;
            let parameter_source = lowerer.unit.sema.bindings[parameter].name.source_vectors;
            if matches!(
                lowerer.repr(declared, parameter_source)?,
                Repr::Aggregate | Repr::Complex { .. }
            ) {
                // C99 §6.9.1p10: the parameter object has its declared
                // storage alignment, independently of its ABI representation.
                let required =
                    super::alignment::storage_alignment(&lowerer.unit.sema.types, declared)
                        .map_err(|kind| at(kind, parameter_source))?;
                let provided = lowerer.place_alignment(value, crate::ir::Align::BYTE);
                let home = if provided.bytes() < required.bytes() {
                    let layout = types::layout(&lowerer.unit.sema.types, declared)
                        .map_err(|kind| at(kind, parameter_source))?;
                    let home = lowerer.fresh_object(layout.size, required, parameter_source)?;
                    let size = lowerer.iconst(Type::I64, i128::from(layout.size));
                    _ = lowerer.draft.push(
                        lowerer.memory_op(InstData::MemoryRange {
                            opcode: Opcode::Copy,
                            flags:  InstFlags::empty(),
                            align:  provided,
                            args:   [home, value, size],
                        }),
                        None,
                    );
                    home
                } else {
                    value
                };
                _ = lowerer.locals.insert(parameter, Local::Address(home));
                continue;
            }
            let argument = Item {
                operand: Operand::Value(value),
                ty:      incoming,
            };
            let converted =
                lowerer.convert_value(argument, declared.unqualified(), parameter_source)?;
            lowerer.operands.push(converted);
            lowerer.initialize(Object::Local(parameter), 0, declared, None)?;
        }
        // Bound expressions refer to prototype bindings. Semantic analysis
        // retains their body correspondence; no translation-unit scan or
        // source-position matching belongs in a per-function constructor.
        // C99: §6.9.1p10, p. 142; PDF p. 154.
        for facts in record.parameter_facts {
            let (Some(prototype), Some(body)) = (facts.prototype_binding, facts.body_binding)
            else {
                continue;
            };
            #[cfg(test)]
            {
                lowerer.unit.parameter_alias_visits += 1;
            }
            if let Some(&local) = lowerer.locals.get(&body) {
                _ = lowerer.locals.insert(prototype, local);
            }
        }
        lowerer.tasks.push(Task::Statement(record.syntax.body));
        let mut bound_tasks = ArenaVec::new_in(scratch);
        if record.old_style {
            for &site in record.parameter_sites {
                Self::prepare_site_tasks(site, &mut bound_tasks);
                lowerer.prepare_bound_tasks(site.bounds, &mut bound_tasks, source)?;
            }
        } else {
            for facts in record.parameter_facts {
                let site = facts.site.ok_or_else(|| {
                    LoweringError::missing("parameter entry site is not retained", source)
                })?;
                if site.ty != facts.original {
                    return Err(LoweringError::missing(
                        "parameter entry type does not match its original type",
                        source,
                    ));
                }
                Self::prepare_site_tasks(site, &mut bound_tasks);
                lowerer.prepare_bound_tasks(site.bounds, &mut bound_tasks, source)?;
            }
        }
        lowerer.push_all(&bound_tasks);
        Ok(lowerer)
    }

    /// Ends the body at its closing brace: `main` returns 0, any other
    /// function returns nothing or an indeterminate value, which its caller
    /// may not use (§6.9.1p12). Label blocks are sealed now that every
    /// `goto` is known.
    /// C99: §5.1.2.2.3 paragraph 1, p. 13; PDF p. 25.
    pub(super) fn finish_body(&mut self) -> Result<(), LoweringError> {
        let source = self.record.syntax.source_vectors;
        let value = match repr(&self.unit.sema.types, self.result)
            .map_err(|kind| at(kind, source))?
        {
            | Repr::Void | Repr::Aggregate | Repr::Complex { .. } => None,
            | representation => {
                let ty = representation
                    .value_type()
                    .ok_or_else(|| LoweringError::missing("a result has no value type", source))?;
                // A `main` whose result is not compatible with `int` returns
                // an unspecified status, so it gets no implicit zero.
                // C99 §5.1.2.2.3p1, p. 13; PDF p. 25.
                let main_zero = matches!(representation, Repr::Int { .. })
                    && self
                        .record
                        .binding
                        .and_then(|binding| self.unit.sema.function_policy(binding))
                        .ok_or_else(|| {
                            LoweringError::missing("function end policy is not retained", source)
                        })?
                        .end
                        == crate::translation_phases::semantic_analysis::FunctionEnd::MainZero;
                Some(if main_zero {
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
