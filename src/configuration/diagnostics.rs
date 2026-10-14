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
        if !option.enabled.unwrap_or(group.enabled_by_default()) {
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
                || (group.pedantic_promoted() && self.extension_policy == ExtensionPolicy::Deny),
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

/// Declares the warning groups once: each variant's Clang group name, its
/// default and whether `-pedantic-errors` promotes it. `ALL`, `COUNT`,
/// `name` and `parse` derive from this one list, so they cannot drift.
macro_rules! diagnostic_groups {
    ($($(#[$doc:meta])* $variant:ident => $name:expr, $default:ident, $pedantic:literal;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum DiagnosticGroup {
            $($(#[$doc])* $variant,)*
        }

        impl DiagnosticGroup {
            /// Every group, in declaration (and settings-index) order.
            pub(crate) const ALL: &[Self] = &[$(Self::$variant,)*];
            pub(crate) const COUNT: usize = Self::ALL.len();

            /// The Clang `-W` spelling, if the group has one.
            pub(crate) const fn name(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            const fn default(self) -> GroupDefault {
                match self {
                    $(Self::$variant => GroupDefault::$default,)*
                }
            }

            /// Clang's ExtWarn/ExtError diagnostics, which `-pedantic-errors`
            /// makes errors; plain warnings keep their severity.
            const fn pedantic_promoted(self) -> bool {
                match self {
                    $(Self::$variant => $pedantic,)*
                }
            }
        }
    };
}

// Groups observed with the bundled Clang. Ordered pointer/integer
// comparison has no named Clang group; it still follows
// -Werror/-pedantic-errors.
diagnostic_groups! {
    CompareDistinctPointerTypes => Some("compare-distinct-pointer-types"), Warning, true;
    IncompatiblePointerTypes => Some("incompatible-pointer-types"), Error, true;
    PointerSign => Some("pointer-sign"), Warning, true;
    PointerQualifiers => Some("incompatible-pointer-types-discards-qualifiers"), Warning, true;
    IncompatibleFunctionPointerTypes => Some("incompatible-function-pointer-types"), Error, true;
    IntConversion => Some("int-conversion"), Error, true;
    PointerIntegerCompare => Some("pointer-integer-compare"), Warning, true;
    OrderedCompareFunctionPointers => Some("ordered-compare-function-pointers"), Warning, true;
    OrderedPointerIntegerCompare => None, Warning, true;
    /// Conditional operands that point to incompatible types.
    PointerTypeMismatch => Some("pointer-type-mismatch"), Warning, true;
    /// A conditional operand pairing a pointer with an integer.
    ConditionalTypeMismatch => Some("conditional-type-mismatch"), Warning, true;
}

/// A group's severity when no option names it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupDefault {
    Error,
    Warning,
    Ignored,
}

impl DiagnosticGroup {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|group| group.name() == Some(name))
    }

    pub(crate) const fn default_severity(self) -> ErrorSeverity {
        match self.default() {
            | GroupDefault::Error => ErrorSeverity::Error,
            | GroupDefault::Warning | GroupDefault::Ignored => ErrorSeverity::Warning,
        }
    }

    /// Whether the group reports anything before an option enables it.
    pub(crate) fn enabled_by_default(self) -> bool {
        self.default() != GroupDefault::Ignored
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
