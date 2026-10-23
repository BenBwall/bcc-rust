//! Bounded execution with explicit activation, object and expression stacks.
//! C99: §6.2.4p5, p. 32; PDF p. 44; §6.5p2,5, p. 67; PDF p. 79;
//! §6.5.2.2p10, p. 72; PDF p. 84; §6.8.6.4p3, p. 139; PDF p. 151.
//! Indeterminate reads stop as policy-dependent execution, not mandatory ISO
//! UB. Conflicts spanning nested sequence points are explicitly unsupported.

use super::{
    arithmetic::{
        self,
        Value,
    },
    model::*,
};
use crate::{
    translation_phases::SourceVectors,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// One deterministic witness. The trace is retained on failure as well.
pub(super) fn evaluate<'a>(
    module: &Module<'_>,
    entry: &str,
    args: &[Value],
    limits: Limits,
    arena: &'a Bump,
) -> Execution<'a> {
    let mut machine = Machine {
        module,
        arena,
        limits,
        frames: ArenaVec::new_in(arena),
        locals: ArenaVec::new_in(arena),
        scopes: ArenaVec::new_in(arena),
        tasks: ArenaVec::new_in(arena),
        values: ArenaVec::new_in(arena),
        events: ArenaVec::new_in(arena),
        steps: 0,
        next_activation: 0,
        next_instance: 0,
        origin: None,
    };
    let result = (|| {
        module.verify().map_err(|e| e.failure)?;
        let function = module
            .functions
            .iter()
            .position(|f| f.signature.name == entry)
            .ok_or(Failure::Unsupported("entry function"))?;
        machine.values.extend(args.iter().map(|&value| Evaluated {
            value,
            effects: Effects::default(),
        }));
        machine.enter(function, args.len())?;
        machine.run()
    })();
    let result = result.map_err(|failure| Error::new(failure, machine.origin));
    if let Err(error) = result
        && machine.events.len() < limits.events
    {
        machine.events.push(Event {
            activation: machine.frames.last().map_or(0, |f| f.activation),
            kind:       EventKind::Failure(error.failure),
            origin:     error.origin,
        });
    }
    Execution {
        result,
        events: arena.alloc_slice_copy(&machine.events),
        steps: machine.steps,
    }
}

struct Machine<'m, 'ir, 'a> {
    module:          &'m Module<'ir>,
    arena:           &'a Bump,
    limits:          Limits,
    frames:          ArenaVec<'a, Frame>,
    locals:          ArenaVec<'a, Cell>,
    scopes:          ArenaVec<'a, usize>,
    tasks:           ArenaVec<'a, Action>,
    values:          ArenaVec<'a, Evaluated>,
    events:          ArenaVec<'a, Event>,
    steps:           usize,
    next_activation: u64,
    next_instance:   u64,
    origin:          Option<SourceVectors>,
}

impl<'ir> Machine<'_, 'ir, '_> {
    fn run(&mut self) -> Result<Value, Failure> {
        while let Some(action) = self.tasks.pop() {
            if self.steps == self.limits.steps {
                return Err(Failure::Limit("steps"));
            }
            self.steps += 1;
            if let Some(value) = self.step(action)? {
                return Ok(value);
            }
        }
        Err(Failure::InvalidIr)
    }

    fn step(&mut self, action: Action) -> Result<Option<Value>, Failure> {
        match action {
            | Action::Step => {
                let frame = self.frames.last_mut().ok_or(Failure::InvalidIr)?;
                let f = &self.module.functions[frame.function];
                let op = *f.code.get(frame.pc).ok_or(Failure::InvalidIr)?;
                frame.pc += 1;
                self.origin = Some(op.origin);
                match op.kind {
                    | InstructionKind::Enter(scope) => {
                        self.scopes.push(scope);
                        let frame = *self.frame()?;
                        for (id, local) in f.locals.iter().enumerate() {
                            if local.scope != scope {
                                continue;
                            }
                            let cell = self
                                .locals
                                .get_mut(frame.base + id)
                                .ok_or(Failure::InvalidIr)?;
                            if cell.live {
                                return Err(Failure::InvalidIr);
                            }
                            self.next_instance += 1;
                            cell.instance = self.next_instance;
                            cell.live = true;
                            if !local.parameter {
                                cell.value = None;
                            }
                            self.event(
                                EventKind::Begin {
                                    local:    id,
                                    instance: self.next_instance,
                                },
                                Some(local.origin),
                            )?;
                        }
                        self.tasks.push(Action::Step);
                    },
                    | InstructionKind::Exit => {
                        let keep = self
                            .scopes
                            .len()
                            .checked_sub(self.frame()?.scope_base + 1)
                            .ok_or(Failure::InvalidIr)?;
                        self.unwind(keep)?;
                        self.tasks.push(Action::Step);
                    },
                    | InstructionKind::Declare { local, initializer } => {
                        self.cell_mut(local)?.value = None;
                        self.event(EventKind::Indeterminate { local }, self.origin)?;
                        if let Some(e) = initializer {
                            self.tasks.push(Action::Initialize(local));
                            self.tasks.push(Action::Expr(e));
                        } else {
                            self.boundary()?;
                            self.tasks.push(Action::Step);
                        }
                    },
                    | InstructionKind::Evaluate(e) => {
                        self.tasks.push(Action::Discard);
                        self.tasks.push(Action::Expr(e));
                    },
                    | InstructionKind::Branch {
                        condition,
                        then,
                        otherwise,
                    } => {
                        self.tasks.push(Action::Branch(then, otherwise));
                        self.tasks.push(Action::Expr(condition));
                    },
                    | InstructionKind::Jump { label, keep } => {
                        self.unwind(keep)?;
                        self.jump(label)?;
                        self.tasks.push(Action::Step);
                    },
                    | InstructionKind::Return(e) => {
                        self.tasks.push(Action::Return);
                        self.tasks.push(Action::Expr(e));
                    },
                    | InstructionKind::Fallthrough => {
                        if f.signature.name != "main" {
                            return Err(Failure::Unsupported("non-main fallthrough"));
                        }
                        self.values.push(Evaluated {
                            value:   Value::new(f.signature.result, 0),
                            effects: Effects::default(),
                        });
                        self.tasks.push(Action::Return);
                    },
                }
            },
            | Action::Expr(id) => {
                let e = *self
                    .function()?
                    .expressions
                    .get(id)
                    .ok_or(Failure::InvalidIr)?;
                self.origin = Some(e.origin);
                match e.kind {
                    | ExpressionKind::Constant(bits) =>
                        self.value(Value::new(e.ty, bits), Effects::default()),
                    | ExpressionKind::Read(local) => {
                        let value = self.read(local)?.cast(e.ty);
                        self.value(
                            value,
                            Effects {
                                reads: 1 << local,
                                ..Effects::default()
                            },
                        );
                    },
                    | ExpressionKind::Convert(a) | ExpressionKind::Unary(_, a) => {
                        self.tasks.push(Action::Finish(id));
                        self.tasks.push(Action::Expr(a));
                    },
                    | ExpressionKind::Binary(_, a, b) | ExpressionKind::Sequence(a, b) => {
                        self.tasks.push(Action::Finish(id));
                        self.tasks.push(Action::Second(id));
                        self.tasks.push(Action::Expr(a));
                        _ = b;
                    },
                    | ExpressionKind::Logical { left, .. }
                    | ExpressionKind::Conditional {
                        condition: left, ..
                    } => {
                        self.tasks.push(Action::Choose(id));
                        self.tasks.push(Action::Expr(left));
                    },
                    | ExpressionKind::Set { value, .. } => {
                        self.tasks.push(Action::Finish(id));
                        self.tasks.push(Action::Expr(value));
                    },
                    | ExpressionKind::Update {
                        local,
                        increment,
                        postfix,
                        promoted,
                    } => {
                        let old = self.read(local)?;
                        let value = arithmetic::binary(
                            if increment {
                                Binary::Add
                            } else {
                                Binary::Subtract
                            },
                            old.cast(promoted),
                            Value::new(promoted, 1),
                            promoted,
                        )?
                        .cast(old.ty);
                        self.write(local, value)?;
                        self.value(
                            if postfix { old } else { value },
                            Effects {
                                reads:       1 << local,
                                writes:      1 << local,
                                tail_writes: 1 << local,
                                sequence:    false,
                            },
                        );
                    },
                    | ExpressionKind::Call {
                        function,
                        arguments,
                    } => {
                        self.tasks.push(Action::Invoke(function, arguments.len()));
                        for &arg in arguments.iter().rev() {
                            self.tasks.push(Action::Expr(arg));
                        }
                    },
                }
            },
            | Action::Second(id) => {
                let e = self.function()?.expressions[id];
                self.origin = Some(e.origin);
                let child = match e.kind {
                    | ExpressionKind::Binary(_, _, b) => b,
                    | ExpressionKind::Sequence(_, b) => {
                        self.boundary()?;
                        b
                    },
                    | _ => return Err(Failure::InvalidIr),
                };
                self.tasks.push(Action::Expr(child));
            },
            | Action::Finish(id) => {
                let e = self.function()?.expressions[id];
                self.origin = Some(e.origin);
                let b = self.pop()?;
                match e.kind {
                    | ExpressionKind::Convert(_) => self.value(b.value.cast(e.ty), b.effects),
                    | ExpressionKind::Unary(op, _) =>
                        self.value(arithmetic::unary(op, b.value, e.ty)?, b.effects),
                    | ExpressionKind::Binary(op, _, _) => {
                        let a = self.pop()?;
                        let effects = a.effects.unsequenced(b.effects)?;
                        self.value(arithmetic::binary(op, a.value, b.value, e.ty)?, effects);
                    },
                    | ExpressionKind::Sequence(_, _) => {
                        let a = self.pop()?;
                        self.value(b.value.cast(e.ty), a.effects.sequenced(b.effects));
                    },
                    | ExpressionKind::Set {
                        local, operation, ..
                    } => {
                        let bit = 1 << local;
                        if b.effects.tail_writes & bit != 0 {
                            return Err(Failure::Undefined(Undefined::UnsequencedAccess));
                        }
                        let mut effects = b.effects;
                        let value = if let Some((op, ty)) = operation {
                            if effects.sequence && effects.writes & bit != 0 {
                                return Err(Failure::Unsupported(
                                    "compound assignment across sequence points",
                                ));
                            }
                            let old = self.read(local)?.cast(ty);
                            effects.reads |= bit;
                            arithmetic::binary(
                                op,
                                old,
                                if matches!(op, Binary::ShiftLeft | Binary::ShiftRight) {
                                    b.value
                                } else {
                                    b.value.cast(ty)
                                },
                                ty,
                            )?
                        } else {
                            b.value
                        };
                        let value = value.cast(self.function()?.locals[local].ty);
                        self.write(local, value)?;
                        effects.writes |= bit;
                        effects.tail_writes |= bit;
                        self.value(value.cast(e.ty), effects);
                    },
                    | _ => return Err(Failure::InvalidIr),
                }
            },
            | Action::Choose(id) => {
                let e = self.function()?.expressions[id];
                self.origin = Some(e.origin);
                let condition = self.pop()?;
                self.boundary()?;
                match e.kind {
                    | ExpressionKind::Logical { right, and, .. } => {
                        if (condition.value.bits != 0) == and {
                            self.tasks.push(Action::Chosen(id, condition.effects));
                            self.tasks.push(Action::Expr(right));
                        } else {
                            self.value(
                                Value::new(e.ty, u64::from(!and)),
                                condition.effects.sequenced(Effects::default()),
                            );
                        }
                    },
                    | ExpressionKind::Conditional {
                        then, otherwise, ..
                    } => {
                        self.tasks.push(Action::Chosen(id, condition.effects));
                        self.tasks.push(Action::Expr(if condition.value.bits != 0 {
                            then
                        } else {
                            otherwise
                        }));
                    },
                    | _ => return Err(Failure::InvalidIr),
                }
            },
            | Action::Chosen(id, effects) => {
                let e = self.function()?.expressions[id];
                self.origin = Some(e.origin);
                let chosen = self.pop()?;
                let value = if matches!(e.kind, ExpressionKind::Logical { .. }) {
                    Value::new(e.ty, u64::from(chosen.value.bits != 0))
                } else {
                    chosen.value.cast(e.ty)
                };
                self.value(value, effects.sequenced(chosen.effects));
            },
            | Action::Invoke(function, count) => {
                let start = self
                    .values
                    .len()
                    .checked_sub(count)
                    .ok_or(Failure::InvalidIr)?;
                let mut effects = Effects::default();
                for i in start..self.values.len() {
                    effects = effects.unsequenced(self.values[i].effects)?;
                }
                self.boundary()?;
                self.tasks.push(Action::CallDone(effects));
                self.enter(function, count)?;
            },
            | Action::CallDone(effects) => {
                let result = self.pop()?;
                self.value(result.value, effects.sequenced(Effects::default()));
            },
            | Action::Initialize(local) => {
                let v = self.pop()?.value.cast(self.function()?.locals[local].ty);
                self.write(local, v)?;
                self.boundary()?;
                self.tasks.push(Action::Step);
            },
            | Action::Discard => {
                _ = self.pop()?;
                self.boundary()?;
                self.tasks.push(Action::Step);
            },
            | Action::Branch(then, otherwise) => {
                let v = self.pop()?;
                self.boundary()?;
                self.jump(if v.value.bits != 0 { then } else { otherwise })?;
                self.tasks.push(Action::Step);
            },
            | Action::Return => {
                let value = self.pop()?.value.cast(self.function()?.signature.result);
                self.boundary()?;
                self.unwind(0)?;
                self.event(EventKind::Return, self.origin)?;
                let frame = self.frames.pop().ok_or(Failure::InvalidIr)?;
                self.locals.truncate(frame.base);
                if self.frames.is_empty() {
                    return Ok(Some(value));
                }
                self.value(value, Effects::default());
            },
        }
        Ok(None)
    }

    fn enter(&mut self, function: usize, count: usize) -> Result<(), Failure> {
        if self.frames.len() == self.limits.call_depth {
            return Err(Failure::Limit("call depth"));
        }
        let f = self
            .module
            .functions
            .get(function)
            .ok_or(Failure::InvalidIr)?;
        if count != f.signature.parameters.len() {
            return Err(Failure::InvalidIr);
        }
        let start = self
            .values
            .len()
            .checked_sub(count)
            .ok_or(Failure::InvalidIr)?;
        let base = self.locals.len();
        for (i, local) in f.locals.iter().enumerate() {
            let value = if i < count {
                Some(self.values[start + i].value.cast(local.ty))
            } else {
                None
            };
            self.locals.push(Cell {
                value,
                live: false,
                instance: 0,
            });
        }
        self.values.truncate(start);
        self.next_activation += 1;
        self.frames.push(Frame {
            function,
            pc: 0,
            base,
            scope_base: self.scopes.len(),
            activation: self.next_activation,
        });
        self.origin = Some(f.signature.origin);
        self.event(EventKind::Call { function }, self.origin)?;
        self.tasks.push(Action::Step);
        Ok(())
    }

    fn unwind(&mut self, keep: usize) -> Result<(), Failure> {
        let frame = *self.frame()?;
        let wanted = frame
            .scope_base
            .checked_add(keep)
            .ok_or(Failure::InvalidIr)?;
        if wanted > self.scopes.len() {
            return Err(Failure::InvalidIr);
        }
        while self.scopes.len() > wanted {
            let scope = self.scopes.pop().ok_or(Failure::InvalidIr)?;
            for (id, local) in self.module.functions[frame.function]
                .locals
                .iter()
                .enumerate()
            {
                if local.scope == scope {
                    let cell = &mut self.locals[frame.base + id];
                    if !cell.live {
                        return Err(Failure::InvalidIr);
                    }
                    cell.live = false;
                    let instance = cell.instance;
                    self.event(
                        EventKind::End {
                            local: id,
                            instance,
                        },
                        self.origin,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, local: usize) -> Result<Value, Failure> {
        let cell = *self.cell_mut(local)?;
        let value = cell.value.ok_or(Failure::IndeterminateRead { local })?;
        self.event(
            EventKind::Read {
                local,
                instance: cell.instance,
            },
            self.origin,
        )?;
        Ok(value)
    }

    fn write(&mut self, local: usize, value: Value) -> Result<(), Failure> {
        let cell = self.cell_mut(local)?;
        cell.value = Some(value);
        let instance = cell.instance;
        self.event(
            EventKind::Write {
                local,
                instance,
                bits: value.bits,
            },
            self.origin,
        )
    }

    fn cell_mut(&mut self, local: usize) -> Result<&mut Cell, Failure> {
        let base = self.frame()?.base;
        let cell = self
            .locals
            .get_mut(base + local)
            .ok_or(Failure::InvalidIr)?;
        if !cell.live {
            return Err(Failure::InvalidIr);
        }
        Ok(cell)
    }

    fn frame(&self) -> Result<&Frame, Failure> {
        self.frames.last().ok_or(Failure::InvalidIr)
    }

    fn function(&self) -> Result<&'ir Function<'ir>, Failure> {
        Ok(&self.module.functions[self.frame()?.function])
    }

    fn jump(&mut self, label: usize) -> Result<(), Failure> {
        let pc = *self
            .function()?
            .labels
            .get(label)
            .ok_or(Failure::InvalidIr)?;
        self.frames.last_mut().ok_or(Failure::InvalidIr)?.pc = pc;
        Ok(())
    }

    fn pop(&mut self) -> Result<Evaluated, Failure> {
        self.values.pop().ok_or(Failure::InvalidIr)
    }

    fn value(&mut self, value: Value, effects: Effects) {
        self.values.push(Evaluated { value, effects });
    }

    fn boundary(&mut self) -> Result<(), Failure> {
        self.event(EventKind::SequenceBoundary, self.origin)
    }

    fn event(&mut self, kind: EventKind, origin: Option<SourceVectors>) -> Result<(), Failure> {
        if self.events.len() == self.limits.events {
            return Err(Failure::Limit("trace events"));
        }
        self.events.push(Event {
            kind,
            origin,
            activation: self.frame()?.activation,
        });
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Execution<'a> {
    pub(super) result: Result<Value, Error>,
    pub(super) events: &'a [Event],
    pub(super) steps:  usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Event {
    pub(super) activation: u64,
    pub(super) kind:       EventKind,
    pub(super) origin:     Option<SourceVectors>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EventKind {
    Call {
        function: usize,
    },
    Return,
    Begin {
        local:    usize,
        instance: u64,
    },
    End {
        local:    usize,
        instance: u64,
    },
    Read {
        local:    usize,
        instance: u64,
    },
    Write {
        local:    usize,
        instance: u64,
        bits:     u64,
    },
    Indeterminate {
        local: usize,
    },
    SequenceBoundary,
    Failure(Failure),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub(super) steps:      usize,
    pub(super) call_depth: usize,
    pub(super) events:     usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            steps:      2_000_000,
            call_depth: 128,
            events:     1_000_000,
        }
    }
}

#[derive(Clone, Copy)]
struct Frame {
    function:   usize,
    pc:         usize,
    base:       usize,
    scope_base: usize,
    activation: u64,
}
#[derive(Clone, Copy)]
struct Cell {
    value:    Option<Value>,
    live:     bool,
    instance: u64,
}
#[derive(Clone, Copy)]
struct Evaluated {
    value:   Value,
    effects: Effects,
}

/// Footprints are activation-local: this subset has no pointers or globals.
/// Sequence-bearing conflicts cannot be classified by this summary alone.
#[derive(Clone, Copy, Default)]
struct Effects {
    reads:       u128,
    writes:      u128,
    tail_writes: u128,
    sequence:    bool,
}

impl Effects {
    fn unsequenced(self, other: Self) -> Result<Self, Failure> {
        let conflict = self.writes & (other.reads | other.writes) | other.writes & self.reads;
        if conflict != 0 {
            return Err(if self.sequence || other.sequence {
                Failure::Unsupported("conflict across nested sequencing")
            } else {
                Failure::Undefined(Undefined::UnsequencedAccess)
            });
        }
        Ok(Self {
            reads:       self.reads | other.reads,
            writes:      self.writes | other.writes,
            tail_writes: self.tail_writes | other.tail_writes,
            sequence:    self.sequence || other.sequence,
        })
    }

    const fn sequenced(self, other: Self) -> Self {
        Self {
            reads:       self.reads | other.reads,
            writes:      self.writes | other.writes,
            tail_writes: other.tail_writes,
            sequence:    true,
        }
    }
}

enum Action {
    Step,
    Expr(usize),
    Second(usize),
    Finish(usize),
    Choose(usize),
    Chosen(usize, Effects),
    Invoke(usize, usize),
    CallDone(Effects),
    Initialize(usize),
    Discard,
    Branch(usize, usize),
    Return,
}
