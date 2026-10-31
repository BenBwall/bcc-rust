//! x86-64 register names in clobber lists, with or without `%`: every
//! width of the sixteen general registers, `%xmm`/`%ymm`/`%zmm` 0-31, the
//! x87 stack, MMX and mask registers, the flags under GCC's and LLVM's
//! names, and `memory`; and the registers GNU local register variables name,
//! as GCC and Clang each accept them.
//! GNU extension: GCC manual, "Extended Asm", "Clobbers and Scratch
//! Registers", and "Local Register Variables".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>

use super::super::AsmClobbers;
use crate::configuration::RuleSet;

/// What the register of a GNU local register variable is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::translation_phases::semantic_analysis) enum RegisterName {
    /// A general register, by hardware encoding, which an `asm` operand
    /// can be placed in.
    General(u8),
    /// A register the compiler accepts that is no general register (SSE,
    /// MMX, x87, mask), which lowering does not place operands in.
    Other,
    /// No register: GCC's "invalid register name", Clang's "unknown
    /// register name".
    Invalid,
    /// GCC: a register the x86-64 target without APX or AVX-512 cannot
    /// access (`r16`, `xmm16`, `k1`).
    Inaccessible,
    /// GCC: a register no variable can live in (`flags`, `fpsr`).
    NotGeneral,
}

/// The registers GCC's x86-64 table numbers, which a register variable may
/// name by number.
const GCC_REGISTERS: usize = 92;

/// The register `name` (with or without `%`) names for a local register
/// variable under `rules`, as `gcc -m64` and the bundled Clang 23 accept
/// them: GCC takes the extended general registers by their 64-bit names
/// alone and `sil`-`bpl`; Clang takes `r8d`-`r15b` but not `sil`-`bpl`.
pub(in crate::translation_phases::semantic_analysis) fn register_variable(
    name: &str,
    rules: RuleSet,
) -> RegisterName {
    let name = name.strip_prefix('%').unwrap_or(name);
    let clang = rules == RuleSet::Clang;
    // Both compilers take a register's number in GCC's x86 table: `ax`,
    // `dx`, `cx`, `bx`, `si`, `di`, `bp`, `sp`, then the x87, flags, SSE
    // and other registers.
    if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
        const GENERAL: [u8; 8] = [0, 2, 1, 3, 6, 7, 5, 4];
        return match name.parse::<usize>() {
            | Ok(number) if number < GENERAL.len() => RegisterName::General(GENERAL[number]),
            | Ok(number) if number < GCC_REGISTERS => RegisterName::Other,
            | _ => RegisterName::Invalid,
        };
    }
    // Clang places a value in the high byte of `ah`-`dh`, which lowering
    // does not model; GCC places it in the low byte.
    if clang && matches!(name, "ah" | "bh" | "ch" | "dh") {
        return RegisterName::Other;
    }
    if let Some(register) = general(name) {
        let extended = register >= 8;
        let byte_legacy = matches!(name, "sil" | "dil" | "spl" | "bpl");
        let plain = name
            .strip_prefix('r')
            .is_some_and(|number| number.parse::<u8>().is_ok());
        let suffix_l = extended && name.ends_with('l');
        let accepted = if clang {
            !byte_legacy && !suffix_l
        } else {
            !extended || plain
        };
        return if accepted {
            RegisterName::General(u8::try_from(register).expect("sixteen registers"))
        } else {
            RegisterName::Invalid
        };
    }
    let apx = name
        .strip_prefix('r')
        .map(|rest| {
            ["d", "w", "b"]
                .iter()
                .find_map(|suffix| rest.strip_suffix(suffix))
                .map_or((rest, false), |rest| (rest, true))
        })
        .and_then(|(number, suffixed)| {
            (!number.starts_with('0'))
                .then(|| number.parse::<u32>().ok())
                .flatten()
                .filter(|number| (16..32).contains(number))
                .map(|_| suffixed)
        });
    if let Some(suffixed) = apx {
        return match (clang, suffixed) {
            | (true, _) => RegisterName::Other,
            | (false, false) => RegisterName::Inaccessible,
            | (false, true) => RegisterName::Invalid,
        };
    }
    if let Some(register) = numbered(name, &["xmm", "ymm", "zmm"], 32) {
        return if clang || register < 16 {
            RegisterName::Other
        } else {
            RegisterName::Inaccessible
        };
    }
    if numbered(name, &["mm"], 8).is_some()
        || name == "st"
        || name
            .strip_prefix("st(")
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|number| matches!(number, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7"))
    {
        return RegisterName::Other;
    }
    if numbered(name, &["k"], 8).is_some() {
        return if clang {
            RegisterName::Other
        } else {
            RegisterName::Inaccessible
        };
    }
    match name {
        | "flags" | "fpsr" if !clang => RegisterName::NotGeneral,
        | "flags" | "fpsr" | "dirflag" if clang => RegisterName::Other,
        | _ => RegisterName::Invalid,
    }
}

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
