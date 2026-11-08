//! The DWARF 5 line number program (§6.2) of the listing's `.file` and `.loc`
//! directives, in `.debug_line` as LLVM MC writes it for the same listing
//! (`MCDwarfLineTable::emit`), so that a direct object and an assembled one
//! agree byte for byte.
//!
//! A `.loc` gives the row of the instruction or data that follows it. The
//! rows of each section form one sequence, in the order the section's first
//! row was seen, which ends at the end of the section (§6.2.5.3,
//! `DW_LNE_end_sequence`). The header names the directories and files through
//! `.debug_line_str` (`DW_FORM_line_strp`, §7.5.5), which holds each string
//! once, in the order the header first names it. A sequence's first address
//! is relocated against its section (`R_X86_64_64`) and each name against
//! `.debug_line_str` (`R_X86_64_32`), as AMD64 psABI §4.4 defines them.

use super::{
    Error,
    Kind,
    LineFile,
    Loc,
    Object,
    Reloc,
    Result,
    Symbol,
};
use crate::util::bump::{
    ArenaVec,
    Bump,
};

/// The line program parameters LLVM MC uses for x86: opcodes below 13 are
/// standard ones, and a special opcode advances the line by -5 to 8.
/// DWARF 5 §6.2.4, items 8-11.
const OPCODE_BASE: u8 = 13;
const LINE_BASE: i64 = -5;
const LINE_RANGE: u64 = 14;
/// The operand count of each standard opcode from 1 to 12 (§6.2.5.2).
const STANDARD_OPCODE_LENGTHS: [u8; 12] = [0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1];

// Standard and extended opcodes, DWARF 5 §7.22, Tables 7.25 and 7.26.
const DW_LNS_COPY: u8 = 0x01;
const DW_LNS_ADVANCE_PC: u8 = 0x02;
const DW_LNS_ADVANCE_LINE: u8 = 0x03;
const DW_LNS_SET_FILE: u8 = 0x04;
const DW_LNS_SET_COLUMN: u8 = 0x05;
const DW_LNS_NEGATE_STMT: u8 = 0x06;
const DW_LNS_CONST_ADD_PC: u8 = 0x08;
const DW_LNS_SET_PROLOGUE_END: u8 = 0x0A;
const DW_LNS_SET_EPILOGUE_BEGIN: u8 = 0x0B;
const DW_LNE_END_SEQUENCE: u8 = 0x01;
const DW_LNE_SET_ADDRESS: u8 = 0x02;
// Entry formats: content types (§7.22, Table 7.27) and forms (§7.5.6).
const DW_LNCT_PATH: u8 = 0x01;
const DW_LNCT_DIRECTORY_INDEX: u8 = 0x02;
const DW_FORM_UDATA: u8 = 0x0F;
const DW_FORM_LINE_STRP: u8 = 0x1F;

const R_X86_64_64: u32 = 1;
const R_X86_64_32: u32 = 10;

/// Appends the line table of the listing's `.file` and `.loc` directives to
/// `.debug_line`, after what the listing put there itself (the label its
/// compilation unit's `DW_AT_stmt_list` names), and writes
/// `.debug_line_str`. A listing without `.file` has no line table.
pub(super) fn generate(object: &mut Object<'_>) -> Result<()> {
    if object.files.is_empty() {
        return Ok(());
    }
    let arena = object.arena;
    let mut rows = ArenaVec::new_in(arena);
    let mut pending = None;
    for item in &object.items {
        match item.kind {
            | Kind::Loc(loc) => pending = Some(loc),
            // MC's `MCDwarfLineEntry::make`, from emitting an instruction,
            // bytes or a value.
            | Kind::Code(_) | Kind::Branch { .. } | Kind::Bytes(_) | Kind::Value(..) =>
                if let Some(loc) = pending.take() {
                    rows.push((item.section, item.offset, loc));
                },
            | _ => {},
        }
    }
    if object
        .sections
        .iter()
        .any(|section| section.name == ".debug_line_str")
    {
        return Err(Error(
            "user section conflicts with generated line table strings",
        ));
    }
    let line = object.section(".debug_line", 1, 0, None)?;
    let strings_section = object.section(".debug_line_str", 1, 0x30, None)?;
    let strings_symbol = section_symbol(object, strings_section);
    let mut table = Table {
        arena,
        data: ArenaVec::new_in(arena),
        relocs: ArenaVec::new_in(arena),
        base: object.sections[line].size,
        strings: Strings {
            bytes:   ArenaVec::new_in(arena),
            offsets: ArenaVec::new_in(arena),
            symbol:  strings_symbol,
        },
    };
    table.header(&object.files)?;
    let mut sections: ArenaVec<'_, usize> = ArenaVec::new_in(arena);
    for &(section, _, _) in &rows {
        if !sections.contains(&section) {
            sections.push(section);
        }
    }
    for section in sections {
        let symbol = section_symbol(object, section);
        let end = object.sections[section].size;
        table.sequence(
            rows.iter()
                .filter(|&&(at, _, _)| at == section)
                .map(|&(_, offset, loc)| (offset, loc)),
            symbol,
            end,
        )?;
    }
    let length = u32::try_from(table.data.len() - 4).map_err(|_| Error("line table too large"))?;
    table.data[..4].copy_from_slice(&length.to_le_bytes());
    let Table {
        data,
        relocs,
        strings,
        ..
    } = table;
    let section = &mut object.sections[line];
    section.data.extend_from_slice(&data);
    section.size = section.data.len() as u64;
    section.relocs.extend(relocs);
    let section = &mut object.sections[strings_section];
    section.size = strings.bytes.len() as u64;
    section.data = strings.bytes;
    Ok(())
}

/// One line table as it is written.
struct Table<'a> {
    arena:   &'a Bump,
    data:    ArenaVec<'a, u8>,
    relocs:  ArenaVec<'a, Reloc>,
    /// Where the table starts in `.debug_line`.
    base:    u64,
    strings: Strings<'a>,
}

impl<'a> Table<'a> {
    /// The header up to the program, with a placeholder for the unit
    /// length. The directories are the root file's, then those the other
    /// files name, as MC's `tryGetFile` files them: a file in the root
    /// file's directory, or in none, is in directory 0, and a name with a
    /// directory part but no directory of its own is refused, as the native
    /// listing never writes one. DWARF 5 §6.2.4.
    fn header(&mut self, files: &[Option<LineFile<'a>>]) -> Result<()> {
        let root = files
            .first()
            .copied()
            .flatten()
            .ok_or(Error("line table without a root file"))?;
        let mut directories: ArenaVec<'_, &'a [u8]> = ArenaVec::new_in(self.arena);
        let mut entries = ArenaVec::new_in(self.arena);
        for file in &files[1..] {
            let file = file.ok_or(Error("unassigned file number"))?;
            // MC's `isRootFile`: without checksums it would take this file
            // for the root file.
            if file.name == root.name {
                return Err(Error("file repeats the root file's name"));
            }
            let directory = if file.directory == root.directory {
                &[][..]
            } else {
                file.directory
            };
            if file.name.is_empty() || (directory.is_empty() && file.name.contains(&b'/')) {
                return Err(Error("unsupported line table file name"));
            }
            let index = if directory.is_empty() {
                0
            } else if let Some(at) = directories.iter().position(|&d| d == directory) {
                at + 1
            } else {
                directories.push(directory);
                directories.len()
            };
            entries.push((file.name, index));
        }
        let data = &mut self.data;
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&5_u16.to_le_bytes());
        // Address size, segment selector size.
        data.extend_from_slice(&[8, 0]);
        let header_length = data.len();
        data.extend_from_slice(&[0; 4]);
        // Minimum instruction length, maximum operations per instruction,
        // default `is_stmt`, line base, line range, opcode base.
        data.extend_from_slice(&[
            1,
            1,
            1,
            LINE_BASE.to_le_bytes()[0],
            LINE_RANGE.to_le_bytes()[0],
            OPCODE_BASE,
        ]);
        data.extend_from_slice(&STANDARD_OPCODE_LENGTHS);
        data.extend_from_slice(&[1, DW_LNCT_PATH, DW_FORM_LINE_STRP]);
        uleb128(data, directories.len() as u64 + 1);
        self.string(root.directory)?;
        for directory in &directories {
            self.string(directory)?;
        }
        self.data.extend_from_slice(&[
            2,
            DW_LNCT_PATH,
            DW_FORM_LINE_STRP,
            DW_LNCT_DIRECTORY_INDEX,
            DW_FORM_UDATA,
        ]);
        uleb128(&mut self.data, files.len() as u64);
        self.string(root.name)?;
        uleb128(&mut self.data, 0);
        for &(name, index) in &entries {
            self.string(name)?;
            uleb128(&mut self.data, index as u64);
        }
        let length = u32::try_from(self.data.len() - header_length - 4)
            .map_err(|_| Error("line table header too large"))?;
        self.data[header_length..header_length + 4].copy_from_slice(&length.to_le_bytes());
        Ok(())
    }

    /// A `DW_FORM_line_strp` reference to `string`.
    fn string(&mut self, string: &'a [u8]) -> Result<()> {
        let offset = self.strings.offset(string)?;
        self.relocs.push(Reloc {
            offset: self.base + self.data.len() as u64,
            symbol: self.strings.symbol,
            kind:   R_X86_64_32,
            addend: i64::from(offset),
            named:  false,
        });
        self.data.extend_from_slice(&[0; 4]);
        Ok(())
    }

    /// The sequence of one section's rows, as MC's `emitDwarfLineTable`
    /// writes it: the registers' changes in its order, then the advance of
    /// line and address, from the section start that `symbol` names to
    /// `end`. DWARF 5 §6.2.5.
    fn sequence(
        &mut self,
        rows: impl Iterator<Item = (u64, Loc)>,
        symbol: usize,
        end: u64,
    ) -> Result<()> {
        let (mut file, mut line, mut column, mut flags) = (1, 1_u32, 0, Loc::IS_STMT);
        let mut last = None;
        for (offset, loc) in rows {
            if loc.file != file {
                file = loc.file;
                self.data.push(DW_LNS_SET_FILE);
                uleb128(&mut self.data, u64::from(file));
            }
            if loc.column != column {
                column = loc.column;
                self.data.push(DW_LNS_SET_COLUMN);
                uleb128(&mut self.data, u64::from(column));
            }
            if (loc.flags ^ flags) & Loc::IS_STMT != 0 {
                flags = loc.flags;
                self.data.push(DW_LNS_NEGATE_STMT);
            }
            if loc.flags & Loc::PROLOGUE_END != 0 {
                self.data.push(DW_LNS_SET_PROLOGUE_END);
            }
            if loc.flags & Loc::EPILOGUE_BEGIN != 0 {
                self.data.push(DW_LNS_SET_EPILOGUE_BEGIN);
            }
            let line_delta = i64::from(loc.line) - i64::from(line);
            if let Some(previous) = last {
                advance(&mut self.data, Some(line_delta), offset - previous);
            } else {
                // DW_LNE_set_address, with the operand's size first.
                self.data.extend_from_slice(&[0, 9, DW_LNE_SET_ADDRESS]);
                self.relocs.push(Reloc {
                    offset: self.base + self.data.len() as u64,
                    symbol,
                    kind: R_X86_64_64,
                    addend: i64::try_from(offset).map_err(|_| Error("code offset too large"))?,
                    named: false,
                });
                self.data.extend_from_slice(&[0; 8]);
                advance(&mut self.data, Some(line_delta), 0);
            }
            line = loc.line;
            last = Some(offset);
        }
        if let Some(previous) = last {
            advance(&mut self.data, None, end - previous);
        }
        Ok(())
    }
}

/// The opcodes that advance the line by `line` and the address by
/// `address` and append a row, or with no `line` end the sequence there:
/// MC's `MCDwarfLineAddr::encode`. A special opcode does both when it can
/// (§6.2.5.1), possibly after `DW_LNS_const_add_pc`, the advance of special
/// opcode 255; otherwise `DW_LNS_advance_line` and `DW_LNS_advance_pc` do.
fn advance(data: &mut ArenaVec<'_, u8>, line: Option<i64>, address: u64) {
    let max_special_address = (255 - u64::from(OPCODE_BASE)) / LINE_RANGE;
    let Some(mut line) = line else {
        if address == max_special_address {
            data.push(DW_LNS_CONST_ADD_PC);
        } else if address != 0 {
            data.push(DW_LNS_ADVANCE_PC);
            uleb128(data, address);
        }
        data.extend_from_slice(&[0, 1, DW_LNE_END_SEQUENCE]);
        return;
    };
    let mut copy = false;
    // The line advance biased by the line base: the special opcode's
    // remainder by the line range.
    let mut biased = line.wrapping_sub(LINE_BASE).cast_unsigned();
    if biased >= LINE_RANGE || biased + u64::from(OPCODE_BASE) > 255 {
        data.push(DW_LNS_ADVANCE_LINE);
        sleb128(data, line);
        line = 0;
        biased = LINE_BASE.unsigned_abs();
        copy = true;
    }
    if line == 0 && address == 0 {
        data.push(DW_LNS_COPY);
        return;
    }
    let opcode = biased + u64::from(OPCODE_BASE);
    if address < 256 + max_special_address {
        if let Ok(special) = u8::try_from(opcode + address * LINE_RANGE) {
            data.push(special);
            return;
        }
        if let Some(special) = address
            .checked_sub(max_special_address)
            .and_then(|rest| u8::try_from(opcode + rest * LINE_RANGE).ok())
        {
            data.extend_from_slice(&[DW_LNS_CONST_ADD_PC, special]);
            return;
        }
    }
    data.push(DW_LNS_ADVANCE_PC);
    uleb128(data, address);
    data.push(if copy {
        DW_LNS_COPY
    } else {
        u8::try_from(opcode).expect("a special opcode within range")
    });
}

/// The strings of `.debug_line_str`, each once.
struct Strings<'a> {
    bytes:   ArenaVec<'a, u8>,
    offsets: ArenaVec<'a, (&'a [u8], u32)>,
    /// The section symbol references are relocated against.
    symbol:  usize,
}

impl<'a> Strings<'a> {
    fn offset(&mut self, string: &'a [u8]) -> Result<u32> {
        if let Some(&(_, offset)) = self.offsets.iter().find(|(s, _)| *s == string) {
            return Ok(offset);
        }
        let offset =
            u32::try_from(self.bytes.len()).map_err(|_| Error("line table strings too large"))?;
        self.bytes.extend_from_slice(string);
        self.bytes.push(0);
        self.offsets.push((string, offset));
        Ok(offset)
    }
}

/// A local symbol at the start of `section`, which the ELF writer turns
/// into the section's `STT_SECTION` symbol.
fn section_symbol(object: &mut Object<'_>, section: usize) -> usize {
    object.symbols.push(Symbol {
        name:       "",
        section:    Some(section),
        value:      0,
        size:       0,
        binding:    0,
        kind:       3,
        visibility: 0,
        alias:      None,
        common:     false,
    });
    object.symbols.len() - 1
}

/// Appends `value` as an unsigned LEB128 number (DWARF 5 §7.6).
pub(super) fn uleb128(data: &mut ArenaVec<'_, u8>, mut value: u64) {
    loop {
        let byte = value.to_le_bytes()[0] & 0x7F;
        value >>= 7;
        data.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            break;
        }
    }
}

/// Appends `value` as a signed LEB128 number (DWARF 5 §7.6).
pub(super) fn sleb128(data: &mut ArenaVec<'_, u8>, mut value: i64) {
    loop {
        let byte = value.to_le_bytes()[0] & 0x7F;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        data.push(byte | if done { 0 } else { 0x80 });
        if done {
            break;
        }
    }
}
