//! Iterative lowering of the deliberately closed integer subset.
//! C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.8, pp. 131-139;
//! PDF pp. 143-151. Required sequencing stays in expression nodes.

use super::model::*;
use crate::{
    target::Scalar,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::{
            declaration_syntax::{
                Declaration,
                InitializerType,
            },
            syntax::{
                BinaryOperator as B,
                BlockItem,
                Expression as AstExpr,
                ExpressionSlot,
                ExpressionType as E,
                ForInitializer,
                Statement,
                StatementType as S,
                UnaryOperator as U,
            },
        },
        semantic_analysis::{
            BindingKind,
            Duration,
            FunctionRecord,
            SemanticTranslationUnit,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

pub(super) struct Lowerer<'s, 'tu, 'ir> {
    context:        &'s Context<'tu>,
    sema:           &'s SemanticTranslationUnit<'tu>,
    signatures:     &'s [Signature<'ir>],
    arena:          &'ir Bump,
    locals:         ArenaVec<'ir, Local<'ir>>,
    bindings:       ArenaMap<'ir, usize, usize>,
    expressions:    ArenaVec<'ir, Expression<'ir>>,
    expression_ids: ArenaMap<'ir, usize, usize>,
    code:           ArenaVec<'ir, Instruction>,
    labels:         ArenaVec<'ir, usize>,
    scopes:         usize,
}

impl<'s, 'tu, 'ir> Lowerer<'s, 'tu, 'ir> {
    /// Two work stacks keep source nesting off the Rust call stack.
    pub(super) fn run(
        context: &'s Context<'tu>,
        sema: &'s SemanticTranslationUnit<'tu>,
        record: &FunctionRecord<'tu>,
        signature: Signature<'ir>,
        signatures: &'s [Signature<'ir>],
        arena: &'ir Bump,
    ) -> Result<Function<'ir>, Error> {
        let mut this = Self {
            context,
            sema,
            signatures,
            arena,
            locals: ArenaVec::new_in(arena),
            bindings: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            expressions: ArenaVec::new_in(arena),
            expression_ids: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            code: ArenaVec::new_in(arena),
            labels: ArenaVec::new_in(arena),
            scopes: 1,
        };
        for facts in record.parameter_facts {
            let binding = facts
                .body_binding
                .ok_or_else(|| unsupported("unnamed parameter", signature.origin))?;
            _ = this.local(binding, 0, true, sema.bindings[binding].name.source_vectors)?;
        }
        this.emit(InstructionKind::Enter(0), signature.origin);
        let mut tasks = ArenaVec::new_in(arena);
        tasks.push(Task::Statement(
            record.syntax.body,
            State {
                depth:   1,
                loop_to: None,
            },
        ));
        while let Some(task) = tasks.pop() {
            match task {
                | Task::Emit(kind, origin) => this.emit(kind, origin),
                | Task::Label(id) => this.labels[id] = this.code.len(),
                | Task::Declaration(decl, scope) => this.declaration(decl, scope)?,
                | Task::Statement(statement, state) =>
                    this.statement(statement, state, &mut tasks)?,
            }
        }
        this.emit(InstructionKind::Fallthrough, signature.origin);
        Ok(Function {
            signature,
            locals: arena.alloc_slice_copy(&this.locals),
            expressions: arena.alloc_slice_copy(&this.expressions),
            code: arena.alloc_slice_copy(&this.code),
            labels: arena.alloc_slice_copy(&this.labels),
            scopes: this.scopes,
        })
    }

    /// C99: §6.8.2-6.8.6, pp. 132-139; PDF pp. 144-151.
    fn statement(
        &mut self,
        statement: &'tu Statement<'tu>,
        state: State,
        tasks: &mut ArenaVec<'ir, Task<'tu>>,
    ) -> Result<(), Error> {
        let origin = statement.source_vectors;
        let emit_task = |kind| Task::Emit(kind, origin);
        match statement.kind {
            | S::Compound { items } => {
                let scope = self.scope();
                self.emit(InstructionKind::Enter(scope), origin);
                tasks.push(emit_task(InstructionKind::Exit));
                let inner = State {
                    depth: state.depth + 1,
                    ..state
                };
                for item in items.iter().rev() {
                    tasks.push(match item {
                        | BlockItem::Statement(s) => Task::Statement(s, inner),
                        | BlockItem::Declaration(d) => Task::Declaration(d, scope),
                        | BlockItem::FunctionDefinition(_) =>
                            return Err(unsupported("nested function", origin)),
                    });
                }
            },
            | S::Declaration(_) =>
                return Err(unsupported("declaration statement extension", origin)),
            | S::Expression(slot) => {
                let id = self.expression(slot_expr(slot, origin)?)?;
                self.emit(InstructionKind::Evaluate(id), origin);
            },
            | S::Null => {},
            | S::Return(Some(slot)) => {
                let id = self.expression(slot_expr(slot, origin)?)?;
                self.emit(InstructionKind::Return(id), origin);
            },
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => {
                let condition = self.expression(slot_expr(condition_expression, origin)?)?;
                let then = self.label();
                let otherwise = self.label();
                let join = self.label();
                self.emit(
                    InstructionKind::Branch {
                        condition,
                        then,
                        otherwise,
                    },
                    origin,
                );
                tasks.push(Task::Label(join));
                if let Some(s) = else_statement {
                    tasks.push(Task::Statement(s, state));
                }
                tasks.push(Task::Label(otherwise));
                tasks.push(emit_task(InstructionKind::Jump {
                    label: join,
                    keep:  state.depth,
                }));
                tasks.push(Task::Statement(then_statement, state));
                tasks.push(Task::Label(then));
            },
            | S::While {
                condition_expression,
                body_statement,
            }
            | S::DoWhile {
                condition_expression,
                body_statement,
            } => {
                let condition = self.expression(slot_expr(condition_expression, origin)?)?;
                let header = self.label();
                let body = self.label();
                let exit = self.label();
                let do_while = matches!(statement.kind, S::DoWhile { .. });
                let inner = State {
                    loop_to: Some((exit, header, state.depth)),
                    ..state
                };
                tasks.push(Task::Label(exit));
                tasks.push(emit_task(InstructionKind::Branch {
                    condition,
                    then: body,
                    otherwise: exit,
                }));
                tasks.push(Task::Label(header));
                tasks.push(Task::Statement(body_statement, inner));
                tasks.push(Task::Label(body));
                if !do_while {
                    self.emit(
                        InstructionKind::Jump {
                            label: header,
                            keep:  state.depth,
                        },
                        origin,
                    );
                }
            },
            | S::For(f) => {
                let scope = self.scope();
                let depth = state.depth + 1;
                self.emit(InstructionKind::Enter(scope), origin);
                if let Some(init) = f.initializer {
                    match init {
                        | ForInitializer::Declaration(d) => self.declaration(d, scope)?,
                        | ForInitializer::Expression(slot) => {
                            let id = self.expression(slot_expr(slot, origin)?)?;
                            self.emit(InstructionKind::Evaluate(id), origin);
                        },
                    }
                }
                let header = self.label();
                let body = self.label();
                let next = self.label();
                let exit = self.label();
                self.labels[header] = self.code.len();
                if let Some(slot) = f.condition_expression {
                    let condition = self.expression(slot_expr(slot, origin)?)?;
                    self.emit(
                        InstructionKind::Branch {
                            condition,
                            then: body,
                            otherwise: exit,
                        },
                        origin,
                    );
                }
                tasks.push(emit_task(InstructionKind::Exit));
                tasks.push(Task::Label(exit));
                tasks.push(emit_task(InstructionKind::Jump {
                    label: header,
                    keep:  depth,
                }));
                if let Some(slot) = f.iteration_expression {
                    let id = self.expression(slot_expr(slot, origin)?)?;
                    tasks.push(emit_task(InstructionKind::Evaluate(id)));
                }
                tasks.push(Task::Label(next));
                tasks.push(Task::Statement(
                    f.body_statement,
                    State {
                        depth,
                        loop_to: Some((exit, next, depth)),
                    },
                ));
                tasks.push(Task::Label(body));
            },
            | S::Break | S::Continue => {
                let (exit, next, keep) = state
                    .loop_to
                    .ok_or_else(|| unsupported("jump outside supported loop", origin))?;
                self.emit(
                    InstructionKind::Jump {
                        label: if matches!(statement.kind, S::Break) {
                            exit
                        } else {
                            next
                        },
                        keep,
                    },
                    origin,
                );
            },
            | _ => return Err(unsupported("statement outside integer slice", origin)),
        }
        Ok(())
    }

    /// C99: §6.2.4p5, p. 32; PDF p. 44; §6.7.8p11, p. 126; PDF p. 138.
    fn declaration(&mut self, decl: &'tu Declaration<'tu>, scope: usize) -> Result<(), Error> {
        let origin = decl.source_vectors;
        if decl.assertion.is_some() || decl.declaration_specifiers.extensions.is_some() {
            return Err(unsupported("declaration extension", origin));
        }
        let site = self
            .sema
            .declaration_site(decl)
            .ok_or_else(|| unsupported("missing declaration facts", origin))?;
        if !site.bounds.is_empty() || !site.operand_evaluations.is_empty() {
            return Err(unsupported("runtime type-site evaluation", origin));
        }
        for init in decl.init_declarators {
            let fact = self
                .sema
                .correspondence
                .declaration(init.declarator)
                .ok_or_else(|| unsupported("missing binding", origin))?;
            let binding = fact
                .binding
                .ok_or_else(|| unsupported("missing binding", origin))?;
            let data = self.sema.bindings[binding];
            if data.kind == BindingKind::Typedef {
                if self.sema.types.variably_modified(data.ty) {
                    return Err(unsupported("variably modified typedef", origin));
                }
                continue;
            }
            if data.kind != BindingKind::Object || data.duration != Duration::Automatic {
                return Err(unsupported("nonautomatic declaration", origin));
            }
            if !fact.site.bounds.is_empty() || !fact.site.operand_evaluations.is_empty() {
                return Err(unsupported("runtime declarator evaluation", origin));
            }
            let local = self.local(binding, scope, false, init.source_vectors)?;
            let initializer = match fact.initializer {
                | None => None,
                | Some(i) => match i.kind {
                    | InitializerType::AssignmentExpression(e) => Some(self.expression(e)?),
                    | _ => return Err(unsupported("braced initializer", origin)),
                },
            };
            self.emit(
                InstructionKind::Declare { local, initializer },
                init.source_vectors,
            );
        }
        Ok(())
    }

    fn local(
        &mut self,
        binding: usize,
        scope: usize,
        parameter: bool,
        origin: SourceVectors,
    ) -> Result<usize, Error> {
        if let Some(&id) = self.bindings.get(&binding) {
            return Ok(id);
        }
        if self.locals.len() == 128 {
            return Err(unsupported("more than 128 logical locals", origin));
        }
        let data = self.sema.bindings[binding];
        let ty = IntType::snapshot(self.sema, data.ty)?;
        let id = self.locals.len();
        self.locals.push(Local {
            binding,
            name: self
                .arena
                .alloc_str(self.context.string_cache.at(data.name.name)),
            ty,
            scope,
            parameter,
            origin,
        });
        _ = self.bindings.insert(binding, id);
        Ok(id)
    }

    /// C99: §6.3, pp. 42-48; PDF pp. 54-60; §6.5, pp. 67-94; PDF pp. 79-106.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Sema integer constants are encoded as low bits of the supported target type."
    )]
    fn expression(&mut self, root: &'tu AstExpr<'tu>) -> Result<usize, Error> {
        let mut tasks = ArenaVec::new_in(self.arena);
        tasks.push((root, false));
        while let Some((e, finish)) = tasks.pop() {
            let ordinal = self
                .sema
                .expression_index(e)
                .ok_or_else(|| unsupported("missing expression facts", e.source_vectors))?;
            if self.expression_ids.contains_key(&ordinal) {
                continue;
            }
            if !finish {
                tasks.push((e, true));
                match e.kind {
                    | E::Binary {
                        left_expression: a,
                        right_expression: b,
                        ..
                    } => {
                        tasks.push((b, false));
                        tasks.push((a, false));
                    },
                    | E::Unary {
                        operand_expression: a,
                        ..
                    }
                    | E::Parenthesized { expression: a }
                    | E::Cast {
                        operand_expression: a,
                        ..
                    } => tasks.push((a, false)),
                    | E::Conditional(c) => {
                        tasks.push((c.else_expression, false));
                        tasks.push((c.then_expression, false));
                        tasks.push((c.condition_expression, false));
                    },
                    | E::Call { arguments, .. } =>
                        for &arg in arguments.iter().rev() {
                            tasks.push((arg, false));
                        },
                    | _ => {},
                }
                continue;
            }
            let info = self
                .sema
                .expression_info(e)
                .ok_or_else(|| unsupported("missing expression type", e.source_vectors))?;
            let ty = IntType::snapshot(self.sema, info.ty)?;
            let child = |a| {
                self.sema
                    .expression_index(a)
                    .and_then(|i| self.expression_ids.get(&i))
                    .copied()
                    .ok_or_else(|| unsupported("missing child", e.source_vectors))
            };
            let place = |a| {
                self.sema
                    .designation(a)
                    .and_then(|d| d.binding)
                    .and_then(|b| self.bindings.get(&b))
                    .copied()
                    .ok_or_else(|| unsupported("nonlocal or noninteger place", e.source_vectors))
            };
            let kind = match e.kind {
                | E::Constant(_) | E::SizeofType(_) | E::SizeofExpr(_) => {
                    let value = info
                        .integer
                        .ok_or_else(|| unsupported("nonconstant integer leaf", e.source_vectors))?;
                    ExpressionKind::Constant((value.value as u64) & ty.mask())
                },
                | E::Identifier(_) => {
                    if let Some(binding) = info.binding
                        && self.sema.bindings[binding].kind == BindingKind::Enumerator
                    {
                        ExpressionKind::Constant(
                            info.integer
                                .ok_or_else(|| {
                                    unsupported("enumerator without value", e.source_vectors)
                                })?
                                .value as u64,
                        )
                    } else {
                        ExpressionKind::Read(place(e)?)
                    }
                },
                | E::Parenthesized { expression: a }
                | E::Cast {
                    operand_expression: a,
                    ..
                } => ExpressionKind::Convert(child(a)?),
                | E::Unary {
                    operator,
                    operand_expression: a,
                } => match operator {
                    | U::Plus => ExpressionKind::Convert(child(a)?),
                    | U::Minus | U::LogicalNot | U::BitwiseNot => ExpressionKind::Unary(
                        match operator {
                            | U::Minus => Unary::Negate,
                            | U::LogicalNot => Unary::Not,
                            | _ => Unary::Invert,
                        },
                        child(a)?,
                    ),
                    | U::PreIncrement | U::PostIncrement | U::PreDecrement | U::PostDecrement => {
                        let local = place(a)?;
                        let promoted = if ty.bits < 32 {
                            IntType {
                                scalar: Scalar::Int,
                                bits:   32,
                                signed: true,
                            }
                        } else {
                            ty
                        };
                        ExpressionKind::Update {
                            local,
                            increment: matches!(operator, U::PreIncrement | U::PostIncrement),
                            postfix: matches!(operator, U::PostIncrement | U::PostDecrement),
                            promoted,
                        }
                    },
                    | _ => return Err(unsupported("unary operation", e.source_vectors)),
                },
                | E::Binary {
                    operator,
                    left_expression: a,
                    right_expression: b,
                } => {
                    let (left, right) = (child(a)?, child(b)?);
                    match operator {
                        | B::Comma => ExpressionKind::Sequence(left, right),
                        | B::LogicalAnd | B::LogicalOr => ExpressionKind::Logical {
                            left,
                            right,
                            and: operator == B::LogicalAnd,
                        },
                        | B::Assignment => ExpressionKind::Set {
                            local:     place(a)?,
                            value:     right,
                            operation: None,
                        },
                        | op if assignment(op).is_some() => ExpressionKind::Set {
                            local:     place(a)?,
                            value:     right,
                            operation: Some((
                                assignment(op).expect("guarded"),
                                IntType::snapshot(
                                    self.sema,
                                    info.operation_type.ok_or_else(|| {
                                        unsupported("compound operation type", e.source_vectors)
                                    })?,
                                )?,
                            )),
                        },
                        | op => ExpressionKind::Binary(
                            binary(op)
                                .ok_or_else(|| unsupported("binary operation", e.source_vectors))?,
                            left,
                            right,
                        ),
                    }
                },
                | E::Conditional(c) => ExpressionKind::Conditional {
                    condition: child(c.condition_expression)?,
                    then:      child(c.then_expression)?,
                    otherwise: child(c.else_expression)?,
                },
                | E::Call {
                    function_expression,
                    arguments,
                } => {
                    let call = self
                        .sema
                        .call_facts(e)
                        .ok_or_else(|| unsupported("missing call facts", e.source_vectors))?;
                    if !call.prototype || call.variadic {
                        return Err(unsupported(
                            "nonprototype or variadic call",
                            e.source_vectors,
                        ));
                    }
                    let binding = self
                        .sema
                        .designation(function_expression)
                        .and_then(|d| d.binding)
                        .ok_or_else(|| unsupported("indirect call", e.source_vectors))?;
                    let entity = self
                        .sema
                        .correspondence
                        .entity_of(binding, self.sema.bindings[binding])
                        .0;
                    let function = self
                        .signatures
                        .iter()
                        .position(|s| s.entity == entity)
                        .ok_or_else(|| unsupported("external call", e.source_vectors))?;
                    let mut args = ArenaVec::new_in(self.arena);
                    for &a in arguments {
                        args.push(child(a)?);
                    }
                    ExpressionKind::Call {
                        function,
                        arguments: self.arena.alloc_slice_copy(&args),
                    }
                },
                | _ =>
                    return Err(unsupported(
                        "expression outside integer slice",
                        e.source_vectors,
                    )),
            };
            let mut id = self.push_expr(kind, ty, e.source_vectors);
            for conversion in self.sema.expression_conversions(e) {
                let target = IntType::snapshot(self.sema, conversion.ty)?;
                if target != self.expressions[id].ty {
                    id = self.push_expr(ExpressionKind::Convert(id), target, e.source_vectors);
                }
            }
            _ = self.expression_ids.insert(ordinal, id);
        }
        Ok(self.expression_ids[&self.sema.expression_index(root).expect("checked")])
    }

    fn push_expr(
        &mut self,
        kind: ExpressionKind<'ir>,
        ty: IntType,
        origin: SourceVectors,
    ) -> usize {
        let id = self.expressions.len();
        self.expressions.push(Expression { kind, ty, origin });
        id
    }

    fn emit(&mut self, kind: InstructionKind, origin: SourceVectors) {
        self.code.push(Instruction { kind, origin });
    }

    fn label(&mut self) -> usize {
        let id = self.labels.len();
        self.labels.push(usize::MAX);
        id
    }

    fn scope(&mut self) -> usize {
        let id = self.scopes;
        self.scopes += 1;
        id
    }
}

#[derive(Clone, Copy)]
struct State {
    depth:   usize,
    loop_to: Option<(usize, usize, usize)>,
}

enum Task<'tu> {
    Statement(&'tu Statement<'tu>, State),
    Declaration(&'tu Declaration<'tu>, usize),
    Emit(InstructionKind, SourceVectors),
    Label(usize),
}

fn slot_expr(slot: ExpressionSlot<'_>, origin: SourceVectors) -> Result<&AstExpr<'_>, Error> {
    match slot {
        | ExpressionSlot::Parsed(e) => Ok(e),
        | _ => Err(unsupported("missing or extended expression slot", origin)),
    }
}

fn unsupported(reason: &'static str, origin: SourceVectors) -> Error {
    Error::new(Failure::Unsupported(reason), Some(origin))
}

fn assignment(op: B) -> Option<Binary> {
    Some(match op {
        | B::AdditionAssignment => Binary::Add,
        | B::SubtractionAssignment => Binary::Subtract,
        | B::MultiplicationAssignment => Binary::Multiply,
        | B::DivisionAssignment => Binary::Divide,
        | B::ModuloAssignment => Binary::Remainder,
        | B::LeftShiftAssignment => Binary::ShiftLeft,
        | B::RightShiftAssignment => Binary::ShiftRight,
        | B::BitwiseAndAssignment => Binary::And,
        | B::BitwiseOrAssignment => Binary::Or,
        | B::BitwiseXorAssignment => Binary::Xor,
        | _ => return None,
    })
}

fn binary(op: B) -> Option<Binary> {
    Some(match op {
        | B::Addition => Binary::Add,
        | B::Subtraction => Binary::Subtract,
        | B::Multiplication => Binary::Multiply,
        | B::Division => Binary::Divide,
        | B::Modulo => Binary::Remainder,
        | B::LeftShift => Binary::ShiftLeft,
        | B::RightShift => Binary::ShiftRight,
        | B::BitwiseAnd => Binary::And,
        | B::BitwiseOr => Binary::Or,
        | B::BitwiseXor => Binary::Xor,
        | B::Equal => Binary::Equal,
        | B::NotEqual => Binary::NotEqual,
        | B::LessThan => Binary::Less,
        | B::LessThanOrEqual => Binary::LessEqual,
        | B::GreaterThan => Binary::Greater,
        | B::GreaterThanOrEqual => Binary::GreaterEqual,
        | _ => return None,
    })
}
