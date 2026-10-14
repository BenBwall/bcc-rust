//! Function entry, calls and returns: the post-ABI signature mapped onto
//! the convention's registers and stack slots.
//!
//! ABI lowering has decided everything C-specific: which aggregates travel
//! in registers (now plain scalars), in memory (`byval`), or come back in
//! memory (`sret`) or registers (`registers`, whose return area is not
//! passed), and which narrow integers are widened (`signext`, `zeroext`).
//! This file only places the resulting scalars.
//!
//! References: psABI §3.2.3 "Parameter Passing" (registers, stack order,
//! `%al` for variadic calls, `sret` returned in `%rax`); Microsoft Learn,
//! "x64 calling convention", sections "Parameter passing", "Varargs" and
//! "Return values".

use super::{
    Selector,
    ValueKind,
    width,
};
use crate::{
    backend::native::{
        conv::{
            ArgLocation,
            ConvKind,
        },
        mir::{
            Base,
            Callee,
            FrameObject,
            FrameSlotData,
            MInst,
            Mem,
            Operand,
            PReg,
            PRegSet,
            Reg,
            SymbolRef,
            Width,
        },
        support::Construct,
    },
    ir::{
        Extension,
        Inst,
        InstData,
        Linkage,
        ParamAbi,
        ResultRegisters,
        Symbol,
        Type,
        Value,
    },
    target::Target,
    util::bump::ArenaVec,
};

/// How a function hands back a structure result at `return`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum ResultArea {
    None,
    /// `sret`: the incoming pointer, returned in `%rax` as well.
    Sret(Reg),
    /// `registers`: the callee's own return area, loaded into `%rax` (and
    /// `%rdx`) at `return`.
    Registers {
        area:      Mem,
        registers: ResultRegisters,
    },
}

/// One argument of a call, after selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum Passed {
    /// A scalar: a register or an immediate.
    Scalar(Operand),
    /// A structure copied to the argument area (System V `byval`).
    ByVal {
        src:   Value,
        size:  u32,
        align: u32,
    },
}

impl Selector<'_, '_, '_> {
    /// Moves the parameters from their registers and stack slots into the
    /// entry block's parameter registers.
    pub(super) fn entry(&mut self) -> Result<(), Construct> {
        let body = self.body;
        let entry = body
            .entry_block()
            .expect("a selected body has an entry block");
        let params = body.block_params(entry);
        let mut assigner = self.conv.assigner();
        let mut stack = ArenaVec::new_in(self.arena);
        for (index, &param) in params.iter().enumerate() {
            let ValueKind::Reg(vreg) = self.kind(param) else {
                unreachable!("parameters are registers");
            };
            let dst = Reg::Virt(vreg);
            match self.signature.param_abi(index) {
                | Some(ParamAbi::Registers(registers)) if index == 0 => {
                    let slot = self.func.slots.len();
                    self.func.slots.push(FrameSlotData {
                        size:  u32::try_from(registers.area_bytes())
                            .expect("a return area is small"),
                        align: u32::try_from(registers.align.bytes().max(8))
                            .expect("a return area is aligned to at most 16"),
                    });
                    let area = Mem::new(
                        Base::Frame(FrameObject::Slot(
                            u32::try_from(slot).expect("frame slots fit u32"),
                        )),
                        0,
                    );
                    self.emit(MInst::Lea { dst, addr: area });
                    self.result = ResultArea::Registers { area, registers };
                },
                | Some(ParamAbi::ByVal(layout)) => {
                    let offset =
                        assigner.memory(byval_size(layout.size)?, align(layout.align.bytes()));
                    stack.push((dst, offset, true));
                },
                | abi => {
                    if matches!(abi, Some(ParamAbi::Sret(_))) {
                        self.result = ResultArea::Sret(dst);
                    }
                    match assigner.scalar() {
                        | ArgLocation::Reg(register) => self.copy(dst, Reg::Phys(register)),
                        | ArgLocation::Stack(offset) => stack.push((dst, offset, false)),
                    }
                },
            }
        }
        for (dst, offset, byval) in stack {
            let mem = Mem::new(Base::Frame(FrameObject::Incoming(offset)), 0);
            if byval {
                self.emit(MInst::Lea { dst, addr: mem });
            } else {
                self.emit(MInst::Mov {
                    width: Width::B64,
                    dst,
                    src: Operand::Mem(mem),
                });
            }
        }
        // Another compiler may leave an `i1` argument's upper bits set.
        for &param in params {
            if let (Type::I1, ValueKind::Reg(vreg)) = (self.ty(param), self.kind(param)) {
                self.canonical_bool(Reg::Virt(vreg));
            }
        }
        // MinGW's `main` runs the constructors first, as GCC and Clang emit.
        if self.target == Target::WindowsGnu
            && self.function.name == "main"
            && self.function.linkage == Linkage::External
        {
            self.call_sequence(Callee::Symbol(SymbolRef::Runtime("__main")), &[], false);
        }
        Ok(())
    }

    /// A direct or indirect call, its arguments and its result.
    pub(super) fn call(&mut self, inst: Inst, data: InstData) -> Result<(), Construct> {
        let body = self.body;
        let (callee, signature, args) = match data {
            | InstData::Call { func, args } => (
                Callee::Symbol(SymbolRef::Ir(Symbol::Function(func))),
                self.module.function_signature(func),
                body.value_list(args),
            ),
            | InstData::CallIndirect { sig, args } => {
                let values = body.value_list(args);
                let (&callee, args) = values.split_first().expect("an indirect call has a callee");
                (
                    Callee::Reg(self.reg(callee)),
                    self.module.signature(sig),
                    args,
                )
            },
            | _ => return Err(Construct::Opcode(data.opcode())),
        };
        let mut passed = ArenaVec::with_capacity_in(args.len(), self.arena);
        let mut area = None;
        for (index, &arg) in args.iter().enumerate() {
            let fixed = index < signature.params.len();
            match signature.param_abi(index).filter(|_| fixed) {
                | Some(ParamAbi::Registers(registers)) => area = Some((arg, registers)),
                | Some(ParamAbi::ByVal(layout)) => passed.push(Passed::ByVal {
                    src:   arg,
                    size:  byval_size(layout.size)?,
                    align: align(layout.align.bytes()),
                }),
                | _ => {
                    let extension = signature.param_extension(index).filter(|_| fixed);
                    let operand = match extension {
                        | Some(Extension::Sign) => Operand::Reg(self.extended(arg, true)),
                        | Some(Extension::Zero) => Operand::Reg(self.extended(arg, false)),
                        | _ => self.operand(arg, Width::B64),
                    };
                    passed.push(Passed::Scalar(operand));
                },
            }
        }
        self.call_sequence(callee, &passed, signature.variadic);
        if let Some(result) = body.inst_result(inst) {
            let ValueKind::Reg(dst) = self.kind(result) else {
                unreachable!("a call result is a register");
            };
            self.copy(Reg::Virt(dst), Reg::Phys(PReg::Rax));
            if self.ty(result) == Type::I1 {
                self.canonical_bool(Reg::Virt(dst));
            }
        }
        if let Some((arg, registers)) = area {
            let mem = self.address(arg);
            for ((offset, ty), register) in registers.parts().zip([PReg::Rax, PReg::Rdx]) {
                self.emit(MInst::Store {
                    width: width(ty),
                    dst:   Mem::new(
                        mem.base,
                        mem.disp + i32::try_from(offset).expect("a part offset is 0 or 8"),
                    ),
                    src:   Operand::Reg(Reg::Phys(register)),
                });
            }
        }
        Ok(())
    }

    /// Places the arguments and emits the call: `byval` copies, stack
    /// arguments, then register arguments (so nothing between them and the
    /// call needs a fixed register), `%al` for a System V variadic callee,
    /// and the `call`, which clobbers the caller-saved registers.
    pub(in crate::backend::native) fn call_sequence(
        &mut self,
        callee: Callee,
        args: &[Passed],
        variadic: bool,
    ) {
        let mut assigner = self.conv.assigner();
        let mut registers = ArenaVec::new_in(self.arena);
        let mut stack = ArenaVec::new_in(self.arena);
        let mut copies = ArenaVec::new_in(self.arena);
        for &arg in args {
            match arg {
                | Passed::Scalar(operand) => match assigner.scalar() {
                    | ArgLocation::Reg(register) => registers.push((register, operand)),
                    | ArgLocation::Stack(offset) => stack.push((offset, operand)),
                },
                | Passed::ByVal { src, size, align } =>
                    copies.push((assigner.memory(size, align), src, size)),
            }
        }
        let home = self.conv.home_area;
        self.func.calls = true;
        self.func.outgoing = self.func.outgoing.max(home + assigner.stack_bytes());
        let outgoing = |offset: u32| Mem::new(Base::Frame(FrameObject::Outgoing(home + offset)), 0);
        for (offset, src, size) in copies {
            let src = self.address(src);
            self.copy_bytes(outgoing(offset), src, u64::from(size));
        }
        for (offset, operand) in stack {
            self.emit(MInst::Store {
                width: Width::B64,
                dst:   outgoing(offset),
                src:   operand,
            });
        }
        let mut uses = PRegSet::EMPTY;
        for (register, operand) in registers {
            if let Operand::Reg(Reg::Virt(vreg)) = operand {
                self.func.hint(vreg, Reg::Phys(register));
            }
            self.emit(MInst::Mov {
                width: Width::B64,
                dst:   Reg::Phys(register),
                src:   operand,
            });
            uses = uses.with(register);
        }
        if variadic && self.conv.kind == ConvKind::SystemV {
            // psABI §3.2.3: %al bounds the vector registers used, none here.
            self.emit(MInst::Mov {
                width: Width::B32,
                dst:   Reg::Phys(PReg::Rax),
                src:   Operand::Imm(0),
            });
            uses = uses.with(PReg::Rax);
        }
        self.emit(MInst::Call {
            callee,
            uses,
            clobbers: self.conv.caller_saved,
        });
    }

    /// `return`: the result into `%rax` (extended as the signature says),
    /// an `sret` pointer into `%rax`, or a register result loaded from the
    /// return area.
    pub(super) fn ret(&mut self, value: Option<Value>) {
        let mut uses = PRegSet::EMPTY;
        match self.result {
            | ResultArea::None => {},
            | ResultArea::Sret(pointer) => {
                self.copy(Reg::Phys(PReg::Rax), pointer);
                uses = uses.with(PReg::Rax);
            },
            | ResultArea::Registers { area, registers } => {
                for ((offset, ty), register) in registers.parts().zip([PReg::Rax, PReg::Rdx]) {
                    let ty = if ty == Type::Ptr { Type::I64 } else { ty };
                    let mem = Mem::new(
                        area.base,
                        area.disp + i32::try_from(offset).expect("a part offset is 0 or 8"),
                    );
                    self.load(ty, Reg::Phys(register), mem);
                    uses = uses.with(register);
                }
            },
        }
        if let Some(value) = value {
            let rax = Reg::Phys(PReg::Rax);
            match self.signature.result_extension {
                | Some(extension @ (Extension::Sign | Extension::Zero)) => {
                    let signed = extension == Extension::Sign;
                    let src = self.extended(value, signed);
                    self.copy(rax, src);
                },
                | _ => {
                    let src = self.operand(value, Width::B64);
                    if let Operand::Reg(Reg::Virt(vreg)) = src {
                        self.func.hint(vreg, rax);
                    }
                    self.emit(MInst::Mov {
                        width: Width::B64,
                        dst: rax,
                        src,
                    });
                },
            }
            uses = uses.with(PReg::Rax);
        }
        self.emit(MInst::Ret { uses });
    }
}

/// A `byval` size, which the support check bounds with the frame.
fn byval_size(size: u64) -> Result<u32, Construct> {
    u32::try_from(size).map_err(|_| Construct::FrameSize(size))
}

/// An alignment in bytes as `u32`; the IR bounds alignments by 2^31.
fn align(bytes: u64) -> u32 {
    u32::try_from(bytes).unwrap_or(1 << 31)
}
