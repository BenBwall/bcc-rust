//! The instruction set: opcodes, the fixed-size instruction record, and the
//! flags and operand handles it carries.
//!
//! An [`InstData`] is 16 bytes. Its variants are instruction *formats*, each
//! shared by the opcodes with the same operand shape. Operand lists that do
//! not fit (call arguments, branch arguments and switch cases) live in the
//! function's value pool and are named by a four-byte handle; constants wider
//! than 64 bits live in the function's constant pool.

use std::fmt;

use bitflags::bitflags;

use super::{
    entities::{
        AccessTag,
        AsmId,
        BlockLabel,
        ConstId,
        FuncId,
        GlobalId,
        SigId,
        StackSlot,
        Value,
    },
    types::Type,
};

/// One instruction: an opcode with its controlling type, flags and operands.
///
/// The result value, if any, is recorded beside the instruction in the
/// function body, not in this record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InstData {
    /// Integer and floating arithmetic, unsigned overflow predicates (result
    /// `i1`), `ptr_add` (type `ptr`) and `va_copy` (type `ptr`, no result).
    Binary {
        opcode: Opcode,
        ty:     Type,
        flags:  InstFlags,
        args:   [Value; 2],
    },
    /// Bit counts/swaps, `fneg`, `freeze`, conversions (type = result type),
    /// `va_arg`, and
    /// `va_start` and `va_end` (type `ptr`, no result).
    Unary {
        opcode: Opcode,
        ty:     Type,
        arg:    Value,
    },
    /// Dynamic stack storage: an `i64` byte count, explicit alignment, and
    /// a `ptr` result. Declaration alignment applies equally to VLAs.
    /// C11: §6.7.5p6, p. 127; PDF p. 145.
    DynAlloca {
        ty:    Type,
        size:  Value,
        align: Align,
    },
    /// Fetches an aggregate into fresh storage. `sig` describes one
    /// `ptr aggregate(...)` parameter, retaining its target layout.
    /// C99: §7.15.1.1, pp. 249-250; PDF pp. 261-262.
    VaArgAggregate {
        sig:  SigId,
        list: Value,
    },
    /// `icmp`; the type is the operands' type and the result is `i1`.
    IntCompare {
        cond: IntCC,
        ty:   Type,
        args: [Value; 2],
    },
    /// `fcmp`; the type is the operands' type and the result is `i1`.
    FloatCompare {
        cond: FloatCC,
        ty:   Type,
        args: [Value; 2],
    },
    /// `object_size`: GCC's object-size query of a `ptr`, answered after
    /// inlining ([`ObjectSizeQuery`]). The type is the result's, an integer.
    /// GNU extension: GCC manual, "Object Size Checking".
    ObjectSize {
        ty:    Type,
        query: ObjectSizeQuery,
        ptr:   Value,
    },
    /// `select cond, a, b`.
    Select {
        ty:   Type,
        args: [Value; 3],
    },
    /// `iconst` or `fconst` of at most 64 bits.
    Const {
        opcode: Opcode,
        ty:     Type,
        bits:   Bits64,
    },
    /// `iconst` or `fconst` of `i128`, `f80` or `f128`, whose bits are in the
    /// constant pool.
    WideConst {
        opcode:   Opcode,
        ty:       Type,
        constant: ConstId,
    },
    /// `poison`, `null` (type `ptr`), and `trap` (no result).
    Nullary {
        opcode: Opcode,
        ty:     Type,
    },
    StackAddr {
        slot: StackSlot,
    },
    GlobalAddr {
        global: GlobalId,
    },
    FuncAddr {
        func: FuncId,
    },
    /// `block_addr label`: the address of an address-taken block of this
    /// function, as a `ptr`. GNU extension: GCC "Labels as Values".
    BlockAddr {
        label: BlockLabel,
    },
    /// Ordered atomic access. Compare-exchange returns the observed old value;
    /// weak operations may be strengthened to strong operations. Operands are
    /// address, then value, then expected, as required by `AtomicOp::arity`.
    /// C11: §7.17.3p6, pp. 275-276; PDF pp. 293-294;
    /// §7.17.7.4p2, p. 283; PDF p. 301.
    Atomic {
        op:      AtomicOp,
        ty:      Type,
        flags:   InstFlags,
        align:   Align,
        order:   MemoryOrder,
        failure: MemoryOrder,
        args:    ValueList,
    },
    Load {
        ty:    Type,
        flags: InstFlags,
        align: Align,
        tag:   Option<AccessTag>,
        addr:  Value,
    },
    /// `store value, addr`.
    Store {
        ty:    Type,
        flags: InstFlags,
        align: Align,
        tag:   Option<AccessTag>,
        args:  [Value; 2],
    },
    /// `copy dst, src, size` and `fill dst, byte, size`.
    MemoryRange {
        opcode: Opcode,
        flags:  InstFlags,
        align:  Align,
        args:   [Value; 3],
    },
    Call {
        func: FuncId,
        args: ValueList,
    },
    /// The callee is the first value of `args`.
    CallIndirect {
        sig:  SigId,
        args: ValueList,
    },
    /// Inline assembly: the module's descriptor `asm` with one value per
    /// descriptor operand. GNU extension: GCC "Extended Asm".
    InlineAsm {
        asm:  AsmId,
        args: ValueList,
    },
    Jump {
        dest: BlockCall,
    },
    /// `brif cond, then, else`.
    Brif {
        cond:  Value,
        dests: [BlockCall; 2],
    },
    Switch {
        value:   Value,
        default: BlockCall,
        cases:   CaseList,
    },
    Return {
        value: Option<Value>,
    },
    /// `indirect_br addr, [dest, ...]`: continues at the listed edge whose
    /// target block's label has the address `addr`. Every target is an
    /// address-taken block; any other address is undefined behaviour, as for
    /// LLVM's `indirectbr`. GNU extension: GCC "Labels as Values".
    IndirectBr {
        addr:  Value,
        dests: BlockList,
    },
    Unreachable,
}

impl InstData {
    pub(crate) const fn opcode(&self) -> Opcode {
        match *self {
            | Self::Binary { opcode, .. }
            | Self::Unary { opcode, .. }
            | Self::Const { opcode, .. }
            | Self::WideConst { opcode, .. }
            | Self::Nullary { opcode, .. }
            | Self::MemoryRange { opcode, .. } => opcode,
            | Self::Atomic { .. } => Opcode::Atomic,
            | Self::DynAlloca { .. } => Opcode::DynAlloca,
            | Self::VaArgAggregate { .. } => Opcode::VaArgAggregate,
            | Self::IntCompare { .. } => Opcode::Icmp,
            | Self::ObjectSize { .. } => Opcode::ObjectSize,
            | Self::FloatCompare { .. } => Opcode::Fcmp,
            | Self::Select { .. } => Opcode::Select,
            | Self::StackAddr { .. } => Opcode::StackAddr,
            | Self::GlobalAddr { .. } => Opcode::GlobalAddr,
            | Self::FuncAddr { .. } => Opcode::FuncAddr,
            | Self::BlockAddr { .. } => Opcode::BlockAddr,
            | Self::Load { .. } => Opcode::Load,
            | Self::Store { .. } => Opcode::Store,
            | Self::Call { .. } => Opcode::Call,
            | Self::CallIndirect { .. } => Opcode::CallIndirect,
            | Self::InlineAsm { .. } => Opcode::InlineAsm,
            | Self::Jump { .. } => Opcode::Jump,
            | Self::Brif { .. } => Opcode::Brif,
            | Self::Switch { .. } => Opcode::Switch,
            | Self::Return { .. } => Opcode::Return,
            | Self::IndirectBr { .. } => Opcode::IndirectBr,
            | Self::Unreachable => Opcode::Unreachable,
        }
    }

    /// The flags of a format that has them, else none.
    pub(crate) const fn flags(&self) -> InstFlags {
        match *self {
            | Self::Atomic { flags, .. }
            | Self::Binary { flags, .. }
            | Self::Load { flags, .. }
            | Self::Store { flags, .. }
            | Self::MemoryRange { flags, .. } => flags,
            | _ => InstFlags::empty(),
        }
    }

    /// The controlling type the textual form writes after the opcode, for a
    /// format that has one.
    pub(crate) const fn controlling_type(&self) -> Option<Type> {
        match *self {
            | Self::Atomic { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Unary { ty, .. }
            | Self::DynAlloca { ty, .. }
            | Self::IntCompare { ty, .. }
            | Self::FloatCompare { ty, .. }
            | Self::ObjectSize { ty, .. }
            | Self::Select { ty, .. }
            | Self::Const { ty, .. }
            | Self::WideConst { ty, .. }
            | Self::Nullary { ty, .. }
            | Self::Load { ty, .. }
            | Self::Store { ty, .. } => Some(ty),
            | _ => None,
        }
    }

    pub(crate) const fn is_terminator(&self) -> bool {
        self.opcode().is_terminator()
    }
}

/// Defines a fieldless enum with a textual name per variant.
macro_rules! named {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal,)+ }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[repr(u8)]
        pub(crate) enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub(crate) const ALL: &[Self] = &[$(Self::$variant,)+];

            /// The name the textual form uses.
            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            pub(crate) fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|item| item.name() == name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.name())
            }
        }
    };
}

named! {
    /// Every operation of the IR.
    Opcode {
        Iadd = "iadd",
        Isub = "isub",
        Imul = "imul",
        Sdiv = "sdiv",
        Udiv = "udiv",
        Srem = "srem",
        Urem = "urem",
        And = "and",
        Or = "or",
        Xor = "xor",
        Shl = "shl",
        Lshr = "lshr",
        Ashr = "ashr",
        Fadd = "fadd",
        Fsub = "fsub",
        Fmul = "fmul",
        Fdiv = "fdiv",
        Frem = "frem",
        Fneg = "fneg",
        Clz = "clz",
        Ctz = "ctz",
        ClzDefined = "clz_defined",
        CtzDefined = "ctz_defined",
        Popcount = "popcount",
        Bswap = "bswap",
        UaddOverflow = "uadd_overflow",
        UmulOverflow = "umul_overflow",
        DynAlloca = "dyn_alloca",
        /// Captures the dynamic stack position for block lifetime cleanup.
        /// C99: §6.2.4p6, p. 32; PDF p. 44.
        StackSave = "stack_save",
        /// The current function's frame address, as GCC's
        /// `__builtin_frame_address (0)`.
        /// <https://gcc.gnu.org/onlinedocs/gcc/Return-Address.html>
        FrameAddress = "frame_address",
        /// The current function's return address, as GCC's
        /// `__builtin_return_address (0)`.
        ReturnAddress = "return_address",
        StackRestore = "stack_restore",
        Trap = "trap",
        /// The x86 time-stamp counter, an `i64` (Clang's `__rdtsc`). Every
        /// read is a distinct observation, never merged or reordered.
        ReadTimestamp = "read_timestamp",
        /// A breakpoint trap after which execution may continue
        /// (Microsoft's `__debugbreak`, LLVM's `llvm.debugtrap`).
        DebugTrap = "debug_trap",
        Icmp = "icmp",
        Fcmp = "fcmp",
        Select = "select",
        Freeze = "freeze",
        Zext = "zext",
        Sext = "sext",
        Trunc = "trunc",
        Fpext = "fpext",
        Fptrunc = "fptrunc",
        Fptosi = "fptosi",
        Fptoui = "fptoui",
        Sitofp = "sitofp",
        Uitofp = "uitofp",
        Ptrtoint = "ptrtoint",
        Inttoptr = "inttoptr",
        Bitcast = "bitcast",
        Iconst = "iconst",
        Fconst = "fconst",
        Poison = "poison",
        Null = "null",
        StackAddr = "stack_addr",
        GlobalAddr = "global_addr",
        FuncAddr = "func_addr",
        BlockAddr = "block_addr",
        PtrAdd = "ptr_add",
        /// `subobject ptr, size`: `ptr` itself, which designates a
        /// subobject with `size` (`i64`) bytes from `ptr` to its end, as
        /// the C expression it was computed from names one. Lowering
        /// passes it to the `always_inline` calls whose `object_size`
        /// queries of modes 1 and 3 ask for the closest surrounding
        /// subobject, which the IR's addresses do not record.
        /// GNU extension: GCC manual, "Object Size Checking".
        Subobject = "subobject",
        /// See [`InstData::ObjectSize`].
        ObjectSize = "object_size",
        Atomic = "atomic",
        Load = "load",
        Store = "store",
        Copy = "copy",
        Fill = "fill",
        Call = "call",
        CallIndirect = "call_indirect",
        /// Inline assembly; see `inline_asm.rs`.
        InlineAsm = "asm",
        VaStart = "va_start",
        VaArg = "va_arg",
        VaArgAggregate = "va_arg_aggregate",
        VaCopy = "va_copy",
        VaEnd = "va_end",
        Jump = "jump",
        Brif = "brif",
        Switch = "switch",
        Return = "return",
        IndirectBr = "indirect_br",
        Unreachable = "unreachable",
    }
}

impl Opcode {
    pub(crate) const fn is_terminator(self) -> bool {
        matches!(
            self,
            Self::Jump
                | Self::Brif
                | Self::Switch
                | Self::Return
                | Self::IndirectBr
                | Self::Unreachable
        )
    }

    /// `iadd` through `ashr`: integer operands and result of one type.
    pub(crate) const fn is_int_binary(self) -> bool {
        matches!(
            self,
            Self::Iadd
                | Self::Isub
                | Self::Imul
                | Self::Sdiv
                | Self::Udiv
                | Self::Srem
                | Self::Urem
                | Self::And
                | Self::Or
                | Self::Xor
                | Self::Shl
                | Self::Lshr
                | Self::Ashr
        )
    }

    pub(crate) const fn is_bit_unary(self) -> bool {
        matches!(
            self,
            Self::Clz
                | Self::Ctz
                | Self::ClzDefined
                | Self::CtzDefined
                | Self::Popcount
                | Self::Bswap
        )
    }

    pub(crate) const fn is_overflow(self) -> bool {
        matches!(self, Self::UaddOverflow | Self::UmulOverflow)
    }

    /// `fadd` through `frem`: floating operands and result of one type.
    pub(crate) const fn is_float_binary(self) -> bool {
        matches!(
            self,
            Self::Fadd | Self::Fsub | Self::Fmul | Self::Fdiv | Self::Frem
        )
    }

    /// The conversions, whose type is the result type.
    pub(crate) const fn is_conversion(self) -> bool {
        matches!(
            self,
            Self::Zext
                | Self::Sext
                | Self::Trunc
                | Self::Fpext
                | Self::Fptrunc
                | Self::Fptosi
                | Self::Fptoui
                | Self::Sitofp
                | Self::Uitofp
                | Self::Ptrtoint
                | Self::Inttoptr
                | Self::Bitcast
        )
    }

    /// Whether the textual form writes the controlling type after the
    /// opcode, as in `iadd.i32`. The others imply it (`ptr_add`, `null`) or
    /// have none.
    pub(crate) const fn has_type_suffix(self) -> bool {
        !matches!(
            self,
            Self::Null
                | Self::StackAddr
                | Self::GlobalAddr
                | Self::FuncAddr
                | Self::BlockAddr
                | Self::PtrAdd
                | Self::Subobject
                | Self::Copy
                | Self::Fill
                | Self::Call
                | Self::CallIndirect
                | Self::InlineAsm
                | Self::VaStart
                | Self::VaCopy
                | Self::VaEnd
                | Self::StackSave
                | Self::FrameAddress
                | Self::ReturnAddress
                | Self::StackRestore
                | Self::VaArgAggregate
                | Self::Jump
                | Self::Brif
                | Self::Switch
                | Self::Return
                | Self::IndirectBr
                | Self::Unreachable
                | Self::Trap
                | Self::ReadTimestamp
                | Self::DebugTrap
        )
    }

    /// The flags this opcode may carry; the verifier rejects any other.
    ///
    /// `nsw` exists because signed overflow is undefined in C.
    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    /// Unsigned arithmetic wraps and gets no flag.
    /// C99: §6.2.5 paragraph 9, p. 34; PDF p. 46.
    /// `inbounds` exists because pointer arithmetic may not leave its array
    /// object.
    /// C99: §6.5.6 paragraph 8, p. 83; PDF p. 95.
    /// `volatile` keeps an access as the abstract machine performs it.
    /// C99: §6.7.3 paragraph 6, p. 109; PDF p. 121.
    /// `may_overlap` exists because a structure assignment may copy an object
    /// onto itself exactly.
    /// C99: §6.5.16.1 paragraph 3, p. 92; PDF p. 104.
    pub(crate) const fn allowed_flags(self) -> InstFlags {
        match self {
            | Self::Iadd | Self::Isub | Self::Imul | Self::Shl =>
                InstFlags::NSW.union(InstFlags::NUW),
            | Self::Sdiv | Self::Udiv | Self::Lshr | Self::Ashr => InstFlags::EXACT,
            | Self::PtrAdd => InstFlags::INBOUNDS,
            | Self::Atomic | Self::Load | Self::Store | Self::Fill => InstFlags::VOLATILE,
            | Self::Copy => InstFlags::VOLATILE.union(InstFlags::MAY_OVERLAP),
            | _ => InstFlags::empty(),
        }
    }
}

named! {
    /// An integer comparison, signed (`s`) or unsigned (`u`).
    IntCC {
        Eq = "eq",
        Ne = "ne",
        Slt = "slt",
        Sle = "sle",
        Sgt = "sgt",
        Sge = "sge",
        Ult = "ult",
        Ule = "ule",
        Ugt = "ugt",
        Uge = "uge",
    }
}

named! {
    /// A floating comparison: ordered (`o`, false if either operand is NaN)
    /// or unordered (`u`, true if either is).
    FloatCC {
        Oeq = "oeq",
        One = "one",
        Olt = "olt",
        Ole = "ole",
        Ogt = "ogt",
        Oge = "oge",
        Ord = "ord",
        Ueq = "ueq",
        Une = "une",
        Ult = "ult",
        Ule = "ule",
        Ugt = "ugt",
        Uge = "uge",
        Uno = "uno",
    }
}

bitflags! {
    /// Facts an instruction states about its operands; see
    /// [`Opcode::allowed_flags`] for where each is legal and why.
    ///
    /// A violated `nsw`, `nuw`, `exact` or `inbounds` makes the result poison.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct InstFlags: u8 {
        /// No signed wrap.
        const NSW = 1 << 0;
        /// No unsigned wrap.
        const NUW = 1 << 1;
        /// The division or right shift discards no nonzero bits.
        const EXACT = 1 << 2;
        /// The offset pointer stays within its object.
        const INBOUNDS = 1 << 3;
        const VOLATILE = 1 << 4;
        /// The source and destination of a `copy` may overlap.
        const MAY_OVERLAP = 1 << 5;
    }
}

impl InstFlags {
    /// Each flag with its textual name, in printing order.
    pub(crate) const NAMES: [(Self, &'static str); 6] = [
        (Self::NSW, "nsw"),
        (Self::NUW, "nuw"),
        (Self::EXACT, "exact"),
        (Self::INBOUNDS, "inbounds"),
        (Self::VOLATILE, "volatile"),
        (Self::MAY_OVERLAP, "may_overlap"),
    ];

    /// The flag the textual form spells `name`.
    pub(crate) fn from_keyword(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|&&(_, text)| text == name)
            .map(|&(flag, _)| flag)
    }
}

impl fmt::Display for InstFlags {
    /// The flag names separated by spaces.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (flag, name) in Self::NAMES {
            if self.contains(flag) {
                write!(f, "{separator}{name}")?;
                separator = " ";
            }
        }
        Ok(())
    }
}

/// What an `object_size` asks, as GCC's `__builtin_object_size (ptr, mode)`
/// and `__builtin_dynamic_object_size`: bit 1 of the mode asks for the
/// fewest bytes left (else the most), bit 0 for the closest surrounding
/// subobject (else the whole object); a dynamic query may be answered by a
/// runtime value. Printed `object_size.i64 [dynamic] ptr, mode`.
/// GNU extension: GCC manual, "Object Size Checking".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ObjectSizeQuery {
    mode:    u8,
    dynamic: bool,
}

impl ObjectSizeQuery {
    /// The query of `mode` (0 through 3), or `None` for another mode.
    pub(crate) const fn new(mode: u8, dynamic: bool) -> Option<Self> {
        if mode <= 3 {
            Some(Self { mode, dynamic })
        } else {
            None
        }
    }

    pub(crate) const fn mode(self) -> u8 {
        self.mode
    }

    pub(crate) const fn dynamic(self) -> bool {
        self.dynamic
    }

    /// Modes 2 and 3: the answer must not exceed the bytes left.
    pub(crate) const fn minimum(self) -> bool {
        self.mode & 2 != 0
    }

    /// Modes 1 and 3: the object is the closest surrounding subobject.
    pub(crate) const fn subobject(self) -> bool {
        self.mode & 1 != 0
    }

    /// The answer when nothing is known, as a `ty` bit pattern: all ones
    /// (`(size_t) -1`) for the most bytes, zero for the fewest.
    pub(crate) const fn unknown(self, ty: Type) -> u128 {
        if self.minimum() { 0 } else { ty.mask() }
    }
}

/// A power-of-two alignment, stored as its base-2 logarithm. The IR limit
/// fits LLVM-C's unsigned alignment argument for all memory operations.
///
/// C99: §6.3.2.3 paragraph 7, pp. 47-48; PDF pp. 59-60 (object pointer
/// alignment); the maximum here is an IR representation limit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Align(u8);

impl Align {
    /// One byte.
    pub(crate) const BYTE: Self = Self(0);
    /// The base-2 logarithm of the largest alignment supported by bcc IR.
    /// LLVM's C++ maximum is 2^32, but `LLVMSetAlignment` takes a `u32`.
    pub(crate) const MAX_LOG2: u32 = 31;

    /// The alignment of `bytes`, if it is a power of two no greater than
    /// 2^31. This constructor enforces the limit for every IR alignment.
    pub(crate) const fn from_bytes(bytes: u64) -> Option<Self> {
        let log2 = bytes.trailing_zeros();
        if bytes.is_power_of_two() && log2 <= Self::MAX_LOG2 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The logarithm was just checked to be at most 31."
            )]
            let log2 = log2 as u8;
            Some(Self(log2))
        } else {
            None
        }
    }

    pub(crate) const fn bytes(self) -> u64 {
        1 << self.0
    }

    pub(crate) const fn log2(self) -> u8 {
        self.0
    }
}

/// The facts of one memory access, as the builder takes them. The record
/// keeps the flags, alignment and tag in separate fields, so the tag does not
/// pad the flags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MemFlags {
    pub(crate) volatile: bool,
    pub(crate) align:    Align,
    pub(crate) tag:      Option<AccessTag>,
}

impl MemFlags {
    /// A plain access with the given alignment.
    pub(crate) const fn aligned(align: Align) -> Self {
        Self {
            volatile: false,
            align,
            tag: None,
        }
    }
}

/// Up to 64 constant bits, split so the record keeps four-byte alignment.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Bits64([u32; 2]);

impl Bits64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "The halves are taken apart deliberately."
    )]
    pub(crate) const fn new(bits: u64) -> Self {
        Self([bits as u32, (bits >> 32) as u32])
    }

    pub(crate) fn get(self) -> u64 {
        u64::from(self.0[0]) | (u64::from(self.0[1]) << 32)
    }
}

/// A list of values in the function's value pool.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ValueList(pub(super) u32);
impl ValueList {
    pub(crate) const EMPTY: Self = Self(0);
}

/// A branch edge in the function's value pool: a target block with the
/// arguments for its parameters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BlockCall(pub(super) u32);

/// The edges of an `indirect_br` in the function's value pool: one block
/// call per listed target.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BlockList(pub(super) u32);

/// The cases of a `switch` in the function's value pool: pairs of a case
/// constant and an edge.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct CaseList(pub(super) u32);

named! {
    /// Atomic instruction contracts. All operations are observable memory
    /// barriers for bcc optimization, including relaxed loads.
    AtomicOp {
        Load = "load",
        Store = "store",
        Exchange = "exchange",
        CompareExchange = "compare_exchange",
        Add = "add",
        Sub = "sub",
        And = "and",
        Or = "or",
        Xor = "xor",
        Nand = "nand",
        Min = "min",
        Max = "max",
        Umin = "umin",
        Umax = "umax",
        Fadd = "fadd",
        Fsub = "fsub",
        Fmin = "fmin",
        Fmax = "fmax",
        LoadBytes = "load_bytes",
        StoreBytes = "store_bytes",
        ExchangeBytes = "exchange_bytes",
        CompareExchangeBytes = "compare_exchange_bytes",
        ThreadFence = "thread_fence",
        SignalFence = "signal_fence",
    }
}
impl AtomicOp {
    pub(crate) const fn arity(self) -> usize {
        match self {
            | Self::ThreadFence | Self::SignalFence => 0,
            | Self::Load => 1,
            | Self::LoadBytes | Self::StoreBytes | Self::CompareExchange => 3,
            | Self::ExchangeBytes | Self::CompareExchangeBytes => 4,
            | _ => 2,
        }
    }

    pub(crate) const fn has_result(self) -> bool {
        !matches!(
            self,
            Self::Store
                | Self::LoadBytes
                | Self::StoreBytes
                | Self::ExchangeBytes
                | Self::ThreadFence
                | Self::SignalFence
        )
    }

    pub(crate) const fn is_bytes(self) -> bool {
        matches!(
            self,
            Self::LoadBytes | Self::StoreBytes | Self::ExchangeBytes | Self::CompareExchangeBytes
        )
    }

    pub(crate) const fn is_fence(self) -> bool {
        matches!(self, Self::ThreadFence | Self::SignalFence)
    }
}
named! {
    /// LLVM-compatible ordering. C11 consume is conservatively acquire.
    /// C11: §7.17.3p6, pp. 275-276; PDF pp. 293-294.
    MemoryOrder {
        Relaxed = "relaxed",
        Acquire = "acquire",
        Release = "release",
        AcqRel = "acq_rel",
        SeqCst = "seq_cst",
    }
}
impl MemoryOrder {
    pub(crate) const fn from_c(order: i128) -> Self {
        match order {
            | 0 => Self::Relaxed,
            | 1 | 2 => Self::Acquire,
            | 3 => Self::Release,
            | 4 => Self::AcqRel,
            | _ => Self::SeqCst,
        }
    }

    pub(crate) const fn valid_failure(self, failure: Self) -> bool {
        matches!(failure, Self::Relaxed)
            || (matches!(failure, Self::Acquire)
                && matches!(self, Self::Acquire | Self::AcqRel | Self::SeqCst))
            || (matches!(failure, Self::SeqCst) && matches!(self, Self::SeqCst))
    }
}
