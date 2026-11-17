//! Named target instructions (`ir/intrinsics.rs`): reads each vector
//! operand's bytes, computes the instruction as Intel's SDM describes it, and
//! writes a vector result's bytes or returns a scalar one. A poison byte in
//! an operand makes the whole result poison. `vrcpps` and `vrsqrtps` are
//! approximations each processor computes its own way, so they stop the run
//! as unsupported rather than give one processor's values.

use super::{
    Machine,
    trap::{
        Fault,
        Unsupported,
    },
    value::RuntimeValue,
};
use crate::ir::{
    Align,
    Intrinsic,
    IntrinsicOperand,
    Type,
    Value,
};

/// A vector operand's bytes, up to 32.
type Bytes = [u8; 32];

impl Machine<'_, '_, '_> {
    /// Executes `intrinsic` with immediate `immediate` on `args`, and
    /// returns its scalar result, if it has one.
    pub(super) fn intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        immediate: u8,
        args: &[Value],
    ) -> Result<Option<RuntimeValue>, Fault> {
        let signature = intrinsic.signature();
        let (destination, operands) = if signature.has_destination() {
            let (&destination, operands) = args.split_first().expect("verified operands");
            (Some(self.value(destination)), operands)
        } else {
            (None, args)
        };
        if let Intrinsic::SqrtF32 | Intrinsic::SqrtF64 = intrinsic {
            return Ok(Some(match self.value(operands[0]) {
                | RuntimeValue::F32(x) => RuntimeValue::F32(x.sqrt()),
                | RuntimeValue::F64(x) => RuntimeValue::F64(x.sqrt()),
                | _ => RuntimeValue::Poison,
            }));
        }
        let mut vectors = [[0; 32]; 2];
        let mut lengths = [0_i32; 2];
        let mut poison = false;
        let (mut vector, mut scalar) = (0, 0);
        for (&operand, &arg) in signature.operands.iter().zip(operands) {
            match operand {
                | IntrinsicOperand::Vector(..) => {
                    let bytes = operand.vector_bytes().expect("a vector operand");
                    match self.read_vector(self.value(arg), bytes)? {
                        | Some(read) => vectors[vector] = read,
                        | None => poison = true,
                    }
                    vector += 1;
                },
                | IntrinsicOperand::Scalar(_) => {
                    match self.value(arg) {
                        | RuntimeValue::Int(bits) => lengths[scalar] = low_i32(bits),
                        | _ => poison = true,
                    }
                    scalar += 1;
                },
            }
        }
        let [a, b] = vectors;
        let result = match intrinsic {
            | Intrinsic::RcpPs256 | Intrinsic::RsqrtPs256 =>
                return Err(Fault::Unsupported(Unsupported::Intrinsic(intrinsic))),
            | Intrinsic::Vzeroupper | Intrinsic::Vzeroall => return Ok(None),
            | Intrinsic::SqrtF32 | Intrinsic::SqrtF64 => unreachable!("computed above"),
            | Intrinsic::AesEnc => aes_round(&a, &b, Direction::Encrypt, true),
            | Intrinsic::AesEncLast => aes_round(&a, &b, Direction::Encrypt, false),
            | Intrinsic::AesDec => aes_round(&a, &b, Direction::Decrypt, true),
            | Intrinsic::AesDecLast => aes_round(&a, &b, Direction::Decrypt, false),
            | Intrinsic::AesImc => {
                let mut state = [0; 16];
                state.copy_from_slice(&a[..16]);
                Output::Vector(widen(mix_columns(state, Direction::Decrypt)))
            },
            | Intrinsic::AesKeygenAssist => Output::Vector(keygen_assist(&a, immediate)),
            | Intrinsic::Pclmulqdq => Output::Vector(clmul(&a, &b, immediate)),
            | Intrinsic::RoundPs | Intrinsic::RoundPs256 =>
                Output::Vector(round_lanes::<4>(&a, &a, immediate, false)),
            | Intrinsic::RoundPd | Intrinsic::RoundPd256 =>
                Output::Vector(round_lanes::<8>(&a, &a, immediate, false)),
            | Intrinsic::RoundSs => Output::Vector(round_lanes::<4>(&b, &a, immediate, true)),
            | Intrinsic::RoundSd => Output::Vector(round_lanes::<8>(&b, &a, immediate, true)),
            | Intrinsic::CvtPs2dq256 => Output::Vector(convert::<4>(&a, 8, false)),
            | Intrinsic::CvttPs2dq256 => Output::Vector(convert::<4>(&a, 8, true)),
            | Intrinsic::CvtPd2dq256 => Output::Vector(convert::<8>(&a, 4, false)),
            | Intrinsic::CvttPd2dq256 => Output::Vector(convert::<8>(&a, 4, true)),
            | Intrinsic::Pcmpistri128
            | Intrinsic::Pcmpistrm128
            | Intrinsic::Pcmpistria128
            | Intrinsic::Pcmpistric128
            | Intrinsic::Pcmpistrio128
            | Intrinsic::Pcmpistris128
            | Intrinsic::Pcmpistriz128 => string_compare(intrinsic, &a, None, &b, None, immediate),
            | Intrinsic::Pcmpestri128
            | Intrinsic::Pcmpestrm128
            | Intrinsic::Pcmpestria128
            | Intrinsic::Pcmpestric128
            | Intrinsic::Pcmpestrio128
            | Intrinsic::Pcmpestris128
            | Intrinsic::Pcmpestriz128 => string_compare(
                intrinsic,
                &a,
                Some(lengths[0]),
                &b,
                Some(lengths[1]),
                immediate,
            ),
        };
        match (result, destination) {
            | (Output::Vector(bytes), Some(destination)) => {
                let size = signature
                    .result
                    .and_then(IntrinsicOperand::vector_bytes)
                    .expect("a vector result");
                self.write_vector(destination, &bytes, size, poison)?;
                Ok(None)
            },
            | (Output::Scalar(value), _) => Ok(Some(if poison {
                RuntimeValue::Poison
            } else {
                RuntimeValue::Int(u128::from(value))
            })),
            | (Output::Vector(_), None) => unreachable!("a vector result has a destination"),
        }
    }

    /// The `size` bytes at `address`, or `None` if any is poison.
    fn read_vector(&self, address: RuntimeValue, size: u32) -> Result<Option<Bytes>, Fault> {
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().take(size as usize).enumerate() {
            let at = self
                .memory
                .ptr_add(address, RuntimeValue::Int(index as u128), false);
            match self.memory.load(Type::I8, at, Align::BYTE)? {
                | RuntimeValue::Int(bits) => *byte = bits.to_le_bytes()[0],
                | _ => return Ok(None),
            }
        }
        Ok(Some(bytes))
    }

    /// Writes `size` bytes of `bytes` at `address`, or poison.
    fn write_vector(
        &mut self,
        address: RuntimeValue,
        bytes: &Bytes,
        size: u32,
        poison: bool,
    ) -> Result<(), Fault> {
        for (index, &byte) in bytes.iter().take(size as usize).enumerate() {
            let at = self
                .memory
                .ptr_add(address, RuntimeValue::Int(index as u128), false);
            let value = if poison {
                RuntimeValue::Poison
            } else {
                RuntimeValue::Int(u128::from(byte))
            };
            self.memory.store(Type::I8, value, at, Align::BYTE)?;
        }
        Ok(())
    }
}

/// The low 32 bits of an `i32` operand's value.
fn low_i32(bits: u128) -> i32 {
    i32::from_le_bytes(bits.to_le_bytes()[..4].try_into().expect("four bytes"))
}

/// What an instruction computes.
#[expect(
    variant_size_differences,
    reason = "a transient result, moved once into memory or a value"
)]
enum Output {
    Vector(Bytes),
    Scalar(u32),
}

fn widen(state: [u8; 16]) -> Bytes {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&state);
    bytes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Encrypt,
    Decrypt,
}

/// One AES round on the state `a` with the round key `b`, the state's byte
/// `4c + r` in row `r` of column `c` (FIPS 197 §3.4): `ShiftRows` and
/// `SubBytes` (or their inverses), `MixColumns` (or its inverse) unless it is
/// the last round, then the key added by exclusive or. Intel SDM Vol. 2A,
/// "AESENC", "AESENCLAST", "AESDEC", "AESDECLAST".
fn aes_round(a: &Bytes, b: &Bytes, direction: Direction, mix: bool) -> Output {
    let mut state = [0; 16];
    for row in 0..4 {
        for column in 0..4 {
            let from = match direction {
                | Direction::Encrypt => (column + row) % 4,
                | Direction::Decrypt => (column + 4 - row) % 4,
            };
            let byte = a[row + 4 * from];
            state[row + 4 * column] = match direction {
                | Direction::Encrypt => sub_byte(byte),
                | Direction::Decrypt => inverse_sub_byte(byte),
            };
        }
    }
    if mix {
        state = mix_columns(state, direction);
    }
    for (byte, key) in state.iter_mut().zip(b) {
        *byte ^= key;
    }
    Output::Vector(widen(state))
}

/// `MixColumns` or `InvMixColumns`: each column multiplied by the fixed
/// polynomial in GF(2^8). FIPS 197 §5.1.3 and §5.3.3.
fn mix_columns(state: [u8; 16], direction: Direction) -> [u8; 16] {
    let factors: [u8; 4] = match direction {
        | Direction::Encrypt => [2, 3, 1, 1],
        | Direction::Decrypt => [14, 11, 13, 9],
    };
    let mut mixed = [0; 16];
    for column in 0..4 {
        for row in 0..4 {
            mixed[row + 4 * column] = (0..4).fold(0, |sum, k| {
                sum ^ gf_multiply(factors[(4 + k - row) % 4], state[k + 4 * column])
            });
        }
    }
    mixed
}

/// The product in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1. FIPS 197 §4.2.
fn gf_multiply(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0;
    while b != 0 {
        if b & 1 != 0 {
            product ^= a;
        }
        a = (a << 1) ^ if a & 0x80 != 0 { 0x1B } else { 0 };
        b >>= 1;
    }
    product
}

/// The S-box: the multiplicative inverse in GF(2^8), then the affine
/// transformation. FIPS 197 §5.1.1.
fn sub_byte(byte: u8) -> u8 {
    // x^254 is the inverse of x, and 0 for 0.
    let mut inverse = 1;
    for _ in 0..254 {
        inverse = gf_multiply(inverse, byte);
    }
    inverse
        ^ inverse.rotate_left(1)
        ^ inverse.rotate_left(2)
        ^ inverse.rotate_left(3)
        ^ inverse.rotate_left(4)
        ^ 0x63
}

/// The inverse S-box. FIPS 197 §5.3.2.
fn inverse_sub_byte(byte: u8) -> u8 {
    (0..=255).find(|&x| sub_byte(x) == byte).unwrap_or(0)
}

/// `aeskeygenassist`: `SubWord` of dwords 1 and 3, and each rotated right by
/// 8 bits and combined with the round constant by exclusive or. Intel SDM
/// Vol. 2A, "AESKEYGENASSIST".
fn keygen_assist(a: &Bytes, rcon: u8) -> Bytes {
    let dword = |index: usize| {
        u32::from_le_bytes([
            a[4 * index],
            a[4 * index + 1],
            a[4 * index + 2],
            a[4 * index + 3],
        ])
    };
    let sub_word = |word: u32| u32::from_le_bytes(word.to_le_bytes().map(sub_byte));
    let (x1, x3) = (sub_word(dword(1)), sub_word(dword(3)));
    let rcon = u32::from(rcon);
    let words = [x1, x1.rotate_right(8) ^ rcon, x3, x3.rotate_right(8) ^ rcon];
    let mut bytes = [0; 32];
    for (index, word) in words.into_iter().enumerate() {
        bytes[4 * index..4 * index + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

/// `pclmulqdq`: the carry-less product of quadword `imm & 1` of `a` and
/// quadword `imm >> 4 & 1` of `b`. Intel SDM Vol. 2B, "PCLMULQDQ".
fn clmul(a: &Bytes, b: &Bytes, immediate: u8) -> Bytes {
    let quadword = |bytes: &Bytes, index: usize| {
        u64::from_le_bytes(
            bytes[8 * index..8 * index + 8]
                .try_into()
                .expect("eight bytes"),
        )
    };
    let x = u128::from(quadword(a, usize::from(immediate & 1)));
    let y = quadword(b, usize::from(immediate >> 4 & 1));
    let product = (0..64)
        .filter(|bit| y >> bit & 1 != 0)
        .fold(0_u128, |product, bit| product ^ (x << bit));
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&product.to_le_bytes());
    bytes
}

/// A lane of `N` bytes (4 or 8) as an `f64`, and back.
fn lane<const N: usize>(bytes: &Bytes, index: usize) -> f64 {
    let at = N * index;
    if N == 4 {
        f64::from(f32::from_le_bytes(
            bytes[at..at + 4].try_into().expect("four bytes"),
        ))
    } else {
        f64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a `float` lane widened exactly, or an integral value it holds"
)]
fn set_lane<const N: usize>(bytes: &mut Bytes, index: usize, value: f64) {
    let at = N * index;
    if N == 4 {
        bytes[at..at + 4].copy_from_slice(&(value as f32).to_le_bytes());
    } else {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
}

/// `roundps` and its kin: each lane of `source` rounded to an integral value
/// as the immediate's bits 0-1 select (to nearest even, down, up, toward
/// zero), or to nearest even when bit 2 defers to MXCSR's default mode. A
/// NaN comes back quiet. With `scalar`, only lane 0 is rounded and the
/// others come from `rest`. Intel SDM Vol. 2B, "ROUNDPS", "ROUNDSS".
fn round_lanes<const N: usize>(source: &Bytes, rest: &Bytes, immediate: u8, scalar: bool) -> Bytes {
    let mut bytes = *rest;
    let lanes = if scalar { 1 } else { 32 / N };
    for index in 0..lanes {
        let x = lane::<N>(source, index);
        let mode = if immediate & 4 != 0 { 0 } else { immediate & 3 };
        let rounded = match mode {
            | 0 => x.round_ties_even(),
            | 1 => x.floor(),
            | 2 => x.ceil(),
            | _ => x.trunc(),
        };
        // Lanes past the operand's are never written back.
        set_lane::<N>(&mut bytes, index, rounded);
    }
    bytes
}

/// `vcvtps2dq` and its kin: `count` lanes of `N` bytes converted to `i32`,
/// rounded to nearest even or truncated; NaN and out-of-range lanes give the
/// integer indefinite `0x80000000`. Intel SDM Vol. 2A, "CVTPS2DQ".
fn convert<const N: usize>(source: &Bytes, count: usize, truncate: bool) -> Bytes {
    let mut bytes = [0; 32];
    for index in 0..count {
        let x = lane::<N>(source, index);
        let rounded = if truncate {
            x.trunc()
        } else {
            x.round_ties_even()
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "an integral value in the range of `i32`"
        )]
        let value = if (-2_147_483_648.0..2_147_483_648.0).contains(&rounded) {
            rounded as i32
        } else {
            i32::MIN
        };
        bytes[4 * index..4 * index + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// SSE4.2's string comparison of the elements of `a` (the first operand)
/// with those of `b`: the immediate's bits 0-1 give the element type, bits
/// 2-3 the aggregation, bits 4-5 the polarity and bit 6 the index or mask
/// form; each builtin returns the index, the mask or one flag. An implicit
/// length ends at the first zero element; an explicit one is the length's
/// magnitude, at most the element count. Intel SDM Vol. 1, §12.10.2-§12.10.5,
/// and Vol. 2B, "PCMPESTRI", "PCMPISTRI".
fn string_compare(
    intrinsic: Intrinsic,
    a: &Bytes,
    a_length: Option<i32>,
    b: &Bytes,
    b_length: Option<i32>,
    immediate: u8,
) -> Output {
    let words = immediate & 1 != 0;
    let signed = immediate & 2 != 0;
    let count = if words { 8 } else { 16 };
    let element = |bytes: &Bytes, index: usize| -> i32 {
        match (words, signed) {
            | (false, false) => i32::from(bytes[index]),
            | (false, true) => i32::from(bytes[index] as i8),
            | (true, false) =>
                i32::from(u16::from_le_bytes([bytes[2 * index], bytes[2 * index + 1]])),
            | (true, true) =>
                i32::from(i16::from_le_bytes([bytes[2 * index], bytes[2 * index + 1]])),
        }
    };
    let length = |bytes: &Bytes, explicit: Option<i32>| match explicit {
        | Some(length) => usize::try_from(length.unsigned_abs())
            .unwrap_or(count)
            .min(count),
        | None => (0..count)
            .find(|&index| element(bytes, index) == 0)
            .unwrap_or(count),
    };
    let (a_valid, b_valid) = (length(a, a_length), length(b, b_length));
    let aggregation = immediate >> 2 & 3;
    // Table 12-11's results for pairs with an invalid element.
    let pair =
        |i: usize, j: usize, compare: &dyn Fn(i32, i32) -> bool| match (i < a_valid, j < b_valid) {
            | (true, true) => compare(element(a, i), element(b, j)),
            | (false, false) => aggregation >= 2,
            | (false, true) => aggregation == 3,
            | (true, false) => false,
        };
    let equal = |x: i32, y: i32| x == y;
    let mut result: u32 = 0;
    for j in 0..count {
        let bit = match aggregation {
            // Equal any: `b[j]` is one of `a`'s elements.
            | 0 => (0..count).any(|i| pair(i, j, &equal)),
            // Ranges: `b[j]` lies in one of `a`'s pairs of bounds.
            | 1 => (0..count)
                .step_by(2)
                .any(|i| pair(i, j, &|low, y| y >= low) && pair(i + 1, j, &|high, y| y <= high)),
            // Equal each: `a[j]` equals `b[j]`.
            | 2 => pair(j, j, &equal),
            // Equal ordered: `a` occurs in `b` from `b[j]`.
            | _ => (0..count - j).all(|i| pair(i, j + i, &equal)),
        };
        result |= u32::from(bit) << j;
    }
    let all = (1_u32 << count) - 1;
    result = match immediate >> 4 & 3 {
        | 1 => !result & all,
        | 3 => result ^ ((1_u32 << b_valid) - 1),
        | _ => result,
    };
    let flag = |set: bool| Output::Scalar(u32::from(set));
    let carry = result != 0;
    let zero = b_valid < count;
    match intrinsic {
        | Intrinsic::Pcmpistrm128 | Intrinsic::Pcmpestrm128 => {
            let mut bytes = [0; 32];
            if immediate & 0x40 == 0 {
                bytes[..4].copy_from_slice(&result.to_le_bytes());
            } else {
                let width = if words { 2 } else { 1 };
                for index in 0..count {
                    if result >> index & 1 != 0 {
                        bytes[width * index..width * (index + 1)].fill(0xFF);
                    }
                }
            }
            Output::Vector(bytes)
        },
        | Intrinsic::Pcmpistri128 | Intrinsic::Pcmpestri128 => Output::Scalar(if result == 0 {
            if words { 8 } else { 16 }
        } else if immediate & 0x40 != 0 {
            result.ilog2()
        } else {
            result.trailing_zeros()
        }),
        | Intrinsic::Pcmpistria128 | Intrinsic::Pcmpestria128 => flag(!carry && !zero),
        | Intrinsic::Pcmpistric128 | Intrinsic::Pcmpestric128 => flag(carry),
        | Intrinsic::Pcmpistrio128 | Intrinsic::Pcmpestrio128 => flag(result & 1 != 0),
        | Intrinsic::Pcmpistris128 | Intrinsic::Pcmpestris128 => flag(a_valid < count),
        | _ => flag(zero),
    }
}
