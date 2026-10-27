//! Retained GNU assembly statements: each checked `asm` statement's template
//! pieces, operands with the constraint alternative analysis chose for them,
//! clobbers and qualifiers, by statement identity. Lowering reads only these
//! facts and the operands' expression records; it never parses a template or
//! a constraint again.
//! C99: §J.5.10, p. 514; PDF p. 526 (the `asm` keyword as a common
//! extension). GNU extension: GCC manual, "Extended Asm" and "Constraints".
//! <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Constraints.html>

use crate::{
    translation_phases::parsing::syntax::{
        Expression,
        Statement,
    },
    util::bump::ArenaMap,
};

/// The checked GNU assembly statements of a translation unit, by statement
/// identity.
#[derive(Debug)]
pub(crate) struct AssemblyFacts<'tu> {
    pub(in crate::translation_phases::semantic_analysis) statements:
        ArenaMap<'tu, usize, AsmFacts<'tu>>,
}

impl<'tu> AssemblyFacts<'tu> {
    /// The facts of a checked `asm` statement; `None` for one with errors.
    pub(crate) fn statement(&self, statement: &Statement<'_>) -> Option<AsmFacts<'tu>> {
        self.statements
            .get(&std::ptr::from_ref(statement).addr())
            .copied()
    }

    /// Every checked statement's facts, in no particular order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &AsmFacts<'tu>> {
        self.statements.values()
    }
}

/// One checked `asm` statement. Operands keep GCC's numbering: outputs in
/// order, then inputs; an output written `+` is one operand that is read
/// and written. Goto labels follow the operands in template numbering.
/// GNU extension: GCC manual, "Extended Asm".
#[derive(Clone, Copy, Debug)]
pub(crate) struct AsmFacts<'tu> {
    /// Written `volatile`, or implicitly volatile: basic assembly, an `asm`
    /// without outputs, and `asm goto`.
    pub(crate) volatile: bool,
    /// The pieces of the template; basic assembly (`asm ("...")` with no
    /// colon) has no substitution and is one text piece.
    pub(crate) template: &'tu [AsmPiece<'tu>],
    pub(crate) operands: &'tu [AsmOperandFacts<'tu>],
    pub(crate) clobbers: AsmClobbers,
    /// The control jump of each `asm goto` label, in label order.
    pub(crate) jumps:    &'tu [super::JumpId],
}

/// A piece of a parsed template.
/// GNU extension: GCC manual, "Extended Asm" (the assembler template),
/// "Generic Operand Modifiers" and "x86 Operand Modifiers".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        variant_size_differences,
        reason = "Text borrows its slice of the template; the pieces live once per statement."
    )
)]
pub(crate) enum AsmPiece<'tu> {
    /// Literal text; `%%`, `%{`, `%|` and `%}` are already unescaped.
    Text(&'tu str),
    /// `%N`, `%[name]` or `%xN`: operand `operand` printed with modifier
    /// `x`.
    Operand { operand: u32, modifier: Option<u8> },
    /// `%lN` or `%l[name]`: the label with this index in the goto list.
    Label(u32),
    /// `%=`: a number unique to this instance of the statement.
    UniqueId,
}

/// One operand with the alternative analysis chose.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AsmOperandFacts<'tu> {
    pub(crate) expression:    &'tu Expression<'tu>,
    pub(crate) role:          AsmRole,
    pub(crate) kind:          AsmOperandKind,
    /// `&`: the output is written before every input is read, so it shares
    /// no register with an input.
    pub(crate) early_clobber: bool,
}

/// Whether an operand is written, read, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsmRole {
    /// `=`: written only.
    Output,
    /// `+`: read and written.
    Update,
    Input,
}

impl AsmRole {
    pub(crate) const fn writes(self) -> bool {
        matches!(self, Self::Output | Self::Update)
    }
}

/// Where an operand lives while the assembly runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsmOperandKind {
    /// A register of the class; the operand's value is a scalar.
    Register(AsmRegisterClass),
    /// The object an lvalue designates, addressed in memory.
    Memory,
    /// A constant: an integer constant expression, or with `symbolic` also
    /// an address constant (`i`, `g` and `X` allow both, `n` and the
    /// ranged x86 letters only integers).
    Immediate { symbolic: bool },
    /// An input in the same location as output operand `N` (a digit or the
    /// output's `[name]`).
    Tied(u32),
    /// An `int` output that is 1 when the x86 condition holds after the
    /// assembly, else 0 (`=@ccCOND`).
    Flag(AsmCondition),
}

/// An x86-64 register class an operand may use.
/// GNU extension: GCC manual, "Machine Constraints", x86 family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsmRegisterClass {
    /// General registers, a mask by hardware encoding (`%rax` is bit 0):
    /// all but `%rsp` for `r`, `q`, `l`, `g`; `a` `b` `c` `d` `S` `D` one
    /// register each; `Q` `%rax`-`%rbx`; `R` the eight legacy registers.
    General(u16),
    /// `x` and `v`: an SSE register.
    Sse,
    /// `A`, `f`, `t`, `u`, `y` and `Y…`: register pairs, the x87 stack and
    /// MMX, which analysis accepts and lowering refuses.
    Other(u8),
}

impl AsmRegisterClass {
    /// Every general register but `%rsp`, which the stack owns.
    pub(crate) const ANY_GENERAL: u16 = !(1 << 4);
    /// `%rax`, `%rcx`, `%rdx` and `%rbx`, whose second byte has a name.
    pub(crate) const HIGH_BYTE: u16 = 0b1111;
    /// The eight registers that predate the REX prefix.
    pub(crate) const LEGACY: u16 = 0xFF;
}

/// The x86 condition of a flag output: an index into
/// [`AsmCondition::NAMES`].
/// GNU extension: GCC manual, "Flag Output Operands", x86 family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AsmCondition(pub(crate) u8);

impl AsmCondition {
    /// The condition names of `=@cc` flag outputs.
    pub(crate) const NAMES: [&'static str; 30] = [
        "a", "ae", "b", "be", "c", "e", "g", "ge", "l", "le", "na", "nae", "nb", "nbe", "nc", "ne",
        "ng", "nge", "nl", "nle", "no", "np", "ns", "nz", "o", "p", "pe", "po", "s", "z",
    ];

    /// The condition named `name`.
    pub(crate) fn named(name: &str) -> Option<Self> {
        let index = Self::NAMES.iter().position(|&known| known == name)?;
        u8::try_from(index).ok().map(Self)
    }

    pub(crate) const fn name(self) -> &'static str {
        Self::NAMES[self.0 as usize]
    }
}

/// Registers and state the assembly overwrites besides its outputs.
/// GNU extension: GCC manual, "Clobbers and Scratch Registers".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AsmClobbers {
    /// `memory`: the assembly may read and write any memory.
    pub(crate) memory:  bool,
    /// `cc`: the flags.
    pub(crate) flags:   bool,
    /// General registers by hardware encoding.
    pub(crate) general: u16,
    /// `%xmm0`-`%xmm31`, also named by their `%ymm` and `%zmm` extensions.
    pub(crate) sse:     u32,
    /// The x87 stack registers.
    pub(crate) x87:     bool,
    /// `%mm0`-`%mm7`.
    pub(crate) mmx:     u8,
    /// `%k0`-`%k7`.
    pub(crate) mask:    u8,
}
