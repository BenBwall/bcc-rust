//! Windows x64 `UNWIND_INFO` and `RUNTIME_FUNCTION` records (Microsoft x64 EH).

use super::{
    super::{
        Kind,
        Reloc,
        Symbol,
        listing::{
            split,
            unsigned,
        },
    },
    Coff,
    Error,
    Result,
};
use crate::util::bump::ArenaVec;

pub(super) fn generate(coff: &mut Coff<'_>) -> Result<()> {
    // MSVC emits ordinary pdata, all associative xdata, then all associative
    // pdata. Multiple COMDAT procedures must not interleave those sections.
    // MinGW's xdata already exists; its pdata follows procedure order.
    for comdat in [false, true] {
        for id in 0..coff.unwind.len() {
            let text = coff.unwind[id].text;
            if (!coff.gnu && coff.object.sections[text].group.is_some()) != comdat {
                continue;
            }
            if coff.unwind[id].xdata.is_none() {
                let xdata = coff.unwind_section(text, ".xdata")?;
                coff.unwind[id].xdata = Some(xdata);
            }
        }
        for id in 0..coff.unwind.len() {
            let text = coff.unwind[id].text;
            if (!coff.gnu && coff.object.sections[text].group.is_some()) == comdat {
                let pdata = coff.unwind_section(text, ".pdata")?;
                coff.unwind[id].pdata = Some(pdata);
            }
        }
    }
    let object = &mut coff.object;
    let mut active = None;
    let mut start = 0;
    let mut prologue = None;
    let mut frame = 0;
    let mut codes = ArenaVec::new_in(object.arena);
    // Each event's slots remain together when reversing prologue operations.
    let mut events = ArenaVec::new_in(object.arena);
    for item in &object.items {
        let Kind::Cfi(op, args) = item.kind else {
            continue;
        };
        match op {
            | ".seh_proc" => {
                if active.is_some() {
                    return Err(Error("nested SEH procedure"));
                }
                let entry = *coff
                    .unwound
                    .get(&item.section)
                    .ok_or(Error("missing SEH sections"))?;
                active = Some(entry);
                start = item.offset;
                prologue = None;
                frame = 0;
                codes.clear();
                events.clear();
            },
            | ".seh_endproc" => {
                let entry = active.take().ok_or(Error("SEH end outside procedure"))?;
                if coff.unwind[entry].text != item.section {
                    return Err(Error("SEH procedure crosses sections"));
                }
                let xdata = coff.unwind[entry].xdata.ok_or(Error("missing SEH xdata"))?;
                let pdata = coff.unwind[entry].pdata.ok_or(Error("missing SEH pdata"))?;
                let xoffset = object.sections[xdata].data.len() as u64;
                let count = u8::try_from(codes.len() / 2)
                    .map_err(|_| Error("too many SEH unwind slots"))?;
                let data = &mut object.sections[xdata].data;
                data.extend_from_slice(&[1, prologue.unwrap_or(0), count, frame]);
                for &(begin, end) in events.iter().rev() {
                    data.extend_from_slice(&codes[begin..end]);
                }
                // LLVM emits at least one two-slot code area, even for leaf
                // functions.
                if count == 0 {
                    data.extend_from_slice(&[0; 4]);
                } else if count & 1 != 0 {
                    data.extend_from_slice(&[0; 2]);
                }
                for (section, addend) in [
                    (item.section, start),
                    (item.section, item.offset),
                    (xdata, xoffset),
                ] {
                    let symbol = object.symbols.len();
                    object.symbols.push(Symbol {
                        name:       "",
                        section:    Some(section),
                        value:      0,
                        size:       0,
                        binding:    0,
                        kind:       3,
                        visibility: 0,
                        alias:      None,
                    });
                    let offset = object.sections[pdata].data.len() as u64;
                    object.sections[pdata].relocs.push(Reloc {
                        offset,
                        symbol,
                        kind: 0x0001_0003,
                        addend: i64::try_from(addend).map_err(|_| Error("SEH offset too large"))?,
                        named: false,
                    });
                    object.sections[pdata].data.extend_from_slice(&[0; 4]);
                }
            },
            | _ => {
                let entry = active.ok_or(Error("SEH directive outside procedure"))?;
                if coff.unwind[entry].text != item.section {
                    return Err(Error("SEH procedure crosses sections"));
                }
                let offset = u8::try_from(item.offset - start)
                    .map_err(|_| Error("SEH prologue exceeds 255 bytes"))?;
                if op == ".seh_endprologue" {
                    if prologue.replace(offset).is_some() {
                        return Err(Error("duplicate SEH endprologue"));
                    }
                    continue;
                }
                if prologue.is_some() {
                    return Err(Error("SEH unwind code after prologue"));
                }
                let begin = codes.len();
                let mut parts = split(args);
                match op {
                    | ".seh_pushreg" => codes.extend_from_slice(&[offset, reg(args)? << 4]),
                    | ".seh_stackalloc" => {
                        let size = unsigned(args)?;
                        if size == 0 || size % 8 != 0 || size > u64::from(u32::MAX) {
                            return Err(Error("invalid SEH stack allocation"));
                        }
                        if size <= 128 {
                            codes.extend_from_slice(&[
                                offset,
                                u8::try_from((size - 8) / 8)
                                    .map_err(|_| Error("SEH allocation overflow"))?
                                    << 4
                                    | 2,
                            ]);
                        } else if let Ok(scaled) = u16::try_from(size / 8) {
                            codes.extend_from_slice(&[offset, 1]);
                            codes.extend_from_slice(&scaled.to_le_bytes());
                        } else {
                            codes.extend_from_slice(&[offset, 0x11]);
                            codes.extend_from_slice(
                                &u32::try_from(size)
                                    .map_err(|_| Error("SEH allocation overflow"))?
                                    .to_le_bytes(),
                            );
                        }
                    },
                    | ".seh_setframe" => {
                        if frame != 0 {
                            return Err(Error("duplicate SEH frame register"));
                        }
                        let register = reg(parts.next().ok_or(Error("missing SEH register"))?)?;
                        let offset_value =
                            unsigned(parts.next().ok_or(Error("missing SEH frame offset"))?)?;
                        if offset_value > 240 || offset_value % 16 != 0 || register == 0 {
                            return Err(Error("invalid SEH frame offset"));
                        }
                        frame = register
                            | (u8::try_from(offset_value)
                                .map_err(|_| Error("SEH frame offset overflow"))?
                                / 16)
                                << 4;
                        codes.extend_from_slice(&[offset, 3]);
                    },
                    | ".seh_savexmm" | ".seh_savereg" => {
                        let name = parts.next().ok_or(Error("missing SEH saved register"))?;
                        let register = if op == ".seh_savexmm" {
                            let n = name
                                .strip_prefix("%xmm")
                                .ok_or(Error("invalid SEH XMM register"))?;
                            let n = u8::try_from(unsigned(n)?)
                                .map_err(|_| Error("invalid SEH XMM register"))?;
                            if n > 15 {
                                return Err(Error("invalid SEH XMM register"));
                            }
                            n
                        } else {
                            reg(name)?
                        };
                        let value =
                            unsigned(parts.next().ok_or(Error("missing SEH save offset"))?)?;
                        let scale = if op == ".seh_savexmm" { 16 } else { 8 };
                        let opcode = if op == ".seh_savexmm" { 8 } else { 4 };
                        if value % scale != 0 || value > u64::from(u32::MAX) {
                            return Err(Error("invalid SEH save offset"));
                        }
                        // LLVM chooses the far form above 512 KiB for both
                        // GPR and XMM saves, even though XMM's 16-byte scale
                        // could represent larger near offsets.
                        if value <= 524_280 {
                            let scaled = u16::try_from(value / scale)
                                .map_err(|_| Error("SEH scaled save offset overflow"))?;
                            codes.extend_from_slice(&[offset, register << 4 | opcode]);
                            codes.extend_from_slice(&scaled.to_le_bytes());
                        } else {
                            codes.extend_from_slice(&[offset, register << 4 | (opcode + 1)]);
                            codes.extend_from_slice(
                                &u32::try_from(value)
                                    .map_err(|_| Error("SEH save offset overflow"))?
                                    .to_le_bytes(),
                            );
                        }
                    },
                    | _ => return Err(Error("unknown SEH directive")),
                }
                events.push((begin, codes.len()));
            },
        }
    }
    if active.is_some() {
        return Err(Error("unterminated SEH procedure"));
    }
    for unwind in &coff.unwind {
        for id in [unwind.xdata, unwind.pdata].into_iter().flatten() {
            let section = &mut object.sections[id];
            section.size = section.data.len() as u64;
        }
    }
    Ok(())
}

fn reg(name: &str) -> Result<u8> {
    [
        "%rax", "%rcx", "%rdx", "%rbx", "%rsp", "%rbp", "%rsi", "%rdi", "%r8", "%r9", "%r10",
        "%r11", "%r12", "%r13", "%r14", "%r15",
    ]
    .iter()
    .position(|&s| s == name)
    .and_then(|n| u8::try_from(n).ok())
    .ok_or(Error("invalid SEH register"))
}
