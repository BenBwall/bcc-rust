//! The Code section (WebAssembly 2.0 §5.5.13) and instructions (§5.4).
//! Each body becomes its locals and an instruction list with each
//! instruction's module offset; structured instructions learn the index of
//! their `else` and `end` here, from a stack of open blocks.

use super::{
    DecodeError,
    block_type,
    reader::Reader,
    value_type,
};
use crate::{
    instr::{
        Instr,
        Load,
        MemArg,
        Store,
    },
    module::Body,
};

/// The most locals one function may declare, as engines limit them.
const MAX_LOCALS: u64 = 50_000;

pub fn section(section: &mut Reader<'_>, declared: &[u32]) -> Result<Vec<Body>, DecodeError> {
    let count = section.count()?;
    if count as usize != declared.len() {
        return Err(section.error("the function and code sections disagree on the count"));
    }
    (0..count).map(|_| body(section)).collect()
}

fn body(section: &mut Reader<'_>) -> Result<Body, DecodeError> {
    let start = section.offset();
    let size = section.u32()? as usize;
    let mut reader = section.split(size)?;
    let mut locals = Vec::new();
    let mut total = 0_u64;
    for _ in 0..reader.count()? {
        let count = reader.u32()?;
        let ty = value_type(&mut reader)?;
        total += u64::from(count);
        if total > MAX_LOCALS {
            return Err(reader.error("a function declares too many locals"));
        }
        locals.extend(std::iter::repeat_n(ty, count as usize));
    }
    let (code, offsets) = expression(&mut reader)?;
    if !reader.is_empty() {
        return Err(reader.error("a function body continues past its final end"));
    }
    Ok(Body {
        locals,
        code,
        offsets,
        start,
        end: reader.offset(),
    })
}

/// Instructions up to and including the `end` that closes the expression,
/// with each one's offset.
pub fn expression(reader: &mut Reader<'_>) -> Result<(Vec<Instr>, Vec<u32>), DecodeError> {
    let mut code = Vec::new();
    let mut offsets = Vec::new();
    // Indices of the open structured instructions.
    let mut open: Vec<usize> = Vec::new();
    loop {
        let offset = reader.offset();
        let instr = instruction(reader)?;
        let index = code.len();
        let index32 = u32::try_from(index).map_err(|_| reader.error("a function is too long"))?;
        match &instr {
            | Instr::Block { .. } | Instr::Loop { .. } | Instr::If { .. } => open.push(index),
            | Instr::Else { .. } => {
                let Some(&opener) = open.last() else {
                    return Err(reader.error("else outside an if"));
                };
                match &mut code[opener] {
                    | Instr::If { else_, end, .. } if *else_ == u32::MAX && *end == u32::MAX =>
                        *else_ = index32,
                    | _ => return Err(reader.error("else outside an if")),
                }
            },
            | Instr::End =>
                if let Some(opener) = open.pop() {
                    match &mut code[opener] {
                        | Instr::Block { end, .. } | Instr::Loop { end, .. } => *end = index32,
                        | Instr::If { else_, end, .. } => {
                            *end = index32;
                            if *else_ == u32::MAX {
                                *else_ = index32;
                            } else {
                                let at = *else_ as usize;
                                if let Instr::Else { end } = &mut code[at] {
                                    *end = index32;
                                }
                            }
                        },
                        | _ => {},
                    }
                } else {
                    code.push(instr);
                    offsets.push(offset);
                    return Ok((code, offsets));
                },
            | _ => {},
        }
        code.push(instr);
        offsets.push(offset);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "One arm per opcode, as the format lists them."
)]
fn instruction(reader: &mut Reader<'_>) -> Result<Instr, DecodeError> {
    let at = reader.offset();
    let opcode = reader.byte()?;
    let pending = u32::MAX;
    Ok(match opcode {
        | 0x00 => Instr::Unreachable,
        | 0x01 => Instr::Nop,
        | 0x02 => Instr::Block {
            ty:  block_type(reader)?,
            end: pending,
        },
        | 0x03 => Instr::Loop {
            ty:  block_type(reader)?,
            end: pending,
        },
        | 0x04 => Instr::If {
            ty:    block_type(reader)?,
            else_: pending,
            end:   pending,
        },
        | 0x05 => Instr::Else { end: pending },
        | 0x0B => Instr::End,
        | 0x0C => Instr::Br(reader.u32()?),
        | 0x0D => Instr::BrIf(reader.u32()?),
        | 0x0E => {
            let count = reader.count()?;
            let targets = (0..count)
                .map(|_| reader.u32())
                .collect::<Result<Vec<_>, _>>()?;
            Instr::BrTable {
                targets: targets.into_boxed_slice(),
                default: reader.u32()?,
            }
        },
        | 0x0F => Instr::Return,
        | 0x10 => Instr::Call(reader.u32()?),
        | 0x11 => {
            let ty = reader.u32()?;
            Instr::CallIndirect {
                ty,
                table: reader.u32()?,
            }
        },
        | 0x12 | 0x13 => return Err(error(at, "tail calls are not supported by the stepper")),
        | 0x1A => Instr::Drop,
        | 0x1B => Instr::Select(None),
        | 0x1C => {
            let types = super::vector(reader, value_type)?;
            match types.as_slice() {
                | [ty] => Instr::Select(Some(*ty)),
                | _ => return Err(error(at, "select must name exactly one type")),
            }
        },
        | 0x20 => Instr::LocalGet(reader.u32()?),
        | 0x21 => Instr::LocalSet(reader.u32()?),
        | 0x22 => Instr::LocalTee(reader.u32()?),
        | 0x23 => Instr::GlobalGet(reader.u32()?),
        | 0x24 => Instr::GlobalSet(reader.u32()?),
        | 0x25 => Instr::TableGet(reader.u32()?),
        | 0x26 => Instr::TableSet(reader.u32()?),
        | 0x28..=0x35 => Instr::Load(Load::ALL[usize::from(opcode - 0x28)], memarg(reader)?),
        | 0x36..=0x3E => Instr::Store(Store::ALL[usize::from(opcode - 0x36)], memarg(reader)?),
        | 0x3F => {
            memory_index(reader)?;
            Instr::MemorySize
        },
        | 0x40 => {
            memory_index(reader)?;
            Instr::MemoryGrow
        },
        | 0x41 => Instr::I32Const(reader.i32()?),
        | 0x42 => Instr::I64Const(reader.i64()?),
        | 0x43 => Instr::F32Const(reader.f32_bits()?),
        | 0x44 => Instr::F64Const(reader.f64_bits()?),
        | 0x45..=0xC4 => Instr::Numeric(opcode),
        | 0xD0 => {
            let ty = value_type(reader)?;
            if !ty.is_reference() {
                return Err(error(at, "ref.null needs a reference type"));
            }
            Instr::RefNull(ty)
        },
        | 0xD1 => Instr::RefIsNull,
        | 0xD2 => Instr::RefFunc(reader.u32()?),
        | 0xFC => prefixed(reader, at)?,
        | 0xFD =>
            return Err(error(
                at,
                "SIMD instructions are not supported by the stepper",
            )),
        | 0xFE =>
            return Err(error(
                at,
                "atomic instructions are not supported by the stepper",
            )),
        | 0x06..=0x0A | 0x18 | 0x19 | 0x1F => {
            return Err(error(
                at,
                "exception handling is not supported by the stepper",
            ));
        },
        | other => return Err(error(at, format!("unknown opcode {other:#04x}"))),
    })
}

/// The `0xFC` instructions: saturating truncation and bulk memory and table
/// operations.
fn prefixed(reader: &mut Reader<'_>, at: u32) -> Result<Instr, DecodeError> {
    let sub = reader.u32()?;
    Ok(match sub {
        | 0..=7 => Instr::TruncSat(sub as u8),
        | 8 => {
            let segment = reader.u32()?;
            memory_index(reader)?;
            Instr::MemoryInit(segment)
        },
        | 9 => Instr::DataDrop(reader.u32()?),
        | 10 => {
            memory_index(reader)?;
            memory_index(reader)?;
            Instr::MemoryCopy
        },
        | 11 => {
            memory_index(reader)?;
            Instr::MemoryFill
        },
        | 12 => {
            let elem = reader.u32()?;
            Instr::TableInit {
                elem,
                table: reader.u32()?,
            }
        },
        | 13 => Instr::ElemDrop(reader.u32()?),
        | 14 => {
            let dst = reader.u32()?;
            Instr::TableCopy {
                dst,
                src: reader.u32()?,
            }
        },
        | 15 => Instr::TableGrow(reader.u32()?),
        | 16 => Instr::TableSize(reader.u32()?),
        | 17 => Instr::TableFill(reader.u32()?),
        | other => return Err(error(at, format!("unknown instruction 0xfc {other}"))),
    })
}

fn memarg(reader: &mut Reader<'_>) -> Result<MemArg, DecodeError> {
    let align = reader.u32()?;
    if align & 0x40 != 0 {
        return Err(reader.error("multiple memories are not supported"));
    }
    Ok(MemArg {
        align,
        offset: reader.u32()?,
    })
}

/// The memory index byte of a memory instruction, which must be 0.
fn memory_index(reader: &mut Reader<'_>) -> Result<(), DecodeError> {
    if reader.byte()? == 0 {
        Ok(())
    } else {
        Err(reader.error("multiple memories are not supported"))
    }
}

fn error(offset: u32, message: impl Into<String>) -> DecodeError {
    DecodeError {
        offset,
        message: message.into(),
    }
}
