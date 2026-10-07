//! Language modes and extension settings for translation phases 1-7.
//!
//! C99: §5.1.1.2, pp. 9-10; PDF pp. 21-22. Extensions follow §4p6,
//! p. 7; PDF p. 19. Availability describes the target contract, not the
//! implementation status; see language-standards.md.

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
}

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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExtensionPolicy {
    Allow,
    Warn,
    Deny,
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

/// Native origin used for policy diagnostics, independent of acceptance.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum FeatureOrigin {
    Standard(CStandard),
    Gnu,
    Msvc(MsvcFeature),
}

/// Shared feature vocabulary. Reserved or unambiguous syntax is accepted
/// earlier as an extension; spellings that change valid programs are gated.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub(crate) enum Feature {
    Digraphs,
    LineComments,
    Trigraphs,
    UnicodeLiteralPrefixes,
    Utf8CharacterConstants,
    DigitSeparators,
    BitIntSuffixes,
    BinaryConstants,
    Elifdef,
    WarningDirective,
    Embed,
    HasInclude,
    HasEmbed,
    HasCAttribute,
    VaOpt,
    OctalPrefix,
    DelimitedEscapes,
    HexFloats,
    VariadicMacros,
    EmptyMacroArguments,
    Inline,
    Restrict,
    Bool,
    Complex,
    Imaginary,
    ImplicitInt,
    MixedDeclarations,
    ForDeclarations,
    DesignatedInitializers,
    CompoundLiterals,
    FlexibleArrayMembers,
    LongLong,
    TrailingEnumComma,
    Func,
    StaticAssert,
    Generic,
    Alignas,
    Alignof,
    Noreturn,
    ThreadLocal,
    Atomic,
    AnonymousAggregates,
    C23Keywords,
    Attributes,
    BitInt,
    DecimalTypes,
    EmptyInitializers,
    EnumUnderlyingType,
    AutoTypeInference,
    Constexpr,
    Nullptr,
    Countof,
    IfSwitchDeclarations,
    NamedLoops,
    GenericTypeOperand,
    CaseRanges,
    GnuAttribute,
    GnuAsm,
    GnuTypeof,
    ExtensionMarker,
    StatementExpressions,
    BuiltinVaArg,
    BuiltinOffsetof,
    BuiltinTypesCompatible,
    BuiltinChooseExpr,
    LabelsAsValues,
    LocalLabels,
    OmittedConditionalOperand,
    ZeroLengthArrays,
    Int128,
    AutoType,
    GnuAlternateKeywords,
    RealImag,
    GnuDesignators,
    UnionCasts,
    EmptyStructs,
    NestedFunctions,
    ImaginaryConstants,
    DollarIdentifiers,
    NamedVariadicMacros,
    GnuVaArgs,
    IncludeNext,
    IdentDirective,
    Counter,
    HasAttribute,
    HasBuiltin,
    MsDeclspec,
    MsIntTypes,
    MsCallingConventions,
    MsTypeQualifiers,
    MsInline,
    MsSeh,
    MsAsm,
    MsPragma,
    MsAnonymousStructs,
    MsVaArgs,
}

impl Feature {
    pub(crate) const ALL: &'static [Self] = &[
        Self::Digraphs,
        Self::LineComments,
        Self::Trigraphs,
        Self::UnicodeLiteralPrefixes,
        Self::Utf8CharacterConstants,
        Self::DigitSeparators,
        Self::BitIntSuffixes,
        Self::BinaryConstants,
        Self::Elifdef,
        Self::WarningDirective,
        Self::Embed,
        Self::HasInclude,
        Self::HasEmbed,
        Self::HasCAttribute,
        Self::VaOpt,
        Self::OctalPrefix,
        Self::DelimitedEscapes,
        Self::HexFloats,
        Self::VariadicMacros,
        Self::EmptyMacroArguments,
        Self::Inline,
        Self::Restrict,
        Self::Bool,
        Self::Complex,
        Self::Imaginary,
        Self::ImplicitInt,
        Self::MixedDeclarations,
        Self::ForDeclarations,
        Self::DesignatedInitializers,
        Self::CompoundLiterals,
        Self::FlexibleArrayMembers,
        Self::LongLong,
        Self::TrailingEnumComma,
        Self::Func,
        Self::StaticAssert,
        Self::Generic,
        Self::Alignas,
        Self::Alignof,
        Self::Noreturn,
        Self::ThreadLocal,
        Self::Atomic,
        Self::AnonymousAggregates,
        Self::C23Keywords,
        Self::Attributes,
        Self::BitInt,
        Self::DecimalTypes,
        Self::EmptyInitializers,
        Self::EnumUnderlyingType,
        Self::AutoTypeInference,
        Self::Constexpr,
        Self::Nullptr,
        Self::Countof,
        Self::IfSwitchDeclarations,
        Self::NamedLoops,
        Self::GenericTypeOperand,
        Self::CaseRanges,
        Self::GnuAttribute,
        Self::GnuAsm,
        Self::GnuTypeof,
        Self::ExtensionMarker,
        Self::StatementExpressions,
        Self::BuiltinVaArg,
        Self::BuiltinOffsetof,
        Self::BuiltinTypesCompatible,
        Self::BuiltinChooseExpr,
        Self::LabelsAsValues,
        Self::LocalLabels,
        Self::OmittedConditionalOperand,
        Self::ZeroLengthArrays,
        Self::Int128,
        Self::AutoType,
        Self::GnuAlternateKeywords,
        Self::RealImag,
        Self::GnuDesignators,
        Self::UnionCasts,
        Self::EmptyStructs,
        Self::NestedFunctions,
        Self::ImaginaryConstants,
        Self::DollarIdentifiers,
        Self::NamedVariadicMacros,
        Self::GnuVaArgs,
        Self::IncludeNext,
        Self::IdentDirective,
        Self::Counter,
        Self::HasAttribute,
        Self::HasBuiltin,
        Self::MsDeclspec,
        Self::MsIntTypes,
        Self::MsCallingConventions,
        Self::MsTypeQualifiers,
        Self::MsInline,
        Self::MsSeh,
        Self::MsAsm,
        Self::MsPragma,
        Self::MsAnonymousStructs,
        Self::MsVaArgs,
    ];

    pub(crate) const fn origin(self) -> FeatureOrigin {
        match self {
            | Self::Digraphs => FeatureOrigin::Standard(CStandard::C95),
            | Self::LineComments
            | Self::HexFloats
            | Self::VariadicMacros
            | Self::EmptyMacroArguments
            | Self::Inline
            | Self::Restrict
            | Self::Bool
            | Self::Complex
            | Self::Imaginary
            | Self::MixedDeclarations
            | Self::ForDeclarations
            | Self::DesignatedInitializers
            | Self::CompoundLiterals
            | Self::FlexibleArrayMembers
            | Self::LongLong
            | Self::TrailingEnumComma
            | Self::Func => FeatureOrigin::Standard(CStandard::C99),
            | Self::Trigraphs => FeatureOrigin::Standard(CStandard::C89),
            | Self::UnicodeLiteralPrefixes
            | Self::StaticAssert
            | Self::Generic
            | Self::Alignas
            | Self::Alignof
            | Self::Noreturn
            | Self::ThreadLocal
            | Self::Atomic
            | Self::AnonymousAggregates => FeatureOrigin::Standard(CStandard::C11),
            | Self::Utf8CharacterConstants
            | Self::DigitSeparators
            | Self::BitIntSuffixes
            | Self::BinaryConstants
            | Self::Elifdef
            | Self::WarningDirective
            | Self::Embed
            | Self::HasInclude
            | Self::HasEmbed
            | Self::HasCAttribute
            | Self::VaOpt
            | Self::C23Keywords
            | Self::Attributes
            | Self::BitInt
            | Self::DecimalTypes
            | Self::EmptyInitializers
            | Self::EnumUnderlyingType
            | Self::AutoTypeInference
            | Self::Constexpr
            | Self::Nullptr => FeatureOrigin::Standard(CStandard::C23),

            | Self::OctalPrefix
            | Self::DelimitedEscapes
            | Self::Countof
            | Self::IfSwitchDeclarations
            | Self::NamedLoops
            | Self::GenericTypeOperand
            | Self::CaseRanges => FeatureOrigin::Standard(CStandard::C2y),

            | Self::ImplicitInt
            | Self::GnuAttribute
            | Self::GnuAsm
            | Self::GnuTypeof
            | Self::ExtensionMarker
            | Self::StatementExpressions
            | Self::BuiltinVaArg
            | Self::BuiltinOffsetof
            | Self::BuiltinTypesCompatible
            | Self::BuiltinChooseExpr
            | Self::LabelsAsValues
            | Self::LocalLabels
            | Self::OmittedConditionalOperand
            | Self::ZeroLengthArrays
            | Self::Int128
            | Self::AutoType
            | Self::GnuAlternateKeywords
            | Self::RealImag
            | Self::GnuDesignators
            | Self::UnionCasts
            | Self::EmptyStructs
            | Self::NestedFunctions
            | Self::ImaginaryConstants
            | Self::DollarIdentifiers
            | Self::NamedVariadicMacros
            | Self::GnuVaArgs
            | Self::IncludeNext
            | Self::IdentDirective
            | Self::Counter
            | Self::HasAttribute
            | Self::HasBuiltin => FeatureOrigin::Gnu,

            | Self::MsDeclspec => FeatureOrigin::Msvc(MsvcFeature::Declspec),
            | Self::MsIntTypes => FeatureOrigin::Msvc(MsvcFeature::IntTypes),
            | Self::MsCallingConventions => FeatureOrigin::Msvc(MsvcFeature::CallingConventions),
            | Self::MsTypeQualifiers => FeatureOrigin::Msvc(MsvcFeature::TypeQualifiers),
            | Self::MsInline => FeatureOrigin::Msvc(MsvcFeature::Inline),
            | Self::MsSeh => FeatureOrigin::Msvc(MsvcFeature::Seh),
            | Self::MsAsm => FeatureOrigin::Msvc(MsvcFeature::Asm),
            | Self::MsPragma => FeatureOrigin::Msvc(MsvcFeature::Pragma),
            | Self::MsAnonymousStructs => FeatureOrigin::Msvc(MsvcFeature::AnonymousStructs),
            | Self::MsVaArgs => FeatureOrigin::Msvc(MsvcFeature::VaArgs),
        }
    }

    const fn bit(self) -> u128 {
        1 << self as u8
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct CompilerConfiguration {
    standard: CStandard,
    gnu: bool,
    msvc: u16,
    extension_policy: ExtensionPolicy,
    accepted: u128,
    native: u128,
    repeated_specifier_warnings: bool,
    source_date_epoch: Option<i64>,
}

impl CompilerConfiguration {
    pub(crate) const fn new(standard: CStandard, extension_policy: ExtensionPolicy) -> Self {
        Self {
            standard,
            gnu: false,
            msvc: 0,
            extension_policy,
            accepted: 0,
            native: 0,
            repeated_specifier_warnings: true,
            source_date_epoch: None,
        }
        .derive_features()
    }

    pub(crate) const fn standard(self) -> CStandard {
        self.standard
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

    pub(crate) const fn with_gnu_extensions(mut self, enabled: bool) -> Self {
        self.gnu = enabled;
        self.derive_features()
    }

    pub(crate) const fn with_extension_policy(mut self, policy: ExtensionPolicy) -> Self {
        self.extension_policy = policy;
        self
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

    pub(crate) const fn with_msvc_extensions(mut self, enabled: bool) -> Self {
        self.msvc = if enabled {
            (1 << MsvcFeature::ALL.len()) - 1
        } else {
            0
        };
        self.derive_features()
    }

    pub(crate) const fn msvc_feature(self, feature: MsvcFeature) -> bool {
        self.msvc & (1 << feature as u8) != 0
    }

    /// One precomputed bit test on hot paths; does not imply implemented
    /// grammar.
    pub(crate) const fn accepts(self, feature: Feature) -> bool {
        self.accepted & feature.bit() != 0
    }

    pub(crate) const fn is_native(self, feature: Feature) -> bool {
        self.native & feature.bit() != 0
    }

    pub(crate) const fn origin_is_native(self, origin: FeatureOrigin) -> bool {
        match origin {
            | FeatureOrigin::Standard(standard) => self.standard as u8 >= standard as u8,
            | FeatureOrigin::Gnu => self.gnu,
            | FeatureOrigin::Msvc(feature) => self.msvc_feature(feature),
        }
    }

    const fn derive_features(mut self) -> Self {
        self.accepted = 0;
        self.native = 0;
        let mut index = 0;
        while index < Feature::ALL.len() {
            let feature = Feature::ALL[index];
            let native = if matches!(feature, Feature::ImplicitInt) {
                (self.standard as u8) < CStandard::C99 as u8
            } else {
                self.origin_is_native(feature.origin())
            };
            if native {
                self.native |= feature.bit();
            }
            let accepted = match feature {
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
                | Feature::Trigraphs => !self.gnu && (self.standard as u8) <= CStandard::C17 as u8,
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
}

impl Default for CompilerConfiguration {
    fn default() -> Self {
        Self::new(CStandard::C99, ExtensionPolicy::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_modes_derive_acceptance_and_native_features() {
        for standard in [
            CStandard::C89,
            CStandard::C95,
            CStandard::C99,
            CStandard::C11,
            CStandard::C17,
            CStandard::C23,
            CStandard::C2y,
        ] {
            for gnu in [false, true] {
                let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_gnu_extensions(gnu);
                for &feature in Feature::ALL {
                    if feature == Feature::ImplicitInt {
                        continue;
                    }
                    assert_eq!(
                        configuration.is_native(feature),
                        configuration.origin_is_native(feature.origin())
                    );
                }
                assert_eq!(
                    configuration.accepts(Feature::LineComments),
                    standard >= CStandard::C99 || gnu
                );
                assert_eq!(
                    configuration.accepts(Feature::Digraphs),
                    standard >= CStandard::C95 || gnu
                );
                assert_eq!(
                    configuration.accepts(Feature::Trigraphs),
                    standard <= CStandard::C17 && !gnu
                );
                assert_eq!(
                    configuration.accepts(Feature::Inline),
                    standard >= CStandard::C99 || gnu
                );
                assert_eq!(
                    configuration.accepts(Feature::Restrict),
                    standard >= CStandard::C99
                );
                assert_eq!(
                    configuration.accepts(Feature::C23Keywords),
                    standard >= CStandard::C23
                );
                assert_eq!(
                    configuration.accepts(Feature::UnicodeLiteralPrefixes),
                    standard >= CStandard::C11
                );
                assert_eq!(
                    configuration.is_native(Feature::ImplicitInt),
                    standard < CStandard::C99
                );
                assert!(configuration.accepts(Feature::StaticAssert));
                assert!(!configuration.accepts(Feature::MsSeh));
            }
        }
    }
    #[test]
    fn msvc_groups_are_independent_and_recomputed() {
        let all = CompilerConfiguration::default().with_msvc_extensions(true);
        for feature in MsvcFeature::ALL {
            assert!(all.msvc_feature(feature));
        }
        let without_seh = all.with_msvc_feature(MsvcFeature::Seh, false);
        assert!(!without_seh.accepts(Feature::MsSeh));
        assert!(without_seh.accepts(Feature::MsDeclspec));
        assert!(!without_seh.gnu_extensions());
        assert_eq!(without_seh.standard(), CStandard::C99);
        assert!(!all.with_msvc_extensions(false).accepts(Feature::MsDeclspec));
    }
    #[test]
    fn dialect_aliases_are_complete() {
        assert_eq!(
            LanguageMode::parse("c89"),
            Some(LanguageMode {
                standard: CStandard::C89,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("c90"),
            Some(LanguageMode {
                standard: CStandard::C89,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:1990"),
            Some(LanguageMode {
                standard: CStandard::C89,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:199409"),
            Some(LanguageMode {
                standard: CStandard::C95,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu89"),
            Some(LanguageMode {
                standard: CStandard::C89,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu90"),
            Some(LanguageMode {
                standard: CStandard::C89,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("c99"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("c9x"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:1999"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:199x"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu99"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu9x"),
            Some(LanguageMode {
                standard: CStandard::C99,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("c11"),
            Some(LanguageMode {
                standard: CStandard::C11,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("c1x"),
            Some(LanguageMode {
                standard: CStandard::C11,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:2011"),
            Some(LanguageMode {
                standard: CStandard::C11,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu11"),
            Some(LanguageMode {
                standard: CStandard::C11,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu1x"),
            Some(LanguageMode {
                standard: CStandard::C11,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("c17"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("c18"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:2017"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:2018"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu17"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu18"),
            Some(LanguageMode {
                standard: CStandard::C17,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("c23"),
            Some(LanguageMode {
                standard: CStandard::C23,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("c2x"),
            Some(LanguageMode {
                standard: CStandard::C23,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("iso9899:2024"),
            Some(LanguageMode {
                standard: CStandard::C23,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu23"),
            Some(LanguageMode {
                standard: CStandard::C23,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu2x"),
            Some(LanguageMode {
                standard: CStandard::C23,
                gnu:      true,
            })
        );
        assert_eq!(
            LanguageMode::parse("c2y"),
            Some(LanguageMode {
                standard: CStandard::C2y,
                gnu:      false,
            })
        );
        assert_eq!(
            LanguageMode::parse("gnu2y"),
            Some(LanguageMode {
                standard: CStandard::C2y,
                gnu:      true,
            })
        );
        assert_eq!(LanguageMode::parse("c98"), None);
    }
}
