//! Language modes and extension settings for translation phases 1-7.
//!
//! C99: §5.1.1.2, pp. 9-10; PDF pp. 21-22. Extensions follow §4p6,
//! p. 7; PDF p. 19. Availability describes the target contract, not the
//! implementation status; see language-standards.md.

mod features;

mod language;

mod options;

use features::MSVC_COMPATIBILITY;
pub(crate) use features::{
    Feature,
    FeatureOrigin,
};
pub(crate) use language::{
    CStandard,
    ExtensionPolicy,
    LanguageMode,
    MsvcFeature,
    PreprocessingOption,
};

impl CompilerConfiguration {
    /// One precomputed bit test on hot paths; does not imply implemented
    /// grammar.
    pub(crate) const fn accepts(self, feature: Feature) -> bool {
        self.accepted & feature.bit() != 0
    }

    pub(crate) const fn is_native(self, feature: Feature) -> bool {
        self.native & feature.bit() != 0
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct CompilerConfiguration {
    target: crate::target::Target,
    standard: CStandard,
    gnu: bool,
    /// One bit per `MsvcFeature`, plus [`MSVC_COMPATIBILITY`].
    msvc: u16,
    extension_policy: ExtensionPolicy,
    accepted: u128,
    native: u128,
    repeated_specifier_warnings: bool,
    source_date_epoch: Option<i64>,
    hosted: bool,
}

#[cfg(test)]
mod tests;
