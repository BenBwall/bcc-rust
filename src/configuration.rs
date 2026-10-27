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
//! - Warning groups and severity overrides: `diagnostics.rs`.
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
mod diagnostics;
mod features;

// Construction and accessors
mod options;

pub(crate) use diagnostics::{
    DiagnosticFormat,
    DiagnosticGroup,
    WarningOption,
};
pub(crate) use features::{
    Feature,
    FeatureOrigin,
};
pub(crate) use language::{
    CStandard,
    CompilerIdentity,
    DefaultVisibility,
    ExtensionPolicy,
    LanguageMode,
    LibraryBuiltins,
    MsvcFeature,
    PreprocessingOption,
    RelaxedFloatingPoint,
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "Language, hosted execution and diagnostic switches are independent compiler options."
)]
pub(crate) struct CompilerConfiguration {
    target: crate::target::Target,
    code_options: crate::target::CodeOptions,
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
    optimization: (bool, bool),
    gnu89_inline: Option<bool>,
    /// Whether plain `char` is signed, overriding the target's choice, as
    /// `-fsigned-char` and `-funsigned-char` do.
    char_signed: Option<bool>,
    /// `-ffloatn-types` or `-fno-floatn-types`, over the identity's default.
    interchange_float_types: Option<bool>,
    /// `-ftrigraphs` or `-fno-trigraphs`, over the mode's default.
    trigraphs: Option<bool>,
    /// Signed integer overflow wraps (`-fwrapv`).
    wrapv: bool,
    /// Every structure is packed and `#pragma pack` ignored (`-fpack-struct`
    /// without a value).
    pack_structures: bool,
    /// The visibility of definitions that name none (`-fvisibility=`).
    default_visibility: DefaultVisibility,
    /// One bit per [`RelaxedFloatingPoint`] option in effect.
    relaxed_floating_point: u8,
    /// One bit per [`RelaxedFloatingPoint`] option given in either form.
    given_floating_point: u8,
    /// The structure packing in effect before any `#pragma pack`, and that
    /// `#pragma pack()` restores (`-fpack-struct=N`, `cl /ZpN`); 0 for
    /// none.
    default_pack: u32,
    suppress_warnings: bool,
    warnings_as_errors: bool,
    /// The compiler the identity macros claim.
    identity: CompilerIdentity,
    diagnostic_format: DiagnosticFormat,
    /// Most recursive grammar productions active at once; see
    /// [`DEFAULT_MAX_NESTING_DEPTH`].
    max_nesting_depth: u32,
    warning_options: [diagnostics::WarningSettings; DiagnosticGroup::COUNT],
}

/// Default ceiling on the recursive grammar productions active at once:
/// every nested expression operand, statement, block, declarator, initializer
/// brace and specifier body counts one level, so one source nesting level
/// costs one to three. 2^18 levels accept nesting far deeper than GCC or
/// Clang compile on their default stacks. The archived stack measurements
/// (`docs/research/parser-reserved-stack.md`) are below 1 KiB per level in
/// release and 8 KiB in debug, so the limit is reached long before the
/// reserved 100 GiB compilation stack is exhausted: deep input gets a
/// diagnostic instead of a stack overflow.
///
/// C99: §5.2.4.1, pp. 20-21; PDF pp. 32-33 requires at least 63 nesting
/// levels of parenthesized expressions, 127 of blocks, and 12 declarator
/// derivations; footnote 13 asks implementations to avoid fixed limits, so
/// the limit is configurable (`--max-nesting-depth`).
pub(crate) const DEFAULT_MAX_NESTING_DEPTH: u32 = 1 << 18;

// Tests
#[cfg(test)]
mod tests;
