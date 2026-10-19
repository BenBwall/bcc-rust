//! Dynamic stack allocation: `dyn_alloca`, `stack_save` and
//! `stack_restore`.
//!
//! A function that moves `%rsp` at run time addresses its fixed frame from
//! `%rbp` (`MFunction::dynamic`). `dyn_alloca` rounds the size up to 16
//! bytes, lowers `%rsp` by it, and returns the address right above the
//! outgoing argument area, which stays at the bottom of the stack for the
//! calls that follow. On Windows the new pages are probed first, through
//! `___chkstk_ms` (MinGW) or `__chkstk` (MSVC), which read the size in
//! `%rax` and change no register ("x64 stack usage"). The frame keeps `%rsp`
//! 16-byte aligned, so the block starts aligned to 16; an alignment above 16
//! allocates `align - 16` extra bytes and rounds the address up within them,
//! never down into the outgoing arguments a callee may write.
//! `stack_save` reads `%rsp`; `stack_restore` writes it back, releasing
//! every later allocation (C99 §6.2.4p6, p. 32; PDF p. 44).

use super::{
    Selector,
    ValueKind,
};
use crate::{
    backend::native::mir::{
        AluOp,
        Base,
        Callee,
        FrameObject,
        MInst,
        Mem,
        Operand,
        PReg,
        PRegSet,
        Reg,
        SymbolRef,
        Width,
    },
    ir::{
        Align,
        Type,
        Value,
    },
    target::Target,
};

impl Selector<'_, '_, '_> {
    /// `dst = dyn_alloca size, align`.
    pub(super) fn dyn_alloca(&mut self, dst: Reg, size: Value, align: Align) {
        let extra = align.bytes().saturating_sub(16);
        let bytes = self.vreg();
        match self.kind(size) {
            | ValueKind::Const(size) => {
                let rounded = size.saturating_add(extra).saturating_add(15) & !15;
                self.constant(bytes, rounded, Type::I64);
            },
            | _ => {
                let size = self.reg(size);
                self.copy(bytes, size);
                let addend = self.vreg();
                self.constant(addend, extra + 15, Type::I64);
                self.emit(MInst::Alu {
                    op:    AluOp::Add,
                    width: Width::B64,
                    dst:   bytes,
                    src:   Operand::Reg(addend),
                });
                self.emit(MInst::Alu {
                    op:    AluOp::And,
                    width: Width::B64,
                    dst:   bytes,
                    src:   Operand::Imm(-16),
                });
            },
        }
        if matches!(self.target, Target::WindowsGnu | Target::WindowsMsvc) {
            let probe = if self.target == Target::WindowsMsvc {
                "__chkstk"
            } else {
                "___chkstk_ms"
            };
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(PReg::Rax),
                src:   Operand::Reg(bytes),
            });
            self.emit(MInst::Call {
                callee:   Callee::Symbol(SymbolRef::Runtime(probe)),
                uses:     PRegSet::of(&[PReg::Rax]),
                clobbers: PRegSet::of(&[PReg::R10, PReg::R11]),
            });
        }
        let rsp = Reg::Phys(PReg::Rsp);
        self.emit(MInst::Alu {
            op:    AluOp::Sub,
            width: Width::B64,
            dst:   rsp,
            src:   Operand::Reg(bytes),
        });
        self.emit(MInst::Lea {
            dst,
            addr: Mem::new(Base::Frame(FrameObject::OutgoingEnd), 0),
        });
        if extra > 0 {
            // The block starts 16-byte aligned and has `extra` spare bytes,
            // so the next multiple of `align` is at most `extra` above it.
            self.emit(MInst::Alu {
                op: AluOp::Add,
                width: Width::B64,
                dst,
                src: Operand::Imm(i32::try_from(extra).expect("the IR bounds alignments by 2^31")),
            });
            let mask = i32::try_from(-i64::try_from(align.bytes()).expect("alignments fit i64"))
                .expect("the IR bounds alignments by 2^31");
            self.emit(MInst::Alu {
                op: AluOp::And,
                width: Width::B64,
                dst,
                src: Operand::Imm(mask),
            });
        }
    }

    /// `dst = stack_save`.
    pub(super) fn stack_save(&mut self, dst: Reg) {
        self.emit(MInst::Mov {
            width: Width::B64,
            dst,
            src: Operand::Reg(Reg::Phys(PReg::Rsp)),
        });
    }

    /// `stack_restore position`.
    pub(super) fn stack_restore(&mut self, position: Value) {
        let src = self.reg(position);
        self.emit(MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rsp),
            src:   Operand::Reg(src),
        });
    }
}
