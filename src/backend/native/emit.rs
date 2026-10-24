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
//! ("x64 stack usage"), as Clang does. Win64's callee-saved SSE registers
//! are saved with `movaps` after the allocation (`.seh_savexmm`). A function
//! whose stack moves sets `%rbp` to `%rsp` after the allocation
//! (`.cfi_def_cfa_register`, `.seh_setframe %rbp, 0`) and its epilogue
//! starts with `lea size(%rbp), %rsp`, the frame-pointer epilogue form. A
//! System V leaf frame in the red zone (`frame.rs`) needs neither the `sub`
//! nor the `add`, and its CFA stays where the pushes left it.
//!
//! Each function's read-only constants follow it in `.rodata` (`.rdata` on
//! COFF), 16 bytes each so that `xorps` may read them.
//!
//! An IR block label is the local symbol `.LBA<function>_<label>` at the
//! block its address names; `block_addr` takes it RIP-relative, and global
//! relocations write it (`.quad`) or a difference of two such labels in one
//! function (`.long a-b`), as Clang does for GCC's "Labels as Values".
//! Thread-local globals go in `.tdata`/`.tbss` on ELF and `.tls$` on COFF.
//!
//! The GNU symbol attributes (GCC manual, "Common Function Attributes" and
//! "Common Variable Attributes") become what Clang writes: `.weak` for weak
//! definitions and declarations, `name = target` for aliases, the requested
//! section and alignment, `.hidden`/`.protected` on ELF, and constructors
//! and destructors in `.init_array`/`.fini_array` (ELF), `.ctors`/`.dtors`
//! (MinGW) or `.CRT$XC*`/`.CRT$XT*` (MSVC), suffixed by priority. COFF
//! reaches a weak declaration, and MinGW data the module does not define,
//! through a `.refptr` stub. `used` needs nothing, since every symbol is
//! emitted.

use std::fmt::{
    self,
    Write,
};

use super::{
    defines,
    frame::Frame,
    imported,
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
    through_stub,
    weak_declaration,
};
use crate::{
    ir::{
        DllStorage,
        Entity,
        FuncId,
        GlobalInit,
        InitKind,
        Linkage,
        Module,
        Relocation,
        RelocationTarget,
        Symbol,
        Visibility,
    },
    target::{
        Target,
        sections::{
            ElfSectionKind,
            family as elf_section_family,
        },
    },
    util::bump::{
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// The comment that opens the file-scope assembly in a listing, as Clang
/// writes it.
pub(crate) const MODULE_ASM_START: &str = "# Start of file scope inline assembly";

/// Writes one module's assembly.
pub(crate) struct Emitter<'o, 'm, 'ir> {
    pub(crate) module: &'m Module<'ir>,
    pub(crate) out:    &'o mut dyn Write,
    elf:               bool,
    target:            Target,
    /// The symbols code reaches through a `.refptr` stub on COFF, whose
    /// stubs follow the data.
    stubs:             ArenaSet<'o, Symbol>,
}

impl<'o, 'm, 'ir> Emitter<'o, 'm, 'ir> {
    pub(crate) fn new(
        module: &'m Module<'ir>,
        target: Target,
        out: &'o mut dyn Write,
        arena: &'o Bump,
    ) -> Self {
        Self {
            module,
            out,
            elf: matches!(target, Target::LinuxGnu | Target::LinuxMusl),
            target,
            stubs: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        }
    }

    /// Validate named BSS contents before either output path writes a file.
    /// GNU extension: GCC Common Variable Attributes, `section`.
    pub(crate) fn header(&mut self) -> Result<(), super::NativeError<'m>> {
        if self.elf {
            for (_, function) in self.module.functions() {
                if function.body.is_some()
                    && let Some(section) = function.attributes.section
                    && elf_section_type(section) == "nobits"
                {
                    return Err(SectionError {
                        section,
                        fixups: false,
                    }
                    .into());
                }
            }
            for (_, global) in self.module.globals() {
                if let Some(section) = global.attributes.section
                    && elf_section_type(section) == "nobits"
                    && let Some(GlobalInit::Bytes { bytes, relocations }) = global.init
                    && (!relocations.is_empty() || bytes.iter().any(|&byte| byte != 0))
                {
                    return Err(SectionError {
                        section,
                        fixups: !relocations.is_empty(),
                    }
                    .into());
                }
            }
        }
        writeln!(self.out, "\t.text")?;
        // File-scope assembly follows verbatim, in source order and before
        // every function and object as Clang places it, then AT&T syntax
        // again. GNU extension: GCC manual, "Basic Asm".
        let blocks = self.module.module_asm();
        if blocks.is_empty() {
            return Ok(());
        }
        writeln!(self.out, "{MODULE_ASM_START}")?;
        for text in blocks {
            self.out.write_str(text)?;
            if !text.ends_with('\n') {
                writeln!(self.out)?;
            }
        }
        writeln!(self.out, "# End of file scope inline assembly")?;
        writeln!(self.out, "\t.att_syntax")?;
        Ok(())
    }

    /// One function: label, prologue, blocks, and the end of its unwind
    /// information. `arena` holds the function's scratch data.
    pub(crate) fn function(
        &mut self,
        id: FuncId,
        func: &MFunction<'_>,
        frame: &Frame<'_>,
        arena: &Bump,
    ) -> fmt::Result {
        let function = self.module.function(id);
        let name = Name(function.name);
        let align = function.align.map_or(4, |align| align.log2().max(4));
        match function.attributes.section {
            | _ if function.linkage.comdat() => self.comdat_section(
                function.attributes.section,
                ".text",
                if self.elf { "ax" } else { "xr" },
                false,
                name,
            )?,
            | Some(section) if self.elf => {
                let kind = elf_section_type(section);
                let flags = elf_section_flags(section, true, false);
                writeln!(
                    self.out,
                    "\t.section\t{},\"{flags}\",@{kind}",
                    Name(section)
                )?;
            },
            | Some(section) => writeln!(self.out, "\t.section\t{},\"xr\"", Name(section))?,
            | None => writeln!(self.out, "\t.text")?,
        }
        if self.elf {
            self.binding(name, function.linkage, function.attributes.visibility)?;
            writeln!(self.out, "\t.p2align\t{align}, 0x90")?;
            writeln!(self.out, "\t.type\t{name},@function")?;
            writeln!(self.out, "{name}:")?;
            writeln!(self.out, "\t.cfi_startproc")?;
        } else {
            self.coff_function(name, function.linkage)?;
            self.binding(name, function.linkage, function.attributes.visibility)?;
            writeln!(self.out, "\t.p2align\t{align}, 0x90")?;
            writeln!(self.out, "{name}:")?;
            writeln!(self.out, "\t.seh_proc\t{name}")?;
        }
        if !self.elf {
            self.stubs.extend(func.indirect_symbols.iter().copied());
        }
        self.prologue(frame)?;
        let aligned = aligned_blocks(func, arena);
        for (index, block) in func.blocks.iter().enumerate() {
            let block_id = MBlock::new(index);
            if aligned[index] {
                writeln!(self.out, "{LOOP_ALIGNMENT}")?;
            }
            writeln!(self.out, "{}:", Label(id, block_id))?;
            for &(label, _) in func.labels.iter().filter(|&&(_, at)| at == block_id) {
                writeln!(self.out, "{}:", BlockLabel(id, label))?;
            }
            let next = (index + 1 < func.blocks.len()).then(|| MBlock::new(index + 1));
            for inst in &block.insts {
                self.instruction(id, inst, frame, next)?;
            }
        }
        if self.elf {
            writeln!(self.out, "\t.cfi_endproc")?;
            writeln!(self.out, "\t.size\t{name}, .-{name}")?;
        } else {
            writeln!(self.out, "\t.seh_endproc")?;
        }
        if !func.constants.is_empty() {
            let section = if self.elf {
                "\t.section\t.rodata"
            } else {
                "\t.section\t.rdata,\"dr\""
            };
            writeln!(self.out, "{section}\n\t.p2align\t4")?;
            for (index, &bits) in func.constants.iter().enumerate() {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "The constant is written as two 64-bit halves."
                )]
                let (low, high) = (bits as u64, (bits >> 64) as u64);
                writeln!(
                    self.out,
                    "{}:\n\t.quad\t{low:#x}\n\t.quad\t{high:#x}",
                    ConstantLabel(id, index)
                )?;
            }
        }
        Ok(())
    }

    /// The COMDAT section of a `selectany` or `linkonce` definition `name`:
    /// the requested section or `base`, keyed by the symbol, as Clang writes
    /// it. ELF names a group (`.text.f`, `"axG"`, `comdat`); COFF selects any
    /// one definition (`discard`), in a section MinGW suffixes with `$f`.
    /// ELF: System V gABI, "COMDAT Group"; COFF: Microsoft Learn, "PE
    /// Format", COMDAT sections.
    fn comdat_section(
        &mut self,
        section: Option<&str>,
        base: &str,
        flags: &str,
        nobits: bool,
        name: Name<'_>,
    ) -> fmt::Result {
        let suffix = match self.target {
            | Target::LinuxGnu | Target::LinuxMusl => ".",
            | Target::WindowsGnu => "$",
            | Target::WindowsMsvc => "",
        };
        write!(self.out, "\t.section\t")?;
        match section {
            | Some(section) => write!(self.out, "{}", Name(section))?,
            | None if suffix.is_empty() => write!(self.out, "{base}")?,
            | None if name.plain() => write!(self.out, "{base}{suffix}{}", name.0)?,
            | None => write!(self.out, "\"{base}{suffix}{}\"", name.0)?,
        }
        if self.elf {
            let kind = section.map_or(if nobits { "nobits" } else { "progbits" }, elf_section_type);
            let flags = section.map_or(flags, |section| {
                elf_section_flags(section, flags.contains('x'), !flags.contains('w'))
            });
            writeln!(self.out, ",\"{flags}G\",@{kind},{name},comdat")
        } else {
            writeln!(self.out, ",\"{flags}\",discard,{name}")
        }
    }

    /// A symbol's binding and, on ELF, its visibility: `.globl` for an
    /// external one, `.weak` for a weak one, nothing for an internal one.
    /// A COMDAT definition is global on COFF, where its section selects one
    /// copy, and weak on ELF, as Clang writes it.
    fn binding(&mut self, name: Name<'_>, linkage: Linkage, visibility: Visibility) -> fmt::Result {
        match linkage {
            | Linkage::Weak => writeln!(self.out, "\t.weak\t{name}")?,
            | Linkage::SelectAny | Linkage::LinkOnce if self.elf =>
                writeln!(self.out, "\t.weak\t{name}")?,
            | Linkage::External | Linkage::SelectAny | Linkage::LinkOnce =>
                writeln!(self.out, "\t.globl\t{name}")?,
            | Linkage::Internal => {},
        }
        match visibility {
            | Visibility::Hidden if self.elf => writeln!(self.out, "\t.hidden\t{name}"),
            | Visibility::Protected if self.elf => writeln!(self.out, "\t.protected\t{name}"),
            | _ => Ok(()),
        }
    }

    /// The COFF symbol record of a function: external storage class 2, or
    /// static 3, and the function type 32.
    fn coff_function(&mut self, name: Name<'_>, linkage: Linkage) -> fmt::Result {
        let class = if linkage == Linkage::Internal { 3 } else { 2 };
        writeln!(
            self.out,
            "\t.def\t{name};\n\t.scl\t{class};\n\t.type\t32;\n\t.endef"
        )
    }

    fn prologue(&mut self, frame: &Frame<'_>) -> fmt::Result {
        let size = frame.allocation();
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
        if size > 0 {
            if !self.elf && size >= 4096 {
                let probe = if self.target == Target::WindowsMsvc {
                    "__chkstk"
                } else {
                    "___chkstk_ms"
                };
                writeln!(self.out, "\tmovl\t${size}, %eax")?;
                writeln!(self.out, "\tcallq\t{probe}")?;
                writeln!(self.out, "\tsubq\t%rax, %rsp")?;
            } else {
                writeln!(self.out, "\tsubq\t${size}, %rsp")?;
            }
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_offset {}", frame.cfa_offset())?;
            } else {
                writeln!(self.out, "\t.seh_stackalloc {size}")?;
            }
        }
        for &(register, offset) in &frame.saved_xmm {
            writeln!(
                self.out,
                "\tmovaps\t%{}, {offset}(%rsp)\n\t.seh_savexmm %{}, {offset}",
                register.name(Width::B64),
                register.name(Width::B64)
            )?;
        }
        if frame.rbp_based {
            writeln!(self.out, "\tmovq\t%rsp, %rbp")?;
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa_register %rbp")?;
            } else {
                writeln!(self.out, "\t.seh_setframe %rbp, 0")?;
            }
        }
        if !self.elf {
            writeln!(self.out, "\t.seh_endprologue")?;
        }
        Ok(())
    }

    fn epilogue(&mut self, frame: &Frame<'_>) -> fmt::Result {
        let size = frame.allocation();
        if self.elf {
            writeln!(self.out, "\t.cfi_remember_state")?;
        }
        let mut cfa = frame.cfa_offset();
        let base = if frame.rbp_based { "rbp" } else { "rsp" };
        for &(register, offset) in &frame.saved_xmm {
            writeln!(
                self.out,
                "\tmovaps\t{offset}(%{base}), %{}",
                register.name(Width::B64)
            )?;
        }
        if frame.rbp_based {
            writeln!(self.out, "\tleaq\t{size}(%rbp), %rsp")?;
            cfa -= size;
            if self.elf {
                writeln!(self.out, "\t.cfi_def_cfa %rsp, {cfa}")?;
            }
        } else if size > 0 {
            writeln!(self.out, "\taddq\t${size}, %rsp")?;
            cfa -= size;
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
            module: self.module,
            func,
            elf: self.elf,
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
                | Callee::Symbol(SymbolRef::Ir(symbol))
                    if !self.elf
                        && (imported(self.module, symbol)
                            || weak_declaration(self.module, symbol)) =>
                    writeln!(out, "\tcallq\t*{}(%rip)", Stub::of(self.module, symbol)),
                | Callee::Symbol(symbol) => {
                    let name = symbol_name(self.module, symbol);
                    let plt = self.elf && !defined(self.module, symbol);
                    writeln!(out, "\tcallq\t{name}{}", if plt { "@PLT" } else { "" })
                },
                | Callee::Reg(target) => writeln!(out, "\tcallq\t*{}", reg(target, Width::B64)),
            },
            | MInst::XMov { width, dst, src } => match src {
                | Operand::Reg(src) =>
                    writeln!(out, "\tmovaps\t{}, {}", reg(src, width), reg(dst, width)),
                | src => writeln!(
                    out,
                    "\t{}\t{}, {}",
                    scalar("movs", width),
                    at(src, width),
                    reg(dst, width)
                ),
            },
            | MInst::Move128 {
                load,
                reg: xmm,
                mem,
            } =>
                if load {
                    writeln!(
                        out,
                        "\tmovups\t{}, {}",
                        at(Operand::Mem(mem), Width::B64),
                        reg(xmm, Width::B64)
                    )
                } else {
                    writeln!(
                        out,
                        "\tmovups\t{}, {}",
                        reg(xmm, Width::B64),
                        at(Operand::Mem(mem), Width::B64)
                    )
                },
            | MInst::XStore { width, dst, src } => writeln!(
                out,
                "\t{}\t{}, {}",
                scalar("movs", width),
                reg(src, width),
                at(Operand::Mem(dst), width)
            ),
            | MInst::XZero { dst } => writeln!(
                out,
                "\txorps\t{}, {}",
                reg(dst, Width::B64),
                reg(dst, Width::B64)
            ),
            | MInst::XAlu {
                op,
                width,
                dst,
                src,
            } => writeln!(
                out,
                "\t{}\t{}, {}",
                op.name(width),
                at(src, width),
                reg(dst, width)
            ),
            | MInst::Ucomi { width, lhs, rhs } => writeln!(
                out,
                "\t{}\t{}, {}",
                scalar("ucomis", width),
                reg(rhs, width),
                reg(lhs, width)
            ),
            | MInst::IntToFloat {
                int,
                float,
                dst,
                src,
            } => writeln!(
                out,
                "\t{}{}\t{}, {}",
                scalar("cvtsi2s", float),
                int.suffix(),
                reg(src, int),
                reg(dst, float)
            ),
            | MInst::FloatToInt {
                float,
                int,
                dst,
                src,
            } => writeln!(
                out,
                "\t{}2si\t{}, {}",
                scalar("cvtts", float),
                reg(src, float),
                reg(dst, int)
            ),
            | MInst::FloatToFloat { from, dst, src } => writeln!(
                out,
                "\t{}\t{}, {}",
                if from == Width::B32 {
                    "cvtss2sd"
                } else {
                    "cvtsd2ss"
                },
                reg(src, from),
                reg(dst, from)
            ),
            | MInst::MovGx { width, dst, src } => writeln!(
                out,
                "\t{}\t{}, {}",
                if width == Width::B64 { "movq" } else { "movd" },
                reg(src, width),
                reg(dst, width)
            ),
            | MInst::ShiftDouble {
                left,
                dst,
                src,
                amount,
            } => {
                let mnemonic = if left { "shldq" } else { "shrdq" };
                match amount {
                    | ShiftAmount::Imm(count) => writeln!(
                        out,
                        "\t{mnemonic}\t${count}, {}, {}",
                        reg(src, Width::B64),
                        reg(dst, Width::B64)
                    ),
                    | ShiftAmount::Cl => writeln!(
                        out,
                        "\t{mnemonic}\t%cl, {}, {}",
                        reg(src, Width::B64),
                        reg(dst, Width::B64)
                    ),
                }
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
            | MInst::JmpIndirect { target, .. } =>
                writeln!(out, "\tjmpq\t*{}", reg(target, Width::B64)),
            | MInst::ThreadPointer { dst } => {
                let pointer = if self.elf { "%fs:0" } else { "%gs:88" };
                writeln!(out, "\tmovq\t{pointer}, {}", reg(dst, Width::B64))
            },
            | MInst::TlsLea { dst, base, symbol } => {
                let offset = if self.elf { "TPOFF" } else { "SECREL32" };
                writeln!(
                    out,
                    "\tleaq\t{}@{offset}({}), {}",
                    symbol_name(self.module, symbol),
                    reg(base, Width::B64),
                    reg(dst, Width::B64)
                )
            },
            | MInst::X87Load { format, mem } => writeln!(
                out,
                "\t{}\t{}",
                format.load(),
                at(Operand::Mem(mem), Width::B64)
            ),
            | MInst::X87Store { format, mem } => writeln!(
                out,
                "\t{}\t{}",
                format.store(),
                at(Operand::Mem(mem), Width::B64)
            ),
            | MInst::X87Arith { op } => writeln!(out, "\t{}\t%st(1), %st", op.name()),
            | MInst::X87Pop => writeln!(out, "\tfstp\t%st(0)"),
            | MInst::X87Compare => writeln!(out, "\tfucomip\t%st(1), %st"),
            | MInst::X87ControlWord { store, mem } => writeln!(
                out,
                "\t{}\t{}",
                if store { "fnstcw" } else { "fldcw" },
                at(Operand::Mem(mem), Width::B16)
            ),
            | MInst::Ret { .. } => self.epilogue(frame),
            | MInst::Ud2 => writeln!(out, "\tud2"),
            | MInst::Int3 => writeln!(out, "\tint3"),
        }
    }

    /// One relocation's bytes: a symbol's or label's address, or a
    /// difference of two labels.
    fn relocation(&mut self, relocation: &Relocation) -> fmt::Result {
        match relocation.target {
            | RelocationTarget::Symbol(symbol) => write!(
                self.out,
                "\t.quad\t{}",
                symbol_name(self.module, SymbolRef::Ir(symbol))
            )?,
            | RelocationTarget::Label { func, label } =>
                write!(self.out, "\t.quad\t{}", BlockLabel(func, label.as_u32()))?,
            | RelocationTarget::LabelDifference {
                func,
                label,
                base,
                width,
            } => {
                let directive = match width.bits() {
                    | 8 => "byte",
                    | 16 => "short",
                    | 32 => "long",
                    | _ => "quad",
                };
                write!(
                    self.out,
                    "\t.{directive}\t{}-{}",
                    BlockLabel(func, label.as_u32()),
                    BlockLabel(func, base.as_u32())
                )?;
            },
        }
        if relocation.addend != 0 {
            write!(self.out, "{:+}", relocation.addend)?;
        }
        writeln!(self.out)
    }

    /// Every defined global, in its section, and the file trailer.
    pub(crate) fn globals(
        &mut self,
        references: &super::support::GlobalReferences<'_>,
    ) -> fmt::Result {
        for (id, global) in self.module.globals() {
            if global.thread_local
                && global.attributes.section.is_some()
                && references.unused_internal_global(self.module, id)
            {
                continue;
            }
            let Some(init) = global.init else { continue };
            let name = Name(global.name);
            let relocated =
                matches!(init, GlobalInit::Bytes { relocations, .. } if !relocations.is_empty());
            if global.linkage.comdat() {
                let zero = init == GlobalInit::Zero;
                let read_only = global.constant && !relocated;
                let (base, flags) = match (self.elf, global.thread_local) {
                    | (false, true) => (".tls$", "dw"),
                    | (true, true) if zero => (".tbss", "awT"),
                    | (true, true) => (".tdata", "awT"),
                    | (true, false) if global.constant && relocated => (".data.rel.ro", "aw"),
                    | (true, false) if read_only => (".rodata", "a"),
                    | (true, false) if zero => (".bss", "aw"),
                    | (true, false) => (".data", "aw"),
                    | (false, false) if global.constant => (".rdata", "dr"),
                    | (false, false) if zero => (".bss", "bw"),
                    | (false, false) => (".data", "dw"),
                };
                let flags = match (global.attributes.section, self.elf) {
                    | (Some(_), true) if read_only => "a",
                    | (Some(_), true) => "aw",
                    | (Some(_), false) if read_only => "dr",
                    | (Some(_), false) => "dw",
                    | (None, _) => flags,
                };
                // Only the zero-filled sections have no file contents; a
                // constant zero object lives in `.rodata`, which has them.
                let nobits = self.elf
                    && matches!(base, ".bss" | ".tbss")
                    && global.attributes.section.is_none();
                self.comdat_section(global.attributes.section, base, flags, nobits, name)?;
            } else if let Some(section) = global.attributes.section {
                let kind = elf_section_type(section);
                let flags = elf_section_flags(section, false, global.constant && !relocated);
                let section = Name(section);
                // A position-independent executable relocates addresses at
                // load time, so a relocated constant needs a writable section.
                match (self.elf, global.constant && !relocated) {
                    | (true, _) => writeln!(self.out, "\t.section\t{section},\"{flags}\",@{kind}")?,
                    | (false, true) => writeln!(self.out, "\t.section\t{section},\"dr\"")?,
                    | (false, false) => writeln!(self.out, "\t.section\t{section},\"dw\"")?,
                }
            }
            let section = match (global.constant, init) {
                | _ if global.attributes.section.is_some() || global.linkage.comdat() => "",
                | _ if global.thread_local && !self.elf => "\t.section\t.tls$,\"dw\"",
                | (_, GlobalInit::Zero) if global.thread_local =>
                    "\t.section\t.tbss,\"awT\",@nobits",
                | _ if global.thread_local => "\t.section\t.tdata,\"awT\",@progbits",
                // Addresses in a position-independent executable are
                // relocated at load time, which a read-only section forbids;
                // the loader write-protects `.data.rel.ro` after relocating.
                | (true, GlobalInit::Bytes { relocations, .. })
                    if self.elf && !relocations.is_empty() =>
                    "\t.section\t.data.rel.ro,\"aw\",@progbits",
                | (true, _) if self.elf => "\t.section\t.rodata",
                | (true, _) => "\t.section\t.rdata,\"dr\"",
                | (false, GlobalInit::Zero) => "\t.bss",
                | (false, GlobalInit::Bytes { .. }) => "\t.data",
            };
            if !section.is_empty() {
                writeln!(self.out, "{section}")?;
            }
            self.binding(name, global.linkage, global.attributes.visibility)?;
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
                        self.relocation(relocation)?;
                        offset += usize::try_from(relocation.size()).map_err(|_| fmt::Error)?;
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
        self.aliases()?;
        self.weak_declarations()?;
        self.init_functions()?;
        if !self.elf {
            self.exports()?;
        }
        if self.elf {
            writeln!(self.out, "\t.section\t.note.GNU-stack,\"\",@progbits")?;
        }
        Ok(())
    }

    /// Each alias as `name = target`, with its binding and, for a function,
    /// its symbol type.
    fn aliases(&mut self) -> fmt::Result {
        let module = self.module;
        let functions = module.functions().filter_map(|(_, function)| {
            let target = function.attributes.alias?;
            Some((
                function.name,
                function.linkage,
                function.attributes,
                target,
                true,
            ))
        });
        let globals = module.globals().filter_map(|(_, global)| {
            let target = global.attributes.alias?;
            Some((
                global.name,
                global.linkage,
                global.attributes,
                target,
                false,
            ))
        });
        for (name, linkage, attributes, target, function) in functions.chain(globals) {
            let name = Name(name);
            if function && !self.elf {
                self.coff_function(name, linkage)?;
            }
            self.binding(name, linkage, attributes.visibility)?;
            if function && self.elf {
                writeln!(self.out, "\t.type\t{name},@function")?;
            }
            writeln!(self.out, "{name} = {}", Name(module.symbol_name(target)))?;
        }
        Ok(())
    }

    /// `.weak` for each weak declaration, and on COFF the `.refptr` stub
    /// through which code reaches it or MinGW data the module does not
    /// define, as Clang writes them. A weak declaration's stub is written
    /// whether or not code uses it; other data gets one only where code
    /// uses it, since the stub refers to the data. A `dllimport`
    /// declaration's `__imp_` pointer comes from the import library instead.
    fn weak_declarations(&mut self) -> fmt::Result {
        let module = self.module;
        let functions = module.functions().map(|(id, _)| Symbol::Function(id));
        let globals = module.globals().map(|(id, _)| Symbol::Global(id));
        for symbol in functions.chain(globals) {
            let weak = weak_declaration(module, symbol);
            let auto_imported =
                !weak && self.stubs.contains(&symbol) && through_stub(module, self.target, symbol);
            if !weak && !auto_imported {
                continue;
            }
            let name = module.symbol_name(symbol);
            if weak {
                writeln!(self.out, "\t.weak\t{}", Name(name))?;
            }
            if !self.elf && !imported(module, symbol) {
                let stub = Stub::of(module, symbol);
                writeln!(self.out, "\t.section\t.rdata${stub},\"dr\",discard,{stub}")?;
                writeln!(self.out, "\t.p2align\t3\n\t.globl\t{stub}\n{stub}:")?;
                writeln!(self.out, "\t.quad\t{}", Name(name))?;
            }
        }
        Ok(())
    }

    /// The `/EXPORT` (MSVC) or `-export` (MinGW) linker directive of each
    /// `dllexport` definition in `.drectve`, marking data `DATA`, in
    /// LLVM's order: functions, globals, then aliases.
    /// MSVC extension: Microsoft Learn, "dllexport, dllimport" and
    /// "/EXPORT (Exports a Function)".
    fn exports(&mut self) -> fmt::Result {
        let module = self.module;
        let exported = || {
            let functions = module
                .functions()
                .map(|(_, function)| (function.name, function.attributes, false));
            let globals = module
                .globals()
                .map(|(_, global)| (global.name, global.attributes, true));
            functions
                .chain(globals)
                .filter(|&(_, attributes, _)| attributes.dll == DllStorage::Export)
        };
        let definitions = exported().filter(|&(_, attributes, _)| attributes.alias.is_none());
        let aliases = exported().filter_map(|(name, attributes, _)| {
            Some((
                name,
                attributes,
                matches!(attributes.alias?, Symbol::Global(_)),
            ))
        });
        let mut first = true;
        for (name, _, data) in definitions.chain(aliases) {
            if first {
                writeln!(self.out, "\t.section\t.drectve,\"yni\"")?;
                first = false;
            }
            let (option, data_flag) = if self.target == Target::WindowsMsvc {
                ("/EXPORT", ",DATA")
            } else {
                ("-export", ",data")
            };
            // LLVM quotes a name with characters other than letters,
            // digits, `_` and `@`.
            let quote = if name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'@')
            {
                ""
            } else {
                "\\\""
            };
            writeln!(
                self.out,
                "\t.ascii\t\" {option}:{quote}{name}{quote}{}\"",
                if data { data_flag } else { "" }
            )?;
        }
        Ok(())
    }

    /// The constructor and destructor arrays, in registration order within
    /// each priority's section, in the sections Clang uses.
    fn init_functions(&mut self) -> fmt::Result {
        for init in self.module.init_functions() {
            let constructor = init.kind == InitKind::Constructor;
            let priority = init.priority;
            let default = priority == DEFAULT_PRIORITY;
            match self.target {
                | Target::LinuxGnu | Target::LinuxMusl => {
                    let (section, kind) = if constructor {
                        (".init_array", "@init_array")
                    } else {
                        (".fini_array", "@fini_array")
                    };
                    write!(self.out, "\t.section\t{section}")?;
                    if !default {
                        write!(self.out, ".{priority}")?;
                    }
                    writeln!(self.out, ",\"aw\",{kind}")?;
                },
                | Target::WindowsGnu => {
                    let section = if constructor { ".ctors" } else { ".dtors" };
                    write!(self.out, "\t.section\t{section}")?;
                    if !default {
                        write!(self.out, ".{:05}", DEFAULT_PRIORITY - priority)?;
                    }
                    writeln!(self.out, ",\"dw\"")?;
                },
                | Target::WindowsMsvc => {
                    // Sections sort by name: `A` before the CRT's `L`
                    // entries, `C` and `T` around them, `U`/`X` last.
                    let kind = if constructor { 'C' } else { 'T' };
                    let (letter, suffix) = match priority {
                        | DEFAULT_PRIORITY => (if constructor { 'U' } else { 'X' }, false),
                        | ..200 => ('A', true),
                        | 200 => ('C', false),
                        | 201..400 => ('C', true),
                        | 400 => ('L', false),
                        | _ => ('T', true),
                    };
                    write!(self.out, "\t.section\t.CRT$X{kind}{letter}")?;
                    if suffix {
                        write!(self.out, "{priority:05}")?;
                    }
                    writeln!(self.out, ",\"dr\"")?;
                },
            }
            let name = Name(self.module.function(init.func).name);
            writeln!(self.out, "\t.p2align\t3\n\t.quad\t{name}")?;
        }
        Ok(())
    }
}

/// The priority of a constructor or destructor that names none.
const DEFAULT_PRIORITY: u16 = 65535;

/// The pointer-sized constant holding a COFF symbol's address that code
/// loads ([`through_stub`]): the import address table entry `__imp_name` of
/// a `dllimport` declaration, else the `.refptr` stub of a weak declaration
/// or MinGW auto-imported datum.
struct Stub<'a> {
    name:   &'a str,
    import: bool,
}

impl<'a> Stub<'a> {
    fn of(module: &Module<'a>, symbol: Symbol) -> Self {
        Self {
            name:   module.symbol_name(symbol),
            import: imported(module, symbol),
        }
    }
}

impl fmt::Display for Stub<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = if self.import { "__imp_" } else { ".refptr." };
        if Name(self.name).plain() {
            write!(f, "{prefix}{}", self.name)
        } else {
            write!(f, "\"{prefix}{}\"", self.name)
        }
    }
}

/// The directive that starts a loop header: the next 16-byte boundary,
/// padded with multi-byte `nop`s, as LLVM aligns loops on x86-64
/// (`X86TargetLowering` prefers 16-byte loop alignment and sets no limit on
/// the padding). A limit, as GCC's `.p2align 4,,10`, measured slower on
/// `benches/c`: a loop it leaves unaligned loses what alignment gained.
const LOOP_ALIGNMENT: &str = "\t.p2align\t4";

/// Which blocks of `func` are loop headers in its final layout: targets of
/// an edge from a later block or from themselves. Every loop the layout
/// keeps has one, entered from the block before it once per run of the loop
/// and from its back edge on every iteration, so its padding runs once per
/// run of the loop. Other blocks inside loops stay where they fall: LLVM
/// also aligns those its block frequencies call hot, but without them that
/// would align, for example, every case of an interpreter's dispatch, which
/// on `benches/c` cost as much as it saved. The entry block is aligned with
/// the function.
fn aligned_blocks<'a>(func: &MFunction<'_>, arena: &'a Bump) -> ArenaVec<'a, bool> {
    let blocks = func.blocks.len();
    let mut aligned = ArenaVec::with_capacity_in(blocks, arena);
    aligned.resize(blocks, false);
    for block in 0..blocks {
        for successor in func.successors(MBlock::new(block)) {
            if successor.index() <= block && successor.index() > 0 {
                aligned[successor.index()] = true;
            }
        }
    }
    aligned
}

/// Whether the module defines `symbol`; runtime symbols never are.
fn defined(module: &Module<'_>, symbol: SymbolRef) -> bool {
    match symbol {
        | SymbolRef::Ir(symbol) => defines(module, symbol),
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

// LLVM's ELF lowering gives explicitly named array and BSS families their
// semantic types too, so section attributes agree with constructor emission.
fn elf_section_type(name: &str) -> &'static str {
    ElfSectionKind::named(name).asm_type()
}

// LLVM's named-section kind overrides the source entity kind for BSS/TLS;
// large data is an x86-64 flag, not a request to make `.lbss` NOBITS.
fn elf_section_flags(name: &str, function: bool, read_only: bool) -> &'static str {
    if ElfSectionKind::named(name).is_thread_local() {
        "awT"
    } else if elf_section_type(name) == "nobits" {
        "aw"
    } else if function {
        if elf_section_family(name, ".ltext") {
            "axl"
        } else {
            "ax"
        }
    } else if [".ldata", ".lbss", ".lrodata"]
        .iter()
        .any(|stem| elf_section_family(name, stem))
    {
        if read_only { "al" } else { "awl" }
    } else if read_only {
        "a"
    } else {
        "aw"
    }
}

/// LLVM MC's BSS diagnostic, shared by text and direct object emission.
#[derive(Debug, thiserror::Error)]
#[error("BSS section '{section}' cannot have {}", if *.fixups { "fixups" } else { "non-zero bytes" })]
pub(crate) struct SectionError<'m> {
    section: &'m str,
    fixups:  bool,
}

/// A symbol name, quoted unless it is a plain identifier.
#[derive(Clone, Copy)]
struct Name<'a>(&'a str);

impl Name<'_> {
    /// Whether the assembler takes the name unquoted.
    fn plain(&self) -> bool {
        self.0.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || byte == b'_'
                || byte == b'.'
                || byte == b'$'
                || (index > 0 && byte.is_ascii_digit())
        })
    }
}

impl fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.plain() {
            f.write_str(self.0)
        } else {
            write!(f, "\"{}\"", self.0)
        }
    }
}

/// A scalar SSE mnemonic: `prefix` and `s` (single) or `d` (double).
const fn scalar(prefix: &str, width: Width) -> Scalar<'_> {
    Scalar(prefix, width)
}

/// A scalar SSE mnemonic.
struct Scalar<'a>(&'a str, Width);

impl fmt::Display for Scalar<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.1 == Width::B64 { 'd' } else { 's' };
        write!(f, "{}{kind}", self.0)
    }
}

/// A function's read-only constant's label.
struct ConstantLabel(FuncId, usize);

impl fmt::Display for ConstantLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".LCPI{}_{}", self.0.index(), self.1)
    }
}

/// A block's label.
struct Label(FuncId, MBlock);

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".LBB{}_{}", self.0.index(), self.1.index())
    }
}

/// An IR block label's symbol: the address `block_addr` takes.
struct BlockLabel(FuncId, u32);

impl fmt::Display for BlockLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".LBA{}_{}", self.0.index(), self.1)
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
    module:  &'m Module<'ir>,
    func:    FuncId,
    elf:     bool,
}

impl fmt::Display for Op<'_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.operand {
            | Operand::Reg(reg) => write!(f, "{}", RegName(reg.phys(), self.width)),
            | Operand::Imm(imm) => write!(f, "${imm}"),
            | Operand::Mem(Mem { base, index, disp }) => {
                let module = self.module;
                let index = fmt::from_fn(|f| match index {
                    | Some(index) => write!(
                        f,
                        ",{},{}",
                        RegName(index.reg.phys(), Width::B64),
                        index.scale
                    ),
                    | None => Ok(()),
                });
                match base {
                    | Base::Reg(reg) => {
                        if disp != 0 {
                            write!(f, "{disp}")?;
                        }
                        write!(f, "({}{index})", RegName(reg.phys(), Width::B64))
                    },
                    | Base::Frame(object) => {
                        let (base, offset) = self.frame.address(object);
                        let offset = offset + i64::from(disp);
                        write!(f, "{offset}({}{index})", RegName(base, Width::B64))
                    },
                    | Base::Constant(index) => {
                        write!(f, "{}", ConstantLabel(self.func, index as usize))?;
                        if disp != 0 {
                            write!(f, "{disp:+}")?;
                        }
                        write!(f, "(%rip)")
                    },
                    | Base::Symbol(symbol) => {
                        write!(f, "{}", symbol_name(module, symbol))?;
                        if disp != 0 {
                            write!(f, "{disp:+}")?;
                        }
                        write!(f, "(%rip)")
                    },
                    | Base::Got(symbol) if self.elf =>
                        write!(f, "{}@GOTPCREL(%rip)", symbol_name(module, symbol)),
                    | Base::Got(symbol) => {
                        let SymbolRef::Ir(symbol) = symbol else {
                            unreachable!("COFF reaches only IR symbols through a stub")
                        };
                        write!(f, "{}(%rip)", Stub::of(module, symbol))
                    },
                    | Base::GotTpoff(symbol) =>
                        write!(f, "{}@GOTTPOFF(%rip)", symbol_name(module, symbol)),
                    | Base::Label(label) => {
                        write!(f, "{}", BlockLabel(self.func, label))?;
                        if disp != 0 {
                            write!(f, "{disp:+}")?;
                        }
                        write!(f, "(%rip)")
                    },
                }
            },
        }
    }
}
