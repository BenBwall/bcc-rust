//! Inline assembly: each operand of an `asm` instruction gets a fixed
//! location before selection emits it, so the allocator sees only
//! physical registers at the statement, as at a call.
//!
//! Register operands take the register their constraint names, or the
//! first free one of their class; every operand gets its own register,
//! except that an output may share an input's fixed register when it is not
//! early-clobber (GCC's `cpuid` idiom, `"=a"(eax) : "a"(leaf)`) and a tied
//! input shares its output's. `%rsp`, `%rbp` (the frame pointer) and the
//! allocator's spill scratch registers (`%r10`, `%r11`, `%xmm14`, `%xmm15`)
//! are never assigned. Inputs are
//! materialized first and then copied into their registers; a memory
//! operand in a frame object or at a defined symbol is written as that
//! operand, any other passes its address in a register (shared by operands
//! at one address, as a `+m` output and its read), and immediates are
//! written into the template. The [`MInst::InlineAsm`] reads the input
//! registers and writes the output and clobbered ones; right after it, flag
//! outputs are read with `setcc`, register outputs are copied into virtual
//! registers, and each is stored to its output pointer.
//!
//! The listing writes the template with each operand substituted
//! (`emit.rs`). The direct object encoder reads only the emitter's own
//! vocabulary, so a module with inline assembly is assembled by the bundled
//! clang, as one with file-scope assembly is.
//!
//! GNU extension: GCC manual, "Extended Asm", "Constraints for asm
//! Operands", "x86 Operand Modifiers" and "Flag Output Operands".

use super::{
    AddrBase,
    Construct,
    Selector,
    ValueKind,
    class_of,
    width,
};
use crate::{
    backend::native::{
        conv::{
            SCRATCH,
            SCRATCH_XMM,
        },
        mir::{
            AsmLocation,
            AsmMachine,
            Base,
            Cond,
            FrameObject,
            MInst,
            Mem,
            Operand,
            PReg,
            PRegSet,
            Reg,
            RegClass,
            SymbolRef,
            Width,
        },
    },
    ir::{
        ASM_CONDITIONS,
        AsmConstraint,
        AsmTemplatePiece as TemplatePiece,
        InstData,
        Value,
        asm_template_pieces as template_pieces,
    },
    util::bump::ArenaVec,
};

impl Selector<'_, '_, '_> {
    /// Selects an `asm` instruction.
    pub(super) fn inline_asm(&mut self, data: InstData) -> Result<(), Construct> {
        let InstData::InlineAsm { asm, args } = data else {
            return Err(Construct::Opcode(data.opcode()));
        };
        let descriptor = *self.module.inline_asm(asm);
        let values = self.body.value_list(args);
        let operands = descriptor.operands;
        let clobbers = descriptor.clobbers;
        let clobbered =
            PRegSet::from_bits(u32::from(clobbers.general) | ((clobbers.sse & 0xFFFF) << 16));
        if clobbered.contains(PReg::Rsp) || clobbered.contains(PReg::Rbp) {
            return Err(Construct::InlineAsm("a clobbered %rsp or %rbp"));
        }
        // The registers no operand may take, and those outputs and inputs
        // hold.
        let reserved = clobbered
            .with(PReg::Rsp)
            .with(PReg::Rbp)
            .with(SCRATCH[0])
            .with(SCRATCH[1])
            .with(SCRATCH_XMM[0])
            .with(SCRATCH_XMM[1]);
        let mut outputs = PRegSet::EMPTY;
        let mut inputs = PRegSet::EMPTY;
        let mut early = PRegSet::EMPTY;
        let mut registers = ArenaVec::with_capacity_in(operands.len(), self.arena);
        registers.resize(operands.len(), None);
        // Fixed registers first, so the free choices avoid them.
        for (index, operand) in operands.iter().enumerate() {
            let AsmConstraint::General(mask) = operand.constraint else {
                continue;
            };
            if !mask.is_power_of_two() {
                continue;
            }
            let register = PReg::ALL[mask.trailing_zeros() as usize];
            let taken = if operand.output {
                outputs.contains(register) || (operand.early_clobber && inputs.contains(register))
            } else {
                inputs.contains(register) || early.contains(register)
            };
            if reserved.contains(register) || taken {
                return Err(Construct::InlineAsm("operands that need the same register"));
            }
            registers[index] = Some(register);
            if operand.output {
                outputs = outputs.with(register);
                if operand.early_clobber {
                    early = early.with(register);
                }
            } else {
                inputs = inputs.with(register);
            }
        }
        for (index, (operand, &value)) in operands.iter().zip(values).enumerate() {
            if registers[index].is_some() {
                continue;
            }
            let class = match operand.constraint {
                | AsmConstraint::General(mask) => Some((RegClass::Gpr, mask)),
                | AsmConstraint::Sse => Some((RegClass::Xmm, u16::MAX)),
                // A frame object or a defined symbol is addressed directly.
                | AsmConstraint::Memory if self.direct_memory(value).is_some() => None,
                // Operands at one address (a `+m` output and its read)
                // share the register that holds it.
                | AsmConstraint::Memory
                    if let Some(earlier) = shared_address(operands, values, index) =>
                {
                    registers[index] = registers[earlier];
                    continue;
                },
                // The address of a memory operand.
                | AsmConstraint::Memory => Some((RegClass::Gpr, AsmConstraint::ANY_GENERAL)),
                | AsmConstraint::Tied(output) => {
                    let register = registers[output as usize]
                        .ok_or(Construct::InlineAsm("a tied input without a register"))?;
                    inputs = inputs.with(register);
                    registers[index] = Some(register);
                    continue;
                },
                | AsmConstraint::Immediate | AsmConstraint::Flag(_) => None,
            };
            let Some((class, mask)) = class else { continue };
            let order = match class {
                | RegClass::Gpr => self.conv.allocation,
                | RegClass::Xmm => self.conv.float_allocation,
            };
            let free = |register: &&PReg| {
                let bit = if class == RegClass::Gpr {
                    mask & (1 << register.index()) != 0
                } else {
                    true
                };
                bit && !reserved.contains(**register)
                    && !outputs.contains(**register)
                    && !inputs.contains(**register)
            };
            let register = *order.iter().find(free).ok_or(Construct::InlineAsm(
                "more register operands than free registers",
            ))?;
            registers[index] = Some(register);
            if operand.output && operand.constraint != AsmConstraint::Memory {
                outputs = outputs.with(register);
            } else {
                inputs = inputs.with(register);
            }
        }
        // Inputs and addresses are materialized before any register is set.
        let mut moves = ArenaVec::with_capacity_in(operands.len(), self.arena);
        let mut locations = ArenaVec::with_capacity_in(operands.len(), self.arena);
        for (index, (operand, &value)) in operands.iter().zip(values).enumerate() {
            let register = registers[index];
            let location = match operand.constraint {
                | AsmConstraint::Memory => match self.direct_memory(value) {
                    | Some(mem) => AsmLocation::Mem(mem),
                    | None => {
                        let register = register.expect("a memory operand has an address register");
                        if shared_address(operands, values, index).is_none() {
                            moves.push((register, Operand::Reg(self.reg(value))));
                        }
                        AsmLocation::Mem(Mem::new(Base::Reg(Reg::Phys(register)), 0))
                    },
                },
                | AsmConstraint::Immediate => self.immediate(value)?,
                | AsmConstraint::Flag(_) => AsmLocation::None,
                | _ => {
                    let register = register.expect("a register operand has a register");
                    if !operand.output {
                        let source = if class_of(operand.ty) == RegClass::Xmm {
                            Operand::Reg(self.reg(value))
                        } else {
                            self.operand(value, Width::B64)
                        };
                        moves.push((register, source));
                    }
                    AsmLocation::Reg(register, width(operand.ty))
                },
            };
            locations.push(location);
        }
        let template = self.arena.alloc_str(descriptor.template);
        check_template(template, &locations)?;
        let index = u32::try_from(self.func.asm.len()).expect("asm statements fit u32");
        self.func.asm.push(AsmMachine {
            template,
            operands: locations.leak(),
        });
        for &(register, source) in &moves {
            if let Operand::Reg(Reg::Virt(vreg)) = source {
                self.func.hint(vreg, Reg::Phys(register));
            }
            let inst = if register.class() == RegClass::Xmm {
                MInst::XMov {
                    width: Width::B64,
                    dst:   Reg::Phys(register),
                    src:   source,
                }
            } else {
                MInst::Mov {
                    width: Width::B64,
                    dst:   Reg::Phys(register),
                    src:   source,
                }
            };
            self.emit(inst);
        }
        let used = moves
            .iter()
            .fold(PRegSet::EMPTY, |set, &(register, _)| set.with(register));
        let defined = registers
            .iter()
            .zip(operands)
            .filter(|(_, operand)| operand.output && operand.constraint != AsmConstraint::Memory)
            .filter_map(|(&register, _)| register)
            .fold(clobbered, PRegSet::with);
        self.emit(MInst::InlineAsm {
            index,
            uses: used,
            defs: defined,
        });
        // Flags first: nothing may run between the statement and `setcc`.
        let mut results = ArenaVec::with_capacity_in(operands.len(), self.arena);
        for (index, (operand, &pointer)) in operands.iter().zip(values).enumerate() {
            if !operand.output {
                continue;
            }
            match operand.constraint {
                | AsmConstraint::Flag(condition) => {
                    let dst = self.vreg();
                    self.set_condition(condition_code(condition), dst);
                    results.push((dst, pointer, operand.ty));
                },
                | AsmConstraint::Memory => {},
                | _ => {
                    let register = registers[index].expect("a register output has a register");
                    let dst = self.vreg_for(operand.ty);
                    self.copy(dst, Reg::Phys(register));
                    results.push((dst, pointer, operand.ty));
                },
            }
        }
        for (register, pointer, ty) in results {
            let mem = self.address(pointer);
            let inst = if ty.is_float() {
                MInst::XStore {
                    width: width(ty),
                    dst:   mem,
                    src:   register,
                }
            } else {
                MInst::Store {
                    width: width(ty),
                    dst:   mem,
                    src:   Operand::Reg(register),
                }
            };
            self.emit(inst);
        }
        Ok(())
    }

    /// The memory operand of a frame object or a symbol the function
    /// addresses directly (`%rip`-relative), which needs no register.
    fn direct_memory(&self, value: Value) -> Option<Mem> {
        match self.kind(value) {
            | ValueKind::Addr(AddrBase::Slot(slot), disp) if disp.abs() < MAX_DIRECT =>
                Some(Mem::new(
                    Base::Frame(FrameObject::Slot(slot)),
                    i32::try_from(disp).ok()?,
                )),
            | ValueKind::Addr(AddrBase::Symbol(symbol), disp)
                if disp.abs() < MAX_DIRECT && self.direct(symbol) =>
                Some(Mem::new(
                    Base::Symbol(SymbolRef::Ir(symbol)),
                    i32::try_from(disp).ok()?,
                )),
            | _ => None,
        }
    }

    /// The location of an immediate: an integer, or a symbol's address
    /// with an offset.
    fn immediate(&mut self, value: Value) -> Result<AsmLocation, Construct> {
        let ty = self.ty(value);
        match self.kind(value) {
            | ValueKind::Const(bits) => {
                let shift = 64 - ty.bits();
                Ok(AsmLocation::Imm((bits.cast_signed() << shift) >> shift))
            },
            | ValueKind::Addr(AddrBase::Symbol(symbol), disp) =>
                Ok(AsmLocation::Symbol(SymbolRef::Ir(symbol), disp)),
            | _ => Err(Construct::InlineAsm(
                "an immediate that is not a constant or symbol",
            )),
        }
    }
}

/// The largest displacement a direct memory operand folds, leaving room
/// for the `H` modifier's eight bytes and the frame offset.
const MAX_DIRECT: i64 = 1 << 30;

/// An earlier memory operand at the same address as operand `index`.
fn shared_address(
    operands: &[crate::ir::AsmOperand],
    values: &[Value],
    index: usize,
) -> Option<usize> {
    (0..index).find(|&earlier| {
        operands[earlier].constraint == AsmConstraint::Memory && values[earlier] == values[index]
    })
}

/// Checks that the native listing can write every operand reference of
/// the template: the modifiers each location takes, and no reference to a
/// flag output, which has no location in the template.
fn check_template(template: &str, locations: &[AsmLocation]) -> Result<(), Construct> {
    for piece in template_pieces(template) {
        let Ok(TemplatePiece::Operand { operand, modifier }) = piece else {
            if piece.is_err() {
                return Err(Construct::InlineAsm("a malformed template"));
            }
            continue;
        };
        let location = locations
            .get(operand as usize)
            .copied()
            .unwrap_or(AsmLocation::None);
        if !location.writes(modifier) {
            return Err(Construct::InlineAsm(
                "an operand modifier the native listing does not write",
            ));
        }
    }
    Ok(())
}

/// The condition code of a flag output.
fn condition_code(condition: u8) -> Cond {
    match ASM_CONDITIONS[usize::from(condition)] {
        | "a" | "nbe" => Cond::A,
        | "ae" | "nb" | "nc" => Cond::Ae,
        | "b" | "c" | "nae" => Cond::B,
        | "be" | "na" => Cond::Be,
        | "e" | "z" => Cond::E,
        | "g" | "nle" => Cond::G,
        | "ge" | "nl" => Cond::Ge,
        | "l" | "nge" => Cond::L,
        | "le" | "ng" => Cond::Le,
        | "ne" | "nz" => Cond::Ne,
        | "no" => Cond::No,
        | "np" | "po" => Cond::Np,
        | "ns" => Cond::Ns,
        | "o" => Cond::O,
        | "p" | "pe" => Cond::P,
        | _ => Cond::S,
    }
}
