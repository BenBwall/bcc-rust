//! COFF headers, relocation records and section/weak auxiliary symbol records.

use std::fmt::Write as _;

use super::{
    Coff,
    Error,
    Result,
};
use crate::util::bump::{
    ArenaString,
    ArenaVec,
};

pub(super) fn write<'a>(coff: &Coff<'a>) -> Result<ArenaVec<'a, u8>> {
    let object = &coff.object;
    let arena = object.arena;
    let count = object.sections.len();
    if count > 0x7FFF {
        return Err(Error("too many COFF sections"));
    }
    let mut strings = ArenaVec::new_in(arena);
    strings.resize(4, 0);
    let mut symbols = ArenaVec::new_in(arena);
    let mut indices = ArenaVec::new_in(arena);
    indices.resize(object.symbols.len(), None);
    let mut next = u32::try_from(count * 2).map_err(|_| Error("too many COFF symbols"))?;
    for (id, symbol) in object.symbols.iter().enumerate() {
        if symbol.name.is_empty() || symbol.name.starts_with(".L") {
            continue;
        }
        indices[id] = Some(next);
        let alias = symbol
            .alias
            .filter(|&target| object.symbols[target].binding != 0);
        let records = match (symbol.binding, alias) {
            | (2, Some(_)) => 2,
            | (2, None) => 3,
            | _ => 1,
        };
        next = next
            .checked_add(records)
            .ok_or(Error("too many COFF symbols"))?;
    }
    let mut out = ArenaVec::new_in(arena);
    out.resize(20 + count * 40, 0);
    let mut checksums = ArenaVec::new_in(arena);
    for (id, section) in object.sections.iter().enumerate() {
        if section.relocs.len() > 0xFFFF {
            return Err(Error("COFF relocation overflow is not supported"));
        }
        if !section.align.is_power_of_two() || section.align > 8192 {
            return Err(Error("invalid COFF section alignment"));
        }
        let at = 20 + id * 40;
        let mut name = [0; 8];
        if section.name.len() <= 8 {
            name[..section.name.len()].copy_from_slice(section.name.as_bytes());
        } else {
            let offset = string(&mut strings, section.name)?;
            let mut text = ArenaString::new_in(arena);
            write!(&mut text, "/{offset}").map_err(|_| Error("COFF name formatting failed"))?;
            if text.len() > 8 {
                return Err(Error("COFF section string offset exceeds header"));
            }
            name[..text.len()].copy_from_slice(text.as_bytes());
        }
        out[at..at + 8].copy_from_slice(&name);
        put32(&mut out, at + 16, narrow(section.size)?);
        put32(
            &mut out,
            at + 36,
            coff.sections[id].characteristics | ((section.align.trailing_zeros() + 1) << 20),
        );
        let mut data = ArenaVec::new_in(arena);
        data.extend_from_slice(&section.data);
        for reloc in &section.relocs {
            let symbol = object.symbols[reloc.symbol];
            let (_, width, bias) = relocation(reloc.kind)?;
            let mut value = i128::from(reloc.addend) + i128::from(bias);
            if symbol.name.is_empty() || symbol.name.starts_with(".L") {
                if symbol.section.is_none() {
                    return Err(Error("undefined COFF temporary symbol"));
                }
                if reloc.kind != 0x0001_000A {
                    value += i128::from(symbol.value);
                }
            }
            // SECTION carries a section index, never a byte offset.
            if reloc.kind == 0x0001_000A && value != 0 {
                return Err(Error("unsupported SECTION relocation addend"));
            }
            let field_bits = width * 8;
            if value < -(1_i128 << (field_bits - 1)) || value >= (1_i128 << field_bits) {
                return Err(Error("COFF relocation addend exceeds field"));
            }
            let start = usize::try_from(reloc.offset)
                .map_err(|_| Error("COFF relocation offset too large"))?;
            if start.checked_add(width).is_none_or(|end| end > data.len()) {
                return Err(Error("COFF relocation outside section"));
            }
            data[start..start + width].copy_from_slice(&value.to_le_bytes()[..width]);
        }
        checksums.push(checksum(&data));
        if section.kind != 8 && !data.is_empty() {
            let raw = narrow(out.len() as u64)?;
            put32(&mut out, at + 20, raw);
            out.extend_from_slice(&data);
        }
        if !section.relocs.is_empty() {
            let raw = narrow(out.len() as u64)?;
            put32(&mut out, at + 24, raw);
            out[at + 32..at + 34]
                .copy_from_slice(&reloc_count(section.relocs.len())?.to_le_bytes());
            for reloc in &section.relocs {
                let symbol = object.symbols[reloc.symbol];
                let index = if symbol.name.is_empty() || symbol.name.starts_with(".L") {
                    u32::try_from(
                        symbol
                            .section
                            .ok_or(Error("missing COFF relocation section"))?
                            * 2,
                    )
                    .map_err(|_| Error("too many COFF sections"))?
                } else {
                    indices[reloc.symbol].ok_or(Error("missing COFF relocation symbol"))?
                };
                out.extend_from_slice(&narrow(reloc.offset)?.to_le_bytes());
                out.extend_from_slice(&index.to_le_bytes());
                out.extend_from_slice(&relocation(reloc.kind)?.0.to_le_bytes());
            }
        }
    }
    for (id, section) in object.sections.iter().enumerate() {
        record(
            &mut symbols,
            &mut strings,
            section.name,
            0,
            section_number(id)?,
            0,
            [3, 1],
        )?;
        symbols.extend_from_slice(&narrow(section.size)?.to_le_bytes());
        symbols.extend_from_slice(&reloc_count(section.relocs.len())?.to_le_bytes());
        symbols.extend_from_slice(&0_u16.to_le_bytes());
        symbols.extend_from_slice(&checksums[id].to_le_bytes());
        symbols.extend_from_slice(
            &section_number(coff.sections[id].associative.unwrap_or(id))?.to_le_bytes(),
        );
        symbols.push(coff.sections[id].selection);
        symbols.extend_from_slice(&[0; 3]);
        if let Some(signature) = section.group
            && object.symbols[signature].section != Some(id)
        {
            return Err(Error("COFF COMDAT signature is outside its section"));
        }
    }
    let default = object
        .symbols
        .iter()
        .find(|s| s.binding == 1 && s.section.is_some())
        .map_or("", |s| s.name);
    for (id, symbol) in object.symbols.iter().enumerate() {
        let Some(index) = indices[id] else {
            continue;
        };
        let section = symbol.section.map(section_number).transpose()?.unwrap_or(0);
        let kind = if symbol.kind == 2 { 32 } else { 0 };
        if symbol.binding == 2 {
            record(&mut symbols, &mut strings, symbol.name, 0, 0, 0, [105, 1])?;
            // Aliases to local symbols need their own external fallback;
            // aliases to external symbols can name that existing record.
            let alias = symbol
                .alias
                .filter(|&target| object.symbols[target].binding != 0);
            let target = if let Some(alias) = alias {
                indices[alias].ok_or(Error("missing COFF weak alias target"))?
            } else {
                index + 2
            };
            symbols.extend_from_slice(&target.to_le_bytes());
            symbols.extend_from_slice(&3_u32.to_le_bytes()); // SEARCH_ALIAS
            symbols.extend_from_slice(&[0; 10]);
            if alias.is_some() {
                continue;
            }
            let mut name = ArenaString::new_in(arena);
            write!(
                &mut name,
                ".weak.{}.default{}{}",
                symbol.name,
                if default.is_empty() { "" } else { "." },
                default
            )
            .map_err(|_| Error("COFF weak name formatting failed"))?;
            record(
                &mut symbols,
                &mut strings,
                &name,
                narrow(symbol.value)?,
                if section == 0 { -1 } else { section },
                kind,
                [2, 0],
            )?;
        } else {
            record(
                &mut symbols,
                &mut strings,
                symbol.name,
                narrow(symbol.value)?,
                section,
                kind,
                [
                    if symbol.binding == 1 || section == 0 {
                        2
                    } else {
                        3
                    },
                    0,
                ],
            )?;
        }
    }
    let symbol_offset = narrow(out.len() as u64)?;
    out.extend_from_slice(&symbols);
    let size = narrow(strings.len() as u64)?;
    strings[..4].copy_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&strings);
    out[..2].copy_from_slice(&0x8664_u16.to_le_bytes());
    out[2..4].copy_from_slice(
        &u16::try_from(count)
            .map_err(|_| Error("too many COFF sections"))?
            .to_le_bytes(),
    );
    put32(&mut out, 8, symbol_offset);
    put32(&mut out, 12, next);
    Ok(out)
}

// The shared encoder uses ELF fixup IDs; COFF-only fixups occupy a separate
// namespace. LLVM's AMD64 COFF assembler uses REL32 with an in-place -N for
// displacement fields followed by N immediate bytes (rather than REL32_N).
fn relocation(kind: u32) -> Result<(u16, usize, i64)> {
    match kind {
        | 1 => Ok((1, 8, 0)),
        | 10 | 11 => Ok((2, 4, 0)),
        | 2 | 4 => Ok((4, 4, 4)),
        | 0x0001_0003 => Ok((3, 4, 0)),
        | 0x0001_0004..=0x0001_0009 => Ok((
            u16::try_from(kind - 0x0001_0000)
                .map_err(|_| Error("COFF relocation type overflow"))?,
            4,
            0,
        )),
        | 0x0001_000A => Ok((10, 2, 0)),
        | 0x0001_000B => Ok((11, 4, 0)),
        | _ => Err(Error("unsupported AMD64 COFF relocation")),
    }
}

fn record(
    table: &mut ArenaVec<'_, u8>,
    strings: &mut ArenaVec<'_, u8>,
    name: &str,
    value: u32,
    section: i16,
    kind: u16,
    class_and_aux: [u8; 2],
) -> Result<()> {
    let mut bytes = [0; 8];
    if name.len() <= 8 {
        bytes[..name.len()].copy_from_slice(name.as_bytes());
    } else {
        bytes[4..].copy_from_slice(&string(strings, name)?.to_le_bytes());
    }
    table.extend_from_slice(&bytes);
    table.extend_from_slice(&value.to_le_bytes());
    table.extend_from_slice(&section.to_le_bytes());
    table.extend_from_slice(&kind.to_le_bytes());
    table.extend_from_slice(&class_and_aux);
    Ok(())
}

fn string(table: &mut ArenaVec<'_, u8>, text: &str) -> Result<u32> {
    let at = narrow(table.len() as u64)?;
    table.extend_from_slice(text.as_bytes());
    table.push(0);
    Ok(at)
}

fn section_number(id: usize) -> Result<i16> {
    i16::try_from(id + 1).map_err(|_| Error("too many COFF sections"))
}

fn reloc_count(count: usize) -> Result<u16> {
    u16::try_from(count).map_err(|_| Error("too many COFF relocations"))
}

fn narrow(value: u64) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error("COFF object exceeds 32-bit limits"))
}
fn put32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

// LLVM's section aux checksum is the uncomplemented reflected CRC32, seed 0.
// Table lookup keeps large constant sections linear without eight bit steps
// per byte in debug builds.
fn checksum(data: &[u8]) -> u32 {
    let mut crc = 0_u32;
    for byte in data {
        crc = (crc >> 8) ^ CRC_TABLE[usize::from(crc.to_le_bytes()[0] ^ *byte)];
    }
    crc
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    let mut seed = 0_u32;
    while index < 256 {
        let mut crc = seed;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0_u32.wrapping_sub(crc & 1)));
            bit += 1;
        }
        table[index] = crc;
        index += 1;
        seed += 1;
    }
    table
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::native::object::Reloc,
        util::bump::Bump,
    };

    #[test]
    fn coff_rel32_variants_preserve_types_and_implicit_addends() {
        let arena = Bump::new();
        let mut coff = Coff::new(&arena, false);
        let section = coff.section(".text", 0x6000_0020, None, 0).unwrap();
        let target = coff.object.symbol("external");
        coff.object.sections[section].data.resize(24, 0);
        coff.object.sections[section].size = 24;
        for variant in 0..=5 {
            coff.object.sections[section].relocs.push(Reloc {
                offset: variant * 4,
                symbol: target,
                kind:   0x0001_0004 + u32::try_from(variant).unwrap(),
                addend: 7,
                named:  false,
            });
        }
        let bytes = write(&coff).unwrap();
        let pointer = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let raw = pointer(40);
        let relocs = pointer(44);
        for variant in 0..=5 {
            assert_eq!(pointer(raw + variant * 4), 7);
            let at = relocs + variant * 10;
            assert_eq!(pointer(at), variant * 4);
            assert_eq!(
                u16::from_le_bytes(bytes[at + 8..at + 10].try_into().unwrap()),
                4 + u16::try_from(variant).unwrap()
            );
        }
    }
}
