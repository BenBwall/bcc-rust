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
        UbKind,
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
        Linkage,
        Module,
        ParamAbi,
        RelocationTarget,
        Signature,
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
    pub(super) func:            FuncId,
    pub(super) block:           Block,
    /// The position in `block` of the next instruction to execute.
    pub(super) next:            usize,
    /// Where this frame's values start in the value stack.
    pub(super) values:          usize,
    /// Where this frame's slot objects start in the slot stack.
    pub(super) slots:           usize,
    pub(super) dynamic_slots:   usize,
    /// Where this frame's variadic arguments start, and how many there are.
    pub(super) varargs:         usize,
    pub(super) varargs_len:     usize,
    /// Lazily prepared incoming argument areas, owned by this frame.
    pub(super) va_backing:      Option<super::varargs::VaBacking>,
    /// A number no other activation shares, so a `va_list` cursor can tell
    /// whether its frame is still the one that made it.
    pub(super) activation:      u64,
    /// Where the frame begins: above every object its callers allocated and
    /// below its own slots.
    pub(super) start:           u64,
    /// The value in the caller that receives this call's result.
    pub(super) caller_value:    Option<Value>,
    /// Caller destination and private callee area for a register result.
    pub(super) register_result: Option<(RuntimeValue, Pointer)>,
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
                | (Type::F128, RuntimeValue::F128(_))
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
        self.check_arguments(signature)
            .and_then(|()| self.push_frame(func, None))
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
        signature: Signature<'ir>,
    ) -> Result<Option<Termination>, Fault> {
        self.arguments.clear();
        self.argument_types.clear();
        self.argument_abis.clear();
        for (index, &arg) in args.iter().enumerate() {
            self.argument_abis.push(signature.param_abi(index));
            let value = self.value(arg);
            self.arguments.push(value);
            self.argument_types.push(body.value_type(arg));
        }
        self.check_arguments(signature)?;
        // An alias runs its target's body, which the call reaches as an
        // indirect call through a compatible type would: `int f()` may alias
        // `int g(void)`. A weak declaration nothing defines has the null
        // address, and calling it is undefined.
        // GNU extension: GCC manual, "Common Function Attributes", alias/weak.
        // C99: §6.5.2.2p9, p. 72; PDF p. 84; §6.7.5.3p15, pp. 119-120;
        // PDF pp. 131-132.
        let target = self.module.function_definition(func);
        if target != func && !self.admits(body, target, signature, args) {
            return Err(UbKind::SignatureMismatch.into());
        }
        let func = target;
        if self.functions[func.index()].provenance.is_none() {
            return Err(UbKind::CallThroughNonFunction.into());
        }
        let callee = self.module.function(func);
        // Fixed parameter contracts also apply to an open call resolved to
        // a prototype. Variadic contracts belong to the call site, whose
        // aggregate register parts may legitimately contain padding poison.
        let actual = self.module.function_signature(func);
        for (index, &arg) in self.arguments.iter().take(actual.params.len()).enumerate() {
            if actual.param_noundef(index) && arg.is_poison() {
                return Err(UbKind::Noundef.into());
            }
        }
        if callee.body.is_some() {
            self.push_frame(func, result)?;
            return Ok(None);
        }
        let call = HostCall {
            name:      callee.name,
            args:      &self.arguments,
            arg_types: &self.argument_types,
            result:    self.module.function_signature(func).result,
        };
        match host.call(call, &mut self.memory)? {
            | HostReturn::Value(value) => {
                // A host's value is untrusted, including custom hosts. Check
                // before it enters a typed SSA slot. C99 §6.5.2.2p9.
                let compatible = match (call.result, value) {
                    | (None, None) => true,
                    | (Some(ty), Some(value)) => value.matches_type(ty),
                    | _ => false,
                };
                if !compatible {
                    return Err(UbKind::SignatureMismatch.into());
                }
                if self.module.function_signature(func).result_noundef
                    && value.is_some_and(RuntimeValue::is_poison)
                {
                    return Err(UbKind::Noundef.into());
                }
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

    /// Enforces the signature's value contract before entering any callee,
    /// including host functions and unused parameters. Pointer attributes
    /// constrain only the address, not the pointed-to object bytes.
    fn check_arguments(&self, signature: Signature<'_>) -> Result<(), Fault> {
        for (index, &arg) in self.arguments.iter().enumerate() {
            if signature.param_noundef(index) && arg.is_poison() {
                return Err(UbKind::Noundef.into());
            }
        }
        Ok(())
    }

    /// Pushes a frame for the defined function `func`, binding the entry
    /// block's parameters to the fixed arguments, keeping the rest as
    /// variadic arguments, and allocating its stack slots and private byval
    /// copies. C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83 (parameter
    /// assignment).
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
        let varargs = self.varargs.len();
        let extra_types = self.argument_types.get(fixed..).unwrap_or_default();
        let extra_values = self.arguments.get(fixed..).unwrap_or_default();
        self.varargs.extend(
            extra_types
                .iter()
                .copied()
                .zip(extra_values.iter().copied())
                .enumerate()
                .map(|(index, (ty, value))| {
                    (
                        ty,
                        value,
                        self.argument_abis.get(fixed + index).copied().flatten(),
                    )
                }),
        );
        let slots = self.slots.len();
        let frame_address = self.memory.next_address();
        for (_, data) in body.stack_slots() {
            let slot = self
                .memory
                .allocate(ObjectKind::Stack, u64::from(data.size), data.align)?;
            self.slots.push(slot);
        }
        let mut register_result = None;
        for (index, &param) in body.block_params(entry).iter().enumerate() {
            let mut arg = self.arguments[index];
            if let Some(ParamAbi::Registers(registers)) =
                self.module.function_signature(func).param_abi(index)
            {
                let area = self.memory.allocate(
                    ObjectKind::Stack,
                    registers.area_bytes(),
                    registers.align,
                )?;
                self.slots.push(area);
                register_result = Some((arg, area));
                arg = RuntimeValue::Ptr(area);
            }
            if let Some(ParamAbi::ByVal(layout)) =
                self.module.function_signature(func).param_abi(index)
            {
                let copy = self
                    .memory
                    .allocate(ObjectKind::Stack, layout.size, layout.align)?;
                self.slots.push(copy);
                self.memory.copy(
                    RuntimeValue::Ptr(copy),
                    arg,
                    RuntimeValue::Int(u128::from(layout.size)),
                    layout.align,
                    false,
                )?;
                arg = RuntimeValue::Ptr(copy);
            }
            self.values[values + param.index()] = arg;
        }
        self.frames.push(Frame {
            func,
            block: entry,
            next: 0,
            values,
            slots,
            dynamic_slots: self.dynamic_slots.len(),
            varargs,
            varargs_len: self.varargs.len() - varargs,
            va_backing: None,
            activation: self.activations,
            start: frame_address,
            caller_value,
            register_result,
        });
        self.activations += 1;
        Ok(())
    }

    /// Returns `value` from the executing frame: frees its slots, pops it,
    /// and stores the value into the caller, or ends the run if the entry
    /// function returned.
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151. Register results transfer only
    /// their shared IR layout's fields, leaving caller padding untouched.
    pub(super) fn return_from(
        &mut self,
        value: Option<RuntimeValue>,
    ) -> Result<Option<Termination>, Fault> {
        let frame = self.frame();
        let signature = self.module.function_signature(frame.func);
        if signature.result_noundef && value.is_some_and(RuntimeValue::is_poison) {
            return Err(UbKind::Noundef.into());
        }
        if let Some((destination, area)) = frame.register_result
            && let Some(ParamAbi::Registers(registers)) =
                self.module.function_signature(frame.func).result_param()
        {
            for (offset, ty) in registers.parts() {
                let align = if offset == 0 {
                    registers.align
                } else {
                    Align::from_bytes(registers.align.bytes().min(offset))
                        .expect("a power-of-two alignment")
                };
                let offset = RuntimeValue::Int(u128::from(offset));
                let to = self.memory.ptr_add(destination, offset, false);
                let from = self.memory.ptr_add(RuntimeValue::Ptr(area), offset, false);
                if signature.result_noundef {
                    // Check each byte before packing; a poison byte in any
                    // register makes returning the value immediate UB.
                    for byte in 0..ty.bytes() {
                        let addr =
                            self.memory
                                .ptr_add(from, RuntimeValue::Int(u128::from(byte)), false);
                        if self
                            .memory
                            .load(Type::I8, addr, Align::from_bytes(1).unwrap())?
                            .is_poison()
                        {
                            return Err(UbKind::Noundef.into());
                        }
                    }
                }
                self.memory.copy(
                    to,
                    from,
                    RuntimeValue::Int(u128::from(ty.bytes())),
                    align,
                    false,
                )?;
            }
        }
        let frame = self.frames.pop().expect("a frame is executing");
        for &slot in &self.slots[frame.slots..] {
            self.memory
                .free(slot.provenance.expect("slots have provenance"));
        }
        self.slots.truncate(frame.slots);
        for &pointer in &self.dynamic_slots[frame.dynamic_slots..] {
            self.memory
                .free(pointer.provenance.expect("stack allocation"));
        }
        self.dynamic_slots.truncate(frame.dynamic_slots);
        self.values.truncate(frame.values);
        self.varargs.truncate(frame.varargs);
        if self.frames.is_empty() {
            return Ok(Some(Termination::Returned(value)));
        }
        if let (Some(result), Some(value)) = (frame.caller_value, value) {
            self.set(result, value);
        }
        Ok(None)
    }

    /// Discards every frame, as `exit` does, freeing their stack objects.
    /// C99: §7.20.4.3p2-4, pp. 315-316; PDF pp. 327-328.
    pub(super) fn unwind(&mut self) {
        for &slot in self.slots.iter().chain(self.dynamic_slots.iter()) {
            self.memory
                .free(slot.provenance.expect("slots have provenance"));
        }
        self.slots.clear();
        self.dynamic_slots.clear();
        self.frames.clear();
        self.values.clear();
        self.varargs.clear();
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
            | Fault::UnknownGlobal(global) => Trap::UnknownGlobal(global, location),
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
    /// function and global, with definitions' initializers and relocations
    /// written. Declarations retain only an unresolved address, never storage.
    /// An alias shares its target's address. A weak declaration that neither
    /// the module nor `host` defines has the null address.
    /// C99: §6.9 paragraph 5, p. 140; PDF p. 152 (external definitions);
    /// §6.9.2 paragraph 2, p. 143; PDF p. 155 (zero-initialized tentative
    /// definitions). GNU extension: GCC manual, "Common Function Attributes",
    /// alias and weak.
    pub(super) fn load(
        module: &'m Module<'ir>,
        arena: &'a Bump,
        limits: Limits,
        host: &dyn Host,
    ) -> Result<Self, Trap> {
        let mut memory = Memory::new(arena, limits.max_object_size);
        let mut functions = ArenaVec::new_in(arena);
        for (func, function) in module.functions() {
            let unresolved = function.linkage == Linkage::Weak
                && function.body.is_none()
                && !host.provides(function.name);
            let pointer = if function.attributes.alias.is_some() || unresolved {
                Pointer::NULL
            } else {
                memory
                    .allocate(ObjectKind::Function(func), 0, function_align(function))
                    .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?
            };
            functions.push(pointer);
        }
        for (func, _) in module.functions() {
            functions[func.index()] = functions[module.function_definition(func).index()];
        }
        let mut labels = ArenaVec::new_in(arena);
        for (func, function) in module.functions() {
            let count = function.body.as_ref().map_or(0, Body::label_count);
            labels.push(if count == 0 {
                None
            } else {
                Some(
                    memory
                        .allocate(ObjectKind::Labels(func), count as u64, Align::BYTE)
                        .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?,
                )
            });
        }
        let mut globals = ArenaVec::new_in(arena);
        for (id, global) in module.globals() {
            if global.attributes.alias.is_some()
                || (global.linkage == Linkage::Weak && global.init.is_none())
            {
                globals.push(Pointer::NULL);
                continue;
            }
            let kind = if global.init.is_none() {
                ObjectKind::UnresolvedGlobal(id)
            } else if global.constant {
                ObjectKind::ConstantGlobal
            } else {
                ObjectKind::Global
            };
            let pointer = memory
                .allocate(kind, global.size, global.align)
                .map_err(|_| Trap::BadEntry(EntryError::GlobalTooLarge))?;
            globals.push(pointer);
        }
        for (id, _) in module.globals() {
            globals[id.index()] = globals[module.global_definition(id).index()];
        }
        for ((_, global), &pointer) in module.globals().zip(&globals) {
            if global.init.is_none() {
                continue;
            }
            let Some(GlobalInit::Bytes { bytes, relocations }) = global.init else {
                memory.initialize(pointer, &[]);
                continue;
            };
            memory.initialize(pointer, bytes);
            for relocation in relocations {
                let target: Pointer = match relocation.target {
                    | RelocationTarget::Symbol(Symbol::Function(func)) => functions[func.index()],
                    | RelocationTarget::Symbol(Symbol::Global(global)) => globals[global.index()],
                    | RelocationTarget::Label { func, label } => {
                        let base = labels[func.index()].expect("a labeled function");
                        Pointer {
                            address:    base.address + label.index() as u64,
                            provenance: base.provenance,
                        }
                    },
                    | RelocationTarget::LabelDifference {
                        label, base, width, ..
                    } => {
                        // Labels are bytes of one object, so their distance
                        // is the difference of their numbers. The verifier
                        // bounds the width; the bytes are sign-extended.
                        let distance = i128::from(
                            (label.index() as i64 - base.index() as i64)
                                .wrapping_add(relocation.addend),
                        );
                        let bytes = distance.to_le_bytes();
                        memory.initialize_bytes(
                            pointer,
                            relocation.offset,
                            &bytes[..(width.bytes() as usize).min(bytes.len())],
                        );
                        continue;
                    },
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
            dynamic_slots: ArenaVec::new_in(arena),
            varargs: ArenaVec::new_in(arena),
            argument_abis: ArenaVec::new_in(arena),
            arguments: ArenaVec::new_in(arena),
            argument_types: ArenaVec::new_in(arena),
            functions,
            labels,
            globals,
            steps: 0,
            activations: 0,
            timestamp: 0,
        })
    }
}

/// The alignment of a function's address object: its requested code
/// alignment, else a byte.
fn function_align(function: &crate::ir::Function<'_>) -> Align {
    function.align.unwrap_or(Align::BYTE)
}
