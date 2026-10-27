//! Inline assembly: the descriptor an `asm` instruction names, kept in the
//! module so the 16-byte instruction record holds only its handle and its
//! operand values.
//!
//! An `asm` instruction has no result. Its operands are values in
//! descriptor order, one per [`AsmOperand`]:
//!
//! - a register or flag **output** is a `ptr`: the assembly's result is stored
//!   there, as a plain store of the operand's type, after the assembly runs;
//! - a **memory** operand, input or output, is the `ptr` the assembly
//!   addresses;
//! - a register, immediate or tied **input** is a value of the operand's type;
//!   an immediate is an `iconst`, a `global_addr` or a `func_addr`.
//!
//! The template is in LLVM's syntax: `$N` or `${N:m}` is operand `N`
//! printed with modifier `m`, `${:uid}` a number unique to the instruction,
//! `$$` a dollar sign, and `$(`, `$|` and `$)` delimit alternatives for each
//! assembler dialect, AT&T first. Passes treat every `asm` as a call that may
//! read and write any memory it can reach, never removing, merging or
//! duplicating it.
//!
//! GNU extension: GCC manual, "Extended Asm".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
//! LLVM Language Reference, "Inline Assembler Expressions".
//! <https://llvm.org/docs/LangRef.html#inline-assembler-expressions>

use std::fmt;

use super::types::Type;
use crate::util::bump::Bump;

/// An inline assembly statement as the back ends emit it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct InlineAsm<'ir> {
    pub(crate) template:     &'ir str,
    pub(crate) operands:     &'ir [AsmOperand],
    pub(crate) clobbers:     AsmClobbers,
    /// The assembly has effects besides its outputs (`volatile`), so it is
    /// neither removed nor merged with an identical one.
    pub(crate) side_effects: bool,
}

impl InlineAsm<'_> {
    /// Copies the descriptor's template and operands into `arena`.
    pub(crate) fn copy_into<'a>(&self, arena: &'a Bump) -> InlineAsm<'a> {
        InlineAsm {
            template:     arena.alloc_str(self.template),
            operands:     arena.alloc_slice_copy(self.operands),
            clobbers:     self.clobbers,
            side_effects: self.side_effects,
        }
    }

    /// Calls `report` with each rule the descriptor breaks, apart from those
    /// on its operand values: outputs come first; each operand's type fits
    /// its constraint (a general register holds an integer or pointer of at
    /// most 64 bits, an SSE register a `f32` or `f64`, a flag output an
    /// integer of at most 64 bits); a tied input names an earlier register
    /// output of its type; only outputs are early-clobber; and the template
    /// is well formed and names existing operands. Lowering checks the same
    /// rules before it emits a descriptor.
    pub(crate) fn check(&self, mut report: impl FnMut(&'static str)) {
        use AsmConstraint as C;
        let outputs = self.output_count();
        if self.operands[outputs..]
            .iter()
            .any(|operand| operand.output)
        {
            report("an output follows an input");
        }
        for operand in self.operands {
            let ty = operand.ty;
            let rule = match operand.constraint {
                | C::General(mask) if mask == 0 || mask & (1 << 4) != 0 =>
                    Some("a general register set is empty or holds %rsp"),
                | C::General(_)
                    if !(ty == Type::Ptr || (ty.is_int() && ty != Type::I1 && ty.bits() <= 64)) =>
                    Some("a general register holds an integer or pointer of at most 64 bits"),
                | C::Sse if !matches!(ty, Type::F32 | Type::F64) =>
                    Some("an SSE register holds a f32 or f64"),
                | C::Memory if operand.bytes == 0 => Some("a memory operand has no bytes"),
                | C::Immediate if operand.output => Some("an immediate is an output"),
                | C::Immediate if !(ty.is_int() || ty == Type::Ptr) =>
                    Some("an immediate is not an integer or pointer"),
                | C::Tied(output) => match self.operands.get(output as usize) {
                    | _ if operand.output => Some("a tied operand is an output"),
                    | Some(tied)
                        if (output as usize) < outputs
                            && matches!(tied.constraint, C::General(_) | C::Sse)
                            && tied.ty == ty =>
                        None,
                    | _ => Some("a tied input names no register output of its type"),
                },
                | C::Flag(condition)
                    if !operand.output
                        || !ty.is_int()
                        || ty.bits() > 64
                        || usize::from(condition) >= CONDITIONS.len() =>
                    Some("a flag operand is not an integer output of at most 64 bits"),
                | _ if operand.early_clobber && !operand.output =>
                    Some("an input is early-clobber"),
                | _ => None,
            };
            if let Some(rule) = rule {
                report(rule);
            }
        }
        for piece in template_pieces(self.template) {
            let rule = match piece {
                | Err(()) => Some("the template has a malformed `$` escape"),
                | Ok(TemplatePiece::Operand { operand, .. })
                    if operand as usize >= self.operands.len() =>
                    Some("the template names a missing operand"),
                | Ok(_) => None,
            };
            if let Some(rule) = rule {
                report(rule);
                break;
            }
        }
    }

    /// The number of operands that are outputs; they come first.
    pub(crate) fn output_count(&self) -> usize {
        self.operands
            .iter()
            .take_while(|operand| operand.output)
            .count()
    }
}

/// One operand: whether the assembly writes it, where it lives, and its
/// type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AsmOperand {
    pub(crate) output:        bool,
    /// The output is written before every input is read, so it shares no
    /// register with an input.
    pub(crate) early_clobber: bool,
    pub(crate) constraint:    AsmConstraint,
    /// The register's or immediate's type, or the scalar a memory operand
    /// holds.
    pub(crate) ty:            Type,
    /// A memory operand's size in bytes; for other operands the bytes of
    /// `ty`.
    pub(crate) bytes:         u32,
}

/// Where an operand lives while the assembly runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum AsmConstraint {
    /// A general register from the mask, by hardware encoding (`%rax` is bit
    /// 0, `%rsp` bit 4, which no mask holds).
    General(u16),
    /// An SSE register.
    Sse,
    /// The object at the operand's address.
    Memory,
    /// A constant written into the instruction.
    Immediate,
    /// An input in the same register as output operand `N`.
    Tied(u32),
    /// An integer that is 1 when the x86 condition holds after the
    /// assembly, else 0. The index is into [`CONDITIONS`].
    Flag(u8),
}

impl AsmConstraint {
    /// Every general register but `%rsp`.
    pub(crate) const ANY_GENERAL: u16 = !(1 << 4);
}

/// The 64-bit names of the general registers, by hardware encoding.
pub(crate) const GENERAL_NAMES: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// The x86 conditions of flag outputs, by GCC's spelling.
/// GNU extension: GCC manual, "Flag Output Operands".
pub(crate) const CONDITIONS: [&str; 30] = [
    "a", "ae", "b", "be", "c", "e", "g", "ge", "l", "le", "na", "nae", "nb", "nbe", "nc", "ne",
    "ng", "nge", "nl", "nle", "no", "np", "ns", "nz", "o", "p", "pe", "po", "s", "z",
];

/// Registers and state the assembly overwrites besides its outputs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) struct AsmClobbers {
    /// The assembly may read and write any memory, not only its operands.
    pub(crate) memory:  bool,
    /// The flags register.
    pub(crate) flags:   bool,
    /// General registers by hardware encoding.
    pub(crate) general: u16,
    /// `%xmm0`-`%xmm31`.
    pub(crate) sse:     u32,
    /// The x87 register stack.
    pub(crate) x87:     bool,
    /// `%mm0`-`%mm7`.
    pub(crate) mmx:     u8,
    /// `%k0`-`%k7`.
    pub(crate) mask:    u8,
}

impl AsmClobbers {
    /// Calls `visit` with the textual name of each clobber, in printing
    /// order.
    pub(crate) fn names(self, mut visit: impl FnMut(&dyn fmt::Display)) {
        if self.memory {
            visit(&"memory");
        }
        if self.flags {
            visit(&"flags");
        }
        for (index, name) in GENERAL_NAMES.iter().enumerate() {
            if self.general & (1 << index) != 0 {
                visit(name);
            }
        }
        for index in 0..32 {
            if self.sse & (1 << index) != 0 {
                visit(&format_args!("xmm{index}"));
            }
        }
        if self.x87 {
            visit(&"st");
        }
        for index in 0..8 {
            if self.mmx & (1 << index) != 0 {
                visit(&format_args!("mm{index}"));
            }
        }
        for index in 0..8 {
            if self.mask & (1 << index) != 0 {
                visit(&format_args!("k{index}"));
            }
        }
    }

    /// Adds the clobber the textual form names `name`; `false` if it names
    /// none.
    pub(crate) fn add(&mut self, name: &str) -> bool {
        let numbered = |prefix: &str, count: u32| {
            name.strip_prefix(prefix)
                .filter(|number| {
                    !number.is_empty() && (number.len() == 1 || !number.starts_with('0'))
                })
                .and_then(|number| number.parse::<u32>().ok())
                .filter(|&number| number < count)
        };
        match name {
            | "memory" => self.memory = true,
            | "flags" => self.flags = true,
            | "st" => self.x87 = true,
            | _ =>
                if let Some(index) = GENERAL_NAMES.iter().position(|&known| known == name) {
                    self.general |= 1 << index;
                } else if let Some(index) = numbered("xmm", 32) {
                    self.sse |= 1 << index;
                } else if let Some(index) = numbered("mm", 8) {
                    self.mmx |= 1 << index;
                } else if let Some(index) = numbered("k", 8) {
                    self.mask |= 1 << index;
                } else {
                    return false;
                },
        }
        true
    }
}

impl fmt::Display for AsmConstraint {
    /// `r` for any general register, `{rax,rdx}` for a set, `x`, `m`, `i`,
    /// the tied output's number, or `@cc` and the condition.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            | Self::General(Self::ANY_GENERAL) => f.write_str("r"),
            | Self::General(mask) => {
                f.write_str("{")?;
                let mut separator = "";
                for (index, name) in GENERAL_NAMES.iter().enumerate() {
                    if mask & (1 << index) != 0 {
                        write!(f, "{separator}{name}")?;
                        separator = ",";
                    }
                }
                f.write_str("}")
            },
            | Self::Sse => f.write_str("x"),
            | Self::Memory => f.write_str("m"),
            | Self::Immediate => f.write_str("i"),
            | Self::Tied(output) => write!(f, "{output}"),
            | Self::Flag(condition) => write!(f, "@cc{}", CONDITIONS[usize::from(condition)]),
        }
    }
}

impl AsmConstraint {
    /// The constraint the textual form spells `text`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            | "r" => Self::General(Self::ANY_GENERAL),
            | "x" => Self::Sse,
            | "m" => Self::Memory,
            | "i" => Self::Immediate,
            | _ =>
                if let Some(condition) = text.strip_prefix("@cc") {
                    let index = CONDITIONS.iter().position(|&known| known == condition)?;
                    Self::Flag(u8::try_from(index).ok()?)
                } else if let Some(names) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}'))
                {
                    let mut mask = 0_u16;
                    for name in names.split(',') {
                        let index = GENERAL_NAMES.iter().position(|&known| known == name)?;
                        mask |= 1 << index;
                    }
                    Self::General(mask)
                } else {
                    Self::Tied(text.parse().ok()?)
                },
        })
    }
}

/// A piece of a template in LLVM's syntax.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        variant_size_differences,
        reason = "Text borrows its slice of the template; pieces are iterated, never stored."
    )
)]
pub(crate) enum TemplatePiece<'t> {
    Text(&'t str),
    /// `$N` or `${N:m}`.
    Operand {
        operand:  u32,
        modifier: Option<u8>,
    },
    /// `${:uid}`.
    UniqueId,
    /// `$(`, `$|` and `$)`.
    DialectOpen,
    DialectNext,
    DialectClose,
}

/// The pieces of `template`, or `Err` at the first malformed `$` escape.
pub(crate) fn template_pieces(
    template: &str,
) -> impl Iterator<Item = Result<TemplatePiece<'_>, ()>> + '_ {
    let mut rest = template;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let Some(escape) = rest.strip_prefix('$') else {
            let end = rest.find('$').unwrap_or(rest.len());
            let (text, tail) = rest.split_at(end);
            rest = tail;
            return Some(Ok(TemplatePiece::Text(text)));
        };
        let (piece, tail) = match escape.as_bytes().first() {
            | Some(b'$') => (Ok(TemplatePiece::Text("$")), &escape[1..]),
            | Some(b'(') => (Ok(TemplatePiece::DialectOpen), &escape[1..]),
            | Some(b'|') => (Ok(TemplatePiece::DialectNext), &escape[1..]),
            | Some(b')') => (Ok(TemplatePiece::DialectClose), &escape[1..]),
            | Some(b'{') => match escape[1..].split_once('}') {
                | Some((":uid", tail)) => (Ok(TemplatePiece::UniqueId), tail),
                | Some((inner, tail)) => {
                    let (number, modifier) = match inner.split_once(':') {
                        | Some((number, modifier)) if modifier.len() == 1 =>
                            (number, Some(modifier.as_bytes()[0])),
                        | Some(_) => ("", None),
                        | None => (inner, None),
                    };
                    let piece = number
                        .parse()
                        .map(|operand| TemplatePiece::Operand { operand, modifier })
                        .map_err(|_| ());
                    (piece, tail)
                },
                | None => (Err(()), ""),
            },
            | Some(digit) if digit.is_ascii_digit() => {
                let end = escape
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(escape.len());
                let piece = escape[..end]
                    .parse()
                    .map(|operand| TemplatePiece::Operand {
                        operand,
                        modifier: None,
                    })
                    .map_err(|_| ());
                (piece, &escape[end..])
            },
            | _ => (Err(()), ""),
        };
        rest = if piece.is_err() { "" } else { tail };
        Some(piece)
    })
}
