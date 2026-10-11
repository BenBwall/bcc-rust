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
    /// Integer and floating arithmetic, `ptr_add` (type `ptr`) and `va_copy`
    /// (type `ptr`, no result).
    Binary {
        opcode: Opcode,
        ty:     Type,
        flags:  InstFlags,
        args:   [Value; 2],
    },
    /// `fneg`, `freeze`, the conversions (type = result type), `va_arg`, and
    /// `va_start` and `va_end` (type `ptr`, no result).
    Unary {
        opcode: Opcode,
        ty:     Type,
        arg:    Value,
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
    /// `poison` and `null` (type `ptr`).
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
            | Self::IntCompare { .. } => Opcode::Icmp,
            | Self::FloatCompare { .. } => Opcode::Fcmp,
            | Self::Select { .. } => Opcode::Select,
            | Self::StackAddr { .. } => Opcode::StackAddr,
            | Self::GlobalAddr { .. } => Opcode::GlobalAddr,
            | Self::FuncAddr { .. } => Opcode::FuncAddr,
            | Self::Load { .. } => Opcode::Load,
            | Self::Store { .. } => Opcode::Store,
            | Self::Call { .. } => Opcode::Call,
            | Self::CallIndirect { .. } => Opcode::CallIndirect,
            | Self::Jump { .. } => Opcode::Jump,
            | Self::Brif { .. } => Opcode::Brif,
            | Self::Switch { .. } => Opcode::Switch,
            | Self::Return { .. } => Opcode::Return,
            | Self::Unreachable => Opcode::Unreachable,
        }
    }

    /// The flags of a format that has them, else none.
    pub(crate) const fn flags(&self) -> InstFlags {
        match *self {
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
            | Self::Binary { ty, .. }
            | Self::Unary { ty, .. }
            | Self::IntCompare { ty, .. }
            | Self::FloatCompare { ty, .. }
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
        PtrAdd = "ptr_add",
        Load = "load",
        Store = "store",
        Copy = "copy",
        Fill = "fill",
        Call = "call",
        CallIndirect = "call_indirect",
        VaStart = "va_start",
        VaArg = "va_arg",
        VaCopy = "va_copy",
        VaEnd = "va_end",
        Jump = "jump",
        Brif = "brif",
        Switch = "switch",
        Return = "return",
        Unreachable = "unreachable",
    }
}

impl Opcode {
    pub(crate) const fn is_terminator(self) -> bool {
        matches!(
            self,
            Self::Jump | Self::Brif | Self::Switch | Self::Return | Self::Unreachable
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
                | Self::PtrAdd
                | Self::Copy
                | Self::Fill
                | Self::Call
                | Self::CallIndirect
                | Self::VaStart
                | Self::VaCopy
                | Self::VaEnd
                | Self::Jump
                | Self::Brif
                | Self::Switch
                | Self::Return
                | Self::Unreachable
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
            | Self::Load | Self::Store | Self::Fill => InstFlags::VOLATILE,
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

/// A power-of-two alignment, stored as its base-2 logarithm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Align(u8);

impl Align {
    /// One byte.
    pub(crate) const BYTE: Self = Self(0);
    /// The base-2 logarithm of the largest alignment LLVM accepts.
    pub(crate) const MAX_LOG2: u32 = 32;

    /// The alignment of `bytes`, if it is a power of two no greater than
    /// 2^32.
    pub(crate) const fn from_bytes(bytes: u64) -> Option<Self> {
        let log2 = bytes.trailing_zeros();
        if bytes.is_power_of_two() && log2 <= Self::MAX_LOG2 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "The logarithm was just checked to be at most 32."
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

/// A branch edge in the function's value pool: a target block with the
/// arguments for its parameters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BlockCall(pub(super) u32);

/// The cases of a `switch` in the function's value pool: pairs of a case
/// constant and an edge.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct CaseList(pub(super) u32);
