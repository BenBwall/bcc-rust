//! The machine IR: x86-64 instructions over virtual and physical registers.
//!
//! Instruction selection produces an [`MFunction`] whose instructions name
//! [`VReg`]s, and physical registers only where the instruction set or the
//! calling convention fixes one (argument registers, `%rax`/`%rdx` around a
//! division, `%cl` for a shift count). Register allocation assigns each
//! virtual register a physical register or a spill slot and rewrites the
//! function in place. Memory operands name frame objects symbolically until
//! the frame is laid out.
//!
//! [`MInst::visit_registers`] is the allocator's only view of the
//! instruction set: it enumerates every register an instruction reads or
//! writes, including the implicit ones.

use std::fmt;

use crate::{
    ir::{
        Entity,
        Symbol,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A function in machine IR: blocks in layout order, the number of virtual
/// registers, allocation hints, and the frame objects selection created.
pub(crate) struct MFunction<'a> {
    pub(crate) blocks:   ArenaVec<'a, MBlockData<'a>>,
    /// One hint per virtual register: the register a copy connects it to.
    pub(crate) hints:    ArenaVec<'a, Option<Reg>>,
    /// IR stack slots first, by their index, then back-end temporaries.
    pub(crate) slots:    ArenaVec<'a, FrameSlotData>,
    /// Bytes of the outgoing argument area, including Win64's home area.
    pub(crate) outgoing: u32,
    /// Whether the function calls anything, which fixes its stack alignment.
    pub(crate) calls:    bool,
}

impl<'a> MFunction<'a> {
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            blocks:   ArenaVec::new_in(arena),
            hints:    ArenaVec::new_in(arena),
            slots:    ArenaVec::new_in(arena),
            outgoing: 0,
            calls:    false,
        }
    }

    /// A fresh virtual register.
    pub(crate) fn vreg(&mut self) -> VReg {
        let vreg = VReg::new(self.hints.len());
        self.hints.push(None);
        vreg
    }

    pub(crate) fn vreg_count(&self) -> usize {
        self.hints.len()
    }

    /// A new, empty block at the end of the layout.
    pub(crate) fn block(&mut self, arena: &'a Bump) -> MBlock {
        let block = MBlock::new(self.blocks.len());
        self.blocks.push(MBlockData {
            insts: ArenaVec::new_in(arena),
        });
        block
    }

    /// Records that `vreg` would like the register `other` has.
    pub(crate) fn hint(&mut self, vreg: VReg, other: Reg) {
        let hint = &mut self.hints[vreg.index()];
        if hint.is_none() {
            *hint = Some(other);
        }
    }
}

/// A block's instructions; the last is a terminator.
pub(crate) struct MBlockData<'a> {
    pub(crate) insts: ArenaVec<'a, MInst>,
}

/// A frame object's size and alignment in bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FrameSlotData {
    pub(crate) size:  u32,
    pub(crate) align: u32,
}

/// A virtual register: a dense index per function.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct VReg(u32);

/// A machine block: a dense index per function, in layout order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct MBlock(u32);

macro_rules! dense {
    ($($name:ident => $prefix:literal;)+) => {$(
        impl Entity for $name {
            fn new(index: usize) -> Self {
                Self(u32::try_from(index).expect(concat!("too many ", $prefix, " entities")))
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    )+};
}

dense! {
    VReg => "%v";
    MBlock => "bb";
}

/// A general-purpose register, numbered by its hardware encoding.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum PReg {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl PReg {
    /// Every register, by encoding.
    pub(crate) const ALL: [Self; 16] = [
        Self::Rax,
        Self::Rcx,
        Self::Rdx,
        Self::Rbx,
        Self::Rsp,
        Self::Rbp,
        Self::Rsi,
        Self::Rdi,
        Self::R8,
        Self::R9,
        Self::R10,
        Self::R11,
        Self::R12,
        Self::R13,
        Self::R14,
        Self::R15,
    ];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The AT&T name of the register's low `width` bits, without `%`.
    pub(crate) const fn name(self, width: Width) -> &'static str {
        const NAMES: [[&str; 4]; 16] = [
            ["al", "ax", "eax", "rax"],
            ["cl", "cx", "ecx", "rcx"],
            ["dl", "dx", "edx", "rdx"],
            ["bl", "bx", "ebx", "rbx"],
            ["spl", "sp", "esp", "rsp"],
            ["bpl", "bp", "ebp", "rbp"],
            ["sil", "si", "esi", "rsi"],
            ["dil", "di", "edi", "rdi"],
            ["r8b", "r8w", "r8d", "r8"],
            ["r9b", "r9w", "r9d", "r9"],
            ["r10b", "r10w", "r10d", "r10"],
            ["r11b", "r11w", "r11d", "r11"],
            ["r12b", "r12w", "r12d", "r12"],
            ["r13b", "r13w", "r13d", "r13"],
            ["r14b", "r14w", "r14d", "r14"],
            ["r15b", "r15w", "r15d", "r15"],
        ];
        NAMES[self as usize][width as usize]
    }
}

/// A set of physical registers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct PRegSet(u16);

impl PRegSet {
    pub(crate) const EMPTY: Self = Self(0);

    pub(crate) const fn of(registers: &[PReg]) -> Self {
        let mut bits = 0;
        let mut index = 0;
        while index < registers.len() {
            bits |= 1 << registers[index] as u16;
            index += 1;
        }
        Self(bits)
    }

    #[must_use]
    pub(crate) const fn with(self, register: PReg) -> Self {
        Self(self.0 | 1 << register as u16)
    }

    pub(crate) const fn contains(self, register: PReg) -> bool {
        self.0 & (1 << register as u16) != 0
    }

    /// The registers in encoding order.
    pub(crate) fn iter(self) -> impl Iterator<Item = PReg> {
        PReg::ALL
            .into_iter()
            .filter(move |&register| self.contains(register))
    }
}

impl fmt::Debug for PRegSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// A register operand: physical (precolored) or virtual.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[expect(
    variant_size_differences,
    reason = "Both variants fit eight bytes; packing them into one word would obscure every               match on them."
)]
pub(crate) enum Reg {
    Phys(PReg),
    Virt(VReg),
}

impl Reg {
    /// The physical register; allocation has replaced every virtual one.
    pub(crate) fn phys(self) -> PReg {
        match self {
            | Self::Phys(register) => register,
            | Self::Virt(vreg) => unreachable!("{vreg:?} was not allocated"),
        }
    }
}

/// An operand width: the instruction suffix and the sub-register.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum Width {
    B8,
    B16,
    B32,
    B64,
}

impl Width {
    pub(crate) const fn suffix(self) -> char {
        match self {
            | Self::B8 => 'b',
            | Self::B16 => 'w',
            | Self::B32 => 'l',
            | Self::B64 => 'q',
        }
    }

    pub(crate) const fn bits(self) -> u32 {
        match self {
            | Self::B8 => 8,
            | Self::B16 => 16,
            | Self::B32 => 32,
            | Self::B64 => 64,
        }
    }
}

/// A memory operand, `disp(base)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mem {
    pub(crate) base: Base,
    pub(crate) disp: i32,
}

impl Mem {
    pub(crate) const fn new(base: Base, disp: i32) -> Self {
        Self { base, disp }
    }
}

/// What a memory operand is relative to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Base {
    Reg(Reg),
    /// A frame object, resolved to an `%rsp` offset after frame layout.
    Frame(FrameObject),
    /// `sym+disp(%rip)`.
    Symbol(SymbolRef),
    /// `sym@GOTPCREL(%rip)`: the GOT entry holding the symbol's address
    /// (ELF only, for symbols the module does not define).
    Got(SymbolRef),
}

/// A frame object, before frame layout gives it an offset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FrameObject {
    /// An entry of [`MFunction::slots`].
    Slot(u32),
    /// The incoming stack arguments, by byte offset from the first one.
    Incoming(u32),
    /// The outgoing argument area at the bottom of the frame.
    Outgoing(u32),
    /// A spill slot the allocator assigned.
    Spill(u32),
}

/// A symbol an instruction refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SymbolRef {
    /// A function or global of the module.
    Ir(Symbol),
    /// A runtime function the back end calls on its own (`memcpy`,
    /// `__main`).
    Runtime(&'static str),
}

/// A source operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Operand {
    Reg(Reg),
    /// A 32-bit immediate, sign-extended to 64-bit operations.
    Imm(i32),
    Mem(Mem),
}

/// A condition code, as `jcc`, `setcc` and `cmovcc` spell it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Cond {
    E,
    Ne,
    L,
    Le,
    G,
    Ge,
    B,
    Be,
    A,
    Ae,
    O,
    No,
}

impl Cond {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::E => "e",
            | Self::Ne => "ne",
            | Self::L => "l",
            | Self::Le => "le",
            | Self::G => "g",
            | Self::Ge => "ge",
            | Self::B => "b",
            | Self::Be => "be",
            | Self::A => "a",
            | Self::Ae => "ae",
            | Self::O => "o",
            | Self::No => "no",
        }
    }

    /// The condition that holds exactly when this one does not.
    pub(crate) const fn negated(self) -> Self {
        match self {
            | Self::E => Self::Ne,
            | Self::Ne => Self::E,
            | Self::L => Self::Ge,
            | Self::Le => Self::G,
            | Self::G => Self::Le,
            | Self::Ge => Self::L,
            | Self::B => Self::Ae,
            | Self::Be => Self::A,
            | Self::A => Self::Be,
            | Self::Ae => Self::B,
            | Self::O => Self::No,
            | Self::No => Self::O,
        }
    }

    /// The condition that holds with the comparison's operands swapped.
    pub(crate) const fn swapped(self) -> Self {
        match self {
            | Self::L => Self::G,
            | Self::Le => Self::Ge,
            | Self::G => Self::L,
            | Self::Ge => Self::Le,
            | Self::B => Self::A,
            | Self::Be => Self::Ae,
            | Self::A => Self::B,
            | Self::Ae => Self::Be,
            | other => other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    /// Two-operand `imul` (16, 32 or 64 bits).
    Imul,
}

impl AluOp {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Add => "add",
            | Self::Sub => "sub",
            | Self::And => "and",
            | Self::Or => "or",
            | Self::Xor => "xor",
            | Self::Imul => "imul",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShiftOp {
    Shl,
    Shr,
    Sar,
    Rol,
}

impl ShiftOp {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Shl => "shl",
            | Self::Shr => "shr",
            | Self::Sar => "sar",
            | Self::Rol => "rol",
        }
    }
}

/// A shift count: an immediate or `%cl`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShiftAmount {
    Imm(u8),
    Cl,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UnaryOp {
    Neg,
    Bswap,
}

impl UnaryOp {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Neg => "neg",
            | Self::Bswap => "bswap",
        }
    }
}

/// What a call calls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Callee {
    Symbol(SymbolRef),
    Reg(Reg),
}

/// How an instruction accesses a register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    Use,
    Def,
    UseDef,
}

impl Role {
    pub(crate) const fn reads(self) -> bool {
        matches!(self, Self::Use | Self::UseDef)
    }

    pub(crate) const fn writes(self) -> bool {
        matches!(self, Self::Def | Self::UseDef)
    }
}

/// One machine instruction. The two-address forms (`Alu`, `Shift`,
/// `Unary`, `Cmov`) read and write `dst`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MInst {
    /// `dst ← src`. A 64-bit move between registers is a copy, which the
    /// allocator deletes when both sides get the same register; a 32-bit
    /// one also clears the upper half, so it never is.
    Mov {
        width: Width,
        dst:   Reg,
        src:   Operand,
    },
    /// `movabsq $imm, dst`.
    MovImm64 {
        dst: Reg,
        imm: i64,
    },
    Store {
        width: Width,
        dst:   Mem,
        src:   Operand,
    },
    /// `leaq addr, dst`.
    Lea {
        dst:  Reg,
        addr: Mem,
    },
    /// `movzx`/`movsx`, or `movl` for a zero extension from 32 bits.
    Extend {
        signed: bool,
        from:   Width,
        to:     Width,
        dst:    Reg,
        src:    Operand,
    },
    Alu {
        op:    AluOp,
        width: Width,
        dst:   Reg,
        src:   Operand,
    },
    /// `imul $imm, src, dst`.
    Imul3 {
        width: Width,
        dst:   Reg,
        src:   Reg,
        imm:   i32,
    },
    /// Sets the flags from `lhs - rhs`.
    Cmp {
        width: Width,
        lhs:   Reg,
        rhs:   Operand,
    },
    /// Sets the flags from `lhs & rhs`.
    Test {
        width: Width,
        lhs:   Reg,
        rhs:   Reg,
    },
    Shift {
        op:     ShiftOp,
        width:  Width,
        dst:    Reg,
        amount: ShiftAmount,
    },
    Unary {
        op:    UnaryOp,
        width: Width,
        dst:   Reg,
    },
    /// `bsr` (reverse) or `bsf`; ZF is set when `src` is zero.
    BitScan {
        reverse: bool,
        width:   Width,
        dst:     Reg,
        src:     Reg,
    },
    /// `cltd`/`cqto`: `%rdx` takes the sign of `%rax`.
    SignExtendAx {
        width: Width,
    },
    /// `div`/`idiv`: `%rdx:%rax` divided by `divisor`.
    Div {
        signed:  bool,
        width:   Width,
        divisor: Reg,
    },
    /// `mul`: `%rdx:%rax ← %rax * src`, OF and CF set on a high half.
    MulWide {
        width: Width,
        src:   Reg,
    },
    /// `setcc` of the low byte of `dst`.
    Setcc {
        cond: Cond,
        dst:  Reg,
    },
    Cmov {
        cond:  Cond,
        width: Width,
        dst:   Reg,
        src:   Reg,
    },
    Call {
        callee:   Callee,
        /// The argument registers the call reads.
        uses:     PRegSet,
        /// The registers the callee may change.
        clobbers: PRegSet,
    },
    Jmp {
        target: MBlock,
    },
    Jcc {
        cond:      Cond,
        taken:     MBlock,
        not_taken: MBlock,
    },
    /// `rep movsb`: copies `%rcx` bytes from `(%rsi)` to `(%rdi)`, forwards.
    RepMovsb,
    /// `rep stosb`: stores `%al` into `%rcx` bytes from `(%rdi)`.
    RepStosb,
    /// Returns, after the epilogue; reads the result registers.
    Ret {
        uses: PRegSet,
    },
    Ud2,
}

impl MInst {
    /// Calls `visit` with every register the instruction reads or writes,
    /// explicit and implicit. Registers in memory operands are reads.
    pub(crate) fn visit_registers(&mut self, mut visit: impl FnMut(&mut Reg, Role)) {
        let phys = |register: PReg, role: Role, visit: &mut dyn FnMut(&mut Reg, Role)| {
            let mut reg = Reg::Phys(register);
            visit(&mut reg, role);
        };
        match self {
            | Self::Mov { dst, src, .. } | Self::Extend { dst, src, .. } => {
                operand(src, &mut visit);
                visit(dst, Role::Def);
            },
            | Self::MovImm64 { dst, .. } | Self::Setcc { dst, .. } => visit(dst, Role::Def),
            | Self::Store { dst, src, .. } => {
                memory(dst, &mut visit);
                operand(src, &mut visit);
            },
            | Self::Lea { dst, addr } => {
                memory(addr, &mut visit);
                visit(dst, Role::Def);
            },
            | Self::Alu { dst, src, .. } => {
                operand(src, &mut visit);
                visit(dst, Role::UseDef);
            },
            | Self::Imul3 { dst, src, .. } | Self::BitScan { dst, src, .. } => {
                visit(src, Role::Use);
                visit(dst, Role::Def);
            },
            | Self::Cmp { lhs, rhs, .. } => {
                visit(lhs, Role::Use);
                operand(rhs, &mut visit);
            },
            | Self::Test { lhs, rhs, .. } => {
                visit(lhs, Role::Use);
                visit(rhs, Role::Use);
            },
            | Self::Shift { dst, amount, .. } => {
                if *amount == ShiftAmount::Cl {
                    phys(PReg::Rcx, Role::Use, &mut visit);
                }
                visit(dst, Role::UseDef);
            },
            | Self::Unary { dst, .. } => visit(dst, Role::UseDef),
            | Self::SignExtendAx { .. } => {
                phys(PReg::Rax, Role::Use, &mut visit);
                phys(PReg::Rdx, Role::Def, &mut visit);
            },
            | Self::Div { divisor, .. } => {
                visit(divisor, Role::Use);
                phys(PReg::Rax, Role::UseDef, &mut visit);
                phys(PReg::Rdx, Role::UseDef, &mut visit);
            },
            | Self::MulWide { src, .. } => {
                visit(src, Role::Use);
                phys(PReg::Rax, Role::UseDef, &mut visit);
                phys(PReg::Rdx, Role::Def, &mut visit);
            },
            | Self::Cmov { dst, src, .. } => {
                visit(src, Role::Use);
                visit(dst, Role::UseDef);
            },
            | Self::Call {
                callee,
                uses,
                clobbers,
            } => {
                if let Callee::Reg(reg) = callee {
                    visit(reg, Role::Use);
                }
                for register in uses.iter() {
                    phys(register, Role::Use, &mut visit);
                }
                for register in clobbers.iter() {
                    phys(register, Role::Def, &mut visit);
                }
            },
            | Self::RepMovsb => {
                phys(PReg::Rdi, Role::UseDef, &mut visit);
                phys(PReg::Rsi, Role::UseDef, &mut visit);
                phys(PReg::Rcx, Role::UseDef, &mut visit);
            },
            | Self::RepStosb => {
                phys(PReg::Rdi, Role::UseDef, &mut visit);
                phys(PReg::Rcx, Role::UseDef, &mut visit);
                phys(PReg::Rax, Role::Use, &mut visit);
            },
            | Self::Ret { uses } =>
                for register in uses.iter() {
                    phys(register, Role::Use, &mut visit);
                },
            | Self::Jmp { .. } | Self::Jcc { .. } | Self::Ud2 => {},
        }
    }

    /// The blocks a terminator may transfer control to.
    pub(crate) fn successors(&self) -> impl Iterator<Item = MBlock> {
        let (first, second) = match *self {
            | Self::Jmp { target } => (Some(target), None),
            | Self::Jcc {
                taken, not_taken, ..
            } => (Some(taken), Some(not_taken)),
            | _ => (None, None),
        };
        first.into_iter().chain(second)
    }

    /// Whether removing the instruction changes nothing but the virtual
    /// register it defines: no memory access (a load may be volatile), no
    /// control transfer, no physical register written, and no flags a later
    /// instruction reads (the selector reads flags only right after `Cmp`,
    /// `Test`, `Alu`, `BitScan` and `MulWide`, none of which is pure).
    pub(crate) fn is_pure(&self) -> bool {
        match self {
            | Self::Mov { dst, src, .. } | Self::Extend { dst, src, .. } =>
                matches!(dst, Reg::Virt(_)) && !matches!(src, Operand::Mem(_)),
            | Self::MovImm64 { dst, .. } | Self::Lea { dst, .. } | Self::Imul3 { dst, .. } =>
                matches!(dst, Reg::Virt(_)),
            | _ => false,
        }
    }
}

/// Visits the registers of a source operand, which are all read.
fn operand(operand: &mut Operand, visit: &mut impl FnMut(&mut Reg, Role)) {
    match operand {
        | Operand::Reg(reg) => visit(reg, Role::Use),
        | Operand::Mem(mem) => memory(mem, visit),
        | Operand::Imm(_) => {},
    }
}

/// Visits the base register of a memory operand, if it has one.
fn memory(mem: &mut Mem, visit: &mut impl FnMut(&mut Reg, Role)) {
    if let Base::Reg(reg) = &mut mem.base {
        visit(reg, Role::Use);
    }
}
