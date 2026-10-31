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
                | Feature::DollarIdentifiers => self.dollar_identifiers,
                | Feature::InterchangeFloatTypes => self.interchange_float_types(),
                | Feature::Float16 | Feature::BFloat16 => self.half_precision_types(),
                | Feature::Fp16 => self.storage_half_type(),
                | Feature::GccOnlyBuiltins => matches!(self.identity, super::CompilerIdentity::Gcc),
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

    /// Whether `_Float16` and `__bf16` are types, as GCC 12 (`__bf16`: 13)
    /// and later and Clang provide them on x86-64 with SSE2. MSVC and TCC
    /// have no half-precision type.
    pub(crate) const fn half_precision_types(self) -> bool {
        matches!(
            self.identity,
            super::CompilerIdentity::Gcc
                | super::CompilerIdentity::Clang
                | super::CompilerIdentity::ClangCl
        )
    }

    /// Whether `__fp16` is a type, as Clang provides it on every target:
    /// binary16 storage whose values promote to `float`. GCC provides it
    /// only on ARM.
    pub(crate) const fn storage_half_type(self) -> bool {
        matches!(
            self.identity,
            super::CompilerIdentity::Clang | super::CompilerIdentity::ClangCl
        )
    }

    /// Whether every `_Float16` value is evaluated to the range and
    /// precision of `float` and rounded only by a conversion to `_Float16`,
    /// as GCC evaluates it on x86-64, under GCC's rules (only GCC's identity
    /// among those has `_Float16`); otherwise only chains of arithmetic
    /// operators keep the excess, as Clang evaluates it. Semantic analysis
    /// documents both rules (`Analyzer::half_in_float`).
    /// C99: §5.2.4.2.2 paragraph 7, p. 24; PDF p. 36 (evaluation formats).
    pub(crate) const fn half_in_float(self) -> bool {
        matches!(self.identity.rules(), super::RuleSet::Gcc)
    }

    /// Whether constant expressions read compound literals as Clang's
    /// evaluator does: a scalar literal's value folds into the expressions
    /// around it (`(int){3} + 1` initializes a static object), and a literal
    /// initializing a static object is no extension to report. GCC's folder
    /// reads neither and reports the latter under `-pedantic`. The identity
    /// policy gives the MSVC and TCC identities GCC's stricter rules.
    /// C99: §6.6 paragraph 10, p. 96; PDF p. 108 (other constant forms).
    pub(crate) const fn folds_compound_literals(self) -> bool {
        matches!(self.identity.rules(), super::RuleSet::Clang)
    }

    /// GCC's rules (TCC's too) or Clang's (MSVC's and clang-cl's too) for
    /// a cast between a pointer and an integer of another width: GCC warns
    /// for any other width, Clang for a narrower integer only, as MSVC's
    /// C4311 and C4312 do, so MSVC takes the identity policy's
    /// MS-compatible rules.
    /// C99: §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    pub(crate) const fn pointer_cast_rules(self) -> super::RuleSet {
        self.identity.ms_compatible_rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// checking a `_BitInt` width: the value itself against 65535 bits, or
    /// its low 64 bits zero-extended against 2^23, with literal folding.
    /// Neither TCC nor MSVC 19.33 has `_BitInt`; like every C23 type it is
    /// an extension before C23.
    /// C23: §6.7.3.1 paragraph 4, p. 104; PDF p. 117.
    pub(crate) const fn bit_int_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// the type-generic bit builtins: which operand types each accepts,
    /// whether a bit-field operand counts at its own width, and whether
    /// `__builtin_clzg` or `__builtin_ctzg` of zero with no result for zero
    /// folds to the width. GNU extension: GCC manual, "Bit Operation
    /// Builtins".
    pub(crate) const fn bit_builtin_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// `BITINT_MAXWIDTH`, the widest `_BitInt` the rules of
    /// [`Self::bit_int_rules`] accept: 65535 bits for GCC on x86-64 and
    /// 2^23 for Clang, which takes 128 on WebAssembly, where no GCC exists.
    /// C23: §5.2.5.3.2 paragraph 1, pp. 22-23; PDF pp. 35-36.
    pub(crate) fn bit_int_max_width(self) -> u64 {
        if matches!(
            self.target(),
            crate::target::Target::Wasm32Unknown | crate::target::Target::Wasm32Wasi
        ) {
            128
        } else if self.bit_int_rules() == super::RuleSet::Gcc {
            65535
        } else {
            1 << 23
        }
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// an array initialized by an array compound literal: static storage
    /// and compatible element main variants, also inside braces; or any
    /// storage, the same element type and only the whole object. MSVC 19.33
    /// has no such initializer; under GCC's rules it stays a reported
    /// extension of static objects.
    /// GNU extension: GCC manual, "Compound Literals".
    pub(crate) const fn array_literal_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// How the identity's compiler rejects a function definition in a
    /// block, which only GCC accepts, as a GNU nested function: `None` for
    /// GCC, else that compiler's message (Clang's and clang-cl's, TCC's,
    /// and `cl`'s error C2601).
    /// GNU extension: GCC manual, "Nested Functions".
    pub(crate) const fn nested_function_refusal(self) -> Option<&'static str> {
        match self.identity {
            | super::CompilerIdentity::Gcc => None,
            | super::CompilerIdentity::Clang | super::CompilerIdentity::ClangCl =>
                Some("function definition is not allowed here"),
            | super::CompilerIdentity::Tcc => Some("cannot use local functions"),
            | super::CompilerIdentity::Msvc => Some("local function definitions are illegal"),
        }
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// bytes of malformed UTF-8 in a literal: GCC keeps them silently in
    /// unprefixed and `u8` literals and rejects the wide ones; Clang warns
    /// in unprefixed literals and rejects every prefixed one.
    pub(crate) const fn source_encoding_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// `__alignof__` of a function type: 1 for GCC and 4 for Clang, on
    /// every target. GNU extension: GCC manual, "Alignment".
    pub(crate) const fn function_alignment_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// an `alloca` block made inside a VLA's scope: GCC keeps it, and the
    /// scope's stack, until the function returns; Clang releases it when
    /// the scope ends. GNU extension: GCC manual, "Other Builtins".
    pub(crate) const fn alloca_scope_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for a
    /// floating constant converted out of an integer's range: GCC's
    /// bit-field saturation, `-Woverflow` in static initializers and
    /// pedantic overflow report, `__builtin_constant_p` 0; or Clang's
    /// `-Wliteral-conversion` and evaluator.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(crate) const fn float_conversion_rules(self) -> super::RuleSet {
        self.identity.rules()
    }

    /// GCC's rules (TCC's and MSVC's too) or Clang's (clang-cl's too) for
    /// complex integers. Division: GCC divides by the larger part of the
    /// divisor first (its `expand_complex_div_wide`) and each part by a
    /// real divisor, Clang computes `(ac + bd) / (cc + dd)` and
    /// `(bc - ad) / (cc + dd)` for any divisor, each in the parts' type.
    /// Common type: of two parts of equal rank and signedness Clang takes
    /// the left operand's, GCC the left's if it is unsigned, else the
    /// right's. GNU extension: GCC manual,
    /// "Complex Numbers".
    pub(crate) const fn complex_integer_rules(self) -> super::RuleSet {
        self.identity.rules()
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

    /// Whether `$` may appear in identifiers, as GCC's, Clang's and TCC's
    /// `-f[no-]dollars-in-identifiers` decide; it may by default, as in
    /// all three. Extension: C99 §6.4.2.1 paragraph 3, p. 51; PDF p. 63
    /// permits other implementation-defined characters.
    pub(crate) const fn with_dollar_identifiers(mut self, enabled: bool) -> Self {
        self.dollar_identifiers = enabled;
        self.derive_features()
    }

    /// Makes signed integer overflow wrap in two's complement instead of
    /// being undefined, as GCC's `-fwrapv` does. Extension:
    /// C99 §6.5 paragraph 5, p. 67; PDF p. 79 leaves the overflow undefined.
    /// When `-ftrapv` is on too, the option given last decides, as in GCC,
    /// where `-fno-wrapv` turns `-fwrapv` off without undoing what it
    /// overrode: `-ftrapv -fwrapv -fno-wrapv` traps.
    pub(crate) const fn with_wrapv(mut self, enabled: bool) -> Self {
        self.wrapv = enabled;
        if enabled {
            self.trapv_last = false;
        }
        self
    }

    /// Makes signed integer `+`, `-` and `*` trap on overflow, as GCC's
    /// `-ftrapv` does, where they would otherwise be undefined; with
    /// `-fwrapv` also on, the option given last decides. Extension:
    /// C99 §6.5 paragraph 5, p. 67; PDF p. 79 leaves the overflow undefined.
    pub(crate) const fn with_trapv(mut self, enabled: bool) -> Self {
        self.trapv = enabled;
        if enabled {
            self.trapv_last = true;
        }
        self
    }

    /// Whether signed integer overflow wraps (`-fwrapv`).
    pub(crate) const fn wrapv(self) -> bool {
        self.wrapv && !(self.trapv && self.trapv_last)
    }

    /// Whether signed integer `+`, `-` and `*` trap on overflow (`-ftrapv`).
    pub(crate) const fn trapv(self) -> bool {
        self.trapv && !(self.wrapv && !self.trapv_last)
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

    /// Asks that signaling NaNs be told from quiet ones, as GCC's
    /// `-fsignaling-nans` does. bcc's code always does: an operation on a
    /// signaling NaN gives a quiet NaN, and the operations that only copy
    /// or change the sign (assignment, `-`, `fabs`, `copysign`) and
    /// `__builtin_issignaling` keep the encoding, under any option. Only
    /// the GCC identity's `__SUPPORT_SNAN__` reads it.
    /// C99: Annex F, §F.2.1 paragraph 1, p. 445; PDF p. 457 (signaling NaNs
    /// need not be supported); GNU extension: GCC manual, "Optimize
    /// Options".
    pub(crate) const fn with_signaling_nans(mut self, enabled: bool) -> Self {
        self.signaling_nans = enabled;
        self
    }

    /// Whether `-fsignaling-nans` is in effect.
    pub(crate) const fn signaling_nans(self) -> bool {
        self.signaling_nans
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
            dollar_identifiers: true,
            wrapv: false,
            trapv: false,
            trapv_last: false,
            default_pack: 0,
            pack_structures: false,
            default_visibility: super::DefaultVisibility::Default,
            relaxed_floating_point: 0,
            given_floating_point: 0,
            signaling_nans: false,
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
