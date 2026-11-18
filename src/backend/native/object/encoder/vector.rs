//! The SSE and AVX instructions of the x86 intrinsics (`isel/vector.rs`),
//! in their SSE forms (a mandatory prefix, `0F` and a map byte) and their
//! VEX forms (Intel SDM Vol. 2A, §2.3.5, "The VEX Prefix", and §3.1.1.2,
//! "Opcode Column in the Instruction Summary Table (Instructions with VEX
//! prefix)").

use super::{
    Code,
    Error,
    Operand,
    Result,
    Shape,
};

/// An instruction's encoding: the mandatory prefix as VEX's `pp` field
/// (none, `66`, `F3`, `F2`), the opcode map as VEX's `mmmmm` (`0F`, `0F38`,
/// `0F3A`) and the opcode.
#[derive(Clone, Copy)]
pub(super) struct Spec {
    vex:      bool,
    pp:       u8,
    map:      u8,
    opcode:   u8,
    /// Whether an 8-bit immediate comes first.
    imm:      bool,
    /// Whether the VEX form names the first source in `vvvv`, as an operand
    /// before the destination: `vaesenc %xmm2, %xmm1, %xmm0`.
    binary:   bool,
    /// What a 32-byte (`L` = 1) form does, if it has one.
    long:     Long,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Long {
    None,
    /// YMM source and destination.
    Same,
    /// A YMM source and an XMM destination (`vcvtpd2dq`).
    Narrow,
}

/// The instructions by their SSE mnemonic.
#[rustfmt::skip]
const TABLE: [(&str, u8, u8, u8, bool, bool, Long, bool); 23] = [
    // name, pp, map, opcode, imm, binary, long, has_vex
    ("aesenc",          1, 2, 0xDC, false, true,  Long::None,   true),
    ("aesenclast",      1, 2, 0xDD, false, true,  Long::None,   true),
    ("aesdec",          1, 2, 0xDE, false, true,  Long::None,   true),
    ("aesdeclast",      1, 2, 0xDF, false, true,  Long::None,   true),
    ("aesimc",          1, 2, 0xDB, false, false, Long::None,   true),
    ("aeskeygenassist", 1, 3, 0xDF, true,  false, Long::None,   true),
    ("pclmulqdq",       1, 3, 0x44, true,  true,  Long::None,   true),
    ("roundps",         1, 3, 0x08, true,  false, Long::Same,   true),
    ("roundpd",         1, 3, 0x09, true,  false, Long::Same,   true),
    ("roundss",         1, 3, 0x0A, true,  true,  Long::None,   true),
    ("roundsd",         1, 3, 0x0B, true,  true,  Long::None,   true),
    ("cvtps2dq",        1, 1, 0x5B, false, false, Long::Same,   true),
    ("cvttps2dq",       2, 1, 0x5B, false, false, Long::Same,   true),
    ("cvtpd2dq",        3, 1, 0xE6, false, false, Long::Narrow, true),
    ("cvttpd2dq",       1, 1, 0xE6, false, false, Long::Narrow, true),
    ("rcpps",           0, 1, 0x53, false, false, Long::Same,   true),
    ("rsqrtps",         0, 1, 0x52, false, false, Long::Same,   true),
    ("sqrtss",          2, 1, 0x51, false, false, Long::None,   false),
    ("sqrtsd",          3, 1, 0x51, false, false, Long::None,   false),
    ("pcmpestrm",       1, 3, 0x60, true,  false, Long::None,   true),
    ("pcmpestri",       1, 3, 0x61, true,  false, Long::None,   true),
    ("pcmpistrm",       1, 3, 0x62, true,  false, Long::None,   true),
    ("pcmpistri",       1, 3, 0x63, true,  false, Long::None,   true),
];

/// The instruction `name` names, if it is one of these.
pub(super) fn spec(name: &str) -> Option<Spec> {
    let (vex, stem) = match name.strip_prefix('v') {
        | Some(stem) => (true, stem),
        | None => (false, name),
    };
    let &(_, pp, map, opcode, imm, binary, long, has_vex) =
        TABLE.iter().find(|row| row.0 == stem)?;
    (!vex || has_vex).then_some(Spec {
        vex,
        pp,
        map,
        opcode,
        imm,
        binary: binary && vex,
        long,
    })
}

impl Spec {
    /// Checks the operand count, kinds and widths: `[$imm,] src, [src1,]
    /// dst`, where only `src` may be memory, and a 32-byte form has YMM
    /// registers (a narrowing one an XMM destination, and a register source
    /// in either VEX width, since only the source's width tells them
    /// apart).
    pub(super) fn validate(self, operands: &[Operand<'_>]) -> Result<()> {
        let fail = || Err(Error("unsupported instruction operand count, kind or width"));
        let registers = 2 + usize::from(self.binary);
        let count = registers + usize::from(self.imm);
        if operands.len() != count {
            return fail();
        }
        if self.imm && !matches!(operands[0], Operand::Imm(0..=255)) {
            return fail();
        }
        let operands = &operands[usize::from(self.imm)..];
        let (src, dst) = (operands[0], operands[registers - 1]);
        if self.vex && self.long == Long::Narrow && !matches!(src, Operand::Reg(_)) {
            return fail();
        }
        let long = match self.long {
            | Long::Narrow => self.vex && src.ymm(),
            | Long::Same => self.vex && dst.ymm(),
            | Long::None => false,
        };
        let (source, destination) = match (long, self.long) {
            | (false, _) => (Shape::Xm, Shape::Xmm),
            | (true, Long::Narrow) => (Shape::Ymm, Shape::Xmm),
            | (true, _) => (Shape::Ym, Shape::Ymm),
        };
        let accepted = source.accepts(src)
            && operands[1..].iter().all(|&op| destination.accepts(op));
        if accepted { Ok(()) } else { fail() }
    }
}

impl<'a> Code<'a> {
    /// Encodes a validated instruction of [`spec`].
    pub(super) fn vector(&mut self, spec: Spec, operands: &[Operand<'a>]) -> Result<()> {
        let imm = if spec.imm {
            Some(operands[0].imm()?)
        } else {
            None
        };
        let operands = &operands[usize::from(spec.imm)..];
        let src = operands[0];
        let dst = operands[operands.len() - 1];
        if spec.vex {
            let first = if spec.binary {
                operands[1].reg()?
            } else {
                0
            };
            let long = src.ymm() || dst.ymm();
            self.vex(spec.pp, spec.map, long, first, spec.opcode, dst.reg()?, src)?;
        } else {
            let prefix: &[u8] = match spec.pp {
                | 0 => &[],
                | 1 => &[0x66],
                | 2 => &[0xF3],
                | _ => &[0xF2],
            };
            let opcode: &[u8] = match spec.map {
                | 1 => &[0x0F, spec.opcode],
                | 2 => &[0x0F, 0x38, spec.opcode],
                | _ => &[0x0F, 0x3A, spec.opcode],
            };
            self.rm(prefix, false, false, opcode, dst.reg()?, src)?;
        }
        if let Some(imm) = imm {
            self.integer(imm, 1)?;
        }
        Ok(())
    }

    /// `vmovups`: a load (`b` the register) or a store of 16 or 32 bytes.
    pub(super) fn vector_move(&mut self, a: Operand<'a>, b: Operand<'a>) -> Result<()> {
        let load = matches!(b, Operand::Reg(_));
        let (reg, mem) = if load { (b, a) } else { (a, b) };
        let opcode = if load { 0x10 } else { 0x11 };
        self.vex(0, 1, reg.ymm(), 0, opcode, reg.reg()?, mem)
    }

    /// A VEX instruction: the two-byte prefix `C5` when the map is `0F` and
    /// the r/m operand needs neither `X` nor `B` (W is always 0 here), else
    /// the three-byte `C4`; then the opcode and the operands as
    /// [`Code::rm`] writes them. `R`, `X`, `B` and `vvvv` are inverted.
    pub(super) fn vex(
        &mut self,
        pp: u8,
        map: u8,
        long: bool,
        vvvv: u8,
        opcode: u8,
        reg: u8,
        rm: Operand<'a>,
    ) -> Result<()> {
        let (base, index) = match rm {
            | Operand::Reg(r) => (r.id, 0),
            | Operand::Mem {
                segment: None,
                base,
                index,
                ..
            } => (base.unwrap_or(0), index.unwrap_or(0)),
            | _ => return Err(Error("expected register or memory")),
        };
        let r = (!reg >> 3) & 1;
        let x = (!index >> 3) & 1;
        let b = (!base >> 3) & 1;
        let tail = ((!vvvv & 15) << 3) | (u8::from(long) << 2) | pp;
        if map == 1 && x == 1 && b == 1 {
            self.raw(&[0xC5, (r << 7) | tail])?;
        } else {
            self.raw(&[0xC4, (r << 7) | (x << 6) | (b << 5) | map, tail])?;
        }
        self.byte(opcode)?;
        self.modrm(reg, rm, false)
    }
}
