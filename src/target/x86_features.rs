//! The x86-64 features beyond the baseline that `-mavx2` and its kin ask for,
//! and that `-march=` names in groups. Each option turns on what its feature
//! needs and off what needs it, as GCC and Clang do, and the result decides the
//! predefined macros, what LLVM may generate, and, for AVX, how vectors of 32
//! bytes and more travel. Intrinsics do not depend on these: bcc's headers
//! compute them on any x86-64.
//!
//! The last `-march=` names defaults, which an explicit option outranks
//! wherever it stands: `-mno-avx -march=x86-64-v3` has no AVX, and `-mavx2
//! -march=x86-64` keeps AVX2. POPCNT and CRC32 come with SSE4.2 only by
//! default: neither is needed by it, so `-mavx -mno-popcnt` keeps AVX. XSAVE is
//! where the compilers part: AVX needs it in GCC, so `-mno-xsave` turns AVX
//! off, while in Clang it comes with AVX only by default, as POPCNT does, so
//! `-mno-xsave` leaves AVX on and turning AVX off takes XSAVE with it. These
//! are `i386-options.cc`'s `ix86_option_override_internal` in GCC and
//! `TargetInfo::initFeatureMap` in Clang (`clang/lib/Basic/Targets/X86.cpp`).
//! GCC manual, "x86 Options"; Clang Command Line Reference, "X86".

use std::ffi::CStr;

/// One feature: the option after `-m`, LLVM's name, its macro, and the
/// features it needs, which it turns on and which turning off takes it along.
struct Feature {
    option:  &'static str,
    llvm:    &'static str,
    defines: &'static str,
    needs:   u32,
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
    feature("sse4.2", "sse4.2", "__SSE4_2__", SSE4_1 | SSSE3 | SSE3),
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

const AVX_NEEDS: u32 = SSE4_2 | SSE4_1 | SSSE3 | SSE3 | XSAVE;
/// What SSE4.2 turns on unless the command line turned it off itself.
const SSE4_2_DEFAULTS: u32 = POPCNT | CRC32;
const V2: u32 = SSE4_2 | SSE4_1 | SSSE3 | SSE3 | POPCNT | SAHF;
const V3: u32 = V2 | AVX | AVX2 | FMA | F16C | BMI | BMI2 | LZCNT | MOVBE | XSAVE;

const fn feature(
    option: &'static str,
    llvm: &'static str,
    defines: &'static str,
    needs: u32,
) -> Feature {
    Feature {
        option,
        llvm,
        defines,
        needs,
    }
}

/// What the feature options said, in the order they came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct X86Features {
    /// The features explicit options left on.
    turned_on:    u32,
    /// The features an explicit option decided, either way: the last
    /// `-march=` has no say in these.
    decided:      u32,
    /// The features of the last `-march=`.
    architecture: u32,
    cpu:          Option<Cpu>,
    /// The last `-march=` names a processor this table doesn't model.
    unknown_cpu:  bool,
    /// A feature option outside this table that turns on AVX is in effect.
    unknown_avx:  bool,
    /// Whether Clang's rules hold where the compilers part, rather than GCC's.
    clang:        bool,
}

/// The option the GCC and Clang personalities give for a `-march=` or feature
/// option outside the table that is in effect at the end of the command line.
pub(crate) const UNKNOWN_VECTOR_ABI: &str = "--unknown-vector-abi";

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
        turned_on:    0,
        decided:      0,
        architecture: 0,
        cpu:          None,
        unknown_cpu:  false,
        unknown_avx:  false,
        clang:        false,
    };

    /// No options yet, under GCC's rules or Clang's.
    const fn for_compiler(clang: bool) -> Self {
        Self {
            clang,
            ..Self::NONE
        }
    }

    /// The features of a command line's `options`. An option with a later
    /// opposite doesn't count at all, as both compilers read them, so `-mavx
    /// -mno-avx` turns nothing on while `-mavx2 -mno-avx` leaves SSE4.2.
    /// GCC's `prune_options` and Clang's driver.
    pub(crate) fn from_options<'o>(
        options: &(impl Iterator<Item = &'o str> + Clone),
        clang: bool,
    ) -> Self {
        let mut features = Self::for_compiler(clang);
        for (index, option) in options.clone().enumerate() {
            let overridden = opposite_family(option, clang).is_some_and(|family| {
                options
                    .clone()
                    .skip(index + 1)
                    .any(|later| opposite_family(later, clang) == Some(family))
            });
            if !overridden {
                features.apply(option);
            }
        }
        features
    }

    /// Reads one option, if it is one of these. A later `-march=` replaces an
    /// earlier one's defaults and leaves the explicit `-m` options alone, as in
    /// both compilers.
    pub(crate) fn apply(&mut self, option: &str) {
        if option == UNKNOWN_VECTOR_ABI {
            self.unknown_avx = true;
        } else if let Some(architecture) = option.strip_prefix("-march=") {
            self.architecture(architecture);
        } else if let Some(name) = option.strip_prefix("-m") {
            self.machine(name);
        }
    }

    /// Reads the text of a `target` attribute, as `"avx2,no-bmi,arch=core2"`,
    /// which says the same as the command-line options.
    /// GCC manual, "x86 Function Attributes", `target`.
    pub(crate) fn apply_target_attribute(&mut self, text: &str) {
        for part in text.split(',') {
            let part = part.trim();
            if let Some(architecture) = part.strip_prefix("arch=") {
                self.architecture(architecture);
            } else if !part.contains('=') {
                self.machine(part);
            }
        }
    }

    fn architecture(&mut self, architecture: &str) {
        let (cpu, features) = match architecture {
            | "x86-64-v2" => (Some(Cpu::V2), V2 | self.architecture_crc32()),
            | "x86-64-v3" => (Some(Cpu::V3), V3 | self.architecture_crc32()),
            | "native" => (Some(Cpu::Native), native()),
            | _ => (None, 0),
        };
        self.cpu = cpu;
        self.architecture = features;
        self.unknown_cpu = !BASELINE_ARCHITECTURES.contains(&architecture) && cpu.is_none();
    }

    /// Reads `name`, what follows `-m`.
    fn machine(&mut self, name: &str) {
        if let Some((bit, feature)) = find(name) {
            let needs = if self.clang {
                feature.needs & !XSAVE
            } else {
                feature.needs
            };
            let on = bit | needs;
            self.turned_on |= on;
            self.decided |= on;
        } else if let Some((bit, _)) = name.strip_prefix("no-").and_then(find_off) {
            let off = if self.clang && bit == XSAVE {
                bit
            } else {
                bit | what_needs(bit)
            };
            self.turned_on &= !off;
            self.decided |= off;
            // What turns on AVX beyond the table needs what these turn off.
            if off & AVX != 0 {
                self.unknown_avx = false;
            }
        } else if turns_on_avx(name) {
            self.unknown_avx = true;
        } else if name.strip_prefix("no-").is_some_and(turns_on_avx) {
            self.unknown_avx = false;
        }
    }

    /// Whether a `target` attribute's `text` changes how vectors of 32 bytes
    /// and more travel from what these options give.
    pub(crate) fn target_changes_vector_abi(self, text: &str) -> bool {
        let mut changed = self;
        changed.apply_target_attribute(text);
        changed.avx() != self.avx() || changed.unknown_vector_abi() != self.unknown_vector_abi()
    }

    /// The features in effect.
    const fn enabled(self) -> u32 {
        let mut enabled = self.turned_on | (self.architecture & !self.decided);
        if enabled & SSE4_2 != 0 {
            enabled |= SSE4_2_DEFAULTS & !self.decided;
        }
        if self.clang && enabled & AVX != 0 {
            enabled |= XSAVE & !self.decided;
        }
        enabled
    }

    /// `-march=x86-64-v2` has CRC32 of its own in Clang, and from SSE4.2 alone
    /// in GCC.
    const fn architecture_crc32(self) -> u32 {
        if self.clang { CRC32 } else { 0 }
    }

    pub(crate) const fn cpu(self) -> Option<Cpu> {
        self.cpu
    }

    /// Whether vectors of 32 bytes travel whole in YMM registers.
    /// System V AMD64 psABI §3.2.3, "Classification", rule 1, `__m256`.
    pub(crate) const fn avx(self) -> bool {
        self.enabled() & AVX != 0
    }

    /// Whether another compiler given these options would pass vectors of 32
    /// bytes or wider, or of 64 bytes, differently from what this table knows:
    /// a `-march=` or feature option bcc only warns about, or the native
    /// processor's AVX-512, which gives 64-byte vectors ZMM registers. The
    /// smallest vector whose convention bcc doesn't know, in bytes.
    pub(crate) fn unknown_vector_abi(self) -> Option<u64> {
        if self.unknown_cpu || self.unknown_avx {
            Some(32)
        } else if self.cpu == Some(Cpu::Native) && self.avx() && native_avx512() {
            Some(64)
        } else {
            None
        }
    }

    /// The macros of the features in effect.
    pub(crate) fn macros(self) -> impl Iterator<Item = &'static str> {
        self.features().map(|feature| feature.defines)
    }

    /// Whether code may use fused multiply-add, which GCC announces with its
    /// `__FP_FAST_FMA` macros.
    pub(crate) const fn fast_fma(self) -> bool {
        self.enabled() & FMA != 0
    }

    /// Whether the processor has `cmpxchg16b`, which both compilers announce
    /// with a macro for every `-march=` from `x86-64-v2` on.
    pub(crate) const fn has_compare_and_swap_16(self) -> bool {
        self.cpu.is_some()
    }

    /// LLVM's feature string, written into `buffer`, which is long enough for
    /// every feature at once. An explicit `-mno-avx` names `-avx`, since the
    /// processor may have it: LLVM takes what needs a feature along with it,
    /// and what `-march=` turned on is what the features turned on say.
    pub(crate) fn llvm_features(self, buffer: &mut [u8; 256]) -> &CStr {
        let mut length = 0;
        let enabled = self.enabled();
        let off = self.decided & !enabled;
        for (index, feature) in FEATURES.iter().enumerate() {
            let sign: &[u8] = match (off >> index & 1, enabled >> index & 1) {
                | (1, _) => b"-",
                | (_, 1) => b"+",
                | _ => continue,
            };
            for byte in [sign, feature.llvm.as_bytes(), b","].concat_bytes() {
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
        let enabled = self.enabled();
        FEATURES
            .iter()
            .enumerate()
            .filter(move |(index, _)| enabled & (1 << index) != 0)
            .map(|(_, feature)| feature)
    }
}

/// What a table option and its opposite have in common, so that the later one
/// overrides the earlier. `-msse4` and `-mno-sse4` aren't opposites in GCC; in
/// Clang they are `-msse4.2` and `-mno-sse4.1`.
fn opposite_family(option: &str, clang: bool) -> Option<(&str, bool)> {
    let name = option.strip_prefix("-m")?;
    let (negative, feature) = match name.strip_prefix("no-") {
        | Some(feature) => (true, feature),
        | None => (false, name),
    };
    _ = find(feature)?;
    Some(match (feature, clang, negative) {
        | ("sse4", true, true) => ("sse4.1", false),
        | ("sse4", true, false) => ("sse4.2", false),
        | ("sse4", false, _) => ("sse4", negative),
        | _ => (feature, false),
    })
}

/// The features that need any of `bits`.
fn what_needs(bits: u32) -> u32 {
    FEATURES
        .iter()
        .enumerate()
        .filter(|(_, feature)| feature.needs & bits != 0)
        .fold(0, |needing, (index, _)| needing | 1 << index)
}

/// The names of the baseline processor, which `unimplemented.rs` also ignores
/// without a warning.
const BASELINE_ARCHITECTURES: [&str; 5] = ["x86-64", "x86-64-v1", "k8", "opteron", "athlon64"];

/// Whether the `-m` option `name`, outside the table, turns on AVX in GCC 14 or
/// Clang: `-dM -E` defines `__AVX__` for these in at least one of them.
fn turns_on_avx(name: &str) -> bool {
    name.starts_with("avx")
        || matches!(
            name,
            "amx-avx512" | "fma4" | "sha512" | "sm3" | "sm4" | "vaes" | "vpclmulqdq" | "xop"
        )
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

/// The feature `-m` followed by `name` turns on. `-msse4` is the newer of the
/// two.
fn find(name: &str) -> Option<(u32, &'static Feature)> {
    let name = match name {
        | "sse4" => "sse4.2",
        | other => other,
    };
    FEATURES
        .iter()
        .enumerate()
        .find(|(_, feature)| feature.option == name)
        .map(|(index, feature)| (1 << index, feature))
}

/// The feature `-mno-` followed by `name` turns off. `-mno-sse4` is the older
/// of the two.
fn find_off(name: &str) -> Option<(u32, &'static Feature)> {
    find(if name == "sse4" { "sse4.1" } else { name })
}

/// Whether `text` is an option this table implements, rather than the options
/// [`X86Features::apply`] only notes.
pub(crate) fn is_feature_option(text: &str) -> bool {
    matches!(
        text,
        "-march=x86-64" | "-march=x86-64-v2" | "-march=x86-64-v3" | "-march=native"
    ) || text.strip_prefix("-m").and_then(find).is_some()
        || text.strip_prefix("-mno-").and_then(find_off).is_some()
}

/// The features of the processor running the compiler.
#[cfg(target_arch = "x86_64")]
fn native() -> u32 {
    let mut features = 0;
    for (found, bit) in [
        (std::is_x86_feature_detected!("sse3"), SSE3),
        (std::is_x86_feature_detected!("ssse3"), SSSE3),
        (std::is_x86_feature_detected!("sse4.1"), SSE4_1),
        (std::is_x86_feature_detected!("sse4.2"), SSE4_2),
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

/// Whether the processor running the compiler has AVX-512.
#[cfg(target_arch = "x86_64")]
fn native_avx512() -> bool {
    std::is_x86_feature_detected!("avx512f")
}

/// A compiler running elsewhere can't ask an x86-64 processor.
#[cfg(not(target_arch = "x86_64"))]
const fn native() -> u32 {
    0
}

/// A compiler running elsewhere can't ask an x86-64 processor.
#[cfg(not(target_arch = "x86_64"))]
const fn native_avx512() -> bool {
    false
}
