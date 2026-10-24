//! Bounded execution by native recursion, following the recursive parser and
//! lowering studies: one Rust call per activation and per expression node.
//! Activation records, objects and lexical scopes remain data, because the
//! trace and debugger views inspect them. The caller supplies the native
//! stack (the tests use the reserved compilation worker); `Limits::nesting`
//! bounds its depth.
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
        limits,
        frames: ArenaVec::new_in(arena),
        locals: ArenaVec::new_in(arena),
        scopes: ArenaVec::new_in(arena),
        arguments: ArenaVec::new_in(arena),
        events: ArenaVec::new_in(arena),
        steps: 0,
        nesting: 0,
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
        if args
            .iter()
            .any(|v| !v.ty.valid() || v.bits & !v.ty.mask() != 0)
        {
            return Err(Failure::InvalidIr);
        }
        machine
            .arguments
            .extend(args.iter().map(|&value| Evaluated {
                value,
                effects: Effects::default(),
            }));
        machine.call(function, 0)
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
    limits:          Limits,
    frames:          ArenaVec<'a, Frame>,
    locals:          ArenaVec<'a, Cell>,
    scopes:          ArenaVec<'a, usize>,
    /// Evaluated arguments of calls whose callee has not been entered yet.
    arguments:       ArenaVec<'a, Evaluated>,
    events:          ArenaVec<'a, Event>,
    steps:           usize,
    nesting:         usize,
    next_activation: u64,
    next_instance:   u64,
    origin:          Option<SourceVectors>,
}

impl<'ir> Machine<'_, 'ir, '_> {
    /// Runs one activation whose arguments are `arguments[start..]`. A failed
    /// activation stays on `frames`, so the failure event names it.
    fn call(&mut self, function: usize, start: usize) -> Result<Value, Failure> {
        self.enter()?;
        self.activate(function, start)?;
        let value = self.body()?;
        let frame = self.frames.pop().ok_or(Failure::InvalidIr)?;
        self.locals.truncate(frame.base);
        self.nesting -= 1;
        Ok(value)
    }

    /// Executes statement instructions in order until the activation returns.
    fn body(&mut self) -> Result<Value, Failure> {
        let f = self.function()?;
        let mut pc = 0;
        loop {
            self.tick()?;
            let op = *f.code.get(pc).ok_or(Failure::InvalidIr)?;
            pc += 1;
            self.origin = Some(op.origin);
            match op.kind {
                | InstructionKind::Enter(scope) => self.begin(scope)?,
                | InstructionKind::Exit => {
                    let keep = self
                        .scopes
                        .len()
                        .checked_sub(self.frame()?.scope_base + 1)
                        .ok_or(Failure::InvalidIr)?;
                    self.unwind(keep)?;
                },
                | InstructionKind::Declare { local, initializer } => {
                    self.cell_mut(local)?.value = None;
                    self.event(EventKind::Indeterminate { local }, self.origin)?;
                    if let Some(e) = initializer {
                        let v = self.expression(e)?.value.cast(f.locals[local].ty);
                        self.write(local, v)?;
                    }
                    self.boundary()?;
                },
                | InstructionKind::Evaluate(e) => {
                    _ = self.expression(e)?;
                    self.boundary()?;
                },
                | InstructionKind::Branch {
                    condition,
                    then,
                    otherwise,
                } => {
                    let v = self.expression(condition)?;
                    self.boundary()?;
                    pc = self.label(if v.value.bits != 0 { then } else { otherwise })?;
                },
                | InstructionKind::Jump { label, keep } => {
                    self.unwind(keep)?;
                    pc = self.label(label)?;
                },
                | InstructionKind::Return(e) => {
                    let value = self.expression(e)?.value;
                    return self.leave(value);
                },
                | InstructionKind::Fallthrough => {
                    if f.signature.name != "main" {
                        return Err(Failure::Unsupported("non-main fallthrough"));
                    }
                    return self.leave(Value::new(f.signature.result, 0));
                },
            }
        }
    }

    /// C99: §6.8.6.4p3, p. 139; PDF p. 151; §5.1.2.2.3p1, p. 13; PDF p. 25.
    fn leave(&mut self, value: Value) -> Result<Value, Failure> {
        let value = value.cast(self.function()?.signature.result);
        self.boundary()?;
        self.unwind(0)?;
        self.event(EventKind::Return, self.origin)?;
        Ok(value)
    }

    /// Evaluates one expression node and its operands in their C order.
    fn expression(&mut self, id: usize) -> Result<Evaluated, Failure> {
        self.enter()?;
        self.tick()?;
        let result = self.expression_inner(id);
        self.nesting -= 1;
        result
    }

    fn expression_inner(&mut self, id: usize) -> Result<Evaluated, Failure> {
        let e = *self
            .function()?
            .expressions
            .get(id)
            .ok_or(Failure::InvalidIr)?;
        self.origin = Some(e.origin);
        let evaluated = |value, effects| Ok(Evaluated { value, effects });
        match e.kind {
            | ExpressionKind::Constant(bits) =>
                evaluated(Value::new(e.ty, bits), Effects::default()),
            | ExpressionKind::Read(local) => {
                let value = self.read(local)?.cast(e.ty);
                evaluated(
                    value,
                    Effects {
                        reads: 1 << local,
                        ..Effects::default()
                    },
                )
            },
            | ExpressionKind::Convert(a) => {
                let a = self.expression(a)?;
                self.origin = Some(e.origin);
                evaluated(a.value.cast(e.ty), a.effects)
            },
            | ExpressionKind::Unary(op, a) => {
                let a = self.expression(a)?;
                self.origin = Some(e.origin);
                evaluated(arithmetic::unary(op, a.value, e.ty)?, a.effects)
            },
            | ExpressionKind::Binary(op, a, b) => {
                let a = self.expression(a)?;
                let b = self.expression(b)?;
                self.origin = Some(e.origin);
                let effects = a.effects.unsequenced(b.effects)?;
                evaluated(arithmetic::binary(op, a.value, b.value, e.ty)?, effects)
            },
            | ExpressionKind::Sequence(a, b) => {
                let a = self.expression(a)?;
                self.origin = Some(e.origin);
                self.boundary()?;
                let b = self.expression(b)?;
                self.origin = Some(e.origin);
                evaluated(b.value.cast(e.ty), a.effects.sequenced(b.effects))
            },
            | ExpressionKind::Logical { left, right, and } => {
                let condition = self.expression(left)?;
                self.origin = Some(e.origin);
                self.boundary()?;
                if (condition.value.bits != 0) == and {
                    let chosen = self.expression(right)?;
                    self.origin = Some(e.origin);
                    evaluated(
                        Value::new(e.ty, u64::from(chosen.value.bits != 0)),
                        condition.effects.sequenced(chosen.effects),
                    )
                } else {
                    evaluated(
                        Value::new(e.ty, u64::from(!and)),
                        condition.effects.sequenced(Effects::default()),
                    )
                }
            },
            | ExpressionKind::Conditional {
                condition,
                then,
                otherwise,
            } => {
                let condition = self.expression(condition)?;
                self.origin = Some(e.origin);
                self.boundary()?;
                let chosen = self.expression(if condition.value.bits != 0 {
                    then
                } else {
                    otherwise
                })?;
                self.origin = Some(e.origin);
                evaluated(
                    chosen.value.cast(e.ty),
                    condition.effects.sequenced(chosen.effects),
                )
            },
            | ExpressionKind::Set {
                local,
                value,
                operation,
            } => {
                let b = self.expression(value)?;
                self.origin = Some(e.origin);
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
                evaluated(value.cast(e.ty), effects)
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
                evaluated(
                    if postfix { old } else { value },
                    Effects {
                        reads:       1 << local,
                        writes:      1 << local,
                        tail_writes: 1 << local,
                        sequence:    false,
                    },
                )
            },
            | ExpressionKind::Call {
                function,
                arguments,
            } => {
                let start = self.arguments.len();
                for &argument in arguments {
                    let argument = self.expression(argument)?;
                    self.arguments.push(argument);
                }
                self.origin = Some(e.origin);
                let mut effects = Effects::default();
                for i in start..self.arguments.len() {
                    effects = effects.unsequenced(self.arguments[i].effects)?;
                }
                self.boundary()?;
                let value = self.call(function, start)?;
                evaluated(value, effects.sequenced(Effects::default()))
            },
        }
    }

    /// Creates the activation's objects from `arguments[start..]`.
    fn activate(&mut self, function: usize, start: usize) -> Result<(), Failure> {
        if self.frames.len() == self.limits.call_depth {
            return Err(Failure::Limit("call depth"));
        }
        let f = self
            .module
            .functions
            .get(function)
            .ok_or(Failure::InvalidIr)?;
        if self.arguments.len().checked_sub(start) != Some(f.signature.parameters.len()) {
            return Err(Failure::InvalidIr);
        }
        let base = self.locals.len();
        for (i, local) in f.locals.iter().enumerate() {
            let value = self
                .arguments
                .get(start + i)
                .filter(|_| i < f.signature.parameters.len())
                .map(|argument| argument.value.cast(local.ty));
            self.locals.push(Cell {
                value,
                live: false,
                instance: 0,
            });
        }
        self.arguments.truncate(start);
        self.next_activation += 1;
        self.frames.push(Frame {
            function,
            base,
            scope_base: self.scopes.len(),
            activation: self.next_activation,
        });
        self.origin = Some(f.signature.origin);
        self.event(EventKind::Call { function }, self.origin)
    }

    /// Starts the lifetime of every object declared in `scope`.
    fn begin(&mut self, scope: usize) -> Result<(), Failure> {
        self.scopes.push(scope);
        let frame = *self.frame()?;
        let f = self.function()?;
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
        Ok(())
    }

    fn enter(&mut self) -> Result<(), Failure> {
        if self.nesting == self.limits.nesting {
            return Err(Failure::Limit("evaluation nesting"));
        }
        self.nesting += 1;
        Ok(())
    }

    fn tick(&mut self) -> Result<(), Failure> {
        if self.steps == self.limits.steps {
            return Err(Failure::Limit("steps"));
        }
        self.steps += 1;
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

    fn label(&self, label: usize) -> Result<usize, Failure> {
        self.function()?
            .labels
            .get(label)
            .copied()
            .ok_or(Failure::InvalidIr)
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
    /// Nested Rust calls (activations plus expression nodes), so that an
    /// untrusted program cannot exhaust a smaller host stack.
    pub(super) nesting:    usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            steps:      2_000_000,
            call_depth: 128,
            events:     1_000_000,
            nesting:    1 << 16,
        }
    }
}

#[derive(Clone, Copy)]
struct Frame {
    function:   usize,
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
