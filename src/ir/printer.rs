//! The textual form, as the printer writes it.
//!
//! A module prints its target lines, then its globals one per line, then each
//! function, with a blank line between these sections. Entities print as
//! their raw numbers (`v3`, `block1`, `slot0`), so verifier errors and printed
//! text name the same things. The parser reads this form back unchanged.

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
        GlobalInit,
        Signature,
        Symbol,
    },
    types::Type,
};

impl fmt::Display for Module<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        if !self.triple().is_empty() || !self.data_layout().is_empty() {
            writeln!(f, "target triple = \"{}\"", self.triple())?;
            writeln!(f, "target datalayout = \"{}\"", self.data_layout())?;
            separator = "\n";
        }
        if self.globals().next().is_some() {
            f.write_str(separator)?;
            for (global, _) in self.globals() {
                writeln!(f, "{}", self.display_global(global))?;
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
                "function @{}{} {}",
                function.name,
                self.display_signature(function.signature),
                function.linkage.name()
            )?;
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
            }
        })
    }

    /// A global's line, without the newline.
    pub(crate) fn display_global(&self, global: GlobalId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| {
            let data = self.global(global);
            write!(f, "global @{} {}", data.name, data.linkage.name())?;
            if data.constant {
                f.write_str(" constant")?;
            }
            write!(f, " size {}, align {}", data.size, data.align.bytes())?;
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
                            self.display_symbol(relocation.symbol)
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

    fn display_symbol(&self, symbol: Symbol) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match symbol {
            | Symbol::Function(func) => write!(f, "{}", self.display_func_name(func)),
            | Symbol::Global(global) => write!(f, "{}", self.display_global_name(global)),
        })
    }

    /// `@name`, or the raw id of a function the module does not have.
    fn display_func_name(&self, func: FuncId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.get_function(func) {
            | Some(function) => write!(f, "@{}", function.name),
            | None => write!(f, "{func}"),
        })
    }

    /// `@name`, or the raw id of a global the module does not have.
    fn display_global_name(&self, global: GlobalId) -> impl fmt::Display + '_ {
        fmt::from_fn(move |f| match self.get_global(global) {
            | Some(data) => write!(f, "@{}", data.name),
            | None => write!(f, "{global}"),
        })
    }
}

fn display_signature(signature: Signature<'_>) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_char('(')?;
        for (index, ty) in signature.params.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            write!(f, "{separator}{ty}")?;
        }
        if signature.variadic {
            f.write_str(if signature.params.is_empty() {
                "..."
            } else {
                ", ..."
            })?;
        }
        f.write_char(')')?;
        signature
            .result
            .map_or(Ok(()), |result| write!(f, " -> {result}"))
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
