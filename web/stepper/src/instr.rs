//! Decoded instructions (WebAssembly 2.0 §2.4) and their text-format names.
//!
//! Structured instructions carry the instruction index of their `else` and
//! `end`, found once while decoding, so a branch jumps without scanning.
//! Numeric instructions keep their opcode, and `exec/numeric.rs` and
//! [`numeric_name`] dispatch on it.

use crate::module::{
    BlockType,
    ValType,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Instr {
    Unreachable,
    Nop,
    Block {
        ty:  BlockType,
        end: u32,
    },
    Loop {
        ty:  BlockType,
        end: u32,
    },
    /// `else_` is the index of the `else`, or `end` when there is none.
    If {
        ty:    BlockType,
        else_: u32,
        end:   u32,
    },
    Else {
        end: u32,
    },
    End,
    Br(u32),
    BrIf(u32),
    BrTable {
        targets: Box<[u32]>,
        default: u32,
    },
    Return,
    Call(u32),
    CallIndirect {
        ty:    u32,
        table: u32,
    },
    Drop,
    /// `select`, or `select t` with its operand type.
    Select(Option<ValType>),
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    TableGet(u32),
    TableSet(u32),
    Load(Load, MemArg),
    Store(Store, MemArg),
    MemorySize,
    MemoryGrow,
    MemoryInit(u32),
    DataDrop(u32),
    MemoryCopy,
    MemoryFill,
    TableInit {
        elem:  u32,
        table: u32,
    },
    ElemDrop(u32),
    TableCopy {
        dst: u32,
        src: u32,
    },
    TableGrow(u32),
    TableSize(u32),
    TableFill(u32),
    I32Const(i32),
    I64Const(i64),
    /// The constant's bits, so NaN payloads survive.
    F32Const(u32),
    F64Const(u64),
    RefNull(ValType),
    RefIsNull,
    RefFunc(u32),
    /// A one-byte numeric opcode, 0x45 through 0xC4: comparisons,
    /// arithmetic, conversions and sign extension.
    Numeric(u8),
    /// A saturating truncation, `0xFC` 0 through 7.
    TruncSat(u8),
}

/// A load's width and extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    I32,
    I64,
    F32,
    F64,
    I32_8S,
    I32_8U,
    I32_16S,
    I32_16U,
    I64_8S,
    I64_8U,
    I64_16S,
    I64_16U,
    I64_32S,
    I64_32U,
}

impl Load {
    pub const ALL: [Self; 14] = [
        Self::I32,
        Self::I64,
        Self::F32,
        Self::F64,
        Self::I32_8S,
        Self::I32_8U,
        Self::I32_16S,
        Self::I32_16U,
        Self::I64_8S,
        Self::I64_8U,
        Self::I64_16S,
        Self::I64_16U,
        Self::I64_32S,
        Self::I64_32U,
    ];

    /// Bytes read.
    pub fn size(self) -> u32 {
        match self {
            | Self::I32_8S | Self::I32_8U | Self::I64_8S | Self::I64_8U => 1,
            | Self::I32_16S | Self::I32_16U | Self::I64_16S | Self::I64_16U => 2,
            | Self::I32 | Self::F32 | Self::I64_32S | Self::I64_32U => 4,
            | Self::I64 | Self::F64 => 8,
        }
    }

    /// The type of the value pushed.
    pub fn ty(self) -> ValType {
        match self {
            | Self::I32 | Self::I32_8S | Self::I32_8U | Self::I32_16S | Self::I32_16U =>
                ValType::I32,
            | Self::F32 => ValType::F32,
            | Self::F64 => ValType::F64,
            | _ => ValType::I64,
        }
    }

    pub fn signed(self) -> bool {
        matches!(
            self,
            Self::I32_8S | Self::I32_16S | Self::I64_8S | Self::I64_16S | Self::I64_32S
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            | Self::I32 => "i32.load",
            | Self::I64 => "i64.load",
            | Self::F32 => "f32.load",
            | Self::F64 => "f64.load",
            | Self::I32_8S => "i32.load8_s",
            | Self::I32_8U => "i32.load8_u",
            | Self::I32_16S => "i32.load16_s",
            | Self::I32_16U => "i32.load16_u",
            | Self::I64_8S => "i64.load8_s",
            | Self::I64_8U => "i64.load8_u",
            | Self::I64_16S => "i64.load16_s",
            | Self::I64_16U => "i64.load16_u",
            | Self::I64_32S => "i64.load32_s",
            | Self::I64_32U => "i64.load32_u",
        }
    }
}

/// A store's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    I32,
    I64,
    F32,
    F64,
    I32_8,
    I32_16,
    I64_8,
    I64_16,
    I64_32,
}

impl Store {
    pub const ALL: [Self; 9] = [
        Self::I32,
        Self::I64,
        Self::F32,
        Self::F64,
        Self::I32_8,
        Self::I32_16,
        Self::I64_8,
        Self::I64_16,
        Self::I64_32,
    ];

    /// Bytes written.
    pub fn size(self) -> u32 {
        match self {
            | Self::I32_8 | Self::I64_8 => 1,
            | Self::I32_16 | Self::I64_16 => 2,
            | Self::I32 | Self::F32 | Self::I64_32 => 4,
            | Self::I64 | Self::F64 => 8,
        }
    }

    /// The type of the value stored.
    pub fn ty(self) -> ValType {
        match self {
            | Self::I32 | Self::I32_8 | Self::I32_16 => ValType::I32,
            | Self::F32 => ValType::F32,
            | Self::F64 => ValType::F64,
            | Self::I64 | Self::I64_8 | Self::I64_16 | Self::I64_32 => ValType::I64,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            | Self::I32 => "i32.store",
            | Self::I64 => "i64.store",
            | Self::F32 => "f32.store",
            | Self::F64 => "f64.store",
            | Self::I32_8 => "i32.store8",
            | Self::I32_16 => "i32.store16",
            | Self::I64_8 => "i64.store8",
            | Self::I64_16 => "i64.store16",
            | Self::I64_32 => "i64.store32",
        }
    }
}

/// A memory instruction's immediate: the alignment hint (log2 bytes) and the
/// constant offset added to the address operand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemArg {
    pub align:  u32,
    pub offset: u32,
}

/// The text-format name of numeric opcode `op` (0x45..=0xC4).
pub fn numeric_name(op: u8) -> &'static str {
    NUMERIC_NAMES
        .get(usize::from(op.wrapping_sub(0x45)))
        .copied()
        .unwrap_or("unknown")
}

/// The text-format name of saturating truncation `op` (0..=7).
pub fn trunc_sat_name(op: u8) -> &'static str {
    [
        "i32.trunc_sat_f32_s",
        "i32.trunc_sat_f32_u",
        "i32.trunc_sat_f64_s",
        "i32.trunc_sat_f64_u",
        "i64.trunc_sat_f32_s",
        "i64.trunc_sat_f32_u",
        "i64.trunc_sat_f64_s",
        "i64.trunc_sat_f64_u",
    ]
    .get(usize::from(op))
    .copied()
    .unwrap_or("unknown")
}

/// A numeric instruction's operand and result types: `(operands, result)`.
pub fn numeric_signature(op: u8) -> Option<(&'static [ValType], ValType)> {
    use ValType::{
        F32,
        F64,
        I32,
        I64,
    };
    Some(match op {
        // Tests, unary integer operators and sign extension.
        | 0x45 | 0x67..=0x69 | 0xC0 | 0xC1 => (&[I32], I32),
        | 0x50 | 0xA7 => (&[I64], I32),
        | 0x79..=0x7B | 0xC2..=0xC4 => (&[I64], I64),
        // Comparisons and binary operators.
        | 0x46..=0x4F | 0x6A..=0x78 => (&[I32, I32], I32),
        | 0x51..=0x5A => (&[I64, I64], I32),
        | 0x5B..=0x60 => (&[F32, F32], I32),
        | 0x61..=0x66 => (&[F64, F64], I32),
        | 0x7C..=0x8A => (&[I64, I64], I64),
        | 0x8B..=0x91 => (&[F32], F32),
        | 0x92..=0x98 => (&[F32, F32], F32),
        | 0x99..=0x9F => (&[F64], F64),
        | 0xA0..=0xA6 => (&[F64, F64], F64),
        // Conversions and reinterpretations.
        | 0xA8 | 0xA9 | 0xBC => (&[F32], I32),
        | 0xAA | 0xAB => (&[F64], I32),
        | 0xAC | 0xAD => (&[I32], I64),
        | 0xAE | 0xAF => (&[F32], I64),
        | 0xB0 | 0xB1 | 0xBD => (&[F64], I64),
        | 0xB2 | 0xB3 | 0xBE => (&[I32], F32),
        | 0xB4 | 0xB5 => (&[I64], F32),
        | 0xB6 => (&[F64], F32),
        | 0xB7 | 0xB8 => (&[I32], F64),
        | 0xB9 | 0xBA | 0xBF => (&[I64], F64),
        | 0xBB => (&[F32], F64),
        | _ => return None,
    })
}

/// A saturating truncation's operand and result types.
pub fn trunc_sat_signature(op: u8) -> Option<(ValType, ValType)> {
    use ValType::{
        F32,
        F64,
        I32,
        I64,
    };
    Some(match op {
        | 0 | 1 => (F32, I32),
        | 2 | 3 => (F64, I32),
        | 4 | 5 => (F32, I64),
        | 6 | 7 => (F64, I64),
        | _ => return None,
    })
}

const NUMERIC_NAMES: [&str; 128] = [
    "i32.eqz",
    "i32.eq",
    "i32.ne",
    "i32.lt_s",
    "i32.lt_u",
    "i32.gt_s",
    "i32.gt_u",
    "i32.le_s",
    "i32.le_u",
    "i32.ge_s",
    "i32.ge_u",
    "i64.eqz",
    "i64.eq",
    "i64.ne",
    "i64.lt_s",
    "i64.lt_u",
    "i64.gt_s",
    "i64.gt_u",
    "i64.le_s",
    "i64.le_u",
    "i64.ge_s",
    "i64.ge_u",
    "f32.eq",
    "f32.ne",
    "f32.lt",
    "f32.gt",
    "f32.le",
    "f32.ge",
    "f64.eq",
    "f64.ne",
    "f64.lt",
    "f64.gt",
    "f64.le",
    "f64.ge",
    "i32.clz",
    "i32.ctz",
    "i32.popcnt",
    "i32.add",
    "i32.sub",
    "i32.mul",
    "i32.div_s",
    "i32.div_u",
    "i32.rem_s",
    "i32.rem_u",
    "i32.and",
    "i32.or",
    "i32.xor",
    "i32.shl",
    "i32.shr_s",
    "i32.shr_u",
    "i32.rotl",
    "i32.rotr",
    "i64.clz",
    "i64.ctz",
    "i64.popcnt",
    "i64.add",
    "i64.sub",
    "i64.mul",
    "i64.div_s",
    "i64.div_u",
    "i64.rem_s",
    "i64.rem_u",
    "i64.and",
    "i64.or",
    "i64.xor",
    "i64.shl",
    "i64.shr_s",
    "i64.shr_u",
    "i64.rotl",
    "i64.rotr",
    "f32.abs",
    "f32.neg",
    "f32.ceil",
    "f32.floor",
    "f32.trunc",
    "f32.nearest",
    "f32.sqrt",
    "f32.add",
    "f32.sub",
    "f32.mul",
    "f32.div",
    "f32.min",
    "f32.max",
    "f32.copysign",
    "f64.abs",
    "f64.neg",
    "f64.ceil",
    "f64.floor",
    "f64.trunc",
    "f64.nearest",
    "f64.sqrt",
    "f64.add",
    "f64.sub",
    "f64.mul",
    "f64.div",
    "f64.min",
    "f64.max",
    "f64.copysign",
    "i32.wrap_i64",
    "i32.trunc_f32_s",
    "i32.trunc_f32_u",
    "i32.trunc_f64_s",
    "i32.trunc_f64_u",
    "i64.extend_i32_s",
    "i64.extend_i32_u",
    "i64.trunc_f32_s",
    "i64.trunc_f32_u",
    "i64.trunc_f64_s",
    "i64.trunc_f64_u",
    "f32.convert_i32_s",
    "f32.convert_i32_u",
    "f32.convert_i64_s",
    "f32.convert_i64_u",
    "f32.demote_f64",
    "f64.convert_i32_s",
    "f64.convert_i32_u",
    "f64.convert_i64_s",
    "f64.convert_i64_u",
    "f64.promote_f32",
    "i32.reinterpret_f32",
    "i64.reinterpret_f64",
    "f32.reinterpret_i32",
    "f64.reinterpret_i64",
    "i32.extend8_s",
    "i32.extend16_s",
    "i64.extend8_s",
    "i64.extend16_s",
    "i64.extend32_s",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_names_line_up_with_their_opcodes() {
        assert_eq!(numeric_name(0x45), "i32.eqz");
        assert_eq!(numeric_name(0x6A), "i32.add");
        assert_eq!(numeric_name(0x7C), "i64.add");
        assert_eq!(numeric_name(0x92), "f32.add");
        assert_eq!(numeric_name(0xA0), "f64.add");
        assert_eq!(numeric_name(0xA7), "i32.wrap_i64");
        assert_eq!(numeric_name(0xBF), "f64.reinterpret_i64");
        assert_eq!(numeric_name(0xC4), "i64.extend32_s");
        assert_eq!(numeric_name(0xC5), "unknown");
        for op in 0x45..=0xC4 {
            assert!(numeric_signature(op).is_some(), "{op:#x}");
        }
    }
}
