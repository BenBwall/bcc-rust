//! The x86-64 features beyond the baseline that `-mavx2` and its kin ask for,
//! that `-march=` names in groups, and that a function's `target` attribute
//! changes for its own code. Each option turns on what its feature needs and
//! off what needs it, as GCC and Clang do, and the result decides the
//! predefined macros, what LLVM may generate, and, for AVX, how vectors of 32
//! bytes and more travel. Intrinsics do not depend on the command line: bcc's
//! headers compute them on any x86-64, and only those whose header function
//! carries a `target` attribute ask the calling function for its features.
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
//! The command line implements 18 of the features ([`COMMAND_LINE`]); the
//! others are options bcc only warns about.
//!
//! A `target` attribute (`"avx2,no-bmi,arch=haswell,tune=znver3"`) reads
//! like those options, from the command line's features, over every feature
//! and processor either compiler knows: `table.rs` holds what each compiler
//! accepts and turns on, measured by `scripts/measure_x86_target_attribute.py`.
//! Each compiler reads the text its own way ([`target_attribute_faults`]):
//! Clang trims each part and ignores the whole attribute, with a warning,
//! for one it does not know; GCC takes the parts as written and refuses one
//! it does not know. In Clang `arch=` replaces the processor's defaults; in
//! GCC it also forgets every feature turned on or off before it, the command
//! line's too.
//! GCC manual, "x86 Options" and "x86 Function Attributes"; Clang Command Line
//! Reference, "X86"; Clang Attributes Reference, "target".

mod table;

use std::{
    ffi::CStr,
    fmt,
};

use table::{
    AES,
    AVX,
    AVX2,
    AVX512F,
    BMI,
    BMI2,
    BUILTINS,
    CRC32,
    F16C,
    FEATURES,
    FMA,
    LZCNT,
    MOVBE,
    PCLMUL,
    POPCNT,
    PROCESSORS,
    SAHF,
    SSE3,
    SSE4_1,
    SSE4_2,
    SSSE3,
    XSAVE,
};

/// One feature: its name in an option or attribute, LLVM's name, its macro,
/// whether GCC and Clang know it, and, for GCC and then Clang, the features
/// it needs, which it turns on and which turning off takes it along, and those
/// it turns on unless an option decided them.
struct Feature {
    name:       &'static str,
    llvm:       Option<&'static str>,
    macro_name: &'static str,
    gcc:        bool,
    clang:      bool,
    needs:      [u128; 2],
    defaults:   [u128; 2],
}

/// A processor `arch=` or `tune=` names: LLVM's names for it, whether each of
/// GCC and Clang takes it after `arch=` and after `tune=`, and its features
/// in each.
struct Processor {
    name:      &'static str,
    llvm:      Option<&'static str>,
    llvm_tune: Option<&'static str>,
    arch:      [bool; 2],
    tune:      [bool; 2],
    features:  [u128; 2],
}

/// The features `-m` options implement, in the order bcc defines their
/// macros and writes LLVM's feature string.
const COMMAND_LINE: [u128; 18] = [
    SSE3, SSSE3, SSE4_1, SSE4_2, POPCNT, CRC32, AVX, AVX2, FMA, F16C, BMI, BMI2, LZCNT, MOVBE, AES,
    PCLMUL, XSAVE, SAHF,
];

const COMMAND_LINE_MASK: u128 = SSE3
    | SSSE3
    | SSE4_1
    | SSE4_2
    | POPCNT
    | CRC32
    | AVX
    | AVX2
    | FMA
    | F16C
    | BMI
    | BMI2
    | LZCNT
    | MOVBE
    | AES
    | PCLMUL
    | XSAVE
    | SAHF;

const ALL_FEATURES: u128 = u128::MAX >> (128 - FEATURES.len());

const V2: u128 = SSE4_2 | SSE4_1 | SSSE3 | SSE3 | POPCNT | SAHF;
const V3: u128 = V2 | AVX | AVX2 | FMA | F16C | BMI | BMI2 | LZCNT | MOVBE | XSAVE;

/// The features x86-64 always has. An attribute may name them; turning one
/// off is something bcc does not compile ([`FunctionTarget::refused`]).
const BASELINE: [&str; 5] = ["x87", "mmx", "fxsr", "sse", "sse2"];

/// What the feature options said, in the order they came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct X86Features {
    /// The features explicit options left on.
    turned_on:    u128,
    /// The features an explicit option decided, either way: the last
    /// `-march=` has no say in these.
    decided:      u128,
    /// The features of the last `-march=`.
    architecture: u128,
    cpu:          Option<Cpu>,
    /// The last `-march=` names a processor this table doesn't model.
    unknown_cpu:  bool,
    /// A feature option outside this table turned on AVX, which nothing has
    /// turned off since. Turning off the option's kin leaves AVX on.
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
    /// The processor of a `target` attribute's `arch=`, by its row in the
    /// processor table.
    Named(u8),
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
    /// [`UNKNOWN_VECTOR_ABI`] is the driver's reading of options it has already
    /// read this way, so no option beside it takes it back. Explicitly turning
    /// off AVX decides how vectors travel whatever an unknown `-march=` holds.
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
        if features.decided & AVX != 0 && features.turned_on & AVX == 0 {
            features.unknown_cpu = false;
        }
        features.unknown_avx |= options.clone().any(|option| option == UNKNOWN_VECTOR_ABI);
        features
    }

    /// Reads one option, if it is one of these. A later `-march=` replaces an
    /// earlier one's defaults and leaves the explicit `-m` options alone, as in
    /// both compilers.
    fn apply(&mut self, option: &str) {
        if let Some(architecture) = option.strip_prefix("-march=") {
            self.architecture(architecture);
        } else if let Some(name) = option.strip_prefix("-m")
            && self.machine(name, COMMAND_LINE_MASK).is_none()
            && turns_on_avx(name)
        {
            self.unknown_avx = true;
        }
    }

    /// Reads a `target` attribute's `text`, which [`target_attribute_faults`]
    /// found nothing wrong with, over these features, as far as a NUL, as GCC
    /// reads a C string.
    /// GCC manual, "x86 Function Attributes", `target`; Clang Attributes
    /// Reference, "target".
    pub(crate) fn target_attribute(self, text: &str) -> FunctionTarget {
        let mut target = FunctionTarget {
            features: self,
            named:    0,
            tune:     None,
            refused:  false,
        };
        target.read(text);
        target
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

    /// Reads `name`, what follows `-m` or a part of a `target` attribute, if it
    /// names a feature of `allowed`: the feature, and whether it is on.
    fn machine(&mut self, name: &str, allowed: u128) -> Option<(u128, bool)> {
        let compiler = usize::from(self.clang);
        if let Some(bit) = find(name, allowed) {
            let on = bit | FEATURES[index(bit)].needs[compiler];
            self.turned_on |= on;
            self.decided |= on;
            Some((bit, true))
        } else if let Some(bit) = name
            .strip_prefix("no-")
            .and_then(|name| find_off(name, allowed))
        {
            let off = bit | what_needs(bit, compiler);
            self.turned_on &= !off;
            self.decided |= off;
            // What turns on AVX beyond the table needs what these turn off.
            if off & AVX != 0 {
                self.unknown_avx = false;
            }
            Some((bit, false))
        } else {
            None
        }
    }

    /// Whether a `target` attribute's `text` changes how vectors of 32 bytes
    /// and more travel from what these options give.
    pub(crate) fn target_changes_vector_abi(self, target: &FunctionTarget) -> bool {
        let changed = target.features;
        changed.avx() != self.avx() || changed.unknown_vector_abi() != self.unknown_vector_abi()
    }

    /// The features in effect: those explicitly turned on, the processor's
    /// that no option decided, and what those bring by default.
    const fn enabled(self) -> u128 {
        let compiler = self.clang as usize;
        let mut enabled = self.turned_on | (self.architecture & !self.decided);
        let mut rest = enabled;
        while rest != 0 {
            let bit = rest.trailing_zeros() as usize;
            rest &= rest - 1;
            enabled |= FEATURES[bit].defaults[compiler] & !self.decided;
        }
        enabled
    }

    /// `-march=x86-64-v2` has CRC32 of its own in Clang, and from SSE4.2 alone
    /// in GCC.
    const fn architecture_crc32(self) -> u128 {
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
    /// a `-march=` or feature option bcc only warns about, or AVX-512, which
    /// gives 64-byte vectors ZMM registers. The smallest vector whose
    /// convention bcc doesn't know, in bytes.
    pub(crate) fn unknown_vector_abi(self) -> Option<u64> {
        if self.unknown_cpu || self.unknown_avx {
            Some(32)
        } else if self.avx()
            && (self.enabled() & AVX512F != 0 || (self.cpu == Some(Cpu::Native) && native_avx512()))
        {
            Some(64)
        } else {
            None
        }
    }

    /// The macros of the features in effect that the command line implements.
    pub(crate) fn macros(self) -> impl Iterator<Item = &'static str> {
        let enabled = self.enabled();
        COMMAND_LINE
            .into_iter()
            .filter(move |&bit| enabled & bit != 0)
            .map(|bit| FEATURES[index(bit)].macro_name)
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

    /// LLVM's feature string for the command line, written into `buffer`,
    /// which is long enough for every feature at once. An explicit `-mno-avx`
    /// names `-avx`, since the processor may have it: LLVM takes what needs a
    /// feature along with it, and what `-march=` turned on is what the features
    /// turned on say.
    pub(crate) fn llvm_features(self, buffer: &mut [u8; 256]) -> &CStr {
        let mut length = 0;
        let enabled = self.enabled();
        let off = self.decided & !enabled;
        for bit in COMMAND_LINE {
            let sign: &[u8] = match (off & bit != 0, enabled & bit != 0) {
                | (true, _) => b"-",
                | (_, true) => b"+",
                | _ => continue,
            };
            let name = FEATURES[index(bit)].llvm.unwrap_or_default();
            for byte in [sign, name.as_bytes(), b","].concat_bytes() {
                buffer[length] = byte;
                length += 1;
            }
        }
        // The last comma becomes the terminator.
        let end = length.saturating_sub(1);
        buffer[end] = 0;
        CStr::from_bytes_with_nul(&buffer[..=end]).expect("a feature name holds no NUL")
    }
}

/// A function's features: the command line's with its `target` attribute read
/// over them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FunctionTarget {
    features: X86Features,
    /// The features the attribute itself turned on.
    named:    u128,
    /// The processor of `tune=`, by its row in the processor table.
    tune:     Option<u8>,
    /// The attribute turns off a baseline feature or asks for general
    /// registers only, which bcc does not compile.
    refused:  bool,
}

impl FunctionTarget {
    /// Reads `text` part by part, in order.
    fn read(&mut self, text: &str) {
        let text = text.split('\0').next().unwrap_or(text);
        let clang = self.features.clang;
        for part in parts(text, clang) {
            let (negated, name) = match part.strip_prefix("no-") {
                | Some(name) => (true, name),
                | None => (false, part),
            };
            if let Some(name) = name.strip_prefix("arch=") {
                if let Some(row) = processor(processor_name(name, clang), clang, false) {
                    self.arch(row);
                }
            } else if let Some(name) = name.strip_prefix("tune=") {
                if let Some(row) = processor(processor_name(name, clang), clang, true) {
                    self.tune = Some(row);
                }
            } else if BASELINE.contains(&name) || name == "general-regs-only" {
                self.refused |= negated != (name == "general-regs-only");
            } else {
                match self.features.machine(part, ALL_FEATURES) {
                    | Some((bit, true)) => self.named |= bit,
                    | Some((_, false)) => self.named &= self.features.turned_on,
                    | None => {},
                }
            }
        }
    }

    /// `arch=` names the processor in table row `row`: its features become
    /// the defaults, and in GCC every feature an option decided before is
    /// forgotten, as `ix86_valid_target_attribute_inner_p` clears them.
    fn arch(&mut self, row: u8) {
        let features = &mut self.features;
        let compiler = usize::from(features.clang);
        features.cpu = Some(Cpu::Named(row));
        features.architecture = PROCESSORS[usize::from(row)].features[compiler];
        features.unknown_cpu = false;
        if !features.clang {
            features.turned_on = 0;
            features.decided = 0;
            features.unknown_avx = false;
            self.named = 0;
        }
    }

    /// These features with GCC's next `target` attribute of the same
    /// declaration, `text`, read over them.
    pub(crate) fn extended(&self, text: &str) -> Self {
        let mut target = *self;
        target.read(text);
        target
    }

    /// Whether bcc refuses to compile the function: the attribute turns off
    /// x87, MMX, FXSR, SSE or SSE2, which every x86-64 function bcc compiles
    /// may use, or asks for general registers only.
    pub(crate) const fn refused(&self) -> bool {
        self.refused
    }

    /// What the back ends read.
    pub(crate) const fn code(&self) -> CodeFeatures {
        let enabled = self.features.enabled();
        CodeFeatures {
            enabled,
            disabled: self.features.decided & !enabled,
            cpu: match self.features.cpu {
                | Some(Cpu::Named(row)) => Some(row),
                | _ => None,
            },
            tune: self.tune,
        }
    }

    /// The feature the x86 builtin `name` needs that this function lacks, if
    /// any, as Clang measured it.
    pub(crate) fn lacks_builtin_feature(&self, name: &str) -> Option<&'static str> {
        let row = BUILTINS
            .binary_search_by(|&(builtin, _)| builtin.cmp(name))
            .ok()?;
        let bit = BUILTINS[row].1;
        (self.features.enabled() & bit == 0).then(|| FEATURES[index(bit)].name)
    }

    /// The first feature `callee` uses that this function lacks, if any, which
    /// Clang names when it refuses to inline an `always_inline` function: one
    /// the callee's attribute itself named, if there is one.
    pub(crate) fn missing(&self, callee: &Self) -> Option<&'static str> {
        let missing = callee.features.enabled() & !self.features.enabled();
        let first = if missing & callee.named != 0 {
            missing & callee.named
        } else {
            missing
        };
        (first != 0).then(|| FEATURES[first.trailing_zeros() as usize].name)
    }
}

/// What a function's code may use, as the IR and the back ends keep it: the
/// features on and those explicitly off, and the processors of `arch=` and
/// `tune=`, by their rows in the processor table. LLVM takes the rest from
/// the command line's processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CodeFeatures {
    enabled:  u128,
    disabled: u128,
    cpu:      Option<u8>,
    tune:     Option<u8>,
}

impl CodeFeatures {
    /// Whether this function's code may use everything `callee`'s does, so
    /// that the callee may be expanded into it, as LLVM's `areInlineCompatible`
    /// compares the subtargets' features.
    pub(crate) const fn includes(&self, callee: &Self) -> bool {
        callee.enabled & !self.enabled == 0
    }

    /// LLVM's names of the features on (`true`) and explicitly off, in table
    /// order; GCC's features without an LLVM name are left out.
    pub(crate) fn llvm_features(&self) -> impl Iterator<Item = (bool, &'static str)> {
        let (enabled, disabled) = (self.enabled, self.disabled);
        FEATURES
            .iter()
            .enumerate()
            .filter_map(move |(bit, feature)| {
                let on = enabled >> bit & 1 != 0;
                let off = disabled >> bit & 1 != 0;
                feature.llvm.filter(|_| on || off).map(|name| (on, name))
            })
    }

    /// LLVM's name of the `arch=` processor; a processor LLVM does not know
    /// stands as baseline x86-64, whose features the feature list completes.
    pub(crate) fn llvm_cpu(&self) -> Option<&'static str> {
        self.cpu
            .map(|row| PROCESSORS[usize::from(row)].llvm.unwrap_or("x86-64"))
    }

    /// LLVM's name of the `tune=` processor, if LLVM knows it.
    pub(crate) fn llvm_tune(&self) -> Option<&'static str> {
        self.tune
            .and_then(|row| PROCESSORS[usize::from(row)].llvm_tune)
    }

    /// The names the IR text gives these: each feature on or off, with `+`
    /// or `-`, and `arch=` and `tune=` with their processor names.
    pub(crate) fn parts(&self) -> impl Iterator<Item = (&'static str, &'static str)> {
        let (enabled, disabled) = (self.enabled, self.disabled);
        let features = FEATURES
            .iter()
            .enumerate()
            .filter_map(move |(bit, feature)| {
                if enabled >> bit & 1 != 0 {
                    Some(("+", feature.name))
                } else if disabled >> bit & 1 != 0 {
                    Some(("-", feature.name))
                } else {
                    None
                }
            });
        let cpu = self
            .cpu
            .map(|row| ("arch=", PROCESSORS[usize::from(row)].name));
        let tune = self
            .tune
            .map(|row| ("tune=", PROCESSORS[usize::from(row)].name));
        features.chain(cpu).chain(tune)
    }

    /// Adds one of [`Self::parts`] back, as the IR parser reads it, or says
    /// it names nothing.
    pub(crate) fn add_part(&mut self, sign: &str, name: &str) -> bool {
        let row = |name: &str| {
            PROCESSORS
                .iter()
                .position(|processor| processor.name == name)
                .and_then(|row| u8::try_from(row).ok())
        };
        match sign {
            | "arch=" => self.cpu = row(name),
            | "tune=" => self.tune = row(name),
            | "+" | "-" => {
                let Some(bit) = FEATURES.iter().position(|feature| feature.name == name) else {
                    return false;
                };
                if sign == "+" {
                    self.enabled |= 1 << bit;
                } else {
                    self.disabled |= 1 << bit;
                }
                return true;
            },
            | _ => return false,
        }
        row(name).is_some()
    }
}

/// The parts of a `target` attribute's `text`: Clang trims each and skips
/// none, while GCC takes each as written and stops at a trailing comma.
fn parts(text: &str, clang: bool) -> impl Iterator<Item = &str> {
    let trailing = !clang && text.ends_with(',');
    let text = if trailing {
        &text[..text.len() - 1]
    } else {
        text
    };
    text.split(',')
        .map(move |part| if clang { part.trim() } else { part })
        .filter(move |_| clang || !text.is_empty())
}

/// The processor `name` after `arch=` or `tune=`, which Clang trims.
fn processor_name(name: &str, clang: bool) -> &str {
    if clang { name.trim() } else { name }
}

/// The row of the processor `name`, if the compiler of `clang` takes it after
/// `arch=` (or `tune=`, with `tune`).
fn processor(name: &str, clang: bool, tune: bool) -> Option<u8> {
    let compiler = usize::from(clang);
    PROCESSORS
        .iter()
        .position(|processor| {
            processor.name == name
                && if tune {
                    processor.tune[compiler]
                } else {
                    processor.arch[compiler]
                }
        })
        .and_then(|row| u8::try_from(row).ok())
}

/// What the compiler of `clang` finds wrong with a `target` attribute's
/// `text`: everything GCC refuses, in order, or the one thing for which Clang
/// ignores the attribute. A whole text of `default`, which names the default
/// version of a multiversioned function, is the caller's to handle.
pub(crate) fn target_attribute_faults(
    text: &str,
    clang: bool,
) -> impl Iterator<Item = TargetFault<'_>> + '_ {
    let text = text.split('\0').next().unwrap_or(text);
    let gcc = (!clang).then(|| gcc_faults(text)).into_iter().flatten();
    clang
        .then(|| clang_fault(text))
        .flatten()
        .into_iter()
        .chain(gcc)
}

/// Clang's `checkTargetAttr`: `fpmath=` anywhere, then an unknown processor,
/// tune processor, a repeated `arch=` or `tune=`, and the first unknown
/// feature, as `TargetInfo::parseTargetAttr` splits the text.
fn clang_fault(text: &str) -> Option<TargetFault<'_>> {
    if text.contains("fpmath=") {
        return Some(TargetFault::Unsupported("fpmath="));
    }
    let (mut cpu, mut tune, mut duplicate) = (None, None, None);
    for part in parts(text, true) {
        for (prefix, slot) in [("arch=", &mut cpu), ("tune=", &mut tune)] {
            if let Some(value) = part.strip_prefix(prefix) {
                if slot.is_some() {
                    duplicate = duplicate.or(Some(prefix));
                } else {
                    *slot = Some(value.trim());
                }
            }
        }
    }
    if let Some(cpu) = cpu.filter(|cpu| !cpu.is_empty() && processor(cpu, true, false).is_none()) {
        return Some(TargetFault::UnknownCpu(cpu));
    }
    if let Some(tune) =
        tune.filter(|tune| !tune.is_empty() && processor(tune, true, true).is_none())
    {
        return Some(TargetFault::UnknownTune(tune));
    }
    if let Some(prefix) = duplicate {
        return Some(TargetFault::Duplicate(prefix));
    }
    parts(text, true)
        .filter(|part| !part.starts_with("arch=") && !part.starts_with("tune="))
        .map(|part| match part.split_once('-') {
            | Some(("no", name)) => name,
            | _ => part,
        })
        .find(|&name| !known(name, true) && !matches!(name, "general-regs-only" | "prefer-256-bit"))
        .map(TargetFault::Unsupported)
}

/// GCC's `ix86_valid_target_attribute_inner_p`, then the processors that
/// `ix86_option_override_internal` refuses.
fn gcc_faults(text: &str) -> impl Iterator<Item = TargetFault<'_>> + '_ {
    let mut seen = [false; 2];
    let parsed = parts(text, false).filter_map(move |part| {
        let name = part.strip_prefix("no-").unwrap_or(part);
        for (slot, prefix) in ["arch=", "tune="].into_iter().enumerate() {
            if let Some(value) = name.strip_prefix(prefix) {
                if value.is_empty() {
                    return Some(TargetFault::UnknownArgument(part));
                }
                if seen[slot] {
                    return Some(TargetFault::AlreadySpecified(part));
                }
                seen[slot] = true;
                return None;
            }
        }
        if let Some(value) = name.strip_prefix("fpmath=") {
            return (!matches!(value, "sse" | "387" | "sse+387" | "387+sse" | "both"))
                .then_some(TargetFault::UnknownValue(part));
        }
        if let Some(value) = name.strip_prefix("prefer-vector-width=") {
            return (!matches!(value, "128" | "256" | "512" | "none"))
                .then_some(TargetFault::UnknownValue(part));
        }
        if name == "general-regs-only" {
            return (name != part).then_some(TargetFault::NegatedForm("general-regs-only"));
        }
        (!known(name, false) && !GCC_OPTIONS.contains(&name))
            .then_some(TargetFault::UnknownArgument(part))
    });
    let mut first = [true; 2];
    let processors = parts(text, false).filter_map(move |part| {
        let name = part.strip_prefix("no-").unwrap_or(part);
        for (slot, prefix) in ["arch", "tune"].into_iter().enumerate() {
            if let Some(value) = name
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('='))
                .filter(|value| !value.is_empty())
            {
                let valid = processor(value, false, slot == 1).is_some();
                let report = first[slot] && !valid;
                first[slot] = false;
                return report.then_some(TargetFault::BadValue(prefix, value));
            }
        }
        None
    });
    parsed.chain(processors)
}

/// The options GCC takes in an attribute that bcc has nothing to do for:
/// they choose code, not instructions it may use.
const GCC_OPTIONS: [&str; 8] = [
    "align-stringops",
    "cld",
    "fancy-math-387",
    "ieee-fp",
    "inline-all-stringops",
    "inline-stringops-dynamically",
    "recip",
    "relax-cmpxchg-loop",
];

/// Whether `name` is a feature the compiler of `clang` knows.
fn known(name: &str, clang: bool) -> bool {
    name == "sse4"
        || (BASELINE.contains(&name) && (clang || name != "x87"))
        || FEATURES
            .iter()
            .any(|feature| feature.name == name && if clang { feature.clang } else { feature.gcc })
}

/// Why a compiler does not take a `target` attribute's text, in its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetFault<'t> {
    /// Clang: a part it does not know, so it ignores the attribute.
    Unsupported(&'t str),
    /// Clang: an `arch=` processor it does not know.
    UnknownCpu(&'t str),
    /// Clang: a `tune=` processor it does not know.
    UnknownTune(&'t str),
    /// Clang: a second `arch=` or `tune=`.
    Duplicate(&'static str),
    /// GCC: a part it does not know.
    UnknownArgument(&'t str),
    /// GCC: a second `arch=` or `tune=`.
    AlreadySpecified(&'t str),
    /// GCC: an `fpmath=` or `prefer-vector-width=` value it does not know.
    UnknownValue(&'t str),
    /// GCC: `no-general-regs-only`.
    NegatedForm(&'static str),
    /// GCC: a processor `arch` or `tune` does not take.
    BadValue(&'static str, &'t str),
}

impl TargetFault<'_> {
    /// Whether the compiler refuses the program, as GCC does, rather than
    /// ignoring the attribute with a warning, as Clang does.
    pub(crate) const fn is_error(self) -> bool {
        !matches!(
            self,
            Self::Unsupported(_) | Self::UnknownCpu(_) | Self::UnknownTune(_) | Self::Duplicate(_)
        )
    }
}

impl fmt::Display for TargetFault<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const IGNORED: &str = "in the 'target' attribute string; 'target' attribute ignored";
        match *self {
            | Self::Unsupported(part) => write!(f, "unsupported '{part}' {IGNORED}"),
            | Self::UnknownCpu(cpu) => write!(f, "unknown CPU '{cpu}' {IGNORED}"),
            | Self::UnknownTune(cpu) => write!(f, "unknown tune CPU '{cpu}' {IGNORED}"),
            | Self::Duplicate(prefix) => write!(f, "duplicate '{prefix}' {IGNORED}"),
            | Self::UnknownArgument(part) =>
                write!(f, "attribute 'target' argument '{part}' is unknown"),
            | Self::AlreadySpecified(part) => write!(
                f,
                "attribute value '{part}' was already specified in 'target' attribute"
            ),
            | Self::UnknownValue(part) => write!(
                f,
                "attribute value '{part}' is unknown in 'target' attribute"
            ),
            | Self::NegatedForm(name) => write!(
                f,
                "pragma or attribute 'target(\"{name}\")' does not allow a negated form"
            ),
            | Self::BadValue(prefix, value) => write!(
                f,
                "bad value '{value}' for 'target(\"{prefix}=\")' attribute"
            ),
        }
    }
}

/// What an option this file reads and its opposite have in common, so that the
/// later one overrides the earlier. `-msse4` and `-mno-sse4` aren't opposites
/// in GCC; in Clang they are `-msse4.2` and `-mno-sse4.1`.
fn opposite_family(option: &str, clang: bool) -> Option<(&str, bool)> {
    let name = option.strip_prefix("-m")?;
    let (negative, feature) = match name.strip_prefix("no-") {
        | Some(feature) => (true, feature),
        | None => (false, name),
    };
    if find(feature, COMMAND_LINE_MASK).is_none() && !turns_on_avx(feature) {
        return None;
    }
    Some(match (feature, clang, negative) {
        | ("sse4", true, true) => ("sse4.1", false),
        | ("sse4", true, false) => ("sse4.2", false),
        | ("sse4", false, _) => ("sse4", negative),
        | _ => (feature, false),
    })
}

/// The features that need `bit` in the compiler of `compiler`.
fn what_needs(bit: u128, compiler: usize) -> u128 {
    FEATURES
        .iter()
        .enumerate()
        .filter(|(_, feature)| feature.needs[compiler] & bit != 0)
        .fold(0, |needing, (index, _)| needing | 1 << index)
}

/// The table row of the feature with the single bit `bit`.
const fn index(bit: u128) -> usize {
    bit.trailing_zeros() as usize
}

/// The names of the baseline processor, which `unimplemented.rs` also ignores
/// without a warning.
const BASELINE_ARCHITECTURES: [&str; 5] = ["x86-64", "x86-64-v1", "k8", "opteron", "athlon64"];

/// Whether the `-m` option `name`, outside the table, turns on AVX in GCC 14 or
/// Clang: `-dM -E` defines `__AVX__` for these in at least one of them. GCC's
/// `-mavx256-split-unaligned-*` only tune code, and its `-msse5` is `-mavx`.
fn turns_on_avx(name: &str) -> bool {
    (name.starts_with("avx") && !name.starts_with("avx256-split-unaligned-"))
        || matches!(
            name,
            "amx-avx512"
                | "fma4"
                | "sha512"
                | "sm3"
                | "sm4"
                | "sse5"
                | "vaes"
                | "vpclmulqdq"
                | "xop"
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

/// The feature of `allowed` that `-m` or an attribute part `name` turns on.
/// `sse4` is the newer of the two.
fn find(name: &str, allowed: u128) -> Option<u128> {
    let name = match name {
        | "sse4" => "sse4.2",
        | other => other,
    };
    FEATURES
        .iter()
        .position(|feature| feature.name == name)
        .map(|index| 1 << index)
        .filter(|bit| allowed & bit != 0)
}

/// The feature `no-` followed by `name` turns off. `no-sse4` is the older of
/// the two.
fn find_off(name: &str, allowed: u128) -> Option<u128> {
    find(if name == "sse4" { "sse4.1" } else { name }, allowed)
}

/// Whether `text` is an option this table implements, rather than the options
/// [`X86Features::apply`] only notes.
pub(crate) fn is_feature_option(text: &str) -> bool {
    matches!(
        text,
        "-march=x86-64" | "-march=x86-64-v2" | "-march=x86-64-v3" | "-march=native"
    ) || text
        .strip_prefix("-m")
        .and_then(|name| find(name, COMMAND_LINE_MASK))
        .is_some()
        || text
            .strip_prefix("-mno-")
            .and_then(|name| find_off(name, COMMAND_LINE_MASK))
            .is_some()
}

/// The features of the processor running the compiler.
#[cfg(target_arch = "x86_64")]
fn native() -> u128 {
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
const fn native() -> u128 {
    0
}

/// A compiler running elsewhere can't ask an x86-64 processor.
#[cfg(not(target_arch = "x86_64"))]
const fn native_avx512() -> bool {
    false
}

#[cfg(test)]
mod tests;
