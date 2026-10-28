//! Source lines for instructions: which line of the C source each
//! instruction came from.
//!
//! A [`LineMap`] is a sorted list of `(module offset, line, column)` rows;
//! each row covers the instructions from its offset to the next row's. Line
//! 0 means "no source line" (compiler-generated code), as in DWARF. A
//! compiler can hand the stepper rows directly (bcc's WebAssembly back
//! end); for a module built by clang with `-g`, [`LineMap::from_dwarf`]
//! reads them from the `.debug_line` custom section (DWARF 4 or 5, §6.2),
//! whose addresses are offsets into the Code section. Rows from files other
//! than the compilation unit's own (headers) become line 0.

use crate::module::Module;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMap {
    rows: Vec<Row>,
}

/// From `offset` on, code comes from `line` (1-based; 0 for none) and
/// `column` (1-based; 0 for unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub offset: u32,
    pub line:   u32,
    pub column: u32,
}

impl LineMap {
    /// A map from rows in any order.
    pub fn new(mut rows: Vec<Row>) -> Self {
        rows.sort_by_key(|row| row.offset);
        // Of rows at one offset, the last with a line wins: a sequence's end
        // may share its address with the next one's start.
        let mut merged: Vec<Row> = Vec::with_capacity(rows.len());
        for row in rows {
            match merged.last_mut() {
                | Some(last) if last.offset == row.offset =>
                    if row.line != 0 || last.line == 0 {
                        *last = row;
                    },
                | _ => merged.push(row),
            }
        }
        Self { rows: merged }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(|row| row.line == 0)
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The source line of the instruction at module `offset`, if any.
    pub fn line_at(&self, offset: u32) -> Option<u32> {
        self.row_at(offset)
            .map(|row| row.line)
            .filter(|&line| line > 0)
    }

    pub fn row_at(&self, offset: u32) -> Option<Row> {
        let index = self.rows.partition_point(|row| row.offset <= offset);
        index.checked_sub(1).map(|index| self.rows[index])
    }

    /// The line table in `module`'s DWARF, if it has one.
    pub fn from_dwarf(module: &Module) -> Option<Self> {
        let table = module.custom(".debug_line")?;
        let strings = module.custom(".debug_line_str").unwrap_or_default();
        let strs = module.custom(".debug_str").unwrap_or_default();
        let mut rows = Vec::new();
        let mut reader = Dwarf {
            bytes:    table,
            position: 0,
        };
        while reader.position < table.len() {
            if program(&mut reader, strings, strs, module.code_base, &mut rows).is_none() {
                break;
            }
        }
        (!rows.is_empty()).then(|| Self::new(rows))
    }
}

struct Dwarf<'a> {
    bytes:    &'a [u8],
    position: usize,
}

impl Dwarf<'_> {
    fn u8(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.position)?;
        self.position += 1;
        Some(byte)
    }

    fn fixed(&mut self, size: usize) -> Option<u64> {
        let bytes = self.bytes.get(self.position..self.position + size)?;
        self.position += size;
        Some(
            bytes
                .iter()
                .rev()
                .fold(0, |value, &byte| (value << 8) | u64::from(byte)),
        )
    }

    fn uleb(&mut self) -> Option<u64> {
        let mut result = 0_u64;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            if shift < 64 {
                result |= u64::from(byte & 0x7F) << shift;
            }
            shift += 7;
            if byte & 0x80 == 0 {
                return Some(result);
            }
        }
    }

    fn sleb(&mut self) -> Option<i64> {
        let mut result = 0_i64;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            if shift < 64 {
                result |= i64::from(byte & 0x7F) << shift;
            }
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    result |= -1 << shift;
                }
                return Some(result);
            }
        }
    }

    fn cstr(&mut self) -> Option<&[u8]> {
        let start = self.position;
        let length = self
            .bytes
            .get(start..)?
            .iter()
            .position(|&byte| byte == 0)?;
        self.position += length + 1;
        Some(&self.bytes[start..start + length])
    }
}

/// A NUL-terminated string at `offset` in a string section.
fn string_at(section: &[u8], offset: u64) -> &[u8] {
    let start = (offset as usize).min(section.len());
    let rest = &section[start..];
    &rest[..rest
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(rest.len())]
}

/// One line-number program (DWARF 5 §6.2.4-§6.2.5): read its header, run it,
/// and append its rows. `None` when it is malformed.
#[expect(
    clippy::too_many_lines,
    reason = "The header, then one arm per opcode."
)]
fn program(
    reader: &mut Dwarf<'_>,
    line_strings: &[u8],
    strings: &[u8],
    code_base: u32,
    rows: &mut Vec<Row>,
) -> Option<()> {
    let length = reader.fixed(4)? as usize;
    if length == 0xFFFF_FFFF {
        // 64-bit DWARF is not produced for wasm32.
        return None;
    }
    let end = reader.position + length;
    let version = reader.fixed(2)?;
    if !(2..=5).contains(&version) {
        reader.position = end;
        return Some(());
    }
    let address_size = if version >= 5 {
        let size = reader.u8()?;
        let _segment_selector = reader.u8()?;
        usize::from(size)
    } else {
        4
    };
    let header_length = reader.fixed(4)? as usize;
    let program_start = reader.position + header_length;
    let min_instruction = u64::from(reader.u8()?);
    if version >= 4 {
        let _max_ops = reader.u8()?;
    }
    let _default_is_stmt = reader.u8()?;
    let line_base = i64::from(reader.u8()? as i8);
    let line_range = u64::from(reader.u8()?).max(1);
    let opcode_base = reader.u8()?;
    let mut lengths = Vec::new();
    for _ in 1..opcode_base {
        lengths.push(reader.u8()?);
    }
    // The compilation unit's own file: entry 0 in DWARF 5 (which also lists
    // it again as entry 1), entry 1 before.
    let files = file_names(reader, version, line_strings, strings)?;
    let primary = files.first().cloned().unwrap_or_default();
    let primary_index = u64::from(version < 5);
    let is_primary = |file: u64| {
        let index = if version >= 5 {
            file
        } else {
            file.wrapping_sub(1)
        };
        file == primary_index || files.get(index as usize) == Some(&primary)
    };
    reader.position = program_start;
    let (mut address, mut file, mut line, mut column) = (0_u64, 1_u64, 1_i64, 0_u64);
    // A sequence whose start address the linker tombstoned (dead code).
    let mut dead = false;
    // `line` is `None` for the end of a sequence, past which no line holds.
    let mut emit = |address: u64, file: u64, line: Option<i64>, column: u64, dead: bool| {
        if dead {
            return;
        }
        let Ok(address) = u32::try_from(address) else {
            return;
        };
        let line = match line {
            | Some(line) if is_primary(file) => u32::try_from(line).unwrap_or(0),
            | _ => 0,
        };
        rows.push(Row {
            offset: code_base.saturating_add(address),
            line,
            column: u32::try_from(column).unwrap_or(0),
        });
    };
    while reader.position < end {
        let opcode = reader.u8()?;
        if opcode >= opcode_base {
            let adjusted = u64::from(opcode - opcode_base);
            address += (adjusted / line_range) * min_instruction;
            line += line_base + (adjusted % line_range) as i64;
            emit(address, file, Some(line), column, dead);
            continue;
        }
        match opcode {
            | 0 => {
                let size = reader.uleb()? as usize;
                let next = reader.position + size;
                match reader.u8()? {
                    // DW_LNE_end_sequence: the address past the sequence has
                    // no line.
                    | 1 => {
                        emit(address, file, None, 0, dead);
                        (address, file, line, column) = (0, 1, 1, 0);
                        dead = false;
                    },
                    // DW_LNE_set_address.
                    | 2 => {
                        address = reader.fixed(address_size)?;
                        let all_ones = if address_size >= 8 {
                            u64::MAX
                        } else {
                            (1 << (8 * address_size)) - 1
                        };
                        dead = address >= all_ones - 1;
                    },
                    | _ => {},
                }
                reader.position = next;
            },
            | 1 => emit(address, file, Some(line), column, dead),
            | 2 => address += reader.uleb()? * min_instruction,
            | 3 => line += reader.sleb()?,
            | 4 => file = reader.uleb()?,
            | 5 => column = reader.uleb()?,
            // negate_stmt, set_basic_block, set_prologue_end,
            // set_epilogue_begin: flags the stepper does not use.
            | 6 | 7 | 10 | 11 => {},
            | 8 => address += ((255 - u64::from(opcode_base)) / line_range) * min_instruction,
            | 9 => address += reader.fixed(2)?,
            | _ =>
                for _ in 0..lengths[usize::from(opcode) - 1] {
                    reader.uleb()?;
                },
        }
    }
    reader.position = end;
    Some(())
}

/// The header's file names, in table order.
fn file_names(
    reader: &mut Dwarf<'_>,
    version: u64,
    line_strings: &[u8],
    strings: &[u8],
) -> Option<Vec<Vec<u8>>> {
    if version < 5 {
        // include_directories, then file_names, each ended by an empty
        // entry.
        while !reader.cstr()?.is_empty() {}
        let mut files = Vec::new();
        loop {
            let name = reader.cstr()?.to_vec();
            if name.is_empty() {
                return Some(files);
            }
            reader.uleb()?;
            reader.uleb()?;
            reader.uleb()?;
            files.push(name);
        }
    }
    // DWARF 5: directories, then files, each described by a format of
    // (content type, form) pairs.
    entries(reader, line_strings, strings)?;
    entries(reader, line_strings, strings)
}

/// One DWARF 5 entry table; returns each entry's path (`DW_LNCT_path`).
fn entries(reader: &mut Dwarf<'_>, line_strings: &[u8], strings: &[u8]) -> Option<Vec<Vec<u8>>> {
    let format_count = reader.u8()?;
    let mut format = Vec::new();
    for _ in 0..format_count {
        format.push((reader.uleb()?, reader.uleb()?));
    }
    let count = reader.uleb()?;
    let mut names = Vec::new();
    for _ in 0..count {
        let mut name = Vec::new();
        for &(content, form) in &format {
            let value = form_value(reader, form, line_strings, strings)?;
            // DW_LNCT_path.
            if content == 1 {
                name = value;
            }
        }
        names.push(name);
    }
    Some(names)
}

/// Read one attribute value of `form`; strings are returned, other values
/// skipped.
fn form_value(
    reader: &mut Dwarf<'_>,
    form: u64,
    line_strings: &[u8],
    strings: &[u8],
) -> Option<Vec<u8>> {
    Some(match form {
        // DW_FORM_string.
        | 0x08 => reader.cstr()?.to_vec(),
        // DW_FORM_line_strp, DW_FORM_strp.
        | 0x1F => string_at(line_strings, reader.fixed(4)?).to_vec(),
        | 0x0E => string_at(strings, reader.fixed(4)?).to_vec(),
        // data1, data2, data4, data8, data16.
        | 0x0B => skip(reader, 1)?,
        | 0x05 => skip(reader, 2)?,
        | 0x06 => skip(reader, 4)?,
        | 0x07 => skip(reader, 8)?,
        | 0x1E => skip(reader, 16)?,
        // DW_FORM_udata.
        | 0x0F => {
            reader.uleb()?;
            Vec::new()
        },
        // DW_FORM_block.
        | 0x09 => {
            let length = reader.uleb()? as usize;
            skip(reader, length)?
        },
        | _ => return None,
    })
}

fn skip(reader: &mut Dwarf<'_>, size: usize) -> Option<Vec<u8>> {
    if reader.position + size > reader.bytes.len() {
        return None;
    }
    reader.position += size;
    Some(Vec::new())
}
