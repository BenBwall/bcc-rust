//! Addresses, loads, stores, `copy` and `fill`.
//!
//! An address operand folds a frame slot, a symbol the module defines (any
//! symbol on COFF but a weak declaration), or a register plus a constant
//! `ptr_add` offset into one memory operand. On ELF a symbol the module does
//! not define is reached through its GOT entry, since executables are
//! position-independent; on COFF a weak declaration, whose address may be
//! null, through a `.refptr` stub holding its address, as Clang does.
//!
//! A thread-local global's address is computed where `global_addr` names
//! it, with the sequences Clang emits. ELF uses the thread pointer at
//! `%fs:0` and the local-exec model (`sym@TPOFF`) for globals the module
//! defines, the initial-exec model (`sym@GOTTPOFF`) for the rest (Drepper,
//! "ELF Handling For Thread-Local Storage", §4.3.6 and §4.3.5). Windows
//! indexes the TEB's `ThreadLocalStoragePointer` (`%gs:0x58`) with the
//! image's `_tls_index` and adds the symbol's offset in the `.tls` section
//! (`sym@SECREL32`; Microsoft Learn, "PE Format", "The .tls Section").
//!
//! `copy` and `fill` of at most [`INLINE_BYTES`] constant bytes become
//! loads and stores; a `copy` loads every piece before storing any, which is
//! correct for any overlap. Longer non-overlapping copies use `rep movsb`,
//! fills `rep stosb`, and possibly overlapping copies call `memmove`. The
//! direction flag is clear at every call boundary (psABI §3.2.1; Microsoft
//! "x64 calling convention", "Caller/callee saved registers"), so the string
//! instructions copy forwards.

use super::{
    AddrBase,
    Selector,
    ValueKind,
    calls::Passed,
    width,
};
use crate::{
    backend::native::{
        defines,
        mir::{
            AluOp,
            Base,
            Callee,
            FrameObject,
            MInst,
            Mem,
            Operand,
            PReg,
            Reg,
            ShiftOp,
            SymbolRef,
            Width,
        },
        support::Construct,
        through_stub,
    },
    ir::{
        GlobalId,
        Inst,
        InstData,
        InstFlags,
        Opcode,
        Symbol,
        Type,
        Value,
        ValueDef,
    },
    target::Target,
};

/// The Windows image's TLS index, which the CRT's TLS directory names.
pub(in crate::backend::native) const TLS_INDEX: &str = "_tls_index";

/// The longest constant `copy` or `fill` expanded into moves.
pub(super) const INLINE_BYTES: u64 = 64;

/// The largest displacement folded into a frame or symbol operand, which
/// keeps the final offset inside a signed 32-bit displacement.
const MAX_DISP: i64 = 1 << 30;

impl Selector<'_, '_, '_> {
    /// `dst ←` the evaluating thread's copy of the thread-local `global`.
    pub(super) fn thread_local_address(&mut self, dst: Reg, global: GlobalId) {
        let symbol = SymbolRef::Ir(Symbol::Global(global));
        if matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc) {
            let index = self.vreg();
            self.emit(MInst::Mov {
                width: Width::B32,
                dst:   index,
                src:   Operand::Mem(Mem::new(Base::Symbol(SymbolRef::Runtime(TLS_INDEX)), 0)),
            });
            let slots = self.vreg();
            self.emit(MInst::ThreadPointer { dst: slots });
            self.shift_imm(ShiftOp::Shl, Width::B64, index, 3);
            self.emit(MInst::Alu {
                op:    AluOp::Add,
                width: Width::B64,
                dst:   slots,
                src:   Operand::Reg(index),
            });
            let block = self.vreg();
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   block,
                src:   Operand::Mem(Mem::new(Base::Reg(slots), 0)),
            });
            self.emit(MInst::TlsLea {
                dst,
                base: block,
                symbol,
            });
        } else if defines(self.module, Symbol::Global(global)) {
            let pointer = self.vreg();
            self.emit(MInst::ThreadPointer { dst: pointer });
            self.emit(MInst::TlsLea {
                dst,
                base: pointer,
                symbol,
            });
        } else {
            self.emit(MInst::ThreadPointer { dst });
            self.emit(MInst::Alu {
                op: AluOp::Add,
                width: Width::B64,
                dst,
                src: Operand::Mem(Mem::new(Base::GotTpoff(symbol), 0)),
            });
        }
    }

    /// Whether `symbol` can be addressed RIP-relative: on ELF only if the
    /// module defines it, on COFF unless its address is loaded from a
    /// `.refptr` stub ([`through_stub`]).
    pub(in crate::backend::native) fn direct(&self, symbol: Symbol) -> bool {
        if matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc) {
            return !through_stub(self.module, self.target, symbol);
        }
        defines(self.module, symbol)
    }

    /// `dst ← base + disp` for a constant address.
    pub(super) fn address_into(&mut self, dst: Reg, base: AddrBase, disp: i64) {
        let folded = if disp.abs() < MAX_DISP {
            i32::try_from(disp).expect("the displacement was bounded")
        } else {
            0
        };
        match base {
            | AddrBase::Slot(slot) => self.emit(MInst::Lea {
                dst,
                addr: Mem::new(Base::Frame(FrameObject::Slot(slot)), folded),
            }),
            | AddrBase::Symbol(symbol) if self.direct(symbol) => self.emit(MInst::Lea {
                dst,
                addr: Mem::new(Base::Symbol(SymbolRef::Ir(symbol)), folded),
            }),
            | AddrBase::Symbol(symbol) => {
                self.func.indirect_symbols.push(symbol);
                self.emit(MInst::Mov {
                    width: Width::B64,
                    dst,
                    src: Operand::Mem(Mem::new(Base::Got(SymbolRef::Ir(symbol)), 0)),
                });
                if folded != 0 {
                    self.emit(MInst::Lea {
                        dst,
                        addr: Mem::new(Base::Reg(dst), folded),
                    });
                }
            },
        }
        let rest = disp - i64::from(folded);
        if rest != 0 {
            let offset = self.vreg();
            self.constant(offset, rest.cast_unsigned(), Type::I64);
            self.emit(MInst::Alu {
                op: AluOp::Add,
                width: Width::B64,
                dst,
                src: Operand::Reg(offset),
            });
        }
    }

    /// The memory operand addressing `value`.
    pub(super) fn address(&mut self, value: Value) -> Mem {
        match self.kind(value) {
            | ValueKind::Addr(AddrBase::Slot(slot), disp) if disp.abs() < MAX_DISP =>
                return Mem::new(
                    Base::Frame(FrameObject::Slot(slot)),
                    i32::try_from(disp).expect("the displacement was bounded"),
                ),
            | ValueKind::Addr(AddrBase::Symbol(symbol), disp)
                if disp.abs() < MAX_DISP && self.direct(symbol) =>
                return Mem::new(
                    Base::Symbol(SymbolRef::Ir(symbol)),
                    i32::try_from(disp).expect("the displacement was bounded"),
                ),
            | ValueKind::Reg(_) =>
                if let Some(mem) = self.offset_address(value) {
                    return mem;
                },
            | _ => {},
        }
        Mem::new(Base::Reg(self.reg(value)), 0)
    }

    /// `base + offset` for a value defined by a `ptr_add` of a register and
    /// a constant that fits a displacement.
    fn offset_address(&self, value: Value) -> Option<Mem> {
        let ValueDef::Result(inst) = self.body.value_def(value) else {
            return None;
        };
        let InstData::Binary {
            opcode: Opcode::PtrAdd,
            args: [base, offset],
            ..
        } = *self.body.inst(inst)
        else {
            return None;
        };
        match (self.kind(base), self.kind(offset)) {
            | (ValueKind::Reg(base), ValueKind::Const(offset))
                if offset.cast_signed().abs() < MAX_DISP =>
                Some(Mem::new(
                    Base::Reg(Reg::Virt(base)),
                    i32::try_from(offset.cast_signed()).expect("the offset was bounded"),
                )),
            | _ => None,
        }
    }

    /// Loads, stores, `copy` and `fill`.
    pub(super) fn memory(&mut self, inst: Inst, data: InstData) -> Result<(), Construct> {
        match data {
            | InstData::Load { ty, addr, .. } if ty != Type::I128 => {
                let Some(ValueKind::Reg(dst)) =
                    self.body.inst_result(inst).map(|value| self.kind(value))
                else {
                    unreachable!("a load has a register result");
                };
                let mem = self.address(addr);
                self.load(ty, Reg::Virt(dst), mem);
            },
            | InstData::Load {
                ty: Type::I128,
                addr,
                ..
            } => {
                let dst = self.result_pair(inst);
                let mem = self.address(addr);
                self.wide_load(dst, mem);
            },
            | InstData::Store {
                ty: Type::I128,
                args: [value, addr],
                ..
            } => {
                let mem = self.address(addr);
                self.wide_store(value, mem);
            },
            | InstData::Store {
                ty,
                args: [value, addr],
                ..
            } if ty.is_float() => {
                let src = self.reg(value);
                let dst = self.address(addr);
                self.emit(MInst::XStore {
                    width: width(ty),
                    dst,
                    src,
                });
            },
            | InstData::Store {
                ty,
                args: [value, addr],
                ..
            } => {
                let width = width(ty);
                let src = self.operand(value, width);
                let dst = self.address(addr);
                self.emit(MInst::Store { width, dst, src });
            },
            | InstData::MemoryRange {
                opcode,
                flags,
                args: [dst, src, size],
                ..
            } => match opcode {
                | Opcode::Copy => self.copy_range(dst, src, size, flags),
                | Opcode::Fill => self.fill_range(dst, src, size),
                | _ => return Err(Construct::Opcode(opcode)),
            },
            | _ => return Err(Construct::Opcode(data.opcode())),
        }
        Ok(())
    }

    /// `dst ← [mem]` of type `ty`; narrow loads zero-extend to 32 bits.
    pub(super) fn load(&mut self, ty: Type, dst: Reg, mem: Mem) {
        let width = width(ty);
        if ty.is_float() {
            self.emit(MInst::XMov {
                width,
                dst,
                src: Operand::Mem(mem),
            });
            return;
        }
        match width {
            | Width::B8 | Width::B16 => self.emit(MInst::Extend {
                signed: false,
                from: width,
                to: Width::B32,
                dst,
                src: Operand::Mem(mem),
            }),
            | Width::B32 | Width::B64 => self.emit(MInst::Mov {
                width,
                dst,
                src: Operand::Mem(mem),
            }),
        }
    }

    /// `copy dst, src, size`.
    fn copy_range(&mut self, dst: Value, src: Value, size: Value, flags: InstFlags) {
        let overlap = flags.contains(InstFlags::MAY_OVERLAP);
        match self.kind(size) {
            | ValueKind::Const(bytes) if bytes <= INLINE_BYTES => {
                let dst = self.address(dst);
                let src = self.address(src);
                self.copy_bytes(dst, src, bytes);
            },
            | _ if overlap => {
                let args =
                    [dst, src, size].map(|value| Passed::Scalar(self.operand(value, Width::B64)));
                self.call_sequence(Callee::Symbol(SymbolRef::Runtime("memmove")), &args, false);
            },
            | _ => {
                let dst = self.reg(dst);
                let src = self.reg(src);
                let size = self.reg(size);
                self.string_copy(Operand::Reg(dst), Operand::Reg(src), Operand::Reg(size));
            },
        }
    }

    /// Copies `bytes` constant bytes: pieces of at most 8 bytes loaded
    /// first and stored after, so any overlap is copied correctly; longer
    /// copies (the selector makes them only for disjoint `byval` areas) use
    /// `rep movsb`.
    pub(super) fn copy_bytes(&mut self, dst: Mem, src: Mem, bytes: u64) {
        if bytes > INLINE_BYTES {
            let dst = self.lea(dst);
            let src = self.lea(src);
            self.string_copy(
                Operand::Reg(dst),
                Operand::Reg(src),
                Operand::Imm(
                    i32::try_from(bytes).expect("the support check bounds argument copies"),
                ),
            );
            return;
        }
        let mut pieces = [(0_i32, Width::B8, Reg::Virt(crate::ir::Entity::new(0))); 16];
        let mut count = 0;
        let mut offset = 0;
        for width in [Width::B64, Width::B32, Width::B16, Width::B8] {
            let size = u64::from(width.bits() / 8);
            while bytes - offset >= size {
                let disp = i32::try_from(offset).expect("an inline copy is short");
                let temp = self.vreg();
                self.load(load_type(width), temp, shifted(src, disp));
                pieces[count] = (disp, width, temp);
                count += 1;
                offset += size;
            }
        }
        for &(disp, width, temp) in &pieces[..count] {
            self.emit(MInst::Store {
                width,
                dst: shifted(dst, disp),
                src: Operand::Reg(temp),
            });
        }
    }

    /// `rep movsb` of `size` bytes from `src` to `dst`.
    fn string_copy(&mut self, dst: Operand, src: Operand, size: Operand) {
        for (register, value) in [(PReg::Rdi, dst), (PReg::Rsi, src), (PReg::Rcx, size)] {
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(register),
                src:   value,
            });
        }
        self.emit(MInst::RepMovsb);
    }

    /// `fill dst, byte, size`.
    fn fill_range(&mut self, dst: Value, byte: Value, size: Value) {
        if let ValueKind::Const(bytes) = self.kind(size)
            && bytes <= INLINE_BYTES
        {
            let dst = self.address(dst);
            let pattern = self.vreg();
            match self.kind(byte) {
                | ValueKind::Const(bits) =>
                    self.constant(pattern, (bits & 0xFF) * 0x0101_0101_0101_0101, Type::I64),
                | _ => {
                    let byte = self.reg(byte);
                    self.emit(MInst::Extend {
                        signed: false,
                        from:   Width::B8,
                        to:     Width::B32,
                        dst:    pattern,
                        src:    Operand::Reg(byte),
                    });
                    let ones = self.vreg();
                    self.constant(ones, 0x0101_0101_0101_0101, Type::I64);
                    self.emit(MInst::Alu {
                        op:    AluOp::Imul,
                        width: Width::B64,
                        dst:   pattern,
                        src:   Operand::Reg(ones),
                    });
                },
            }
            let mut offset = 0;
            for width in [Width::B64, Width::B32, Width::B16, Width::B8] {
                let size = u64::from(width.bits() / 8);
                while bytes - offset >= size {
                    let disp = i32::try_from(offset).expect("an inline fill is short");
                    self.emit(MInst::Store {
                        width,
                        dst: shifted(dst, disp),
                        src: Operand::Reg(pattern),
                    });
                    offset += size;
                }
            }
            return;
        }
        let dst = self.reg(dst);
        let byte = self.reg(byte);
        let size = self.reg(size);
        for (register, value) in [(PReg::Rdi, dst), (PReg::Rax, byte), (PReg::Rcx, size)] {
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(register),
                src:   Operand::Reg(value),
            });
        }
        self.emit(MInst::RepStosb);
    }

    /// The address of a memory operand in a fresh register.
    fn lea(&mut self, mem: Mem) -> Reg {
        let dst = self.vreg();
        self.emit(MInst::Lea { dst, addr: mem });
        dst
    }
}

/// `mem` moved by `disp` bytes; callers keep the sum small.
fn shifted(mem: Mem, disp: i32) -> Mem {
    Mem::new(mem.base, mem.disp + disp)
}

/// The integer type a piece of `width` is loaded as.
const fn load_type(width: Width) -> Type {
    match width {
        | Width::B8 => Type::I8,
        | Width::B16 => Type::I16,
        | Width::B32 => Type::I32,
        | Width::B64 => Type::I64,
    }
}
