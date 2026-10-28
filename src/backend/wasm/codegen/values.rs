//! Locals for SSA values, and pushing and storing those values.

use core::fmt::Write as _;

use super::{
    Codegen,
    calls::{
        registers_scalar,
        wide,
    },
};
use crate::{
    backend::wasm::{
        instr::{
            Instr,
            ValType,
        },
        low32,
        low64,
        u32_of,
    },
    ir::{
        Entity,
        InstData,
        Opcode,
        Type,
        Value,
        ValueDef,
    },
    util::bump::ArenaString,
};

/// How a value of an IR type is held: one local, or a pair of `i64`
/// locals (low half first) for `i128` and `f128`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Repr {
    I32,
    I64,
    F32,
    F64,
    Pair,
}

impl Repr {
    /// `None` for the x87 format, which WebAssembly has no use for.
    pub(crate) const fn of(ty: Type) -> Option<Self> {
        Some(match ty {
            | Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::Ptr | Type::F16 => Self::I32,
            | Type::I64 => Self::I64,
            | Type::F32 => Self::F32,
            | Type::F64 => Self::F64,
            | Type::I128 | Type::F128 => Self::Pair,
            | Type::F80 => return None,
        })
    }

    /// The value types of the locals or parameters.
    pub(crate) const fn types(self) -> &'static [ValType] {
        match self {
            | Self::I32 => &[ValType::I32],
            | Self::I64 => &[ValType::I64],
            | Self::F32 => &[ValType::F32],
            | Self::F64 => &[ValType::F64],
            | Self::Pair => &[ValType::I64, ValType::I64],
        }
    }
}

/// The width of a value held zero-extended in an `i32`, if it is narrower.
pub(super) const fn narrow(ty: Type) -> Option<u32> {
    match ty {
        | Type::I1 => Some(1),
        | Type::I8 => Some(8),
        | Type::I16 => Some(16),
        | _ => None,
    }
}

impl Codegen<'_, '_, '_, '_, '_> {
    /// Gives the parameters their locals in the signature's order, then
    /// every other value that needs one.
    pub(super) fn assign_locals(&mut self) -> Result<(), &'static str> {
        let count = self.body.value_count();
        self.value_locals.resize(count, u32::MAX);
        let entry = self.body.entry_block().ok_or("a function without blocks")?;
        let params = self.body.block_params(entry);
        if wide(self.signature.result) {
            self.result_pointer = Some(self.local(ValType::I32, "result.ptr"));
        }
        let skip_area = registers_scalar(self.signature).is_some();
        for (index, &value) in params.iter().enumerate() {
            if index == 0 && skip_area {
                continue;
            }
            self.value_local(value);
        }
        if self.signature.variadic {
            self.varargs = Some(self.local(ValType::I32, "varargs"));
        }
        if skip_area {
            self.value_local(params[0]);
        }
        for block in self.body.blocks() {
            if block != entry {
                for &param in self.body.block_params(block) {
                    self.value_local(param);
                }
            }
            for &inst in self.body.block_insts(block) {
                if let Some(result) = self.body.inst_result(inst)
                    && !self.recomputed(result)
                {
                    self.value_local(result);
                }
            }
        }
        Ok(())
    }

    /// Allocates `value`'s local or locals.
    fn value_local(&mut self, value: Value) {
        let ty = self.body.value_type(value);
        let repr = self.repr(ty);
        let first = u32_of(self.locals.len());
        let mut name = ArenaString::new_in(self.arena);
        for (part, &local) in repr.types().iter().enumerate() {
            name.clear();
            _ = write!(name, "{value}");
            if repr == Repr::Pair {
                name.push_str(if part == 0 { ".lo" } else { ".hi" });
            }
            _ = self.local(local, &name);
        }
        self.value_locals[value.index()] = first;
    }

    /// Whether `value` is recomputed at each use instead of living in a
    /// local: a constant, or the address of a global, function, slot or
    /// label.
    pub(super) fn recomputed(&self, value: Value) -> bool {
        let ValueDef::Result(inst) = self.body.value_def(value) else {
            return false;
        };
        matches!(
            self.body.inst(inst),
            InstData::Const { .. }
                | InstData::WideConst { .. }
                | InstData::Nullary {
                    opcode: Opcode::Poison | Opcode::Null,
                    ..
                }
                | InstData::GlobalAddr { .. }
                | InstData::FuncAddr { .. }
                | InstData::StackAddr { .. }
                | InstData::BlockAddr { .. }
        )
    }

    /// The value's type.
    pub(super) fn ty(&self, value: Value) -> Type {
        self.body.value_type(value)
    }

    /// Pushes `value`; a pair pushes its low half, then its high half.
    pub(super) fn push(&mut self, value: Value) {
        if self.ty(value).bits() == 128 && Repr::of(self.ty(value)) == Some(Repr::Pair) {
            self.push_part(value, false);
            self.push_part(value, true);
            return;
        }
        if self.recomputed(value) {
            self.rematerialize(value, false);
        } else {
            let local = self.value_locals[value.index()];
            self.emit(Instr::LocalGet(local));
        }
    }

    /// Pushes the low (`high` false) or high half of a pair.
    pub(super) fn push_part(&mut self, value: Value, high: bool) {
        if self.recomputed(value) {
            self.rematerialize(value, high);
        } else {
            let local = self.value_locals[value.index()] + u32::from(high);
            self.emit(Instr::LocalGet(local));
        }
    }

    /// Pops into `value`'s locals: a pair's high half is on top.
    pub(super) fn set(&mut self, value: Value) {
        let local = self.value_locals[value.index()];
        if local == u32::MAX {
            // A recomputed value's definition is not emitted.
            self.emit(Instr::Drop);
            return;
        }
        if Repr::of(self.ty(value)) == Some(Repr::Pair) {
            self.emit(Instr::LocalSet(local + 1));
        }
        self.emit(Instr::LocalSet(local));
    }

    /// Pops one half of a pair into `value`'s locals.
    pub(super) fn set_part(&mut self, value: Value, high: bool) {
        let local = self.value_locals[value.index()] + u32::from(high);
        self.emit(Instr::LocalSet(local));
    }

    /// The constant bits of `value`, if it is a constant.
    pub(super) fn constant(&self, value: Value) -> Option<u128> {
        let ValueDef::Result(inst) = self.body.value_def(value) else {
            return None;
        };
        match *self.body.inst(inst) {
            | InstData::Const { ty, bits, .. } => Some(u128::from(bits.get()) & ty.mask()),
            | InstData::WideConst { constant, .. } => Some(self.body.constant(constant)),
            | InstData::Nullary {
                opcode: Opcode::Null,
                ..
            } => Some(0),
            | _ => None,
        }
    }

    /// Emits a recomputed value, or one half of a pair one.
    fn rematerialize(&mut self, value: Value, high: bool) {
        let ValueDef::Result(inst) = self.body.value_def(value) else {
            unreachable!("only results are recomputed");
        };
        let ty = self.ty(value);
        match *self.body.inst(inst) {
            | InstData::Const { .. } | InstData::WideConst { .. } => {
                let bits = self.constant(value).unwrap_or(0);
                self.constant_of(ty, bits, high);
            },
            | InstData::Nullary { .. } => self.constant_of(ty, 0, high),
            | InstData::GlobalAddr { global } => {
                let address = self.link.global_address(self.unit, global);
                self.i32_const(address as i32);
            },
            | InstData::FuncAddr { func } => {
                let index = self.link.function_address(self.unit, func);
                self.i32_const(index as i32);
            },
            | InstData::StackAddr { slot } => {
                let offset = self.frame_slots[slot.index()];
                self.emit(Instr::LocalGet(self.fp));
                if offset != 0 {
                    self.i32_const(offset as i32);
                    self.num(crate::backend::wasm::instr::op::I32_ADD);
                }
            },
            | InstData::BlockAddr { label } => self.i32_const(label.as_u32() as i32 + 1),
            | _ => unreachable!("only constants and addresses are recomputed"),
        }
    }

    /// Pushes the constant `bits` of type `ty`, or one half of a pair one.
    pub(super) fn constant_of(&mut self, ty: Type, bits: u128, high: bool) {
        match Repr::of(ty) {
            | Some(Repr::I32) => self.i32_const(low32(bits & ty.mask()).cast_signed()),
            | Some(Repr::I64) => self.i64_const(low64(bits) as i64),
            | Some(Repr::F32) => self.emit(Instr::F32Const(low32(bits))),
            | Some(Repr::F64) => self.emit(Instr::F64Const(low64(bits))),
            | Some(Repr::Pair) => {
                let half = if high { bits >> 64 } else { bits };
                self.i64_const(low64(half) as i64);
            },
            | None => self.i32_const(0),
        }
    }

    /// Pushes a zero of type `ty`, both halves for a pair.
    pub(super) fn zero(&mut self, ty: Type) {
        self.constant_of(ty, 0, false);
        if Repr::of(ty) == Some(Repr::Pair) {
            self.constant_of(ty, 0, true);
        }
    }
}
