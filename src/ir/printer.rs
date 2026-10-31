//! The textual form, as the printer writes it.
//!
//! A module prints its target lines, then its file-scope assembly blocks one
//! per line, then its globals one per line, then its constructors and
//! destructors one per line, then each function, with a blank line between
//! these sections. Entities print as their raw numbers
//! (`v3`, `block1`, `slot0`, `label0`), so verifier errors and printed text
//! name the same things. The parser reads this form back unchanged.

use std::fmt::{
    self,
    Write as _,
};

use super::{
    Body,
    FuncId,
    GlobalId,
    Module,
    entities::{
        Entity,
        Inst,
    },
    instructions::{
        BlockCall,
        InstData,
    },
    module::{
        AggregateFlags,
        GlobalInit,
        InitFunction,
        ParamAbi,
        RelocationTarget,
        Signature,
        Symbol,
        SymbolAttributes,
    },
    types::Type,
    verifier::Profile,
};

impl fmt::Display for Module<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        if !self.triple().is_empty() || !self.data_layout().is_empty() {
            writeln!(f, "target triple = \"{}\"", self.triple())?;
            writeln!(f, "target datalayout = \"{}\"", self.data_layout())?;
            separator = "\n";
        }
        if self.profile() == Profile::PostAbi {
            writeln!(f, "abi lowered")?;
            separator = "\n";
        }
        if !self.module_asm().is_empty() {
            f.write_str(separator)?;
            for text in self.module_asm() {
                writeln!(f, "module asm {}", display_asm_text(text))?;
            }
            separator = "\n";
        }
        if self.globals().next().is_some() {
            f.write_str(separator)?;
            for (global, _) in self.globals() {
                writeln!(f, "{}", self.display_global(global))?;
            }
            separator = "\n";
        }
        if !self.init_functions().is_empty() {
            f.write_str(separator)?;
            for &init in self.init_functions() {
                writeln!(f, "{}", self.display_init_function(init))?;
            }
            separator = "\n";
        }
        for (func, _) in self.functions() {
            f.write_str(separator)?;
            write!(f, "{}", self.display_function(func))?;
            separator = "\n";
        }
        Ok(())
    }
}

impl<'ir> Module<'ir> {
    /// A function's text: its header line, and its body if it has one, each
    /// line ending in a newline.
    pub(crate) fn display_function(&self, func: FuncId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| {
            let function = self.function(func);
            write!(
                f,
                "function {}{} {}",
                display_name(function.name),
                self.display_signature(function.signature),
                function.linkage.name()
            )?;
            if let Some(align) = function.align {
                write!(f, " align {}", align.bytes())?;
            }
            if function.returns_twice {
                f.write_str(" returns_twice")?;
            }
            if let Some(keyword) = function.inlining.keyword() {
                write!(f, " {keyword}")?;
            }
            if let Some(external) = function.inline_copy_of {
                write!(f, " inline_copy_of {}", display_name(external))?;
            }
            write!(f, "{}", self.display_symbol_attributes(function.attributes))?;
            let Some(body) = &function.body else {
                return f.write_char('\n');
            };
            f.write_str(" {\n")?;
            for (slot, data) in body.stack_slots() {
                writeln!(
                    f,
                    "    {slot} = stack_slot {}, align {}",
                    data.size,
                    data.align.bytes()
                )?;
            }
            for (label, block) in body.labels() {
                writeln!(f, "    {label} = block_label {block}")?;
            }
            for block in body.blocks() {
                write!(f, "{block}")?;
                let params = body.block_params(block);
                if !params.is_empty() {
                    f.write_char('(')?;
                    for (index, &param) in params.iter().enumerate() {
                        let separator = if index == 0 { "" } else { ", " };
                        write!(f, "{separator}{param}: {}", body.value_type(param))?;
                    }
                    f.write_char(')')?;
                }
                f.write_str(":\n")?;
                for &inst in body.block_insts(block) {
                    writeln!(f, "    {}", self.display_inst(body, inst))?;
                }
            }
            f.write_str("}\n")
        })
    }

    /// One instruction without indentation or newline, as
    /// `v2 = iadd.i32 nsw v0, v1`.
    pub(crate) fn display_inst<'a>(
        &'a self,
        body: &'a Body<'ir>,
        inst: Inst,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let data = body.inst(inst);
            if let Some(result) = body.inst_result(inst) {
                write!(f, "{result} = ")?;
            }
            let opcode = data.opcode();
            f.write_str(opcode.name())?;
            if opcode.has_type_suffix()
                && let Some(ty) = data.controlling_type()
            {
                write!(f, ".{ty}")?;
            }
            let flags = data.flags();
            if !flags.is_empty() {
                write!(f, " {flags}")?;
            }
            match *data {
                | InstData::Binary { args: [a, b], .. } => write!(f, " {a}, {b}"),
                | InstData::Unary { arg, .. } => write!(f, " {arg}"),
                | InstData::ObjectSize { query, ptr, .. } => {
                    let dynamic = if query.dynamic() { " dynamic" } else { "" };
                    write!(f, "{dynamic} {ptr}, {}", query.mode())
                },
                | InstData::DynAlloca { size, align, .. } =>
                    write!(f, " {size}, align {}", align.bytes()),
                | InstData::VaArgAggregate { sig, list } =>
                    write!(f, " {list} : {}", self.display_signature(sig)),
                | InstData::IntCompare {
                    cond, args: [a, b], ..
                } => write!(f, " {cond} {a}, {b}"),
                | InstData::FloatCompare {
                    cond, args: [a, b], ..
                } => write!(f, " {cond} {a}, {b}"),
                | InstData::Select {
                    args: [cond, a, b], ..
                } => write!(f, " {cond}, {a}, {b}"),
                | InstData::Const { opcode, ty, .. } | InstData::WideConst { opcode, ty, .. } => {
                    let bits = body.const_bits(data).unwrap_or(0);
                    if opcode == super::Opcode::Fconst {
                        write!(
                            f,
                            " 0x{bits:0width$X}",
                            width = ty.bits().div_ceil(4) as usize
                        )
                    } else {
                        write!(f, " {}", display_int(bits, ty))
                    }
                },
                | InstData::Nullary { .. } | InstData::Unreachable => Ok(()),
                | InstData::StackAddr { slot } => write!(f, " {slot}"),
                | InstData::GlobalAddr { global } =>
                    write!(f, " {}", self.display_global_name(global)),
                | InstData::FuncAddr { func } => write!(f, " {}", self.display_func_name(func)),
                | InstData::BlockAddr { label } => write!(f, " {label}"),
                | InstData::Atomic {
                    op,
                    order,
                    failure,
                    align,
                    args,
                    ..
                } => {
                    write!(f, " {op} {order} {failure} align {} (", align.bytes())?;
                    for (index, arg) in body.value_list(args).iter().enumerate() {
                        if index != 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{arg}")?;
                    }
                    f.write_str(")")
                },
                | InstData::Load {
                    align, tag, addr, ..
                } => {
                    write!(f, " {addr}, align {}", align.bytes())?;
                    tag.map_or(Ok(()), |tag| write!(f, ", tag {}", tag.index()))
                },
                | InstData::Store {
                    align,
                    tag,
                    args: [value, addr],
                    ..
                } => {
                    write!(f, " {value}, {addr}, align {}", align.bytes())?;
                    tag.map_or(Ok(()), |tag| write!(f, ", tag {}", tag.index()))
                },
                | InstData::MemoryRange {
                    align,
                    args: [a, b, c],
                    ..
                } => write!(f, " {a}, {b}, {c}, align {}", align.bytes()),
                | InstData::Call { func, args } => write!(
                    f,
                    " {}({})",
                    self.display_func_name(func),
                    display_values(body.value_list(args))
                ),
                | InstData::CallIndirect { sig, args } => {
                    let values = body.value_list(args);
                    let (callee, rest) = values
                        .split_first()
                        .map_or((None, values), |(c, r)| (Some(c), r));
                    if let Some(callee) = callee {
                        write!(f, " {callee}")?;
                    }
                    write!(
                        f,
                        "({}) : {}",
                        display_values(rest),
                        self.display_signature(sig)
                    )
                },
                | InstData::InlineAsm { asm, args } => write!(
                    f,
                    " {}",
                    self.display_inline_asm(asm, body.value_list(args))
                ),
                | InstData::Jump { dest } => write!(f, " {}", display_block_call(body, dest)),
                | InstData::Brif {
                    cond,
                    dests: [then, otherwise],
                } => write!(
                    f,
                    " {cond}, {}, {}",
                    display_block_call(body, then),
                    display_block_call(body, otherwise)
                ),
                | InstData::Switch {
                    value,
                    default,
                    cases,
                } => {
                    let ty = if value.index() < body.value_count() {
                        body.value_type(value)
                    } else {
                        Type::I128
                    };
                    write!(f, " {value}, {}, [", display_block_call(body, default))?;
                    for (index, (bits, dest)) in body.switch_cases(cases).enumerate() {
                        let separator = if index == 0 { "" } else { ", " };
                        write!(
                            f,
                            "{separator}{}: {}",
                            display_int(bits, ty),
                            display_block_call(body, dest)
                        )?;
                    }
                    f.write_char(']')
                },
                | InstData::Return { value } => value.map_or(Ok(()), |value| write!(f, " {value}")),
                | InstData::IndirectBr { addr, dests } => {
                    write!(f, " {addr}, [")?;
                    for (index, dest) in body.block_list(dests).enumerate() {
                        let separator = if index == 0 { "" } else { ", " };
                        write!(f, "{separator}{}", display_block_call(body, dest))?;
                    }
                    f.write_char(']')
                },
            }
        })
    }

    /// A global's line, without the newline.
    pub(crate) fn display_global(&self, global: GlobalId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| {
            let data = self.global(global);
            write!(
                f,
                "global {} {}",
                display_name(data.name),
                data.linkage.name()
            )?;
            if data.thread_local {
                f.write_str(" thread_local")?;
            }
            if data.constant {
                f.write_str(" constant")?;
            }
            write!(f, " size {}, align {}", data.size, data.align.bytes())?;
            write!(f, "{}", self.display_symbol_attributes(data.attributes))?;
            match data.init {
                | None => Ok(()),
                | Some(GlobalInit::Zero) => f.write_str(" = zero"),
                | Some(GlobalInit::Bytes { bytes, relocations }) => {
                    f.write_str(" = bytes \"")?;
                    for byte in bytes {
                        write!(f, "{byte:02x}")?;
                    }
                    f.write_char('"')?;
                    if relocations.is_empty() {
                        return Ok(());
                    }
                    f.write_str(" relocs [")?;
                    for (index, relocation) in relocations.iter().enumerate() {
                        let separator = if index == 0 { "" } else { ", " };
                        write!(
                            f,
                            "{separator}{}: {}",
                            relocation.offset,
                            self.display_relocation_target(relocation.target)
                        )?;
                        match relocation.addend {
                            | 0 => {},
                            | addend if addend < 0 => write!(f, " - {}", addend.unsigned_abs())?,
                            | addend => write!(f, " + {addend}")?,
                        }
                    }
                    f.write_char(']')
                },
            }
        })
    }

    /// A signature as `(i32, ptr, ...) -> i32`; `-> ty` is absent without a
    /// result.
    pub(crate) fn display_signature(&self, sig: super::SigId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.get_signature(sig) {
            | Some(signature) => write!(f, "{}", display_signature(signature)),
            | None => write!(f, "{sig}"),
        })
    }

    /// An `asm` instruction's operands after the opcode:
    /// `sideeffect "template" (out "r" i32 v1, in "m" i8 bytes 16 v2)
    /// clobber(memory, rcx)`, or the bare handle of a missing descriptor.
    fn display_inline_asm<'a>(
        &'a self,
        asm: super::AsmId,
        values: &'a [super::Value],
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            let Some(descriptor) = self.get_inline_asm(asm) else {
                return write!(f, "{asm}({})", display_values(values));
            };
            if descriptor.side_effects {
                f.write_str("sideeffect ")?;
            }
            write!(f, "{} (", display_asm_text(descriptor.template))?;
            for (index, operand) in descriptor.operands.iter().enumerate() {
                if index != 0 {
                    f.write_str(", ")?;
                }
                let role = if operand.output { "out" } else { "in" };
                let early = if operand.early_clobber { "&" } else { "" };
                write!(f, "{role} \"{early}{}\" {}", operand.constraint, operand.ty)?;
                if operand.bytes != operand.ty.bytes() {
                    write!(f, " bytes {}", operand.bytes)?;
                }
                match values.get(index) {
                    | Some(value) => write!(f, " {value}")?,
                    | None => f.write_str(" <missing>")?,
                }
            }
            for value in values.iter().skip(descriptor.operands.len()) {
                write!(f, ", {value}")?;
            }
            f.write_char(')')?;
            if descriptor.clobbers != super::AsmClobbers::default() {
                f.write_str(" clobber(")?;
                let mut separator = "";
                let mut result = Ok(());
                descriptor.clobbers.names(|name| {
                    result = result.and_then(|()| write!(f, "{separator}{name}"));
                    separator = ", ";
                });
                result?;
                f.write_char(')')?;
            }
            Ok(())
        })
    }

    /// ` hidden`/` protected`, ` dllimport`/` dllexport`, ` section
    /// "name"`, ` used` and ` alias @target`, each when present.
    fn display_symbol_attributes<'a>(
        &'a self,
        attributes: SymbolAttributes<'a>,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |f| {
            if let Some(visibility) = attributes.visibility.keyword() {
                write!(f, " {visibility}")?;
            }
            if let Some(dll) = attributes.dll.keyword() {
                write!(f, " {dll}")?;
            }
            if let Some(section) = attributes.section {
                write!(f, " section \"{section}\"")?;
            }
            if attributes.used {
                f.write_str(" used")?;
            }
            if let Some(target) = attributes.alias {
                write!(f, " alias {}", self.display_symbol(target))?;
            }
            Ok(())
        })
    }

    /// `constructor @f priority N` or `destructor @f priority N`.
    pub(crate) fn display_init_function(&self, init: InitFunction) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| {
            write!(
                f,
                "{} {} priority {}",
                init.kind.keyword(),
                self.display_func_name(init.func),
                init.priority
            )
        })
    }

    /// `@symbol`, `blockaddress(@f, label0)` or
    /// `labeldiff.i32(@f, label1, label0)`.
    fn display_relocation_target(&self, target: RelocationTarget) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match target {
            | RelocationTarget::Symbol(symbol) => write!(f, "{}", self.display_symbol(symbol)),
            | RelocationTarget::Label { func, label } =>
                write!(f, "blockaddress({}, {label})", self.display_func_name(func)),
            | RelocationTarget::LabelDifference {
                func,
                label,
                base,
                width,
            } => write!(
                f,
                "labeldiff.{width}({}, {label}, {base})",
                self.display_func_name(func)
            ),
        })
    }

    fn display_symbol(&self, symbol: Symbol) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match symbol {
            | Symbol::Function(func) => write!(f, "{}", self.display_func_name(func)),
            | Symbol::Global(global) => write!(f, "{}", self.display_global_name(global)),
        })
    }

    /// `@name`, or the raw id of a function the module does not have.
    fn display_func_name(&self, func: FuncId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.get_function(func) {
            | Some(function) => write!(f, "{}", display_name(function.name)),
            | None => write!(f, "{func}"),
        })
    }

    /// `@name`, or the raw id of a global the module does not have.
    fn display_global_name(&self, global: GlobalId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.get_global(global) {
            | Some(data) => write!(f, "{}", display_name(data.name)),
            | None => write!(f, "{global}"),
        })
    }
}

/// Quotes names outside the bare-symbol alphabet, preserving UTF-8 and
/// escaping delimiters and controls. Locals and blocks have numeric IDs.
///
/// C99: §6.4.2.1 paragraph 2, p. 51; PDF p. 63 (identifiers may include
/// non-Latin characters); quoting is the bcc IR representation rule.
fn display_name(name: &str) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_char('@')?;
        if !name.is_empty()
            && name.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '_' | '.' | '$')
                    || (!c.is_ascii() && !c.is_control())
            })
        {
            return f.write_str(name);
        }
        f.write_char('"')?;
        for c in name.chars() {
            match c {
                | '"' => f.write_str("\\\"")?,
                | '\\' => f.write_str("\\\\")?,
                | c if c.is_control() => write!(f, "\\u{{{:x}}}", u32::from(c))?,
                | c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    })
}

/// A file-scope assembly block as one quoted line. Backslashes, quotes and
/// controls (such as the block's newlines) become escapes, and a quote is
/// written as `\u{22}` so the string token never contains one; the parser
/// decodes the same escapes as in quoted symbol names.
fn display_asm_text(text: &str) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_char('"')?;
        for c in text.chars() {
            match c {
                | '\\' => f.write_str("\\\\")?,
                | '"' => f.write_str("\\u{22}")?,
                | c if c.is_control() => write!(f, "\\u{{{:x}}}", u32::from(c))?,
                | c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    })
}

fn display_signature(signature: Signature<'_>) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_char('(')?;
        for (index, ty) in signature.params.iter().enumerate() {
            if signature.variadic_fixed.map(|n| n as usize) == Some(index) {
                f.write_str(if index == 0 { "..." } else { ", ..." })?;
                if signature.variadic_noundef {
                    f.write_str(" noundef")?;
                }
                f.write_str(", ")?;
            }
            let separator =
                if index == 0 || signature.variadic_fixed.map(|n| n as usize) == Some(index) {
                    ""
                } else {
                    ", "
                };
            write!(f, "{separator}{ty}")?;
            if let Some(abi) = signature.param_abi(index) {
                write!(f, " {}", display_param_abi(abi))?;
            }
            if let Some(extension) = signature.param_extension(index) {
                write!(f, " {}", extension.keyword())?;
            }
            if signature.param_noundef(index) {
                f.write_str(" noundef")?;
            }
            if signature.chain_param() == Some(index) {
                f.write_str(" chain")?;
            }
        }
        if signature.variadic && signature.variadic_fixed.is_none() {
            f.write_str(if signature.params.is_empty() {
                "..."
            } else {
                ", ..."
            })?;
            if signature.variadic_noundef {
                f.write_str(" noundef")?;
            }
        }
        f.write_char(')')?;
        if let Some(result) = signature.result {
            write!(f, " -> {result}")?;
        }
        signature
            .result_extension
            .map_or(Ok(()), |extension| write!(f, " {}", extension.keyword()))?;
        if signature.result_noundef {
            f.write_str(if signature.result.is_some() {
                " noundef"
            } else {
                " -> noundef"
            })?;
        }
        Ok(())
    })
}

/// An aggregate parameter's kind and facts, as
/// `aggregate(size 12, align 4, [0: i32, 4: f32, 8: f32])`, `byval(size 24,
/// align 8)` or `registers(i64, f64, align 8)`. Flags such as `unaligned`
/// follow the alignment, then a transparent union's `first(size 8)`.
fn display_param_abi(abi: ParamAbi<'_>) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        write!(f, "{}(", abi.keyword())?;
        let Some(layout) = abi.layout() else {
            if let ParamAbi::Registers(registers) = abi {
                for ty in registers.types() {
                    write!(f, "{ty}, ")?;
                }
                if registers.second_offset != 8 {
                    write!(f, "offset {}, ", registers.second_offset)?;
                }
                if registers.complex {
                    f.write_str("complex, ")?;
                }
                write!(f, "align {}", registers.align.bytes())?;
            }
            return f.write_char(')');
        };
        write!(f, "size {}, align {}", layout.size, layout.align.bytes())?;
        for (flag, name) in AggregateFlags::NAMES {
            if layout.flags.contains(flag) {
                write!(f, ", {name}")?;
            }
        }
        if let Some(first) = layout.first {
            write!(f, ", first(size {}", first.size)?;
            if let Some(extension) = first.extension {
                write!(f, ", {}", extension.keyword())?;
            }
            f.write_char(')')?;
        }
        if !layout.scalars.is_empty() {
            f.write_str(", [")?;
            for (index, scalar) in layout.scalars.iter().enumerate() {
                let separator = if index == 0 { "" } else { ", " };
                write!(f, "{separator}{}: {}", scalar.offset, scalar.ty)?;
            }
            f.write_char(']')?;
        }
        f.write_char(')')
    })
}

/// `block1` or `block1(v2, v3)`.
fn display_block_call<'a>(body: &'a Body<'_>, call: BlockCall) -> impl fmt::Display + 'a {
    fmt::from_fn(move |f| {
        let (block, args) = body.block_call(call);
        write!(f, "{block}")?;
        if args.is_empty() {
            Ok(())
        } else {
            write!(f, "({})", display_values(args))
        }
    })
}

fn display_values(values: &[super::Value]) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        for (index, value) in values.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            write!(f, "{separator}{value}")?;
        }
        Ok(())
    })
}

/// Integer bits of a type as the textual form spells them: signed, except
/// that an `i1` is 0 or 1.
fn display_int(bits: u128, ty: Type) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        if ty == Type::I1 || !ty.is_int() {
            write!(f, "{bits}")
        } else {
            let shift = 128 - ty.bits();
            write!(f, "{}", ((bits << shift) as i128) >> shift)
        }
    })
}
