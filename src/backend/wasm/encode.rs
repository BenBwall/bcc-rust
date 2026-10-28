//! The binary format: a [`WasmModule`] as the bytes of a `.wasm` file, and
//! the offset of each IR position's code in them.
//!
//! Sections follow in the order the specification requires: type, import,
//! function, table, memory, global, export, start, element, code and data,
//! then two custom sections: `name` (function and local names, which
//! engines show in stack traces and debuggers) and `bcc.sourcemap`, which
//! maps code offsets to IR positions and source lines:
//!
//! ```text
//! bcc.sourcemap := version:u8 (= 1) count:u32 entry*count
//! entry         := offset:u32 function:u32 block:u32 inst:u32 line:u32
//! ```
//!
//! Every number after the version is an unsigned LEB128. `offset` is the
//! byte offset, from the start of the module, of the first instruction of
//! the code for IR instruction `inst` of block `block` of function
//! `function` (its index in the function index space, imports included);
//! `inst` is `0xFFFFFFFF` for the block's terminator, and `line` is the
//! source line, 0 when unknown. Entries are in offset order and each
//! position's code extends to the next entry's offset or the end of its
//! function. Custom sections follow the code, so their presence does not
//! move it.
//!
//! Reference: WebAssembly Core Specification 2.0, §5 "Binary Format", and
//! appendix §7.4 "Custom Sections" for the name section.

use super::{
    instr::Instr,
    module::{
        ExportKind,
        WasmModule,
    },
    u32_of,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// One source-map entry: see the module documentation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SourceEntry {
    pub(crate) offset:   u32,
    pub(crate) function: u32,
    pub(crate) block:    u32,
    pub(crate) inst:     u32,
    pub(crate) line:     u32,
}

/// The encoded module and the code offset of each marked IR position.
pub(crate) struct Encoded<'a> {
    pub(crate) bytes:   ArenaVec<'a, u8>,
    pub(crate) entries: ArenaVec<'a, SourceEntry>,
    /// Each defined function's code, as `[start, end)` byte offsets from
    /// the module's start, by defined-function position.
    pub(crate) bodies:  ArenaVec<'a, (u32, u32)>,
    /// The offset of every instruction, in the order of the functions'
    /// code, marks left out.
    pub(crate) offsets: ArenaVec<'a, u32>,
}

/// Encodes `module`.
pub(crate) fn encode<'a>(module: &WasmModule<'_>, arena: &'a Bump) -> Encoded<'a> {
    let mut out = ArenaVec::new_in(arena);
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&1_u32.to_le_bytes());
    let mut section = ArenaVec::new_in(arena);

    // Type section.
    uleb(&mut section, module.types.len() as u64);
    for ty in &module.types {
        section.push(0x60);
        uleb(&mut section, ty.params.len() as u64);
        section.extend(ty.params.iter().map(|param| param.code()));
        uleb(&mut section, ty.results.len() as u64);
        section.extend(ty.results.iter().map(|result| result.code()));
    }
    finish_section(&mut out, &mut section, 1);

    if !module.imports.is_empty() {
        uleb(&mut section, module.imports.len() as u64);
        for import in &module.imports {
            name(&mut section, import.module);
            name(&mut section, import.field);
            section.push(0x00);
            uleb(&mut section, u64::from(import.ty));
        }
        finish_section(&mut out, &mut section, 2);
    }

    uleb(&mut section, module.functions.len() as u64);
    for function in &module.functions {
        uleb(&mut section, u64::from(function.ty));
    }
    finish_section(&mut out, &mut section, 3);

    // One funcref table holding null and the address-taken functions.
    let table_size = module.table.len() as u64 + 1;
    uleb(&mut section, 1);
    section.push(0x70);
    section.push(0x01);
    uleb(&mut section, table_size);
    uleb(&mut section, table_size);
    finish_section(&mut out, &mut section, 4);

    // One memory without a maximum.
    uleb(&mut section, 1);
    section.push(0x00);
    uleb(&mut section, u64::from(module.memory_pages));
    finish_section(&mut out, &mut section, 5);

    // Global 0: the mutable stack pointer.
    uleb(&mut section, 1);
    section.push(0x7F);
    section.push(0x01);
    section.push(0x41);
    sleb(&mut section, i64::from(module.stack_pointer as i32));
    section.push(0x0B);
    finish_section(&mut out, &mut section, 6);

    uleb(&mut section, module.exports.len() as u64);
    for export in &module.exports {
        name(&mut section, export.name);
        section.push(match export.kind {
            | ExportKind::Function => 0x00,
            | ExportKind::Memory => 0x02,
        });
        uleb(&mut section, u64::from(export.index));
    }
    finish_section(&mut out, &mut section, 7);

    if let Some(start) = module.start {
        uleb(&mut section, u64::from(start));
        finish_section(&mut out, &mut section, 8);
    }

    if !module.table.is_empty() {
        uleb(&mut section, 1);
        section.push(0x00);
        section.push(0x41);
        sleb(&mut section, 1);
        section.push(0x0B);
        uleb(&mut section, module.table.len() as u64);
        for &function in &module.table {
            uleb(&mut section, u64::from(function));
        }
        finish_section(&mut out, &mut section, 9);
    }

    // Code section: written in place, so offsets are absolute.
    let mut entries = ArenaVec::new_in(arena);
    let mut bodies = ArenaVec::with_capacity_in(module.functions.len(), arena);
    let mut code = ArenaVec::new_in(arena);
    let mut body = ArenaVec::new_in(arena);
    let mut marks = ArenaVec::new_in(arena);
    let mut placed = ArenaVec::new_in(arena);
    let mut offsets = ArenaVec::new_in(arena);
    let mut function_offsets = ArenaVec::new_in(arena);
    uleb(&mut code, module.functions.len() as u64);
    for (position, function) in module.functions.iter().enumerate() {
        body.clear();
        marks.clear();
        function_offsets.clear();
        // Locals, run-length encoded.
        let mut runs = 0_u64;
        let mut previous = None;
        for &local in function.locals {
            if previous != Some(local) {
                runs += 1;
                previous = Some(local);
            }
        }
        uleb(&mut body, runs);
        let mut index = 0;
        while index < function.locals.len() {
            let local = function.locals[index];
            let run = function.locals[index..]
                .iter()
                .take_while(|&&other| other == local)
                .count();
            uleb(&mut body, run as u64);
            body.push(local.code());
            index += run;
        }
        for &instr in function.code {
            if let Instr::Mark { block, inst, line } = instr {
                marks.push((body.len(), block, inst, line));
                continue;
            }
            function_offsets.push(body.len());
            instruction(&mut body, instr, function.labels);
        }
        body.push(0x0B);
        let size = body.len() as u64;
        placed.push((code.len(), leb_len(size), size, position));
        uleb(&mut code, size);
        let start = code.len();
        code.extend_from_slice(&body);
        for &(at, block, inst, line) in &marks {
            entries.push(SourceEntry {
                offset: u32_of(start + at),
                function: u32_of(module.imports.len() + position),
                block,
                inst,
                line,
            });
        }
        offsets.extend(function_offsets.iter().map(|&at| u32_of(start + at)));
        bodies.push((u32_of(start), u32_of(code.len())));
    }
    // The code section's header precedes its contents in the module.
    let header = out.len() + 1 + leb_len(code.len() as u64);
    for entry in &mut entries {
        entry.offset += u32_of(header);
    }
    for body in &mut bodies {
        body.0 += u32_of(header);
        body.1 += u32_of(header);
    }
    for offset in &mut offsets {
        *offset += u32_of(header);
    }
    out.push(10);
    uleb(&mut out, code.len() as u64);
    out.extend_from_slice(&code);

    if !module.data.is_empty() {
        uleb(&mut section, module.data.len() as u64);
        for segment in &module.data {
            section.push(0x00);
            section.push(0x41);
            sleb(&mut section, i64::from(segment.offset as i32));
            section.push(0x0B);
            uleb(&mut section, segment.bytes.len() as u64);
            section.extend_from_slice(segment.bytes);
        }
        finish_section(&mut out, &mut section, 11);
    }

    names(&mut out, &mut section, module, arena);
    source_map(&mut out, &mut section, &entries);
    Encoded {
        bytes: out,
        entries,
        bodies,
        offsets,
    }
}

/// The `name` custom section: function names (subsection 1) and local
/// names (subsection 2).
fn names(
    out: &mut ArenaVec<'_, u8>,
    section: &mut ArenaVec<'_, u8>,
    module: &WasmModule<'_>,
    arena: &Bump,
) {
    name(section, "name");
    let mut sub = ArenaVec::new_in(arena);
    let count = module.imports.len() + module.functions.len();
    uleb(&mut sub, count as u64);
    for index in 0..count {
        uleb(&mut sub, index as u64);
        name(&mut sub, module.function_name(u32_of(index)));
    }
    section.push(1);
    uleb(section, sub.len() as u64);
    section.extend_from_slice(&sub);
    sub.clear();
    uleb(&mut sub, module.functions.len() as u64);
    for (position, function) in module.functions.iter().enumerate() {
        uleb(&mut sub, (module.imports.len() + position) as u64);
        let named = function
            .local_names
            .iter()
            .filter(|name| !name.is_empty())
            .count();
        uleb(&mut sub, named as u64);
        for (local, local_name) in function.local_names.iter().enumerate() {
            if !local_name.is_empty() {
                uleb(&mut sub, local as u64);
                name(&mut sub, local_name);
            }
        }
    }
    section.push(2);
    uleb(section, sub.len() as u64);
    section.extend_from_slice(&sub);
    finish_section(out, section, 0);
}

/// The `bcc.sourcemap` custom section; see the module documentation.
fn source_map(out: &mut ArenaVec<'_, u8>, section: &mut ArenaVec<'_, u8>, entries: &[SourceEntry]) {
    name(section, "bcc.sourcemap");
    section.push(1);
    uleb(section, entries.len() as u64);
    for entry in entries {
        uleb(section, u64::from(entry.offset));
        uleb(section, u64::from(entry.function));
        uleb(section, u64::from(entry.block));
        uleb(section, u64::from(entry.inst));
        uleb(section, u64::from(entry.line));
    }
    finish_section(out, section, 0);
}

/// Appends one instruction's encoding.
fn instruction(out: &mut ArenaVec<'_, u8>, instr: Instr, labels: &[u32]) {
    match instr {
        | Instr::Unreachable => out.push(0x00),
        | Instr::Block => out.extend_from_slice(&[0x02, 0x40]),
        | Instr::Loop => out.extend_from_slice(&[0x03, 0x40]),
        | Instr::If => out.extend_from_slice(&[0x04, 0x40]),
        | Instr::Else => out.push(0x05),
        | Instr::End => out.push(0x0B),
        | Instr::Br(depth) => {
            out.push(0x0C);
            uleb(out, u64::from(depth));
        },
        | Instr::BrIf(depth) => {
            out.push(0x0D);
            uleb(out, u64::from(depth));
        },
        | Instr::BrTable {
            start,
            count,
            default,
        } => {
            out.push(0x0E);
            uleb(out, u64::from(count));
            for &label in &labels[start as usize..(start + count) as usize] {
                uleb(out, u64::from(label));
            }
            uleb(out, u64::from(default));
        },
        | Instr::Return => out.push(0x0F),
        | Instr::Call(function) => {
            out.push(0x10);
            uleb(out, u64::from(function));
        },
        | Instr::CallIndirect(ty) => {
            out.push(0x11);
            uleb(out, u64::from(ty));
            out.push(0x00);
        },
        | Instr::Drop => out.push(0x1A),
        | Instr::Select => out.push(0x1B),
        | Instr::LocalGet(index) => {
            out.push(0x20);
            uleb(out, u64::from(index));
        },
        | Instr::LocalSet(index) => {
            out.push(0x21);
            uleb(out, u64::from(index));
        },
        | Instr::LocalTee(index) => {
            out.push(0x22);
            uleb(out, u64::from(index));
        },
        | Instr::GlobalGet(index) => {
            out.push(0x23);
            uleb(out, u64::from(index));
        },
        | Instr::GlobalSet(index) => {
            out.push(0x24);
            uleb(out, u64::from(index));
        },
        | Instr::Memory {
            opcode,
            align,
            offset,
        } => {
            out.push(opcode);
            uleb(out, u64::from(align));
            uleb(out, u64::from(offset));
        },
        | Instr::I32Const(value) => {
            out.push(0x41);
            sleb(out, i64::from(value));
        },
        | Instr::I64Const(value) => {
            out.push(0x42);
            sleb(out, value);
        },
        | Instr::F32Const(bits) => {
            out.push(0x43);
            out.extend_from_slice(&bits.to_le_bytes());
        },
        | Instr::F64Const(bits) => {
            out.push(0x44);
            out.extend_from_slice(&bits.to_le_bytes());
        },
        | Instr::Num(opcode) => out.push(opcode),
        | Instr::TruncSat(sub) => {
            out.push(0xFC);
            uleb(out, u64::from(sub));
        },
        | Instr::MemoryCopy => out.extend_from_slice(&[0xFC, 10, 0x00, 0x00]),
        | Instr::MemoryFill => out.extend_from_slice(&[0xFC, 11, 0x00]),
        | Instr::MemorySize => out.extend_from_slice(&[0x3F, 0x00]),
        | Instr::MemoryGrow => out.extend_from_slice(&[0x40, 0x00]),
        | Instr::Mark { .. } => {},
    }
}

fn finish_section(out: &mut ArenaVec<'_, u8>, section: &mut ArenaVec<'_, u8>, id: u8) {
    out.push(id);
    uleb(out, section.len() as u64);
    out.extend_from_slice(section);
    section.clear();
}

fn name(out: &mut ArenaVec<'_, u8>, text: &str) {
    uleb(out, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}

pub(crate) fn uleb(out: &mut ArenaVec<'_, u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

pub(crate) fn sleb(out: &mut ArenaVec<'_, u8>, mut value: i64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        if done {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn leb_len(mut value: u64) -> usize {
    let mut length = 1;
    while value >= 0x80 {
        value >>= 7;
        length += 1;
    }
    length
}
