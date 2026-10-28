//! Instruction selection for computation: the dispatcher over a block's
//! instructions, integer and floating arithmetic, comparisons, bit
//! operations and conversions.
//!
//! `i128` addition, subtraction, bitwise operations, comparisons and
//! extensions are inline on the two halves; multiplication, division,
//! shifts and conversions to and from floating types call the
//! compiler-rt routines Clang calls on wasm32 (`__multi3`, `__divti3`,
//! `__ashlti3`, `__floattidf`, ...). `f128` (`long double`) computes
//! entirely through compiler-rt's soft-float routines (`__addtf3`,
//! `__eqtf2`, `__extenddftf2`, ...).

use super::{
    Codegen,
    calls::{
        Arg,
        Routine,
    },
    values::{
        Repr,
        narrow,
    },
};
use crate::{
    backend::wasm::{
        instr::{
            Instr,
            ValType,
            op,
        },
        low32,
    },
    ir::{
        FloatCC,
        Inst,
        InstData,
        IntCC,
        Opcode,
        Type,
        Value,
    },
};

impl Codegen<'_, '_, '_, '_, '_> {
    /// Translates one non-terminator instruction.
    pub(super) fn instruction(&mut self, inst: Inst) {
        let result = self.body.inst_result(inst);
        if result.is_some_and(|value| self.recomputed(value)) {
            return;
        }
        let data = *self.body.inst(inst);
        match data {
            | InstData::Binary {
                opcode, ty, args, ..
            } => self.binary(opcode, ty, args, result),
            | InstData::Unary { opcode, ty, arg } => self.unary(opcode, ty, arg, result),
            | InstData::IntCompare { cond, ty, args } => {
                self.int_compare(cond, ty, args);
                self.store_result(result);
            },
            | InstData::FloatCompare { cond, ty, args } => {
                self.float_compare(cond, ty, args);
                self.store_result(result);
            },
            | InstData::Select { ty, args } => {
                if self.repr(ty) == Repr::Pair {
                    for high in [false, true] {
                        self.push_part(args[1], high);
                        self.push_part(args[2], high);
                        self.push(args[0]);
                        self.emit(Instr::Select);
                    }
                } else {
                    self.push(args[1]);
                    self.push(args[2]);
                    self.push(args[0]);
                    self.emit(Instr::Select);
                }
                self.store_result(result);
            },
            | InstData::Nullary { opcode, ty } => self.nullary(opcode, ty, result),
            | InstData::ObjectSize { ty, query, .. } => {
                // An unanswered query gets the unknown answer, as in the
                // other back ends.
                self.constant_of(ty, query.unknown(ty), false);
                self.store_result(result);
            },
            | InstData::Load {
                ty, align, addr, ..
            } => self.load(ty, align, addr, result),
            | InstData::Store {
                ty, align, args, ..
            } => self.store(ty, align, args[0], args[1]),
            | InstData::MemoryRange { opcode, args, .. } => self.memory_range(opcode, args),
            | InstData::Atomic {
                op,
                ty,
                align,
                args,
                ..
            } => {
                let args = self.body.value_list(args);
                self.atomic(op, ty, align, args, result);
            },
            | InstData::DynAlloca { size, align, .. } => self.dyn_alloca(size, align, result),
            | InstData::Call { func, args } => {
                let args = self.body.value_list(args);
                self.direct_call(func, args, result);
            },
            | InstData::CallIndirect { sig, args } => {
                let args = self.body.value_list(args);
                self.indirect_call(sig, args, result);
            },
            | InstData::InlineAsm { .. } => self.refuse("inline assembly"),
            | InstData::VaArgAggregate { .. } =>
                self.refuse("va_arg of an aggregate before ABI lowering"),
            // Recomputed where they are used, and the terminators, which
            // the stackifier's emitter translates.
            | InstData::Const { .. }
            | InstData::WideConst { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. }
            | InstData::BlockAddr { .. }
            | InstData::Jump { .. }
            | InstData::Brif { .. }
            | InstData::Switch { .. }
            | InstData::Return { .. }
            | InstData::IndirectBr { .. }
            | InstData::Unreachable => {},
        }
    }

    /// Stores the value on the stack into `result`, if any.
    pub(super) fn store_result(&mut self, result: Option<Value>) {
        if let Some(result) = result {
            self.set(result);
        } else {
            self.emit(Instr::Drop);
        }
    }

    fn nullary(&mut self, opcode: Opcode, ty: Type, result: Option<Value>) {
        match opcode {
            | Opcode::Trap | Opcode::DebugTrap => self.emit(Instr::Unreachable),
            | Opcode::StackSave => {
                self.emit(Instr::GlobalGet(0));
                self.store_result(result);
            },
            | Opcode::FrameAddress => {
                self.emit(Instr::LocalGet(self.fp));
                self.store_result(result);
            },
            | Opcode::ReturnAddress =>
                self.refuse("__builtin_return_address (WebAssembly code has no addresses)"),
            | Opcode::ReadTimestamp => self.refuse("reading the time-stamp counter"),
            | Opcode::SjljSetjmp => self.refuse("__builtin_setjmp"),
            | _ => {
                self.zero(ty);
                self.store_result(result);
            },
        }
    }

    /// Masks the `i32` on the stack to `ty`'s width, if `ty` is narrower.
    pub(super) fn mask(&mut self, ty: Type) {
        if let Some(width) = narrow(ty) {
            self.i32_const(low32((1_u128 << width) - 1).cast_signed());
            self.num(op::I32_AND);
        }
    }

    /// Sign-extends the zero-extended narrow `i32` on the stack.
    pub(super) fn sign_extend(&mut self, ty: Type) {
        match narrow(ty) {
            | Some(8) => self.num(op::I32_EXTEND8_S),
            | Some(16) => self.num(op::I32_EXTEND16_S),
            | Some(width) => {
                self.i32_const((32 - width) as i32);
                self.num(op::I32_SHL);
                self.i32_const((32 - width) as i32);
                self.num(op::I32_SHR_S);
            },
            | None => {},
        }
    }

    /// Pushes `value` sign-extended to its full `i32` or `i64` width.
    fn push_signed(&mut self, value: Value) {
        self.push(value);
        self.sign_extend(self.ty(value));
    }

    fn binary(&mut self, opcode: Opcode, ty: Type, args: [Value; 2], result: Option<Value>) {
        let [a, b] = args;
        match opcode {
            | Opcode::PtrAdd => {
                self.push(a);
                self.push_wrapped(b);
                self.num(op::I32_ADD);
            },
            | Opcode::Subobject => self.push(a),
            | Opcode::VaCopy => {
                self.push(a);
                self.push(b);
                self.emit(Instr::Memory {
                    opcode: op::I32_LOAD,
                    align:  2,
                    offset: 0,
                });
                self.emit(Instr::Memory {
                    opcode: op::I32_STORE,
                    align:  2,
                    offset: 0,
                });
                return;
            },
            | Opcode::UaddOverflow | Opcode::UmulOverflow => self.overflow(opcode, ty, a, b),
            | _ if opcode.is_float_binary() =>
                if !self.float_binary(opcode, ty, a, b, result) {
                    return;
                },
            | _ => match self.repr(ty) {
                | Repr::Pair => {
                    self.wide_binary(opcode, a, b, result);
                    return;
                },
                | Repr::I64 => self.int_binary64(opcode, a, b),
                | _ => self.int_binary32(opcode, ty, a, b),
            },
        }
        self.store_result(result);
    }

    /// Pushes an `i64` offset or size wrapped to 32 bits, folding constants.
    pub(super) fn push_wrapped(&mut self, value: Value) {
        if let Some(bits) = self.constant(value) {
            self.i32_const(low32(bits) as i32);
            return;
        }
        self.push(value);
        match self.repr(self.ty(value)) {
            | Repr::I64 => self.num(op::I32_WRAP_I64),
            | Repr::Pair => {
                self.emit(Instr::Drop);
                self.num(op::I32_WRAP_I64);
            },
            | _ => {},
        }
    }

    fn int_binary32(&mut self, opcode: Opcode, ty: Type, a: Value, b: Value) {
        let (code, signed, masked) = match opcode {
            | Opcode::Iadd => (op::I32_ADD, false, true),
            | Opcode::Isub => (op::I32_SUB, false, true),
            | Opcode::Imul => (op::I32_MUL, false, true),
            | Opcode::Shl => (op::I32_SHL, false, true),
            | Opcode::And => (op::I32_AND, false, false),
            | Opcode::Or => (op::I32_OR, false, false),
            | Opcode::Xor => (op::I32_XOR, false, false),
            | Opcode::Lshr => (op::I32_SHR_U, false, false),
            | Opcode::Udiv => (op::I32_DIV_U, false, false),
            | Opcode::Urem => (op::I32_REM_U, false, false),
            | Opcode::Ashr => (op::I32_SHR_S, true, true),
            | Opcode::Sdiv => (op::I32_DIV_S, true, true),
            | Opcode::Srem => (op::I32_REM_S, true, true),
            | _ => {
                self.refuse("an integer operation");
                (op::I32_ADD, false, false)
            },
        };
        if signed {
            self.push_signed(a);
            if opcode == Opcode::Ashr {
                self.push(b);
            } else {
                self.push_signed(b);
            }
        } else {
            self.push(a);
            self.push(b);
        }
        self.num(code);
        if masked {
            self.mask(ty);
        }
    }

    fn int_binary64(&mut self, opcode: Opcode, a: Value, b: Value) {
        let code = match opcode {
            | Opcode::Iadd => op::I64_ADD,
            | Opcode::Isub => op::I64_SUB,
            | Opcode::Imul => op::I64_MUL,
            | Opcode::Shl => op::I64_SHL,
            | Opcode::And => op::I64_AND,
            | Opcode::Or => op::I64_OR,
            | Opcode::Xor => op::I64_XOR,
            | Opcode::Lshr => op::I64_SHR_U,
            | Opcode::Udiv => op::I64_DIV_U,
            | Opcode::Urem => op::I64_REM_U,
            | Opcode::Ashr => op::I64_SHR_S,
            | Opcode::Sdiv => op::I64_DIV_S,
            | Opcode::Srem => op::I64_REM_S,
            | _ => {
                self.refuse("an integer operation");
                op::I64_ADD
            },
        };
        self.push(a);
        self.push(b);
        self.num(code);
    }

    /// `i128` arithmetic: inline where it is cheap, else a routine.
    fn wide_binary(&mut self, opcode: Opcode, a: Value, b: Value, result: Option<Value>) {
        let Some(result) = result else { return };
        match opcode {
            | Opcode::And | Opcode::Or | Opcode::Xor => {
                let code = match opcode {
                    | Opcode::And => op::I64_AND,
                    | Opcode::Or => op::I64_OR,
                    | _ => op::I64_XOR,
                };
                for high in [false, true] {
                    self.push_part(a, high);
                    self.push_part(b, high);
                    self.num(code);
                    self.set_part(result, high);
                }
            },
            | Opcode::Iadd | Opcode::Isub => {
                let add = opcode == Opcode::Iadd;
                let low = self.temp(ValType::I64, 0);
                self.push_part(a, false);
                self.push_part(b, false);
                self.num(if add { op::I64_ADD } else { op::I64_SUB });
                self.emit(Instr::LocalSet(low));
                // The carry out of the low half, or the borrow into it.
                self.push_part(a, true);
                self.push_part(b, true);
                self.num(if add { op::I64_ADD } else { op::I64_SUB });
                if add {
                    self.emit(Instr::LocalGet(low));
                    self.push_part(a, false);
                } else {
                    self.push_part(a, false);
                    self.push_part(b, false);
                }
                self.num(op::I64_LT_U);
                self.num(op::I64_EXTEND_I32_U);
                self.num(if add { op::I64_ADD } else { op::I64_SUB });
                self.set_part(result, true);
                self.emit(Instr::LocalGet(low));
                self.set_part(result, false);
            },
            | Opcode::Shl | Opcode::Lshr | Opcode::Ashr => {
                let routine = match opcode {
                    | Opcode::Shl => Routine::Ashlti3,
                    | Opcode::Lshr => Routine::Lshrti3,
                    | _ => Routine::Ashrti3,
                };
                self.call_routine(routine, &[Arg::Value(a), Arg::Low32(b)], Some(result));
            },
            | _ => {
                let routine = match opcode {
                    | Opcode::Imul => Routine::Multi3,
                    | Opcode::Sdiv => Routine::Divti3,
                    | Opcode::Udiv => Routine::Udivti3,
                    | Opcode::Srem => Routine::Modti3,
                    | Opcode::Urem => Routine::Umodti3,
                    | _ => {
                        self.refuse("an i128 operation");
                        return;
                    },
                };
                self.call_routine(routine, &[Arg::Value(a), Arg::Value(b)], Some(result));
            },
        }
    }

    /// Floating arithmetic. Returns whether the result is on the stack.
    fn float_binary(
        &mut self,
        opcode: Opcode,
        ty: Type,
        a: Value,
        b: Value,
        result: Option<Value>,
    ) -> bool {
        let routine = match (ty, opcode) {
            | (Type::F32 | Type::F64, Opcode::Frem) => Some(if ty == Type::F32 {
                Routine::Fmodf
            } else {
                Routine::Fmod
            }),
            | (Type::F128, Opcode::Fadd) => Some(Routine::Addtf3),
            | (Type::F128, Opcode::Fsub) => Some(Routine::Subtf3),
            | (Type::F128, Opcode::Fmul) => Some(Routine::Multf3),
            | (Type::F128, Opcode::Fdiv) => Some(Routine::Divtf3),
            | (Type::F128, _) => Some(Routine::Fmodl),
            | _ => None,
        };
        if let Some(routine) = routine {
            let args = [Arg::Value(a), Arg::Value(b)];
            if ty == Type::F128 {
                self.call_routine(routine, &args, result);
                return false;
            }
            self.call_routine_scalar(routine, &args);
            return true;
        }
        let f64 = match ty {
            | Type::F32 => false,
            | Type::F64 => true,
            | _ => {
                self.refuse("floating arithmetic on this type");
                false
            },
        };
        let code = match (opcode, f64) {
            | (Opcode::Fadd, false) => op::F32_ADD,
            | (Opcode::Fsub, false) => op::F32_SUB,
            | (Opcode::Fmul, false) => op::F32_MUL,
            | (Opcode::Fdiv, false) => op::F32_DIV,
            | (Opcode::Fadd, true) => op::F64_ADD,
            | (Opcode::Fsub, true) => op::F64_SUB,
            | (Opcode::Fmul, true) => op::F64_MUL,
            | _ => op::F64_DIV,
        };
        self.push(a);
        self.push(b);
        self.num(code);
        true
    }

    /// `uadd_overflow` and `umul_overflow`: whether the unsigned operation
    /// wraps.
    fn overflow(&mut self, opcode: Opcode, ty: Type, a: Value, b: Value) {
        let add = opcode == Opcode::UaddOverflow;
        match (self.repr(ty), narrow(ty)) {
            | (Repr::I32, Some(width)) => {
                self.push(a);
                self.push(b);
                self.num(if add { op::I32_ADD } else { op::I32_MUL });
                self.i32_const(low32((1_u128 << width) - 1).cast_signed());
                self.num(op::I32_GT_U);
            },
            | (Repr::I32, None) if add => {
                self.push(a);
                self.push(b);
                self.num(op::I32_ADD);
                self.push(a);
                self.num(op::I32_LT_U);
            },
            | (Repr::I32, None) => {
                self.push(a);
                self.num(op::I64_EXTEND_I32_U);
                self.push(b);
                self.num(op::I64_EXTEND_I32_U);
                self.num(op::I64_MUL);
                self.i64_const(32);
                self.num(op::I64_SHR_U);
                self.num(op::I64_EQZ);
                self.num(op::I32_EQZ);
            },
            | (Repr::I64, _) if add => {
                self.push(a);
                self.push(b);
                self.num(op::I64_ADD);
                self.push(a);
                self.num(op::I64_LT_U);
            },
            | (Repr::I64, _) => {
                // (a * b) / a != b, with a zero `a` dividing by one.
                self.push(a);
                self.push(b);
                self.num(op::I64_MUL);
                self.push(a);
                self.push(a);
                self.num(op::I64_EQZ);
                self.num(op::I64_EXTEND_I32_U);
                self.num(op::I64_OR);
                self.num(op::I64_DIV_U);
                self.push(b);
                self.num(op::I64_NE);
                self.push(a);
                self.num(op::I64_EQZ);
                self.num(op::I32_EQZ);
                self.num(op::I32_AND);
            },
            | (Repr::Pair, _) if add => {
                // The sum's high half is below a's, or equal with the low
                // half below a's.
                let low = self.temp(ValType::I64, 0);
                let high = self.temp(ValType::I64, 1);
                self.push_part(a, false);
                self.push_part(b, false);
                self.num(op::I64_ADD);
                self.emit(Instr::LocalTee(low));
                self.push_part(a, false);
                self.num(op::I64_LT_U);
                self.num(op::I64_EXTEND_I32_U);
                self.push_part(a, true);
                self.num(op::I64_ADD);
                self.push_part(b, true);
                self.num(op::I64_ADD);
                self.emit(Instr::LocalTee(high));
                self.push_part(a, true);
                self.num(op::I64_LT_U);
                self.emit(Instr::LocalGet(high));
                self.push_part(a, true);
                self.num(op::I64_EQ);
                self.emit(Instr::LocalGet(low));
                self.push_part(a, false);
                self.num(op::I64_LT_U);
                self.num(op::I32_AND);
                self.num(op::I32_OR);
            },
            | _ => {
                self.refuse("128-bit multiplication overflow checks");
                self.i32_const(0);
            },
        }
    }

    fn unary(&mut self, opcode: Opcode, ty: Type, arg: Value, result: Option<Value>) {
        match opcode {
            | Opcode::VaStart => {
                let Some(varargs) = self.varargs else {
                    self.refuse("va_start outside a variadic function");
                    return;
                };
                self.push(arg);
                self.emit(Instr::LocalGet(varargs));
                self.emit(Instr::Memory {
                    opcode: op::I32_STORE,
                    align:  2,
                    offset: 0,
                });
                return;
            },
            | Opcode::VaEnd => return,
            | Opcode::StackRestore => {
                self.push(arg);
                self.emit(Instr::GlobalSet(0));
                return;
            },
            | Opcode::SjljLongjmp => {
                self.refuse("__builtin_longjmp");
                return;
            },
            | Opcode::VaArg => {
                self.refuse("va_arg before ABI lowering");
                return;
            },
            | Opcode::Freeze => {
                self.push(arg);
                self.store_result(result);
                return;
            },
            | Opcode::Fneg => {
                match ty {
                    | Type::F32 => {
                        self.push(arg);
                        self.num(op::F32_NEG);
                    },
                    | Type::F64 => {
                        self.push(arg);
                        self.num(op::F64_NEG);
                    },
                    | _ => {
                        self.push_part(arg, false);
                        self.push_part(arg, true);
                        self.i64_const(i64::MIN);
                        self.num(op::I64_XOR);
                    },
                }
                self.store_result(result);
                return;
            },
            | _ if opcode.is_bit_unary() => {
                self.bit_unary(opcode, ty, arg, result);
                return;
            },
            | _ => {},
        }
        let from = self.ty(arg);
        if from == Type::F128 || ty == Type::F128 {
            self.quad_conversion(opcode, from, ty, arg, result);
            return;
        }
        if (from == Type::I128 || ty == Type::I128)
            && matches!(
                opcode,
                Opcode::Fptosi | Opcode::Fptoui | Opcode::Sitofp | Opcode::Uitofp
            )
        {
            self.wide_float_conversion(opcode, from, ty, arg, result);
            return;
        }
        if from == Type::F16 || ty == Type::F16 {
            if opcode == Opcode::Bitcast {
                self.push(arg);
                self.store_result(result);
            } else {
                self.refuse("_Float16 conversions");
            }
            return;
        }
        self.conversion(opcode, from, ty, arg);
        self.store_result(result);
    }

    /// Integer and pointer conversions, and those among `f32`, `f64` and
    /// integers of at most 64 bits. Pushes the result.
    fn conversion(&mut self, opcode: Opcode, from: Type, to: Type, arg: Value) {
        let from_repr = self.repr(from);
        let to_repr = self.repr(to);
        match opcode {
            | Opcode::Zext | Opcode::Ptrtoint | Opcode::Inttoptr | Opcode::Trunc | Opcode::Sext => {
                let signed = opcode == Opcode::Sext;
                match (from_repr, to_repr) {
                    | (Repr::I32, Repr::I32) => {
                        self.push(arg);
                        if signed {
                            self.sign_extend(from);
                        }
                        self.mask(to);
                    },
                    | (Repr::I32, Repr::I64) => {
                        self.push(arg);
                        if signed {
                            self.sign_extend(from);
                            self.num(op::I64_EXTEND_I32_S);
                        } else {
                            self.num(op::I64_EXTEND_I32_U);
                        }
                    },
                    | (Repr::I64, Repr::I32) => {
                        self.push(arg);
                        self.num(op::I32_WRAP_I64);
                        self.mask(to);
                    },
                    | (Repr::I64, Repr::I64) | (Repr::Pair, Repr::Pair) => self.push(arg),
                    | (Repr::I32 | Repr::I64, Repr::Pair) => {
                        self.push(arg);
                        if from_repr == Repr::I32 {
                            if signed {
                                self.sign_extend(from);
                                self.num(op::I64_EXTEND_I32_S);
                            } else {
                                self.num(op::I64_EXTEND_I32_U);
                            }
                        }
                        if signed {
                            let low = self.temp(ValType::I64, 0);
                            self.emit(Instr::LocalTee(low));
                            self.emit(Instr::LocalGet(low));
                            self.i64_const(63);
                            self.num(op::I64_SHR_S);
                        } else {
                            self.i64_const(0);
                        }
                    },
                    | (Repr::Pair, Repr::I64) => self.push_part(arg, false),
                    | (Repr::Pair, Repr::I32) => {
                        self.push_part(arg, false);
                        self.num(op::I32_WRAP_I64);
                        self.mask(to);
                    },
                    | _ => self.refuse("a conversion"),
                }
            },
            | Opcode::Bitcast => {
                self.push(arg);
                match (from_repr, to_repr) {
                    | (Repr::I32, Repr::F32) => self.num(op::F32_REINTERPRET_I32),
                    | (Repr::F32, Repr::I32) => self.num(op::I32_REINTERPRET_F32),
                    | (Repr::I64, Repr::F64) => self.num(op::F64_REINTERPRET_I64),
                    | (Repr::F64, Repr::I64) => self.num(op::I64_REINTERPRET_F64),
                    | (a, b) if a == b => {},
                    | _ => self.refuse("a bitcast"),
                }
            },
            | Opcode::Fpext => {
                self.push(arg);
                self.num(op::F64_PROMOTE_F32);
            },
            | Opcode::Fptrunc => {
                self.push(arg);
                self.num(op::F32_DEMOTE_F64);
            },
            | Opcode::Fptosi | Opcode::Fptoui => {
                let signed = opcode == Opcode::Fptosi;
                self.push(arg);
                // Out-of-range values are poison, so saturation will do.
                let sub = match (to_repr == Repr::I64, from == Type::F64, signed) {
                    | (false, false, true) => op::I32_TRUNC_SAT_F32_S,
                    | (false, false, false) => op::I32_TRUNC_SAT_F32_U,
                    | (false, true, true) => op::I32_TRUNC_SAT_F64_S,
                    | (false, true, false) => op::I32_TRUNC_SAT_F64_U,
                    | (true, false, true) => op::I64_TRUNC_SAT_F32_S,
                    | (true, false, false) => op::I64_TRUNC_SAT_F32_U,
                    | (true, true, true) => op::I64_TRUNC_SAT_F64_S,
                    | (true, true, false) => op::I64_TRUNC_SAT_F64_U,
                };
                self.emit(Instr::TruncSat(sub));
                if to_repr == Repr::I32 {
                    self.mask(to);
                }
            },
            | Opcode::Sitofp | Opcode::Uitofp => {
                let signed = opcode == Opcode::Sitofp;
                self.push(arg);
                if signed {
                    self.sign_extend(from);
                }
                let code = match (from_repr == Repr::I64, to == Type::F64, signed) {
                    | (false, false, true) => op::F32_CONVERT_I32_S,
                    | (false, false, false) => op::F32_CONVERT_I32_U,
                    | (false, true, true) => op::F64_CONVERT_I32_S,
                    | (false, true, false) => op::F64_CONVERT_I32_U,
                    | (true, false, true) => op::F32_CONVERT_I64_S,
                    | (true, false, false) => op::F32_CONVERT_I64_U,
                    | (true, true, true) => op::F64_CONVERT_I64_S,
                    | (true, true, false) => op::F64_CONVERT_I64_U,
                };
                self.num(code);
            },
            | _ => self.refuse("a conversion"),
        }
    }

    /// Conversions to or from `f128`, through compiler-rt.
    fn quad_conversion(
        &mut self,
        opcode: Opcode,
        from: Type,
        to: Type,
        arg: Value,
        result: Option<Value>,
    ) {
        let routine = match (opcode, from, to) {
            | (Opcode::Bitcast, _, _) => {
                self.push(arg);
                self.store_result(result);
                return;
            },
            | (Opcode::Fpext, Type::F32, _) => Routine::Extendsftf2,
            | (Opcode::Fpext, Type::F64, _) => Routine::Extenddftf2,
            | (Opcode::Fptrunc, _, Type::F32) => Routine::Trunctfsf2,
            | (Opcode::Fptrunc, _, Type::F64) => Routine::Trunctfdf2,
            | (Opcode::Fptosi, _, Type::I128) => Routine::Fixtfti,
            | (Opcode::Fptoui, _, Type::I128) => Routine::Fixunstfti,
            | (Opcode::Fptosi, _, Type::I64) => Routine::Fixtfdi,
            | (Opcode::Fptoui, _, Type::I64) => Routine::Fixunstfdi,
            | (Opcode::Fptosi, _, _) => Routine::Fixtfsi,
            | (Opcode::Fptoui, _, _) => Routine::Fixunstfsi,
            | (Opcode::Sitofp, Type::I128, _) => Routine::Floattitf,
            | (Opcode::Uitofp, Type::I128, _) => Routine::Floatuntitf,
            | (Opcode::Sitofp, Type::I64, _) => Routine::Floatditf,
            | (Opcode::Uitofp, Type::I64, _) => Routine::Floatunditf,
            | (Opcode::Sitofp, _, _) => Routine::Floatsitf,
            | (Opcode::Uitofp, _, _) => Routine::Floatunsitf,
            | _ => {
                self.refuse("a long double conversion");
                return;
            },
        };
        let args = [if matches!(opcode, Opcode::Sitofp) {
            Arg::Signed(arg)
        } else {
            Arg::Value(arg)
        }];
        if to == Type::F128 || to == Type::I128 {
            self.call_routine(routine, &args, result);
        } else {
            self.call_routine_scalar(routine, &args);
            if matches!(opcode, Opcode::Fptosi | Opcode::Fptoui) {
                self.mask(to);
            }
            self.store_result(result);
        }
    }

    /// Conversions between `i128` and `f32` or `f64`, through compiler-rt.
    fn wide_float_conversion(
        &mut self,
        opcode: Opcode,
        from: Type,
        to: Type,
        arg: Value,
        result: Option<Value>,
    ) {
        let routine = match (opcode, from == Type::F64 || to == Type::F64) {
            | (Opcode::Fptosi, false) => Routine::Fixsfti,
            | (Opcode::Fptosi, true) => Routine::Fixdfti,
            | (Opcode::Fptoui, false) => Routine::Fixunssfti,
            | (Opcode::Fptoui, true) => Routine::Fixunsdfti,
            | (Opcode::Sitofp, false) => Routine::Floattisf,
            | (Opcode::Sitofp, true) => Routine::Floattidf,
            | (Opcode::Uitofp, false) => Routine::Floatuntisf,
            | _ => Routine::Floatuntidf,
        };
        let args = [Arg::Value(arg)];
        if to == Type::I128 {
            self.call_routine(routine, &args, result);
        } else {
            self.call_routine_scalar(routine, &args);
            self.store_result(result);
        }
    }

    fn bit_unary(&mut self, opcode: Opcode, ty: Type, arg: Value, result: Option<Value>) {
        match self.repr(ty) {
            | Repr::I32 => {
                let width = narrow(ty).unwrap_or(32);
                match opcode {
                    | Opcode::Clz | Opcode::ClzDefined => {
                        self.push(arg);
                        self.num(op::I32_CLZ);
                        if width < 32 {
                            self.i32_const((32 - width) as i32);
                            self.num(op::I32_SUB);
                        }
                    },
                    | Opcode::Ctz | Opcode::CtzDefined => {
                        self.push(arg);
                        if width < 32 {
                            self.i32_const(1 << width);
                            self.num(op::I32_OR);
                        }
                        self.num(op::I32_CTZ);
                    },
                    | Opcode::Popcount => {
                        self.push(arg);
                        self.num(op::I32_POPCNT);
                    },
                    | _ => match width {
                        | 16 => {
                            self.push(arg);
                            self.i32_const(8);
                            self.num(op::I32_SHL);
                            self.push(arg);
                            self.i32_const(8);
                            self.num(op::I32_SHR_U);
                            self.num(op::I32_OR);
                            self.mask(ty);
                        },
                        | 32 => {
                            self.push(arg);
                            self.bswap32();
                        },
                        | _ => self.push(arg),
                    },
                }
            },
            | Repr::I64 => match opcode {
                | Opcode::Clz | Opcode::ClzDefined => {
                    self.push(arg);
                    self.num(op::I64_CLZ);
                },
                | Opcode::Ctz | Opcode::CtzDefined => {
                    self.push(arg);
                    self.num(op::I64_CTZ);
                },
                | Opcode::Popcount => {
                    self.push(arg);
                    self.num(op::I64_POPCNT);
                },
                | _ => {
                    self.push(arg);
                    self.bswap64();
                },
            },
            | _ => {
                let Some(result) = result else { return };
                match opcode {
                    | Opcode::Clz | Opcode::ClzDefined | Opcode::Ctz | Opcode::CtzDefined => {
                        // The count of the half that decides, plus 64 when
                        // that half is zero.
                        let leading = matches!(opcode, Opcode::Clz | Opcode::ClzDefined);
                        let count = if leading { op::I64_CLZ } else { op::I64_CTZ };
                        let (first, second) = if leading {
                            (true, false)
                        } else {
                            (false, true)
                        };
                        self.push_part(arg, first);
                        self.num(count);
                        self.push_part(arg, second);
                        self.num(count);
                        self.i64_const(64);
                        self.num(op::I64_ADD);
                        self.push_part(arg, first);
                        self.num(op::I64_EQZ);
                        self.num(op::I32_EQZ);
                        self.emit(Instr::Select);
                        self.set_part(result, false);
                        self.i64_const(0);
                        self.set_part(result, true);
                    },
                    | Opcode::Popcount => {
                        self.push_part(arg, false);
                        self.num(op::I64_POPCNT);
                        self.push_part(arg, true);
                        self.num(op::I64_POPCNT);
                        self.num(op::I64_ADD);
                        self.set_part(result, false);
                        self.i64_const(0);
                        self.set_part(result, true);
                    },
                    | _ => {
                        self.push_part(arg, true);
                        self.bswap64();
                        self.set_part(result, false);
                        self.push_part(arg, false);
                        self.bswap64();
                        self.set_part(result, true);
                    },
                }
                return;
            },
        }
        self.store_result(result);
    }

    /// Reverses the bytes of the `i32` on the stack.
    fn bswap32(&mut self) {
        let x = self.temp(ValType::I32, 0);
        self.emit(Instr::LocalTee(x));
        self.i32_const(8);
        self.num(op::I32_ROTL);
        self.i32_const(0x00FF_00FF);
        self.num(op::I32_AND);
        self.emit(Instr::LocalGet(x));
        self.i32_const(8);
        self.num(op::I32_ROTR);
        self.i32_const(0xFF00_FF00_u32 as i32);
        self.num(op::I32_AND);
        self.num(op::I32_OR);
    }

    /// Reverses the bytes of the `i64` on the stack.
    fn bswap64(&mut self) {
        let x = self.temp(ValType::I64, 0);
        self.emit(Instr::LocalTee(x));
        self.num(op::I32_WRAP_I64);
        self.bswap32();
        self.num(op::I64_EXTEND_I32_U);
        self.i64_const(32);
        self.num(op::I64_SHL);
        self.emit(Instr::LocalGet(x));
        self.i64_const(32);
        self.num(op::I64_SHR_U);
        self.num(op::I32_WRAP_I64);
        self.bswap32();
        self.num(op::I64_EXTEND_I32_U);
        self.num(op::I64_OR);
    }

    /// Pushes the `i32` truth of an integer comparison.
    pub(super) fn int_compare(&mut self, cond: IntCC, ty: Type, [a, b]: [Value; 2]) {
        let signed = matches!(cond, IntCC::Slt | IntCC::Sle | IntCC::Sgt | IntCC::Sge);
        match self.repr(ty) {
            | Repr::I32 => {
                if signed {
                    self.push_signed(a);
                    self.push_signed(b);
                } else {
                    self.push(a);
                    self.push(b);
                }
                self.num(match cond {
                    | IntCC::Eq => op::I32_EQ,
                    | IntCC::Ne => op::I32_NE,
                    | IntCC::Slt => op::I32_LT_S,
                    | IntCC::Sle => op::I32_LE_S,
                    | IntCC::Sgt => op::I32_GT_S,
                    | IntCC::Sge => op::I32_GE_S,
                    | IntCC::Ult => op::I32_LT_U,
                    | IntCC::Ule => op::I32_LE_U,
                    | IntCC::Ugt => op::I32_GT_U,
                    | IntCC::Uge => op::I32_GE_U,
                });
            },
            | Repr::I64 => {
                self.push(a);
                self.push(b);
                self.num(i64_compare(cond));
            },
            | _ => match cond {
                | IntCC::Eq | IntCC::Ne => {
                    self.push_part(a, false);
                    self.push_part(b, false);
                    self.num(op::I64_XOR);
                    self.push_part(a, true);
                    self.push_part(b, true);
                    self.num(op::I64_XOR);
                    self.num(op::I64_OR);
                    self.num(op::I64_EQZ);
                    if cond == IntCC::Ne {
                        self.num(op::I32_EQZ);
                    }
                },
                | _ => {
                    // The high halves decide unless they are equal; then
                    // the low halves, unsigned.
                    let low = match cond {
                        | IntCC::Slt | IntCC::Ult => IntCC::Ult,
                        | IntCC::Sle | IntCC::Ule => IntCC::Ule,
                        | IntCC::Sgt | IntCC::Ugt => IntCC::Ugt,
                        | _ => IntCC::Uge,
                    };
                    let strict = match cond {
                        | IntCC::Sle => IntCC::Slt,
                        | IntCC::Sge => IntCC::Sgt,
                        | IntCC::Ule => IntCC::Ult,
                        | IntCC::Uge => IntCC::Ugt,
                        | other => other,
                    };
                    self.push_part(a, false);
                    self.push_part(b, false);
                    self.num(i64_compare(low));
                    self.push_part(a, true);
                    self.push_part(b, true);
                    self.num(i64_compare(strict));
                    self.push_part(a, true);
                    self.push_part(b, true);
                    self.num(op::I64_EQ);
                    self.emit(Instr::Select);
                },
            },
        }
    }

    /// Pushes the `i32` truth of a floating comparison.
    fn float_compare(&mut self, cond: FloatCC, ty: Type, [a, b]: [Value; 2]) {
        if ty == Type::F128 {
            self.quad_compare(cond, a, b);
            return;
        }
        let f64 = ty == Type::F64;
        let pick = |f32_op: u8, f64_op: u8| if f64 { f64_op } else { f32_op };
        let (eq, ne, lt, gt, le, ge) = (
            pick(op::F32_EQ, op::F64_EQ),
            pick(op::F32_NE, op::F64_NE),
            pick(op::F32_LT, op::F64_LT),
            pick(op::F32_GT, op::F64_GT),
            pick(op::F32_LE, op::F64_LE),
            pick(op::F32_GE, op::F64_GE),
        );
        // Each predicate as a direct comparison, negated or combined.
        let (code, negate) = match cond {
            | FloatCC::Oeq => (eq, false),
            | FloatCC::Une => (ne, false),
            | FloatCC::Olt => (lt, false),
            | FloatCC::Ole => (le, false),
            | FloatCC::Ogt => (gt, false),
            | FloatCC::Oge => (ge, false),
            | FloatCC::Ult => (ge, true),
            | FloatCC::Ule => (gt, true),
            | FloatCC::Ugt => (le, true),
            | FloatCC::Uge => (lt, true),
            | FloatCC::One | FloatCC::Ueq => {
                self.push(a);
                self.push(b);
                self.num(lt);
                self.push(a);
                self.push(b);
                self.num(gt);
                self.num(op::I32_OR);
                if cond == FloatCC::Ueq {
                    self.num(op::I32_EQZ);
                }
                return;
            },
            | FloatCC::Ord | FloatCC::Uno => {
                self.push(a);
                self.push(a);
                self.num(eq);
                self.push(b);
                self.push(b);
                self.num(eq);
                self.num(op::I32_AND);
                if cond == FloatCC::Uno {
                    self.num(op::I32_EQZ);
                }
                return;
            },
        };
        self.push(a);
        self.push(b);
        self.num(code);
        if negate {
            self.num(op::I32_EQZ);
        }
    }

    /// `f128` comparisons through compiler-rt's comparison routines, whose
    /// results order the operands (`__lttf2 < 0` when `a < b`), and
    /// `__unordtf2`, nonzero when either is a NaN. An unordered pair makes
    /// `__getf2` and `__gttf2` return -1 and `__letf2` and `__lttf2` 1,
    /// so each unordered predicate is the opposite ordered test.
    fn quad_compare(&mut self, cond: FloatCC, a: Value, b: Value) {
        let (routine, test) = match cond {
            | FloatCC::Oeq => (Routine::Eqtf2, op::I32_EQZ),
            | FloatCC::Une => (Routine::Netf2, 0),
            | FloatCC::Olt => (Routine::Lttf2, op::I32_LT_S),
            | FloatCC::Ole => (Routine::Letf2, op::I32_LE_S),
            | FloatCC::Ogt => (Routine::Gttf2, op::I32_GT_S),
            | FloatCC::Oge => (Routine::Getf2, op::I32_GE_S),
            | FloatCC::Ult => (Routine::Getf2, op::I32_LT_S),
            | FloatCC::Ule => (Routine::Gttf2, op::I32_LE_S),
            | FloatCC::Ugt => (Routine::Letf2, op::I32_GT_S),
            | FloatCC::Uge => (Routine::Lttf2, op::I32_GE_S),
            | FloatCC::Uno => (Routine::Unordtf2, 0),
            | FloatCC::Ord => (Routine::Unordtf2, op::I32_EQZ),
            | FloatCC::One | FloatCC::Ueq => {
                // Ordered and unequal, or its negation.
                let args = [Arg::Value(a), Arg::Value(b)];
                self.call_routine_scalar(Routine::Unordtf2, &args);
                self.num(op::I32_EQZ);
                self.call_routine_scalar(Routine::Netf2, &args);
                self.i32_const(0);
                self.num(op::I32_NE);
                self.num(op::I32_AND);
                if cond == FloatCC::Ueq {
                    self.num(op::I32_EQZ);
                }
                return;
            },
        };
        self.call_routine_scalar(routine, &[Arg::Value(a), Arg::Value(b)]);
        match test {
            | 0 => {
                self.i32_const(0);
                self.num(op::I32_NE);
            },
            | op::I32_EQZ => self.num(op::I32_EQZ),
            | test => {
                self.i32_const(0);
                self.num(test);
            },
        }
    }
}

/// The `i64` comparison opcode for `cond`.
fn i64_compare(cond: IntCC) -> u8 {
    match cond {
        | IntCC::Eq => op::I64_EQ,
        | IntCC::Ne => op::I64_NE,
        | IntCC::Slt => op::I64_LT_S,
        | IntCC::Sle => op::I64_LE_S,
        | IntCC::Sgt => op::I64_GT_S,
        | IntCC::Sge => op::I64_GE_S,
        | IntCC::Ult => op::I64_LT_U,
        | IntCC::Ule => op::I64_LE_U,
        | IntCC::Ugt => op::I64_GT_U,
        | IntCC::Uge => op::I64_GE_U,
    }
}
