//! The frame on the shadow stack, and memory access: loads, stores,
//! copies, fills, atomic operations and dynamic allocation.
//!
//! WebAssembly runs one thread here (the threads proposal is not used), so
//! an atomic operation is its plain memory access: a read-modify-write is a
//! load, the operation and a store, and fences do nothing.

use super::{
    Codegen,
    calls::{
        registers_scalar,
        variadic_buffer,
        wide,
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
        u32_of,
    },
    ir::{
        Align,
        AtomicOp,
        Entity,
        InstData,
        Opcode,
        Type,
        Value,
        ValueDef,
    },
};

/// The function's frame: offsets from the frame pointer.
#[derive(Default)]
pub(super) struct Frame {
    /// The 16-byte area results returned in memory land in.
    pub(super) wide:     u32,
    /// The buffer of a variadic call's extra arguments.
    pub(super) varargs:  u32,
    /// The function's own return area for a result returned as a scalar.
    pub(super) area:     u32,
    pub(super) size:     u32,
    pub(super) align:    u32,
    /// Whether the function moves the stack pointer.
    pub(super) moves_sp: bool,
}

impl Codegen<'_, '_, '_, '_, '_> {
    /// Lays out the frame and allocates the frame pointer locals.
    pub(super) fn layout_frame(&mut self) -> Result<(), &'static str> {
        let mut offset = 0_u64;
        let mut align = 16_u64;
        let mut place = |size: u64, slot_align: u64, offset: &mut u64| {
            *offset = offset.next_multiple_of(slot_align);
            let at = *offset;
            *offset += size;
            align = align.max(slot_align);
            at
        };
        for (_, slot) in self.body.stack_slots() {
            let at = low32(u128::from(place(
                u64::from(slot.size),
                slot.align.bytes(),
                &mut offset,
            )));
            self.frame_slots.push(at);
        }
        let mut needs_wide = wide(self.signature.result);
        let mut buffer = 0_u64;
        let mut moves_sp = false;
        for block in self.body.blocks() {
            for &inst in self.body.block_insts(block) {
                if let Some(result) = self.body.inst_result(inst)
                    && Repr::of(self.ty(result)) == Some(Repr::Pair)
                {
                    needs_wide = true;
                }
                match *self.body.inst(inst) {
                    | InstData::Call { func, args } => {
                        let signature = self.module.function_signature(func);
                        buffer = buffer.max(variadic_buffer(
                            self,
                            signature,
                            self.body.value_list(args),
                        ));
                    },
                    | InstData::CallIndirect { sig, args } => {
                        let signature = self.module.signature(sig);
                        buffer = buffer.max(variadic_buffer(
                            self,
                            signature,
                            &self.body.value_list(args)[1..],
                        ));
                    },
                    | InstData::DynAlloca { .. }
                    | InstData::Nullary {
                        opcode: Opcode::StackSave | Opcode::FrameAddress,
                        ..
                    }
                    | InstData::Unary {
                        opcode: Opcode::StackRestore,
                        ..
                    } => moves_sp = true,
                    | _ => {},
                }
            }
        }
        let wide_at = if needs_wide {
            place(16, 16, &mut offset)
        } else {
            0
        };
        let varargs_at = if buffer > 0 {
            place(buffer, 16, &mut offset)
        } else {
            0
        };
        let area_at = match (
            registers_scalar(self.signature),
            self.signature.param_abi(0),
        ) {
            | (Some(_), Some(crate::ir::ParamAbi::Registers(registers))) =>
                place(registers.area_bytes(), registers.align.bytes(), &mut offset),
            | _ => 0,
        };
        let size = offset.next_multiple_of(align);
        let size = u32::try_from(size).map_err(|_| "a frame of 4 GiB or more")?;
        self.frame.wide = low32(u128::from(wide_at));
        self.frame.varargs = low32(u128::from(varargs_at));
        self.frame.area = low32(u128::from(area_at));
        self.frame.size = size;
        self.frame.align = low32(u128::from(align));
        self.frame.moves_sp = moves_sp || size > 0;
        if self.frame.moves_sp {
            self.sp = self.local(ValType::I32, "sp");
            self.fp = self.local(ValType::I32, "fp");
        }
        Ok(())
    }

    /// Moves the stack pointer down by the frame, and fills the parameters
    /// that the caller does not pass.
    pub(super) fn prologue(&mut self) {
        if self.frame.moves_sp {
            self.emit(Instr::GlobalGet(0));
            self.emit(Instr::LocalTee(self.sp));
            if self.frame.size > 0 {
                self.i32_const(self.frame.size as i32);
                self.num(op::I32_SUB);
                if self.frame.align > 16 {
                    self.i32_const(-(self.frame.align as i32));
                    self.num(op::I32_AND);
                }
            }
            self.emit(Instr::LocalTee(self.fp));
            self.emit(Instr::GlobalSet(0));
        }
        let entry = self
            .body
            .entry_block()
            .expect("checked when locals were assigned");
        let params = self.body.block_params(entry);
        if registers_scalar(self.signature).is_some() {
            self.emit(Instr::LocalGet(self.fp));
            self.i32_const(self.frame.area as i32);
            self.num(op::I32_ADD);
            self.set(params[0]);
        }
        // A narrow parameter's upper bits are the caller's extension.
        for &param in params {
            if narrow(self.ty(param)).is_some() {
                self.push(param);
                self.mask(self.ty(param));
                self.set(param);
            }
        }
    }

    /// Restores the stack pointer before a return.
    pub(super) fn epilogue(&mut self) {
        if self.frame.moves_sp {
            self.emit(Instr::LocalGet(self.sp));
            self.emit(Instr::GlobalSet(0));
        }
    }

    /// Pushes the address `value` designates and returns the constant
    /// offset to add to it, folding a constant `ptr_add` and a stack slot's
    /// offset into the instruction.
    fn address(&mut self, value: Value) -> u32 {
        if let ValueDef::Result(inst) = self.body.value_def(value) {
            match *self.body.inst(inst) {
                | InstData::StackAddr { slot } => {
                    self.emit(Instr::LocalGet(self.fp));
                    return self.frame_slots[slot.index()];
                },
                | InstData::Binary {
                    opcode: Opcode::PtrAdd,
                    args: [base, offset],
                    ..
                } if let Some(bits) = self.constant(offset)
                    && bits < 1 << 30 =>
                {
                    let inner = self.address(base);
                    if let Some(total) = inner.checked_add(low32(bits)) {
                        return total;
                    }
                    self.i32_const(inner as i32);
                    self.num(op::I32_ADD);
                    return low32(bits);
                },
                | _ => {},
            }
        }
        self.push(value);
        0
    }

    /// A load of `ty` from `addr` into `result`.
    pub(super) fn load(&mut self, ty: Type, align: Align, addr: Value, result: Option<Value>) {
        let offset = self.address(addr);
        let repr = self.repr(ty);
        if repr == Repr::Pair {
            let Some(result) = result else {
                self.emit(Instr::Drop);
                return;
            };
            let base = self.temp(ValType::I32, 0);
            self.emit(Instr::LocalTee(base));
            self.memory(op::I64_LOAD, align, 8, offset);
            self.set_part(result, false);
            self.emit(Instr::LocalGet(base));
            self.memory(op::I64_LOAD, align, 8, offset.saturating_add(8));
            self.set_part(result, true);
            return;
        }
        let (opcode, natural) = load_opcode(ty);
        self.memory(opcode, align, natural, offset);
        self.store_result(result);
    }

    /// A store of `value` of type `ty` to `addr`.
    pub(super) fn store(&mut self, ty: Type, align: Align, value: Value, addr: Value) {
        let offset = self.address(addr);
        if self.repr(ty) == Repr::Pair {
            let base = self.temp(ValType::I32, 0);
            self.emit(Instr::LocalTee(base));
            self.push_part(value, false);
            self.memory(op::I64_STORE, align, 8, offset);
            self.emit(Instr::LocalGet(base));
            self.push_part(value, true);
            self.memory(op::I64_STORE, align, 8, offset.saturating_add(8));
            return;
        }
        self.push(value);
        let (opcode, natural) = store_opcode(ty);
        self.memory(opcode, align, natural, offset);
    }

    /// A load or store instruction with the alignment hint `align`, at most
    /// the access's `natural` size.
    fn memory(&mut self, opcode: u8, align: Align, natural: u64, offset: u32) {
        let hint = super::calls::log2(align.bytes().min(natural));
        self.emit(Instr::Memory {
            opcode,
            align: hint,
            offset,
        });
    }

    /// `copy` (`memory.copy`, which allows overlap) or `fill`
    /// (`memory.fill`).
    pub(super) fn memory_range(&mut self, opcode: Opcode, [dst, src, size]: [Value; 3]) {
        self.push(dst);
        self.push(src);
        self.push_wrapped(size);
        self.emit(if opcode == Opcode::Copy {
            Instr::MemoryCopy
        } else {
            Instr::MemoryFill
        });
    }

    /// `dyn_alloca`: the stack pointer moves down by `size`, aligned to at
    /// least the stack's sixteen bytes.
    pub(super) fn dyn_alloca(&mut self, size: Value, align: Align, result: Option<Value>) {
        let align = align.bytes().max(16);
        self.emit(Instr::GlobalGet(0));
        self.push_wrapped(size);
        self.num(op::I32_SUB);
        self.i32_const(low32(u128::from(align)).wrapping_neg().cast_signed());
        self.num(op::I32_AND);
        let address = self.temp(ValType::I32, 0);
        self.emit(Instr::LocalTee(address));
        self.emit(Instr::GlobalSet(0));
        self.emit(Instr::LocalGet(address));
        self.store_result(result);
    }

    /// An atomic operation as plain memory accesses (one thread).
    pub(super) fn atomic(
        &mut self,
        op_kind: AtomicOp,
        ty: Type,
        align: Align,
        args: &[Value],
        result: Option<Value>,
    ) {
        match op_kind {
            | AtomicOp::ThreadFence | AtomicOp::SignalFence => {},
            | AtomicOp::Load => self.load(ty, align, args[0], result),
            | AtomicOp::Store => self.store(ty, align, args[1], args[0]),
            | AtomicOp::LoadBytes | AtomicOp::StoreBytes | AtomicOp::ExchangeBytes => {
                let size = *args.last().expect("a buffer operation has a size");
                let object = args[0];
                let (copies, count): ([(Value, Value); 2], usize) = match op_kind {
                    | AtomicOp::LoadBytes => ([(args[1], object), (object, object)], 1),
                    | AtomicOp::StoreBytes => ([(object, args[1]), (object, object)], 1),
                    | _ => ([(args[2], object), (object, args[1])], 2),
                };
                for &(dst, src) in &copies[..count] {
                    self.push(dst);
                    self.push(src);
                    self.push_wrapped(size);
                    self.emit(Instr::MemoryCopy);
                }
            },
            | AtomicOp::CompareExchangeBytes =>
                self.refuse("an atomic compare-exchange of an object without a lock-free size"),
            | _ => self.read_modify_write(op_kind, ty, align, args, result),
        }
    }

    /// Exchange, compare-exchange and the arithmetic read-modify-writes:
    /// the old value is the result, and the new one is stored.
    fn read_modify_write(
        &mut self,
        op_kind: AtomicOp,
        ty: Type,
        align: Align,
        args: &[Value],
        result: Option<Value>,
    ) {
        let repr = self.repr(ty);
        // The old value, in a scratch local or pair of locals.
        let parts = if repr == Repr::Pair { 2 } else { 1 };
        let val_type = match repr {
            | Repr::I32 => ValType::I32,
            | Repr::F32 => ValType::F32,
            | Repr::F64 => ValType::F64,
            | _ => ValType::I64,
        };
        let old = [self.temp(val_type, 1), self.temp(val_type, 2)];
        let address = self.temp(ValType::I32, 0);
        let offset = self.address(args[0]);
        self.emit(Instr::LocalSet(address));
        let (load, natural) = load_opcode(ty);
        let (store, _) = store_opcode(ty);
        for (part, &local) in old.iter().take(parts).enumerate() {
            self.emit(Instr::LocalGet(address));
            if parts == 2 {
                self.memory(op::I64_LOAD, align, 8, offset + 8 * u32_of(part));
            } else {
                self.memory(load, align, natural, offset);
            }
            self.emit(Instr::LocalSet(local));
        }
        let store_new = |this: &mut Self, compute: &dyn Fn(&mut Self, usize)| {
            for part in 0..parts {
                this.emit(Instr::LocalGet(address));
                compute(this, part);
                if parts == 2 {
                    this.memory(op::I64_STORE, align, 8, offset + 8 * u32_of(part));
                } else {
                    this.memory(store, align, natural, offset);
                }
            }
        };
        let value = args.get(1).copied();
        match op_kind {
            | AtomicOp::Exchange => {
                let value = value.expect("an exchange has a value");
                store_new(self, &|this, part| this.push_piece(value, part, parts));
            },
            | AtomicOp::CompareExchange => {
                // Operands: address, new value, expected value.
                let (new, expected) = (args[1], args[2]);
                self.emit(Instr::LocalGet(old[0]));
                self.push_piece(expected, 0, parts);
                self.num(eq_opcode(repr));
                if parts == 2 {
                    self.emit(Instr::LocalGet(old[1]));
                    self.push_piece(expected, 1, parts);
                    self.num(op::I64_EQ);
                    self.num(op::I32_AND);
                }
                self.emit(Instr::If);
                store_new(self, &|this, part| this.push_piece(new, part, parts));
                self.emit(Instr::End);
            },
            | _ => {
                let value = value.expect("a read-modify-write has a value");
                if parts == 2 {
                    self.refuse("a 128-bit atomic read-modify-write");
                    return;
                }
                let local = old[0];
                store_new(self, &|this, _| {
                    this.rmw_operation(op_kind, ty, repr, local, value);
                });
            },
        }
        if let Some(result) = result {
            for &local in old.iter().take(parts) {
                self.emit(Instr::LocalGet(local));
            }
            self.set(result);
        }
    }

    /// Pushes part `part` of `value`: the value itself, or a pair's half.
    fn push_piece(&mut self, value: Value, part: usize, parts: usize) {
        if parts == 2 {
            self.push_part(value, part == 1);
        } else {
            self.push(value);
        }
    }

    /// Pushes `old op value` for an atomic read-modify-write.
    fn rmw_operation(&mut self, op_kind: AtomicOp, ty: Type, repr: Repr, old: u32, value: Value) {
        let wide = repr == Repr::I64;
        let pick = |narrow_op: u8, wide_op: u8| if wide { wide_op } else { narrow_op };
        match op_kind {
            | AtomicOp::Add | AtomicOp::Sub | AtomicOp::And | AtomicOp::Or | AtomicOp::Xor => {
                self.emit(Instr::LocalGet(old));
                self.push(value);
                self.num(match op_kind {
                    | AtomicOp::Add => pick(op::I32_ADD, op::I64_ADD),
                    | AtomicOp::Sub => pick(op::I32_SUB, op::I64_SUB),
                    | AtomicOp::And => pick(op::I32_AND, op::I64_AND),
                    | AtomicOp::Or => pick(op::I32_OR, op::I64_OR),
                    | _ => pick(op::I32_XOR, op::I64_XOR),
                });
            },
            | AtomicOp::Nand => {
                self.emit(Instr::LocalGet(old));
                self.push(value);
                self.num(pick(op::I32_AND, op::I64_AND));
                if wide {
                    self.i64_const(-1);
                    self.num(op::I64_XOR);
                } else {
                    self.i32_const(-1);
                    self.num(op::I32_XOR);
                }
            },
            | AtomicOp::Min | AtomicOp::Max | AtomicOp::Umin | AtomicOp::Umax => {
                let signed = matches!(op_kind, AtomicOp::Min | AtomicOp::Max);
                let less = matches!(op_kind, AtomicOp::Min | AtomicOp::Umin);
                // select(old, value, old <cmp> value)
                self.emit(Instr::LocalGet(old));
                self.push(value);
                self.emit(Instr::LocalGet(old));
                if signed {
                    self.sign_extend(ty);
                }
                self.push(value);
                if signed {
                    self.sign_extend(ty);
                }
                self.num(match (wide, signed, less) {
                    | (false, true, true) => op::I32_LT_S,
                    | (false, true, false) => op::I32_GT_S,
                    | (false, false, true) => op::I32_LT_U,
                    | (false, false, false) => op::I32_GT_U,
                    | (true, true, true) => op::I64_LT_S,
                    | (true, true, false) => op::I64_GT_S,
                    | (true, false, true) => op::I64_LT_U,
                    | (true, false, false) => op::I64_GT_U,
                });
                self.emit(Instr::Select);
            },
            | AtomicOp::Fadd | AtomicOp::Fsub | AtomicOp::Fmin | AtomicOp::Fmax => {
                let f64 = repr == Repr::F64;
                self.emit(Instr::LocalGet(old));
                self.push(value);
                self.num(match (op_kind, f64) {
                    | (AtomicOp::Fadd, false) => op::F32_ADD,
                    | (AtomicOp::Fsub, false) => op::F32_SUB,
                    | (AtomicOp::Fmin, false) => op::F32_MIN,
                    | (AtomicOp::Fmax, false) => op::F32_MAX,
                    | (AtomicOp::Fadd, true) => op::F64_ADD,
                    | (AtomicOp::Fsub, true) => op::F64_SUB,
                    | (AtomicOp::Fmin, true) => op::F64_MIN,
                    | _ => op::F64_MAX,
                });
            },
            | _ => {
                self.refuse("an atomic operation");
                self.emit(Instr::LocalGet(old));
            },
        }
    }
}

/// The equality opcode of a representation.
fn eq_opcode(repr: Repr) -> u8 {
    match repr {
        | Repr::I32 => op::I32_EQ,
        | Repr::F32 => op::F32_EQ,
        | Repr::F64 => op::F64_EQ,
        | _ => op::I64_EQ,
    }
}

/// The load of `ty`, zero-extending narrow integers, and its size.
pub(super) fn load_opcode(ty: Type) -> (u8, u64) {
    match ty {
        | Type::I1 | Type::I8 => (op::I32_LOAD8_U, 1),
        | Type::I16 | Type::F16 => (op::I32_LOAD16_U, 2),
        | Type::I64 => (op::I64_LOAD, 8),
        | Type::F32 => (op::F32_LOAD, 4),
        | Type::F64 => (op::F64_LOAD, 8),
        | _ => (op::I32_LOAD, 4),
    }
}

/// The store of `ty` and its size.
pub(super) fn store_opcode(ty: Type) -> (u8, u64) {
    match ty {
        | Type::I1 | Type::I8 => (op::I32_STORE8, 1),
        | Type::I16 | Type::F16 => (op::I32_STORE16, 2),
        | Type::I64 => (op::I64_STORE, 8),
        | Type::F32 => (op::F32_STORE, 4),
        | Type::F64 => (op::F64_STORE, 8),
        | _ => (op::I32_STORE, 4),
    }
}
