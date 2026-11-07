//! Clang-compatible warning groups and severity overrides, independent of
//! feature availability. C99: §5.1.1.3p1, p. 11; PDF p. 23.

use super::{
    CompilerConfiguration,
    ExtensionPolicy,
};
use crate::translation_phases::ErrorSeverity;

/// What `#pragma GCC diagnostic` or `#pragma clang diagnostic` makes of a
/// warning option. Clang's `fatal` is an `Error` the context also marks.
/// C99: implementation-defined pragma behavior, §6.10.6 paragraph 1, p. 159;
/// PDF p. 171.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PragmaClass {
    Ignored,
    Warning,
    Error,
}

/// How diagnostics are printed, which `--diagnostic-format` and the driver
/// personalities select: bcc's annotated snippets, Clang's and GCC's
/// `file:line:col: error: message` lines with the snippet (Clang adds the
/// error and warning counts), MSVC's `file(line,col): error: message`, or
/// TCC's `file:line: error: message`.
/// The wording and the snippets are bcc's in every format, except for the
/// diagnostics TCC's users meet most and the C type-safety diagnostics GCC 14
/// and Clang make errors, which the TCC, GCC and Clang formats word as
/// their compilers do.
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

/// What the options ask of color in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticColor {
    /// No option: the terminal and bcc's environment variables decide.
    Unset,
    /// `-fdiagnostics-color=auto`: the terminal alone decides.
    Auto,
    Always,
    Never,
}

/// What GCC's and Clang's formats write of a diagnostic, each from the last
/// option that sets it. The other formats have their own rules.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each is a separate option's on or off, in any combination."
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiagnosticPresentation {
    /// The quoted source and its caret: `-f[no-]diagnostics-show-caret`,
    /// `-f[no-]caret-diagnostics`.
    pub(crate) caret:        bool,
    /// The column after the line: `-f[no-]show-column`.
    pub(crate) column:       bool,
    /// The line numbers beside quoted source:
    /// `-f[no-]diagnostics-show-line-numbers`.
    pub(crate) line_numbers: bool,
    /// The `[-Wname]` after a message: `-f[no-]diagnostics-show-option`.
    pub(crate) option_names: bool,
    /// What an option says about color: `-fdiagnostics-color=`,
    /// `-f[no-]color-diagnostics`.
    pub(crate) color:        DiagnosticColor,
}

impl Default for DiagnosticPresentation {
    fn default() -> Self {
        Self {
            caret:        true,
            column:       true,
            line_numbers: true,
            option_names: true,
            color:        DiagnosticColor::Unset,
        }
    }
}

impl CompilerConfiguration {
    pub(crate) const fn with_diagnostic_format(mut self, format: DiagnosticFormat) -> Self {
        self.diagnostic_format = format;
        self
    }

    pub(crate) const fn with_diagnostic_presentation(
        mut self,
        presentation: DiagnosticPresentation,
    ) -> Self {
        self.diagnostic_presentation = presentation;
        self
    }

    pub(crate) const fn diagnostic_presentation(self) -> DiagnosticPresentation {
        self.diagnostic_presentation
    }

    /// What the options ask of color: nothing, in the formats that do not
    /// read them.
    pub(crate) const fn diagnostic_color(self) -> DiagnosticColor {
        match self.diagnostic_format {
            | DiagnosticFormat::Gcc | DiagnosticFormat::Clang => self.diagnostic_presentation.color,
            | _ => DiagnosticColor::Unset,
        }
    }

    pub(crate) const fn diagnostic_format(self) -> DiagnosticFormat {
        self.diagnostic_format
    }

    /// Whether diagnostics take `cl`'s codes and words where bcc models
    /// them: the `cl` identity writing one of `cl`'s formats. clang-cl
    /// writes Clang's words in the same formats.
    pub(crate) fn msvc_wording(self) -> bool {
        self.identity == super::CompilerIdentity::Msvc
            && matches!(self.diagnostic_format, DiagnosticFormat::Msvc { .. })
    }

    /// Specific group options override global -Werror regardless of order.
    /// Parent group options update their children at the same argv position.
    pub(crate) fn with_warning_option(
        mut self,
        group: DiagnosticGroup,
        option: WarningOption,
    ) -> Self {
        let clang = self.identity.rules() == super::RuleSet::Clang;
        self.warning_options[group as usize].apply(option, clang);
        let implies: &[DiagnosticGroup] = match group {
            | DiagnosticGroup::Unused => DiagnosticGroup::UNUSED,
            | DiagnosticGroup::UnusedVariable if !clang => &[DiagnosticGroup::UnusedConstVariable],
            | _ => &[],
        };
        // `-Wunused` holds its groups as `-Wall` does, and GCC's
        // `-Wunused-variable` its `-Wunused-const-variable=1`: their own
        // options outrank it. An option that only sets their severity is an
        // ordinary parent's.
        for &child in implies {
            match option {
                | WarningOption::Enable(enabled) =>
                    self.warning_options[child as usize].apply_all(enabled),
                | WarningOption::Error(error) if error || group == DiagnosticGroup::Unused =>
                    self.warning_options[child as usize].apply(option, clang),
                | WarningOption::Error(_) => {},
            }
        }
        if group == DiagnosticGroup::Unused {
            return self;
        }
        if group == DiagnosticGroup::UnknownPragmas || (group == DiagnosticGroup::Pragmas && clang)
        {
            self.warning_options[DiagnosticGroup::UnknownStdcPragma as usize].apply(option, clang);
        }
        // Clang's `-Wpragmas` holds `-Wunknown-pragmas`; GCC's stands alone.
        if group == DiagnosticGroup::Pragmas && clang {
            self.warning_options[DiagnosticGroup::UnknownPragmas as usize].apply(option, clang);
        }
        let children: &[DiagnosticGroup] = match group {
            | DiagnosticGroup::PointerToIntCast => &[
                DiagnosticGroup::PointerToEnumCast,
                DiagnosticGroup::VoidPointerToIntCast,
                DiagnosticGroup::VoidPointerToEnumCast,
            ],
            | DiagnosticGroup::PointerToEnumCast | DiagnosticGroup::VoidPointerToIntCast =>
                &[DiagnosticGroup::VoidPointerToEnumCast],
            | DiagnosticGroup::IntToPointerCast => &[DiagnosticGroup::IntToVoidPointerCast],
            | DiagnosticGroup::Implicit => &[
                DiagnosticGroup::ImplicitFunctionDeclaration,
                DiagnosticGroup::ImplicitLibraryFunction,
                DiagnosticGroup::ImplicitInt,
            ],
            | DiagnosticGroup::ImplicitFunctionDeclaration =>
                &[DiagnosticGroup::ImplicitLibraryFunction],
            | DiagnosticGroup::ReturnMismatch => &[DiagnosticGroup::VoidReturnMismatch],
            | DiagnosticGroup::IncompatiblePointerTypes =>
                &[DiagnosticGroup::GccConditionalPointerMismatch],
            | DiagnosticGroup::Normalized => &[DiagnosticGroup::PedanticNormalized],
            | DiagnosticGroup::UnusedVariable if clang => &[DiagnosticGroup::UnusedConstVariable],
            | DiagnosticGroup::UnusedButSetVariable if clang =>
                &[DiagnosticGroup::UnusedButSetGlobal],
            | DiagnosticGroup::UnusedValue if clang => &[DiagnosticGroup::UnusedComparison],
            | DiagnosticGroup::Main => &[DiagnosticGroup::MainExtension],
            | DiagnosticGroup::DivByZero => &[DiagnosticGroup::DivisionByZero],
            | DiagnosticGroup::ShiftOverflow => &[DiagnosticGroup::GccShiftOverflow],
            | DiagnosticGroup::TypeLimits if clang => &[
                DiagnosticGroup::TautologicalUnsignedZeroCompare,
                DiagnosticGroup::TautologicalTypeLimitCompare,
            ],
            | DiagnosticGroup::TautologicalConstantCompare =>
                &[DiagnosticGroup::TautologicalConstantOutOfRangeCompare],
            | _ => &[],
        };
        for &child in children {
            self.warning_options[child as usize].apply(option, clang);
        }
        if group == DiagnosticGroup::Overflow {
            self.warning_options[DiagnosticGroup::PedanticOverflow as usize]
                .apply_to_pedantic(option);
        }
        if group == DiagnosticGroup::IncompatiblePointerTypes {
            self.warning_options[DiagnosticGroup::PointerQualifiers as usize].apply(option, clang);
            self.warning_options[DiagnosticGroup::IncompatibleFunctionPointerTypes as usize]
                .apply(option, clang);
        }
        self
    }

    /// `-Wall` (true) or `-Wno-all` at its argv position, for the groups
    /// whose meaning under it bcc models: `-Wimplicit`'s,
    /// `-Wreturn-mismatch`'s and GCC's `-Wmain`. Each group records the setting
    /// apart from its own options, which take precedence in GCC whatever their
    /// order and only when they come later in Clang
    /// ([`WarningSettings::enabled_with_all`]).
    pub(crate) fn with_all_warnings(mut self, enabled: bool) -> Self {
        for group in [
            DiagnosticGroup::ImplicitFunctionDeclaration,
            DiagnosticGroup::ImplicitLibraryFunction,
            DiagnosticGroup::ImplicitInt,
            DiagnosticGroup::ReturnMismatch,
            DiagnosticGroup::VoidReturnMismatch,
        ] {
            self.warning_options[group as usize].apply_all(enabled);
        }
        self.warning_options[DiagnosticGroup::Unused as usize].apply_all(enabled);
        // GCC's `-Wunused`, given or refused in either order, outranks
        // `-Wall`, as the groups' own options outrank it.
        if self.identity.rules() == super::RuleSet::Clang
            || self.warning_options[DiagnosticGroup::Unused as usize]
                .enabled()
                .is_none()
        {
            for &group in DiagnosticGroup::UNUSED {
                self.warning_options[group as usize].apply_all(enabled);
            }
        }
        // GCC's `-Wunused-variable`, given or refused in either order, outranks
        // it for `-Wunused-const-variable=` as well.
        if self.identity.rules() != super::RuleSet::Clang
            && let Some(variable) =
                self.warning_options[DiagnosticGroup::UnusedVariable as usize].enabled()
        {
            self.warning_options[DiagnosticGroup::UnusedConstVariable as usize].apply_all(variable);
        }
        // Clang's `-Wmain` and `-Wpointer-bool-conversion` are on already, and
        // its `-Wall` does not hold them.
        if self.identity.rules() != super::RuleSet::Clang {
            self.warning_options[DiagnosticGroup::Main as usize].apply_all(enabled);
            self.warning_options[DiagnosticGroup::PointerBoolConversion as usize]
                .apply_all(enabled);
        }
        self
    }

    /// `-Wextra` (true) or `-Wno-extra` at its argv position, for the groups
    /// bcc models under it. Clang's holds the parameter groups directly;
    /// GCC's holds them only with `-Wunused`, which
    /// [`Self::diagnostic_severity`] asks when it reads them.
    pub(crate) fn with_extra_warnings(mut self, enabled: bool) -> Self {
        self.warning_options[DiagnosticGroup::Extra as usize].apply_all(enabled);
        for group in [
            DiagnosticGroup::SignCompare,
            DiagnosticGroup::MissingFieldInitializers,
        ] {
            self.warning_options[group as usize].apply_all(enabled);
        }
        if self.identity.rules() != super::RuleSet::Clang {
            self.warning_options[DiagnosticGroup::TypeLimits as usize].apply_all(enabled);
        }
        if self.identity.rules() == super::RuleSet::Clang {
            for group in [
                DiagnosticGroup::UnusedParameter,
                DiagnosticGroup::UnusedButSetParameter,
            ] {
                self.warning_options[group as usize].apply_all(enabled);
            }
        }
        self
    }

    /// This configuration after `#pragma GCC diagnostic` or `#pragma clang
    /// diagnostic` sets the warning option `name` (without its `-W`) to
    /// `class`. A pragma outranks the command line: `warning` keeps a warning
    /// from `-Werror`, and `error` makes one of a warning `-Wno-error=` kept,
    /// though `-w` still silences both, as in GCC and Clang. An option bcc
    /// does not model changes nothing.
    /// GCC: <https://gcc.gnu.org/onlinedocs/gcc/Diagnostic-Pragmas.html>.
    /// Clang: <https://clang.llvm.org/docs/UsersManual.html#controlling-diagnostics-via-pragmas>.
    /// C99: implementation-defined pragma behavior, §6.10.6 paragraph 1,
    /// p. 159; PDF p. 171.
    pub(crate) fn with_pragma_class(self, name: &str, class: PragmaClass) -> Self {
        let clang = self.identity.rules() == super::RuleSet::Clang;
        match name {
            | "pedantic" => self.with_pragma_pedantic(class),
            // GCC's `ignored` of a parent option silences nothing it holds,
            // as it says nothing about the options the parent enabled.
            | "all" | "unused" | "extra" if class == PragmaClass::Ignored && !clang => self,
            | "extra" => self.with_pragma_extra(class),
            // Nor does GCC's `ignored "-Wunused-variable"` reach the
            // `-Wunused-const-variable=` it enables.
            | "unused-variable" if class == PragmaClass::Ignored && !clang => {
                let mut configuration = self;
                configuration.warning_options[DiagnosticGroup::UnusedVariable as usize]
                    .apply(WarningOption::Enable(false), clang);
                configuration
            },
            // GCC's `-Wall` holds `-Wmain`; Clang's does not.
            | "all" => DiagnosticGroup::UNDER_ALL
                .iter()
                .chain((!clang).then_some(&DiagnosticGroup::Main))
                .fold(self, |configuration, &group| {
                    configuration.with_pragma_group(group, class)
                }),
            | "everything" if clang => DiagnosticGroup::ALL
                .iter()
                .fold(self.with_pragma_pedantic(class), |configuration, &group| {
                    configuration.with_pragma_group(group, class)
                }),
            // GCC's `-Waddress` and Clang's group around
            // `-Wpointer-bool-conversion`, the part of them bcc reports.
            | "address" => self.with_pragma_group(DiagnosticGroup::PointerBoolConversion, class),
            | _ => DiagnosticGroup::parse(name)
                .map_or(self, |group| self.with_pragma_group(group, class)),
        }
    }

    /// [`Self::with_pragma_class`] for one group. A group an earlier pragma
    /// disabled keeps its default when `error` turns it back on, so that `-w`
    /// leaves a default error in Clang.
    pub(crate) fn with_pragma_group(self, group: DiagnosticGroup, class: PragmaClass) -> Self {
        let mut configuration = self;
        if self.identity.rules() != super::RuleSet::Clang {
            match group {
                // GCC enables the parameter groups only with `-Wextra` too,
                // which a pragma naming `-Wunused` does not say, but it sets
                // their class.
                | DiagnosticGroup::Unused =>
                    configuration =
                        configuration.with_pragma_class_of(DiagnosticGroup::PARAMETERS, class),
                // `warning` also takes back an error, as `-Wno-error=` does
                // not for this child.
                | DiagnosticGroup::UnusedVariable if class == PragmaClass::Warning => {
                    configuration.warning_options[DiagnosticGroup::UnusedConstVariable as usize]
                        .set(WarningSettings::ERROR, Some(false));
                },
                | _ => {},
            }
        }
        configuration.with_pragma_own_group(group, class)
    }

    fn with_pragma_own_group(self, group: DiagnosticGroup, class: PragmaClass) -> Self {
        match class {
            | PragmaClass::Ignored => self.with_warning_option(group, WarningOption::Enable(false)),
            | PragmaClass::Warning => self
                .with_warning_option(group, WarningOption::Enable(true))
                .with_warning_option(group, WarningOption::Error(false)),
            | PragmaClass::Error => {
                let mut configuration = self.with_warning_option(group, WarningOption::Error(true));
                configuration.warning_options[group as usize].0 &= !WarningSettings::REENABLED;
                configuration
            },
        }
    }

    /// `-Wextra` named by a pragma, which holds what `-Wextra` on the command
    /// line does ([`Self::with_extra_warnings`]).
    fn with_pragma_extra(self, class: PragmaClass) -> Self {
        self.with_extra_warnings(class != PragmaClass::Ignored)
            .with_pragma_class_of(DiagnosticGroup::UNDER_EXTRA, class)
    }

    /// Sets whether `groups` are errors, without enabling them: what enables
    /// them is the option's own business.
    fn with_pragma_class_of(mut self, groups: &[DiagnosticGroup], class: PragmaClass) -> Self {
        let error = match class {
            | PragmaClass::Ignored => return self,
            | PragmaClass::Warning => false,
            | PragmaClass::Error => true,
        };
        for &group in groups {
            self.warning_options[group as usize].set(WarningSettings::ERROR, Some(error));
        }
        self
    }

    /// `-Wpedantic` named by a pragma: an `error` is `-Werror=pedantic`
    /// with the diagnostics on, and `-w` silences it as it does that option.
    fn with_pragma_pedantic(self, class: PragmaClass) -> Self {
        let command_line_pedantic_deny = Some(self.command_line_pedantic_deny());
        let configuration = match class {
            | PragmaClass::Ignored => self.with_pedantic(false),
            | PragmaClass::Warning => self.with_pedantic_as_error(false).with_pedantic(true),
            | PragmaClass::Error if self.suppress_warnings => self.with_pedantic(false),
            | PragmaClass::Error => self
                .with_pedantic_as_error(true)
                .with_werror_pedantic()
                .with_pedantic(true),
        };
        // What the option holds besides the extension policy, whatever the
        // command line said of each.
        let configuration = DiagnosticGroup::ALL
            .iter()
            .filter(|group| group.default() == GroupDefault::Extension)
            .fold(configuration, |configuration, &group| {
                configuration.with_pragma_group(group, class)
            });
        Self {
            command_line_pedantic_deny,
            ..configuration
        }
    }

    /// Whether the command line made pedantic warnings errors.
    fn command_line_pedantic_deny(self) -> bool {
        self.command_line_pedantic_deny
            .unwrap_or(self.extension_policy == ExtensionPolicy::Deny)
    }

    /// One bit per [`DiagnosticGroup`] that a pragma naming the warning
    /// option `name` reaches, parents' children included.
    pub(crate) fn pragma_group_mask(self, name: &str) -> u128 {
        const {
            assert!(
                DiagnosticGroup::COUNT <= u128::BITS as usize,
                "a mask holds one bit per group"
            );
        }
        let mut cleared = self;
        cleared.warning_options = [WarningSettings::DEFAULT; DiagnosticGroup::COUNT];
        let set = cleared.with_pragma_class(name, PragmaClass::Error);
        set.warning_options
            .iter()
            .enumerate()
            .filter(|(_, settings)| **settings != WarningSettings::DEFAULT)
            .fold(0, |mask, (group, _)| mask | (1 << group))
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

    /// Whether `-Werror` makes warnings errors, as `-w` does not let it.
    pub(crate) const fn warnings_are_errors(self) -> bool {
        self.warnings_as_errors && !self.suppress_warnings
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
        self.warning_options[group as usize].enabled() == Some(true)
    }

    /// Constraint diagnostics remain enabled under Allow; pedantic Deny
    /// promotes them. Explicit group overrides can suppress or demote even
    /// default errors, just as Clang does.
    /// C99: §5.1.1.3p1, p. 11; PDF p. 23 permits warning diagnostics.
    pub(crate) fn diagnostic_severity(self, group: DiagnosticGroup) -> Option<ErrorSeverity> {
        let mut option = self.warning_options[group as usize];
        // Clang's `-Wconversion` holds these groups; a child without an
        // option of its own takes the parent's. GCC's `-Wconversion` holds
        // none of Clang's groups, so its options leave them alone.
        if option == WarningSettings::DEFAULT
            && group.converts()
            && self.float_conversion_rules() == super::RuleSet::Clang
        {
            option = self.warning_options[DiagnosticGroup::Conversion as usize];
        }
        let permerror =
            (group.default() == GroupDefault::Permerror).then(|| self.permerror_default(group));
        let enabled = match permerror {
            | Some(default) => default.is_some(),
            | None if group.default() == GroupDefault::ClangWarning =>
                self.identity.rules() == super::RuleSet::Clang,
            | None if matches!(
                group,
                DiagnosticGroup::UnusedParameter | DiagnosticGroup::UnusedButSetParameter
            ) && self.identity.rules() != super::RuleSet::Clang =>
                self.warning_options[DiagnosticGroup::Extra as usize].all() == Some(true)
                    && self.warning_options[DiagnosticGroup::Unused as usize]
                        .enabled_with_all(DiagnosticGroup::Unused, false)
                        == Some(true),
            | None if group.default() == GroupDefault::OnInClang =>
                self.identity != super::CompilerIdentity::Gcc
                    || self.extension_policy != ExtensionPolicy::Allow,
            | None =>
                group.enabled_by_default()
                    || (group.default() == GroupDefault::Extension
                        && self.extension_policy != ExtensionPolicy::Allow),
        };
        let clang = self.identity.rules() == super::RuleSet::Clang;
        if !option.enabled_with_all(group, clang).unwrap_or(enabled) {
            return None;
        }
        // GCC re-enables a disabled group at its default severity.
        let default = if option.reenabled_warning() && clang {
            ErrorSeverity::Warning
        } else {
            match permerror {
                | Some(default) => default.unwrap_or(ErrorSeverity::Warning),
                | None => group.default_severity(),
            }
        };
        // GCC makes a pedantic warning an error before `-w` can silence
        // it, though not one of its preprocessor's (`-Wnormalized`);
        // Clang's `-w` silences it first.
        let gcc = self.identity == super::CompilerIdentity::Gcc;
        let pedantic = self.pedantic_error(group);
        let preprocessor = group == DiagnosticGroup::PedanticNormalized;
        // Clang 23's `-w` also silences its default-error
        // `-Wargument-outside-range`, though not `-Wint-conversion` or
        // `-Wincompatible-function-pointer-types`.
        if self.suppress_warnings
            && !(gcc && pedantic && !preprocessor)
            && (default == ErrorSeverity::Warning || group == DiagnosticGroup::ArgumentOutsideRange)
        {
            return None;
        }
        // GCC's C89 implicit int, when an option enables it, is a pedantic
        // warning that `-fno-permissive` makes an error too.
        let no_permissive =
            gcc && group == DiagnosticGroup::ImplicitInt && self.permissive == Some(false);
        let error = option.error().unwrap_or(
            default == ErrorSeverity::Error || self.warnings_as_errors || pedantic || no_permissive,
        );
        if gcc && pedantic && error {
            return Some(ErrorSeverity::Error);
        }
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

    /// Whether a diagnostic of `group` at `severity` is reported in a
    /// system header, which withholds warnings and warning groups: GCC 14
    /// reports its permerrors there when they are errors, while Clang
    /// withholds its default-error groups too.
    /// Only an error of GCC's own making counts: the permerror's default,
    /// or `-pedantic-errors`; `-Werror` and `-Werror=X` promote warnings
    /// that a system header never reports.
    pub(crate) fn reported_in_system_headers(
        self,
        group: DiagnosticGroup,
        severity: ErrorSeverity,
    ) -> bool {
        severity == ErrorSeverity::Error
            && self.identity == super::CompilerIdentity::Gcc
            && group.default() == GroupDefault::Permerror
            && (self.permerror_default(group) == Some(ErrorSeverity::Error)
                || self.pedantic_error(group))
    }

    /// Whether `-pedantic-errors` makes `group` an error: Clang's
    /// `ExtWarn` diagnostics, except that GCC's `-fpermissive` keeps its
    /// permerrors warnings whatever `-pedantic-errors` says.
    /// GCC's C89 implicit int and `-Wmain`, and Clang's implicitly declared
    /// library function, are pedantic warnings as well, in their compilers
    /// only.
    pub(crate) fn pedantic_error(self, group: DiagnosticGroup) -> bool {
        let promoted = group.pedantic_promoted()
            || match group {
                | DiagnosticGroup::ImplicitInt | DiagnosticGroup::Main =>
                    self.identity == super::CompilerIdentity::Gcc,
                | DiagnosticGroup::ImplicitLibraryFunction =>
                    self.identity.rules() == super::RuleSet::Clang,
                | _ => false,
            };
        // A pragma naming `-Wpedantic` reaches that option's own groups. The
        // rest are errors under `-pedantic-errors` whatever it says, in both
        // compilers: GCC's `pedwarn` and Clang's ExtWarn stay so.
        let deny = if group.default() == GroupDefault::Extension {
            self.extension_policy == ExtensionPolicy::Deny
        } else {
            self.command_line_pedantic_deny()
        };
        promoted
            && deny
            && !(self.identity == super::CompilerIdentity::Gcc
                && group.default() == GroupDefault::Permerror
                && self.permissive == Some(true))
    }

    /// The severity of a [`GroupDefault::Permerror`] group before any
    /// option names it, `None` where it reports nothing: the C type-safety
    /// rules GCC 14 made errors ("permerrors") in C99 and later, which
    /// `-fpermissive` makes warnings again and which stay warnings, or stay
    /// silent, in C89 and GNU89 ("Porting to GCC 14"); Clang 16 and later
    /// make them errors too, a type mismatch and `return` in every mode
    /// (Clang 23, `DiagnosticSemaKinds.td`). TCC and `cl` warn about an
    /// implicit declaration and a `return` mismatch and say nothing about
    /// implicit int; the pointer groups keep Clang's errors under MSVC, and
    /// the TCC driver makes them warnings. `-w` leaves an error.
    /// C99: Foreword paragraph 5, pp. xii-xiii; PDF pp. 10-11 ("remove
    /// implicit int", "remove implicit function declaration", "return
    /// without expression not permitted in function that returns a value
    /// (and vice versa)"); §6.8.6.4p1, p. 139; PDF p. 151; §6.5.16.1p1,
    /// p. 92; PDF p. 104.
    fn permerror_default(self, group: DiagnosticGroup) -> Option<ErrorSeverity> {
        use DiagnosticGroup as G;
        use ErrorSeverity::{
            Error,
            Warning,
        };

        use super::CompilerIdentity as I;
        let c99 = self.standard >= super::CStandard::C99;
        match self.identity {
            | I::Tcc | I::Msvc => match group {
                | G::ImplicitFunctionDeclaration
                | G::ImplicitLibraryFunction
                | G::ReturnMismatch
                | G::VoidReturnMismatch => Some(Warning),
                | G::ImplicitInt => None,
                | G::DeclarationMissingParameterType if self.identity == I::Tcc => None,
                | _ => Some(Error),
            },
            | I::Clang | I::ClangCl => match group {
                | G::ImplicitFunctionDeclaration | G::ImplicitInt => c99.then_some(Error),
                // Clang warns about an implicit library function in C89.
                | G::ImplicitLibraryFunction => Some(if c99 { Error } else { Warning }),
                | _ => Some(Error),
            },
            | I::Gcc if c99 => Some(if self.permissive == Some(true) {
                Warning
            } else {
                Error
            }),
            | I::Gcc => match group {
                | G::ImplicitFunctionDeclaration | G::ImplicitInt | G::ReturnMismatch => None,
                // GCC's own "incompatible implicit declaration of built-in
                // function" warning stands in for a library function, and
                // GCC 14 keeps a conditional's mismatch a warning in C89
                // whatever `-fno-permissive` says.
                | G::ImplicitLibraryFunction | G::GccConditionalPointerMismatch => Some(Warning),
                // An explicit `-fno-permissive` makes the others errors.
                | _ if self.permissive == Some(false) => Some(Error),
                | _ => Some(Warning),
            },
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
    IncompatiblePointerTypes => Some("incompatible-pointer-types"), Permerror, true;
    PointerSign => Some("pointer-sign"), Warning, true;
    NonLiteralNullConversion => Some("non-literal-null-conversion"), Warning, false;
    PointerCompare => Some("pointer-compare"), Warning, false;
    NullConversion => Some("null-conversion"), Warning, false;
    /// GCC's `-Waddress`, which only its `-Wall` holds.
    PointerBoolConversion => Some("pointer-bool-conversion"), ClangWarning, false;
    /// A cast from a pointer to a narrower integer (GCC: an integer of any
    /// other width); the parent of Clang's enumeration and `void *` groups.
    PointerToIntCast => Some("pointer-to-int-cast"), Warning, false;
    /// Clang: a cast from a pointer to a narrower enumeration.
    PointerToEnumCast => Some("pointer-to-enum-cast"), Warning, false;
    /// Clang: a cast from a `void *` to a narrower integer.
    VoidPointerToIntCast => Some("void-pointer-to-int-cast"), Warning, false;
    /// Clang: a cast from a `void *` to a narrower enumeration.
    VoidPointerToEnumCast => Some("void-pointer-to-enum-cast"), Warning, false;
    /// A cast to a pointer from a narrower integer that is no constant
    /// (GCC: an integer of any other width); the parent of Clang's
    /// `void *` group.
    IntToPointerCast => Some("int-to-pointer-cast"), Warning, false;
    /// Clang: a cast to a `void *` from a narrower integer.
    IntToVoidPointerCast => Some("int-to-void-pointer-cast"), Warning, false;
    VoidPtrDereference => Some("void-ptr-dereference"), Warning, false;
    Nonnull => Some("nonnull"), Warning, false;
    PredefinedOutsideFunction => Some("predefined-identifier-outside-function"), Warning, false;
    PointerQualifiers => Some("incompatible-pointer-types-discards-qualifiers"), Warning, true;
    /// GCC's `-Wincompatible-pointer-types` holds it.
    IncompatibleFunctionPointerTypes => Some("incompatible-function-pointer-types"), Permerror, true;
    IntConversion => Some("int-conversion"), Permerror, true;
    /// A call to an undeclared function, which declares it `int ()`.
    ImplicitFunctionDeclaration => Some("implicit-function-declaration"), Permerror, false;
    /// A call to an undeclared library function, which declares it with
    /// its library type; Clang reports it in C89 too.
    ImplicitLibraryFunction => Some("implicit-function-declaration"), Permerror, false;
    /// A declaration without a type specifier, or an old-style parameter
    /// without a declaration, which defaults to `int`.
    ImplicitInt => Some("implicit-int"), Permerror, false;
    /// GCC's and Clang's `-Wimplicit`, the parent of the implicit
    /// declaration and implicit int groups; it reports nothing itself.
    Implicit => Some("implicit"), Ignored, false;
    /// `return;` in a function returning a value.
    ReturnMismatch => Some("return-mismatch"), Permerror, false;
    /// `return` with a value in a function returning `void`, which GCC
    /// reports in C89 too.
    VoidReturnMismatch => Some("return-mismatch"), Permerror, true;
    /// GCC: conditional operands that point to incompatible types, which
    /// GCC 14 reports under `-Wincompatible-pointer-types` (Clang:
    /// `PointerTypeMismatch`).
    GccConditionalPointerMismatch => Some("incompatible-pointer-types"), Permerror, true;
    /// GCC: an identifier list in a function declarator that is no
    /// definition (`void f(x);`). Clang refuses it outright.
    DeclarationMissingParameterType => Some("declaration-missing-parameter-type"), Permerror, false;
    /// Clang: an x86 builtin's immediate constant outside its range. A
    /// non-constant one, and GCC's out-of-range one, is an error.
    ArgumentOutsideRange => Some("argument-outside-range"), Error, false;
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
    /// The members of `-Wunknown-pragmas` that report by default: an
    /// unknown `STDC` pragma, and Clang's malformed diagnostic pragmas.
    UnknownStdcPragma => Some("unknown-pragmas"), Warning, false;
    /// GCC: a pragma it knows, used wrongly. In Clang the parent of
    /// `-Wunknown-pragmas`.
    Pragmas => Some("pragmas"), Warning, false;
    /// `#warning`; GCC's `-Wcpp` and Clang's `-W#warnings`, whose other name
    /// [`DiagnosticGroup::parse`] reads.
    Cpp => Some("cpp"), Warning, false;
    /// Clang: `#pragma GCC warning`. GCC names no option for it.
    PragmaMessages => Some("#pragma-messages"), Warning, false;
    /// Clang: tokens after a directive is complete.
    ExtraTokens => Some("extra-tokens"), Warning, false;
    /// Clang: a macro `#pragma clang deprecated` marks is expanded.
    DeprecatedPragma => Some("deprecated-pragma"), Warning, false;
    /// `#pragma once` in the main file.
    PragmaOnceOutsideHeader => Some("pragma-once-outside-header"), Warning, false;
    /// Clang: `#pragma GCC system_header` in the main file.
    PragmaSystemHeaderOutsideHeader => Some("pragma-system-header-outside-header"), Warning, false;
    /// Clang: `#include_next` in the main file.
    IncludeNextOutsideHeader => Some("include-next-outside-header"), Warning, false;
    /// Clang: `#include_next` in a file found by no search entry.
    IncludeNextAbsolutePath => Some("include-next-absolute-path"), Warning, false;
    /// Clang: a `'` or `"` that closes on no line, in text phase 7 never reads.
    InvalidPpToken => Some("invalid-pp-token"), Warning, false;
    /// Clang: a macro redefined differently; a warning `-pedantic-errors`
    /// promotes as it does in GCC, which names no option for it.
    MacroRedefined => Some("macro-redefined"), Warning, true;
    /// A `\` before a character that begins no escape sequence; Clang's
    /// extension warning, which `-pedantic-errors` promotes.
    UnknownEscapeSequence => Some("unknown-escape-sequence"), Warning, true;
    /// Clang: an unprefixed literal that holds bytes of malformed UTF-8;
    /// an extension warning, which `-pedantic-errors` promotes.
    InvalidSourceEncoding => Some("invalid-source-encoding"), Warning, true;
    UnknownWarningOption => Some("unknown-warning-option"), Warning, false;
    /// An attribute name unavailable in its spelling on the selected target.
    UnknownAttributes => Some("unknown-attributes"), Warning, false;
    /// A recognized attribute whose effect bcc drops.
    IgnoredAttributes => Some("ignored-attributes"), Warning, false;
    /// A handler calls a function that may change registers it must save.
    ExcessiveRegsave => Some("excessive-regsave"), Warning, false;
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
    /// An element beyond the object it initializes, or a string literal
    /// longer than its array, null character aside: a warning by default that `-pedantic-errors` makes
    /// an error, as in GCC (a pedantic warning without an option) and Clang.
    ExcessInitializers => Some("excess-initializers"), Warning, true;
    /// GCC: a floating constant converted out of an integer type's range in
    /// a static initializer.
    Overflow => Some("overflow"), Warning, false;
    /// GCC's pedantic "overflow in constant expression" for a static
    /// initializer: reported under `-pedantic`, an error under
    /// `-pedantic-errors`, and controlled by `-Woverflow`'s options, which
    /// disable it or set its severity but never enable it alone.
    PedanticOverflow => Some("overflow"), Extension, true;
    /// Clang: a signed constant operation out of its type's range. GCC's is
    /// [`Self::Overflow`].
    IntegerOverflow => Some("integer-overflow"), Warning, false;
    /// A shift count not below the width of the shifted type.
    ShiftCountOverflow => Some("shift-count-overflow"), Warning, false;
    /// A negative shift count.
    ShiftCountNegative => Some("shift-count-negative"), Warning, false;
    /// A nonnegative constant shifted left past the sign bit: Clang's option,
    /// and the parent of GCC's spelling.
    ShiftOverflow => Some("shift-overflow"), Warning, false;
    /// GCC's report of [`Self::ShiftOverflow`], which names the option with
    /// the level it takes.
    GccShiftOverflow => Some("shift-overflow="), Warning, false;
    /// Clang: a negative constant shifted left. GCC reports it only under
    /// `-Wextra`, which is not modeled.
    ShiftNegativeValue => Some("shift-negative-value"), Warning, false;
    /// An ordinary multi-character constant.
    Multichar => Some("multichar"), Warning, false;
    /// Clang: a declaration that declares nothing.
    MissingDeclarations => Some("missing-declarations"), Warning, false;
    /// GCC's report of the same, which no option controls but `-w`.
    GccEmptyDeclaration => None, Warning, false;
    /// GCC's word for a zero divisor, which also sets Clang's
    /// [`Self::DivisionByZero`], as Clang takes it as an alias.
    DivByZero => Some("div-by-zero"), Warning, false;
    /// Clang: an integer divided by a constant zero.
    DivisionByZero => Some("division-by-zero"), Warning, false;
    /// GCC: an identifier not in Normalization Form C, before C23, which
    /// does not require the form: a warning no pedantic option changes.
    Normalized => Some("normalized"), Warning, false;
    /// GCC: the same in C23, which requires the form, so `-pedantic-errors`
    /// makes it an error; `-Wnormalized`'s options set it with its parent.
    PedanticNormalized => Some("normalized"), Warning, true;
    /// Clang: a floating literal whose implicit conversion to an integer
    /// type is out of range or changes its value; `-Wconversion` holds it.
    LiteralConversion => Some("literal-conversion"), Warning, false;
    /// Clang's `-Wconversion`, which reports nothing itself: under Clang's
    /// rules the parent of the conversion groups bcc models
    /// (`DiagnosticGroup::converts`), which take its `-W`, `-Wno-` and
    /// `-Werror=` options unless they have their own.
    Conversion => Some("conversion"), Ignored, false;
    /// A signed operand the usual arithmetic conversions make unsigned, under
    /// `-Wextra`.
    SignCompare => Some("sign-compare"), Ignored, false;
    /// A braced list that leaves later members zero, under `-Wextra`.
    MissingFieldInitializers => Some("missing-field-initializers"), Ignored, false;
    /// A comparison the operands' types decide. `-Wextra` holds it in GCC;
    /// in Clang it holds the unsigned-zero and type-limit groups.
    TypeLimits => Some("type-limits"), Ignored, false;
    /// Clang's, on by default: holds the out-of-range group.
    TautologicalConstantCompare => Some("tautological-constant-compare"), ClangWarning, false;
    /// Clang's constant outside the operand type's range, on by default.
    TautologicalConstantOutOfRangeCompare =>
        Some("tautological-constant-out-of-range-compare"), ClangWarning, false;
    /// Clang's unsigned operand compared with zero.
    TautologicalUnsignedZeroCompare => Some("tautological-unsigned-zero-compare"), Ignored, false;
    /// Clang's constant at the limit of the operand type's range.
    TautologicalTypeLimitCompare => Some("tautological-type-limit-compare"), Ignored, false;
    /// `-Wextra`, which reports nothing itself.
    Extra => Some("extra"), Ignored, false;
    /// `-Wunused`, which reports nothing itself and holds
    /// [`DiagnosticGroup::UNUSED`]. `-Wall` holds it.
    Unused => Some("unused"), Ignored, false;
    /// A local variable, or a file-scope object in Clang's words, that
    /// nothing refers to. It holds the const group.
    UnusedVariable => Some("unused-variable"), Ignored, false;
    /// An internal `const` object in the main file that nothing refers to.
    UnusedConstVariable => Some("unused-const-variable"), Ignored, false;
    /// A parameter of a definition that nothing refers to, under `-Wextra`.
    UnusedParameter => Some("unused-parameter"), Ignored, false;
    /// A variable only ever assigned to.
    UnusedButSetVariable => Some("unused-but-set-variable"), Ignored, false;
    /// Clang: an internal object only ever assigned to.
    UnusedButSetGlobal => Some("unused-but-set-global"), Ignored, false;
    /// A parameter only ever assigned to, under `-Wextra`.
    UnusedButSetParameter => Some("unused-but-set-parameter"), Ignored, false;
    /// A label nothing jumps to.
    UnusedLabel => Some("unused-label"), Ignored, false;
    /// A statement's discarded value that computed nothing it needed: a
    /// warning by default in Clang, and under `-Wall` in GCC.
    UnusedValue => Some("unused-value"), ClangWarning, false;
    /// Clang: a discarded comparison, also a warning by default.
    UnusedComparison => Some("unused-comparison"), ClangWarning, false;
    /// The forms of `main` that Clang warns about and GCC's `-Wmain`, which
    /// is off until `-Wall` or `-Wpedantic` enables it and which every form
    /// `-pedantic-errors` makes an error of.
    Main => Some("main"), OnInClang, false;
    /// Clang's extension warnings about `main`, which `-pedantic-errors`
    /// makes errors; `-Wmain`'s options set them with the group.
    MainExtension => Some("main"), Warning, true;
    /// Clang: `main` returning anything but `int` in a GNU mode. GCC has no
    /// such option, its `-Wmain` holding this form.
    MainReturnType => Some("main-return-type"), Warning, true;
    /// GCC's warning about `inline` or `_Noreturn` on `main`, which no
    /// option controls and `-pedantic-errors` makes an error.
    MainSpecifier => None, Warning, true;
}

/// A group's severity when no option names it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupDefault {
    Error,
    /// Depends on the identity, the C revision and `-fpermissive`
    /// ([`CompilerConfiguration::permerror_default`]).
    Permerror,
    Warning,
    Ignored,
    Extension,
    /// A warning by default under Clang's rules, and otherwise off.
    ClangWarning,
    /// Clang's warning by default; GCC's only under `-Wpedantic`.
    OnInClang,
}

impl DiagnosticGroup {
    /// The groups GCC's `-Wunused` holds only with `-Wextra` too.
    const PARAMETERS: &[Self] = &[Self::UnusedParameter, Self::UnusedButSetParameter];
    /// The groups bcc models under `-Wall`, as a pragma naming it reaches
    /// them.
    const UNDER_ALL: &[Self] = &[
        Self::Unused,
        Self::ImplicitFunctionDeclaration,
        Self::ImplicitLibraryFunction,
        Self::ImplicitInt,
        Self::ReturnMismatch,
        Self::VoidReturnMismatch,
    ];
    /// The groups bcc models under `-Wextra`, as a pragma naming it reaches
    /// them.
    const UNDER_EXTRA: &[Self] = &[
        Self::SignCompare,
        Self::TypeLimits,
        Self::MissingFieldInitializers,
        Self::UnusedParameter,
        Self::UnusedButSetParameter,
    ];
    /// The groups `-Wunused` and so `-Wall` hold. `-Wunused-parameter` and
    /// `-Wunused-but-set-parameter` are `-Wextra`'s.
    pub(crate) const UNUSED: &[Self] = &[
        Self::UnusedFunction,
        Self::UnusedVariable,
        Self::UnusedConstVariable,
        Self::UnusedButSetVariable,
        Self::UnusedButSetGlobal,
        Self::UnusedLabel,
        Self::UnusedValue,
        Self::UnusedComparison,
    ];

    /// Whether Clang's `-Wconversion` holds this group.
    const fn converts(self) -> bool {
        matches!(
            self,
            Self::NullConversion
                | Self::NonLiteralNullConversion
                | Self::PointerBoolConversion
                | Self::ConstantConversion
                | Self::LiteralConversion
        )
    }

    /// Whether only GCC has the group, so that Clang reports its option as
    /// unknown.
    pub(crate) const fn gcc_only(self) -> bool {
        matches!(self, Self::Normalized | Self::PedanticNormalized)
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        if name == "#warnings" {
            return Some(Self::Cpp);
        }
        Self::ALL
            .iter()
            .copied()
            .find(|group| group.name() == Some(name))
    }

    pub(crate) const fn default_severity(self) -> ErrorSeverity {
        match self.default() {
            | GroupDefault::Error | GroupDefault::Permerror => ErrorSeverity::Error,
            | GroupDefault::Warning
            | GroupDefault::Ignored
            | GroupDefault::Extension
            | GroupDefault::ClangWarning
            | GroupDefault::OnInClang => ErrorSeverity::Warning,
        }
    }

    /// Whether the group reports anything before an option enables it.
    pub(crate) fn enabled_by_default(self) -> bool {
        matches!(
            self.default(),
            GroupDefault::Error
                | GroupDefault::Permerror
                | GroupDefault::Warning
                | GroupDefault::OnInClang
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WarningOption {
    Enable(bool),
    Error(bool),
}

/// A group's options, packed into a byte so that the per-group table keeps
/// [`CompilerConfiguration`] small enough to pass by value: whether it is
/// enabled and whether it is an error, each unset or set (bits 0-1 and
/// 2-3: 0 unset, 2 false, 3 true); bit 4 for a group re-enabled after it
/// was disabled, which Clang reports as a warning even when it defaults to
/// an error; and what the last `-Wall` or `-Wno-all` said (bits 5-6, as
/// the others), with bit 7 set while it came after the group's own options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WarningSettings(u8);

impl WarningSettings {
    const ALL: u32 = 5;
    const ALL_LAST: u8 = 1 << 7;
    pub(super) const DEFAULT: Self = Self(0);
    const ENABLED: u32 = 0;
    const ERROR: u32 = 2;
    const REENABLED: u8 = 1 << 4;

    const fn get(self, shift: u32) -> Option<bool> {
        match (self.0 >> shift) & 3 {
            | 0 => None,
            | 3 => Some(true),
            | _ => Some(false),
        }
    }

    const fn set(&mut self, shift: u32, value: Option<bool>) {
        let bits = match value {
            | None => 0,
            | Some(false) => 2,
            | Some(true) => 3,
        };
        self.0 = (self.0 & !(3 << shift)) | (bits << shift);
    }

    const fn enabled(self) -> Option<bool> {
        self.get(Self::ENABLED)
    }

    /// What the last `-Wall`, or the option standing for it here, said.
    const fn all(self) -> Option<bool> {
        self.get(Self::ALL)
    }

    const fn error(self) -> Option<bool> {
        self.get(Self::ERROR)
    }

    /// Clang re-enables a disabled default-error group as a warning.
    const fn reenabled_warning(self) -> bool {
        self.0 & Self::REENABLED != 0
    }

    /// Whether the group's options and `-Wall` enable or disable it.
    /// GCC's `-Wall` sets only what no option of the group's own set,
    /// whatever their order (`LangEnabledBy`), and holds the implicit
    /// groups; any `-Wall` or `-Wno-all` makes GCC 14 warn about C89's
    /// `return;` in a function with a result. Clang's `-Wall` and
    /// `-Wno-all` set every group they hold, `-Wreturn-mismatch` included,
    /// unless the group's own option comes later.
    const fn enabled_with_all(self, group: DiagnosticGroup, clang: bool) -> Option<bool> {
        let all = self.get(Self::ALL);
        let explicit = self.enabled();
        if clang {
            if self.0 & Self::ALL_LAST != 0 {
                all
            } else if explicit.is_some() {
                explicit
            } else {
                all
            }
        } else if explicit.is_some() {
            explicit
        } else {
            match group {
                | DiagnosticGroup::ImplicitFunctionDeclaration
                | DiagnosticGroup::ImplicitLibraryFunction
                | DiagnosticGroup::ImplicitInt
                | DiagnosticGroup::SignCompare
                | DiagnosticGroup::MissingFieldInitializers
                | DiagnosticGroup::TypeLimits
                | DiagnosticGroup::Unused
                | DiagnosticGroup::UnusedFunction
                | DiagnosticGroup::UnusedVariable
                | DiagnosticGroup::UnusedConstVariable
                | DiagnosticGroup::UnusedButSetVariable
                | DiagnosticGroup::UnusedButSetGlobal
                | DiagnosticGroup::UnusedLabel
                | DiagnosticGroup::UnusedValue
                | DiagnosticGroup::UnusedComparison
                | DiagnosticGroup::Main
                | DiagnosticGroup::PointerBoolConversion => all,
                | DiagnosticGroup::ReturnMismatch if all.is_some() => Some(true),
                | _ => None,
            }
        }
    }

    /// Records `-Wall` (true) or `-Wno-all`, which Clang's later `-Wall`
    /// re-enables from, as from a group's own `-Wno-`.
    const fn apply_all(&mut self, enabled: bool) {
        self.set(Self::ALL, Some(enabled));
        self.0 |= Self::ALL_LAST;
        if !enabled {
            self.0 |= Self::REENABLED;
        }
    }

    /// Applies a parent group's option to a child that only `-pedantic`
    /// enables: disabling it, or setting its severity, but leaving whether
    /// it reports to the extension policy otherwise.
    fn apply_to_pedantic(&mut self, option: WarningOption) {
        match option {
            | WarningOption::Enable(false) => self.apply(option, true),
            | WarningOption::Enable(true) => self.set(Self::ENABLED, None),
            | WarningOption::Error(error) => self.set(Self::ERROR, Some(error)),
        }
    }

    /// Applies one of the group's own options. Clang's `-Wno-X` forgets an
    /// earlier `-Werror=X`, which GCC's keeps for a later `-WX`; an option
    /// that enables or disables the group overrides an earlier Clang
    /// `-Wall`, while `-Wno-error=X` changes only its severity.
    fn apply(&mut self, option: WarningOption, clang: bool) {
        match option {
            | WarningOption::Enable(enabled) => {
                self.0 &= !Self::ALL_LAST;
                self.set(Self::ENABLED, Some(enabled));
                if !enabled {
                    self.0 |= Self::REENABLED;
                    if clang && self.error() == Some(true) {
                        self.set(Self::ERROR, None);
                    }
                }
            },
            | WarningOption::Error(error) => {
                self.set(Self::ERROR, Some(error));
                if error {
                    self.0 &= !Self::ALL_LAST;
                    self.set(Self::ENABLED, Some(true));
                }
            },
        }
    }
}
