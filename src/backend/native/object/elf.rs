//! ELF64 little-endian relocatable files for `EM_X86_64`.
//! ELF gABI (sections, symbols, groups) and AMD64 psABI §4.4 (RELA).

use super::{
    Error,
    Object,
    Result,
};
use crate::util::bump::ArenaVec;

pub(super) fn write<'a>(object: &Object<'a>) -> Result<ArenaVec<'a, u8>> {
    let arena = object.arena;
    let count = object.sections.len();
    let mut headers = ArenaVec::new_in(arena);
    headers.push(Header::default());
    let mut names = ArenaVec::new_in(arena);
    names.push(0);
    // LLVM shares symbol and section-name strings. Match its section order:
    // string table first, each group before its members, each RELA after its
    // target, and the symbol table last. Object section IDs stay independent.
    let strtab = headers.len();
    headers.push(Header {
        name: string(&mut names, ".strtab")?,
        kind: 3,
        align: 1,
        ..Header::default()
    });
    let shstrtab = strtab;
    let mut section_indices = ArenaVec::new_in(arena);
    let mut relocation_sections = ArenaVec::new_in(arena);
    relocation_sections.resize(count, None);
    let mut groups = ArenaVec::new_in(arena);
    for (i, section) in object.sections.iter().enumerate() {
        if let Some(signature) = section.group
            && !groups.iter().any(|&(s, _)| s == signature)
        {
            groups.push((signature, headers.len()));
            headers.push(Header {
                name: string(&mut names, ".group")?,
                kind: 17,
                align: 4,
                entry: 4,
                ..Header::default()
            });
        }
        let index = headers.len();
        section_indices.push(index);
        headers.push(Header {
            name: string(&mut names, section.name)?,
            kind: section.kind,
            flags: section.flags,
            align: section.align,
            size: section.size,
            ..Header::default()
        });
        if !section.relocs.is_empty() {
            let name = u32::try_from(names.len()).map_err(|_| Error("string table too large"))?;
            names.extend_from_slice(b".rela");
            names.extend_from_slice(section.name.as_bytes());
            names.push(0);
            relocation_sections[i] = Some(headers.len());
            headers.push(Header {
                name,
                kind: 4,
                flags: if section.group.is_some() { 0x200 } else { 0x40 },
                align: 8,
                entry: 24,
                info: u32::try_from(index).map_err(|_| Error("too many sections"))?,
                ..Header::default()
            });
        }
    }
    let symtab = headers.len();
    headers.push(Header {
        name: string(&mut names, ".symtab")?,
        kind: 2,
        align: 8,
        entry: 24,
        ..Header::default()
    });
    if headers.len() >= 0xFF00 {
        return Err(Error("too many ELF sections"));
    }
    let mut symbols = ArenaVec::new_in(arena);
    symbols.resize(24, 0);
    let mut indices = ArenaVec::new_in(arena);
    indices.resize(object.symbols.len(), 0_u32);
    let mut section_symbols = ArenaVec::new_in(arena);
    section_symbols.resize(count, 0_u32);
    // Local references use STT_SECTION plus their offset, as clang does.
    for section in &object.sections {
        for reloc in &section.relocs {
            let symbol = object.symbols[reloc.symbol];
            if symbol.binding == 0
                && symbol.kind != 6
                && !reloc.named
                && let Some(target) = symbol.section
                && section_symbols[target] == 0
            {
                section_symbols[target] = u32::try_from(symbols.len() / 24)
                    .map_err(|_| Error("symbol table too large"))?;
                symbol_record(
                    &mut symbols,
                    0,
                    3,
                    0,
                    u16::try_from(section_indices[target])
                        .map_err(|_| Error("too many sections"))?,
                    0,
                    0,
                );
            }
        }
    }
    for global in [false, true] {
        for (i, symbol) in object.symbols.iter().enumerate() {
            let binding = if symbol.section.is_none() && symbol.binding == 0 {
                1
            } else {
                symbol.binding
            };
            if (binding != 0) != global || symbol.name.is_empty() || symbol.name.starts_with(".L") {
                continue;
            }
            indices[i] =
                u32::try_from(symbols.len() / 24).map_err(|_| Error("symbol table too large"))?;
            symbol_record(
                &mut symbols,
                string(&mut names, symbol.name)?,
                (binding << 4) | symbol.kind,
                symbol.visibility,
                if symbol.common {
                    SHN_COMMON
                } else {
                    u16::try_from(symbol.section.map_or(0, |s| section_indices[s]))
                        .map_err(|_| Error("too many sections"))?
                },
                symbol.value,
                symbol.size,
            );
        }
        if !global {
            headers[symtab].info =
                u32::try_from(symbols.len() / 24).map_err(|_| Error("symbol table too large"))?;
        }
    }
    headers[symtab].link = u32::try_from(strtab).map_err(|_| Error("too many sections"))?;
    let mut out = ArenaVec::new_in(arena);
    out.resize(64, 0);
    for (i, section) in object.sections.iter().enumerate() {
        pad(&mut out, section.align)?;
        headers[section_indices[i]].offset = out.len() as u64;
        if section.kind != 8 {
            out.extend_from_slice(&section.data);
        }
    }
    for (i, section) in object.sections.iter().enumerate() {
        if let Some(id) = relocation_sections[i] {
            pad(&mut out, 8)?;
            headers[id].offset = out.len() as u64;
            headers[id].link = u32::try_from(symtab).map_err(|_| Error("too many sections"))?;
            for reloc in &section.relocs {
                let symbol = object.symbols[reloc.symbol];
                let (index, addend) = if symbol.binding == 0
                    && symbol.kind != 6
                    && !reloc.named
                    && symbol.section.is_some()
                {
                    (
                        section_symbols[symbol.section.ok_or(Error("missing section"))?],
                        reloc.addend.wrapping_add(symbol.value as i64),
                    )
                } else {
                    (indices[reloc.symbol], reloc.addend)
                };
                if index == 0 {
                    return Err(Error("relocation has no symbol table entry"));
                }
                out.extend_from_slice(&reloc.offset.to_le_bytes());
                out.extend_from_slice(
                    &((u64::from(index) << 32) | u64::from(reloc.kind)).to_le_bytes(),
                );
                out.extend_from_slice(&addend.to_le_bytes());
            }
            headers[id].size = out.len() as u64 - headers[id].offset;
        }
    }
    for (signature, id) in groups {
        pad(&mut out, 4)?;
        headers[id].offset = out.len() as u64;
        headers[id].link = u32::try_from(symtab).map_err(|_| Error("too many sections"))?;
        headers[id].info = indices[signature];
        if indices[signature] == 0 {
            return Err(Error("missing COMDAT signature symbol"));
        }
        out.extend_from_slice(&1_u32.to_le_bytes());
        for (i, section) in object
            .sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.group == Some(signature))
        {
            out.extend_from_slice(
                &u32::try_from(section_indices[i])
                    .map_err(|_| Error("too many sections"))?
                    .to_le_bytes(),
            );
            if let Some(id) = relocation_sections[i] {
                out.extend_from_slice(
                    &u32::try_from(id)
                        .map_err(|_| Error("too many sections"))?
                        .to_le_bytes(),
                );
            }
            _ = section;
        }
        headers[id].size = out.len() as u64 - headers[id].offset;
    }
    pad(&mut out, 8)?;
    headers[symtab].offset = out.len() as u64;
    headers[symtab].size = symbols.len() as u64;
    out.extend_from_slice(&symbols);
    headers[shstrtab].offset = out.len() as u64;
    headers[shstrtab].size = names.len() as u64;
    out.extend_from_slice(&names);
    pad(&mut out, 8)?;
    let shoff = out.len() as u64;
    for header in &headers {
        out.extend_from_slice(&header.name.to_le_bytes());
        out.extend_from_slice(&header.kind.to_le_bytes());
        out.extend_from_slice(&header.flags.to_le_bytes());
        out.extend_from_slice(&0_u64.to_le_bytes());
        out.extend_from_slice(&header.offset.to_le_bytes());
        out.extend_from_slice(&header.size.to_le_bytes());
        out.extend_from_slice(&header.link.to_le_bytes());
        out.extend_from_slice(&header.info.to_le_bytes());
        out.extend_from_slice(&header.align.to_le_bytes());
        out.extend_from_slice(&header.entry.to_le_bytes());
    }
    out[..16].copy_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out[16..18].copy_from_slice(&1_u16.to_le_bytes());
    out[18..20].copy_from_slice(&62_u16.to_le_bytes());
    out[20..24].copy_from_slice(&1_u32.to_le_bytes());
    out[40..48].copy_from_slice(&shoff.to_le_bytes());
    out[52..54].copy_from_slice(&64_u16.to_le_bytes());
    out[58..60].copy_from_slice(&64_u16.to_le_bytes());
    out[60..62].copy_from_slice(
        &u16::try_from(headers.len())
            .map_err(|_| Error("too many sections"))?
            .to_le_bytes(),
    );
    out[62..64].copy_from_slice(
        &u16::try_from(shstrtab)
            .map_err(|_| Error("too many sections"))?
            .to_le_bytes(),
    );
    Ok(out)
}

#[derive(Default)]
struct Header {
    name:   u32,
    kind:   u32,
    flags:  u64,
    offset: u64,
    size:   u64,
    link:   u32,
    info:   u32,
    align:  u64,
    entry:  u64,
}

fn string(table: &mut ArenaVec<'_, u8>, value: &str) -> Result<u32> {
    let at = u32::try_from(table.len()).map_err(|_| Error("string table too large"))?;
    table.extend_from_slice(value.as_bytes());
    table.push(0);
    Ok(at)
}

fn symbol_record(
    table: &mut ArenaVec<'_, u8>,
    name: u32,
    info: u8,
    visibility: u8,
    section: u16,
    value: u64,
    size: u64,
) {
    table.extend_from_slice(&name.to_le_bytes());
    table.push(info);
    table.push(visibility);
    table.extend_from_slice(&section.to_le_bytes());
    table.extend_from_slice(&value.to_le_bytes());
    table.extend_from_slice(&size.to_le_bytes());
}

fn pad(out: &mut ArenaVec<'_, u8>, align: u64) -> Result<()> {
    let count = (out.len() as u64).wrapping_neg() & (align - 1);
    let size = out
        .len()
        .checked_add(usize::try_from(count).map_err(|_| Error("object too large"))?)
        .ok_or(Error("object too large"))?;
    out.resize(size, 0);
    Ok(())
}

/// The section index of a common symbol (ELF gABI, "Sections", special
/// section indexes).
const SHN_COMMON: u16 = 0xFFF2;
