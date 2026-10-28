//! Tables (WebAssembly 2.0 §4.2.7) and the table instructions (§4.4.6):
//! arrays of references that `call_indirect` reads, as C function pointers
//! compile to.

use super::{
    Effect,
    Machine,
    Trap,
    TrapKind,
};
use crate::{
    instr::Instr,
    value::Value,
};

/// The most entries the stepper lets a table grow to.
pub const MAX_ENTRIES: u32 = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub elements: Vec<Value>,
    pub max:      Option<u32>,
}

impl Table {
    fn check(&self, start: u32, length: u32) -> Result<usize, Trap> {
        let end = u64::from(start) + u64::from(length);
        if end > self.elements.len() as u64 {
            return Err(Trap::new(
                TrapKind::TableOutOfBounds,
                format!(
                    "out of bounds table access: entries {start}..{end} of a table with {}",
                    self.elements.len()
                ),
            ));
        }
        Ok(start as usize)
    }
}

impl Machine {
    pub(super) fn table_instruction(&mut self, instr: &Instr) -> Result<(), Trap> {
        match *instr {
            | Instr::TableGet(table) => {
                let index = self.pop_i32()? as u32;
                let table_ref = self.table(table)?;
                let at = table_ref.check(index, 1)?;
                let value = table_ref.elements[at];
                self.event.effect = Effect::Table { index: table };
                self.push(value)?;
            },
            | Instr::TableSet(table) => {
                let value = self.pop()?;
                let index = self.pop_i32()? as u32;
                let table_ref = self.table_mut(table)?;
                let at = table_ref.check(index, 1)?;
                table_ref.elements[at] = value;
                self.event.effect = Effect::Table { index: table };
            },
            | Instr::TableSize(table) => {
                let size = self.table(table)?.elements.len() as i32;
                self.push(Value::I32(size))?;
            },
            | Instr::TableGrow(table) => {
                let delta = self.pop_i32()? as u32;
                let init = self.pop()?;
                let table_ref = self.table_mut(table)?;
                let previous = table_ref.elements.len() as u32;
                let limit = table_ref.max.unwrap_or(u32::MAX).min(MAX_ENTRIES);
                let result = match previous.checked_add(delta) {
                    | Some(size) if size <= limit => {
                        table_ref.elements.resize(size as usize, init);
                        previous as i32
                    },
                    | _ => -1,
                };
                self.push(Value::I32(result))?;
            },
            | Instr::TableFill(table) => {
                let length = self.pop_i32()? as u32;
                let value = self.pop()?;
                let start = self.pop_i32()? as u32;
                let table_ref = self.table_mut(table)?;
                let at = table_ref.check(start, length)?;
                table_ref.elements[at..at + length as usize].fill(value);
            },
            | Instr::TableCopy { dst, src } => {
                let length = self.pop_i32()? as u32;
                let source = self.pop_i32()? as u32;
                let destination = self.pop_i32()? as u32;
                let from = self.table(src)?;
                let at = from.check(source, length)?;
                let values = from.elements[at..at + length as usize].to_vec();
                let to = self.table_mut(dst)?;
                let at = to.check(destination, length)?;
                to.elements[at..at + values.len()].copy_from_slice(&values);
            },
            | Instr::TableInit { elem, table } => {
                let length = self.pop_i32()? as u32;
                let source = self.pop_i32()? as u32;
                let destination = self.pop_i32()? as u32;
                let segment = self
                    .elements
                    .get(elem as usize)
                    .cloned()
                    .unwrap_or_default();
                let end = u64::from(source) + u64::from(length);
                if end > segment.len() as u64 {
                    return Err(Trap::new(
                        TrapKind::TableOutOfBounds,
                        format!(
                            "out of bounds table access: table.init read entries {source}..{end} \
                             of a {}-entry segment",
                            segment.len()
                        ),
                    ));
                }
                let to = self.table_mut(table)?;
                let at = to.check(destination, length)?;
                to.elements[at..at + length as usize]
                    .copy_from_slice(&segment[source as usize..end as usize]);
            },
            | Instr::ElemDrop(elem) =>
                if let Some(segment) = self.elements.get_mut(elem as usize) {
                    segment.clear();
                },
            | _ =>
                return Err(Trap::new(
                    TrapKind::Internal,
                    "stepper error: not a table op",
                )),
        }
        Ok(())
    }

    fn table(&self, index: u32) -> Result<&Table, Trap> {
        self.tables
            .get(index as usize)
            .ok_or_else(|| Trap::new(TrapKind::Internal, "stepper error: table"))
    }

    fn table_mut(&mut self, index: u32) -> Result<&mut Table, Trap> {
        self.tables
            .get_mut(index as usize)
            .ok_or_else(|| Trap::new(TrapKind::Internal, "stepper error: table"))
    }
}
