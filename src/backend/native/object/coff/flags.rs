//! LLVM `COFFAsmParser`'s ordered flag interpretation and `MCObjectFileInfo`'s
//! pre-created AMD64 section attributes. These are object-format rules.

use super::{
    Error,
    Result,
};

pub(super) fn parse(name: &str, flags: &str) -> Result<u32> {
    const ALLOC: u16 = 1;
    const CODE: u16 = 2;
    const LOAD: u16 = 4;
    const DATA: u16 = 8;
    const SHARED: u16 = 16;
    const NOLOAD: u16 = 32;
    const NOREAD: u16 = 64;
    const NOWRITE: u16 = 128;
    const DISCARD: u16 = 256;
    const INFO: u16 = 512;
    let mut bits = 0;
    let mut readonly_removed = false;
    for flag in flags.bytes() {
        match flag {
            | b'a' => {},
            | b'b' => {
                if bits & DATA != 0 {
                    return Err(Error("conflicting COFF section flags"));
                }
                bits = (bits | ALLOC) & !LOAD;
            },
            | b'd' => {
                if bits & ALLOC != 0 {
                    return Err(Error("conflicting COFF section flags"));
                }
                bits = (bits | DATA) & !NOWRITE;
                if bits & NOLOAD == 0 {
                    bits |= LOAD;
                }
            },
            | b'n' => bits = (bits | NOLOAD) & !LOAD,
            | b'D' => bits |= DISCARD,
            | b'r' => {
                readonly_removed = false;
                bits |= NOWRITE;
                if bits & CODE == 0 {
                    bits |= DATA;
                }
                if bits & NOLOAD == 0 {
                    bits |= LOAD;
                }
            },
            | b's' => {
                bits = (bits | SHARED | DATA) & !NOWRITE;
                if bits & NOLOAD == 0 {
                    bits |= LOAD;
                }
            },
            | b'w' => {
                bits &= !NOWRITE;
                readonly_removed = true;
            },
            | b'x' => {
                bits |= CODE;
                if bits & NOLOAD == 0 {
                    bits |= LOAD;
                }
                if !readonly_removed {
                    bits |= NOWRITE;
                }
            },
            | b'y' => bits |= NOREAD | NOWRITE,
            | b'i' => bits |= INFO,
            | _ => return Err(Error("unknown COFF section flag")),
        }
    }
    if bits == 0 {
        bits = DATA;
    }
    let mut result = 0;
    for (enabled, value) in [
        (bits & CODE != 0, 0x2000_0020),
        (bits & DATA != 0, 0x40),
        (bits & ALLOC != 0 && bits & LOAD == 0, 0x80),
        (bits & NOLOAD != 0, 0x800),
        (
            bits & DISCARD != 0 || name.starts_with(".debug"),
            0x0200_0000,
        ),
        (bits & NOREAD == 0, 0x4000_0000),
        (bits & NOWRITE == 0, 0x8000_0000),
        (bits & SHARED != 0, 0x1000_0000),
        (bits & INFO != 0, 0x200),
    ] {
        if enabled {
            result |= value;
        }
    }
    Ok(result)
}

pub(super) fn predefined(name: &str) -> Option<u32> {
    match name {
        | ".eh_frame" | ".rdata" | ".pdata" | ".xdata" | ".gehcont$y" | ".gfids$y" | ".giats$y"
        | ".gljmp$y" | ".llvm_stackmaps" => Some(0x4000_0040),
        | ".bss" => Some(0xC000_0080),
        | ".text" => Some(0x6000_0020),
        | ".data" | ".tls$" => Some(0xC000_0040),
        | ".retplne" | ".sxdata" => Some(0x0000_0200),
        | ".debug$S"
        | ".debug$T"
        | ".debug$H"
        | ".debug_abbrev"
        | ".debug_info"
        | ".debug_line"
        | ".debug_line_str"
        | ".debug_frame"
        | ".debug_pubnames"
        | ".debug_pubtypes"
        | ".debug_gnu_pubnames"
        | ".debug_gnu_pubtypes"
        | ".debug_str"
        | ".debug_str_offsets"
        | ".debug_loc"
        | ".debug_loclists"
        | ".debug_aranges"
        | ".debug_ranges"
        | ".debug_rnglists"
        | ".debug_macinfo"
        | ".debug_macro"
        | ".debug_addr"
        | ".debug_cu_index"
        | ".debug_tu_index"
        | ".debug_names"
        | ".apple_names"
        | ".apple_namespaces"
        | ".apple_types"
        | ".apple_objc"
        | ".pseudo_probe"
        | ".pseudo_probe_desc" => Some(0x4200_0040),
        | ".debug_macinfo.dwo"
        | ".debug_macro.dwo"
        | ".debug_info.dwo"
        | ".debug_types.dwo"
        | ".debug_abbrev.dwo"
        | ".debug_str.dwo"
        | ".debug_line.dwo"
        | ".debug_loc.dwo"
        | ".debug_loclists.dwo"
        | ".debug_str_offsets.dwo"
        | ".debug_rnglists.dwo" => Some(0x4200_0840),
        | ".drectve" => Some(0x0000_0A00),
        | _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::{
        backend::native::tests::object_coff::equivalent,
        target::Target,
        util::bump::{
            ArenaString,
            ArenaVec,
            Bump,
        },
    };

    #[test]
    fn coff_section_flags_match_clang() {
        let arena = Bump::new();
        let letters = "abdnDrswxyi";
        let mut cases = ArenaVec::new_in(&arena);
        cases.push("");
        for first in letters.chars() {
            let mut flags = ArenaString::new_in(&arena);
            flags.push(first);
            cases.push(flags.into_str());
            for second in letters.chars() {
                let mut flags = ArenaString::new_in(&arena);
                flags.push(first);
                flags.push(second);
                cases.push(flags.into_str());
            }
        }
        cases.extend_from_slice(&["xr", "dr", "dw", "bw", "yni", "wrx", "rwx", "nrd", "dnr"]);
        for &flags in &cases {
            if parse("probe", flags).is_err() {
                continue;
            }
            for target in [Target::WindowsGnu, Target::WindowsMsvc] {
                let mut text = ArenaString::new_in(&arena);
                write!(&mut text, ".section probe,\"{flags}\"\n.zero 1\n").unwrap();
                equivalent(&text, target).unwrap();
            }
        }
        for flags in ["bd", "db", "brd", "q"] {
            _ = parse("probe", flags).unwrap_err();
        }
    }

    #[test]
    fn coff_predefined_section_names_match_clang() {
        let arena = Bump::new();
        for name in [
            ".eh_frame",
            ".rdata",
            ".gehcont$y",
            ".gfids$y",
            ".giats$y",
            ".gljmp$y",
            ".llvm_stackmaps",
            ".bss",
            ".text",
            ".data",
            ".tls$",
            ".retplne",
            ".sxdata",
            ".debug$S",
            ".debug$T",
            ".debug$H",
            ".debug_abbrev",
            ".debug_info",
            ".debug_line",
            ".debug_line_str",
            ".debug_frame",
            ".debug_pubnames",
            ".debug_pubtypes",
            ".debug_gnu_pubnames",
            ".debug_gnu_pubtypes",
            ".debug_str",
            ".debug_str_offsets",
            ".debug_loc",
            ".debug_loclists",
            ".debug_aranges",
            ".debug_ranges",
            ".debug_rnglists",
            ".debug_macinfo",
            ".debug_macro",
            ".debug_addr",
            ".debug_cu_index",
            ".debug_tu_index",
            ".debug_names",
            ".apple_names",
            ".apple_namespaces",
            ".apple_types",
            ".apple_objc",
            ".pseudo_probe",
            ".pseudo_probe_desc",
            ".debug_macinfo.dwo",
            ".debug_macro.dwo",
            ".debug_info.dwo",
            ".debug_types.dwo",
            ".debug_abbrev.dwo",
            ".debug_str.dwo",
            ".debug_line.dwo",
            ".debug_loc.dwo",
            ".debug_loclists.dwo",
            ".debug_str_offsets.dwo",
            ".debug_rnglists.dwo",
            ".drectve",
            "probe",
            ".rdata$foo",
            ".text$foo",
            ".debug_custom",
        ] {
            for flags in ["", ",\"dw\"", ",\"dr\""] {
                for target in [Target::WindowsGnu, Target::WindowsMsvc] {
                    let mut text = ArenaString::new_in(&arena);
                    write!(&mut text, ".section {name}{flags}\n.zero 1\n").unwrap();
                    equivalent(&text, target).unwrap();
                }
            }
        }
    }
}
