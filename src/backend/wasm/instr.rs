//! The WebAssembly instructions the back end emits, shared by the binary
//! encoder and the text printer so the two always agree.
//!
//! The instruction set is the WebAssembly 1.0 (MVP) core with three
//! standardized, universally supported extensions: sign-extension
//! operators (`i32.extend8_s` and friends), non-trapping float-to-int
//! conversions (`i32.trunc_sat_f64_s` and friends) and the bulk memory
//! operations `memory.copy` and `memory.fill`. Mutable globals are used for
//! the stack pointer, which the MVP allows for a defined, unexported global.
//! Neither multi-value results nor reference types are needed.
//!
//! Reference: WebAssembly Core Specification 2.0, §5.4 "Instructions"
//! (binary format) and §6.5 (text format).

/// One instruction. Numeric instructions without immediates share
/// [`Instr::Num`] with their one-byte opcode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Instr {
    Unreachable,
    Block,
    Loop,
    If,
    Else,
    End,
    Br(u32),
    BrIf(u32),
    /// `br_table` over `count` labels at `start` in the function's label
    /// pool, then the default label.
    BrTable {
        start:   u32,
        count:   u32,
        default: u32,
    },
    Return,
    Call(u32),
    /// `call_indirect` of a type through table 0.
    CallIndirect(u32),
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    /// A load or store opcode with its alignment's base-2 logarithm and a
    /// constant offset.
    Memory {
        opcode: u8,
        align:  u8,
        offset: u32,
    },
    I32Const(i32),
    I64Const(i64),
    /// The bits of an `f32` constant.
    F32Const(u32),
    /// The bits of an `f64` constant.
    F64Const(u64),
    /// A numeric instruction without immediates, `0x45` through `0xC4`.
    Num(u8),
    /// A saturating truncation, `0xFC` with sub-opcode 0 through 7.
    TruncSat(u8),
    MemoryCopy,
    MemoryFill,
    MemorySize,
    MemoryGrow,
    /// Not an instruction: the start of the code of IR instruction `inst`
    /// (`u32::MAX` for a block's terminator) of block `block`, from source
    /// line `line` (0 if unknown), which the source map and the text's
    /// comments report.
    Mark {
        block: u32,
        inst:  u32,
        line:  u32,
    },
}

/// A value type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    pub(crate) const fn code(self) -> u8 {
        match self {
            | Self::I32 => 0x7F,
            | Self::I64 => 0x7E,
            | Self::F32 => 0x7D,
            | Self::F64 => 0x7C,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::I32 => "i32",
            | Self::I64 => "i64",
            | Self::F32 => "f32",
            | Self::F64 => "f64",
        }
    }
}

/// Numeric opcodes, by their specification names.
#[expect(
    dead_code,
    reason = "The full table documents the opcode space the names cover."
)]
pub(crate) mod op {
    pub(crate) const I32_EQZ: u8 = 0x45;
    pub(crate) const I32_EQ: u8 = 0x46;
    pub(crate) const I32_NE: u8 = 0x47;
    pub(crate) const I32_LT_S: u8 = 0x48;
    pub(crate) const I32_LT_U: u8 = 0x49;
    pub(crate) const I32_GT_S: u8 = 0x4A;
    pub(crate) const I32_GT_U: u8 = 0x4B;
    pub(crate) const I32_LE_S: u8 = 0x4C;
    pub(crate) const I32_LE_U: u8 = 0x4D;
    pub(crate) const I32_GE_S: u8 = 0x4E;
    pub(crate) const I32_GE_U: u8 = 0x4F;
    pub(crate) const I64_EQZ: u8 = 0x50;
    pub(crate) const I64_EQ: u8 = 0x51;
    pub(crate) const I64_NE: u8 = 0x52;
    pub(crate) const I64_LT_S: u8 = 0x53;
    pub(crate) const I64_LT_U: u8 = 0x54;
    pub(crate) const I64_GT_S: u8 = 0x55;
    pub(crate) const I64_GT_U: u8 = 0x56;
    pub(crate) const I64_LE_S: u8 = 0x57;
    pub(crate) const I64_LE_U: u8 = 0x58;
    pub(crate) const I64_GE_S: u8 = 0x59;
    pub(crate) const I64_GE_U: u8 = 0x5A;
    pub(crate) const F32_EQ: u8 = 0x5B;
    pub(crate) const F32_NE: u8 = 0x5C;
    pub(crate) const F32_LT: u8 = 0x5D;
    pub(crate) const F32_GT: u8 = 0x5E;
    pub(crate) const F32_LE: u8 = 0x5F;
    pub(crate) const F32_GE: u8 = 0x60;
    pub(crate) const F64_EQ: u8 = 0x61;
    pub(crate) const F64_NE: u8 = 0x62;
    pub(crate) const F64_LT: u8 = 0x63;
    pub(crate) const F64_GT: u8 = 0x64;
    pub(crate) const F64_LE: u8 = 0x65;
    pub(crate) const F64_GE: u8 = 0x66;
    pub(crate) const I32_CLZ: u8 = 0x67;
    pub(crate) const I32_CTZ: u8 = 0x68;
    pub(crate) const I32_POPCNT: u8 = 0x69;
    pub(crate) const I32_ADD: u8 = 0x6A;
    pub(crate) const I32_SUB: u8 = 0x6B;
    pub(crate) const I32_MUL: u8 = 0x6C;
    pub(crate) const I32_DIV_S: u8 = 0x6D;
    pub(crate) const I32_DIV_U: u8 = 0x6E;
    pub(crate) const I32_REM_S: u8 = 0x6F;
    pub(crate) const I32_REM_U: u8 = 0x70;
    pub(crate) const I32_AND: u8 = 0x71;
    pub(crate) const I32_OR: u8 = 0x72;
    pub(crate) const I32_XOR: u8 = 0x73;
    pub(crate) const I32_SHL: u8 = 0x74;
    pub(crate) const I32_SHR_S: u8 = 0x75;
    pub(crate) const I32_SHR_U: u8 = 0x76;
    pub(crate) const I32_ROTL: u8 = 0x77;
    pub(crate) const I32_ROTR: u8 = 0x78;
    pub(crate) const I64_CLZ: u8 = 0x79;
    pub(crate) const I64_CTZ: u8 = 0x7A;
    pub(crate) const I64_POPCNT: u8 = 0x7B;
    pub(crate) const I64_ADD: u8 = 0x7C;
    pub(crate) const I64_SUB: u8 = 0x7D;
    pub(crate) const I64_MUL: u8 = 0x7E;
    pub(crate) const I64_DIV_S: u8 = 0x7F;
    pub(crate) const I64_DIV_U: u8 = 0x80;
    pub(crate) const I64_REM_S: u8 = 0x81;
    pub(crate) const I64_REM_U: u8 = 0x82;
    pub(crate) const I64_AND: u8 = 0x83;
    pub(crate) const I64_OR: u8 = 0x84;
    pub(crate) const I64_XOR: u8 = 0x85;
    pub(crate) const I64_SHL: u8 = 0x86;
    pub(crate) const I64_SHR_S: u8 = 0x87;
    pub(crate) const I64_SHR_U: u8 = 0x88;
    pub(crate) const I64_ROTL: u8 = 0x89;
    pub(crate) const I64_ROTR: u8 = 0x8A;
    pub(crate) const F32_ABS: u8 = 0x8B;
    pub(crate) const F32_NEG: u8 = 0x8C;
    pub(crate) const F32_CEIL: u8 = 0x8D;
    pub(crate) const F32_FLOOR: u8 = 0x8E;
    pub(crate) const F32_TRUNC: u8 = 0x8F;
    pub(crate) const F32_NEAREST: u8 = 0x90;
    pub(crate) const F32_SQRT: u8 = 0x91;
    pub(crate) const F32_ADD: u8 = 0x92;
    pub(crate) const F32_SUB: u8 = 0x93;
    pub(crate) const F32_MUL: u8 = 0x94;
    pub(crate) const F32_DIV: u8 = 0x95;
    pub(crate) const F32_MIN: u8 = 0x96;
    pub(crate) const F32_MAX: u8 = 0x97;
    pub(crate) const F32_COPYSIGN: u8 = 0x98;
    pub(crate) const F64_ABS: u8 = 0x99;
    pub(crate) const F64_NEG: u8 = 0x9A;
    pub(crate) const F64_CEIL: u8 = 0x9B;
    pub(crate) const F64_FLOOR: u8 = 0x9C;
    pub(crate) const F64_TRUNC: u8 = 0x9D;
    pub(crate) const F64_NEAREST: u8 = 0x9E;
    pub(crate) const F64_SQRT: u8 = 0x9F;
    pub(crate) const F64_ADD: u8 = 0xA0;
    pub(crate) const F64_SUB: u8 = 0xA1;
    pub(crate) const F64_MUL: u8 = 0xA2;
    pub(crate) const F64_DIV: u8 = 0xA3;
    pub(crate) const F64_MIN: u8 = 0xA4;
    pub(crate) const F64_MAX: u8 = 0xA5;
    pub(crate) const F64_COPYSIGN: u8 = 0xA6;
    pub(crate) const I32_WRAP_I64: u8 = 0xA7;
    pub(crate) const I64_EXTEND_I32_S: u8 = 0xAC;
    pub(crate) const I64_EXTEND_I32_U: u8 = 0xAD;
    pub(crate) const F32_CONVERT_I32_S: u8 = 0xB2;
    pub(crate) const F32_CONVERT_I32_U: u8 = 0xB3;
    pub(crate) const F32_CONVERT_I64_S: u8 = 0xB4;
    pub(crate) const F32_CONVERT_I64_U: u8 = 0xB5;
    pub(crate) const F32_DEMOTE_F64: u8 = 0xB6;
    pub(crate) const F64_CONVERT_I32_S: u8 = 0xB7;
    pub(crate) const F64_CONVERT_I32_U: u8 = 0xB8;
    pub(crate) const F64_CONVERT_I64_S: u8 = 0xB9;
    pub(crate) const F64_CONVERT_I64_U: u8 = 0xBA;
    pub(crate) const F64_PROMOTE_F32: u8 = 0xBB;
    pub(crate) const I32_REINTERPRET_F32: u8 = 0xBC;
    pub(crate) const I64_REINTERPRET_F64: u8 = 0xBD;
    pub(crate) const F32_REINTERPRET_I32: u8 = 0xBE;
    pub(crate) const F64_REINTERPRET_I64: u8 = 0xBF;
    pub(crate) const I32_EXTEND8_S: u8 = 0xC0;
    pub(crate) const I32_EXTEND16_S: u8 = 0xC1;
    pub(crate) const I64_EXTEND8_S: u8 = 0xC2;
    pub(crate) const I64_EXTEND16_S: u8 = 0xC3;
    pub(crate) const I64_EXTEND32_S: u8 = 0xC4;

    pub(crate) const I32_LOAD: u8 = 0x28;
    pub(crate) const I64_LOAD: u8 = 0x29;
    pub(crate) const F32_LOAD: u8 = 0x2A;
    pub(crate) const F64_LOAD: u8 = 0x2B;
    pub(crate) const I32_LOAD8_S: u8 = 0x2C;
    pub(crate) const I32_LOAD8_U: u8 = 0x2D;
    pub(crate) const I32_LOAD16_S: u8 = 0x2E;
    pub(crate) const I32_LOAD16_U: u8 = 0x2F;
    pub(crate) const I64_LOAD8_U: u8 = 0x31;
    pub(crate) const I64_LOAD16_U: u8 = 0x33;
    pub(crate) const I64_LOAD32_U: u8 = 0x35;
    pub(crate) const I32_STORE: u8 = 0x36;
    pub(crate) const I64_STORE: u8 = 0x37;
    pub(crate) const F32_STORE: u8 = 0x38;
    pub(crate) const F64_STORE: u8 = 0x39;
    pub(crate) const I32_STORE8: u8 = 0x3A;
    pub(crate) const I32_STORE16: u8 = 0x3B;
    pub(crate) const I64_STORE8: u8 = 0x3C;
    pub(crate) const I64_STORE16: u8 = 0x3D;
    pub(crate) const I64_STORE32: u8 = 0x3E;

    /// `i32.trunc_sat_f32_s` and the seven after it, by sub-opcode.
    pub(crate) const I32_TRUNC_SAT_F32_S: u8 = 0;
    pub(crate) const I32_TRUNC_SAT_F32_U: u8 = 1;
    pub(crate) const I32_TRUNC_SAT_F64_S: u8 = 2;
    pub(crate) const I32_TRUNC_SAT_F64_U: u8 = 3;
    pub(crate) const I64_TRUNC_SAT_F32_S: u8 = 4;
    pub(crate) const I64_TRUNC_SAT_F32_U: u8 = 5;
    pub(crate) const I64_TRUNC_SAT_F64_S: u8 = 6;
    pub(crate) const I64_TRUNC_SAT_F64_U: u8 = 7;
}

/// The text name of numeric opcode `opcode` (`0x45` through `0xC4`).
pub(crate) fn num_name(opcode: u8) -> &'static str {
    NUM_NAMES
        .get(usize::from(opcode.wrapping_sub(0x45)))
        .copied()
        .unwrap_or("<invalid>")
}

/// The text name of load or store opcode `opcode` (`0x28` through `0x3E`).
pub(crate) fn memory_name(opcode: u8) -> &'static str {
    MEMORY_NAMES
        .get(usize::from(opcode.wrapping_sub(0x28)))
        .copied()
        .unwrap_or("<invalid>")
}

/// The text name of saturating truncation `sub`.
pub(crate) fn trunc_sat_name(sub: u8) -> &'static str {
    TRUNC_SAT_NAMES
        .get(usize::from(sub))
        .copied()
        .unwrap_or("<invalid>")
}

const NUM_NAMES: [&str; 0xC5 - 0x45] = [
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

const MEMORY_NAMES: [&str; 0x3F - 0x28] = [
    "i32.load",
    "i64.load",
    "f32.load",
    "f64.load",
    "i32.load8_s",
    "i32.load8_u",
    "i32.load16_s",
    "i32.load16_u",
    "i64.load8_s",
    "i64.load8_u",
    "i64.load16_s",
    "i64.load16_u",
    "i64.load32_s",
    "i64.load32_u",
    "i32.store",
    "i64.store",
    "f32.store",
    "f64.store",
    "i32.store8",
    "i32.store16",
    "i64.store8",
    "i64.store16",
    "i64.store32",
];

const TRUNC_SAT_NAMES: [&str; 8] = [
    "i32.trunc_sat_f32_s",
    "i32.trunc_sat_f32_u",
    "i32.trunc_sat_f64_s",
    "i32.trunc_sat_f64_u",
    "i64.trunc_sat_f32_s",
    "i64.trunc_sat_f32_u",
    "i64.trunc_sat_f64_s",
    "i64.trunc_sat_f64_u",
];
