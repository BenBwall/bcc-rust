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
//!
//! Registers come in two classes: general-purpose registers for integers
//! and pointers, and SSE registers (`%xmm0`-`%xmm15`) for `f32` and `f64`.
//! Each virtual register has one class, recorded in [`MFunction::classes`].
//! `i128`, `f80` and `f128` values live in pairs of general registers; the
//! x87 instructions that compute on `f80` read and write memory only.

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
    pub(crate) blocks:           ArenaVec<'a, MBlockData<'a>>,
    /// One hint per virtual register: the register a copy connects it to.
    pub(crate) hints:            ArenaVec<'a, Option<Reg>>,
    /// The class of each virtual register.
    pub(crate) classes:          ArenaVec<'a, RegClass>,
    /// IR stack slots first, by their index, then back-end temporaries.
    pub(crate) slots:            ArenaVec<'a, FrameSlotData>,
    /// The function's read-only constants, 16 bytes each, addressed through
    /// [`Base::Constant`].
    pub(crate) constants:        ArenaVec<'a, u128>,
    /// Bytes of the outgoing argument area, including Win64's home area.
    pub(crate) outgoing:         u32,
    /// The largest [`FrameObject::Incoming`] offset the function names:
    /// where its last stack parameter, or its variadic arguments, start
    /// beyond Win64's home area.
    pub(crate) incoming:         u32,
    /// Whether the function calls anything, which fixes its stack alignment.
    pub(crate) calls:            bool,
    /// Whether `%rsp` moves after the prologue (`dyn_alloca`,
    /// `stack_restore`), so frame objects are addressed from `%rbp`.
    pub(crate) dynamic:          bool,
    /// Whether the frame holds a System V register save area for
    /// `va_start`.
    pub(crate) register_save:    bool,
    /// The largest alignment an IR stack slot asks for; a slot aligned
    /// above 16 bytes rounds its address up from `%rsp` at run time.
    pub(crate) slot_align:       u64,
    /// A callee-saved register the prologue pushes first, written or not:
    /// `%rbp` when the function takes its frame address, so the caller's
    /// `%rbp` is where a frame pointer would save it.
    pub(crate) first_push:       Option<PReg>,
    /// The targets of each [`MInst::JmpIndirect`], by its `dests` index.
    pub(crate) indirect:         ArenaVec<'a, ArenaVec<'a, MBlock>>,
    /// Each IR block label with the block its address names: the labeled
    /// block, or a landing block that assigns its parameters first.
    pub(crate) labels:           ArenaVec<'a, (u32, MBlock)>,
    /// The IR symbols the function reaches through their GOT entry or
    /// `.refptr` stub, possibly repeated.
    pub(crate) indirect_symbols: ArenaVec<'a, Symbol>,
}

impl<'a> MFunction<'a> {
    pub(crate) fn new_in(arena: &'a Bump) -> Self {
        Self {
            blocks:           ArenaVec::new_in(arena),
            hints:            ArenaVec::new_in(arena),
            classes:          ArenaVec::new_in(arena),
            slots:            ArenaVec::new_in(arena),
            constants:        ArenaVec::new_in(arena),
            outgoing:         0,
            incoming:         0,
            calls:            false,
            dynamic:          false,
            register_save:    false,
            slot_align:       1,
            first_push:       None,
            indirect:         ArenaVec::new_in(arena),
            labels:           ArenaVec::new_in(arena),
            indirect_symbols: ArenaVec::new_in(arena),
        }
    }

    /// The blocks the terminator of `block` may transfer control to.
    pub(crate) fn successors(&self, block: MBlock) -> impl Iterator<Item = MBlock> {
        let last = self.blocks[block.index()].insts.last();
        let indirect = match last {
            | Some(&MInst::JmpIndirect { dests, .. }) => &self.indirect[dests as usize][..],
            | _ => &[],
        };
        last.into_iter()
            .flat_map(MInst::successors)
            .chain(indirect.iter().copied())
    }

    /// A fresh general-purpose virtual register.
    pub(crate) fn vreg(&mut self) -> VReg {
        self.vreg_of(RegClass::Gpr)
    }

    /// A fresh virtual register of `class`.
    pub(crate) fn vreg_of(&mut self, class: RegClass) -> VReg {
        let vreg = VReg::new(self.hints.len());
        self.hints.push(None);
        self.classes.push(class);
        vreg
    }

    /// The class of a register operand.
    pub(crate) fn class(&self, reg: Reg) -> RegClass {
        match reg {
            | Reg::Phys(register) => register.class(),
            | Reg::Virt(vreg) => self.classes[vreg.index()],
        }
    }

    /// The index of a 16-byte read-only constant, added once.
    pub(crate) fn constant(&mut self, bits: u128) -> u32 {
        let index = match self.constants.iter().position(|&constant| constant == bits) {
            | Some(index) => index,
            | None => {
                self.constants.push(bits);
                self.constants.len() - 1
            },
        };
        u32::try_from(index).expect("constant indices fit u32")
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

/// A register class.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum RegClass {
    /// The sixteen general-purpose registers.
    Gpr,
    /// The sixteen SSE registers, holding `f32` and `f64` values.
    Xmm,
}

/// A physical register: the general-purpose registers by their hardware
/// encoding, then the SSE registers.
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
    Xmm0,
    Xmm1,
    Xmm2,
    Xmm3,
    Xmm4,
    Xmm5,
    Xmm6,
    Xmm7,
    Xmm8,
    Xmm9,
    Xmm10,
    Xmm11,
    Xmm12,
    Xmm13,
    Xmm14,
    Xmm15,
}

impl PReg {
    /// Every register, general-purpose first.
    pub(crate) const ALL: [Self; Self::COUNT] = [
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
        Self::Xmm0,
        Self::Xmm1,
        Self::Xmm2,
        Self::Xmm3,
        Self::Xmm4,
        Self::Xmm5,
        Self::Xmm6,
        Self::Xmm7,
        Self::Xmm8,
        Self::Xmm9,
        Self::Xmm10,
        Self::Xmm11,
        Self::Xmm12,
        Self::Xmm13,
        Self::Xmm14,
        Self::Xmm15,
    ];
    /// The number of physical registers.
    pub(crate) const COUNT: usize = 32;

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    pub(crate) const fn class(self) -> RegClass {
        if (self as usize) < 16 {
            RegClass::Gpr
        } else {
            RegClass::Xmm
        }
    }

    /// The AT&T name of the register's low `width` bits, without `%`; an
    /// SSE register has one name.
    pub(crate) const fn name(self, width: Width) -> &'static str {
        const XMM: [&str; 16] = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7", "xmm8", "xmm9",
            "xmm10", "xmm11", "xmm12", "xmm13", "xmm14", "xmm15",
        ];
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
        if (self as usize) >= 16 {
            return XMM[self as usize - 16];
        }
        NAMES[self as usize][width as usize]
    }
}

/// A set of physical registers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct PRegSet(u32);

impl PRegSet {
    pub(crate) const EMPTY: Self = Self(0);

    pub(crate) const fn of(registers: &[PReg]) -> Self {
        let mut bits = 0;
        let mut index = 0;
        while index < registers.len() {
            bits |= 1 << registers[index] as u32;
            index += 1;
        }
        Self(bits)
    }

    #[must_use]
    pub(crate) const fn with(self, register: PReg) -> Self {
        Self(self.0 | 1 << register as u32)
    }

    pub(crate) const fn contains(self, register: PReg) -> bool {
        self.0 & (1 << register as u32) != 0
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[expect(
    variant_size_differences,
    reason = "Both variants fit eight bytes; packing them into one word would obscure every match \
              on them."
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

/// A memory operand, `disp(base)` or `disp(base, index, scale)`. Only a
/// register or frame base takes an index; RIP-relative operands cannot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mem {
    pub(crate) base:  Base,
    pub(crate) index: Option<Index>,
    pub(crate) disp:  i32,
}

impl Mem {
    pub(crate) const fn new(base: Base, disp: i32) -> Self {
        Self {
            base,
            index: None,
            disp,
        }
    }

    /// The same operand `disp` bytes further on.
    #[must_use]
    pub(crate) const fn offset(self, disp: i32) -> Self {
        Self {
            disp: self.disp + disp,
            ..self
        }
    }
}

/// The scaled index register of a memory operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Index {
    pub(crate) reg:   Reg,
    /// 1, 2, 4 or 8.
    pub(crate) scale: u8,
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
    /// An entry of [`MFunction::constants`], `.LCPI…(%rip)`.
    Constant(u32),
    /// The address of the function's IR block label with this index,
    /// `.LBA…(%rip)`. GNU extension: GCC "Labels as Values".
    Label(u32),
    /// `sym@GOTTPOFF(%rip)`: the GOT entry holding a thread-local symbol's
    /// offset from the thread pointer (ELF initial-exec model).
    GotTpoff(SymbolRef),
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
    /// Win64's home area the caller reserved above the return address, by
    /// byte offset: the slots of the four register arguments.
    Home(u32),
    /// The System V register save area of a variadic function, by byte
    /// offset: six general registers, then eight SSE registers of 16 bytes.
    RegisterSave(u32),
    /// Just above the outgoing argument area at the current `%rsp`: where a
    /// dynamic allocation starts.
    OutgoingEnd,
    /// The return address the call pushed, at the CFA minus 8.
    ReturnAddress,
    /// Where a frame pointer saves the caller's `%rbp`, the CFA minus 16: the
    /// value of `__builtin_frame_address (0)` with a frame pointer.
    FrameAddress,
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
    S,
    Ns,
    P,
    Np,
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
            | Self::S => "s",
            | Self::Ns => "ns",
            | Self::P => "p",
            | Self::Np => "np",
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
            | Self::S => Self::Ns,
            | Self::Ns => Self::S,
            | Self::P => Self::Np,
            | Self::Np => Self::P,
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
    /// Add with carry, right after an `Add` of the low halves.
    Adc,
    /// Subtract with borrow, right after a `Sub` of the low halves.
    Sbb,
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
            | Self::Adc => "adc",
            | Self::Sbb => "sbb",
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

/// A scalar SSE operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum XOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `xorps`, on the whole register.
    Xor,
}

impl XOp {
    /// The mnemonic for a single (`B32`) or double (`B64`) operand.
    pub(crate) const fn name(self, width: Width) -> &'static str {
        let double = matches!(width, Width::B64);
        match self {
            | Self::Add if double => "addsd",
            | Self::Add => "addss",
            | Self::Sub if double => "subsd",
            | Self::Sub => "subss",
            | Self::Mul if double => "mulsd",
            | Self::Mul => "mulss",
            | Self::Div if double => "divsd",
            | Self::Div => "divss",
            | Self::Xor => "xorps",
        }
    }
}

/// The memory format of an x87 load or store.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum X87Format {
    F32,
    F64,
    /// The 80-bit extended format, ten bytes.
    F80,
    I16,
    I32,
    I64,
}

impl X87Format {
    /// The mnemonic that pushes a value of this format.
    pub(crate) const fn load(self) -> &'static str {
        match self {
            | Self::F32 => "flds",
            | Self::F64 => "fldl",
            | Self::F80 => "fldt",
            | Self::I16 => "filds",
            | Self::I32 => "fildl",
            | Self::I64 => "fildll",
        }
    }

    /// The mnemonic that stores `%st(0)` in this format and pops it.
    pub(crate) const fn store(self) -> &'static str {
        match self {
            | Self::F32 => "fstps",
            | Self::F64 => "fstpl",
            | Self::F80 => "fstpt",
            | Self::I16 => "fistps",
            | Self::I32 => "fistpl",
            | Self::I64 => "fistpll",
        }
    }
}

/// An x87 arithmetic operation, `%st(0) ← %st(0) op %st(1)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum X87Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl X87Op {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Add => "fadd",
            | Self::Sub => "fsub",
            | Self::Mul => "fmul",
            | Self::Div => "fdiv",
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
    /// `jmpq *target`; the possible targets are [`MFunction::indirect`]
    /// entry `dests`.
    JmpIndirect {
        target: Reg,
        dests:  u32,
    },
    /// `dst ←` the thread pointer: `%fs:0` on ELF, the TEB's
    /// `ThreadLocalStoragePointer` (`%gs:0x58`) on Windows.
    ThreadPointer {
        dst: Reg,
    },
    /// `leaq sym@TPOFF(base), dst` on ELF (local-exec) or
    /// `leaq sym@SECREL32(base), dst` on Windows: a thread-local symbol's
    /// address from its thread's block.
    TlsLea {
        dst:    Reg,
        base:   Reg,
        symbol: SymbolRef,
    },
    /// An SSE move: a register copy (`movaps`, deleted between equal
    /// registers when `width` is `B64`) or a load (`movss`/`movsd`).
    XMov {
        width: Width,
        dst:   Reg,
        src:   Operand,
    },
    /// `movups`: all 16 bytes of an SSE register loaded from or stored to
    /// memory: Win64 `__int128` results and System V `__float128` values in
    /// whole registers, and the System V register save area.
    Move128 {
        load: bool,
        reg:  Reg,
        mem:  Mem,
    },
    /// `movss`/`movsd` of an SSE register to memory.
    XStore {
        width: Width,
        dst:   Mem,
        src:   Reg,
    },
    /// `xorps dst, dst`: positive zero.
    XZero {
        dst: Reg,
    },
    /// A scalar SSE operation in two-address form.
    XAlu {
        op:    XOp,
        width: Width,
        dst:   Reg,
        src:   Operand,
    },
    /// `ucomiss`/`ucomisd`: ZF, PF and CF from comparing `lhs` with `rhs`;
    /// an unordered pair sets all three.
    Ucomi {
        width: Width,
        lhs:   Reg,
        rhs:   Reg,
    },
    /// `cvtsi2ss`/`cvtsi2sd` of a 32- or 64-bit signed integer.
    IntToFloat {
        int:   Width,
        float: Width,
        dst:   Reg,
        src:   Reg,
    },
    /// `cvttss2si`/`cvttsd2si`: truncation to a 32- or 64-bit integer.
    FloatToInt {
        float: Width,
        int:   Width,
        dst:   Reg,
        src:   Reg,
    },
    /// `cvtss2sd` (from `B32`) or `cvtsd2ss` (from `B64`).
    FloatToFloat {
        from: Width,
        dst:  Reg,
        src:  Reg,
    },
    /// `movd`/`movq` between a general and an SSE register.
    MovGx {
        width: Width,
        dst:   Reg,
        src:   Reg,
    },
    /// `shld`/`shrd` of 64-bit registers: `dst` shifted, filled from `src`.
    ShiftDouble {
        left:   bool,
        dst:    Reg,
        src:    Reg,
        amount: ShiftAmount,
    },
    /// `rep movsb`: copies `%rcx` bytes from `(%rsi)` to `(%rdi)`, forwards.
    RepMovsb,
    /// `rep stosb`: stores `%al` into `%rcx` bytes from `(%rdi)`.
    RepStosb,
    /// Pushes a value from memory onto the x87 stack (`fld`, `fild`).
    /// The x87 stack is not allocated: every sequence that pushes pops
    /// everything again before anything else runs, except the `%st(0)`
    /// result that a `Ret` returns.
    X87Load {
        format: X87Format,
        mem:    Mem,
    },
    /// Stores `%st(0)` to memory and pops it (`fstp`, `fistp`, which
    /// rounds by the control word's rounding mode).
    X87Store {
        format: X87Format,
        mem:    Mem,
    },
    /// `%st(0) ← %st(0) op %st(1)`.
    X87Arith {
        op: X87Op,
    },
    /// `fstp %st(0)`: pops the x87 stack.
    X87Pop,
    /// `fucomip %st(1), %st`: ZF, PF and CF from comparing `%st(0)` with
    /// `%st(1)`, as `ucomis*` sets them, then pops `%st(0)`.
    X87Compare,
    /// `fnstcw` (`store`) or `fldcw` of the x87 control word.
    X87ControlWord {
        store: bool,
        mem:   Mem,
    },
    /// Returns, after the epilogue; reads the result registers.
    Ret {
        uses: PRegSet,
    },
    Ud2,
    /// A breakpoint trap (`__debugbreak`).
    Int3,
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
            | Self::Mov { dst, src, .. }
            | Self::Extend { dst, src, .. }
            | Self::XMov { dst, src, .. } => {
                operand(src, &mut visit);
                visit(dst, Role::Def);
            },
            | Self::MovImm64 { dst, .. }
            | Self::Setcc { dst, .. }
            | Self::XZero { dst }
            | Self::ThreadPointer { dst } => visit(dst, Role::Def),
            | Self::TlsLea { dst, base, .. } => {
                visit(base, Role::Use);
                visit(dst, Role::Def);
            },
            | Self::JmpIndirect { target, .. } => visit(target, Role::Use),
            | Self::XStore { dst, src, .. } => {
                memory(dst, &mut visit);
                visit(src, Role::Use);
            },
            | Self::Move128 { load, reg, mem } => {
                memory(mem, &mut visit);
                visit(reg, if *load { Role::Def } else { Role::Use });
            },
            | Self::ShiftDouble {
                dst, src, amount, ..
            } => {
                if *amount == ShiftAmount::Cl {
                    phys(PReg::Rcx, Role::Use, &mut visit);
                }
                visit(src, Role::Use);
                visit(dst, Role::UseDef);
            },
            | Self::Store { dst, src, .. } => {
                memory(dst, &mut visit);
                operand(src, &mut visit);
            },
            | Self::Lea { dst, addr } => {
                memory(addr, &mut visit);
                visit(dst, Role::Def);
            },
            | Self::Alu { dst, src, .. } | Self::XAlu { dst, src, .. } => {
                operand(src, &mut visit);
                visit(dst, Role::UseDef);
            },
            | Self::Imul3 { dst, src, .. }
            | Self::BitScan { dst, src, .. }
            | Self::IntToFloat { dst, src, .. }
            | Self::FloatToInt { dst, src, .. }
            | Self::FloatToFloat { dst, src, .. }
            | Self::MovGx { dst, src, .. } => {
                visit(src, Role::Use);
                visit(dst, Role::Def);
            },
            | Self::Cmp { lhs, rhs, .. } => {
                visit(lhs, Role::Use);
                operand(rhs, &mut visit);
            },
            | Self::Test { lhs, rhs, .. } | Self::Ucomi { lhs, rhs, .. } => {
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
            | Self::X87Load { mem, .. }
            | Self::X87Store { mem, .. }
            | Self::X87ControlWord { mem, .. } => memory(mem, &mut visit),
            | Self::Ret { uses } =>
                for register in uses.iter() {
                    phys(register, Role::Use, &mut visit);
                },
            | Self::X87Arith { .. }
            | Self::X87Pop
            | Self::X87Compare
            | Self::Jmp { .. }
            | Self::Jcc { .. }
            | Self::Ud2
            | Self::Int3 => {},
        }
    }

    /// The blocks a direct branch may transfer control to; an indirect
    /// jump's are in [`MFunction::successors`].
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
            | Self::Mov { dst, src, .. }
            | Self::Extend { dst, src, .. }
            | Self::XMov { dst, src, .. } =>
                matches!(dst, Reg::Virt(_)) && !matches!(src, Operand::Mem(_)),
            | Self::MovImm64 { dst, .. }
            | Self::Lea { dst, .. }
            | Self::Imul3 { dst, .. }
            | Self::XZero { dst }
            | Self::IntToFloat { dst, .. }
            | Self::FloatToInt { dst, .. }
            | Self::FloatToFloat { dst, .. }
            | Self::MovGx { dst, .. } => matches!(dst, Reg::Virt(_)),
            | _ => false,
        }
    }

    /// The source operand the instruction may read from memory instead of
    /// a register: where a spilled value can be used in its slot without a
    /// reload. (`xorps` would need 16 aligned bytes.)
    pub(crate) const fn memory_source(&mut self) -> Option<&mut Operand> {
        match self {
            | Self::Mov { src, .. }
            | Self::Extend { src, .. }
            | Self::XMov { src, .. }
            | Self::Alu { src, .. }
            | Self::Cmp { rhs: src, .. } => Some(src),
            | Self::XAlu { op, src, .. } if !matches!(op, XOp::Xor) => Some(src),
            | _ => None,
        }
    }

    /// The instruction's memory operand, if it has one; no instruction has
    /// two.
    pub(crate) const fn memory_operand(&mut self) -> Option<&mut Mem> {
        match self {
            | Self::Mov {
                src: Operand::Mem(mem),
                ..
            }
            | Self::Extend {
                src: Operand::Mem(mem),
                ..
            }
            | Self::XMov {
                src: Operand::Mem(mem),
                ..
            }
            | Self::Alu {
                src: Operand::Mem(mem),
                ..
            }
            | Self::XAlu {
                src: Operand::Mem(mem),
                ..
            }
            | Self::Cmp {
                rhs: Operand::Mem(mem),
                ..
            }
            | Self::Store { dst: mem, .. }
            | Self::XStore { dst: mem, .. }
            | Self::Lea { addr: mem, .. }
            | Self::Move128 { mem, .. }
            | Self::X87Load { mem, .. }
            | Self::X87Store { mem, .. }
            | Self::X87ControlWord { mem, .. } => Some(mem),
            | _ => None,
        }
    }

    /// The virtual register the memory source reads, when the instruction
    /// names it nowhere else.
    pub(crate) fn memory_source_vreg(&self) -> Option<VReg> {
        let mut inst = *self;
        let Some(&mut Operand::Reg(Reg::Virt(vreg))) = inst.memory_source() else {
            return None;
        };
        let mut count = 0;
        inst.visit_registers(|reg, _| count += usize::from(*reg == Reg::Virt(vreg)));
        (count == 1).then_some(vreg)
    }

    /// Whether removing the instruction changes nothing but the virtual
    /// register it defines and the flags: integer arithmetic on registers
    /// and immediates.
    pub(crate) fn is_pure_but_flags(&self) -> bool {
        match self {
            | Self::Alu { dst, src, .. } =>
                matches!(dst, Reg::Virt(_)) && !matches!(src, Operand::Mem(_)),
            | Self::Shift { dst, .. } | Self::Unary { dst, .. } | Self::BitScan { dst, .. } =>
                matches!(dst, Reg::Virt(_)),
            | _ => false,
        }
    }

    /// Whether the instruction reads the flags, and whether it certainly
    /// overwrites all of them (a shift by `%cl` may leave them alone).
    pub(crate) const fn flags(&self) -> (bool, bool) {
        match self {
            | Self::Jcc { .. } | Self::Setcc { .. } | Self::Cmov { .. } => (true, false),
            | Self::Alu {
                op: AluOp::Adc | AluOp::Sbb,
                ..
            } => (true, true),
            | Self::Alu { .. }
            | Self::Imul3 { .. }
            | Self::Cmp { .. }
            | Self::Test { .. }
            | Self::Ucomi { .. }
            | Self::BitScan { .. }
            | Self::Div { .. }
            | Self::MulWide { .. }
            | Self::Call { .. }
            | Self::X87Compare
            | Self::Unary {
                op: UnaryOp::Neg, ..
            }
            | Self::Shift {
                amount: ShiftAmount::Imm(1..),
                ..
            }
            | Self::ShiftDouble {
                amount: ShiftAmount::Imm(1..),
                ..
            } => (false, true),
            | _ => (false, false),
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

/// Visits the base and index registers of a memory operand.
fn memory(mem: &mut Mem, visit: &mut impl FnMut(&mut Reg, Role)) {
    if let Base::Reg(reg) = &mut mem.base {
        visit(reg, Role::Use);
    }
    if let Some(index) = &mut mem.index {
        visit(&mut index.reg, Role::Use);
    }
}
