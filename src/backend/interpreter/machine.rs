//! Loading a module and moving between frames and blocks: the entry call,
//! calls to defined and host functions, returns, and branches.

use super::{
    Machine,
    host::{
        Host,
        HostCall,
        HostReturn,
    },
    memory::{
        Memory,
        ObjectKind,
    },
    outcome::{
        Limits,
        Termination,
    },
    trap::{
        EntryError,
        Fault,
        Location,
        Trap,
    },
    value::{
        Pointer,
        RuntimeValue,
    },
};
use crate::{
    ir::{
        Align,
        Block,
        BlockCall,
        Body,
        Entity,
        FuncId,
        GlobalInit,
        Inst,
        InstData,
        Module,
        Symbol,
        Type,
        Value,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// One activation of a defined function on the call stack.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub(super) func:         FuncId,
    pub(super) block:        Block,
    /// The position in `block` of the next instruction to execute.
    pub(super) next:         usize,
    /// Where this frame's values start in the value stack.
    pub(super) values:       usize,
    /// Where this frame's slot objects start in the slot stack.
    pub(super) slots:        usize,
    /// Where this frame's variadic arguments start, and how many there are.
    pub(super) varargs:      usize,
    pub(super) varargs_len:  usize,
    /// A number no other activation shares, so a `va_list` cursor can tell
    /// whether its frame is still the one that made it.
    pub(super) activation:   u64,
    /// The value in the caller that receives this call's result.
    pub(super) caller_value: Option<Value>,
}

impl<'m, 'ir> Machine<'m, 'ir, '_> {
    /// Calls `entry` with `args`: checks them against its signature and
    /// pushes its frame.
    pub(super) fn enter(&mut self, entry: &str, args: &[RuntimeValue]) -> Result<(), Trap> {
        let Some(Symbol::Function(func)) = self.module.symbol(entry) else {
            return Err(Trap::BadEntry(EntryError::NotFound));
        };
        let Some(body) = &self.module.function(func).body else {
            return Err(Trap::BadEntry(EntryError::NotDefined));
        };
        let signature = self.module.function_signature(func);
        if signature.params.len() != args.len() {
            return Err(Trap::BadEntry(EntryError::Arguments));
        }
        self.arguments.clear();
        self.argument_types.clear();
        for (&ty, &arg) in signature.params.iter().zip(args) {
            let arg = match (ty, arg) {
                | (ty, RuntimeValue::Int(bits)) if ty.is_int() => RuntimeValue::int(ty, bits),
                | (_, RuntimeValue::Poison)
                | (Type::F32, RuntimeValue::F32(_))
                | (Type::F64, RuntimeValue::F64(_))
                | (Type::Ptr, RuntimeValue::Ptr(_)) => arg,
                | _ => return Err(Trap::BadEntry(EntryError::Arguments)),
            };
            self.arguments.push(arg);
            self.argument_types.push(ty);
        }
        let location = Location {
            func,
            inst: body.block_insts(Block::new(0))[0],
        };
        self.push_frame(func, None)
            .map_err(|fault| self.trap(fault, location))
    }

    /// The next instruction of the executing frame, which this advances
    /// past it, with its location.
    pub(super) fn fetch(&mut self) -> (Location, Inst) {
        let module = self.module;
        let frame = self.frames.last_mut().expect("a frame is executing");
        let body = module
            .function(frame.func)
            .body
            .as_ref()
            .expect("frames run defined functions");
        let inst = body.block_insts(frame.block)[frame.next];
        frame.next += 1;
        (
            Location {
                func: frame.func,
                inst,
            },
            inst,
        )
    }

    /// Calls `func` with the values `args`, putting its result into
    /// `result`: pushes a frame for a defined function, or asks the host to
    /// run a declared one.
    pub(super) fn call(
        &mut self,
        body: &Body<'_>,
        func: FuncId,
        args: &[Value],
        result: Option<Value>,
        host: &mut dyn Host,
    ) -> Result<Option<Termination>, Fault> {
        self.arguments.clear();
        self.argument_types.clear();
        for &arg in args {
            let value = self.value(arg);
            self.arguments.push(value);
            self.argument_types.push(body.value_type(arg));
        }
        let callee = self.module.function(func);
        if callee.body.is_some() {
            self.push_frame(func, result)?;
            return Ok(None);
        }
        let call = HostCall {
            name:   callee.name,
            args:   &self.arguments,
            result: self.module.function_signature(func).result,
        };
        match host.call(call, &mut self.memory)? {
            | HostReturn::Value(value) => {
                if let (Some(result), Some(value)) = (result, value) {
                    self.set(result, value);
                }
                Ok(None)
            },
            | HostReturn::Exit(status) => Ok(Some(Termination::Exited(status))),
            | HostReturn::Abort => Ok(Some(Termination::Aborted)),
            | HostReturn::NotProvided => Err(Fault::UnknownFunction),
        }
    }

    /// Pushes a frame for the defined function `func`, binding the entry
    /// block's parameters to the fixed arguments, keeping the rest as
    /// variadic arguments, and allocating its stack slots.
    fn push_frame(&mut self, func: FuncId, caller_value: Option<Value>) -> Result<(), Fault> {
        if self.frames.len() >= self.limits.stack_depth {
            return Err(Fault::StackDepthLimit);
        }
        let body = self.body(func);
        let fixed = self.module.function_signature(func).params.len();
        let values = self.values.len();
        self.values
            .resize(values + body.value_count(), RuntimeValue::Poison);
        let entry = Block::new(0);
        for (&param, &arg) in body.block_params(entry).iter().zip(&self.arguments) {
            self.values[values + param.index()] = arg;
        }
        let varargs = self.varargs.len();
        let extra_types = self.argument_types.get(fixed..).unwrap_or_default();
        let extra_values = self.arguments.get(fixed..).unwrap_or_default();
        self.varargs.extend(
            extra_types
                .iter()
                .copied()
                .zip(extra_values.iter().copied()),
        );
        let slots = self.slots.len();
        for (_, data) in body.stack_slots() {
            let slot = self
                .memory
                .allocate(ObjectKind::Stack, u64::from(data.size), data.align)?;
            self.slots.push(slot);
        }
        self.frames.push(Frame {
            func,
            block: entry,
            next: 0,
            values,
            slots,
            varargs,
            varargs_len: self.varargs.len() - varargs,
            activation: self.activations,
            caller_value,
        });
        self.activations += 1;
        Ok(())
    }

    /// Returns `value` from the executing frame: frees its slots, pops it,
    /// and stores the value into the caller, or ends the run if the entry
    /// function returned.
    pub(super) fn return_from(&mut self, value: Option<RuntimeValue>) -> Option<Termination> {
        let frame = self.frames.pop().expect("a frame is executing");
        for &slot in &self.slots[frame.slots..] {
            self.memory
                .free(slot.provenance.expect("slots have provenance"));
        }
        self.slots.truncate(frame.slots);
        self.values.truncate(frame.values);
        self.varargs.truncate(frame.varargs);
        if self.frames.is_empty() {
            return Some(Termination::Returned(value));
        }
        if let (Some(result), Some(value)) = (frame.caller_value, value) {
            self.set(result, value);
        }
        None
    }

    /// Takes the edge `call`: assigns its arguments to the target block's
    /// parameters, all at once, and continues at the block's start.
    pub(super) fn branch(&mut self, body: &Body<'_>, call: BlockCall) {
        let (block, args) = body.block_call(call);
        self.arguments.clear();
        for &arg in args {
            let value = self.value(arg);
            self.arguments.push(value);
        }
        let frame = self.frames.last_mut().expect("a frame is executing");
        frame.block = block;
        frame.next = 0;
        let base = frame.values;
        for (&param, &arg) in body.block_params(block).iter().zip(&self.arguments) {
            self.values[base + param.index()] = arg;
        }
    }

    /// The trap `fault` becomes at `location`, the executing instruction.
    pub(super) fn trap(&self, fault: Fault, location: Location) -> Trap {
        match fault {
            | Fault::Ub(kind) => Trap::UndefinedBehavior(kind, location),
            | Fault::Unsupported(what) => Trap::Unsupported(what, location),
            | Fault::StackDepthLimit => Trap::StackDepthLimit(location),
            | Fault::UnknownFunction => {
                let body = self.body(location.func);
                let callee = match *body.inst(location.inst) {
                    | InstData::Call { func, .. } => func,
                    | InstData::CallIndirect { args, .. } => {
                        let callee = self.value(body.value_list(args)[0]);
                        self.memory
                            .function_at(callee)
                            .expect("the call resolved its callee before failing")
                    },
                    | _ => unreachable!("only calls call unknown functions"),
                };
                Trap::UnknownFunction(callee, location)
            },
        }
    }

    /// The executing frame.
    pub(super) fn frame(&self) -> &Frame {
        self.frames.last().expect("a frame is executing")
    }

    /// The body of the defined function `func`.
    pub(super) fn body(&self, func: FuncId) -> &'m Body<'ir> {
        let module = self.module;
        module
            .function(func)
            .body
            .as_ref()
            .expect("only defined functions have frames")
    }

    /// The contents of `value` in the executing frame.
    pub(super) fn value(&self, value: Value) -> RuntimeValue {
        self.values[self.frame().values + value.index()]
    }

    /// Sets `value` in the executing frame.
    pub(super) fn set(&mut self, value: Value, contents: RuntimeValue) {
        let base = self.frame().values;
        self.values[base + value.index()] = contents;
    }
}

impl<'m, 'ir, 'a> Machine<'m, 'ir, 'a> {
    /// Loads `module` into fresh memory in `arena`: an object for each
    /// function and global, with the globals' initializers and relocations
    /// written.
    pub(super) fn load(
        module: &'m Module<'ir>,
        arena: &'a Bump,
        limits: Limits,
    ) -> Result<Self, Trap> {
        let mut memory = Memory::new(arena, limits.max_object_size);
        let mut functions = ArenaVec::new_in(arena);
        for (func, _) in module.functions() {
            let pointer = memory
                .allocate(ObjectKind::Function(func), 0, Align::BYTE)
                .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?;
            functions.push(pointer);
        }
        let mut globals = ArenaVec::new_in(arena);
        for (_, global) in module.globals() {
            let kind = if global.constant {
                ObjectKind::ConstantGlobal
            } else {
                ObjectKind::Global
            };
            let pointer = memory
                .allocate(kind, global.size, global.align)
                .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?;
            globals.push(pointer);
        }
        for ((_, global), &pointer) in module.globals().zip(&globals) {
            let Some(GlobalInit::Bytes { bytes, relocations }) = global.init else {
                memory.initialize(pointer, &[]);
                continue;
            };
            memory.initialize(pointer, bytes);
            for relocation in relocations {
                let target: Pointer = match relocation.symbol {
                    | Symbol::Function(func) => functions[func.index()],
                    | Symbol::Global(global) => globals[global.index()],
                };
                let value = Pointer {
                    address:    target.address.wrapping_add_signed(relocation.addend),
                    provenance: target.provenance,
                };
                memory.relocate(pointer, relocation.offset, value);
            }
        }
        Ok(Self {
            module,
            limits,
            memory,
            frames: ArenaVec::new_in(arena),
            values: ArenaVec::new_in(arena),
            slots: ArenaVec::new_in(arena),
            varargs: ArenaVec::new_in(arena),
            arguments: ArenaVec::new_in(arena),
            argument_types: ArenaVec::new_in(arena),
            functions,
            globals,
            steps: 0,
            activations: 0,
        })
    }
}
