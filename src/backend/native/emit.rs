//! AT&T assembly text for LLVM's integrated assembler (bundled clang).
//!
//! Functions get a prologue that pushes the callee-saved registers they
//! write and allocates the frame, and an epilogue before every `ret`, with
//! unwind directives the assembler turns into tables: `.cfi_*` into ELF's
//! `.eh_frame` (LSB 5.0 §10.6), `.seh_*` into COFF's `.pdata` and `.xdata`
//! (Microsoft Learn, "x64 exception handling"). Each epilogue has the shape
//! the Windows unwinder recognizes ("x64 prolog and epilog": `add rsp`,
//! `pop`s, `ret`); on ELF, `.cfi_remember_state` and `.cfi_restore_state`
//! keep the rules after an early return correct.
//!
//! A Windows frame of 4096 bytes or more probes its pages through
//! `___chkstk_ms` (MinGW) or `__chkstk` (MSVC) before moving `%rsp`
//! ("x64 stack usage"), as Clang does.

use std::fmt::{
    self,
    Write,
};

use super::{
    frame::Frame,
    mir::{
        Base,
        Callee,
        MBlock,
        MFunction,
        MInst,
        Mem,
        Operand,
        PReg,
        Reg,
        ShiftAmount,
        SymbolRef,
        Width,
    },
};
use crate::{
    ir::{
        Entity,
        FuncId,
        GlobalInit,
        Linkage,
        Module,
        Symbol,
    },
    target::Target,
};

/// Writes one module's assembly.
pub(crate) struct Emitter<'o, 'm, 'ir> {
    pub(crate) module: &'m Module<'ir>,
    pub(crate) out:    &'o mut dyn Write,
    elf:               bool,
    target:            Target,
}

impl<'o, 'm, 'ir> Emitter<'o, 'm, 'ir> {
    pub(crate) fn new(module: &'m Module<'ir>, target: Target, out: &'o mut dyn Write) -> Self {
        Self {
            module,
            out,
            elf: matches!(target, Target::LinuxGnu | Target::LinuxMusl),
            target,
        }
    }

    /// The file header.
    pub(crate) fn header(&mut self) -> fmt::Result {
        writeln!(self.out, "\t.text")
    }

    /// One function: label, prologue, blocks, and the end of its unwind
    /// information.
    pub(crate) fn function(
        &mut self,
        id: FuncId,
        func: &MFunction<'_>,
        frame: &Frame<'_>,
    ) -> fmt::Result {
        let function = self.module.function(id);
        let name = Name(function.name);
        let external = function.linkage == Linkage::External;
        writeln!(self.out, "\t.text")?;
        if self.elf {
            if external {
                writeln!(self.out, "\t.globl\t{name}")?;
            }
            writeln!(self.out, "\t.p2align\t4, 0x90")?;
            writeln!(self.out, "\t.type\t{name},@function")?;
            writeln!(self.out, "{name}:")?;
            writeln!(self.out, "\t.cfi_startproc")?;
        } else {
            let class = if external { 2 } else { 3 };
            writeln!(
                self.out,
                "\t.def\t{name};\n\t.scl\t{class};\n\t.type\t32;\n\t.endef"
            )?;
            if external {
                writeln!(self.out, "\t.globl\t{name}")?;
            }
            writeln!(self.out, "\t.p2align\t4, 0x90")?;
            writeln!(self.out, "{name}:")?;
            writeln!(self.out, "\t.seh_proc\t{name}")?;
        }
        self.prologue(frame)?;
        for (index, block) in func.blocks.iter().enumerate() {
            let block_id = MBlock::new(index);
            writeln!(self.out, "{}:", Label(id, block_id))?;
            let next = (index + 1 < func.blocks.len()).then(|| MBlock::new(index + 1));
            for inst in &block.insts {
                self.instruction(id, inst, frame, next)?;
            }
        }
        if self.elf {
            writeln!(self.out, "\t.cfi_endproc")?;
            writeln!(self.out, "\t.size\t{name}, .-{name}")
        } else {
            writeln!(self.out, "\t.seh_endproc")
        }
    }

    fn prologue(&mut self, frame: &Frame<'_>) -> fmt::Result {
        let mut cfa = 8;
        for &register in &frame.saved {
            writeln!(self.out, "\tpushq\t%{}", register.name(Width::B64))?;
            cfa += 8;
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_offset {cfa}")?;
                writeln!(
                    self.out,
                    "\t.cfi_offset %{}, -{cfa}",
                    register.name(Width::B64)
                )?;
            } else {
                writeln!(self.out, "\t.seh_pushreg %{}", register.name(Width::B64))?;
            }
        }
        if frame.size > 0 {
            if !self.elf && frame.size >= 4096 {
                let probe = if self.target == Target::WindowsMsvc {
                    "__chkstk"
                } else {
                    "___chkstk_ms"
                };
                writeln!(self.out, "\tmovl\t${}, %eax", frame.size)?;
                writeln!(self.out, "\tcallq\t{probe}")?;
                writeln!(self.out, "\tsubq\t%rax, %rsp")?;
            } else {
                writeln!(self.out, "\tsubq\t${}, %rsp", frame.size)?;
            }
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_offset {}", frame.cfa_offset())?;
            } else {
                writeln!(self.out, "\t.seh_stackalloc {}", frame.size)?;
            }
        }
        if !self.elf {
            writeln!(self.out, "\t.seh_endprologue")?;
        }
        Ok(())
    }

    fn epilogue(&mut self, frame: &Frame<'_>) -> fmt::Result {
        if self.elf {
            writeln!(self.out, "\t.cfi_remember_state")?;
        }
        let mut cfa = frame.cfa_offset();
        if frame.size > 0 {
            writeln!(self.out, "\taddq\t${}, %rsp", frame.size)?;
            cfa -= frame.size;
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_offset {cfa}")?;
            }
        }
        for &register in frame.saved.iter().rev() {
            writeln!(self.out, "\tpopq\t%{}", register.name(Width::B64))?;
            cfa -= 8;
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_offset {cfa}")?;
            }
        }
        writeln!(self.out, "\tretq")?;
        if self.elf {
            writeln!(self.out, "\t.cfi_restore_state")?;
        }
        Ok(())
    }

    /// One instruction; `next` is the block laid out after this one, which
    /// a branch reaches by falling through.
    fn instruction(
        &mut self,
        func: FuncId,
        inst: &MInst,
        frame: &Frame<'_>,
        next: Option<MBlock>,
    ) -> fmt::Result {
        let out = &mut *self.out;
        let at = |operand: Operand, width: Width| Op {
            operand,
            width,
            frame,
            emitter: (self.module, self.elf),
        };
        let reg = |reg: Reg, width: Width| RegName(reg.phys(), width);
        match *inst {
            | MInst::Mov { width, dst, src } => writeln!(
                out,
                "\tmov{}\t{}, {}",
                width.suffix(),
                at(src, width),
                reg(dst, width)
            ),
            | MInst::MovImm64 { dst, imm } =>
                writeln!(out, "\tmovabsq\t${imm}, {}", reg(dst, Width::B64)),
            | MInst::Store { width, dst, src } => writeln!(
                out,
                "\tmov{}\t{}, {}",
                width.suffix(),
                at(src, width),
                at(Operand::Mem(dst), width)
            ),
            | MInst::Lea { dst, addr } => writeln!(
                out,
                "\tleaq\t{}, {}",
                at(Operand::Mem(addr), Width::B64),
                reg(dst, Width::B64)
            ),
            | MInst::Extend {
                signed,
                from,
                to,
                dst,
                src,
            } => {
                let (mnemonic, dst_width) = match (signed, from, to) {
                    | (false, Width::B8, _) => ("movzbl", Width::B32),
                    | (false, Width::B16, _) => ("movzwl", Width::B32),
                    | (false, _, _) => ("movl", Width::B32),
                    | (true, Width::B8, Width::B64) => ("movsbq", Width::B64),
                    | (true, Width::B8, _) => ("movsbl", Width::B32),
                    | (true, Width::B16, Width::B64) => ("movswq", Width::B64),
                    | (true, Width::B16, _) => ("movswl", Width::B32),
                    | (true, _, _) => ("movslq", Width::B64),
                };
                writeln!(
                    out,
                    "\t{mnemonic}\t{}, {}",
                    at(src, from),
                    reg(dst, dst_width)
                )
            },
            | MInst::Alu {
                op,
                width,
                dst,
                src,
            } => writeln!(
                out,
                "\t{}{}\t{}, {}",
                op.name(),
                width.suffix(),
                at(src, width),
                reg(dst, width)
            ),
            | MInst::Imul3 {
                width,
                dst,
                src,
                imm,
            } => writeln!(
                out,
                "\timul{}\t${imm}, {}, {}",
                width.suffix(),
                reg(src, width),
                reg(dst, width)
            ),
            | MInst::Cmp { width, lhs, rhs } => writeln!(
                out,
                "\tcmp{}\t{}, {}",
                width.suffix(),
                at(rhs, width),
                reg(lhs, width)
            ),
            | MInst::Test { width, lhs, rhs } => writeln!(
                out,
                "\ttest{}\t{}, {}",
                width.suffix(),
                reg(rhs, width),
                reg(lhs, width)
            ),
            | MInst::Shift {
                op,
                width,
                dst,
                amount,
            } => match amount {
                | ShiftAmount::Imm(count) => writeln!(
                    out,
                    "\t{}{}\t${count}, {}",
                    op.name(),
                    width.suffix(),
                    reg(dst, width)
                ),
                | ShiftAmount::Cl => writeln!(
                    out,
                    "\t{}{}\t%cl, {}",
                    op.name(),
                    width.suffix(),
                    reg(dst, width)
                ),
            },
            | MInst::Unary { op, width, dst } => writeln!(
                out,
                "\t{}{}\t{}",
                op.name(),
                width.suffix(),
                reg(dst, width)
            ),
            | MInst::BitScan {
                reverse,
                width,
                dst,
                src,
            } => writeln!(
                out,
                "\t{}{}\t{}, {}",
                if reverse { "bsr" } else { "bsf" },
                width.suffix(),
                reg(src, width),
                reg(dst, width)
            ),
            | MInst::SignExtendAx { width } => writeln!(
                out,
                "\t{}",
                if width == Width::B64 { "cqto" } else { "cltd" }
            ),
            | MInst::Div {
                signed,
                width,
                divisor,
            } => writeln!(
                out,
                "\t{}{}\t{}",
                if signed { "idiv" } else { "div" },
                width.suffix(),
                reg(divisor, width)
            ),
            | MInst::MulWide { width, src } =>
                writeln!(out, "\tmul{}\t{}", width.suffix(), reg(src, width)),
            | MInst::Setcc { cond, dst } =>
                writeln!(out, "\tset{}\t{}", cond.name(), reg(dst, Width::B8)),
            | MInst::Cmov {
                cond,
                width,
                dst,
                src,
            } => writeln!(
                out,
                "\tcmov{}{}\t{}, {}",
                cond.name(),
                width.suffix(),
                reg(src, width),
                reg(dst, width)
            ),
            | MInst::Call { callee, .. } => match callee {
                | Callee::Symbol(symbol) => {
                    let name = symbol_name(self.module, symbol);
                    let plt = self.elf && !defined(self.module, symbol);
                    writeln!(out, "\tcallq\t{name}{}", if plt { "@PLT" } else { "" })
                },
                | Callee::Reg(target) => writeln!(out, "\tcallq\t*{}", reg(target, Width::B64)),
            },
            | MInst::RepMovsb => writeln!(out, "\trep movsb"),
            | MInst::RepStosb => writeln!(out, "\trep stosb"),
            | MInst::Jmp { target } =>
                if Some(target) == next {
                    Ok(())
                } else {
                    writeln!(out, "\tjmp\t{}", Label(func, target))
                },
            | MInst::Jcc {
                cond,
                taken,
                not_taken,
            } =>
                if Some(not_taken) == next {
                    writeln!(out, "\tj{}\t{}", cond.name(), Label(func, taken))
                } else if Some(taken) == next {
                    writeln!(
                        out,
                        "\tj{}\t{}",
                        cond.negated().name(),
                        Label(func, not_taken)
                    )
                } else {
                    writeln!(out, "\tj{}\t{}", cond.name(), Label(func, taken))?;
                    writeln!(out, "\tjmp\t{}", Label(func, not_taken))
                },
            | MInst::Ret { .. } => self.epilogue(frame),
            | MInst::Ud2 => writeln!(out, "\tud2"),
        }
    }

    /// Every defined global, in its section, and the file trailer.
    pub(crate) fn globals(&mut self) -> fmt::Result {
        for (_, global) in self.module.globals() {
            let Some(init) = global.init else { continue };
            let name = Name(global.name);
            let section = match (global.constant, init) {
                | (true, _) if self.elf => "\t.section\t.rodata",
                | (true, _) => "\t.section\t.rdata,\"dr\"",
                | (false, GlobalInit::Zero) => "\t.bss",
                | (false, GlobalInit::Bytes { .. }) => "\t.data",
            };
            writeln!(self.out, "{section}")?;
            if global.linkage == Linkage::External {
                writeln!(self.out, "\t.globl\t{name}")?;
            }
            writeln!(self.out, "\t.p2align\t{}", global.align.log2())?;
            if self.elf {
                writeln!(self.out, "\t.type\t{name},@object")?;
            }
            writeln!(self.out, "{name}:")?;
            let mut written = 0_u64;
            if let GlobalInit::Bytes { bytes, relocations } = init {
                let mut offset = 0_usize;
                let mut relocations = relocations.iter().peekable();
                while offset < bytes.len() {
                    if let Some(relocation) =
                        relocations.next_if(|relocation| relocation.offset == offset as u64)
                    {
                        let target = symbol_name(self.module, SymbolRef::Ir(relocation.symbol));
                        write!(self.out, "\t.quad\t{target}")?;
                        if relocation.addend != 0 {
                            write!(self.out, "{:+}", relocation.addend)?;
                        }
                        writeln!(self.out)?;
                        offset += 8;
                        continue;
                    }
                    let end = relocations
                        .peek()
                        .map_or(bytes.len(), |relocation| {
                            usize::try_from(relocation.offset).unwrap_or(usize::MAX)
                        })
                        .min(bytes.len())
                        .min(offset + 16);
                    write!(self.out, "\t.byte\t")?;
                    for (index, byte) in bytes[offset..end].iter().enumerate() {
                        let separator = if index == 0 { "" } else { "," };
                        write!(self.out, "{separator}{byte}")?;
                    }
                    writeln!(self.out)?;
                    offset = end;
                }
                written = bytes.len() as u64;
            }
            if global.size > written {
                writeln!(self.out, "\t.zero\t{}", global.size - written)?;
            }
            if self.elf {
                writeln!(self.out, "\t.size\t{name}, {}", global.size)?;
            }
        }
        if self.elf {
            writeln!(self.out, "\t.section\t.note.GNU-stack,\"\",@progbits")?;
        }
        Ok(())
    }
}

/// Whether the module defines `symbol`; runtime symbols never are.
fn defined(module: &Module<'_>, symbol: SymbolRef) -> bool {
    match symbol {
        | SymbolRef::Ir(Symbol::Function(func)) => module.function(func).body.is_some(),
        | SymbolRef::Ir(Symbol::Global(global)) => module.global(global).init.is_some(),
        | SymbolRef::Runtime(_) => false,
    }
}

/// The assembler spelling of a symbol.
fn symbol_name<'m>(module: &Module<'m>, symbol: SymbolRef) -> Name<'m> {
    match symbol {
        | SymbolRef::Ir(symbol) => Name(module.symbol_name(symbol)),
        | SymbolRef::Runtime(name) => Name(name),
    }
}

/// A symbol name, quoted unless it is a plain identifier.
#[derive(Clone, Copy)]
struct Name<'a>(&'a str);

impl fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plain = self.0.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || byte == b'_'
                || byte == b'.'
                || byte == b'$'
                || (index > 0 && byte.is_ascii_digit())
        });
        if plain {
            f.write_str(self.0)
        } else {
            write!(f, "\"{}\"", self.0)
        }
    }
}

/// A block's label.
struct Label(FuncId, MBlock);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".LBB{}_{}", self.0.index(), self.1.index())
    }
}

/// A register at a width.
struct RegName(PReg, Width);

impl fmt::Display for RegName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0.name(self.1))
    }
}

/// An operand at a width, with what memory operands need to resolve.
struct Op<'f, 'm, 'ir> {
    operand: Operand,
    width:   Width,
    frame:   &'f Frame<'f>,
    emitter: (&'m Module<'ir>, bool),
}

impl fmt::Display for Op<'_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.operand {
            | Operand::Reg(reg) => write!(f, "{}", RegName(reg.phys(), self.width)),
            | Operand::Imm(imm) => write!(f, "${imm}"),
            | Operand::Mem(Mem { base, disp }) => {
                let (module, _) = self.emitter;
                match base {
                    | Base::Reg(reg) => {
                        if disp != 0 {
                            write!(f, "{disp}")?;
                        }
                        write!(f, "({})", RegName(reg.phys(), Width::B64))
                    },
                    | Base::Frame(object) => {
                        let offset = self.frame.offset(object) + i64::from(disp);
                        write!(f, "{offset}(%rsp)")
                    },
                    | Base::Symbol(symbol) => {
                        write!(f, "{}", symbol_name(module, symbol))?;
                        if disp != 0 {
                            write!(f, "{disp:+}")?;
                        }
                        write!(f, "(%rip)")
                    },
                    | Base::Got(symbol) =>
                        write!(f, "{}@GOTPCREL(%rip)", symbol_name(module, symbol)),
                }
            },
        }
    }
}
