//! [`CompilerConfiguration`] records the target, C revision, extension
//! policy, and independent Microsoft feature flags. Configuration changes
//! recompute which features are accepted and which belong to the selected
//! language mode. Phases query these sets instead of repeating dialect checks.
//!
//! For strict C99, `inline` is native. Most GNU syntax, such as statement
//! expressions, is accepted in every mode; enabling GNU extensions makes it
//! native too, so [`CompilerConfiguration::is_native`] separates a GNU mode
//! from a strict one. [`Feature::origin`] still marks the syntax as non-ISO.
//!
//! Read [`CompilerConfiguration::accepts`] and
//! [`CompilerConfiguration::is_native`], then [`CompilerConfiguration`],
//! [`Feature::origin`], and [`CompilerConfiguration::new`].
//!
//! Files by role:
//! - Language choices: `language.rs`.
//! - Feature vocabulary and origin: `features.rs`.
//! - Construction, option changes, and accessors: `options.rs`.
//! - Feature-policy fixtures: `tests.rs`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Extensions: §4 paragraph 6, p. 7; PDF p. 19.
//! Feature availability describes the target contract; implementation status
//! is recorded in language-standards.md.

// Language choices
mod language;

// Feature policy
mod features;

// Construction and accessors
mod options;

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
    /// One bit per `MsvcFeature`, plus
    /// [`MSVC_COMPATIBILITY`](features::MSVC_COMPATIBILITY).
    msvc: u16,
    extension_policy: ExtensionPolicy,
    accepted: u128,
    native: u128,
    repeated_specifier_warnings: bool,
    source_date_epoch: Option<i64>,
    hosted: bool,
}

// Tests
#[cfg(test)]
mod tests;
