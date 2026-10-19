//! Function entry, calls and returns: the post-ABI signature mapped onto
//! the convention's registers and stack slots, and the variadic entry
//! sequence `va_start` reads.
//!
//! ABI lowering has decided everything C-specific: which aggregates travel
//! in registers (now plain scalars), in memory (`byval`), or come back in
//! memory (`sret`) or registers (`registers`, whose return area is not
//! passed), which narrow integers are widened (`signext`, `zeroext`), and
//! how `va_arg` walks a `va_list`. This file only places the resulting
//! scalars and builds the `va_list` the expanded `va_arg` expects.
//!
//! The wide scalars ABI lowering leaves as they are travel as LLVM's code
//! generator passes them, which is what Clang's output links against: under
//! System V an `i128` in a register pair, an `f80` in a 16-byte stack slot
//! and returned in `%st(0)`, an `f128` in a whole SSE register (or a 16-byte
//! stack slot) and returned in `%xmm0`; under Win64 an `f80` or `f128`
//! argument as a pointer to a 16-byte copy, and its result through a hidden
//! pointer passed first and returned in `%rax`. Runtime routines
//! ([`Selector::libcall`]) take and return them the same way, except that
//! Win64 routines take an `i128` by reference and return one in `%xmm0`.
//!
//! References: psABI §3.2.3 "Parameter Passing" (registers, stack order,
//! `%al` for variadic calls, `sret` returned in `%rax`, SSE results in
//! `%xmm0`/`%xmm1`, X87 results in `%st0`) and §3.5.7 "Variable Argument
//! Lists" (the register save area and `va_list`, Figure 3.33); Microsoft
//! Learn, "x64 calling convention", sections "Parameter passing", "Varargs"
//! (floating-point variadic arguments are duplicated in the integer register
//! of their position) and "Return values".

use super::{
    Selector,
    ValueKind,
    class_of,
    is_pair,
    wide::Halves,
    width,
};
use crate::{
    backend::native::{
        conv::{
            ArgLocation,
            ConvKind,
        },
        frame::REGISTER_SAVE_BYTES,
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
            RegClass,
            SymbolRef,
            Width,
            X87Format,
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
    /// `registers`: the callee's own return area, loaded into the result
    /// registers at `return`.
    Registers {
        area:      Mem,
        registers: ResultRegisters,
    },
}

/// What `va_start` stores: where the named arguments stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum VaStart {
    None,
    /// System V: the `gp_offset` and `fp_offset` of the first variadic
    /// argument and the stack offset of the first variadic stack argument.
    SystemV {
        gp:    u32,
        fp:    u32,
        stack: u32,
    },
    /// Win64: the home-area offset of the first variadic slot.
    Win64 {
        slot: u32,
    },
}

/// One argument of a call, after selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum Passed {
    /// A scalar: a register (of either class) or an immediate.
    Scalar(Operand),
    /// A structure copied to the argument area (System V `byval`).
    ByVal {
        src:   Value,
        size:  u32,
        align: u32,
    },
    /// A System V `__int128`: low and high halves.
    Pair(Reg, Reg),
    /// A System V `long double`, always in a 16-byte stack slot: the low
    /// eight significand bytes, then the sign and exponent.
    Memory(Reg, Reg),
    /// A System V `__float128`: all of an SSE register, or a 16-byte stack
    /// slot.
    Vector(Reg, Reg),
}

/// An argument of a runtime routine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum LibArg {
    /// An integer of at most 64 bits, already extended to what the routine
    /// reads.
    Gpr(Reg),
    /// An `f32` or `f64`.
    Xmm(Reg),
    /// An `i128`, `f80` or `f128` pair.
    Wide(Type, Halves),
}

/// The result of a runtime routine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::backend::native) enum LibResult {
    /// An integer of at most 64 bits, from `%rax`.
    Gpr(Reg),
    /// An `f32` or `f64`, from `%xmm0`.
    Xmm(Reg),
    /// An `i128`, `f80` or `f128` pair.
    Wide(Type, Halves),
}

impl Selector<'_, '_, '_> {
    /// Moves the parameters from their registers and stack slots into the
    /// entry block's parameter registers, after saving the argument
    /// registers a variadic function's `va_start` reads.
    pub(super) fn entry(&mut self, uses_va_start: bool) -> Result<(), Construct> {
        let body = self.body;
        let entry = body
            .entry_block()
            .expect("a selected body has an entry block");
        let params = body.block_params(entry);
        if uses_va_start {
            self.save_argument_registers();
        }
        let mut assigner = self.conv.assigner();
        let mut stack = ArenaVec::new_in(self.arena);
        // Win64 `f80` and `f128` parameters: the pointers to their copies.
        let mut indirect = ArenaVec::new_in(self.arena);
        if self.conv.kind == ConvKind::Win64
            && matches!(self.signature.result, Some(Type::F80 | Type::F128))
        {
            let pointer = self.vreg();
            match assigner.scalar(RegClass::Gpr) {
                | ArgLocation::Reg(register) => self.copy(pointer, Reg::Phys(register)),
                | ArgLocation::Stack(_) => unreachable!("the first position is a register"),
            }
            self.result = ResultArea::Sret(pointer);
        }
        for (index, &param) in params.iter().enumerate() {
            let vreg = match self.kind(param) {
                | ValueKind::Reg(vreg) => vreg,
                | ValueKind::Pair(low, high) => {
                    let halves = (Reg::Virt(low), Reg::Virt(high));
                    let at = |offset: u32| {
                        [
                            (halves.0, offset, Some(Type::I64)),
                            (halves.1, offset + 8, Some(Type::I64)),
                        ]
                    };
                    match (self.conv.kind, self.ty(param)) {
                        | (ConvKind::SystemV, Type::I128) => match assigner.pair() {
                            | Ok((first, second)) => {
                                self.copy(halves.0, Reg::Phys(first));
                                self.copy(halves.1, Reg::Phys(second));
                            },
                            | Err(offset) => stack.extend(at(offset)),
                        },
                        | (ConvKind::SystemV, Type::F80) =>
                            stack.extend(at(assigner.memory(16, 16))),
                        | (ConvKind::SystemV, _) => match assigner.vector() {
                            | Ok(register) => {
                                let temp = self.wide_temporary();
                                self.emit(MInst::Move128 {
                                    load: false,
                                    reg:  Reg::Phys(register),
                                    mem:  temp,
                                });
                                self.wide_load(halves, temp);
                            },
                            | Err(offset) => stack.extend(at(offset)),
                        },
                        | (ConvKind::Win64, ty) => {
                            let pointer = self.vreg();
                            match assigner.scalar(RegClass::Gpr) {
                                | ArgLocation::Reg(register) =>
                                    self.copy(pointer, Reg::Phys(register)),
                                | ArgLocation::Stack(offset) =>
                                    stack.push((pointer, offset, Some(Type::I64))),
                            }
                            indirect.push((ty, halves, pointer));
                        },
                    }
                    continue;
                },
                | _ => unreachable!("parameters are registers"),
            };
            let dst = Reg::Virt(vreg);
            match self.signature.param_abi(index) {
                | Some(ParamAbi::Registers(registers)) if index == 0 => {
                    let slot = self.func.slots.len();
                    self.func.slots.push(FrameSlotData {
                        size:  u32::try_from(registers.area_bytes())
                            .expect("a return area is small"),
                        align: u32::try_from(registers.align.bytes().max(16))
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
                    stack.push((dst, offset, None));
                },
                | abi => {
                    if matches!(abi, Some(ParamAbi::Sret(_))) {
                        self.result = ResultArea::Sret(dst);
                    }
                    let ty = self.ty(param);
                    match assigner.scalar(class_of(ty)) {
                        | ArgLocation::Reg(register) => self.copy(dst, Reg::Phys(register)),
                        | ArgLocation::Stack(offset) => stack.push((dst, offset, Some(ty))),
                    }
                },
            }
        }
        // The highest incoming offset a frame operand names: a stack
        // parameter's, or where `va_start` finds the variadic arguments.
        let named = stack.iter().map(|&(_, offset, _)| offset).max();
        let variadic = uses_va_start.then(|| assigner.stack_bytes());
        self.func.incoming = named.max(variadic).unwrap_or(0);
        for (dst, offset, ty) in stack {
            let mem = Mem::new(Base::Frame(FrameObject::Incoming(offset)), 0);
            match ty {
                | None => self.emit(MInst::Lea { dst, addr: mem }),
                | Some(ty) if ty.is_float() => self.emit(MInst::XMov {
                    width: Width::B64,
                    dst,
                    src: Operand::Mem(mem),
                }),
                | Some(_) => self.emit(MInst::Mov {
                    width: Width::B64,
                    dst,
                    src: Operand::Mem(mem),
                }),
            }
        }
        for (ty, halves, pointer) in indirect {
            self.pair_load(ty, halves, Mem::new(Base::Reg(pointer), 0));
        }
        if uses_va_start {
            let (gpr, xmm) = assigner.registers_used();
            self.va_start = match self.conv.kind {
                | ConvKind::SystemV => VaStart::SystemV {
                    gp:    8 * u32::try_from(gpr).expect("six registers"),
                    fp:    48 + 16 * u32::try_from(xmm).expect("eight registers"),
                    stack: assigner.stack_bytes(),
                },
                // Every position, register or not, has an 8-byte slot from
                // the home area on.
                | ConvKind::Win64 => VaStart::Win64 {
                    slot: 8 * u32::try_from(gpr).expect("positions fit u32")
                        + assigner.stack_bytes(),
                },
            };
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

    /// Saves the argument registers where `va_arg` finds them: System V's
    /// register save area (every SSE argument register too: `%al` only
    /// bounds them), or Win64's home area, which makes all arguments one
    /// array of 8-byte slots.
    fn save_argument_registers(&mut self) {
        let conv = self.conv;
        match conv.kind {
            | ConvKind::SystemV => {
                self.func.register_save = true;
                for (index, &register) in conv.arguments.iter().enumerate() {
                    let offset = 8 * u32::try_from(index).expect("six registers");
                    self.emit(MInst::Store {
                        width: Width::B64,
                        dst:   Mem::new(Base::Frame(FrameObject::RegisterSave(offset)), 0),
                        src:   Operand::Reg(Reg::Phys(register)),
                    });
                }
                for (index, &register) in conv.float_arguments.iter().enumerate() {
                    let offset = 48 + 16 * u32::try_from(index).expect("eight registers");
                    debug_assert!(offset < REGISTER_SAVE_BYTES, "the save area holds it");
                    // All 16 bytes: a `__float128` argument fills its register.
                    self.emit(MInst::Move128 {
                        load: false,
                        reg:  Reg::Phys(register),
                        mem:  Mem::new(Base::Frame(FrameObject::RegisterSave(offset)), 0),
                    });
                }
            },
            | ConvKind::Win64 =>
                for (index, &register) in conv.arguments.iter().enumerate() {
                    let offset = 8 * u32::try_from(index).expect("four registers");
                    self.emit(MInst::Store {
                        width: Width::B64,
                        dst:   Mem::new(Base::Frame(FrameObject::Home(offset)), 0),
                        src:   Operand::Reg(Reg::Phys(register)),
                    });
                },
        }
    }

    /// `va_start list`: the target's `va_list` for the arguments after the
    /// named ones. System V's is a structure of `gp_offset`, `fp_offset`,
    /// `overflow_arg_area` and `reg_save_area` (psABI Figure 3.33); Win64's
    /// is a pointer to the next 8-byte slot.
    pub(super) fn va_start(&mut self, list: Value) {
        let mem = self.address(list);
        let at = |disp: i32| Mem::new(mem.base, mem.disp + disp);
        match self.va_start {
            | VaStart::SystemV { gp, fp, stack } => {
                let overflow = self.vreg();
                self.emit(MInst::Lea {
                    dst:  overflow,
                    addr: Mem::new(Base::Frame(FrameObject::Incoming(stack)), 0),
                });
                let save = self.vreg();
                self.emit(MInst::Lea {
                    dst:  save,
                    addr: Mem::new(Base::Frame(FrameObject::RegisterSave(0)), 0),
                });
                for (disp, value) in [(0, gp), (4, fp)] {
                    self.emit(MInst::Store {
                        width: Width::B32,
                        dst:   at(disp),
                        src:   Operand::Imm(i32::try_from(value).expect("offsets are small")),
                    });
                }
                for (disp, value) in [(8, overflow), (16, save)] {
                    self.emit(MInst::Store {
                        width: Width::B64,
                        dst:   at(disp),
                        src:   Operand::Reg(value),
                    });
                }
            },
            | VaStart::Win64 { slot } => {
                let next = self.vreg();
                self.emit(MInst::Lea {
                    dst:  next,
                    addr: Mem::new(Base::Frame(FrameObject::Home(slot)), 0),
                });
                self.emit(MInst::Store {
                    width: Width::B64,
                    dst:   at(0),
                    src:   Operand::Reg(next),
                });
            },
            | VaStart::None => unreachable!("entry prepared va_start"),
        }
    }

    /// `va_copy dst, src`: the `va_list` object copied.
    pub(super) fn va_copy(&mut self, dst: Value, src: Value) {
        let bytes = match self.conv.kind {
            | ConvKind::SystemV => 24,
            | ConvKind::Win64 => 8,
        };
        let dst = self.address(dst);
        let src = self.address(src);
        self.copy_bytes(dst, src, bytes);
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
        let mut passed = ArenaVec::with_capacity_in(args.len() + 1, self.arena);
        let mut area = None;
        let sret = signature
            .result
            .and_then(|ty| self.wide_result_area(ty, &mut passed));
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
                    let ty = self.ty(arg);
                    if is_pair(ty) {
                        let halves = self.pair(arg);
                        let arg = self.pass_wide(ty, halves);
                        passed.push(arg);
                        continue;
                    }
                    let operand = match extension {
                        | Some(Extension::Sign) => Operand::Reg(self.extended(arg, true)),
                        | Some(Extension::Zero) => Operand::Reg(self.extended(arg, false)),
                        | _ if self.ty(arg).is_float() => Operand::Reg(self.reg(arg)),
                        | _ => self.operand(arg, Width::B64),
                    };
                    passed.push(Passed::Scalar(operand));
                },
            }
        }
        self.call_sequence(callee, &passed, signature.variadic);
        let result = body.inst_result(inst);
        if let Some(result) = result
            && let ValueKind::Pair(low, high) = self.kind(result)
        {
            self.receive_wide(self.ty(result), (Reg::Virt(low), Reg::Virt(high)), sret);
        } else if result.is_none()
            && signature.result == Some(Type::F80)
            && self.conv.kind == ConvKind::SystemV
        {
            // The x87 stack must be empty again.
            self.emit(MInst::X87Pop);
        } else if let Some(result) = result {
            let ValueKind::Reg(dst) = self.kind(result) else {
                unreachable!("a call result is a register");
            };
            let ty = self.ty(result);
            let register = if ty.is_float() { PReg::Xmm0 } else { PReg::Rax };
            self.copy(Reg::Virt(dst), Reg::Phys(register));
            if ty == Type::I1 {
                self.canonical_bool(Reg::Virt(dst));
            }
        }
        if let Some((arg, registers)) = area {
            let mem = self.address(arg);
            if matches!(registers.first, Type::I128 | Type::F128) {
                // Win64's `__int128` or System V's `__float128` result, all
                // of `%xmm0`.
                self.emit(MInst::Move128 {
                    load: false,
                    reg: Reg::Phys(PReg::Xmm0),
                    mem,
                });
                return Ok(());
            }
            if registers.first == Type::F80 {
                // `%st(0)`, and `%st(1)` for a complex `long double`.
                self.emit(MInst::X87Store {
                    format: X87Format::F80,
                    mem,
                });
                if registers.second == Some(Type::F80) {
                    let offset =
                        i32::try_from(registers.second_offset).expect("a part offset is small");
                    self.emit(MInst::X87Store {
                        format: X87Format::F80,
                        mem:    Mem::new(mem.base, mem.disp + offset),
                    });
                }
                return Ok(());
            }
            for (offset, ty, register) in result_registers(registers) {
                let dst = Mem::new(mem.base, mem.disp + offset);
                if ty.is_float() {
                    self.emit(MInst::XStore {
                        width: width(ty),
                        dst,
                        src: Reg::Phys(register),
                    });
                } else {
                    self.emit(MInst::Store {
                        width: width(ty),
                        dst,
                        src: Operand::Reg(Reg::Phys(register)),
                    });
                }
            }
        }
        Ok(())
    }

    /// Places the arguments and emits the call: `byval` copies, stack
    /// arguments, then register arguments (so nothing between them and the
    /// call needs a fixed register), `%al` for a System V variadic callee,
    /// and the `call`, which clobbers the caller-saved registers. A Win64
    /// variadic callee also gets each SSE argument in its position's
    /// integer register.
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
        let mut vectors = ArenaVec::new_in(self.arena);
        for &arg in args {
            match arg {
                | Passed::Scalar(operand) => {
                    let class = match operand {
                        | Operand::Reg(reg) => self.func.class(reg),
                        | _ => RegClass::Gpr,
                    };
                    match assigner.scalar(class) {
                        | ArgLocation::Reg(register) => registers.push((register, operand)),
                        | ArgLocation::Stack(offset) => stack.push((offset, operand, class)),
                    }
                },
                | Passed::ByVal { src, size, align } =>
                    copies.push((assigner.memory(size, align), src, size)),
                | Passed::Pair(low, high) => match assigner.pair() {
                    | Ok((first, second)) => {
                        registers.push((first, Operand::Reg(low)));
                        registers.push((second, Operand::Reg(high)));
                    },
                    | Err(offset) => {
                        stack.push((offset, Operand::Reg(low), RegClass::Gpr));
                        stack.push((offset + 8, Operand::Reg(high), RegClass::Gpr));
                    },
                },
                | Passed::Memory(low, high) => {
                    let offset = assigner.memory(16, 16);
                    stack.push((offset, Operand::Reg(low), RegClass::Gpr));
                    stack.push((offset + 8, Operand::Reg(high), RegClass::Gpr));
                },
                | Passed::Vector(low, high) => match assigner.vector() {
                    | Ok(register) => vectors.push((register, low, high)),
                    | Err(offset) => {
                        stack.push((offset, Operand::Reg(low), RegClass::Gpr));
                        stack.push((offset + 8, Operand::Reg(high), RegClass::Gpr));
                    },
                },
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
        for (offset, operand, class) in stack {
            match (class, operand) {
                | (RegClass::Xmm, Operand::Reg(src)) => self.emit(MInst::XStore {
                    width: Width::B64,
                    dst: outgoing(offset),
                    src,
                }),
                | _ => self.emit(MInst::Store {
                    width: Width::B64,
                    dst:   outgoing(offset),
                    src:   operand,
                }),
            }
        }
        // A whole SSE register is loaded from a copy of its halves.
        let mut loads = ArenaVec::with_capacity_in(vectors.len(), self.arena);
        for (register, low, high) in vectors {
            let temp = self.wide_temporary();
            for (half, disp) in [(low, 0), (high, 8)] {
                self.emit(MInst::Store {
                    width: Width::B64,
                    dst:   Mem::new(temp.base, disp),
                    src:   Operand::Reg(half),
                });
            }
            loads.push((register, temp));
        }
        let mut uses = PRegSet::EMPTY;
        for (register, mem) in loads {
            self.emit(MInst::Move128 {
                load: true,
                reg: Reg::Phys(register),
                mem,
            });
            uses = uses.with(register);
        }
        for (register, operand) in registers {
            if let Operand::Reg(Reg::Virt(vreg)) = operand {
                self.func.hint(vreg, Reg::Phys(register));
            }
            match (register.class(), operand) {
                | (RegClass::Xmm, Operand::Reg(src)) => {
                    self.emit(MInst::XMov {
                        width: Width::B64,
                        dst:   Reg::Phys(register),
                        src:   Operand::Reg(src),
                    });
                    if variadic && self.conv.kind == ConvKind::Win64 {
                        let position = self
                            .conv
                            .float_arguments
                            .iter()
                            .position(|&candidate| candidate == register)
                            .expect("an SSE argument register");
                        let gpr = self.conv.arguments[position];
                        self.emit(MInst::MovGx {
                            width: Width::B64,
                            dst: Reg::Phys(gpr),
                            src,
                        });
                        uses = uses.with(gpr);
                    }
                },
                | _ => self.emit(MInst::Mov {
                    width: Width::B64,
                    dst:   Reg::Phys(register),
                    src:   operand,
                }),
            }
            uses = uses.with(register);
        }
        if variadic && self.conv.kind == ConvKind::SystemV {
            // psABI §3.2.3: %al bounds the number of vector registers used.
            let (_, xmm) = assigner.registers_used();
            self.emit(MInst::Mov {
                width: Width::B32,
                dst:   Reg::Phys(PReg::Rax),
                src:   Operand::Imm(i32::try_from(xmm).expect("eight registers")),
            });
            uses = uses.with(PReg::Rax);
        }
        self.emit(MInst::Call {
            callee,
            uses,
            clobbers: self.conv.caller_saved,
        });
    }

    /// `return`: the result into `%rax` or `%xmm0` (extended as the
    /// signature says), an `sret` pointer into `%rax`, or a register result
    /// loaded from the return area.
    pub(super) fn ret(&mut self, value: Option<Value>) {
        let mut uses = PRegSet::EMPTY;
        match self.result {
            | ResultArea::None => {},
            | ResultArea::Sret(pointer) => {
                self.copy(Reg::Phys(PReg::Rax), pointer);
                uses = uses.with(PReg::Rax);
            },
            | ResultArea::Registers { area, registers }
                if matches!(registers.first, Type::I128 | Type::F128) =>
            {
                self.emit(MInst::Move128 {
                    load: true,
                    reg:  Reg::Phys(PReg::Xmm0),
                    mem:  area,
                });
                uses = uses.with(PReg::Xmm0);
            },
            | ResultArea::Registers { area, registers } if registers.first == Type::F80 => {
                // A complex `long double` returns its imaginary part in
                // `%st(1)`, so that part is pushed first.
                if registers.second == Some(Type::F80) {
                    let offset =
                        i32::try_from(registers.second_offset).expect("a part offset is small");
                    self.emit(MInst::X87Load {
                        format: X87Format::F80,
                        mem:    Mem::new(area.base, area.disp + offset),
                    });
                }
                self.emit(MInst::X87Load {
                    format: X87Format::F80,
                    mem:    area,
                });
            },
            | ResultArea::Registers { area, registers } => {
                for (offset, ty, register) in result_registers(registers) {
                    let ty = if ty == Type::Ptr { Type::I64 } else { ty };
                    let mem = Mem::new(area.base, area.disp + offset);
                    self.load(ty, Reg::Phys(register), mem);
                    uses = uses.with(register);
                }
            },
        }
        if let Some(value) = value {
            let ty = self.ty(value);
            if matches!(ty, Type::F80 | Type::F128) {
                self.return_wide(ty, value, &mut uses);
            } else if ty == Type::I128 {
                let (low, high) = self.pair(value);
                self.copy(Reg::Phys(PReg::Rax), low);
                self.copy(Reg::Phys(PReg::Rdx), high);
                uses = uses.with(PReg::Rax).with(PReg::Rdx);
            } else if ty.is_float() {
                let src = self.reg(value);
                self.copy(Reg::Phys(PReg::Xmm0), src);
                uses = uses.with(PReg::Xmm0);
            } else {
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
        }
        self.emit(MInst::Ret { uses });
    }
}

impl Selector<'_, '_, '_> {
    /// How an `i128`, `f80` or `f128` argument travels (see the module
    /// documentation).
    pub(super) fn pass_wide(&mut self, ty: Type, (low, high): Halves) -> Passed {
        match (self.conv.kind, ty) {
            | (ConvKind::SystemV, Type::I128) => Passed::Pair(low, high),
            | (ConvKind::SystemV, Type::F80) => Passed::Memory(low, high),
            | (ConvKind::SystemV, _) => Passed::Vector(low, high),
            | (ConvKind::Win64, _) => {
                let temp = self.wide_temporary();
                for (half, disp) in [(low, 0), (high, 8)] {
                    self.emit(MInst::Store {
                        width: Width::B64,
                        dst:   Mem::new(temp.base, disp),
                        src:   Operand::Reg(half),
                    });
                }
                let pointer = self.vreg();
                self.emit(MInst::Lea {
                    dst:  pointer,
                    addr: temp,
                });
                Passed::Scalar(Operand::Reg(pointer))
            },
        }
    }

    /// For a Win64 call whose result is an `f80` or `f128`: the area the
    /// callee returns it in, whose address goes first in `passed`.
    fn wide_result_area(&mut self, ty: Type, passed: &mut ArenaVec<'_, Passed>) -> Option<Mem> {
        if self.conv.kind != ConvKind::Win64 || !matches!(ty, Type::F80 | Type::F128) {
            return None;
        }
        let temp = self.wide_temporary();
        let pointer = self.vreg();
        self.emit(MInst::Lea {
            dst:  pointer,
            addr: temp,
        });
        passed.push(Passed::Scalar(Operand::Reg(pointer)));
        Some(temp)
    }

    /// Moves an `i128`, `f80` or `f128` result into `dst` right after the
    /// call: from the hidden result area `sret`, `%rdx:%rax`, `%st(0)` or
    /// `%xmm0`.
    pub(super) fn receive_wide(&mut self, ty: Type, dst: Halves, sret: Option<Mem>) {
        if let Some(area) = sret {
            self.pair_load(ty, dst, area);
            return;
        }
        match (self.conv.kind, ty) {
            | (ConvKind::SystemV, Type::I128) => {
                self.copy(dst.0, Reg::Phys(PReg::Rax));
                self.copy(dst.1, Reg::Phys(PReg::Rdx));
            },
            | (ConvKind::SystemV, Type::F80) => {
                let mem = self.scratch(0);
                self.emit(MInst::X87Store {
                    format: X87Format::F80,
                    mem,
                });
                self.pair_load(ty, dst, mem);
            },
            // System V's `f128`, Win64's `i128` from a runtime routine.
            | _ => {
                let mem = self.scratch(0);
                self.emit(MInst::Move128 {
                    load: false,
                    reg: Reg::Phys(PReg::Xmm0),
                    mem,
                });
                self.wide_load(dst, mem);
            },
        }
    }

    /// `return` of an `f80` or `f128`: through the hidden pointer on Win64
    /// (which the entry put in [`ResultArea::Sret`], and `ret` in `%rax`),
    /// in `%st(0)` or `%xmm0` on System V.
    fn return_wide(&mut self, ty: Type, value: Value, uses: &mut PRegSet) {
        match (self.conv.kind, ty) {
            | (ConvKind::Win64, _) => {
                let ResultArea::Sret(pointer) = self.result else {
                    unreachable!("the entry received the result pointer");
                };
                self.pair_store(ty, value, Mem::new(Base::Reg(pointer), 0));
            },
            | (ConvKind::SystemV, Type::F80) => self.x87_push(value, 0),
            | (ConvKind::SystemV, _) => {
                let mem = self.scratch(0);
                self.pair_store(ty, value, mem);
                self.emit(MInst::Move128 {
                    load: true,
                    reg: Reg::Phys(PReg::Xmm0),
                    mem,
                });
                *uses = uses.with(PReg::Xmm0);
            },
        }
    }

    /// Calls the runtime routine `name` (`__floattidf`, `__addtf3` …) as
    /// LLVM's code generator does, which is how the runtime libraries
    /// Clang links were built to be called.
    pub(in crate::backend::native) fn libcall(
        &mut self,
        name: &'static str,
        args: &[LibArg],
        result: LibResult,
    ) {
        let mut passed = ArenaVec::with_capacity_in(args.len() + 1, self.arena);
        let sret = match result {
            | LibResult::Wide(ty, _) => self.wide_result_area(ty, &mut passed),
            | _ => None,
        };
        for &arg in args {
            let arg = match arg {
                | LibArg::Gpr(reg) | LibArg::Xmm(reg) => Passed::Scalar(Operand::Reg(reg)),
                | LibArg::Wide(ty, halves) => self.pass_wide(ty, halves),
            };
            passed.push(arg);
        }
        self.call_sequence(Callee::Symbol(SymbolRef::Runtime(name)), &passed, false);
        match result {
            | LibResult::Gpr(dst) => self.copy(dst, Reg::Phys(PReg::Rax)),
            | LibResult::Xmm(dst) => self.copy(dst, Reg::Phys(PReg::Xmm0)),
            | LibResult::Wide(ty, dst) => self.receive_wide(ty, dst, sret),
        }
    }
}

/// Each part of a register result with its offset in the return area and
/// its register: integer parts in `%rax` then `%rdx`, SSE parts in `%xmm0`
/// then `%xmm1`, in order (psABI §3.2.3, "Returning of Values").
fn result_registers(registers: ResultRegisters) -> impl Iterator<Item = (i32, Type, PReg)> {
    let mut integer = [PReg::Rax, PReg::Rdx].into_iter();
    let mut float = [PReg::Xmm0, PReg::Xmm1].into_iter();
    registers.parts().map(move |(offset, ty)| {
        let register = if ty.is_float() {
            float.next()
        } else {
            integer.next()
        };
        (
            i32::try_from(offset).expect("a part offset is 0 or 8"),
            ty,
            register.expect("at most two parts"),
        )
    })
}

/// A `byval` size, which the support check bounds with the frame.
fn byval_size(size: u64) -> Result<u32, Construct> {
    u32::try_from(size).map_err(|_| Construct::FrameSize(size))
}

/// An alignment in bytes as `u32`; the IR bounds alignments by 2^31.
fn align(bytes: u64) -> u32 {
    u32::try_from(bytes).unwrap_or(1 << 31)
}
