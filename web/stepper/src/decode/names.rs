//! The `name` custom section (WebAssembly 2.0 §7.4.1), with the extended
//! subsections toolchains emit: labels (3), globals (7) and data segments
//! (9). Unknown subsections are skipped.

use super::{
    DecodeError,
    reader::Reader,
};
use crate::module::Names;

pub fn section(mut reader: Reader<'_>) -> Result<Names, DecodeError> {
    let mut names = Names::default();
    while !reader.is_empty() {
        let id = reader.byte()?;
        let size = reader.u32()? as usize;
        let mut sub = reader.split(size)?;
        match id {
            | 0 => names.module = Some(sub.name()?),
            | 1 => names.functions = name_map(&mut sub)?,
            | 2 => names.locals = indirect_map(&mut sub)?,
            | 3 => names.labels = indirect_map(&mut sub)?,
            | 7 => names.globals = name_map(&mut sub)?,
            | 9 => names.data = name_map(&mut sub)?,
            | _ => {},
        }
    }
    Ok(names)
}

fn name_map(reader: &mut Reader<'_>) -> Result<Vec<(u32, String)>, DecodeError> {
    let count = reader.count()?;
    (0..count)
        .map(|_| Ok((reader.u32()?, reader.name()?)))
        .collect()
}

#[expect(clippy::type_complexity, reason = "The section's own nesting.")]
fn indirect_map(reader: &mut Reader<'_>) -> Result<Vec<(u32, Vec<(u32, String)>)>, DecodeError> {
    let count = reader.count()?;
    (0..count)
        .map(|_| Ok((reader.u32()?, name_map(reader)?)))
        .collect()
}
