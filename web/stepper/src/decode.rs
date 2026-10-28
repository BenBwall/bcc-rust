//! The binary format (WebAssembly 2.0 §5): a module's sections in order,
//! each function body decoded into [`Instr`]s with their byte offsets, and
//! the `name` section. Other custom sections are kept whole, for the DWARF
//! line tables `lines.rs` reads.
//!
//! Decoding checks the format itself (lengths, section order, opcode
//! immediates, block nesting); `validate.rs` checks types afterwards.

mod code;
mod names;
mod reader;

use std::fmt;

use reader::Reader;

use crate::module::{
    BlockType,
    ConstExpr,
    Custom,
    Data,
    Element,
    Export,
    ExternKind,
    Func,
    FuncKind,
    FuncType,
    Global,
    GlobalType,
    Import,
    ImportKind,
    Limits,
    Module,
    SegmentMode,
    TableType,
    ValType,
};

/// Decode a module from its binary format.
///
/// # Errors
/// When `bytes` is not a well-formed module, or uses a feature the stepper
/// does not support (SIMD, threads, multiple memories, exceptions).
pub fn decode(bytes: &[u8]) -> Result<Module, DecodeError> {
    let mut reader = Reader::new(bytes);
    if reader.take(4).ok() != Some(b"\0asm".as_slice()) {
        return Err(reader.error("this is not a WebAssembly module (no \\0asm magic number)"));
    }
    if reader.take(4)? != [1, 0, 0, 0] {
        return Err(reader.error("only version 1 of the binary format is supported"));
    }
    let mut module = Module::default();
    let mut declared = Vec::new();
    let mut last = 0;
    let mut data_count = None;
    while !reader.is_empty() {
        let id = reader.byte()?;
        let size = reader.u32()? as usize;
        let mut section = reader.split(size)?;
        if id != 0 {
            // Sections appear in a fixed order; DataCount (12) sits between
            // Element (9) and Code (10).
            let rank = if id == 12 { 19 } else { u32::from(id) * 2 };
            if rank <= last {
                return Err(section.error(format!("section {id} is out of order or repeated")));
            }
            last = rank;
        }
        match id {
            | 0 => custom(&mut section, &mut module)?,
            | 1 => module.types = vector(&mut section, func_type)?,
            | 2 => imports(&mut section, &mut module)?,
            | 3 => declared = vector(&mut section, Reader::u32)?,
            | 4 => module.tables = vector(&mut section, table_type)?,
            | 5 => module.memories = vector(&mut section, limits)?,
            | 6 => module.globals = vector(&mut section, global)?,
            | 7 => module.exports = vector(&mut section, export)?,
            | 8 => module.start = Some(section.u32()?),
            | 9 => module.elements = vector(&mut section, element)?,
            | 12 => data_count = Some(section.u32()?),
            | 10 => {
                module.code_base = section.offset();
                let bodies = code::section(&mut section, &declared)?;
                for (ty, body) in declared.iter().zip(bodies) {
                    module.funcs.push(Func {
                        ty:   *ty,
                        kind: FuncKind::Defined(body),
                    });
                }
            },
            | 11 => module.data = vector(&mut section, data)?,
            | _ => return Err(section.error(format!("unknown section id {id}"))),
        }
        if id != 0 && !section.is_empty() {
            return Err(section.error(format!("section {id} is longer than its contents")));
        }
    }
    let defined = module
        .funcs
        .iter()
        .filter(|func| matches!(func.kind, FuncKind::Defined(_)))
        .count();
    if defined != declared.len() {
        return Err(reader.error("the function and code sections disagree on the count"));
    }
    if data_count.is_some_and(|count| count as usize != module.data.len()) {
        return Err(reader.error("the data count section disagrees with the data section"));
    }
    let memories = module.memories.len()
        + module
            .imports
            .iter()
            .filter(|import| matches!(import.kind, ImportKind::Memory(_)))
            .count();
    if memories > 1 {
        return Err(reader.error("multiple memories are not supported"));
    }
    Ok(module)
}

/// A decoding failure at a module offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub offset:  u32,
    pub message: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (at byte {:#x})", self.message, self.offset)
    }
}

fn vector<'a, T>(
    reader: &mut Reader<'a>,
    mut item: impl FnMut(&mut Reader<'a>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    let count = reader.count()?;
    (0..count).map(|_| item(reader)).collect()
}

fn custom(section: &mut Reader<'_>, module: &mut Module) -> Result<(), DecodeError> {
    let name = section.name()?;
    let rest = section.rest();
    if name == "name" {
        // A malformed name section only loses names (§7.4).
        if let Ok(names) = names::section(Reader::new(rest)) {
            module.names = names;
        }
    } else {
        module.customs.push(Custom {
            name,
            bytes: rest.to_vec(),
        });
    }
    Ok(())
}

pub(crate) fn value_type(reader: &mut Reader<'_>) -> Result<ValType, DecodeError> {
    let at = reader.offset();
    let error = |message: String| DecodeError {
        offset: at,
        message,
    };
    match reader.byte()? {
        | 0x7F => Ok(ValType::I32),
        | 0x7E => Ok(ValType::I64),
        | 0x7D => Ok(ValType::F32),
        | 0x7C => Ok(ValType::F64),
        | 0x70 => Ok(ValType::FuncRef),
        | 0x6F => Ok(ValType::ExternRef),
        | 0x7B => Err(error("SIMD (v128) is not supported by the stepper".into())),
        | other => Err(error(format!("unknown value type {other:#04x}"))),
    }
}

fn reference_type(reader: &mut Reader<'_>) -> Result<ValType, DecodeError> {
    let ty = value_type(reader)?;
    if ty.is_reference() {
        Ok(ty)
    } else {
        Err(reader.error("expected a reference type"))
    }
}

pub(crate) fn block_type(reader: &mut Reader<'_>) -> Result<BlockType, DecodeError> {
    match reader.peek() {
        | Some(0x40) => {
            reader.byte()?;
            Ok(BlockType::Empty)
        },
        | Some(0x7F | 0x7E | 0x7D | 0x7C | 0x70 | 0x6F | 0x7B) =>
            value_type(reader).map(BlockType::Value),
        | _ => {
            let index = reader.signed_block_index()?;
            Ok(BlockType::Type(index))
        },
    }
}

impl Reader<'_> {
    /// A block type's type index, an `s33` (§5.4.1) that must be positive.
    fn signed_block_index(&mut self) -> Result<u32, DecodeError> {
        let index = self.i64()?;
        u32::try_from(index).map_err(|_| self.error("a block type index is out of range"))
    }
}

fn func_type(reader: &mut Reader<'_>) -> Result<FuncType, DecodeError> {
    if reader.byte()? != 0x60 {
        return Err(reader.error("expected a function type (0x60)"));
    }
    Ok(FuncType {
        params:  vector(reader, value_type)?,
        results: vector(reader, value_type)?,
    })
}

fn limits(reader: &mut Reader<'_>) -> Result<Limits, DecodeError> {
    match reader.byte()? {
        | 0x00 => Ok(Limits {
            min: reader.u32()?,
            max: None,
        }),
        | 0x01 => Ok(Limits {
            min: reader.u32()?,
            max: Some(reader.u32()?),
        }),
        | 0x02 | 0x03 => Err(reader.error("shared memories (threads) are not supported")),
        | 0x04..=0x07 => Err(reader.error("64-bit memories are not supported")),
        | other => Err(reader.error(format!("unknown limits flag {other:#04x}"))),
    }
}

fn table_type(reader: &mut Reader<'_>) -> Result<TableType, DecodeError> {
    Ok(TableType {
        element: reference_type(reader)?,
        limits:  limits(reader)?,
    })
}

fn global_type(reader: &mut Reader<'_>) -> Result<GlobalType, DecodeError> {
    let value = value_type(reader)?;
    let mutable = match reader.byte()? {
        | 0 => false,
        | 1 => true,
        | _ => return Err(reader.error("a global's mutability must be 0 or 1")),
    };
    Ok(GlobalType { value, mutable })
}

fn imports(section: &mut Reader<'_>, module: &mut Module) -> Result<(), DecodeError> {
    for _ in 0..section.count()? {
        let module_name = section.name()?;
        let name = section.name()?;
        let kind = match section.byte()? {
            | 0x00 => {
                let ty = section.u32()?;
                module.funcs.push(Func {
                    ty,
                    kind: FuncKind::Import {
                        module: module_name.clone(),
                        name:   name.clone(),
                    },
                });
                ImportKind::Func(ty)
            },
            | 0x01 => ImportKind::Table(table_type(section)?),
            | 0x02 => ImportKind::Memory(limits(section)?),
            | 0x03 => ImportKind::Global(global_type(section)?),
            | 0x04 => return Err(section.error("tag imports (exceptions) are not supported")),
            | other => return Err(section.error(format!("unknown import kind {other:#04x}"))),
        };
        module.imports.push(Import {
            module: module_name,
            name,
            kind,
        });
    }
    Ok(())
}

fn global(reader: &mut Reader<'_>) -> Result<Global, DecodeError> {
    Ok(Global {
        ty:   global_type(reader)?,
        init: const_expr(reader)?,
    })
}

fn export(reader: &mut Reader<'_>) -> Result<Export, DecodeError> {
    let name = reader.name()?;
    let kind = match reader.byte()? {
        | 0x00 => ExternKind::Func,
        | 0x01 => ExternKind::Table,
        | 0x02 => ExternKind::Memory,
        | 0x03 => ExternKind::Global,
        | other => return Err(reader.error(format!("unknown export kind {other:#04x}"))),
    };
    Ok(Export {
        name,
        kind,
        index: reader.u32()?,
    })
}

/// A constant expression: instructions up to `end` (§5.4.9).
fn const_expr(reader: &mut Reader<'_>) -> Result<ConstExpr, DecodeError> {
    let (code, _) = code::expression(reader)?;
    Ok(ConstExpr { code })
}

/// An element segment (§5.5.12), in any of its eight encodings.
fn element(reader: &mut Reader<'_>) -> Result<Element, DecodeError> {
    let flags = reader.u32()?;
    if flags > 7 {
        return Err(reader.error(format!("unknown element segment flags {flags}")));
    }
    let passive_or_declarative = flags & 1 != 0;
    let explicit_table = flags & 2 != 0;
    let expressions = flags & 4 != 0;
    let mode = if passive_or_declarative {
        if explicit_table {
            SegmentMode::Declarative
        } else {
            SegmentMode::Passive
        }
    } else {
        let index = if explicit_table { reader.u32()? } else { 0 };
        SegmentMode::Active {
            index,
            offset: const_expr(reader)?,
        }
    };
    // Flags 0 and 4 imply funcref; the others name the element kind or type.
    let ty = if flags == 0 || flags == 4 {
        ValType::FuncRef
    } else if expressions {
        reference_type(reader)?
    } else {
        if reader.byte()? != 0x00 {
            return Err(reader.error("unknown element kind"));
        }
        ValType::FuncRef
    };
    let items = if expressions {
        vector(reader, const_expr)?
    } else {
        vector(reader, |reader| {
            Ok(ConstExpr {
                code: vec![crate::instr::Instr::RefFunc(reader.u32()?)],
            })
        })?
    };
    Ok(Element { ty, items, mode })
}

/// A data segment (§5.5.14).
fn data(reader: &mut Reader<'_>) -> Result<Data, DecodeError> {
    let mode = match reader.u32()? {
        | 0 => SegmentMode::Active {
            index:  0,
            offset: const_expr(reader)?,
        },
        | 1 => SegmentMode::Passive,
        | 2 => SegmentMode::Active {
            index:  reader.u32()?,
            offset: const_expr(reader)?,
        },
        | other => return Err(reader.error(format!("unknown data segment flags {other}"))),
    };
    let length = reader.u32()? as usize;
    let offset = reader.offset();
    let bytes = reader.take(length)?.to_vec();
    Ok(Data {
        bytes,
        mode,
        offset,
    })
}
