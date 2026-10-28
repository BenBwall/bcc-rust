//! `.eh_frame` CIE/FDE records from the exact CFI events in the shared emitter.
//! LSB 5.0 §10.6; DWARF 5 §6.4. CFA register numbers follow AMD64 psABI §3.6.2.

use super::{
    Error,
    Kind,
    Object,
    Reloc,
    Result,
    Symbol,
    listing::{
        number,
        split,
    },
};
use crate::util::bump::ArenaVec;

pub(super) fn generate(object: &mut Object<'_>) -> Result<()> {
    if !object.items.iter().any(|i| matches!(i.kind, Kind::Cfi(..))) {
        return Ok(());
    }
    if object
        .sections
        .iter()
        .any(|section| section.name == ".eh_frame")
    {
        return Err(Error(
            "user section conflicts with generated unwind section",
        ));
    }
    let section = object.section(".eh_frame", 0x7000_0001, 2, None)?;
    object.sections[section].align = 8;
    let mut data = ArenaVec::new_in(object.arena);
    // Version 1, augmentation zR, code alignment 1, data alignment -8,
    // return address RIP (16), pcrel/sdata4 pointers, CFA = RSP+8, RIP at
    // CFA-8.
    data.extend_from_slice(&[
        0x14, 0, 0, 0, 0, 0, 0, 0, 1, b'z', b'R', 0, 1, 0x78, 16, 1, 0x1B, 0x0C, 7, 8, 0x90, 1, 0,
        0,
    ]);
    let mut remaining = object
        .items
        .iter()
        .filter(|item| matches!(item.kind, Kind::Cfi(".cfi_startproc", _)))
        .count();
    let mut active = None;
    let mut start = 0;
    let mut location = 0;
    let mut record = 0;
    let mut remembered = 0_u32;
    for item in &object.items {
        let Kind::Cfi(op, args) = item.kind else {
            continue;
        };
        match op {
            | ".cfi_startproc" => {
                if active.is_some() {
                    return Err(Error("nested CFI procedure"));
                }
                active = Some(item.section);
                remembered = 0;
                remaining -= 1;
                start = item.offset;
                location = start;
                record = data.len();
                data.extend_from_slice(&[0; 4]);
                data.extend_from_slice(
                    &u32::try_from(record + 4)
                        .map_err(|_| Error("unwind table too large"))?
                        .to_le_bytes(),
                );
                let symbol = object.symbols.len();
                object.symbols.push(Symbol {
                    name:       "",
                    section:    Some(item.section),
                    value:      0,
                    size:       0,
                    binding:    0,
                    kind:       3,
                    visibility: 0,
                    alias:      None,
                    common:     false,
                });
                object.sections[section].relocs.push(Reloc {
                    offset: data.len() as u64,
                    symbol,
                    kind: 2,
                    named: false,
                    addend: i64::try_from(start).map_err(|_| Error("function offset too large"))?,
                });
                data.extend_from_slice(&[0; 8]);
                data.push(0); // z augmentation length.
            },
            | ".cfi_endproc" => {
                if active.take() != Some(item.section) {
                    return Err(Error("CFI end outside procedure"));
                }
                let length = u32::try_from(item.offset - start)
                    .map_err(|_| Error("unwind range exceeds 32 bits"))?;
                data[record + 12..record + 16].copy_from_slice(&length.to_le_bytes());
                while !data
                    .len()
                    .is_multiple_of(if remaining == 0 { 8 } else { 4 })
                {
                    data.push(0);
                }
                let size = u32::try_from(data.len() - record - 4)
                    .map_err(|_| Error("unwind table too large"))?;
                data[record..record + 4].copy_from_slice(&size.to_le_bytes());
            },
            | _ => {
                if active != Some(item.section) {
                    return Err(Error("CFI rule outside procedure"));
                }
                let delta = item.offset - location;
                if delta > 0 {
                    if delta < 64 {
                        data.push(
                            0x40 + u8::try_from(delta).map_err(|_| Error("CFI delta too large"))?,
                        );
                    } else if let Ok(delta) = u8::try_from(delta) {
                        data.extend_from_slice(&[2, delta]);
                    } else if let Ok(delta) = u16::try_from(delta) {
                        data.push(3);
                        data.extend_from_slice(&delta.to_le_bytes());
                    } else {
                        data.push(4);
                        data.extend_from_slice(
                            &u32::try_from(delta)
                                .map_err(|_| Error("CFI delta exceeds 32 bits"))?
                                .to_le_bytes(),
                        );
                    }
                    location = item.offset;
                }
                let mut parts = split(args);
                match op {
                    | ".cfi_def_cfa_offset" => {
                        data.push(0x0E);
                        uleb(
                            &mut data,
                            number(args)?
                                .try_into()
                                .map_err(|_| Error("negative CFA offset"))?,
                        );
                    },
                    | ".cfi_offset" => {
                        let reg = dwarf(parts.next().ok_or(Error("missing CFI register"))?)?;
                        let offset = number(parts.next().ok_or(Error("missing CFI offset"))?)?;
                        if offset > 0 || offset % 8 != 0 {
                            return Err(Error("invalid CFI saved-register offset"));
                        }
                        data.push(0x80 + reg);
                        uleb(&mut data, offset.unsigned_abs() / 8);
                    },
                    | ".cfi_def_cfa_register" => {
                        data.push(0x0D);
                        uleb(&mut data, u64::from(dwarf(args)?));
                    },
                    | ".cfi_def_cfa" => {
                        data.push(0x0C);
                        uleb(
                            &mut data,
                            u64::from(dwarf(parts.next().ok_or(Error("missing CFA register"))?)?),
                        );
                        uleb(
                            &mut data,
                            number(parts.next().ok_or(Error("missing CFA offset"))?)?
                                .try_into()
                                .map_err(|_| Error("negative CFA offset"))?,
                        );
                    },
                    | ".cfi_remember_state" => {
                        remembered = remembered
                            .checked_add(1)
                            .ok_or(Error("CFI state depth overflow"))?;
                        data.push(0x0A);
                    },
                    | ".cfi_restore_state" => {
                        remembered = remembered
                            .checked_sub(1)
                            .ok_or(Error("CFI restore without remembered state"))?;
                        data.push(0x0B);
                    },
                    | _ => return Err(Error("unknown CFI directive")),
                }
            },
        }
    }
    if active.is_some() {
        return Err(Error("unterminated CFI procedure"));
    }
    object.sections[section].size = data.len() as u64;
    object.sections[section].data = data;
    Ok(())
}

fn uleb(data: &mut ArenaVec<'_, u8>, mut value: u64) {
    loop {
        let byte = value.to_le_bytes()[0] & 0x7F;
        value >>= 7;
        data.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            break;
        }
    }
}

fn dwarf(reg: &str) -> Result<u8> {
    Ok(match reg {
        | "%rax" => 0,
        | "%rdx" => 1,
        | "%rcx" => 2,
        | "%rbx" => 3,
        | "%rsi" => 4,
        | "%rdi" => 5,
        | "%rbp" => 6,
        | "%rsp" => 7,
        | "%r8" => 8,
        | "%r9" => 9,
        | "%r10" => 10,
        | "%r11" => 11,
        | "%r12" => 12,
        | "%r13" => 13,
        | "%r14" => 14,
        | "%r15" => 15,
        | _ => return Err(Error("unknown CFI register")),
    })
}
