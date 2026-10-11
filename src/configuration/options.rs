//! Constructs configurations, applies independent options, and recomputes
//! accepted and native feature sets after language choices change.

use super::{
    CStandard,
    CompilerConfiguration,
    ExtensionPolicy,
    Feature,
    FeatureOrigin,
    MSVC_COMPATIBILITY,
    MsvcFeature,
};

impl CompilerConfiguration {
    pub(super) const fn derive_features(mut self) -> Self {
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

    pub(crate) const fn with_gnu_extensions(mut self, enabled: bool) -> Self {
        self.gnu = enabled;
        self.derive_features()
    }

    pub(crate) const fn with_extension_policy(mut self, policy: ExtensionPolicy) -> Self {
        self.extension_policy = policy;
        self
    }

    pub(crate) const fn with_hosted(mut self, hosted: bool) -> Self {
        self.hosted = hosted;
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

    /// Whether the last MSVC umbrella flag was `-fms-extensions`, which
    /// predefines `_MSC_VER` as Clang does; the individual groups do not.
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
            standard,
            gnu: false,
            msvc: 0,
            extension_policy,
            accepted: 0,
            native: 0,
            repeated_specifier_warnings: true,
            source_date_epoch: None,
            hosted: true,
        }
        .derive_features()
    }
}

impl Default for CompilerConfiguration {
    fn default() -> Self {
        Self::new(CStandard::C99, ExtensionPolicy::Allow)
    }
}
