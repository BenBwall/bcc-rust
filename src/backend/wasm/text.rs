//! The text format: a [`WasmModule`] as WebAssembly text (`.wat`) that
//! `wat2wasm` or `wasm-tools parse` accepts and that reads like the binary
//! it was written beside: one instruction per line in linear form, indented
//! by nesting, with symbolic names for functions, locals and the stack
//! pointer, and a `;; blockN` comment where the code of IR block `N` starts
//! (with the IR instruction's position and source line on later marks).
//!
//! Reference: WebAssembly Core Specification 2.0, §6 "Text Format".

use std::fmt::{
    self,
    Write,
};

use super::{
    instr::{
        Instr,
        memory_name,
        num_name,
        trunc_sat_name,
    },
    module::{
        ExportKind,
        WasmModule,
    },
};

/// Writes `module` as text, recording in `lines` the 1-based line of each
/// instruction, in the order of the functions' code, marks left out.
pub(crate) fn write_module(
    module: &WasmModule<'_>,
    out: &mut impl Write,
    lines: &mut impl Extend<u32>,
) -> fmt::Result {
    let mut out = Counting { out, lines: 1 };
    let out = &mut out;
    writeln!(out, "(module")?;
    for (index, ty) in module.types.iter().enumerate() {
        write!(out, "  (type $t{index} (func")?;
        if !ty.params.is_empty() {
            write!(out, " (param")?;
            for param in ty.params {
                write!(out, " {}", param.name())?;
            }
            write!(out, ")")?;
        }
        if !ty.results.is_empty() {
            write!(out, " (result")?;
            for result in ty.results {
                write!(out, " {}", result.name())?;
            }
            write!(out, ")")?;
        }
        writeln!(out, "))")?;
    }
    for import in &module.imports {
        writeln!(
            out,
            "  (import \"{}\" \"{}\" (func ${} (type $t{})))",
            Escaped(import.module),
            Escaped(import.field),
            Id(import.name),
            import.ty
        )?;
    }
    let table_size = module.table.len() + 1;
    writeln!(
        out,
        "  (table $__indirect_function_table {table_size} {table_size} funcref)"
    )?;
    writeln!(out, "  (memory $memory {})", module.memory_pages)?;
    writeln!(
        out,
        "  (global $__stack_pointer (mut i32) (i32.const {}))",
        module.stack_pointer as i32
    )?;
    for export in &module.exports {
        match export.kind {
            | ExportKind::Function => writeln!(
                out,
                "  (export \"{}\" (func ${}))",
                Escaped(export.name),
                Id(module.function_name(export.index))
            )?,
            | ExportKind::Memory => writeln!(
                out,
                "  (export \"{}\" (memory $memory))",
                Escaped(export.name)
            )?,
        }
    }
    if let Some(start) = module.start {
        writeln!(out, "  (start ${})", Id(module.function_name(start)))?;
    }
    if !module.table.is_empty() {
        write!(out, "  (elem (i32.const 1) func")?;
        for &function in &module.table {
            write!(out, " ${}", Id(module.function_name(function)))?;
        }
        writeln!(out, ")")?;
    }
    for function in &module.functions {
        let ty = module.types[function.ty as usize];
        write!(
            out,
            "  (func ${} (type $t{})",
            Id(function.name),
            function.ty
        )?;
        for (index, param) in ty.params.iter().enumerate() {
            write!(
                out,
                " (param ${} {})",
                Id(function.local_names[index]),
                param.name()
            )?;
        }
        if !ty.results.is_empty() {
            write!(out, " (result")?;
            for result in ty.results {
                write!(out, " {}", result.name())?;
            }
            write!(out, ")")?;
        }
        writeln!(out)?;
        for (offset, local) in function.locals.iter().enumerate() {
            writeln!(
                out,
                "    (local ${} {})",
                Id(function.local_names[ty.params.len() + offset]),
                local.name()
            )?;
        }
        let local = |index: u32| Id(function.local_names[index as usize]);
        let mut depth = 2_usize;
        for &instr in function.code {
            if matches!(instr, Instr::End | Instr::Else) {
                depth = depth.saturating_sub(1);
            }
            let indent = depth * 2;
            match instr {
                | Instr::Mark { block, inst, line } => {
                    write!(out, "{:indent$};; block{block}", "")?;
                    if inst == u32::MAX {
                        write!(out, " terminator")?;
                    } else {
                        write!(out, " inst {inst}")?;
                    }
                    if line != 0 {
                        write!(out, ", line {line}")?;
                    }
                    writeln!(out)?;
                    continue;
                },
                | _ => {
                    lines.extend([out.lines]);
                    write!(out, "{:indent$}", "")?;
                },
            }
            match instr {
                | Instr::Unreachable => write!(out, "unreachable")?,
                | Instr::Block => write!(out, "block")?,
                | Instr::Loop => write!(out, "loop")?,
                | Instr::If => write!(out, "if")?,
                | Instr::Else => write!(out, "else")?,
                | Instr::End => write!(out, "end")?,
                | Instr::Br(label) => write!(out, "br {label}")?,
                | Instr::BrIf(label) => write!(out, "br_if {label}")?,
                | Instr::BrTable {
                    start,
                    count,
                    default,
                } => {
                    write!(out, "br_table")?;
                    for label in &function.labels[start as usize..(start + count) as usize] {
                        write!(out, " {label}")?;
                    }
                    write!(out, " {default}")?;
                },
                | Instr::Return => write!(out, "return")?,
                | Instr::Call(callee) => write!(out, "call ${}", Id(module.function_name(callee)))?,
                | Instr::CallIndirect(ty) => write!(out, "call_indirect (type $t{ty})")?,
                | Instr::Drop => write!(out, "drop")?,
                | Instr::Select => write!(out, "select")?,
                | Instr::LocalGet(index) => write!(out, "local.get ${}", local(index))?,
                | Instr::LocalSet(index) => write!(out, "local.set ${}", local(index))?,
                | Instr::LocalTee(index) => write!(out, "local.tee ${}", local(index))?,
                | Instr::GlobalGet(_) => write!(out, "global.get $__stack_pointer")?,
                | Instr::GlobalSet(_) => write!(out, "global.set $__stack_pointer")?,
                | Instr::Memory {
                    opcode,
                    align,
                    offset,
                } => {
                    write!(out, "{}", memory_name(opcode))?;
                    if offset != 0 {
                        write!(out, " offset={offset}")?;
                    }
                    write!(out, " align={}", 1_u32 << align)?;
                },
                | Instr::I32Const(value) => write!(out, "i32.const {value}")?,
                | Instr::I64Const(value) => write!(out, "i64.const {value}")?,
                | Instr::F32Const(bits) => write!(out, "f32.const {}", Float32(bits))?,
                | Instr::F64Const(bits) => write!(out, "f64.const {}", Float64(bits))?,
                | Instr::Num(opcode) => write!(out, "{}", num_name(opcode))?,
                | Instr::TruncSat(sub) => write!(out, "{}", trunc_sat_name(sub))?,
                | Instr::MemoryCopy => write!(out, "memory.copy")?,
                | Instr::MemoryFill => write!(out, "memory.fill")?,
                | Instr::MemorySize => write!(out, "memory.size")?,
                | Instr::MemoryGrow => write!(out, "memory.grow")?,
                | Instr::Mark { .. } => {},
            }
            writeln!(out)?;
            if matches!(instr, Instr::Block | Instr::Loop | Instr::If | Instr::Else) {
                depth += 1;
            }
        }
        writeln!(out, "  )")?;
    }
    for segment in &module.data {
        write!(out, "  (data (i32.const {}) \"", segment.offset as i32)?;
        for &byte in segment.bytes {
            if byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'.' | b',' | b'_' | b'-') {
                out.write_char(char::from(byte))?;
            } else {
                write!(out, "\\{byte:02x}")?;
            }
        }
        writeln!(out, "\")")?;
    }
    writeln!(out, ")")
}

/// A writer that counts the lines written through it.
struct Counting<'w, W> {
    out:   &'w mut W,
    lines: u32,
}

impl<W: Write> Write for Counting<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.lines += u32::try_from(s.bytes().filter(|&byte| byte == b'\n').count())
            .expect("fewer than 2^32 lines");
        self.out.write_str(s)
    }
}

/// A symbol as a text identifier: `idchar`s (WebAssembly 2.0 §6.3.5) pass
/// through and anything else becomes its code point in hex between
/// backticks. The backtick is an `idchar` too, but it is the escape's
/// delimiter, so it is always escaped and distinct names stay distinct.
struct Id<'a>(&'a str);

impl fmt::Display for Id<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for ch in self.0.chars() {
            let plain = ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '!' | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '/'
                        | ':'
                        | '<'
                        | '='
                        | '>'
                        | '?'
                        | '@'
                        | '\\'
                        | '^'
                        | '_'
                        | '|'
                        | '~'
                );
            if plain {
                f.write_char(ch)?;
            } else {
                write!(f, "`{:x}`", u32::from(ch))?;
            }
        }
        Ok(())
    }
}

/// A string with every byte outside printable ASCII, and quotes and
/// backslashes, escaped.
struct Escaped<'a>(&'a str);

impl fmt::Display for Escaped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in self.0.as_bytes() {
            if (0x20..0x7F).contains(&byte) && byte != b'"' && byte != b'\\' {
                f.write_char(char::from(byte))?;
            } else {
                write!(f, "\\{byte:02x}")?;
            }
        }
        Ok(())
    }
}

/// An `f32` constant's exact value, in hexadecimal float notation.
struct Float32(u32);

impl fmt::Display for Float32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex_float(f, u64::from(self.0), 23, 8)
    }
}

/// An `f64` constant's exact value, in hexadecimal float notation.
struct Float64(u64);

impl fmt::Display for Float64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex_float(f, self.0, 52, 11)
    }
}

/// The WebAssembly text spelling of a binary floating value with
/// `mantissa` fraction bits and `exponent` exponent bits: `inf`, `nan:0x…`
/// with its payload, or `0x1.…p…` exactly.
fn hex_float(f: &mut fmt::Formatter<'_>, bits: u64, mantissa: u32, exponent: u32) -> fmt::Result {
    let sign = bits >> (mantissa + exponent) & 1 != 0;
    let biased = (bits >> mantissa) & ((1 << exponent) - 1);
    let fraction = bits & ((1 << mantissa) - 1);
    if sign {
        f.write_char('-')?;
    }
    let max = (1 << exponent) - 1;
    if biased == max {
        return if fraction == 0 {
            f.write_str("inf")
        } else {
            write!(f, "nan:0x{fraction:x}")
        };
    }
    if biased == 0 && fraction == 0 {
        return f.write_str("0x0p+0");
    }
    let exponent_bias = (1_i64 << (exponent - 1)) - 1;
    let (lead, power) = if biased == 0 {
        (0, 1 - exponent_bias)
    } else {
        (1, biased.cast_signed() - exponent_bias)
    };
    // Pad the fraction to whole hex digits, then drop trailing zeros.
    let digits = mantissa.div_ceil(4);
    let mut padded = fraction << (digits * 4 - mantissa);
    let mut count = digits;
    while count > 0 && padded.trailing_zeros() >= 4 {
        padded >>= 4;
        count -= 1;
    }
    if count == 0 {
        write!(f, "0x{lead}p{power:+}")
    } else {
        write!(
            f,
            "0x{lead}.{padded:0width$x}p{power:+}",
            width = count as usize
        )
    }
}
