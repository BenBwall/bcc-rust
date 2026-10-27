//! Clang-compatible warning groups and severity overrides, independent of
//! feature availability. C99: §5.1.1.3p1, p. 11; PDF p. 23.

use super::{
    CompilerConfiguration,
    ExtensionPolicy,
};
use crate::translation_phases::ErrorSeverity;

/// How diagnostics are printed, which `--diagnostic-format` and the driver
/// personalities select: bcc's annotated snippets, Clang's and GCC's
/// `file:line:col: error: message` lines with the snippet (Clang adds the
/// error and warning counts), MSVC's `file(line,col): error: message`, or
/// TCC's `file:line: error: message`.
/// The wording and the snippets are bcc's in every format.
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23 leaves the form of a
/// diagnostic implementation-defined.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub(crate) enum DiagnosticFormat {
    #[default]
    Bcc,
    Clang,
    Gcc,
    /// `cl.exe`'s location format, with the column for `/diagnostics:column`
    /// (the default here, as clang-cl writes it) and the snippet for
    /// `/diagnostics:caret`.
    Msvc {
        column: bool,
        caret:  bool,
    },
    /// TCC's `file:line: error: message`, without a snippet.
    Tcc,
}

impl CompilerConfiguration {
    pub(crate) const fn with_diagnostic_format(mut self, format: DiagnosticFormat) -> Self {
        self.diagnostic_format = format;
        self
    }

    pub(crate) const fn diagnostic_format(self) -> DiagnosticFormat {
        self.diagnostic_format
    }

    /// Specific group options override global -Werror regardless of order.
    /// Parent group options update their children at the same argv position.
    pub(crate) fn with_warning_option(
        mut self,
        group: DiagnosticGroup,
        option: WarningOption,
    ) -> Self {
        self.warning_options[group as usize].apply(option);
        if group == DiagnosticGroup::UnknownPragmas {
            self.warning_options[DiagnosticGroup::UnknownStdcPragma as usize].apply(option);
        }
        if group == DiagnosticGroup::IncompatiblePointerTypes {
            self.warning_options[DiagnosticGroup::PointerQualifiers as usize].apply(option);
            self.warning_options[DiagnosticGroup::IncompatibleFunctionPointerTypes as usize]
                .apply(option);
        }
        self
    }

    /// Enables every group that is off by default, for tests that observe
    /// those diagnostics directly.
    #[cfg(test)]
    pub(crate) fn with_default_off_groups(mut self) -> Self {
        for &group in DiagnosticGroup::ALL {
            if !group.enabled_by_default() {
                self = self.with_warning_option(group, WarningOption::Enable(true));
            }
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

    /// Whether a warning was explicitly requested, independently of its
    /// context-dependent default (for example GNU folding outside bounds).
    pub(crate) fn diagnostic_requested(self, group: DiagnosticGroup) -> bool {
        self.warning_options[group as usize].enabled == Some(true)
    }

    /// Constraint diagnostics remain enabled under Allow; pedantic Deny
    /// promotes them. Explicit group overrides can suppress or demote even
    /// default errors, just as Clang does.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23 permits warning diagnostics.
    pub(crate) fn diagnostic_severity(self, group: DiagnosticGroup) -> Option<ErrorSeverity> {
        let option = self.warning_options[group as usize];
        if !option.enabled.unwrap_or(
            group.enabled_by_default()
                || (group.default() == GroupDefault::Extension
                    && self.extension_policy != ExtensionPolicy::Allow),
        ) {
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

            /// The `-W` spelling, if the group has one: Clang's, except for
            /// the bcc-only `constant-promotion`.
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
    NonLiteralNullConversion => Some("non-literal-null-conversion"), Warning, false;
    PointerCompare => Some("pointer-compare"), Warning, false;
    NullConversion => Some("null-conversion"), Warning, false;
    PointerBoolConversion => Some("pointer-bool-conversion"), Warning, false;
    PointerToIntCast => Some("pointer-to-int-cast"), Warning, false;
    VoidPtrDereference => Some("void-ptr-dereference"), Warning, false;
    Nonnull => Some("nonnull"), Warning, false;
    PredefinedOutsideFunction => Some("predefined-identifier-outside-function"), Warning, false;
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
    /// A nonempty source file that does not end in a new-line; Clang reports
    /// it only when requested and never through `-pedantic`.
    NewlineEof => Some("newline-eof"), Ignored, false;
    /// An identifier evaluated as 0 in `#if`.
    Undef => Some("undef"), Ignored, false;
    /// A `#pragma` that names nothing bcc knows.
    UnknownPragmas => Some("unknown-pragmas"), Ignored, false;
    UnknownStdcPragma => Some("unknown-pragmas"), Warning, false;
    /// A `\` before a character that begins no escape sequence; Clang's
    /// extension warning, which `-pedantic-errors` promotes.
    UnknownEscapeSequence => Some("unknown-escape-sequence"), Warning, true;
    UnknownWarningOption => Some("unknown-warning-option"), Warning, false;
    /// An attribute name unavailable in its spelling on the selected target.
    UnknownAttributes => Some("unknown-attributes"), Warning, false;
    /// A recognized attribute whose effect bcc drops.
    IgnoredAttributes => Some("ignored-attributes"), Warning, false;
    /// A surviving direct call carries a GNU `warning` attribute.
    AttributeWarning => Some("attribute-warning"), Warning, false;
    /// A redeclaration that adds `dllimport` or `dllexport`.
    DllAttributeOnRedeclaration => Some("dll-attribute-on-redeclaration"), Warning, false;
    /// A redeclaration of an imported entity without `dllimport`.
    InconsistentDllimport => Some("inconsistent-dllimport"), Warning, false;
    /// A static function declared but neither defined nor used.
    UnusedFunction => Some("unused-function"), Ignored, false;
    /// An initializer that replaces an earlier one for the same subobject
    /// (C99 §6.7.8p19); Clang warns by default.
    InitializerOverrides => Some("initializer-overrides"), Warning, false;
    /// Folding a non-ICE array bound; other constant contexts enable this
    /// group only explicitly or under pedantic, as Clang does.
    GnuFoldingConstant => Some("gnu-folding-constant"), Warning, true;
    /// Clang's additional constant form for addition to a null pointer.
    GnuNullPointerArithmetic => Some("gnu-null-pointer-arithmetic"), Extension, true;
    /// A decimal or unsuffixed constant that takes a wider type of its list.
    /// Valid C99 that neither GCC nor Clang diagnoses, so it stays off; the
    /// bcc-only `-Wconstant-promotion` reports it.
    ConstantPromotion => Some("constant-promotion"), Ignored, false;
    /// An integer constant whose implicit conversion changes its value.
    ConstantConversion => Some("constant-conversion"), Warning, false;
}

/// A group's severity when no option names it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupDefault {
    Error,
    Warning,
    Ignored,
    Extension,
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
            | GroupDefault::Warning | GroupDefault::Ignored | GroupDefault::Extension =>
                ErrorSeverity::Warning,
        }
    }

    /// Whether the group reports anything before an option enables it.
    pub(crate) fn enabled_by_default(self) -> bool {
        matches!(self.default(), GroupDefault::Error | GroupDefault::Warning)
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
