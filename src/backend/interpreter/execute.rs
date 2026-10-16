//! One instruction: reads its operands from the executing frame, applies
//! its semantics, and writes its result or transfers control.

use super::{
    Machine,
    arithmetic::{
        convert,
        float_binary,
        float_compare,
        float_negate,
        int_binary,
        int_compare,
    },
    host::Host,
    memory::truncate_u64,
    outcome::Termination,
    trap::{
        Fault,
        UbKind,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::ir::{
    Body,
    Entity,
    FuncId,
    Inst,
    InstData,
    InstFlags,
    Opcode,
    Type,
    Value,
};

impl Machine<'_, '_, '_> {
    /// Executes `inst`, the executing frame's current instruction, and says
    /// whether the program ended.
    pub(super) fn execute(
        &mut self,
        inst: Inst,
        host: &mut dyn Host,
    ) -> Result<Option<Termination>, Fault> {
        let body = self.body(self.frame().func);
        let result = body.inst_result(inst);
        let value = match *body.inst(inst) {
            | InstData::Binary {
                opcode,
                ty,
                flags,
                args: [a, b],
            } => {
                let (a, b) = (self.value(a), self.value(b));
                match opcode {
                    | _ if opcode.is_overflow() => match (a, b) {
                        | (RuntimeValue::Int(a), RuntimeValue::Int(b)) => RuntimeValue::Int(
                            u128::from(crate::ir::unsigned_overflow(opcode, ty, a, b)),
                        ),
                        | _ => RuntimeValue::Poison,
                    },
                    | Opcode::PtrAdd =>
                        self.memory
                            .ptr_add(a, b, flags.contains(InstFlags::INBOUNDS)),
                    | Opcode::VaCopy => {
                        self.va_copy(a, b)?;
                        return Ok(None);
                    },
                    | _ if opcode.is_int_binary() => int_binary(opcode, ty, flags, a, b)?,
                    | _ => float_binary(opcode, ty, a, b)?,
                }
            },
            | InstData::DynAlloca { size, align, .. } => {
                let RuntimeValue::Int(size) = self.value(size) else {
                    return Err(Fault::Ub(UbKind::PoisonAddress));
                };
                let pointer = self.memory.allocate(
                    super::memory::ObjectKind::Stack,
                    truncate_u64(size),
                    align,
                )?;
                self.dynamic_slots.push(pointer);
                RuntimeValue::Ptr(pointer)
            },
            | InstData::Unary { opcode, ty, arg } => {
                let value = self.value(arg);
                match opcode {
                    | _ if opcode.is_bit_unary() => match value {
                        | RuntimeValue::Int(bits) => crate::ir::bit_unary(opcode, ty, bits)
                            .map_or(RuntimeValue::Poison, |bits| RuntimeValue::int(ty, bits)),
                        | _ => RuntimeValue::Poison,
                    },
                    | Opcode::StackRestore => {
                        let RuntimeValue::Ptr(mark) = value else {
                            return Err(UbKind::PoisonAddress.into());
                        };
                        let position = usize::try_from(mark.address & u64::from(u32::MAX))
                            .map_err(|_| UbKind::PoisonAddress)?;
                        let frame = self.frame();
                        if mark.address >> 32 != frame.activation
                            || position < frame.dynamic_slots
                            || position > self.dynamic_slots.len()
                        {
                            return Err(UbKind::PoisonAddress.into());
                        }
                        for &pointer in &self.dynamic_slots[position..] {
                            self.memory
                                .free(pointer.provenance.expect("stack allocation"));
                        }
                        self.dynamic_slots.truncate(position);
                        return Ok(None);
                    },
                    | Opcode::Fneg => float_negate(ty, value)?,
                    | Opcode::Freeze if value.is_poison() => RuntimeValue::zero(ty),
                    | Opcode::Freeze => value,
                    | Opcode::VaStart => {
                        self.va_start(value)?;
                        return Ok(None);
                    },
                    | Opcode::VaEnd => {
                        self.va_end(value)?;
                        return Ok(None);
                    },
                    | Opcode::VaArg => self.va_arg(ty, value)?,
                    | Opcode::Inttoptr => match value {
                        | RuntimeValue::Int(bits) =>
                            RuntimeValue::Ptr(self.memory.pointer_from_address(truncate_u64(bits))),
                        | _ => RuntimeValue::Poison,
                    },
                    | _ => convert(opcode, body.value_type(arg), ty, value)?,
                }
            },
            | InstData::IntCompare {
                cond,
                ty,
                args: [a, b],
            } => int_compare(cond, ty, self.value(a), self.value(b)),
            | InstData::FloatCompare {
                cond,
                ty,
                args: [a, b],
            } => float_compare(cond, ty, self.value(a), self.value(b))?,
            | InstData::Select {
                args: [cond, a, b], ..
            } => match self.value(cond) {
                | RuntimeValue::Int(1) => self.value(a),
                | RuntimeValue::Int(_) => self.value(b),
                | _ => RuntimeValue::Poison,
            },
            | data @ (InstData::Const { ty, .. } | InstData::WideConst { ty, .. }) => {
                let bits = body.const_bits(&data).expect("constants have bits");
                match ty {
                    | Type::F32 => {
                        #[expect(clippy::cast_possible_truncation, reason = "An f32 has 32 bits.")]
                        let bits = bits as u32;
                        RuntimeValue::F32(f32::from_bits(bits))
                    },
                    | Type::F64 => RuntimeValue::F64(f64::from_bits(truncate_u64(bits))),
                    | Type::F128 => RuntimeValue::F128(bits),
                    | Type::F80 => return Err(Fault::Unsupported(Unsupported::WideFloat)),
                    | _ => RuntimeValue::int(ty, bits),
                }
            },
            | InstData::Nullary {
                opcode: Opcode::StackSave,
                ..
            } => RuntimeValue::Ptr(super::value::Pointer {
                address:    (self.frame().activation << 32) | self.dynamic_slots.len() as u64,
                provenance: None,
            }),
            // Addresses without provenance. The frame address lies between
            // the callers' objects and this frame's own, as a machine frame
            // does; the interpreter has no code addresses, so the return
            // address is an opaque nonnull value.
            | InstData::Nullary {
                opcode: Opcode::FrameAddress,
                ..
            } => RuntimeValue::Ptr(super::value::Pointer {
                address:    self.frame().start,
                provenance: None,
            }),
            | InstData::Nullary {
                opcode: Opcode::ReturnAddress,
                ..
            } => RuntimeValue::Ptr(super::value::Pointer {
                address:    (self.frame().activation << 32) | 1 << 31,
                provenance: None,
            }),
            // Without a debugger, a breakpoint ends the program as the
            // operating system's breakpoint exception does.
            | InstData::Nullary {
                opcode: Opcode::Trap | Opcode::DebugTrap,
                ..
            } => return Ok(Some(Termination::Aborted)),
            // A counter that increases with every read, as the time-stamp
            // counter does; its values say nothing about real time.
            | InstData::Nullary {
                opcode: Opcode::ReadTimestamp,
                ..
            } => {
                self.timestamp = self.timestamp.wrapping_add(1);
                RuntimeValue::Int(u128::from(self.timestamp))
            },
            | InstData::Nullary { opcode, .. } =>
                if opcode == Opcode::Null {
                    RuntimeValue::NULL
                } else {
                    RuntimeValue::Poison
                },
            | InstData::StackAddr { slot } =>
                RuntimeValue::Ptr(self.slots[self.frame().slots + slot.index()]),
            | InstData::GlobalAddr { global } => RuntimeValue::Ptr(self.globals[global.index()]),
            | InstData::FuncAddr { func } => RuntimeValue::Ptr(self.functions[func.index()]),
            | InstData::BlockAddr { label } => {
                let base = self.labels[self.frame().func.index()]
                    .expect("a function with block_addr has labels");
                RuntimeValue::Ptr(super::value::Pointer {
                    address:    base.address + label.index() as u64,
                    provenance: base.provenance,
                })
            },
            | InstData::Load {
                ty, align, addr, ..
            } => self.memory.load(ty, self.value(addr), align)?,
            | InstData::Store {
                ty,
                align,
                args: [value, addr],
                ..
            } => {
                let (value, addr) = (self.value(value), self.value(addr));
                self.memory.store(ty, value, addr, align)?;
                return Ok(None);
            },
            | InstData::MemoryRange {
                opcode,
                flags,
                align,
                args: [destination, source, size],
            } => {
                let destination = self.value(destination);
                let (source, size) = (self.value(source), self.value(size));
                if opcode == Opcode::Copy {
                    let may_overlap = flags.contains(InstFlags::MAY_OVERLAP);
                    self.memory
                        .copy(destination, source, size, align, may_overlap)?;
                } else {
                    self.memory.fill(destination, source, size, align)?;
                }
                return Ok(None);
            },
            | InstData::VaArgAggregate { sig, list } => {
                let layout = self
                    .module
                    .signature(sig)
                    .param_abi(0)
                    .and_then(crate::ir::ParamAbi::layout)
                    .expect("verified descriptor");
                self.va_arg_aggregate(layout, self.value(list))?
            },
            | InstData::Call { func, args } =>
                return self.call(
                    body,
                    func,
                    body.value_list(args),
                    result,
                    host,
                    self.module.function_signature(func),
                ),
            | InstData::CallIndirect { sig, args } => {
                let (&callee, args) = body
                    .value_list(args)
                    .split_first()
                    .expect("call_indirect names its callee");
                let func = self.memory.function_at(self.value(callee))?;
                let expected = self.module.signature(sig);
                if !self.admits(body, func, expected, args) {
                    return Err(UbKind::SignatureMismatch.into());
                }
                for (index, &arg) in args.iter().enumerate() {
                    if expected.param_noundef(index) && self.value(arg).is_poison() {
                        return Err(UbKind::Noundef.into());
                    }
                }
                return self.call(body, func, args, result, host, expected);
            },
            | InstData::Jump { dest } => {
                self.branch(body, dest);
                return Ok(None);
            },
            | InstData::Brif { cond, dests } => {
                let taken = match self.value(cond) {
                    | RuntimeValue::Int(1) => dests[0],
                    | RuntimeValue::Int(_) => dests[1],
                    | _ => return Err(UbKind::BranchOnPoison.into()),
                };
                self.branch(body, taken);
                return Ok(None);
            },
            | InstData::Switch {
                value,
                default,
                cases,
            } => {
                let RuntimeValue::Int(bits) = self.value(value) else {
                    return Err(UbKind::SwitchOnPoison.into());
                };
                let taken = body
                    .switch_cases(cases)
                    .find(|&(case, _)| case == bits)
                    .map_or(default, |(_, dest)| dest);
                self.branch(body, taken);
                return Ok(None);
            },
            | InstData::Return { value } => {
                let value = value.map(|value| self.value(value));
                return self.return_from(value);
            },
            | InstData::IndirectBr { addr, dests } => {
                let label = self.memory.label_at(self.value(addr), self.frame().func)?;
                let block = body.label_block(crate::ir::BlockLabel::new(label));
                let taken = body
                    .block_list(dests)
                    .find(|&dest| Some(body.block_call(dest).0) == block)
                    .ok_or(UbKind::IndirectBranch)?;
                self.branch(body, taken);
                return Ok(None);
            },
            | InstData::Unreachable => return Err(UbKind::Unreachable.into()),
        };
        if let Some(result) = result {
            self.set(result, value);
        }
        Ok(None)
    }

    /// Whether a call whose site has the signature `expected` and the
    /// operands `args` may run `func`, whose own signature may differ: the
    /// same signature, the same variadic prefix, an open host declaration,
    /// or an open call site whose promoted operands match a prototype. An
    /// indirect call and a call through an alias resolve this way.
    pub(super) fn admits(
        &self,
        body: &Body<'_>,
        func: FuncId,
        expected: crate::ir::Signature<'_>,
        args: &[Value],
    ) -> bool {
        let actual = self.module.function_signature(func);
        // A declared host function with an open external signature
        // represents a C declaration without a prototype. Its call
        // site supplies the default-promoted parameter types, also
        // through function pointers. C99: §6.5.2.2 paragraph 6, pp.
        // 71-72; PDF pp. 83-84.
        let open_host = self.module.function(func).body.is_none()
            && actual.variadic
            && actual.params.is_empty()
            && actual.result == expected.result
            && actual.result_extension == expected.result_extension;
        let open_host = open_host && actual.result_noundef == expected.result_noundef;
        // An open call site can also resolve to a prototype (for
        // example after a builtin declares the same host symbol).
        // Lowering already applied default argument promotions;
        // compare those operand types with the resolved parameters.
        // A variadic prototype cannot be called without a prototype.
        // C99: §6.5.2.2 paragraphs 6 and 9, pp. 71-72; PDF pp. 83-84.
        let compatible_open_call = expected.variadic
            && expected.params.is_empty()
            && !actual.variadic
            && actual.result == expected.result
            && actual.result_extension == expected.result_extension
            && actual.result_noundef == expected.result_noundef
            && actual.aggregates == expected.aggregates
            && actual.extensions == expected.extensions
            && actual.params.len() == args.len()
            && actual
                .params
                .iter()
                .zip(args)
                .all(|(&param, &arg)| param == body.value_type(arg));
        actual == expected || variadic_match(actual, expected) || open_host || compatible_open_call
    }
}

/// Call-site ABI descriptions after the ellipsis do not alter its function
/// type.
fn variadic_match(actual: crate::ir::Signature<'_>, expected: crate::ir::Signature<'_>) -> bool {
    let fixed = actual.fixed_count();
    actual.variadic
        && expected.variadic
        && fixed == expected.fixed_count()
        && actual.params[..fixed] == expected.params[..fixed]
        && actual.result == expected.result
        && actual.result_extension == expected.result_extension
        && actual.result_noundef == expected.result_noundef
        && actual
            .aggregates
            .iter()
            .filter(|p| (p.index as usize) < fixed)
            .eq(expected
                .aggregates
                .iter()
                .filter(|p| (p.index as usize) < fixed))
        && actual
            .extensions
            .iter()
            .filter(|p| (p.index as usize) < fixed)
            .eq(expected
                .extensions
                .iter()
                .filter(|p| (p.index as usize) < fixed))
        && actual
            .noundef
            .iter()
            .filter(|&&p| (p as usize) < fixed)
            .eq(expected.noundef.iter().filter(|&&p| (p as usize) < fixed))
}
