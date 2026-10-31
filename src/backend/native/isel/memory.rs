//! Addresses, loads, stores, `copy` and `fill`.
//!
//! An address operand folds a frame slot, a symbol the module defines (any
//! symbol on COFF but a weak declaration), or a chain of `ptr_add`s of a
//! base, constants and one scaled register offset into one memory operand
//! (`disp(base, index, scale)`). A symbol base with an index is loaded into
//! a register once, at the function's entry. On ELF a symbol the module does
//! not define is reached through its GOT entry, since executables are
//! position-independent; on COFF a weak declaration, whose address may be
//! null, through a `.refptr` stub holding its address, as Clang does.
//!
//! A thread-local global's address is computed where `global_addr` names
//! it, with the sequences Clang emits for an executable, the only code the
//! back end generates. ELF uses the thread pointer at `%fs:0` and the
//! local-exec model (`sym@TPOFF`) for globals the module defines, or for
//! every global with `-ftls-model=local-exec`, and the initial-exec model
//! (`sym@GOTTPOFF`) for the rest (Drepper, "ELF Handling For Thread-Local
//! Storage", §4.3.6 and §4.3.5). Windows
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
    is_pair,
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
            Index,
            MBlock,
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
        Entity,
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
    target::{
        Target,
        TlsModel,
    },
    util::bump::ArenaVec,
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
        } else if self.tls_model == TlsModel::LocalExec
            || defines(self.module, Symbol::Global(global))
        {
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
                if let Some(mem) = self.folded_address(value) {
                    return mem;
                },
            | _ => {},
        }
        Mem::new(Base::Reg(self.reg(value)), 0)
    }

    /// The address a `ptr_add` chain computes, as one operand: `disp(base,
    /// index, scale)` from a base register or frame slot, at most one
    /// register offset (scaled when an `imul` or `shl` by 1, 2, 4 or 8
    /// computed it) and constant offsets. A symbol base with an index is
    /// loaded into a register first, since a RIP-relative operand has no
    /// index. `None` when `value` is no such chain.
    pub(super) fn folded_address(&mut self, value: Value) -> Option<Mem> {
        let mut current = value;
        let mut index = None;
        let mut disp = 0_i64;
        // The chains the optimizer leaves are short; the bound keeps the
        // walk finite on any input.
        for _ in 0..4 {
            let ValueDef::Result(inst) = self.body.value_def(current) else {
                break;
            };
            let InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, offset],
                ..
            } = *self.body.inst(inst)
            else {
                break;
            };
            match self.kind(offset) {
                | ValueKind::Const(bits) => disp = disp.checked_add(bits.cast_signed())?,
                | ValueKind::Reg(_) if index.is_none() => index = Some(self.scaled(offset)),
                | _ => break,
            }
            current = base;
        }
        if current == value {
            return None;
        }
        let (base, disp) = match self.kind(current) {
            | ValueKind::Reg(base) => (Base::Reg(Reg::Virt(base)), disp),
            | ValueKind::Addr(AddrBase::Slot(slot), offset) => (
                Base::Frame(FrameObject::Slot(slot)),
                disp.checked_add(offset)?,
            ),
            | ValueKind::Addr(AddrBase::Symbol(symbol), offset)
                if index.is_none() && self.direct(symbol) =>
                (
                    Base::Symbol(SymbolRef::Ir(symbol)),
                    disp.checked_add(offset)?,
                ),
            | ValueKind::Addr(AddrBase::Symbol(symbol), offset) if index.is_some() => (
                Base::Reg(self.symbol_base(symbol)),
                disp.checked_add(offset)?,
            ),
            | _ => return None,
        };
        if disp.abs() >= MAX_DISP {
            return None;
        }
        Some(Mem {
            base,
            index,
            disp: i32::try_from(disp).expect("the displacement was bounded"),
        })
    }

    /// A register holding `symbol`'s address, loaded once at the
    /// function's entry: an indexed operand needs its base in a register,
    /// and a loop would otherwise recompute it at every access. The
    /// allocator splits or spills it like any other value where registers
    /// are scarce.
    fn symbol_base(&mut self, symbol: Symbol) -> Reg {
        if let Some(&(_, reg)) = self.bases.iter().find(|&&(other, _)| other == symbol) {
            return reg;
        }
        let reg = self.vreg();
        self.bases.push((symbol, reg));
        reg
    }

    /// Puts the loads of [`Self::symbol_base`]'s registers at the start of
    /// the entry block, which dominates every use.
    pub(super) fn load_bases(&mut self) {
        if self.bases.is_empty() {
            return;
        }
        let entry = MBlock::new(0);
        let rest = std::mem::replace(
            &mut self.func.blocks[entry.index()].insts,
            ArenaVec::new_in(self.arena),
        );
        self.current = entry;
        for index in 0..self.bases.len() {
            let (symbol, reg) = self.bases[index];
            self.address_into(reg, AddrBase::Symbol(symbol), 0);
        }
        self.func.blocks[entry.index()].insts.extend(rest);
    }

    /// An `i64` offset as a scaled index: the multiplicand of an `imul` by
    /// 1, 2, 4 or 8 or of a `shl` by at most 3, else the offset itself.
    fn scaled(&self, offset: Value) -> Index {
        let ValueKind::Reg(reg) = self.kind(offset) else {
            unreachable!("only a register offset is an index");
        };
        let plain = Index {
            reg:   Reg::Virt(reg),
            scale: 1,
        };
        let ValueDef::Result(inst) = self.body.value_def(offset) else {
            return plain;
        };
        let InstData::Binary {
            opcode,
            ty: Type::I64,
            args: [lhs, rhs],
            ..
        } = *self.body.inst(inst)
        else {
            return plain;
        };
        let (value, factor) = match (opcode, self.kind(lhs), self.kind(rhs)) {
            | (Opcode::Imul, ValueKind::Reg(value), ValueKind::Const(factor))
            | (Opcode::Imul, ValueKind::Const(factor), ValueKind::Reg(value)) => (value, factor),
            | (Opcode::Shl, ValueKind::Reg(value), ValueKind::Const(count)) if count <= 3 =>
                (value, 1 << count),
            | _ => return plain,
        };
        match factor {
            | 1 | 2 | 4 | 8 => Index {
                reg:   Reg::Virt(value),
                #[expect(clippy::cast_possible_truncation, reason = "The factor is at most 8.")]
                scale: factor as u8,
            },
            | _ => plain,
        }
    }

    /// Loads, stores, `copy` and `fill`.
    pub(super) fn memory(&mut self, inst: Inst, data: InstData) -> Result<(), Construct> {
        match data {
            | InstData::Load { ty, addr, .. } if !is_pair(ty) => {
                let Some(ValueKind::Reg(dst)) =
                    self.body.inst_result(inst).map(|value| self.kind(value))
                else {
                    unreachable!("a load has a register result");
                };
                let mem = self.address(addr);
                self.load(ty, Reg::Virt(dst), mem);
            },
            | InstData::Load { ty, addr, .. } => {
                let dst = self.result_pair(inst);
                let mem = self.address(addr);
                self.pair_load(ty, dst, mem);
            },
            | InstData::Store {
                ty,
                args: [value, addr],
                ..
            } if is_pair(ty) => {
                let mem = self.address(addr);
                self.pair_store(ty, value, mem);
            },
            | InstData::Store {
                ty,
                args: [value, addr],
                ..
            } if ty.half_format().is_some() => {
                let src = self.reg(value);
                let dst = self.address(addr);
                self.half_store(src, dst);
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
                | Opcode::Trampoline => self.trampoline(dst, src, size),
                | _ => return Err(Construct::Opcode(opcode)),
            },
            | _ => return Err(Construct::Opcode(data.opcode())),
        }
        Ok(())
    }

    /// Writes a trampoline at `storage`: `movabs $func, %r11`, `movabs
    /// $chain, %r10`, `jmp *%r11`, as GCC's x86-64 trampoline does, which
    /// runs on the stack the module marks executable.
    /// GNU extension: GCC manual, "Nested Functions".
    fn trampoline(&mut self, storage: Value, func: Value, chain: Value) {
        let at = self.address(storage);
        let func = self.reg(func);
        let chain = self.reg(chain);
        // REX.W+B `mov` with a 64-bit immediate into r11 and r10, and
        // REX.B `jmp` through r11 with a `nop` after it.
        for (offset, width, src) in [
            (0, Width::B16, Operand::Imm(0xBB49)),
            (2, Width::B64, Operand::Reg(func)),
            (10, Width::B16, Operand::Imm(0xBA49)),
            (12, Width::B64, Operand::Reg(chain)),
            (20, Width::B32, Operand::Imm(0x90E3_FF49_u32.cast_signed())),
        ] {
            self.emit(MInst::Store {
                width,
                dst: at.offset(offset),
                src,
            });
        }
    }

    /// `dst ← [mem]` of type `ty`; narrow loads zero-extend to 32 bits.
    pub(super) fn load(&mut self, ty: Type, dst: Reg, mem: Mem) {
        let width = width(ty);
        if ty.half_format().is_some() {
            self.half_load(dst, mem);
            return;
        }
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
        let mut pieces = [(0_i32, Width::B8, Reg::Virt(Entity::new(0))); 16];
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
    mem.offset(disp)
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
