//! Programs of the named x86 instructions (`ir/intrinsics.rs`) for the
//! native oracles: vectors of random bytes and special `float` and `double`
//! lanes in stack slots, then a random sequence of the instructions the
//! interpreter computes, each result printed. One function runs them
//! without AVX, where the native back end takes their SSE forms, and one
//! with AVX, where it takes their VEX forms and the 32-byte instructions
//! join in. A result may overwrite an operand of a later instruction, and
//! an instruction's destination may be one of its own operands.
//!
//! The approximate reciprocals are left out, since the interpreter does not
//! compute them, and so is every XCR0 bit but the x87, SSE and AVX ones,
//! which the interpreter models and an AVX processor has. `vzeroall` is
//! left out too: it would print nothing of its own, and the native
//! end-to-end tests run it.

use std::fmt::Write as _;

use super::generate::Rng;
use crate::ir::{
    Intrinsic,
    IntrinsicOperand,
};

/// Vector slots per function.
const SLOTS: usize = 6;

/// The program for `seed`: its `main` calls both functions.
pub(super) fn generate_named(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::from(
        "global @fmt_q internal constant size 6, align 1 = bytes \"256c6c780a00\"\nfunction \
         @printf(ptr, ...) -> i32 external\n",
    );
    for (name, avx) in [("sse", false), ("vex", true)] {
        out.push('\n');
        Function::new(&mut rng, avx).write(&mut out, name);
    }
    out.push_str(
        "\nfunction @main() -> i32 external {\nblock0:\n    v0 = call @sse()\n    v1 = call \
         @vex()\n    v2 = iconst.i32 0\n    return v2\n}\n",
    );
    out
}

/// Special lanes, as their bits: zeros, halves, infinities, NaNs with
/// payloads, and values at the edges of the integer conversions.
const FLOATS: [u32; 12] = [
    0x0000_0000, 0x8000_0000, 0x3f00_0000, 0xbfc0_0000, 0x4020_0000, 0x7f80_0000, 0xff80_0000,
    0x7fc0_1234, 0xffa0_0001, 0x4f00_0000, 0xcf00_0000, 0x4b7f_ffff,
];
const DOUBLES: [u64; 12] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x3fe0_0000_0000_0000,
    0xbff8_0000_0000_0000,
    0x4004_0000_0000_0000,
    0x7ff0_0000_0000_0000,
    0xfff0_0000_0000_0000,
    0x7ff8_0000_0000_1234,
    0xfff4_0000_0000_0001,
    0x41e0_0000_0000_0000,
    0xc1e0_0000_0020_0000,
    0x432f_ffff_ffff_ffff,
];

struct Function<'r> {
    rng:   &'r mut Rng,
    avx:   bool,
    body:  String,
    value: u32,
}

impl<'r> Function<'r> {
    fn new(rng: &'r mut Rng, avx: bool) -> Self {
        Self {
            rng,
            avx,
            body: String::new(),
            value: SLOTS as u32,
        }
    }

    /// Defines a fresh value with `text`.
    fn def(&mut self, text: std::fmt::Arguments<'_>) -> u32 {
        let value = self.value;
        self.value += 1;
        let _ = writeln!(self.body, "    v{value} = {text}");
        value
    }

    fn write(mut self, out: &mut String, name: &str) {
        for slot in 0..SLOTS {
            self.fill(slot);
        }
        let format = self.def(format_args!("global_addr @fmt_q"));
        let zero = self.def(format_args!("iconst.i32 0"));
        let xcr0 = self.def(format_args!("intrinsic x86.xgetbv v{zero}"));
        let mask = self.def(format_args!("iconst.i64 7"));
        let low = self.def(format_args!("and.i64 v{xcr0}, v{mask}"));
        let _ = self.def(format_args!("call @printf(v{format}, v{low})"));
        let candidates: Vec<Intrinsic> = Intrinsic::ALL
            .iter()
            .copied()
            .filter(|&intrinsic| self.allowed(intrinsic))
            .collect();
        for _ in 0..24 {
            let intrinsic = candidates[self.rng.index(candidates.len())];
            self.instruction(intrinsic, format);
        }
        let features = if self.avx {
            "+avx,+aes,+pclmul,+sse4.2,+xsave"
        } else {
            "+aes,+pclmul,+sse4.2,+xsave"
        };
        let _ = writeln!(
            out,
            "function @{name}() -> i32 internal noinline target \"{features}\" {{"
        );
        for slot in 0..SLOTS {
            let _ = writeln!(out, "    slot{slot} = stack_slot 32, align 8");
        }
        out.push_str("block0:\n");
        for slot in 0..SLOTS {
            let _ = writeln!(out, "    v{slot} = stack_addr slot{slot}");
        }
        out.push_str(&self.body);
        let _ = writeln!(out, "    return v{zero}\n}}");
    }

    /// Whether the function may run `intrinsic`: the interpreter computes
    /// it, and a 32-byte operand needs AVX.
    fn allowed(&self, intrinsic: Intrinsic) -> bool {
        let signature = intrinsic.signature();
        let wide = signature
            .operands
            .iter()
            .chain(&signature.result)
            .any(|operand| operand.vector_bytes().is_some_and(|bytes| bytes > 16));
        !matches!(
            intrinsic,
            Intrinsic::RcpPs256
                | Intrinsic::RsqrtPs256
                | Intrinsic::SqrtF32
                | Intrinsic::SqrtF64
                | Intrinsic::Xgetbv
                | Intrinsic::Vzeroall
        ) && (self.avx || !wide && intrinsic != Intrinsic::Vzeroupper)
    }

    /// Stores 32 bytes into slot `slot`: random bytes, special `float`
    /// lanes or special `double` lanes, or small letters for the string
    /// comparisons.
    fn fill(&mut self, slot: usize) {
        let kind = self.rng.index(4);
        for word in 0..4 {
            let bits = match kind {
                | 0 => self.rng.next(),
                | 1 => {
                    let low = FLOATS[self.rng.index(FLOATS.len())];
                    let high = FLOATS[self.rng.index(FLOATS.len())];
                    u64::from(low) | u64::from(high) << 32
                },
                | 2 => DOUBLES[self.rng.index(DOUBLES.len())],
                | _ => {
                    let mut bits = 0;
                    for byte in 0..8 {
                        let letter = b"abc\0\0z09"[self.rng.index(8)];
                        bits |= u64::from(letter) << (8 * byte);
                    }
                    bits
                },
            };
            let constant = self.def(format_args!("iconst.i64 {}", bits.cast_signed()));
            let address = self.word(slot, word);
            let _ = writeln!(self.body, "    store.i64 v{constant}, v{address}, align 1");
        }
    }

    /// The address of 8-byte word `word` of slot `slot`.
    fn word(&mut self, slot: usize, word: usize) -> u32 {
        let base = slot as u32;
        if word == 0 {
            return base;
        }
        let offset = self.def(format_args!("iconst.i64 {}", 8 * word));
        self.def(format_args!("ptr_add inbounds v{base}, v{offset}"))
    }

    /// Runs `intrinsic` on random slots and prints its result.
    fn instruction(&mut self, intrinsic: Intrinsic, format: u32) {
        let signature = intrinsic.signature();
        let mut operands = Vec::new();
        let destination = signature
            .has_destination()
            .then(|| self.rng.index(SLOTS));
        if let Some(slot) = destination {
            operands.push(format!("v{slot}"));
        }
        for operand in signature.operands {
            match operand {
                | IntrinsicOperand::Vector(..) => {
                    operands.push(format!("v{}", self.rng.index(SLOTS)));
                },
                | IntrinsicOperand::Scalar(_) => {
                    let length = self.rng.int(40) - 12;
                    let value = self.def(format_args!("iconst.i32 {length}"));
                    operands.push(format!("v{value}"));
                },
            }
        }
        let immediate = match signature.immediate {
            // The roundings take a rounding control of four bits.
            | Some(_) if intrinsic.name().starts_with("x86.round") =>
                format!("{}, ", self.rng.index(16)),
            | Some(_) => format!("{}, ", self.rng.index(256)),
            | None => String::new(),
        };
        let text = format!(
            "intrinsic {} {immediate}{}",
            intrinsic.name(),
            operands.join(", ")
        );
        match signature.result {
            | Some(IntrinsicOperand::Scalar(_)) => {
                let result = self.def(format_args!("{text}"));
                let wide = self.def(format_args!("sext.i64 v{result}"));
                let _ = self.def(format_args!("call @printf(v{format}, v{wide})"));
            },
            | Some(vector) => {
                let _ = writeln!(self.body, "    {text}");
                let slot = destination.expect("a vector result has a destination");
                let words = vector.vector_bytes().expect("a vector") / 8;
                for word in 0..words as usize {
                    let address = self.word(slot, word);
                    let loaded = self.def(format_args!("load.i64 v{address}, align 1"));
                    let _ = self.def(format_args!("call @printf(v{format}, v{loaded})"));
                }
            },
            | None => {
                let _ = writeln!(self.body, "    {text}");
            },
        }
    }
}
