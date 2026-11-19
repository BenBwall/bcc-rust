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
//!
//! The function with AVX also calls two helpers with AVX between its
//! instructions, which take two 32-byte vectors in YMM registers around an
//! `i32` and return a 32-byte vector in YMM0 or a 64-byte one in YMM0 and
//! YMM1, so each back end's side of that convention meets the interpreter's
//! (System V AMD64 psABI §3.2.3, `__m256`).

use std::fmt::Write as _;

use super::generate::Rng;
use crate::ir::{
    Intrinsic,
    IntrinsicOperand,
};

/// Vector slots per function.
const SLOTS: u32 = 6;

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
    out.push_str(YMM_HELPERS);
    out.push_str(
        "\nfunction @main() -> i32 external {\nblock0:\n    v0 = call @sse()\n    v1 = call \
         @vex()\n    v2 = iconst.i32 0\n    return v2\n}\n",
    );
    out
}

/// The helpers whose vectors travel in YMM registers: `@ymm` returns
/// `a - b * k` by 64-bit word, and `@ymm64` returns `a ^ b` and then `b - a`.
const YMM_HELPERS: &str = "
function @ymm(ptr result(size 32, align 32, vector, wide), ptr aggregate(size 32, align 32, \
                           vector, wide), i32, ptr aggregate(size 32, align 32, vector, wide)) \
                           internal noinline target \"+avx\" {
block0(v0: ptr, v1: ptr, v2: i32, v3: ptr):
    v4 = sext.i64 v2
    v5 = iconst.i64 8
    v6 = load.i64 v1, align 1
    v7 = load.i64 v3, align 1
    v8 = imul.i64 v7, v4
    v9 = isub.i64 v6, v8
    store.i64 v9, v0, align 1
    v10 = ptr_add inbounds v1, v5
    v11 = ptr_add inbounds v3, v5
    v12 = ptr_add inbounds v0, v5
    v13 = load.i64 v10, align 1
    v14 = load.i64 v11, align 1
    v15 = imul.i64 v14, v4
    v16 = isub.i64 v13, v15
    store.i64 v16, v12, align 1
    v17 = ptr_add inbounds v10, v5
    v18 = ptr_add inbounds v11, v5
    v19 = ptr_add inbounds v12, v5
    v20 = load.i64 v17, align 1
    v21 = load.i64 v18, align 1
    v22 = imul.i64 v21, v4
    v23 = isub.i64 v20, v22
    store.i64 v23, v19, align 1
    v24 = ptr_add inbounds v17, v5
    v25 = ptr_add inbounds v18, v5
    v26 = ptr_add inbounds v19, v5
    v27 = load.i64 v24, align 1
    v28 = load.i64 v25, align 1
    v29 = imul.i64 v28, v4
    v30 = isub.i64 v27, v29
    store.i64 v30, v26, align 1
    return
}

function @ymm64(ptr result(size 64, align 64, vector, wide), ptr aggregate(size 32, align 32, \
                           vector, wide), ptr aggregate(size 32, align 32, vector, wide)) internal \
                           noinline target \"+avx\" {
block0(v0: ptr, v1: ptr, v2: ptr):
    v3 = iconst.i64 8
    v4 = iconst.i64 32
    v5 = ptr_add inbounds v0, v4
    v6 = load.i64 v1, align 1
    v7 = load.i64 v2, align 1
    v8 = xor.i64 v6, v7
    store.i64 v8, v0, align 1
    v9 = isub.i64 v7, v6
    store.i64 v9, v5, align 1
    v10 = ptr_add inbounds v1, v3
    v11 = ptr_add inbounds v2, v3
    v12 = ptr_add inbounds v0, v3
    v13 = ptr_add inbounds v5, v3
    v14 = load.i64 v10, align 1
    v15 = load.i64 v11, align 1
    v16 = xor.i64 v14, v15
    store.i64 v16, v12, align 1
    v17 = isub.i64 v15, v14
    store.i64 v17, v13, align 1
    v18 = ptr_add inbounds v10, v3
    v19 = ptr_add inbounds v11, v3
    v20 = ptr_add inbounds v12, v3
    v21 = ptr_add inbounds v13, v3
    v22 = load.i64 v18, align 1
    v23 = load.i64 v19, align 1
    v24 = xor.i64 v22, v23
    store.i64 v24, v20, align 1
    v25 = isub.i64 v23, v22
    store.i64 v25, v21, align 1
    v26 = ptr_add inbounds v18, v3
    v27 = ptr_add inbounds v19, v3
    v28 = ptr_add inbounds v20, v3
    v29 = ptr_add inbounds v21, v3
    v30 = load.i64 v26, align 1
    v31 = load.i64 v27, align 1
    v32 = xor.i64 v30, v31
    store.i64 v32, v28, align 1
    v33 = isub.i64 v31, v30
    store.i64 v33, v29, align 1
    return
}
";

/// Special lanes, as their bits: zeros, halves, infinities, NaNs with
/// payloads, and values at the edges of the integer conversions.
const FLOATS: [u32; 12] = [
    0x0000_0000,
    0x8000_0000,
    0x3F00_0000,
    0xBFC0_0000,
    0x4020_0000,
    0x7F80_0000,
    0xFF80_0000,
    0x7FC0_1234,
    0xFFA0_0001,
    0x4F00_0000,
    0xCF00_0000,
    0x4B7F_FFFF,
];
const DOUBLES: [u64; 12] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x3FE0_0000_0000_0000,
    0xBFF8_0000_0000_0000,
    0x4004_0000_0000_0000,
    0x7FF0_0000_0000_0000,
    0xFFF0_0000_0000_0000,
    0x7FF8_0000_0000_1234,
    0xFFF4_0000_0000_0001,
    0x41E0_0000_0000_0000,
    0xC1E0_0000_0020_0000,
    0x432F_FFFF_FFFF_FFFF,
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
            value: SLOTS,
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
        let wide = self
            .avx
            .then(|| self.def(format_args!("stack_addr slot{SLOTS}")));
        for _ in 0..24 {
            match wide {
                | Some(wide) if self.rng.index(6) == 0 => self.ymm_call(wide, format),
                | _ => {
                    let intrinsic = candidates[self.rng.index(candidates.len())];
                    self.instruction(intrinsic, format);
                },
            }
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
        if self.avx {
            // The result of `@ymm64`, whose area the call must align.
            let _ = writeln!(out, "    slot{SLOTS} = stack_slot 64, align 64");
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
    fn fill(&mut self, slot: u32) {
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
    fn word(&mut self, slot: u32, word: usize) -> u32 {
        let base = slot;
        if word == 0 {
            return base;
        }
        let offset = self.def(format_args!("iconst.i64 {}", 8 * word));
        self.def(format_args!("ptr_add inbounds v{base}, v{offset}"))
    }

    /// A random slot.
    fn slot(&mut self) -> u32 {
        u32::try_from(self.rng.below(u64::from(SLOTS))).expect("a slot index is small")
    }

    /// Calls `@ymm` on two different random slots into a third, or `@ymm64`
    /// on two into `wide`, the 64-byte slot, and prints the result.
    fn ymm_call(&mut self, wide: u32, format: u32) {
        let first = self.slot();
        let second = (first + 1 + self.slot() % (SLOTS - 1)) % SLOTS;
        let (destination, words) = if self.rng.index(2) == 0 {
            let destination = (0..SLOTS)
                .find(|&slot| slot != first && slot != second)
                .expect("three slots");
            let scale = self.rng.int(1000) - 500;
            let scale = self.def(format_args!("iconst.i32 {scale}"));
            let _ = writeln!(
                self.body,
                "    call @ymm(v{destination}, v{first}, v{scale}, v{second})"
            );
            (destination, 4)
        } else {
            let _ = writeln!(self.body, "    call @ymm64(v{wide}, v{first}, v{second})");
            (wide, 8)
        };
        for word in 0..words {
            let address = self.word(destination, word);
            let loaded = self.def(format_args!("load.i64 v{address}, align 1"));
            let _ = self.def(format_args!("call @printf(v{format}, v{loaded})"));
        }
    }

    /// Runs `intrinsic` on random slots and prints its result.
    fn instruction(&mut self, intrinsic: Intrinsic, format: u32) {
        let signature = intrinsic.signature();
        let mut operands = Vec::new();
        let destination = signature.has_destination().then(|| self.slot());
        if let Some(slot) = destination {
            operands.push(format!("v{slot}"));
        }
        for operand in signature.operands {
            match operand {
                | IntrinsicOperand::Vector(..) => {
                    operands.push(format!("v{}", self.slot()));
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
