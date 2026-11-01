//! Language revisions, dialect selection, extension policy, Microsoft
//! feature switches, and borrowed preprocessing options.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct LanguageMode {
    pub(crate) standard: CStandard,
    pub(crate) gnu:      bool,
}

impl LanguageMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (standard, gnu) = match value {
            | "c89" | "c90" | "iso9899:1990" => (CStandard::C89, false),
            | "iso9899:199409" => (CStandard::C95, false),
            | "gnu89" | "gnu90" => (CStandard::C89, true),
            | "c99" | "c9x" | "iso9899:1999" | "iso9899:199x" => (CStandard::C99, false),
            | "gnu99" | "gnu9x" => (CStandard::C99, true),
            | "c11" | "c1x" | "iso9899:2011" => (CStandard::C11, false),
            | "gnu11" | "gnu1x" => (CStandard::C11, true),
            | "c17" | "c18" | "iso9899:2017" | "iso9899:2018" => (CStandard::C17, false),
            | "gnu17" | "gnu18" => (CStandard::C17, true),
            | "c23" | "c2x" | "iso9899:2024" => (CStandard::C23, false),
            | "gnu23" | "gnu2x" => (CStandard::C23, true),
            | "c2y" => (CStandard::C2y, false),
            | "gnu2y" => (CStandard::C2y, true),
            | _ => return None,
        };
        Some(Self { standard, gnu })
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum CStandard {
    C89,
    C95,
    C99,
    C11,
    C17,
    C23,
    C2y,
}

impl CStandard {
    /// C99: §6.10.8p1, p. 160; PDF p. 172. Later editions and the C95
    /// amendment prescribe their own versions. C2y follows Clang (202400L).
    pub(crate) const fn version_macro(self) -> Option<&'static str> {
        match self {
            | Self::C89 => None,
            | Self::C95 => Some("199409L\0"),
            | Self::C99 => Some("199901L\0"),
            | Self::C11 => Some("201112L\0"),
            | Self::C17 => Some("201710L\0"),
            | Self::C23 => Some("202311L\0"),
            | Self::C2y => Some("202400L\0"),
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::C89 => "C89",
            | Self::C95 => "C95",
            | Self::C99 => "C99",
            | Self::C11 => "C11",
            | Self::C17 => "C17",
            | Self::C23 => "C23",
            | Self::C2y => "C2y",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub(crate) enum MsvcFeature {
    Declspec,
    IntTypes,
    CallingConventions,
    TypeQualifiers,
    Inline,
    Seh,
    Asm,
    Pragma,
    AnonymousStructs,
    VaArgs,
}

impl MsvcFeature {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            | "declspec" => Some(Self::Declspec),
            | "int-types" => Some(Self::IntTypes),
            | "calling-conventions" => Some(Self::CallingConventions),
            | "type-qualifiers" => Some(Self::TypeQualifiers),
            | "inline" => Some(Self::Inline),
            | "seh" => Some(Self::Seh),
            | "asm" => Some(Self::Asm),
            | "pragma" => Some(Self::Pragma),
            | "anonymous-structs" => Some(Self::AnonymousStructs),
            | "va-args" => Some(Self::VaArgs),
            | _ => None,
        }
    }
}

impl MsvcFeature {
    pub(crate) const ALL: [Self; 10] = [
        Self::Declspec,
        Self::IntTypes,
        Self::CallingConventions,
        Self::TypeQualifiers,
        Self::Inline,
        Self::Seh,
        Self::Asm,
        Self::Pragma,
        Self::AnonymousStructs,
        Self::VaArgs,
    ];
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExtensionPolicy {
    Allow,
    Warn,
    Deny,
}

/// Startup preprocessing operations, borrowed from the caller or an arena.
/// This is separate from the scalar language configuration so that the latter
/// remains a lifetime-free, constant value. GCC command-line extension to C99
/// §6.10.3p9, p. 152; PDF p. 164 and §6.10.3.5, p. 155; PDF p. 167.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreprocessingOption<'a> {
    Define(&'a str),
    Undefine(&'a str),
    Include(&'a str),
}

/// The ELF visibility a definition with external linkage gets when it
/// names none, as GCC's `-fvisibility=` sets it. GNU extension: GCC manual,
/// "Code Gen Options", `-fvisibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DefaultVisibility {
    #[default]
    Default,
    /// `hidden`, and `internal`, which bcc emits as hidden.
    Hidden,
    Protected,
}

/// An option that gives up part of IEC 60559 semantics, as GCC's and
/// Clang's `-ffast-math` family does. The LLVM back end gives each
/// floating operation the matching fast-math flags
/// ([`FastMathFlags`](crate::configuration::FastMathFlags)), so LLVM's
/// optimizers transform the program as Clang's would; bcc's own optimizer,
/// constant folding and its native, interpreter and WebAssembly back ends
/// keep IEC 60559 semantics, which every option permits. The GCC identity
/// also reports them in predefined macros (`__GCC_IEC_559`,
/// `__NO_SIGNED_ZEROS__` and the rest), and both identities define
/// `__FAST_MATH__` and `__FINITE_MATH_ONLY__` as their compiler does.
/// C99: Annex F, §F.1 paragraph 1, p. 444; PDF p. 456 (an implementation
/// that defines `__STDC_IEC_559__` conforms to Annex F, which these options
/// give up); Annex G, §G.1 paragraph 1, p. 467; PDF p. 479. GNU extension:
/// GCC manual, "Optimize Options"; `cpp_iec_559_value` in GCC's
/// `c-family/c-cppbuiltin.cc`; Clang's "Controlling Floating-point
/// Behavior".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelaxedFloatingPoint {
    /// `-fno-signed-zeros`.
    NoSignedZeros,
    /// `-freciprocal-math`.
    ReciprocalMath,
    /// `-fno-trapping-math`.
    NoTrappingMath,
    /// `-fno-math-errno`: the math functions need not set `errno`, so
    /// they read and write no memory.
    NoMathErrno,
    /// `-fassociative-math`, which GCC drops unless signed zeros and traps
    /// are given up too.
    AssociativeMath,
    /// `-fcx-limited-range`: complex multiplication and division without
    /// Annex G's treatment of infinities, unless `-fcx-fortran-rules` or
    /// its `-fno-` form is given, which then decides alone, as in GCC 15.
    CxLimitedRange,
    /// `-fcx-fortran-rules`: Fortran's complex division, also without
    /// Annex G's treatment of infinities.
    CxFortranRules,
    /// Clang's `-fno-honor-nans`, half of `-ffinite-math-only`: no operand
    /// or result is a NaN.
    NoNans,
    /// Clang's `-fno-honor-infinities`, the other half: no operand or
    /// result is an infinity.
    NoInfinities,
    /// Clang's `-fapprox-func`, which `-funsafe-math-optimizations` implies
    /// in both compilers: a math function may be approximated.
    ApproxFunc,
    /// GCC's `-funsafe-math-optimizations` itself, which GCC's
    /// `__FAST_MATH__` asks for besides the options it implies.
    UnsafeMath,
}

impl RelaxedFloatingPoint {
    /// What `-ffast-math` gives up and `-fno-fast-math` restores: all but
    /// the complex arithmetic, which they change only when no
    /// `-f[no-]cx-limited-range` was given, as GCC's `set_fast_math_flags`
    /// does.
    pub(crate) const FAST_MATH: [Self; 9] = [
        Self::NoSignedZeros,
        Self::ReciprocalMath,
        Self::NoTrappingMath,
        Self::NoMathErrno,
        Self::AssociativeMath,
        Self::NoNans,
        Self::NoInfinities,
        Self::ApproxFunc,
        Self::UnsafeMath,
    ];
    /// What `-ffinite-math-only` gives up and its `-fno-` form restores.
    pub(crate) const FINITE_MATH: [Self; 2] = [Self::NoNans, Self::NoInfinities];
    /// What `-funsafe-math-optimizations` gives up and its `-fno-` form
    /// restores.
    pub(crate) const UNSAFE_MATH: [Self; 6] = [
        Self::NoSignedZeros,
        Self::ReciprocalMath,
        Self::NoTrappingMath,
        Self::AssociativeMath,
        Self::ApproxFunc,
        Self::UnsafeMath,
    ];

    pub(crate) const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The LLVM fast-math flags the relaxed options in effect give each
/// floating operation, as Clang sets them: `nnan`, `ninf`, `nsz`, `arcp`,
/// `reassoc` (with `contract`, which Clang's `-ffast-math` and
/// `-funsafe-math-optimizations` imply through `-ffp-contract=fast`) and
/// `afn`. Empty unless an option relaxes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "One independent flag per LLVM fast-math flag or attribute, as LLVM names them."
)]
pub(crate) struct FastMathFlags {
    pub(crate) no_nans:          bool,
    pub(crate) no_infinities:    bool,
    pub(crate) no_signed_zeros:  bool,
    pub(crate) reciprocal:       bool,
    pub(crate) reassociate:      bool,
    pub(crate) contract:         bool,
    pub(crate) approximate_func: bool,
    /// `-fno-trapping-math`, which LLVM reads as a function attribute.
    pub(crate) no_trapping:      bool,
    /// `-fno-math-errno`: calls of the math functions read and write no
    /// memory, which lets LLVM treat them as operations.
    pub(crate) no_math_errno:    bool,
}

impl FastMathFlags {
    /// IEC 60559 semantics in full.
    pub(crate) const NONE: Self = Self {
        no_nans:          false,
        no_infinities:    false,
        no_signed_zeros:  false,
        reciprocal:       false,
        reassociate:      false,
        contract:         false,
        approximate_func: false,
        no_trapping:      false,
        no_math_errno:    false,
    };
}

/// Which standard library functions the compiler may treat as builtins:
/// recognize a call by its name and replace or transform it (`sqrt` as an
/// instruction, a copying loop as `memcpy`). GCC's `-fno-builtin` and
/// `-ffreestanding` allow none, and `-fno-builtin-NAME` excludes NAME.
/// Separate from the scalar configuration because it borrows names.
/// C99: §7.1.4 paragraph 1, pp. 166-167; PDF pp. 178-179 (a library
/// function may also be implemented as a builtin); §5.1.2.1, p. 11; PDF
/// p. 23 (a freestanding program defines its own). GNU extension: GCC
/// manual, "Other Builtins" and "C Dialect Options".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LibraryBuiltins<'a> {
    /// Every library function.
    #[default]
    All,
    /// None (`-fno-builtin`, `-ffreestanding`).
    None,
    /// Every library function but these (`-fno-builtin-NAME`).
    Except(&'a [&'a str]),
}

/// The compiler the predefined identity macros claim, which
/// `--compiler-identity` and the driver personalities select. Clang is the
/// default: bcc claims the pinned Clang, and the GCC 4.2.1 that Clang claims.
/// The others claim GCC 14.2, MSVC 19.33 (`cl.exe`), Clang 23 in its MSVC
/// mode (`clang-cl`), or TCC 0.9.27, and withdraw Clang's own macros where
/// that compiler does not define them.
/// C99: further predefined macro names, §6.10.8 paragraph 4, p. 161;
/// PDF p. 173; reserved identifiers, §7.1.3 paragraph 1, p. 166; PDF p. 178.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub(crate) enum CompilerIdentity {
    #[default]
    Clang,
    Gcc,
    Msvc,
    ClangCl,
    Tcc,
}
