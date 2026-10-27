//! x86-64 register names in clobber lists, with or without `%`: every
//! width of the sixteen general registers, `%xmm`/`%ymm`/`%zmm` 0-31, the
//! x87 stack, MMX and mask registers, the flags under GCC's and LLVM's
//! names, and `memory`.
//! GNU extension: GCC manual, "Extended Asm", "Clobbers and Scratch
//! Registers".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>

use super::super::AsmClobbers;

/// Adds the clobber `name` to `clobbers`; `false` for a name that is not
/// an x86-64 register or `memory`.
pub(super) fn clobber(name: &str, clobbers: &mut AsmClobbers) -> bool {
    let name = name.strip_prefix('%').unwrap_or(name);
    match name {
        | "memory" => clobbers.memory = true,
        | "cc" | "flags" | "eflags" | "rflags" | "dirflag" | "fpsr" | "fpcr" | "mxcsr" =>
            clobbers.flags = true,
        | "st" => clobbers.x87 = true,
        | _ =>
            if let Some(register) = general(name) {
                clobbers.general |= 1 << register;
            } else if let Some(register) = numbered(name, &["xmm", "ymm", "zmm"], 32) {
                clobbers.sse |= 1 << register;
            } else if let Some(register) = numbered(name, &["mm"], 8) {
                clobbers.mmx |= 1 << register;
            } else if let Some(register) = numbered(name, &["k"], 8) {
                clobbers.mask |= 1 << register;
            } else if name
                .strip_prefix("st(")
                .and_then(|rest| rest.strip_suffix(')'))
                .is_some_and(|number| {
                    matches!(number, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7")
                })
            {
                clobbers.x87 = true;
            } else {
                return false;
            },
    }
    true
}

/// The hardware encoding of a general register named at any width.
pub(super) fn general(name: &str) -> Option<u32> {
    const LEGACY: [[&str; 5]; 8] = [
        ["rax", "eax", "ax", "al", "ah"],
        ["rcx", "ecx", "cx", "cl", "ch"],
        ["rdx", "edx", "dx", "dl", "dh"],
        ["rbx", "ebx", "bx", "bl", "bh"],
        ["rsp", "esp", "sp", "spl", ""],
        ["rbp", "ebp", "bp", "bpl", ""],
        ["rsi", "esi", "si", "sil", ""],
        ["rdi", "edi", "di", "dil", ""],
    ];
    if let Some(register) = LEGACY
        .iter()
        .position(|names| !name.is_empty() && names.contains(&name))
    {
        return u32::try_from(register).ok();
    }
    let number = name.strip_prefix('r')?;
    let number = ["d", "w", "b", "l"]
        .iter()
        .find_map(|suffix| number.strip_suffix(suffix))
        .unwrap_or(number);
    let register: u32 = number.parse().ok()?;
    ((8..16).contains(&register) && !number.starts_with('0')).then_some(register)
}

/// The number of a register named `prefix` and a decimal below `count`.
fn numbered(name: &str, prefixes: &[&str], count: u32) -> Option<u32> {
    let number = prefixes
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))?;
    if number.is_empty() || (number.len() > 1 && number.starts_with('0')) {
        return None;
    }
    let register: u32 = number.parse().ok()?;
    (register < count).then_some(register)
}
