//! The x86-64 features beyond the baseline that `-mavx2` and its kin ask for,
//! and that `-march=` names in groups. Each option turns on what its feature
//! needs and off what needs it, as GCC and Clang do, and the result decides the
//! predefined macros and what LLVM may generate. Intrinsics do not depend on
//! these: bcc's headers compute them on any x86-64.
//! GCC manual, "x86 Options"; Clang Command Line Reference, "X86".

use std::ffi::CStr;

/// One feature: the option after `-m`, LLVM's name, its macro, and the
/// features it turns on.
struct Feature {
    option:  &'static str,
    llvm:    &'static str,
    defines: &'static str,
    implies: u32,
}

const SSE3: u32 = 1 << 0;
const SSSE3: u32 = 1 << 1;
const SSE4_1: u32 = 1 << 2;
const SSE4_2: u32 = 1 << 3;
const POPCNT: u32 = 1 << 4;
const CRC32: u32 = 1 << 5;
const AVX: u32 = 1 << 6;
const AVX2: u32 = 1 << 7;
const FMA: u32 = 1 << 8;
const F16C: u32 = 1 << 9;
const BMI: u32 = 1 << 10;
const BMI2: u32 = 1 << 11;
const LZCNT: u32 = 1 << 12;
const MOVBE: u32 = 1 << 13;
const AES: u32 = 1 << 14;
const PCLMUL: u32 = 1 << 15;
const XSAVE: u32 = 1 << 16;
const SAHF: u32 = 1 << 17;

/// In bit order: entry `n` is the feature with bit `n`.
const FEATURES: [Feature; 18] = [
    feature("sse3", "sse3", "__SSE3__", 0),
    feature("ssse3", "ssse3", "__SSSE3__", SSE3),
    feature("sse4.1", "sse4.1", "__SSE4_1__", SSSE3 | SSE3),
    feature(
        "sse4.2",
        "sse4.2",
        "__SSE4_2__",
        SSE4_1 | SSSE3 | SSE3 | POPCNT | CRC32,
    ),
    feature("popcnt", "popcnt", "__POPCNT__", 0),
    feature("crc32", "crc32", "__CRC32__", 0),
    feature("avx", "avx", "__AVX__", AVX_NEEDS),
    feature("avx2", "avx2", "__AVX2__", AVX | AVX_NEEDS),
    feature("fma", "fma", "__FMA__", AVX | AVX_NEEDS),
    feature("f16c", "f16c", "__F16C__", AVX | AVX_NEEDS),
    feature("bmi", "bmi", "__BMI__", 0),
    feature("bmi2", "bmi2", "__BMI2__", 0),
    feature("lzcnt", "lzcnt", "__LZCNT__", 0),
    feature("movbe", "movbe", "__MOVBE__", 0),
    feature("aes", "aes", "__AES__", 0),
    feature("pclmul", "pclmul", "__PCLMUL__", 0),
    feature("xsave", "xsave", "__XSAVE__", 0),
    feature("sahf", "sahf", "__LAHF_SAHF__", 0),
];

const AVX_NEEDS: u32 = SSE4_2 | SSE4_1 | SSSE3 | SSE3 | POPCNT | CRC32 | XSAVE;
const V2: u32 = SSE4_2 | SSE4_1 | SSSE3 | SSE3 | POPCNT | CRC32 | SAHF;
const V3: u32 = V2 | AVX | AVX2 | FMA | F16C | BMI | BMI2 | LZCNT | MOVBE | XSAVE;

const fn feature(
    option: &'static str,
    llvm: &'static str,
    defines: &'static str,
    implies: u32,
) -> Feature {
    Feature {
        option,
        llvm,
        defines,
        implies,
    }
}

/// The features in effect, and the processor `-march=` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct X86Features {
    enabled: u32,
    cpu:     Option<Cpu>,
}

/// A processor LLVM generates for, apart from baseline `x86-64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cpu {
    V2,
    V3,
    /// `-march=native`: the processor running the compiler.
    Native,
}

impl X86Features {
    pub(crate) const NONE: Self = Self {
        enabled: 0,
        cpu:     None,
    };

    /// Reads one option, if it is one of these. `-march=` replaces what an
    /// earlier one chose and adds to the `-m` options, as in both compilers.
    pub(crate) fn apply(&mut self, option: &str) {
        match option {
            | "-march=x86-64" => self.choose(None, 0),
            | "-march=x86-64-v2" => self.choose(Some(Cpu::V2), V2),
            | "-march=x86-64-v3" => self.choose(Some(Cpu::V3), V3),
            | "-march=native" => self.choose(Some(Cpu::Native), native()),
            | _ =>
                if let Some((bit, feature)) = find(option.strip_prefix("-m")) {
                    self.enabled |= bit | feature.implies;
                } else if let Some((bit, _)) = find(no_option(option)) {
                    self.enabled &= !bit;
                    for (index, feature) in FEATURES.iter().enumerate() {
                        if feature.implies & bit != 0 {
                            self.enabled &= !(1 << index);
                        }
                    }
                },
        }
    }

    const fn choose(&mut self, cpu: Option<Cpu>, features: u32) {
        self.cpu = cpu;
        self.enabled |= features;
    }

    pub(crate) const fn cpu(self) -> Option<Cpu> {
        self.cpu
    }

    /// The macros of the features in effect.
    pub(crate) fn macros(self) -> impl Iterator<Item = &'static str> {
        self.features().map(|feature| feature.defines)
    }

    /// Whether code may use fused multiply-add, which GCC announces with its
    /// `__FP_FAST_FMA` macros.
    pub(crate) const fn fast_fma(self) -> bool {
        self.enabled & FMA != 0
    }

    /// Whether the processor has `cmpxchg16b`, which Clang announces with a
    /// macro for every `-march=` from `x86-64-v2` on.
    pub(crate) const fn has_compare_and_swap_16(self) -> bool {
        self.cpu.is_some()
    }

    /// LLVM's feature string, written into `buffer`, which is long enough for
    /// every feature at once.
    pub(crate) fn llvm_features(self, buffer: &mut [u8; 256]) -> &CStr {
        let mut length = 0;
        for feature in self.features() {
            for byte in [b"+", feature.llvm.as_bytes(), b","].concat_bytes() {
                buffer[length] = byte;
                length += 1;
            }
        }
        // The last comma becomes the terminator.
        let end = length.saturating_sub(1);
        buffer[end] = 0;
        CStr::from_bytes_with_nul(&buffer[..=end]).expect("a feature name holds no NUL")
    }

    fn features(self) -> impl Iterator<Item = &'static Feature> {
        FEATURES
            .iter()
            .enumerate()
            .filter(move |(index, _)| self.enabled & (1 << index) != 0)
            .map(|(_, feature)| feature)
    }
}

/// The bytes of several slices in order.
trait ConcatBytes {
    fn concat_bytes(self) -> impl Iterator<Item = u8>;
}

impl<const N: usize> ConcatBytes for [&[u8]; N] {
    fn concat_bytes(self) -> impl Iterator<Item = u8> {
        self.into_iter().flatten().copied()
    }
}

fn find(name: Option<&str>) -> Option<(u32, &'static Feature)> {
    // `-msse4` is the newer of the two.
    let name = match name? {
        | "sse4" => "sse4.2",
        | other => other,
    };
    FEATURES
        .iter()
        .enumerate()
        .find(|(_, feature)| feature.option == name)
        .map(|(index, feature)| (1 << index, feature))
}

/// The feature a `-mno-` option turns off. `-mno-sse4` is the older of the two.
fn no_option(option: &str) -> Option<&str> {
    match option.strip_prefix("-mno-")? {
        | "sse4" => Some("sse4.1"),
        | other => Some(other),
    }
}

/// Whether `text` is an option [`X86Features::apply`] reads.
pub(crate) fn is_feature_option(text: &str) -> bool {
    matches!(
        text,
        "-march=x86-64" | "-march=x86-64-v2" | "-march=x86-64-v3" | "-march=native"
    ) || find(text.strip_prefix("-m")).is_some()
        || find(text.strip_prefix("-mno-")).is_some()
}

/// The features of the processor running the compiler.
#[cfg(target_arch = "x86_64")]
fn native() -> u32 {
    let mut features = 0;
    for (found, bit) in [
        (std::is_x86_feature_detected!("sse3"), SSE3),
        (std::is_x86_feature_detected!("ssse3"), SSSE3),
        (std::is_x86_feature_detected!("sse4.1"), SSE4_1),
        (std::is_x86_feature_detected!("sse4.2"), SSE4_2 | CRC32),
        (std::is_x86_feature_detected!("popcnt"), POPCNT),
        (std::is_x86_feature_detected!("avx"), AVX),
        (std::is_x86_feature_detected!("avx2"), AVX2),
        (std::is_x86_feature_detected!("fma"), FMA),
        (std::is_x86_feature_detected!("f16c"), F16C),
        (std::is_x86_feature_detected!("bmi1"), BMI),
        (std::is_x86_feature_detected!("bmi2"), BMI2),
        (std::is_x86_feature_detected!("lzcnt"), LZCNT),
        (std::is_x86_feature_detected!("movbe"), MOVBE),
        (std::is_x86_feature_detected!("aes"), AES),
        (std::is_x86_feature_detected!("pclmulqdq"), PCLMUL),
        (std::is_x86_feature_detected!("xsave"), XSAVE),
    ] {
        if found {
            features |= bit;
        }
    }
    features
}

/// A compiler running elsewhere can't ask an x86-64 processor.
#[cfg(not(target_arch = "x86_64"))]
const fn native() -> u32 {
    0
}
