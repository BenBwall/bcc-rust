//! The allocator's view of every machine instruction form, against x86.
//!
//! [`MInst::visit_registers`] and [`MInst::flags`] are all the register
//! allocator and its checker know of what an instruction reads and writes.
//! The checker interprets the program and its rewrite with the same roles,
//! so it cannot see a wrong one: with `cmov`'s destination classified as
//! written only, the rewrite lets the destination share a scratch register
//! with the source, and the program silently computes something else. The
//! table here states, for each form, what the instruction the emitter
//! prints for it does with each register and with the flags, written from
//! the Intel SDM (Volume 2) rather than from the classification, and the
//! test requires the two to agree.
//!
//! Both [`x86`] and [`forms`] name every [`MInst`] variant, so a new
//! variant compiles only once it has an entry and sample forms.

use std::cell::Cell;

use crate::{
    backend::native::mir::{
        AluOp,
        Base,
        Callee,
        Cond,
        FrameObject,
        Index,
        MBlock,
        MInst,
        Mem,
        Operand,
        PReg,
        PRegSet,
        Reg,
        Role,
        ShiftAmount,
        ShiftOp,
        SymbolRef,
        UnaryOp,
        VReg,
        Width,
        X87Format,
        X87Op,
        XOp,
    },
    ir::Entity,
};

#[test]
fn every_form_reads_and_writes_what_x86_does() {
    let mut failures = Vec::new();
    for inst in forms() {
        let (registers, flags) = x86(inst);
        let mut expected: Vec<(Reg, Role)> = registers
            .iter()
            .map(|&(reg, access)| (reg, access.role()))
            .collect();
        let mut actual = Vec::new();
        let mut copy = inst;
        copy.visit_registers(|reg, role| actual.push((*reg, role)));
        expected.sort_unstable_by_key(|&(reg, role)| (reg, rank(role)));
        actual.sort_unstable_by_key(|&(reg, role)| (reg, rank(role)));
        if actual != expected {
            failures.push(format!(
                "{inst:?}\n  x86:            {registers:?}\n  classification: {actual:?}"
            ));
        }
        if inst.flags() != flags.reads_and_overwrites() {
            failures.push(format!(
                "{inst:?}\n  x86 flags: {flags:?}, classified (reads, overwrites): {:?}",
                inst.flags()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} forms are misclassified:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The two-address forms are the ones a misclassification breaks silently:
/// spot-check that the table itself says they read their destination.
#[test]
fn the_table_reads_the_destination_of_every_two_address_form() {
    for inst in forms() {
        let (MInst::Alu { dst, .. }
        | MInst::XAlu { dst, .. }
        | MInst::Shift { dst, .. }
        | MInst::Unary { dst, .. }
        | MInst::Cmov { dst, .. }
        | MInst::ShiftDouble { dst, .. }) = inst
        else {
            continue;
        };
        let (registers, _) = x86(inst);
        assert!(
            registers.contains(&(dst, Access::ReadWrite)),
            "{inst:?}: {registers:?}"
        );
    }
}

/// What an instruction does with a register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    Read,
    /// Every bit written, none read.
    Write,
    /// Read, then written.
    ReadWrite,
    /// Written in part, with the rest of the old value kept: the high bits
    /// of a narrow integer, the upper lanes of a scalar float. Nothing reads
    /// those (the machine IR's invariant), so the old value is dead and the
    /// classification is a write.
    Merge,
    /// A physical register the instruction may leave alone, or that the
    /// calling convention lets a callee change: classified as a write, which
    /// for a physical register is the safe side.
    Clobber,
}

impl Access {
    /// The role the allocator must give an operand with this access.
    const fn role(self) -> Role {
        match self {
            | Self::Read => Role::Use,
            | Self::ReadWrite => Role::UseDef,
            | Self::Write | Self::Merge | Self::Clobber => Role::Def,
        }
    }
}

/// [`Role`] has no order; the comparison sorts by this.
const fn rank(role: Role) -> u8 {
    match role {
        | Role::Use => 0,
        | Role::Def => 1,
        | Role::UseDef => 2,
    }
}

/// What an instruction does with the status flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flags {
    Untouched,
    Read,
    /// Every status flag written or left undefined.
    Written,
    ReadWritten,
    /// Some flags written and others kept, or all kept for a zero count.
    Partial,
}

impl Flags {
    /// [`MInst::flags`]: whether the instruction reads the flags, and
    /// whether it certainly overwrites all of them.
    const fn reads_and_overwrites(self) -> (bool, bool) {
        match self {
            | Self::Untouched | Self::Partial => (false, false),
            | Self::Read => (true, false),
            | Self::Written => (false, true),
            | Self::ReadWritten => (true, true),
        }
    }
}

/// The registers an instruction names, with their accesses.
#[derive(Default)]
struct Operands(Vec<(Reg, Access)>);

impl Operands {
    fn reg(&mut self, reg: Reg, access: Access) {
        self.0.push((reg, access));
    }

    fn phys(&mut self, register: PReg, access: Access) {
        self.reg(Reg::Phys(register), access);
    }

    /// An address computation reads its base and index.
    fn mem(&mut self, mem: Mem) {
        if let Base::Reg(base) = mem.base {
            self.reg(base, Access::Read);
        }
        if let Some(index) = mem.index {
            self.reg(index.reg, Access::Read);
        }
    }

    fn source(&mut self, operand: Operand) {
        match operand {
            | Operand::Reg(reg) => self.reg(reg, Access::Read),
            | Operand::Mem(mem) => self.mem(mem),
            | Operand::Imm(_) => {},
        }
    }
}

/// A write of an integer register at `width`: 32-bit results zero the upper
/// half, 8- and 16-bit ones keep it.
const fn integer_write(width: Width) -> Access {
    match width {
        | Width::B8 | Width::B16 => Access::Merge,
        | Width::B32 | Width::B64 => Access::Write,
    }
}

/// What the instruction the emitter prints for `inst` does with each
/// register `inst` names, implicit ones included, and with the flags. The
/// x87 register stack and `%rsp` are not allocated and not listed.
fn x86(inst: MInst) -> (Vec<(Reg, Access)>, Flags) {
    use Access::{
        Clobber,
        Merge,
        Read,
        ReadWrite,
        Write,
    };
    let mut at = Operands::default();
    let flags = match inst {
        // `mov`: a narrow move keeps the destination's upper bits.
        | MInst::Mov { width, dst, src } => {
            at.source(src);
            at.reg(dst, integer_write(width));
            Flags::Untouched
        },
        // `movabsq $imm, dst`; `movq %fs:0, dst` or `movq %gs:88, dst`;
        // `xorps dst, dst`, which is zero whatever dst held.
        | MInst::MovImm64 { dst, .. } | MInst::ThreadPointer { dst } | MInst::XZero { dst } => {
            at.reg(dst, Write);
            Flags::Untouched
        },
        | MInst::Store { dst, src, .. } => {
            at.mem(dst);
            at.source(src);
            Flags::Untouched
        },
        | MInst::Lea { dst, addr } => {
            at.mem(addr);
            at.reg(dst, Write);
            Flags::Untouched
        },
        // `movz*`/`movs*` write at least 32 bits; `movl` zero-extends.
        // `movaps` copies all 16 bytes; a `movss`/`movsd` load zeroes the
        // upper lanes.
        | MInst::Extend { dst, src, .. } | MInst::XMov { dst, src, .. } => {
            at.source(src);
            at.reg(dst, Write);
            Flags::Untouched
        },
        // `op src, dst`: dst ← dst op src. `adc`/`sbb` read CF; every
        // arithmetic and logic op sets or leaves undefined all six status
        // flags (two-operand `imul` defines CF and OF only).
        | MInst::Alu { op, dst, src, .. } => {
            at.source(src);
            at.reg(dst, ReadWrite);
            match op {
                | AluOp::Adc | AluOp::Sbb => Flags::ReadWritten,
                | _ => Flags::Written,
            }
        },
        // `imul $imm, src, dst`: dst ← src × imm, the old dst unread.
        | MInst::Imul3 {
            width, dst, src, ..
        } => {
            at.reg(src, Read);
            at.reg(dst, integer_write(width));
            Flags::Written
        },
        | MInst::Cmp { lhs, rhs, .. } => {
            at.reg(lhs, Read);
            at.source(rhs);
            Flags::Written
        },
        // `ucomis*` sets ZF, PF and CF and clears OF, SF and AF.
        | MInst::Test { lhs, rhs, .. } | MInst::Ucomi { lhs, rhs, .. } => {
            at.reg(lhs, Read);
            at.reg(rhs, Read);
            Flags::Written
        },
        // A shift masks its count to 5 bits (6 at 64 bits) and touches no
        // flag for a zero count; `%cl` may hold one. A rotate writes only CF
        // and OF.
        | MInst::Shift {
            op,
            width,
            dst,
            amount,
        } => {
            if amount == ShiftAmount::Cl {
                at.phys(PReg::Rcx, Read);
            }
            at.reg(dst, ReadWrite);
            let mask = if width == Width::B64 { 63 } else { 31 };
            match amount {
                | ShiftAmount::Imm(count) if count & mask == 0 => Flags::Untouched,
                | ShiftAmount::Imm(_) if op != ShiftOp::Rol => Flags::Written,
                | _ => Flags::Partial,
            }
        },
        | MInst::Unary { op, dst, .. } => {
            at.reg(dst, ReadWrite);
            match op {
                | UnaryOp::Neg => Flags::Written,
                | UnaryOp::Bswap => Flags::Untouched,
            }
        },
        // `bsf`/`bsr` set ZF for a zero source and leave the destination
        // undefined (Intel) or unchanged (AMD); the selector replaces that
        // result with a `cmove` and never reads it.
        | MInst::BitScan { dst, src, .. } => {
            at.reg(src, Read);
            at.reg(dst, Merge);
            Flags::Written
        },
        // `cltd`/`cqto`: %edx/%rdx ← the sign of %eax/%rax.
        | MInst::SignExtendAx { .. } => {
            at.phys(PReg::Rax, Read);
            at.phys(PReg::Rdx, Write);
            Flags::Untouched
        },
        // `div`/`idiv` of %edx:%eax or %rdx:%rax (the selector divides at
        // 32 or 64 bits only): quotient to %rax, remainder to %rdx. Every
        // flag is left undefined.
        | MInst::Div { divisor, .. } => {
            at.reg(divisor, Read);
            at.phys(PReg::Rax, ReadWrite);
            at.phys(PReg::Rdx, ReadWrite);
            Flags::Written
        },
        // `mul src`: the product of %al/%ax/%eax/%rax and src. An 8-bit
        // product goes to %ax and leaves %rdx alone; wider ones put the high
        // half in %dx/%edx/%rdx. CF and OF are defined, the rest undefined.
        | MInst::MulWide { width, src } => {
            at.reg(src, Read);
            at.phys(PReg::Rax, ReadWrite);
            at.phys(
                PReg::Rdx,
                match width {
                    | Width::B8 => Clobber,
                    | Width::B16 => Merge,
                    | Width::B32 | Width::B64 => Write,
                },
            );
            Flags::Written
        },
        // `setcc` writes the low byte only.
        | MInst::Setcc { dst, .. } => {
            at.reg(dst, Merge);
            Flags::Read
        },
        // `cmovcc src, dst`: dst keeps its value when the condition fails.
        | MInst::Cmov { dst, src, .. } => {
            at.reg(src, Read);
            at.reg(dst, ReadWrite);
            Flags::Read
        },
        // The convention passes arguments in `uses` and lets the callee
        // change `clobbers` and the status flags.
        | MInst::Call {
            callee,
            uses,
            clobbers,
        } => {
            if let Callee::Reg(target) = callee {
                at.reg(target, Read);
            }
            for register in uses.iter() {
                at.phys(register, Read);
            }
            for register in clobbers.iter() {
                at.phys(register, Clobber);
            }
            Flags::Written
        },
        | MInst::Jmp { .. } | MInst::X87Arith { .. } | MInst::X87Pop | MInst::Ud2 | MInst::Int3 =>
            Flags::Untouched,
        | MInst::Jcc { .. } => Flags::Read,
        | MInst::JmpIndirect { target, .. } => {
            at.reg(target, Read);
            Flags::Untouched
        },
        | MInst::TlsLea { dst, base, .. } => {
            at.reg(base, Read);
            at.reg(dst, Write);
            Flags::Untouched
        },
        | MInst::Move128 { load, reg, mem } => {
            at.mem(mem);
            at.reg(reg, if load { Write } else { Read });
            Flags::Untouched
        },
        | MInst::XStore { dst, src, .. } => {
            at.mem(dst);
            at.reg(src, Read);
            Flags::Untouched
        },
        // `addsd src, dst`: dst ← dst op src in the low lane; `xorps` on
        // all four.
        | MInst::XAlu { dst, src, .. } => {
            at.source(src);
            at.reg(dst, ReadWrite);
            Flags::Untouched
        },
        // `cvtsi2s*` and `cvts*2s*` write the low lane and keep the rest.
        | MInst::IntToFloat { dst, src, .. } | MInst::FloatToFloat { dst, src, .. } => {
            at.reg(src, Read);
            at.reg(dst, Merge);
            Flags::Untouched
        },
        // `cvtts*2si` writes a 32- or 64-bit integer register. `movd`/`movq`
        // to an SSE register zero the upper lanes; to a general one, `movd`
        // zero-extends.
        | MInst::FloatToInt { dst, src, .. } | MInst::MovGx { dst, src, .. } => {
            at.reg(src, Read);
            at.reg(dst, Write);
            Flags::Untouched
        },
        // `shld`/`shrd count, src, dst`: dst shifted, filled from src.
        // The count is masked to six bits. A nonzero masked count writes CF,
        // SF, ZF and PF, writes OF for a count of one and leaves it undefined
        // otherwise, and leaves AF undefined; a zero one touches no flag.
        | MInst::ShiftDouble {
            dst, src, amount, ..
        } => {
            if amount == ShiftAmount::Cl {
                at.phys(PReg::Rcx, Read);
            }
            at.reg(src, Read);
            at.reg(dst, ReadWrite);
            match amount {
                | ShiftAmount::Imm(count) if count % 64 == 0 => Flags::Untouched,
                | ShiftAmount::Imm(_) => Flags::Written,
                | ShiftAmount::Cl => Flags::Partial,
            }
        },
        // `rep movsb` advances %rsi and %rdi and counts %rcx down to zero.
        | MInst::RepMovsb => {
            at.phys(PReg::Rdi, ReadWrite);
            at.phys(PReg::Rsi, ReadWrite);
            at.phys(PReg::Rcx, ReadWrite);
            Flags::Untouched
        },
        // `rep stosb` stores %al.
        | MInst::RepStosb => {
            at.phys(PReg::Rdi, ReadWrite);
            at.phys(PReg::Rcx, ReadWrite);
            at.phys(PReg::Rax, Read);
            Flags::Untouched
        },
        | MInst::X87Load { mem, .. }
        | MInst::X87Store { mem, .. }
        | MInst::X87ControlWord { mem, .. } => {
            at.mem(mem);
            Flags::Untouched
        },
        // `fucomip` sets ZF, PF and CF like `ucomis*`.
        | MInst::X87Compare => Flags::Written,
        // The epilogue may or may not adjust %rsp before `ret`; nothing
        // follows it.
        | MInst::Ret { uses } => {
            for register in uses.iter() {
                at.phys(register, Read);
            }
            Flags::Untouched
        },
    };
    (at.0, flags)
}

/// Immediate shift counts on either side of the five- and six-bit masks the
/// CPU applies: a masked count of zero touches no flag.
const COUNTS: [u8; 9] = [0, 1, 31, 32, 33, 63, 64, 65, 255];

/// Samples of every form the selector emits: each variant with each
/// operation, width it selects at, kind of source operand, and implicit
/// register, every register a distinct virtual one.
fn forms() -> Vec<MInst> {
    let next = Cell::new(0);
    let v = || {
        next.set(next.get() + 1);
        Reg::Virt(VReg::new(next.get()))
    };
    let mem = || Mem {
        base:  Base::Reg(v()),
        index: Some(Index {
            reg:   v(),
            scale: 8,
        }),
        disp:  16,
    };
    let frame = Mem::new(Base::Frame(FrameObject::Spill(3)), 0);
    let sources = || [Operand::Reg(v()), Operand::Imm(7), Operand::Mem(mem())];
    let all = [Width::B8, Width::B16, Width::B32, Width::B64];
    let wide = [Width::B32, Width::B64];
    let conds = [
        Cond::E,
        Cond::Ne,
        Cond::L,
        Cond::Le,
        Cond::G,
        Cond::Ge,
        Cond::B,
        Cond::Be,
        Cond::A,
        Cond::Ae,
        Cond::O,
        Cond::No,
        Cond::S,
        Cond::Ns,
        Cond::P,
        Cond::Np,
    ];
    let formats = [
        X87Format::F32,
        X87Format::F64,
        X87Format::F80,
        X87Format::I16,
        X87Format::I32,
        X87Format::I64,
    ];
    let mut forms = Vec::new();
    // Lists each variant's samples, and names every variant in a match, so
    // that a new one fails to compile until it has samples here.
    macro_rules! variants {
        ($($variant:ident => $samples:expr,)+) => {
            let _exhaustive = |inst: MInst| match inst {
                $(MInst::$variant { .. } => {},)+
            };
            $(
                let start = forms.len();
                for sample in $samples {
                    let sample: MInst = sample;
                    assert!(
                        matches!(sample, MInst::$variant { .. }),
                        "{sample:?} is listed as {}",
                        stringify!($variant)
                    );
                    forms.push(sample);
                }
                assert!(forms.len() > start, "{} has no samples", stringify!($variant));
            )+
        };
    }
    variants! {
        Mov => all.into_iter().flat_map(|width| {
            sources().map(|src| MInst::Mov { width, dst: v(), src })
        }).chain([MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::Rax),
            src:   Operand::Mem(frame),
        }]).collect::<Vec<_>>(),
        MovImm64 => [MInst::MovImm64 { dst: v(), imm: -1 << 40 }],
        Store => all.into_iter().flat_map(|width| {
            [Operand::Reg(v()), Operand::Imm(7)]
                .map(|src| MInst::Store { width, dst: mem(), src })
        }).chain([MInst::Store {
            width: Width::B64,
            dst:   frame,
            src:   Operand::Reg(v()),
        }]).collect::<Vec<_>>(),
        Lea => [MInst::Lea { dst: v(), addr: mem() }, MInst::Lea { dst: v(), addr: frame }],
        Extend => [
            (false, Width::B8, Width::B32),
            (false, Width::B16, Width::B32),
            (false, Width::B32, Width::B64),
            (true, Width::B8, Width::B32),
            (true, Width::B8, Width::B64),
            (true, Width::B16, Width::B32),
            (true, Width::B16, Width::B64),
            (true, Width::B32, Width::B64),
        ].into_iter().flat_map(|(signed, from, to)| {
            [Operand::Reg(v()), Operand::Mem(mem())]
                .map(|src| MInst::Extend { signed, from, to, dst: v(), src })
        }).collect::<Vec<_>>(),
        Alu => [
            AluOp::Add,
            AluOp::Sub,
            AluOp::And,
            AluOp::Or,
            AluOp::Xor,
            AluOp::Imul,
            AluOp::Adc,
            AluOp::Sbb,
        ].into_iter().flat_map(|op| {
            // There is no 8-bit two-operand `imul`.
            all.into_iter()
                .filter(move |&width| op != AluOp::Imul || width != Width::B8)
                .flat_map(move |width| sources().map(|src| MInst::Alu { op, width, dst: v(), src }))
        }).collect::<Vec<_>>(),
        Imul3 => [Width::B16, Width::B32, Width::B64]
            .map(|width| MInst::Imul3 { width, dst: v(), src: v(), imm: 1000 }),
        Cmp => all.into_iter().flat_map(|width| {
            sources().map(|rhs| MInst::Cmp { width, lhs: v(), rhs })
        }).collect::<Vec<_>>(),
        Test => all.map(|width| MInst::Test { width, lhs: v(), rhs: v() }),
        Shift => [ShiftOp::Shl, ShiftOp::Shr, ShiftOp::Sar, ShiftOp::Rol].into_iter().flat_map(|op| {
            all.into_iter().flat_map(move |width| {
                COUNTS.into_iter().map(ShiftAmount::Imm).chain([ShiftAmount::Cl])
                    .map(move |amount| MInst::Shift { op, width, dst: v(), amount })
            })
        }).collect::<Vec<_>>(),
        Unary => all.map(|width| MInst::Unary { op: UnaryOp::Neg, width, dst: v() })
            .into_iter()
            .chain(wide.map(|width| MInst::Unary { op: UnaryOp::Bswap, width, dst: v() }))
            .collect::<Vec<_>>(),
        BitScan => [false, true].into_iter().flat_map(|reverse| {
            wide.map(|width| MInst::BitScan { reverse, width, dst: v(), src: v() })
        }).collect::<Vec<_>>(),
        SignExtendAx => wide.map(|width| MInst::SignExtendAx { width }),
        Div => [false, true].into_iter().flat_map(|signed| {
            wide.map(|width| MInst::Div { signed, width, divisor: v() })
        }).collect::<Vec<_>>(),
        MulWide => all.map(|width| MInst::MulWide { width, src: v() }),
        Setcc => conds.map(|cond| MInst::Setcc { cond, dst: v() }),
        Cmov => conds.into_iter().flat_map(|cond| {
            [Width::B16, Width::B32, Width::B64]
                .map(|width| MInst::Cmov { cond, width, dst: v(), src: v() })
        }).collect::<Vec<_>>(),
        Call => [
            Callee::Symbol(SymbolRef::Runtime("memcpy")),
            Callee::Reg(v()),
        ].map(|callee| MInst::Call {
            callee,
            uses: PRegSet::of(&[PReg::Rdi, PReg::Rsi, PReg::Xmm0]),
            clobbers: PRegSet::of(&[PReg::Rax, PReg::Rcx, PReg::R11, PReg::Xmm0, PReg::Xmm15]),
        }),
        Jmp => [MInst::Jmp { target: MBlock::new(1) }],
        Jcc => conds.map(|cond| MInst::Jcc {
            cond,
            taken: MBlock::new(1),
            not_taken: MBlock::new(2),
        }),
        JmpIndirect => [MInst::JmpIndirect { target: v(), dests: 0 }],
        ThreadPointer => [MInst::ThreadPointer { dst: v() }],
        TlsLea => [MInst::TlsLea { dst: v(), base: v(), symbol: SymbolRef::Runtime("tls") }],
        XMov => wide.into_iter().flat_map(|width| {
            [Operand::Reg(v()), Operand::Mem(mem())].map(|src| MInst::XMov { width, dst: v(), src })
        }).collect::<Vec<_>>(),
        Move128 => [true, false].map(|load| MInst::Move128 { load, reg: v(), mem: mem() }),
        XStore => wide.map(|width| MInst::XStore { width, dst: mem(), src: v() }),
        XZero => [MInst::XZero { dst: v() }],
        XAlu => [XOp::Add, XOp::Sub, XOp::Mul, XOp::Div, XOp::Xor].into_iter().flat_map(|op| {
            wide.into_iter().flat_map(move |width| {
                // `xorps` would need an aligned 16-byte memory operand.
                let memory = (op != XOp::Xor).then(|| Operand::Mem(mem()));
                [Some(Operand::Reg(v())), memory]
                    .into_iter()
                    .flatten()
                    .map(move |src| MInst::XAlu { op, width, dst: v(), src })
                    .collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>(),
        Ucomi => wide.map(|width| MInst::Ucomi { width, lhs: v(), rhs: v() }),
        IntToFloat => wide.into_iter().flat_map(|int| {
            wide.map(|float| MInst::IntToFloat { int, float, dst: v(), src: v() })
        }).collect::<Vec<_>>(),
        FloatToInt => wide.into_iter().flat_map(|float| {
            wide.map(|int| MInst::FloatToInt { float, int, dst: v(), src: v() })
        }).collect::<Vec<_>>(),
        FloatToFloat => wide.map(|from| MInst::FloatToFloat { from, dst: v(), src: v() }),
        MovGx => wide.map(|width| MInst::MovGx { width, dst: v(), src: v() }),
        ShiftDouble => [true, false].into_iter().flat_map(|left| {
            COUNTS.into_iter().map(ShiftAmount::Imm).chain([ShiftAmount::Cl])
                .map(move |amount| MInst::ShiftDouble { left, dst: v(), src: v(), amount })
        }).collect::<Vec<_>>(),
        RepMovsb => [MInst::RepMovsb],
        RepStosb => [MInst::RepStosb],
        X87Load => formats.map(|format| MInst::X87Load { format, mem: mem() }),
        X87Store => formats.map(|format| MInst::X87Store { format, mem: mem() }),
        X87Arith => [X87Op::Add, X87Op::Sub, X87Op::Mul, X87Op::Div]
            .map(|op| MInst::X87Arith { op }),
        X87Pop => [MInst::X87Pop],
        X87Compare => [MInst::X87Compare],
        X87ControlWord => [true, false].map(|store| MInst::X87ControlWord { store, mem: mem() }),
        Ret => [MInst::Ret { uses: PRegSet::of(&[PReg::Rax, PReg::Rdx, PReg::Xmm0]) }],
        Ud2 => [MInst::Ud2],
        Int3 => [MInst::Int3],
    }
    forms
}
