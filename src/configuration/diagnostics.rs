//! Clang-compatible warning groups and severity overrides, independent of
//! feature availability. C99: §5.1.1.3p1, p. 11; PDF p. 23.

use super::{
    CompilerConfiguration,
    ExtensionPolicy,
};
use crate::translation_phases::ErrorSeverity;

impl CompilerConfiguration {
    /// Specific group options override global -Werror regardless of order.
    /// Parent group options update their children at the same argv position.
    pub(crate) fn with_warning_option(
        mut self,
        group: DiagnosticGroup,
        option: WarningOption,
    ) -> Self {
        self.warning_options[group as usize].apply(option);
        if group == DiagnosticGroup::IncompatiblePointerTypes {
            self.warning_options[DiagnosticGroup::PointerQualifiers as usize].apply(option);
            self.warning_options[DiagnosticGroup::IncompatibleFunctionPointerTypes as usize]
                .apply(option);
        }
        self
    }

    pub(crate) const fn with_warnings_as_errors(mut self, enabled: bool) -> Self {
        self.warnings_as_errors = enabled;
        self
    }

    /// Used for existing, ungrouped diagnostics from every translation phase.
    pub(crate) const fn warning_severity(self, severity: ErrorSeverity) -> ErrorSeverity {
        if matches!(severity, ErrorSeverity::Warning)
            && self.warnings_as_errors
            && !self.suppress_warnings
        {
            ErrorSeverity::Error
        } else {
            severity
        }
    }

    /// Constraint diagnostics remain enabled under Allow; pedantic Deny
    /// promotes them. Explicit group overrides can suppress or demote even
    /// default errors, just as Clang does.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23 permits warning diagnostics.
    pub(crate) fn diagnostic_severity(self, group: DiagnosticGroup) -> Option<ErrorSeverity> {
        let option = self.warning_options[group as usize];
        if option.enabled == Some(false) {
            return None;
        }
        let default = if option.reenabled_warning {
            ErrorSeverity::Warning
        } else {
            group.default_severity()
        };
        if self.suppress_warnings && default == ErrorSeverity::Warning {
            return None;
        }
        let error = option.error.unwrap_or(
            default == ErrorSeverity::Error
                || self.warnings_as_errors
                || self.extension_policy == ExtensionPolicy::Deny,
        );
        let severity = if error {
            ErrorSeverity::Error
        } else {
            ErrorSeverity::Warning
        };
        if self.suppress_warnings && severity == ErrorSeverity::Warning {
            None
        } else {
            Some(severity)
        }
    }
}

/// Pointer constraints observed with the bundled Clang. Ordered pointer/integer
/// comparison has no named Clang group; it still follows
/// -Werror/-pedantic-errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticGroup {
    CompareDistinctPointerTypes,
    IncompatiblePointerTypes,
    PointerSign,
    PointerQualifiers,
    IncompatibleFunctionPointerTypes,
    IntConversion,
    PointerIntegerCompare,
    OrderedCompareFunctionPointers,
    OrderedPointerIntegerCompare,
}

impl DiagnosticGroup {
    pub(crate) const COUNT: usize = 9;

    pub(crate) const fn name(self) -> Option<&'static str> {
        match self {
            | Self::CompareDistinctPointerTypes => Some("compare-distinct-pointer-types"),
            | Self::IncompatiblePointerTypes => Some("incompatible-pointer-types"),
            | Self::PointerSign => Some("pointer-sign"),
            | Self::PointerQualifiers => Some("incompatible-pointer-types-discards-qualifiers"),
            | Self::IncompatibleFunctionPointerTypes => Some("incompatible-function-pointer-types"),
            | Self::IntConversion => Some("int-conversion"),
            | Self::PointerIntegerCompare => Some("pointer-integer-compare"),
            | Self::OrderedCompareFunctionPointers => Some("ordered-compare-function-pointers"),
            | Self::OrderedPointerIntegerCompare => None,
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        [
            Self::CompareDistinctPointerTypes,
            Self::IncompatiblePointerTypes,
            Self::PointerSign,
            Self::PointerQualifiers,
            Self::IncompatibleFunctionPointerTypes,
            Self::IntConversion,
            Self::PointerIntegerCompare,
            Self::OrderedCompareFunctionPointers,
        ]
        .into_iter()
        .find(|group| group.name() == Some(name))
    }

    pub(crate) const fn default_severity(self) -> ErrorSeverity {
        match self {
            | Self::IncompatiblePointerTypes
            | Self::IncompatibleFunctionPointerTypes
            | Self::IntConversion => ErrorSeverity::Error,
            | _ => ErrorSeverity::Warning,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WarningOption {
    Enable(bool),
    Error(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WarningSettings {
    enabled:           Option<bool>,
    error:             Option<bool>,
    /// Clang re-enables a disabled default-error group as a warning.
    reenabled_warning: bool,
}

impl WarningSettings {
    pub(super) const DEFAULT: Self = Self {
        enabled:           None,
        error:             None,
        reenabled_warning: false,
    };

    fn apply(&mut self, option: WarningOption) {
        match option {
            | WarningOption::Enable(enabled) => {
                self.enabled = Some(enabled);
                if !enabled {
                    self.reenabled_warning = true;
                    if self.error == Some(true) {
                        self.error = None;
                    }
                }
            },
            | WarningOption::Error(error) => {
                self.error = Some(error);
                if error {
                    self.enabled = Some(true);
                }
            },
        }
    }
}
