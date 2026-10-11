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
    Entity,
    Inst,
    InstData,
    InstFlags,
    Opcode,
    Type,
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
            | InstData::Unary { opcode, ty, arg } => {
                let value = self.value(arg);
                match opcode {
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
                    | Type::F80 | Type::F128 =>
                        return Err(Fault::Unsupported(Unsupported::WideFloat)),
                    | _ => RuntimeValue::int(ty, bits),
                }
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
            | InstData::Call { func, args } =>
                return self.call(body, func, body.value_list(args), result, host),
            | InstData::CallIndirect { sig, args } => {
                let (&callee, args) = body
                    .value_list(args)
                    .split_first()
                    .expect("call_indirect names its callee");
                let func = self.memory.function_at(self.value(callee))?;
                let expected = self.module.signature(sig);
                if self.module.function_signature(func) != expected {
                    return Err(UbKind::SignatureMismatch.into());
                }
                return self.call(body, func, args, result, host);
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
                return Ok(self.return_from(value));
            },
            | InstData::Unreachable => return Err(UbKind::Unreachable.into()),
        };
        if let Some(result) = result {
            self.set(result, value);
        }
        Ok(None)
    }
}
