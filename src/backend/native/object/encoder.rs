//! x86-64 v1 encodings for the native emitter (Intel SDM volume 2).
//!
//! `instruction` selects an opcode; `Code::rm` owns REX, `ModRM`, SIB and
//! displacements. Fixups retain the instruction-end bias, including any
//! immediate after the displacement. Branches are relaxed by the parent.

use super::{
    Error,
    Result,
    listing::{
        number,
        split,
    },
};

pub(super) fn instruction<'a>(name: &str, args: &'a str) -> Result<Code<'a>> {
    let mut code = Code {
        bytes: [0; 15],
        len:   0,
        fixup: None,
    };
    let [a, b, d] = operands(name, args)?;
    match name {
        | "retq" | "callq" | "jmpq" => code.control_transfer(name, args, a)?,
        | "ud2" => code.raw(&[0x0F, 0x0B])?,
        | "int3" => code.byte(0xCC)?,
        | "cltd" => code.byte(0x99)?,
        | "cqto" => code.raw(&[0x48, 0x99])?,
        | "rep" => code.raw(match args {
            | "movsb" => &[0xF3, 0xA4],
            | "stosb" => &[0xF3, 0xAA],
            | _ => return Err(Error("unknown repeat instruction")),
        })?,
        | "pushq" | "popq" => {
            let r = a.reg()?;
            code.rex(false, 0, 0, r, false)?;
            code.byte((if name == "pushq" { 0x50 } else { 0x58 }) + (r & 7))?;
        },
        | "movabsq" => {
            let r = b.reg()?;
            code.rex(true, 0, 0, r, false)?;
            code.byte(0xB8 + (r & 7))?;
            code.integer(a.imm()?, 8)?;
        },
        | "movq" | "movd" if a.xmm() || b.xmm() => {
            if a.xmm() && b.xmm() {
                return Err(Error("expected integer/vector register transfer"));
            }
            let (reg, rm, opcode) = if b.xmm() {
                (b.reg()?, a, 0x6E)
            } else {
                (a.reg()?, b, 0x7E)
            };
            code.rm(&[0x66], name == "movq", false, &[0x0F, opcode], reg, rm)?;
        },
        | "movb" | "movw" | "movl" | "movq" => {
            let width = width(name)?;
            let prefix = if width == 16 { &[0x66][..] } else { &[][..] };
            if let Operand::Imm(value) = a {
                if let Operand::Reg(r) = b {
                    if width == 64 {
                        code.rm(prefix, true, false, &[0xC7], 0, b)?;
                    } else {
                        code.raw(prefix)?;
                        code.rex(false, 0, 0, r.id, width == 8 && r.id >= 4)?;
                        code.byte((if width == 8 { 0xB0 } else { 0xB8 }) + (r.id & 7))?;
                    }
                } else {
                    code.rm(
                        prefix,
                        width == 64,
                        width == 8,
                        &[if width == 8 { 0xC6 } else { 0xC7 }],
                        0,
                        b,
                    )?;
                }
                if width == 64 {
                    signed32(value)?;
                }
                code.integer(value, if width == 64 { 4 } else { width / 8 })?;
            } else if matches!(b, Operand::Reg(_)) {
                if let Operand::Reg(r) = a {
                    code.rm(
                        prefix,
                        width == 64,
                        width == 8,
                        &[if width == 8 { 0x88 } else { 0x89 }],
                        r.id,
                        b,
                    )?;
                } else {
                    code.rm(
                        prefix,
                        width == 64,
                        width == 8,
                        &[if width == 8 { 0x8A } else { 0x8B }],
                        b.reg()?,
                        a,
                    )?;
                }
            } else {
                code.rm(
                    prefix,
                    width == 64,
                    width == 8,
                    &[if width == 8 { 0x88 } else { 0x89 }],
                    a.reg()?,
                    b,
                )?;
            }
        },
        | "leaq" => code.rm(&[], true, false, &[0x8D], b.reg()?, a)?,
        | "movzbl" | "movzwl" | "movsbl" | "movswl" | "movsbq" | "movswq" | "movslq" => {
            let opcode = match name {
                | "movzbl" => 0xB6,
                | "movzwl" => 0xB7,
                | "movsbl" | "movsbq" => 0xBE,
                | "movswl" | "movswq" => 0xBF,
                | _ => 0x63,
            };
            let bytes = [0x0F, opcode];
            code.rm(
                &[],
                name.ends_with('q'),
                false,
                if name == "movslq" {
                    &bytes[1..]
                } else {
                    &bytes
                },
                b.reg()?,
                a,
            )?;
        },
        | "movaps" | "movups" | "movss" | "movsd" => {
            let prefix = match name {
                | "movss" => &[0xF3][..],
                | "movsd" => &[0xF2][..],
                | _ => &[][..],
            };
            let load = matches!(b, Operand::Reg(_));
            let opcode = match name {
                | "movaps" =>
                    if load {
                        0x28
                    } else {
                        0x29
                    },
                | _ =>
                    if load {
                        0x10
                    } else {
                        0x11
                    },
            };
            code.rm(
                prefix,
                false,
                false,
                &[0x0F, opcode],
                if load { b.reg()? } else { a.reg()? },
                if load { a } else { b },
            )?;
        },
        | "xorps" | "addss" | "addsd" | "subss" | "subsd" | "mulss" | "mulsd" | "divss"
        | "divsd" => {
            let prefix = if name == "xorps" {
                &[][..]
            } else if name.ends_with("sd") {
                &[0xF2][..]
            } else {
                &[0xF3][..]
            };
            let opcode = match &name[..3] {
                | "xor" => 0x57,
                | "add" => 0x58,
                | "sub" => 0x5C,
                | "mul" => 0x59,
                | _ => 0x5E,
            };
            code.rm(prefix, false, false, &[0x0F, opcode], b.reg()?, a)?;
        },
        | "ucomiss" | "ucomisd" => code.rm(
            if name == "ucomisd" { &[0x66] } else { &[] },
            false,
            false,
            &[0x0F, 0x2E],
            b.reg()?,
            a,
        )?,
        | "cvtsi2ssl" | "cvtsi2ssq" | "cvtsi2sdl" | "cvtsi2sdq" => code.rm(
            if name.starts_with("cvtsi2sd") {
                &[0xF2]
            } else {
                &[0xF3]
            },
            name.ends_with('q'),
            false,
            &[0x0F, 0x2A],
            b.reg()?,
            a,
        )?,
        | "cvttss2si" | "cvttsd2si" => code.rm(
            if name == "cvttsd2si" {
                &[0xF2]
            } else {
                &[0xF3]
            },
            b.bits()? == 64,
            false,
            &[0x0F, 0x2C],
            b.reg()?,
            a,
        )?,
        | "cvtss2sd" | "cvtsd2ss" => code.rm(
            if name == "cvtsd2ss" { &[0xF2] } else { &[0xF3] },
            false,
            false,
            &[0x0F, 0x5A],
            b.reg()?,
            a,
        )?,
        | "flds" | "fldl" | "fldt" | "filds" | "fildl" | "fildll" | "fstps" | "fstpl" | "fstpt"
        | "fistps" | "fistpl" | "fistpll" | "fnstcw" | "fldcw" => {
            let (opcode, ext) = match name {
                | "flds" => (0xD9, 0),
                | "fldl" => (0xDD, 0),
                | "fldt" => (0xDB, 5),
                | "filds" => (0xDF, 0),
                | "fildl" => (0xDB, 0),
                | "fildll" => (0xDF, 5),
                | "fstps" => (0xD9, 3),
                | "fstpl" => (0xDD, 3),
                | "fstpt" => (0xDB, 7),
                | "fistps" => (0xDF, 3),
                | "fistpl" => (0xDB, 3),
                | "fistpll" => (0xDF, 7),
                | "fnstcw" => (0xD9, 7),
                | _ => (0xD9, 5),
            };
            code.rm(&[], false, false, &[opcode], ext, a)?;
        },
        | "fadd" | "fsub" | "fmul" | "fdiv" => {
            let source = a.st()?;
            let dest = b.st()?;
            code.raw(&[
                if dest == 0 { 0xD8 } else { 0xDC },
                (match name {
                    | "fadd" => 0xC0,
                    | "fmul" => 0xC8,
                    | "fsub" => 0xE0,
                    | _ => 0xF0,
                }) + if dest == 0 { source } else { dest },
            ])?;
        },
        | "fstp" => code.raw(&[0xDD, 0xD8 + a.st()?])?,
        | "fucomip" => code.raw(&[0xDF, 0xE8 + a.st()?])?,
        | name if name.starts_with("set") => code.rm(
            &[],
            false,
            true,
            &[0x0F, 0x90 + condition(&name[3..])?],
            0,
            a,
        )?,
        | name if name.starts_with("cmov") => {
            let cond = condition(&name[4..name.len() - 1])?;
            code.rm(
                if width(name)? == 16 { &[0x66] } else { &[] },
                width(name)? == 64,
                false,
                &[0x0F, 0x40 + cond],
                b.reg()?,
                a,
            )?;
        },
        | name if name.starts_with("imul") => {
            let bits = width(name)?;
            if let Operand::Imm(imm) = a {
                let imm = signed_immediate(imm, bits)?;
                let small = i8::try_from(imm).is_ok();
                code.rm(
                    if bits == 16 { &[0x66] } else { &[] },
                    bits == 64,
                    false,
                    &[if small { 0x6B } else { 0x69 }],
                    d.reg()?,
                    b,
                )?;
                code.integer(
                    imm,
                    if small {
                        1
                    } else if bits == 16 {
                        2
                    } else {
                        4
                    },
                )?;
            } else {
                code.rm(
                    if bits == 16 { &[0x66] } else { &[] },
                    bits == 64,
                    false,
                    &[0x0F, 0xAF],
                    b.reg()?,
                    a,
                )?;
            }
        },
        | name if name.starts_with("shld") || name.starts_with("shrd") => {
            let left = name.starts_with("shld");
            let immediate = matches!(a, Operand::Imm(_));
            code.rm(
                if width(name)? == 16 { &[0x66] } else { &[] },
                width(name)? == 64,
                false,
                &[
                    0x0F,
                    match (left, immediate) {
                        | (true, true) => 0xA4,
                        | (true, false) => 0xA5,
                        | (false, true) => 0xAC,
                        | _ => 0xAD,
                    },
                ],
                b.reg()?,
                d,
            )?;
            if immediate {
                code.integer(a.imm()?, 1)?;
            }
        },
        | name if name.starts_with("bswap") => {
            let r = a.reg()?;
            code.rex(width(name)? == 64, 0, 0, r, false)?;
            code.raw(&[0x0F, 0xC8 + (r & 7)])?;
        },
        | name if name.starts_with("bsr") || name.starts_with("bsf") => code.rm(
            if width(name)? == 16 { &[0x66] } else { &[] },
            width(name)? == 64,
            false,
            &[0x0F, if name.starts_with("bsr") { 0xBD } else { 0xBC }],
            b.reg()?,
            a,
        )?,
        | name if ["shl", "shr", "sar", "rol"]
            .iter()
            .any(|prefix| name.starts_with(prefix)) =>
        {
            let bits = width(name)?;
            let ext = match &name[..3] {
                | "shl" => 4,
                | "shr" => 5,
                | "sar" => 7,
                | _ => 0,
            };
            let immediate = matches!(a, Operand::Imm(_));
            let one = matches!(a, Operand::Imm(1));
            code.rm(
                if bits == 16 { &[0x66] } else { &[] },
                bits == 64,
                bits == 8,
                &[if one {
                    if bits == 8 { 0xD0 } else { 0xD1 }
                } else if immediate {
                    if bits == 8 { 0xC0 } else { 0xC1 }
                } else if bits == 8 {
                    0xD2
                } else {
                    0xD3
                }],
                ext,
                b,
            )?;
            if immediate && !one {
                code.integer(a.imm()?, 1)?;
            }
        },
        | name if ["neg", "idiv", "div", "mul"]
            .iter()
            .any(|prefix| name.starts_with(prefix)) =>
        {
            let bits = width(name)?;
            let ext = if name.starts_with("neg") {
                3
            } else if name.starts_with("idiv") {
                7
            } else if name.starts_with("div") {
                6
            } else {
                4
            };
            code.rm(
                if bits == 16 { &[0x66] } else { &[] },
                bits == 64,
                bits == 8,
                &[if bits == 8 { 0xF6 } else { 0xF7 }],
                ext,
                a,
            )?;
        },
        | name => {
            let bits = width(name)?;
            let stem = &name[..name.len() - 1];
            let ext = match stem {
                | "add" => 0,
                | "or" => 1,
                | "adc" => 2,
                | "sbb" => 3,
                | "and" => 4,
                | "sub" => 5,
                | "xor" => 6,
                | "cmp" => 7,
                | "test" => 8,
                | _ => return Err(Error("unknown machine instruction")),
            };
            let prefix = if bits == 16 { &[0x66][..] } else { &[][..] };
            if let Operand::Imm(imm) = a {
                let imm = signed_immediate(imm, bits)?;
                if ext == 8 {
                    return Err(Error("immediate test is not in emitter vocabulary"));
                }
                let small = bits != 8 && i8::try_from(imm).is_ok();
                if !small && matches!(b, Operand::Reg(Register { id: 0, .. })) {
                    code.raw(prefix)?;
                    code.rex(bits == 64, 0, 0, 0, false)?;
                    code.byte(ext * 8 + if bits == 8 { 4 } else { 5 })?;
                } else {
                    code.rm(
                        prefix,
                        bits == 64,
                        bits == 8,
                        &[if bits == 8 {
                            0x80
                        } else if small {
                            0x83
                        } else {
                            0x81
                        }],
                        ext,
                        b,
                    )?;
                }
                code.integer(
                    imm,
                    if small || bits == 8 {
                        1
                    } else if bits == 16 {
                        2
                    } else {
                        4
                    },
                )?;
            } else if ext == 8 {
                code.rm(
                    prefix,
                    bits == 64,
                    bits == 8,
                    &[if bits == 8 { 0x84 } else { 0x85 }],
                    a.reg()?,
                    b,
                )?;
            } else if let Operand::Reg(r) = a {
                code.rm(
                    prefix,
                    bits == 64,
                    bits == 8,
                    &[ext * 8 + u8::from(bits != 8)],
                    r.id,
                    b,
                )?;
            } else {
                code.rm(
                    prefix,
                    bits == 64,
                    bits == 8,
                    &[ext * 8 + if bits == 8 { 2 } else { 3 }],
                    b.reg()?,
                    a,
                )?;
            }
        },
    }
    if let Some(fixup) = &mut code.fixup
        && matches!(fixup.kind, 2 | 4 | 9 | 22 | 41 | 42)
    {
        fixup.bias = i64::from(fixup.at) - i64::from(code.len);
    }
    Ok(code)
}

fn operands<'a>(name: &str, args: &'a str) -> Result<[Operand<'a>; 3]> {
    if !name.is_ascii() {
        return Err(Error("unknown machine instruction"));
    }
    let mut operands = [Operand::Empty; 3];
    let mut count = 0;
    if name != "rep" && !args.is_empty() {
        for (i, arg) in split(args).enumerate() {
            if i >= 3 {
                return Err(Error("too many instruction operands"));
            }
            if arg.starts_with('*') && (i != 0 || !matches!(name, "callq" | "jmpq")) {
                return Err(Error("unsupported indirect operand"));
            }
            operands[i] = if name == "callq" && !args.starts_with('*') {
                _ = super::listing::expression(arg.strip_suffix("@PLT").unwrap_or(arg))?;
                Operand::Label
            } else {
                operand(arg.strip_prefix('*').unwrap_or(arg))?
            };
            count += 1;
        }
    }
    validate(name, args, &operands[..count])?;
    Ok(operands)
}

// Keep a closed table of operand forms in front of opcode selection. A new
// emitter mnemonic must specify arity, register class and width here first.
fn validate(name: &str, args: &str, operands: &[Operand<'_>]) -> Result<()> {
    use Shape::{
        Count,
        Imm,
        Label,
        Mem,
        Reg,
        Rm,
        St,
        Xm,
        Xmm,
    };
    let form = |shapes: &[Shape]| -> Result<()> {
        if shapes.len() != operands.len()
            || !shapes
                .iter()
                .zip(operands)
                .all(|(shape, op)| shape.accepts(*op))
        {
            return Err(Error(
                "unsupported instruction operand count, kind or width",
            ));
        }
        Ok(())
    };
    let a = operands.first().copied().unwrap_or(Operand::Empty);
    let b = operands.get(1).copied().unwrap_or(Operand::Empty);
    match name {
        | "rep" =>
            if matches!(args, "movsb" | "stosb") {
                Ok(())
            } else {
                Err(Error("unknown repeat instruction"))
            },
        | "retq" =>
            if operands.is_empty() {
                form(&[])
            } else {
                form(&[Imm])?;
                _ = u16::try_from(a.imm()?)
                    .map_err(|_| Error("return adjustment exceeds 16 bits"))?;
                Ok(())
            },
        | "ud2" | "int3" | "cltd" | "cqto" => form(&[]),
        | "callq" | "jmpq" =>
            if args.starts_with('*') {
                form(&[Rm(64)])
            } else if name == "callq" {
                form(&[Label])
            } else {
                Err(Error("unsupported direct jump"))
            },
        | "pushq" | "popq" => form(&[Reg(64)]),
        | "movabsq" => form(&[Imm, Reg(64)]),
        | "movd" | "movq" if a.xmm() || b.xmm() => {
            let bits = if name == "movq" { 64 } else { 32 };
            if a.xmm() {
                form(&[Xmm, Reg(bits)])
            } else {
                form(&[Reg(bits), Xmm])
            }
        },
        | "movb" | "movw" | "movl" | "movq" => {
            let bits = width(name)?;
            if matches!(a, Operand::Imm(_)) {
                form(&[Imm, Rm(bits)])
            } else if matches!(b, Operand::Reg(_)) {
                form(&[Rm(bits), Reg(bits)])
            } else {
                form(&[Reg(bits), Mem])
            }
        },
        | "leaq" => form(&[Mem, Reg(64)]),
        | "movzbl" | "movzwl" | "movsbl" | "movswl" | "movsbq" | "movswq" | "movslq" => {
            let source = match name.as_bytes()[4] {
                | b'b' => 8,
                | b'w' => 16,
                | _ => 32,
            };
            form(&[Rm(source), Reg(width(name)?)])
        },
        | "movaps" | "movups" | "movss" | "movsd" =>
            if b.xmm() {
                form(&[Xm, Xmm])
            } else {
                form(&[Xmm, Mem])
            },
        | "xorps" | "addss" | "addsd" | "subss" | "subsd" | "mulss" | "mulsd" | "divss"
        | "divsd" | "ucomiss" | "ucomisd" | "cvtss2sd" | "cvtsd2ss" => form(&[Xm, Xmm]),
        | "cvtsi2ssl" | "cvtsi2ssq" | "cvtsi2sdl" | "cvtsi2sdq" => form(&[Rm(width(name)?), Xmm]),
        | "cvttss2si" | "cvttsd2si" => {
            if !matches!(b.bits()?, 32 | 64) {
                return Err(Error("unsupported integer conversion width"));
            }
            form(&[Xm, Reg(b.bits()?)])
        },
        | "flds" | "fldl" | "fldt" | "filds" | "fildl" | "fildll" | "fstps" | "fstpl" | "fstpt"
        | "fistps" | "fistpl" | "fistpll" | "fnstcw" | "fldcw" => form(&[Mem]),
        | "fstp" => form(&[St]),
        | "fucomip" => {
            form(&[St, St])?;
            if b.st()? != 0 {
                return Err(Error("x87 comparison requires st0 destination"));
            }
            Ok(())
        },
        | "fadd" | "fmul" | "fsub" | "fdiv" => {
            form(&[St, St])?;
            if a.st()? != 0 && b.st()? != 0 {
                return Err(Error("x87 arithmetic requires st0"));
            }
            // Reverse subtract/divide spellings are outside the private
            // listing.
            if b.st()? != 0 && matches!(name, "fsub" | "fdiv") {
                return Err(Error("unsupported reversed x87 arithmetic"));
            }
            Ok(())
        },
        | name if name.starts_with("set") => {
            _ = condition(&name[3..])?;
            form(&[Rm(8)])
        },
        | name if name.starts_with("cmov") && name.len() > 5 => {
            _ = condition(&name[4..name.len() - 1])?;
            let bits = width(name)?;
            if bits == 8 {
                return Err(Error("invalid conditional move width"));
            }
            form(&[Rm(bits), Reg(bits)])
        },
        | _ => {
            let bits = width(name)?;
            let stem = &name[..name.len() - 1];
            match stem {
                | "imul" if bits != 8 =>
                    if matches!(a, Operand::Imm(_)) {
                        form(&[Imm, Rm(bits), Reg(bits)])
                    } else {
                        form(&[Rm(bits), Reg(bits)])
                    },
                | "shld" | "shrd" if bits != 8 => form(&[Count, Reg(bits), Rm(bits)]),
                | "shl" | "shr" | "sar" | "rol" => form(&[Count, Rm(bits)]),
                | "neg" | "idiv" | "div" | "mul" => form(&[Rm(bits)]),
                | "bswap" if matches!(bits, 32 | 64) => form(&[Reg(bits)]),
                | "bsr" | "bsf" if bits != 8 => form(&[Rm(bits), Reg(bits)]),
                | "add" | "or" | "adc" | "sbb" | "and" | "sub" | "xor" | "cmp" | "test" => {
                    if matches!(a, Operand::Imm(_)) {
                        form(&[Imm, Rm(bits)])
                    } else if matches!(a, Operand::Reg(_)) {
                        form(&[Reg(bits), Rm(bits)])
                    } else {
                        form(&[Mem, Reg(bits)])
                    }
                },
                | _ => Err(Error("unknown machine instruction or width")),
            }
        },
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Reg(u8),
    Rm(u8),
    Mem,
    Imm,
    Label,
    Xmm,
    Xm,
    St,
    Count,
}

impl Shape {
    fn accepts(self, op: Operand<'_>) -> bool {
        match self {
            | Self::Reg(bits) =>
                matches!(op, Operand::Reg(Register { bits: n, xmm: false, .. }) if n == bits),
            | Self::Rm(bits) =>
                matches!(op, Operand::Mem { .. })
                    || matches!(op, Operand::Reg(Register { bits: n, xmm: false, .. }) if n == bits),
            | Self::Mem => matches!(op, Operand::Mem { .. }),
            | Self::Imm => matches!(op, Operand::Imm(_)),
            | Self::Label => matches!(op, Operand::Label),
            | Self::Xmm => op.xmm(),
            | Self::Xm => op.xmm() || matches!(op, Operand::Mem { .. }),
            | Self::St => matches!(op, Operand::St(_)),
            | Self::Count => matches!(
                op,
                Operand::Imm(0..=255)
                    | Operand::Reg(Register {
                        id:   1,
                        bits: 8,
                        xmm:  false,
                    })
            ),
        }
    }
}

fn signed32(value: i64) -> Result<()> {
    i32::try_from(value)
        .map(|_| ())
        .map_err(|_| Error("value exceeds signed 32-bit encoding field"))
}

// Validate before truncating, then select sign-extending imm8 forms from the
// operand-width bit pattern (e.g. 4294967295 is -1 for a 32-bit operation).
fn signed_immediate(value: i64, bits: u8) -> Result<i64> {
    if bits == 64 {
        signed32(value)?;
    } else if i128::from(value) < -(1_i128 << (bits - 1)) || i128::from(value) >= (1_i128 << bits) {
        return Err(Error("integer does not fit operand width"));
    }
    let shift = 64 - bits;
    Ok((value << shift) >> shift)
}

#[derive(Clone, Copy)]
pub(super) struct Code<'a> {
    pub(super) bytes: [u8; 15],
    pub(super) len:   u8,
    pub(super) fixup: Option<Fixup<'a>>,
}

#[derive(Clone, Copy)]
pub(super) struct Fixup<'a> {
    pub(super) at:         u8,
    pub(super) expression: &'a str,
    pub(super) kind:       u32,
    pub(super) bias:       i64,
}

impl<'a> Code<'a> {
    fn rm(
        &mut self,
        prefix: &[u8],
        wide: bool,
        byte: bool,
        opcode: &[u8],
        reg: u8,
        rm: Operand<'a>,
    ) -> Result<()> {
        let (base, index) = match rm {
            | Operand::Reg(r) => (Some(r.id), None),
            | Operand::Mem { base, index, .. } => (base, index),
            | _ => return Err(Error("expected register or memory")),
        };
        if let Operand::Mem {
            segment: Some(segment),
            ..
        } = rm
        {
            self.byte(segment)?;
        }
        self.raw(prefix)?;
        let group = opcode.len() == 1
            && matches!(
                opcode[0],
                0x80 | 0x81
                    | 0x83
                    | 0xC0
                    | 0xC1
                    | 0xC6
                    | 0xC7
                    | 0xD0
                    | 0xD1
                    | 0xD2
                    | 0xD3
                    | 0xF6
                    | 0xF7
                    | 0xFF
            );
        let force = byte && !group && reg >= 4
            || matches!(
                rm,
                Operand::Reg(Register {
                    bits: 8,
                    id: 4..=7,
                    ..
                })
            );
        let before_rex = self.len;
        self.rex(wide, reg, index.unwrap_or(0), base.unwrap_or(0), force)?;
        let has_rex = self.len != before_rex;
        self.raw(opcode)?;
        if let Operand::Reg(r) = rm {
            self.byte(0xC0 | ((reg & 7) << 3) | (r.id & 7))?;
            return Ok(());
        }
        let Operand::Mem {
            base,
            index,
            scale,
            disp,
            rip,
            ..
        } = rm
        else {
            return Err(Error("expected memory"));
        };
        let (expr, kind) = if let Some(expr) = disp.strip_suffix("@TPOFF") {
            (expr, 23)
        } else if let Some(expr) = disp.strip_suffix("@GOTTPOFF") {
            if !rip {
                return Err(Error("GOTTPOFF requires RIP-relative addressing"));
            }
            (expr, 22)
        } else if let Some(expr) = disp.strip_suffix("@GOTPCREL") {
            if !rip {
                return Err(Error("GOTPCREL requires RIP-relative addressing"));
            }
            (expr, if has_rex { 42 } else { 41 })
        } else {
            (disp, if rip { 2 } else { 11 })
        };
        if expr.contains('@') {
            return Err(Error("unsupported relocation suffix"));
        }
        if !expr.is_empty() {
            let parsed = super::listing::expression(expr)?;
            if parsed.subtract.is_some() || disp.contains('@') && parsed.symbol.is_none() {
                return Err(Error("unsupported displacement expression"));
            }
        }
        let constant = if expr.is_empty() {
            Some(0)
        } else {
            number(expr).ok()
        };
        let mode =
            if rip || base.is_none() || constant == Some(0) && base.is_some_and(|b| b & 7 != 5) {
                0
            } else if constant.is_some_and(|n| i8::try_from(n).is_ok()) {
                1
            } else {
                2
            };
        let sib = !rip && (index.is_some() || base.is_none() || base.is_some_and(|r| r & 7 == 4));
        self.byte(
            (mode << 6)
                | ((reg & 7) << 3)
                | if rip {
                    5
                } else if sib {
                    4
                } else {
                    base.ok_or(Error("missing base register"))? & 7
                },
        )?;
        if sib {
            self.byte(
                (scale << 6) | (index.map_or(4, |r| r & 7) << 3) | base.map_or(5, |r| r & 7),
            )?;
        }
        let bytes = if rip || base.is_none() || mode == 2 {
            4
        } else {
            u8::from(mode == 1)
        };
        if let Some(value) = constant {
            signed32(value)?;
            self.integer(value, bytes)?;
        } else {
            if bytes != 4 {
                return Err(Error("symbolic displacement is not 32 bits"));
            }
            self.fixup = Some(Fixup {
                at: self.len,
                expression: expr,
                kind,
                bias: 0,
            });
            self.integer(0, 4)?;
        }
        Ok(())
    }

    fn control_transfer(&mut self, name: &str, args: &'a str, a: Operand<'a>) -> Result<()> {
        match name {
            | "retq" =>
                if matches!(a, Operand::Empty) {
                    self.byte(0xC3)?;
                } else {
                    self.byte(0xC2)?;
                    self.integer(a.imm()?, 2)?;
                },
            | "callq" | "jmpq" =>
                if args.starts_with('*') {
                    self.rm(
                        &[],
                        false,
                        false,
                        &[0xFF],
                        if name == "callq" { 2 } else { 4 },
                        a,
                    )?;
                } else if name == "callq" {
                    let target =
                        super::listing::expression(args.strip_suffix("@PLT").unwrap_or(args))?;
                    if target.symbol.is_none() || target.subtract.is_some() || target.addend != 0 {
                        return Err(Error("unsupported call target expression"));
                    }
                    self.byte(0xE8)?;
                    self.fixup = Some(Fixup {
                        at:         self.len,
                        expression: args,
                        // LLVM uses PC32 even for a binary expression whose
                        // addend is zero; a plain symbol uses PLT32.
                        kind:       if super::listing::symbol_name(
                            args.strip_suffix("@PLT").unwrap_or(args),
                        )
                        .is_ok()
                        {
                            4
                        } else {
                            2
                        },
                        bias:       0,
                    });
                    self.integer(0, 4)?;
                } else {
                    return Err(Error("unknown direct jump"));
                },
            | _ => return Err(Error("unknown control transfer")),
        }
        Ok(())
    }

    fn rex(&mut self, wide: bool, reg: u8, index: u8, base: u8, force: bool) -> Result<()> {
        let rex =
            0x40 | (u8::from(wide) << 3) | ((reg >> 3) << 2) | ((index >> 3) << 1) | (base >> 3);
        if rex != 0x40 || force {
            self.byte(rex)?;
        }
        Ok(())
    }

    fn byte(&mut self, value: u8) -> Result<()> {
        let at = usize::from(self.len);
        *self
            .bytes
            .get_mut(at)
            .ok_or(Error("instruction exceeds 15 bytes"))? = value;
        self.len += 1;
        Ok(())
    }

    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        for &byte in bytes {
            self.byte(byte)?;
        }
        Ok(())
    }

    fn integer(&mut self, value: i64, bytes: u8) -> Result<()> {
        if bytes < 8 {
            let bits = u32::from(bytes) * 8;
            let value = i128::from(value);
            if value < -(1_i128 << bits.saturating_sub(1)) || value >= (1_i128 << bits) {
                return Err(Error("integer does not fit encoding field"));
            }
        }
        self.raw(&value.to_le_bytes()[..usize::from(bytes)])
    }
}

#[derive(Clone, Copy)]
enum Operand<'a> {
    Empty,
    Reg(Register),
    Imm(i64),
    Mem {
        base:    Option<u8>,
        index:   Option<u8>,
        scale:   u8,
        disp:    &'a str,
        rip:     bool,
        segment: Option<u8>,
    },
    Label,
    St(u8),
}

#[derive(Clone, Copy)]
struct Register {
    id:   u8,
    bits: u8,
    xmm:  bool,
}

impl Operand<'_> {
    fn st(self) -> Result<u8> {
        if let Self::St(index) = self {
            Ok(index)
        } else {
            Err(Error("expected x87 register"))
        }
    }

    fn reg(self) -> Result<u8> {
        if let Self::Reg(r) = self {
            Ok(r.id)
        } else {
            Err(Error("expected register"))
        }
    }

    fn bits(self) -> Result<u8> {
        if let Self::Reg(r) = self {
            Ok(r.bits)
        } else {
            Err(Error("expected register width"))
        }
    }

    fn xmm(self) -> bool {
        matches!(self, Self::Reg(Register { xmm: true, .. }))
    }

    fn imm(self) -> Result<i64> {
        if let Self::Imm(value) = self {
            Ok(value)
        } else {
            Err(Error("expected immediate"))
        }
    }
}

fn operand(text: &str) -> Result<Operand<'_>> {
    if let Some(imm) = text.strip_prefix('$') {
        return Ok(Operand::Imm(number(imm)?));
    }
    if let Some(disp) = text.strip_prefix("%fs:") {
        return Ok(Operand::Mem {
            base: None,
            index: None,
            scale: 0,
            disp,
            rip: false,
            segment: Some(0x64),
        });
    }
    if text == "%st" {
        return Ok(Operand::St(0));
    }
    if let Some(index) = text.strip_prefix("%st(").and_then(|s| s.strip_suffix(')')) {
        let index: u8 = index.parse().map_err(|_| Error("invalid x87 register"))?;
        if index > 7 {
            return Err(Error("invalid x87 register"));
        }
        return Ok(Operand::St(index));
    }
    if text.starts_with('%') {
        return Ok(Operand::Reg(register(text)?));
    }
    if let Some(at) = text.rfind('(') {
        let disp = &text[..at];
        let inside = text[at + 1..]
            .strip_suffix(')')
            .ok_or(Error("invalid memory operand"))?;
        let mut parts = split(inside);
        let base = parts.next().unwrap_or("");
        let rip = base == "%rip";
        let base = if base.is_empty() || rip {
            None
        } else {
            let r = register(base)?;
            if r.bits != 64 || r.xmm {
                return Err(Error("expected 64-bit address register"));
            }
            Some(r.id)
        };
        let index = parts
            .next()
            .filter(|s| !s.is_empty())
            .map(register)
            .transpose()?
            .map(|r| {
                if r.bits != 64 || r.xmm {
                    return Err(Error("expected 64-bit index register"));
                }
                Ok(r.id)
            })
            .transpose()?;
        if index == Some(4) || rip && index.is_some() {
            return Err(Error("invalid memory index register"));
        }
        let scale = match parts.next().unwrap_or("1") {
            | "1" => 0,
            | "2" => 1,
            | "4" => 2,
            | "8" => 3,
            | _ => return Err(Error("invalid index scale")),
        };
        if parts.next().is_some() || index.is_none() && scale != 0 {
            return Err(Error("unsupported memory operand"));
        }
        return Ok(Operand::Mem {
            base,
            index,
            scale,
            disp,
            rip,
            segment: None,
        });
    }
    _ = super::listing::expression(text)?;
    Ok(Operand::Label)
}

fn register(text: &str) -> Result<Register> {
    let name = text
        .strip_prefix('%')
        .ok_or(Error("expected register name"))?;
    if let Some(n) = name.strip_prefix("xmm") {
        let id: u8 = n.parse().map_err(|_| Error("invalid XMM register"))?;
        return if id < 16 {
            Ok(Register {
                id,
                bits: 128,
                xmm: true,
            })
        } else {
            Err(Error("XMM register outside baseline ISA"))
        };
    }
    let names = [
        ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil"],
        ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"],
        ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"],
        ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"],
    ];
    for (w, row) in names.iter().enumerate() {
        if let Some(id) = row.iter().position(|&n| n == name) {
            return Ok(Register {
                id:   u8::try_from(id).map_err(|_| Error("register too large"))?,
                bits: 8 << w,
                xmm:  false,
            });
        }
    }
    if let Some(tail) = name.strip_prefix('r') {
        let (n, bits) = if let Some(n) = tail.strip_suffix('b') {
            (n, 8)
        } else if let Some(n) = tail.strip_suffix('w') {
            (n, 16)
        } else if let Some(n) = tail.strip_suffix('d') {
            (n, 32)
        } else {
            (tail, 64)
        };
        let id: u8 = n.parse().map_err(|_| Error("unknown general register"))?;
        if (8..16).contains(&id) {
            return Ok(Register {
                id,
                bits,
                xmm: false,
            });
        }
    }
    Err(Error("unknown register"))
}

fn width(name: &str) -> Result<u8> {
    match name.as_bytes().last() {
        | Some(b'b') => Ok(8),
        | Some(b'w') => Ok(16),
        | Some(b'l') => Ok(32),
        | Some(b'q') => Ok(64),
        | _ => Err(Error("missing instruction width")),
    }
}

pub(super) fn condition(name: &str) -> Result<u8> {
    Ok(match name {
        | "o" => 0,
        | "no" => 1,
        | "b" => 2,
        | "ae" => 3,
        | "e" => 4,
        | "ne" => 5,
        | "be" => 6,
        | "a" => 7,
        | "s" => 8,
        | "ns" => 9,
        | "p" => 10,
        | "np" => 11,
        | "l" => 12,
        | "ge" => 13,
        | "le" => 14,
        | "g" => 15,
        | _ => return Err(Error("unknown condition code")),
    })
}

pub(super) fn nops(bytes: &mut [u8], offset: u64, mut count: u64) -> Result<()> {
    // LLVM's x86-64 baseline uses canonical multi-byte NOPs, up to ten
    // bytes (the longest includes operand-size and CS prefixes).
    const NOPS: [&[u8]; 11] = [
        &[],
        &[0x90],
        &[0x66, 0x90],
        &[0x0F, 0x1F, 0],
        &[0x0F, 0x1F, 0x40, 0],
        &[0x0F, 0x1F, 0x44, 0, 0],
        &[0x66, 0x0F, 0x1F, 0x44, 0, 0],
        &[0x0F, 0x1F, 0x80, 0, 0, 0, 0],
        &[0x0F, 0x1F, 0x84, 0, 0, 0, 0, 0],
        &[0x66, 0x0F, 0x1F, 0x84, 0, 0, 0, 0, 0],
        &[0x66, 0x2E, 0x0F, 0x1F, 0x84, 0, 0, 0, 0, 0],
    ];
    let mut at = usize::try_from(offset).map_err(|_| Error("code section too large"))?;
    while count > 0 {
        let n = usize::try_from(count.min(10)).map_err(|_| Error("NOP too large"))?;
        bytes[at..at + n].copy_from_slice(NOPS[n]);
        at += n;
        count -= n as u64;
    }
    Ok(())
}
