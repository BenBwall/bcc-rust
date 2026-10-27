//! Constructs configurations, applies independent options, and recomputes
//! accepted and native feature sets after language choices change.

use super::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
    Feature,
    FeatureOrigin,
    MsvcFeature,
    features::MSVC_COMPATIBILITY,
};

impl CompilerConfiguration {
    const fn derive_features(mut self) -> Self {
        self.accepted = 0;
        self.native = 0;
        let mut index = 0;
        while index < Feature::ALL.len() {
            let feature = Feature::ALL[index];
            let native = match feature {
                | Feature::Trigraphs => (self.standard as u8) <= CStandard::C17 as u8,
                | _ => self.origin_is_native(feature.origin()),
            };
            if native {
                self.native |= feature.bit();
            }
            let accepted = match feature {
                | Feature::OldStyleFunctionDeclarators
                | Feature::VoidExpressionReturn
                | Feature::Digraphs
                | Feature::UnicodeLiteralPrefixes
                | Feature::Utf8CharacterConstants
                | Feature::DigitSeparators
                | Feature::BitIntSuffixes
                | Feature::OctalPrefix
                | Feature::DelimitedEscapes
                | Feature::Restrict
                | Feature::C23Keywords
                | Feature::MsDeclspec
                | Feature::MsIntTypes
                | Feature::MsCallingConventions
                | Feature::MsTypeQualifiers
                | Feature::MsInline
                | Feature::MsSeh
                | Feature::MsAsm
                | Feature::MsPragma
                | Feature::MsAnonymousStructs
                | Feature::MsVaArgs => native,
                | Feature::CaseRanges
                | Feature::LineComments
                | Feature::BinaryConstants
                | Feature::Elifdef
                | Feature::WarningDirective
                | Feature::Embed
                | Feature::HasInclude
                | Feature::HasEmbed
                | Feature::HasCAttribute
                | Feature::VaOpt
                | Feature::Inline => native || self.gnu,
                | Feature::Trigraphs => match self.trigraphs {
                    | Some(enabled) => enabled,
                    | None => !self.gnu && (self.standard as u8) <= CStandard::C17 as u8,
                },
                | Feature::InterchangeFloatTypes => self.interchange_float_types(),
                | Feature::Float16 => self.half_precision_types(),
                | _ => true,
            };
            if accepted {
                self.accepted |= feature.bit();
            }
            index += 1;
        }
        // GNU89 includes the C95 digraph extension.
        if self.gnu {
            self.accepted |= Feature::Digraphs.bit();
        }
        self
    }

    pub(crate) const fn origin_is_native(self, origin: FeatureOrigin) -> bool {
        match origin {
            | FeatureOrigin::Standard(standard) => self.standard as u8 >= standard as u8,
            | FeatureOrigin::Removed { since, removed } =>
                self.standard as u8 >= since as u8 && (self.standard as u8) < removed as u8,
            | FeatureOrigin::Gnu => self.gnu,
            | FeatureOrigin::Msvc(feature) => self.msvc_feature(feature),
        }
    }

    pub(crate) const fn with_msvc_feature(mut self, feature: MsvcFeature, enabled: bool) -> Self {
        let bit = 1 << feature as u8;
        if enabled {
            self.msvc |= bit;
        } else {
            self.msvc &= !bit;
        }
        self.derive_features()
    }

    /// Enables or disables every MSVC group, as `-fms-extensions` and
    /// `-fno-ms-extensions` do; the umbrella also decides
    /// [`Self::msvc_compatibility`].
    pub(crate) const fn with_msvc_extensions(mut self, enabled: bool) -> Self {
        self.msvc = if enabled {
            ((1 << MsvcFeature::ALL.len()) - 1) | MSVC_COMPATIBILITY
        } else {
            0
        };
        self.derive_features()
    }

    pub(crate) const fn with_target(mut self, target: crate::target::Target) -> Self {
        self.target = target;
        self
    }

    /// Selects the relocation and thread-local storage models, which
    /// `__PIC__` and `__PIE__` and the back ends follow.
    pub(crate) const fn with_code_options(mut self, options: crate::target::CodeOptions) -> Self {
        self.code_options = options;
        self
    }

    pub(crate) const fn code_options(self) -> crate::target::CodeOptions {
        self.code_options
    }

    pub(crate) const fn with_gnu_extensions(mut self, enabled: bool) -> Self {
        self.gnu = enabled;
        self.derive_features()
    }

    pub(crate) const fn with_extension_policy(mut self, policy: ExtensionPolicy) -> Self {
        self.extension_policy = policy;
        self
    }

    pub(crate) const fn with_optimization(mut self, optimization: (bool, bool)) -> Self {
        self.optimization = optimization;
        self
    }

    pub(crate) const fn optimization(self) -> (bool, bool) {
        self.optimization
    }

    pub(crate) const fn with_hosted(mut self, hosted: bool) -> Self {
        self.hosted = hosted;
        self
    }

    /// Overrides the dialect default, like GCC's `-f[no-]gnu89-inline`.
    /// GNU extension: <https://gcc.gnu.org/onlinedocs/gcc/Inline.html>.
    pub(crate) const fn with_gnu89_inline(mut self, enabled: bool) -> Self {
        self.gnu89_inline = Some(enabled);
        self
    }

    /// GNU89 reverses external inline emission relative to C99.
    /// C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125.
    pub(crate) const fn gnu89_inline(self) -> bool {
        match self.gnu89_inline {
            | Some(enabled) => enabled,
            | None => (self.standard as u8) < CStandard::C99 as u8,
        }
    }

    /// Makes plain `char` signed or unsigned, overriding the target, like
    /// GCC's and Clang's `-fsigned-char` and `-funsigned-char`. Plain `char`
    /// stays a type distinct from `signed char` and `unsigned char`.
    /// C99: implementation-defined, §6.2.5 paragraph 15, p. 35; PDF p. 47.
    /// GCC manual, "C Dialect Options",
    /// <https://gcc.gnu.org/onlinedocs/gcc/C-Dialect-Options.html#index-funsigned-char>.
    pub(crate) const fn with_char_signed(mut self, signed: bool) -> Self {
        self.char_signed = Some(signed);
        self
    }

    /// The target's scalar layout, with plain `char`'s signedness as
    /// [`Self::with_char_signed`] chose it. Semantic analysis, constant
    /// folding and lowering read their types from this layout.
    pub(crate) const fn layout(self) -> crate::target::TargetLayout {
        let mut layout = self.target.layout();
        if let Some(signed) = self.char_signed {
            layout.char_signed = signed;
        }
        layout
    }

    /// Makes `_Float32`, `_Float64`, `_Float128`, `_Float32x` and `_Float64x`
    /// keywords naming distinct types, as GCC 7 and later do, and predefines
    /// their `__FLT32_*__`-family characteristics macros, overriding the
    /// identity's default.
    /// C23: §H.5.1 paragraph 1, p. 560; PDF p. 573.
    pub(crate) const fn with_interchange_float_types(mut self, enabled: bool) -> Self {
        self.interchange_float_types = Some(enabled);
        self.derive_features()
    }

    /// Whether the `_FloatN` keywords name distinct types: as
    /// [`Self::with_interchange_float_types`] chose, or else for a GCC
    /// identity alone, since GCC 7 and later provide them in every mode.
    /// glibc declares these names as typedefs for the Clang, MSVC and TCC
    /// identities, none of whose compilers provides them in C, so they stay
    /// identifiers there.
    pub(crate) const fn interchange_float_types(self) -> bool {
        match self.interchange_float_types {
            | Some(enabled) => enabled,
            | None => matches!(self.identity, super::CompilerIdentity::Gcc),
        }
    }

    /// Whether `_Float16` is a type, as GCC 12 and later and Clang provide
    /// it on x86-64 with SSE2. MSVC and TCC have no half-precision type.
    pub(crate) const fn half_precision_types(self) -> bool {
        matches!(
            self.identity,
            super::CompilerIdentity::Gcc
                | super::CompilerIdentity::Clang
                | super::CompilerIdentity::ClangCl
        )
    }

    /// Whether every `_Float16` value is evaluated to the range and
    /// precision of `float` and rounded only by a conversion to `_Float16`,
    /// as GCC evaluates it on x86-64; otherwise only chains of arithmetic
    /// operators keep the excess, as Clang evaluates it. Semantic analysis
    /// documents both rules (`Analyzer::half_in_float`).
    /// C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (evaluation formats).
    pub(crate) const fn half_in_float(self) -> bool {
        matches!(self.identity, super::CompilerIdentity::Gcc)
    }

    /// Selects the compiler the identity macros claim, and with it whether
    /// the `_FloatN` keywords are accepted.
    pub(crate) const fn with_compiler_identity(
        mut self,
        identity: super::CompilerIdentity,
    ) -> Self {
        self.identity = identity;
        self.derive_features()
    }

    pub(crate) const fn compiler_identity(self) -> super::CompilerIdentity {
        self.identity
    }

    /// Overrides whether phase 1 replaces trigraphs, which the mode
    /// otherwise decides (strict C89 through C17), as Clang's
    /// `-f[no-]trigraphs` and GCC's `-trigraphs` do.
    /// C99: §5.2.1.1, p. 18; PDF p. 30.
    pub(crate) const fn with_trigraphs(mut self, enabled: bool) -> Self {
        self.trigraphs = Some(enabled);
        self.derive_features()
    }

    /// Makes signed integer overflow wrap in two's complement instead of
    /// being undefined, as GCC's `-fwrapv` does. Extension:
    /// C99 §6.5 paragraph 5, p. 67; PDF p. 79 leaves the overflow undefined.
    pub(crate) const fn with_wrapv(mut self, enabled: bool) -> Self {
        self.wrapv = enabled;
        self
    }

    /// Whether signed integer overflow wraps (`-fwrapv`).
    pub(crate) const fn wrapv(self) -> bool {
        self.wrapv
    }

    /// Sets the structure packing in effect before any `#pragma pack`, as
    /// GCC's `-fpack-struct=N` and `cl /ZpN` do; 0 packs nothing.
    /// Implementation-defined: C99 §6.7.2.1 paragraph 12, p. 103; PDF p. 115.
    pub(crate) const fn with_default_pack(mut self, alignment: u32) -> Self {
        self.default_pack = alignment;
        self
    }

    /// Sets the visibility of external definitions that name none, as
    /// GCC's `-fvisibility=` does.
    pub(crate) const fn with_default_visibility(
        mut self,
        visibility: super::DefaultVisibility,
    ) -> Self {
        self.default_visibility = visibility;
        self
    }

    /// The visibility of external definitions that name none.
    pub(crate) const fn default_visibility(self) -> super::DefaultVisibility {
        self.default_visibility
    }

    /// Gives up (or keeps) part of IEC 60559 semantics, as GCC's
    /// `-fno-signed-zeros` and its relatives do. Only predefined macros
    /// read it.
    pub(crate) const fn with_relaxed_floating_point(
        mut self,
        option: super::RelaxedFloatingPoint,
        relaxed: bool,
    ) -> Self {
        if relaxed {
            self.relaxed_floating_point |= option.bit();
        } else {
            self.relaxed_floating_point &= !option.bit();
        }
        self.given_floating_point |= option.bit();
        self
    }

    pub(crate) const fn relaxes_floating_point(self, option: super::RelaxedFloatingPoint) -> bool {
        self.relaxed_floating_point & option.bit() != 0
    }

    /// Whether `option` was given in either form, rather than left at its
    /// default.
    pub(crate) const fn gives_floating_point(self, option: super::RelaxedFloatingPoint) -> bool {
        self.given_floating_point & option.bit() != 0
    }

    /// Packs every structure and union as the `packed` attribute does and
    /// ignores `#pragma pack`, as GCC's `-fpack-struct` without a value
    /// does. Implementation-defined: C99 §6.7.2.1 paragraph 12, p. 103;
    /// PDF p. 115.
    pub(crate) const fn with_pack_structures(mut self, enabled: bool) -> Self {
        self.pack_structures = enabled;
        self
    }

    /// Whether every structure is packed (`-fpack-struct`).
    pub(crate) const fn pack_structures(self) -> bool {
        self.pack_structures
    }

    /// The structure packing in effect before any `#pragma pack`.
    pub(crate) const fn default_pack(self) -> u32 {
        self.default_pack
    }

    pub(crate) const fn with_suppress_warnings(mut self, enabled: bool) -> Self {
        self.suppress_warnings = enabled;
        self
    }

    pub(crate) const fn suppress_warnings(self) -> bool {
        self.suppress_warnings
    }

    /// The most recursive grammar productions the parser keeps active at
    /// once before it stops with a resource diagnostic.
    pub(crate) const fn max_nesting_depth(self) -> u32 {
        self.max_nesting_depth
    }

    pub(crate) const fn with_max_nesting_depth(mut self, depth: u32) -> Self {
        self.max_nesting_depth = depth;
        self
    }

    pub(crate) const fn standard(self) -> CStandard {
        self.standard
    }

    pub(crate) const fn target(self) -> crate::target::Target {
        self.target
    }

    pub(crate) const fn gnu_extensions(self) -> bool {
        self.gnu
    }

    pub(crate) const fn strict_ansi(self) -> bool {
        !self.gnu
    }

    pub(crate) const fn extension_policy(self) -> ExtensionPolicy {
        self.extension_policy
    }

    /// Whether the last MSVC umbrella flag was `-fms-extensions`, which on
    /// the MSVC target predefines `_MSC_VER` as Clang does; the individual
    /// groups do not.
    pub(crate) const fn msvc_compatibility(self) -> bool {
        self.msvc & MSVC_COMPATIBILITY != 0
    }

    pub(crate) const fn msvc_feature(self, feature: MsvcFeature) -> bool {
        self.msvc & (1 << feature as u8) != 0
    }

    pub(crate) const fn repeated_specifier_warnings(self) -> bool {
        self.repeated_specifier_warnings
    }

    pub(crate) const fn with_repeated_specifier_warnings(mut self, enabled: bool) -> Self {
        self.repeated_specifier_warnings = enabled;
        self
    }

    pub(crate) const fn source_date_epoch(self) -> Option<i64> {
        self.source_date_epoch
    }

    pub(crate) const fn with_source_date_epoch(mut self, seconds: Option<i64>) -> Self {
        self.source_date_epoch = seconds;
        self
    }

    /// Whether translation targets a hosted execution environment, the
    /// default, rather than a freestanding one. It sets `__STDC_HOSTED__`,
    /// lets the resource headers defer to the C library, and applies the
    /// hosted requirements on `main`.
    ///
    /// C99: §4 paragraph 6, p. 7; PDF p. 19; §5.1.2 paragraph 1 and §5.1.2.1
    /// paragraph 1, p. 11; PDF p. 23; §6.10.8 paragraph 1, p. 160; PDF p. 172.
    pub(crate) const fn hosted(self) -> bool {
        self.hosted
    }

    pub(crate) const fn new(standard: CStandard, extension_policy: ExtensionPolicy) -> Self {
        Self {
            target: crate::target::Target::LinuxGnu,
            code_options: crate::target::CodeOptions {
                relocation: None,
                tls_model:  crate::target::TlsModel::GlobalDynamic,
            },
            standard,
            gnu: false,
            msvc: 0,
            extension_policy,
            accepted: 0,
            native: 0,
            repeated_specifier_warnings: true,
            source_date_epoch: None,
            hosted: true,
            optimization: (false, false),
            gnu89_inline: None,
            char_signed: None,
            interchange_float_types: None,
            trigraphs: None,
            wrapv: false,
            default_pack: 0,
            pack_structures: false,
            default_visibility: super::DefaultVisibility::Default,
            relaxed_floating_point: 0,
            given_floating_point: 0,
            suppress_warnings: false,
            max_nesting_depth: super::DEFAULT_MAX_NESTING_DEPTH,
            warnings_as_errors: false,
            identity: super::CompilerIdentity::Clang,
            diagnostic_format: super::DiagnosticFormat::Bcc,
            warning_options: [super::diagnostics::WarningSettings::DEFAULT;
                super::DiagnosticGroup::COUNT],
        }
        .derive_features()
    }
}

impl Default for CompilerConfiguration {
    fn default() -> Self {
        Self::new(CStandard::C99, ExtensionPolicy::Allow)
    }
}
