//! The bits of `__cpu_model` and `__cpu_features2` that
//! `__builtin_cpu_supports` and `__builtin_cpu_is` read. libgcc and compiler-rt
//! define those objects and fill them in a constructor, `__cpu_indicator_init`,
//! so the numbers are an ABI: GCC's `gcc/common/config/i386/i386-cpuinfo.h` and
//! LLVM's `X86TargetParser.def`.
//!
//! Each row holds what each compiler accepts and reads, measured from `gcc
//! -O2`'s and `clang -O2`'s assembly for every name in the compiler's strings:
//! GCC 14.2 and Clang 23, which agrees with `X86TargetParser.def`. The numbers
//! can differ: `avx10.1` is bit 113 in GCC and 114 in Clang. GCC also tests
//! `shanghai`, `istanbul`, `bdver1` and `bdver2` against the type where Clang
//! tests the subtype, and `btver1` against the subtype where Clang tests the
//! type, and each personality keeps its compiler's answer.
//! GCC manual, "x86 Built-in Functions"; Clang Language Extensions.

/// A processor feature `__builtin_cpu_supports` knows: its bit, if GCC and if
/// Clang knows it. Bit 0 to 31 are in `__cpu_model.__cpu_features[0]`, and the
/// rest in `__cpu_features2`.
const FEATURES: &[(&str, Option<u8>, Option<u8>)] = &[
    ("3dnow", Some(38), None),
    ("3dnowp", Some(39), None),
    ("64bit", None, Some(55)),
    ("abm", Some(41), None),
    ("adx", Some(40), Some(40)),
    ("aes", Some(18), Some(18)),
    ("aeskle", Some(91), None),
    ("amx-avx512", None, Some(117)),
    ("amx-bf16", Some(87), Some(87)),
    ("amx-complex", Some(106), Some(106)),
    ("amx-fp16", Some(103), Some(103)),
    ("amx-fp8", None, Some(120)),
    ("amx-int8", Some(86), Some(86)),
    ("amx-movrs", None, Some(122)),
    ("amx-tile", Some(85), Some(85)),
    ("apxf", Some(111), Some(111)),
    ("avx", Some(9), Some(9)),
    ("avx10.1", Some(113), Some(114)),
    ("avx10.1-256", Some(113), None),
    ("avx10.1-512", Some(114), None),
    ("avx10.2", None, Some(116)),
    ("avx2", Some(10), Some(10)),
    ("avx5124fmaps", Some(29), None),
    ("avx5124vnniw", Some(28), None),
    ("avx512bf16", Some(36), Some(36)),
    ("avx512bitalg", Some(35), Some(35)),
    ("avx512bmm", None, Some(123)),
    ("avx512bw", Some(21), Some(21)),
    ("avx512cd", Some(23), Some(23)),
    ("avx512dq", Some(22), Some(22)),
    ("avx512er", Some(24), None),
    ("avx512f", Some(15), Some(15)),
    ("avx512fp16", Some(94), Some(94)),
    ("avx512ifma", Some(27), Some(27)),
    ("avx512pf", Some(25), None),
    ("avx512vbmi", Some(26), Some(26)),
    ("avx512vbmi2", Some(31), Some(31)),
    ("avx512vl", Some(20), Some(20)),
    ("avx512vnni", Some(34), Some(34)),
    ("avx512vp2intersect", Some(37), Some(37)),
    ("avx512vpopcntdq", Some(30), Some(30)),
    ("avxifma", Some(99), Some(99)),
    ("avxneconvert", Some(101), Some(101)),
    ("avxvnni", Some(93), Some(93)),
    ("avxvnniint16", Some(107), Some(107)),
    ("avxvnniint8", Some(100), Some(100)),
    ("bmi", Some(16), Some(16)),
    ("bmi2", Some(17), Some(17)),
    ("cldemote", Some(42), Some(42)),
    ("clflushopt", Some(43), Some(43)),
    ("clwb", Some(44), Some(44)),
    ("clzero", Some(45), Some(45)),
    ("cmov", Some(0), Some(0)),
    ("cmpccxadd", Some(102), Some(102)),
    ("cmpxchg16b", Some(46), None),
    ("cmpxchg8b", Some(47), None),
    ("cx16", None, Some(46)),
    ("enqcmd", Some(48), Some(48)),
    ("f16c", Some(49), Some(49)),
    ("fma", Some(14), Some(14)),
    ("fma4", Some(12), Some(12)),
    ("fsgsbase", Some(50), Some(50)),
    ("fxsave", Some(51), None),
    ("gfni", Some(32), Some(32)),
    ("hreset", Some(89), Some(89)),
    ("ibt", Some(53), None),
    ("kl", Some(90), Some(90)),
    ("lahf_lm", Some(54), None),
    ("lm", Some(55), None),
    ("lwp", Some(56), Some(56)),
    ("lzcnt", Some(57), Some(57)),
    ("mmx", Some(1), Some(1)),
    ("movbe", Some(58), Some(58)),
    ("movdir64b", Some(59), Some(59)),
    ("movdiri", Some(60), Some(60)),
    ("movrs", None, Some(121)),
    ("mwaitx", Some(61), Some(61)),
    ("osxsave", Some(62), None),
    ("pclmul", Some(19), Some(19)),
    ("pconfig", Some(63), Some(63)),
    ("pku", Some(64), Some(64)),
    ("popcnt", Some(2), Some(2)),
    ("prefetchi", Some(104), Some(104)),
    ("prefetchwt1", Some(65), None),
    ("prfchw", Some(66), Some(66)),
    ("ptwrite", Some(67), Some(67)),
    ("raoint", Some(105), Some(105)),
    ("rdpid", Some(68), Some(68)),
    ("rdrnd", Some(69), Some(69)),
    ("rdseed", Some(70), Some(70)),
    ("rtm", Some(71), Some(71)),
    ("sahf", None, Some(54)),
    ("serialize", Some(72), Some(72)),
    ("sgx", Some(73), Some(73)),
    ("sha", Some(74), Some(74)),
    ("sha512", Some(109), Some(109)),
    ("shstk", Some(75), Some(75)),
    ("sm3", Some(108), Some(108)),
    ("sm4", Some(110), Some(110)),
    ("sse", Some(3), Some(3)),
    ("sse2", Some(4), Some(4)),
    ("sse3", Some(5), Some(5)),
    ("sse4.1", Some(7), Some(7)),
    ("sse4.2", Some(8), Some(8)),
    ("sse4a", Some(11), Some(11)),
    ("ssse3", Some(6), Some(6)),
    ("tbm", Some(76), Some(76)),
    ("tsxldtrk", Some(77), Some(77)),
    ("uintr", Some(88), Some(88)),
    ("usermsr", Some(112), Some(112)),
    ("vaes", Some(78), Some(78)),
    ("vpclmulqdq", Some(33), Some(33)),
    ("waitpkg", Some(79), Some(79)),
    ("wbnoinvd", Some(80), Some(80)),
    ("widekl", Some(92), Some(92)),
    ("x86-64", Some(95), Some(95)),
    ("x86-64-v2", Some(96), Some(96)),
    ("x86-64-v3", Some(97), Some(97)),
    ("x86-64-v4", Some(98), Some(98)),
    ("xop", Some(13), Some(13)),
    ("xsave", Some(81), Some(81)),
    ("xsavec", Some(82), Some(82)),
    ("xsaveopt", Some(83), Some(83)),
    ("xsaves", Some(84), Some(84)),
];

/// A processor `__builtin_cpu_is` knows: the field it compares and the value it
/// compares with, if GCC and if Clang knows it.
const PROCESSORS: &[(&str, Comparison, Comparison)] = &[
    (
        "alderlake",
        Some((CpuField::Subtype, 25)),
        Some((CpuField::Subtype, 25)),
    ),
    (
        "amd",
        Some((CpuField::Vendor, 2)),
        Some((CpuField::Vendor, 2)),
    ),
    ("amdfam10", None, Some((CpuField::Type, 4))),
    (
        "amdfam10h",
        Some((CpuField::Type, 4)),
        Some((CpuField::Type, 4)),
    ),
    ("amdfam15", None, Some((CpuField::Type, 5))),
    (
        "amdfam15h",
        Some((CpuField::Type, 5)),
        Some((CpuField::Type, 5)),
    ),
    (
        "amdfam17h",
        Some((CpuField::Type, 10)),
        Some((CpuField::Type, 10)),
    ),
    (
        "amdfam19h",
        Some((CpuField::Type, 15)),
        Some((CpuField::Type, 15)),
    ),
    ("amdfam1a", None, Some((CpuField::Type, 20))),
    ("amdfam1ah", None, Some((CpuField::Type, 20))),
    (
        "arrowlake",
        Some((CpuField::Subtype, 32)),
        Some((CpuField::Subtype, 32)),
    ),
    (
        "arrowlake-s",
        Some((CpuField::Subtype, 33)),
        Some((CpuField::Subtype, 33)),
    ),
    ("atom", Some((CpuField::Type, 1)), Some((CpuField::Type, 1))),
    (
        "barcelona",
        Some((CpuField::Subtype, 4)),
        Some((CpuField::Subtype, 4)),
    ),
    (
        "bdver1",
        Some((CpuField::Type, 7)),
        Some((CpuField::Subtype, 7)),
    ),
    (
        "bdver2",
        Some((CpuField::Type, 8)),
        Some((CpuField::Subtype, 8)),
    ),
    (
        "bdver3",
        Some((CpuField::Subtype, 9)),
        Some((CpuField::Subtype, 9)),
    ),
    (
        "bdver4",
        Some((CpuField::Subtype, 10)),
        Some((CpuField::Subtype, 10)),
    ),
    (
        "bonnell",
        Some((CpuField::Type, 1)),
        Some((CpuField::Type, 1)),
    ),
    (
        "broadwell",
        Some((CpuField::Subtype, 14)),
        Some((CpuField::Subtype, 14)),
    ),
    (
        "btver1",
        Some((CpuField::Subtype, 7)),
        Some((CpuField::Type, 8)),
    ),
    (
        "btver2",
        Some((CpuField::Type, 9)),
        Some((CpuField::Type, 9)),
    ),
    ("c86-4g-m4", None, Some((CpuField::Subtype, 41))),
    ("c86-4g-m6", None, Some((CpuField::Subtype, 42))),
    ("c86-4g-m7", None, Some((CpuField::Subtype, 43))),
    ("c86-4g-m8", None, Some((CpuField::Subtype, 44))),
    (
        "cannonlake",
        Some((CpuField::Subtype, 17)),
        Some((CpuField::Subtype, 17)),
    ),
    (
        "cascadelake",
        Some((CpuField::Subtype, 21)),
        Some((CpuField::Subtype, 21)),
    ),
    (
        "clearwaterforest",
        Some((CpuField::Type, 19)),
        Some((CpuField::Type, 19)),
    ),
    (
        "cooperlake",
        Some((CpuField::Subtype, 23)),
        Some((CpuField::Subtype, 23)),
    ),
    (
        "core2",
        Some((CpuField::Type, 2)),
        Some((CpuField::Type, 2)),
    ),
    (
        "corei7",
        Some((CpuField::Type, 3)),
        Some((CpuField::Type, 3)),
    ),
    ("diamondrapids", None, Some((CpuField::Subtype, 38))),
    (
        "emeraldrapids",
        Some((CpuField::Subtype, 24)),
        Some((CpuField::Subtype, 24)),
    ),
    (
        "goldmont",
        Some((CpuField::Type, 12)),
        Some((CpuField::Type, 12)),
    ),
    (
        "goldmont-plus",
        Some((CpuField::Type, 13)),
        Some((CpuField::Type, 13)),
    ),
    (
        "gracemont",
        Some((CpuField::Subtype, 25)),
        Some((CpuField::Subtype, 25)),
    ),
    (
        "grandridge",
        Some((CpuField::Type, 18)),
        Some((CpuField::Type, 18)),
    ),
    (
        "graniterapids",
        Some((CpuField::Subtype, 30)),
        Some((CpuField::Subtype, 30)),
    ),
    (
        "graniterapids-d",
        Some((CpuField::Subtype, 31)),
        Some((CpuField::Subtype, 31)),
    ),
    (
        "haswell",
        Some((CpuField::Subtype, 13)),
        Some((CpuField::Subtype, 13)),
    ),
    ("hygonfam18h", None, Some((CpuField::Type, 21))),
    (
        "icelake-client",
        Some((CpuField::Subtype, 18)),
        Some((CpuField::Subtype, 18)),
    ),
    (
        "icelake-server",
        Some((CpuField::Subtype, 19)),
        Some((CpuField::Subtype, 19)),
    ),
    (
        "intel",
        Some((CpuField::Vendor, 1)),
        Some((CpuField::Vendor, 1)),
    ),
    (
        "istanbul",
        Some((CpuField::Type, 6)),
        Some((CpuField::Subtype, 6)),
    ),
    (
        "ivybridge",
        Some((CpuField::Subtype, 12)),
        Some((CpuField::Subtype, 12)),
    ),
    ("knl", Some((CpuField::Type, 7)), Some((CpuField::Type, 7))),
    (
        "knm",
        Some((CpuField::Type, 11)),
        Some((CpuField::Type, 11)),
    ),
    (
        "lunarlake",
        Some((CpuField::Subtype, 33)),
        Some((CpuField::Subtype, 33)),
    ),
    (
        "meteorlake",
        Some((CpuField::Subtype, 25)),
        Some((CpuField::Subtype, 25)),
    ),
    (
        "nehalem",
        Some((CpuField::Subtype, 1)),
        Some((CpuField::Subtype, 1)),
    ),
    ("novalake", None, Some((CpuField::Subtype, 39))),
    ("other", None, Some((CpuField::Vendor, 5))),
    (
        "pantherlake",
        Some((CpuField::Subtype, 34)),
        Some((CpuField::Subtype, 34)),
    ),
    (
        "raptorlake",
        Some((CpuField::Subtype, 25)),
        Some((CpuField::Subtype, 25)),
    ),
    (
        "rocketlake",
        Some((CpuField::Subtype, 27)),
        Some((CpuField::Subtype, 27)),
    ),
    (
        "sandybridge",
        Some((CpuField::Subtype, 3)),
        Some((CpuField::Subtype, 3)),
    ),
    (
        "sapphirerapids",
        Some((CpuField::Subtype, 24)),
        Some((CpuField::Subtype, 24)),
    ),
    (
        "shanghai",
        Some((CpuField::Type, 5)),
        Some((CpuField::Subtype, 5)),
    ),
    (
        "sierraforest",
        Some((CpuField::Type, 17)),
        Some((CpuField::Type, 17)),
    ),
    (
        "silvermont",
        Some((CpuField::Type, 6)),
        Some((CpuField::Type, 6)),
    ),
    (
        "skylake",
        Some((CpuField::Subtype, 15)),
        Some((CpuField::Subtype, 15)),
    ),
    (
        "skylake-avx512",
        Some((CpuField::Subtype, 16)),
        Some((CpuField::Subtype, 16)),
    ),
    ("slm", Some((CpuField::Type, 6)), Some((CpuField::Type, 6))),
    (
        "tigerlake",
        Some((CpuField::Subtype, 22)),
        Some((CpuField::Subtype, 22)),
    ),
    (
        "tremont",
        Some((CpuField::Type, 14)),
        Some((CpuField::Type, 14)),
    ),
    (
        "westmere",
        Some((CpuField::Subtype, 2)),
        Some((CpuField::Subtype, 2)),
    ),
    ("wildcatlake", None, Some((CpuField::Subtype, 34))),
    ("zhaoxin_fam7h", None, Some((CpuField::Type, 16))),
    (
        "zhaoxin_fam7h_lujiazui",
        None,
        Some((CpuField::Subtype, 28)),
    ),
    (
        "znver1",
        Some((CpuField::Subtype, 11)),
        Some((CpuField::Subtype, 11)),
    ),
    (
        "znver2",
        Some((CpuField::Subtype, 20)),
        Some((CpuField::Subtype, 20)),
    ),
    (
        "znver3",
        Some((CpuField::Subtype, 26)),
        Some((CpuField::Subtype, 26)),
    ),
    (
        "znver4",
        Some((CpuField::Subtype, 29)),
        Some((CpuField::Subtype, 29)),
    ),
    (
        "znver5",
        Some((CpuField::Subtype, 36)),
        Some((CpuField::Subtype, 36)),
    ),
    ("znver6", None, Some((CpuField::Subtype, 40))),
];

/// The field `__builtin_cpu_is` compares and its value, if the compiler knows
/// the name.
type Comparison = Option<(CpuField, u8)>;

/// A field of `__cpu_model`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CpuField {
    Vendor,
    Type,
    Subtype,
}

impl CpuField {
    /// Its offset in `__cpu_model`.
    pub(crate) const fn offset(self) -> u64 {
        match self {
            | Self::Vendor => 0,
            | Self::Type => 4,
            | Self::Subtype => 8,
        }
    }
}

/// The name a processor query's string gives: as far as the first NUL in GCC,
/// which reads it as a C string, and all of it in Clang.
/// GCC `fold_builtin_cpu`; Clang's `checkBuiltinCpu`.
pub(crate) fn cpu_query_name(text: &str, clang: bool) -> &str {
    if clang {
        text
    } else {
        text.split('\0').next().unwrap_or(text)
    }
}

/// The bit of the feature `name` for `__builtin_cpu_supports`, in Clang's table
/// or GCC's, if the compiler knows it.
pub(crate) fn cpu_feature_bit(name: &str, clang: bool) -> Option<u8> {
    let &(_, gcc_bit, clang_bit) = find(FEATURES, name)?;
    if clang { clang_bit } else { gcc_bit }
}

/// What `__builtin_cpu_is` compares for `name`, in Clang's table or GCC's, if
/// the compiler knows it.
pub(crate) fn cpu_processor(name: &str, clang: bool) -> Comparison {
    let &(_, gcc_value, clang_value) = find(PROCESSORS, name)?;
    if clang { clang_value } else { gcc_value }
}

fn find<'t, T>(table: &'t [(&str, T, T)], name: &str) -> Option<&'t (&'t str, T, T)> {
    table
        .binary_search_by_key(&name, |row| row.0)
        .ok()
        .map(|index| &table[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup needs the rows sorted, and the bits have to fit
    /// `__cpu_features2`.
    #[test]
    fn tables_are_sorted_and_in_range() {
        assert!(FEATURES.is_sorted_by_key(|row| row.0));
        assert!(PROCESSORS.is_sorted_by_key(|row| row.0));
        for &(name, gcc, clang) in FEATURES {
            assert!(gcc.is_some() || clang.is_some(), "{name}");
            assert!(gcc.max(clang).is_some_and(|bit| bit < 128), "{name}");
        }
    }

    /// The common features are the same bits in both, in GCC's `i386-cpuinfo.h`
    /// order.
    #[test]
    fn the_abi_order_holds() {
        for (index, name) in [
            "cmov", "mmx", "popcnt", "sse", "sse2", "sse3", "ssse3", "sse4.1", "sse4.2", "avx",
            "avx2", "sse4a", "fma4", "xop", "fma", "avx512f", "bmi", "bmi2", "aes", "pclmul",
        ]
        .into_iter()
        .enumerate()
        {
            let bit = u8::try_from(index).unwrap();
            assert_eq!(cpu_feature_bit(name, false), Some(bit), "{name}");
            assert_eq!(cpu_feature_bit(name, true), Some(bit), "{name}");
        }
        assert_eq!(cpu_feature_bit("avx10.1", false), Some(113));
        assert_eq!(cpu_feature_bit("avx10.1", true), Some(114));
        assert_eq!(cpu_feature_bit("lm", false), Some(55));
        assert_eq!(cpu_feature_bit("lm", true), None);
        assert_eq!(cpu_feature_bit("64bit", true), Some(55));
        assert_eq!(cpu_processor("amd", false), Some((CpuField::Vendor, 2)));
        assert_eq!(cpu_processor("znver5", true), Some((CpuField::Subtype, 36)));
        assert_eq!(cpu_processor("shanghai", false), Some((CpuField::Type, 5)));
        assert_eq!(
            cpu_processor("shanghai", true),
            Some((CpuField::Subtype, 5))
        );
    }
}
