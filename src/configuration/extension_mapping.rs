//! How the command line's warning options classify an extension diagnostic:
//! Clang's mapping, which its options set diagnostic by diagnostic in argv
//! order, and GCC's classification of `-Wpedantic`.
//!
//! C99: §4 paragraph 6, p. 7; PDF p. 19 permits extensions; §5.1.1.3
//! paragraph 1, p. 11; PDF p. 23 leaves the form and severity of their
//! diagnostics to the implementation.

use super::CompilerConfiguration;
use crate::translation_phases::ErrorSeverity;

impl CompilerConfiguration {
    /// The severity Clang gives an extension diagnostic of `class` under the
    /// command line's options, `None` where it reports nothing, as
    /// `DiagnosticIDs::getDiagnosticSeverity` computes it: the diagnostic's
    /// mapping (its default unless an option set it), raised to `-pedantic`'s
    /// or `-pedantic-errors`' level unless an option turned it off, and only
    /// to a warning after `-Wno-error=` named it; then `-w` silences it,
    /// as it silences every diagnostic that is not an error by default, and
    /// `-Werror` makes a warning an error unless `-Wno-error=` named it.
    /// Clang 23, `DiagnosticIDs.cpp`, `Warnings.cpp` and `Diagnostic.cpp`
    /// (`setDiagnosticGroupWarningAsError`).
    pub(crate) fn clang_extension_severity(self, class: ExtensionClass) -> Option<ErrorSeverity> {
        let mapping = self.extension_mappings.mapping(class);
        let no_werror = self.extension_mappings.no_werror(class);
        let mut level = match mapping {
            | Some(level) => level,
            | None if class.by_default() || self.everything => Level::Warning,
            | None => Level::Ignored,
        };
        if mapping != Some(Level::Ignored) {
            // A configuration whose policy no option set, as the explorer and
            // the library's callers build one, reads it as `-pedantic` or
            // `-pedantic-errors`; on the command line it says no more than
            // they and the options mapping the class do.
            let policy = match (mapping, self.extension_policy) {
                | (None, super::ExtensionPolicy::Deny) => Level::Error,
                | (None, super::ExtensionPolicy::Warn) => Level::Warning,
                | _ => Level::Ignored,
            };
            let behavior = if self.pedantic_errors {
                Level::Error
            } else if self.pedantic_flag {
                Level::Warning
            } else {
                Level::Ignored
            }
            .max(policy);
            level = level.max(if no_werror {
                behavior.min(Level::Warning)
            } else {
                behavior
            });
        }
        match level {
            | Level::Ignored => None,
            | _ if self.suppress_warnings => None,
            | Level::Warning if self.warnings_as_errors && !no_werror => Some(ErrorSeverity::Error),
            | Level::Warning => Some(ErrorSeverity::Warning),
            | Level::Error => Some(ErrorSeverity::Error),
        }
    }

    /// GCC's severity for one of its pedantic warnings (a `pedwarn`), filed
    /// as `filing` says, where `enabled` says whether the options enable it:
    /// `-w` silences it before anything else can make it an error;
    /// `-pedantic-errors` makes it an error, as `-Werror` does; and under
    /// `-Wpedantic` the last of `-pedantic-errors`, `-Werror=pedantic` and
    /// `-Wno-error=pedantic` classifies it in the end, the first two as an
    /// error and the last as a warning, which outranks both.
    /// `-Werror=pedantic` also enables it. An option `-Wpedantic` enables
    /// takes the error of the first two too, which `-Wno-error=pedantic`
    /// does not take back; one filed under no option takes neither.
    /// GCC 14, `diagnostic.cc` (`diagnostic_report_diagnostic`) and
    /// `opts.cc` (`OPT_pedantic_errors`).
    pub(crate) fn gcc_pedantic_severity(
        self,
        enabled: bool,
        filing: GccFiling,
    ) -> Option<ErrorSeverity> {
        if self.suppress_warnings || !(enabled || self.werror_pedantic().is_some()) {
            return None;
        }
        // A policy no option set reads as `-pedantic-errors`, as for Clang.
        let promoted = self.pedantic_errors
            || self.extension_policy == super::ExtensionPolicy::Deny
            || self.warnings_as_errors;
        let error = match (filing, self.extension_mappings.gcc_pedantic) {
            | (GccFiling::Pedantic, Some(level)) => level == Level::Error,
            | (GccFiling::Enabled, _) => promoted || self.extension_mappings.gcc_pedantic_error,
            | _ => promoted,
        };
        Some(if error {
            ErrorSeverity::Error
        } else {
            ErrorSeverity::Warning
        })
    }

    /// `-pedantic` itself, as Clang reads it apart from `-Wpedantic`: it
    /// raises every extension diagnostic no option has turned off to a
    /// warning, wherever it appears (`ProcessWarningOptions`). It does not
    /// change [`Self::extension_policy`], which `-Wpedantic` sets too.
    pub(crate) const fn with_pedantic_flag(mut self) -> Self {
        self.pedantic_flag = true;
        self
    }

    /// Records a warning option at its argv position in Clang's mapping of
    /// the extension diagnostics it reaches; see [`ExtensionMappings`].
    pub(crate) const fn with_extension_option(
        mut self,
        option: ExtensionOptionName,
        setting: super::WarningOption,
    ) -> Self {
        self.extension_mappings = self.extension_mappings.with_option(option, setting);
        self
    }
}

/// Where GCC files a pedantic warning, which decides the options that reach
/// it ([`CompilerConfiguration::gcc_pedantic_severity`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GccFiling {
    /// Under `-Wpedantic`, as its C front end files most of them.
    Pedantic,
    /// Under an option of its own that `-Wpedantic` enables and classifies,
    /// such as `-Wpointer-arith` ([`super::Feature::gcc_filing`]).
    Enabled,
    /// Under no option, or one that `-Wpedantic` does not classify, such as
    /// `-Wlong-long`, `-Wdeclaration-after-statement` and `-Wvla`, which
    /// `-pedantic` enables before C99 (`pedwarn_c90` in `c-errors.cc`)
    /// ([`super::Feature::gcc_filing`]).
    Unclassified,
    /// By its preprocessor, which files its pedantic warnings under options
    /// of their own or none, which bcc does not model one by one: what bcc's
    /// preprocessor reports, before the parser starts, keeps the extension
    /// policy's severity.
    Preprocessor,
}

/// The extension diagnostics bcc reports under the extension policy, by the
/// warning options that reach them in Clang. `-Wpedantic` holds the
/// extensions Clang reports only on request (`InferPedantic` in
/// `ClangDiagnosticsEmitter.cpp`); a later revision's syntax is also in that
/// revision's group (`-Wc11-extensions`, `-Wc23-extensions`). What Clang
/// warns of by default, such as a label at the end of a block before C23, is
/// in its own group alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtensionClass {
    /// In `-Wpedantic` alone, as far as bcc models it: GNU and MSVC syntax,
    /// C99's before C99, and `#warning` before C23.
    Pedantic,
    /// In `-Wpedantic` and `-Wc11-extensions`.
    C11,
    /// In `-Wpedantic` and `-Wc23-extensions`.
    C23,
    /// An omitted variadic argument before C23 (`ext_c_missing_varargs_arg`):
    /// in `-Wpedantic` and `-Wvariadic-macro-arguments-omitted`, which
    /// `-Wc23-extensions`, `-Wgnu-zero-variadic-macro-arguments` (and so
    /// `-Wgnu`), `-Wpre-c23-compat` and `-Wc++20-extensions` hold.
    OmittedVariadicArguments,
    /// A warning by default, in `-Wc23-extensions` alone.
    C23ByDefault,
    /// A warning by default, in `-Wtypedef-redefinition` alone: a typedef
    /// redefinition before C11 (`ext_redefinition_of_typedef`, an `ExtWarn`).
    TypedefRedefinition,
    /// A warning by default, in no group bcc models.
    ByDefault,
}

impl ExtensionClass {
    const ALL: [Self; 7] = [
        Self::Pedantic,
        Self::C11,
        Self::C23,
        Self::OmittedVariadicArguments,
        Self::C23ByDefault,
        Self::TypedefRedefinition,
        Self::ByDefault,
    ];

    /// Whether Clang warns of the class's diagnostics by default.
    const fn by_default(self) -> bool {
        matches!(
            self,
            Self::C23ByDefault | Self::TypedefRedefinition | Self::ByDefault
        )
    }

    /// The group Clang's tag names after a diagnostic of the class
    /// (`printDiagnosticOptions` in `TextDiagnosticPrinter.cpp`): its own
    /// group where bcc models it, and otherwise `-Wpedantic`, which names
    /// the extensions in no group.
    pub(crate) const fn clang_group(self) -> ExtensionGroup {
        match self {
            | Self::C11 => ExtensionGroup::C11Extensions,
            | Self::C23 | Self::C23ByDefault => ExtensionGroup::C23Extensions,
            | Self::OmittedVariadicArguments => ExtensionGroup::VariadicMacroArgumentsOmitted,
            | Self::TypedefRedefinition => ExtensionGroup::TypedefRedefinition,
            | Self::Pedantic | Self::ByDefault => ExtensionGroup::Pedantic,
        }
    }

    /// The class of an extension diagnostic of syntax from `origin`, of
    /// `feature` where the caller names it, which Clang warns of by default
    /// when `by_default` is set.
    pub(crate) const fn of(
        origin: super::FeatureOrigin,
        feature: Option<super::Feature>,
        by_default: bool,
    ) -> Self {
        use super::{
            CStandard,
            FeatureOrigin,
        };
        match feature {
            // `ext_redefinition_of_typedef`, which Clang warns of by default.
            | Some(super::Feature::TypedefRedefinition) => return Self::TypedefRedefinition,
            | Some(super::Feature::OmittedVariadicArguments) =>
                return Self::OmittedVariadicArguments,
            // `ext_pp_warning_directive`, in no group.
            | Some(super::Feature::WarningDirective) if by_default => return Self::ByDefault,
            | Some(super::Feature::WarningDirective) => return Self::Pedantic,
            | _ => {},
        }
        match origin {
            | FeatureOrigin::Standard(CStandard::C23) if by_default => Self::C23ByDefault,
            | _ if by_default => Self::ByDefault,
            | FeatureOrigin::Standard(CStandard::C23) => Self::C23,
            | FeatureOrigin::Standard(CStandard::C11) => Self::C11,
            | _ => Self::Pedantic,
        }
    }
}

/// The warning option an extension diagnostic's tag names: the group its
/// compiler files it under where bcc models that group
/// ([`ExtensionClass::clang_group`], [`super::Feature::clang_group`],
/// [`super::Feature::gcc_group`]), and otherwise `-Wpedantic`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtensionGroup {
    Pedantic,
    C11Extensions,
    C23Extensions,
    TypedefRedefinition,
    /// GCC's `-Wpointer-arith`.
    PointerArith,
    /// GCC's `-Wlong-long`.
    LongLong,
    /// GCC's `-Wdeclaration-after-statement`.
    DeclarationAfterStatement,
    /// GCC's `-Wvla`.
    Vla,
    /// GCC's and Clang's `-Wvariadic-macros`.
    VariadicMacros,
    /// Clang's `-Wvariadic-macro-arguments-omitted`.
    VariadicMacroArgumentsOmitted,
}

impl ExtensionGroup {
    /// The group's name, without `-W`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::Pedantic => "pedantic",
            | Self::C11Extensions => "c11-extensions",
            | Self::C23Extensions => "c23-extensions",
            | Self::TypedefRedefinition => "typedef-redefinition",
            | Self::PointerArith => "pointer-arith",
            | Self::LongLong => "long-long",
            | Self::DeclarationAfterStatement => "declaration-after-statement",
            | Self::Vla => "vla",
            | Self::VariadicMacros => "variadic-macros",
            | Self::VariadicMacroArgumentsOmitted => "variadic-macro-arguments-omitted",
        }
    }
}

/// The warning options that set Clang's mapping of extension diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtensionOptionName {
    /// `-Wpedantic` and its `-Wno-`, `-Werror=` and `-Wno-error=` forms.
    Pedantic,
    C11Extensions,
    C23Extensions,
    TypedefRedefinition,
    /// The groups that hold `-Wvariadic-macro-arguments-omitted` and
    /// nothing else bcc models: itself, `-Wgnu`,
    /// `-Wgnu-zero-variadic-macro-arguments`, `-Wpre-c23-compat` (with its
    /// `-pedantic` form and their `c2x` spellings) and `-Wc++20-extensions`
    /// (`c++2a`). Clang 23, `DiagnosticGroups.td`.
    VariadicMacroArgumentsOmitted,
    /// `-Wno-everything`, which turns every diagnostic off.
    Everything,
    /// `-pedantic-errors`, which GCC also reads as `-Werror=pedantic`.
    PedanticErrors,
}

impl ExtensionOptionName {
    /// The option a `-W` group name, without `-W`, sets only Clang's mapping
    /// of extension diagnostics through, for the names bcc has no warning
    /// group for.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            | "typedef-redefinition" => Some(Self::TypedefRedefinition),
            | "variadic-macro-arguments-omitted"
            | "gnu-zero-variadic-macro-arguments"
            | "gnu"
            | "pre-c23-compat"
            | "pre-c2x-compat"
            | "pre-c23-compat-pedantic"
            | "pre-c2x-compat-pedantic"
            | "c++20-extensions"
            | "c++2a-extensions" => Some(Self::VariadicMacroArgumentsOmitted),
            | _ => None,
        }
    }

    /// Whether the option reaches the diagnostics of `class`.
    const fn reaches(self, class: ExtensionClass) -> bool {
        match self {
            | Self::Pedantic => matches!(
                class,
                ExtensionClass::Pedantic
                    | ExtensionClass::C11
                    | ExtensionClass::C23
                    | ExtensionClass::OmittedVariadicArguments
            ),
            | Self::C11Extensions => matches!(class, ExtensionClass::C11),
            | Self::C23Extensions => matches!(
                class,
                ExtensionClass::C23
                    | ExtensionClass::C23ByDefault
                    | ExtensionClass::OmittedVariadicArguments
            ),
            | Self::TypedefRedefinition => matches!(class, ExtensionClass::TypedefRedefinition),
            | Self::VariadicMacroArgumentsOmitted =>
                matches!(class, ExtensionClass::OmittedVariadicArguments),
            | Self::Everything => true,
            | Self::PedanticErrors => false,
        }
    }
}

/// A diagnostic's level in Clang's mapping, ordered as it compares them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Level {
    Ignored,
    Warning,
    Error,
}

/// Clang's mapping of each [`ExtensionClass`], one byte each: the level an
/// option set (bits 0-1: 0 for none, which leaves the default, then
/// [`Level`] plus one) and whether `-Wno-error=` named it (bit 2), which
/// no later option clears. `-WX` maps X's diagnostics to a warning, `-Wno-X`
/// turns them off, `-Werror=X` maps them to an error, and `-Wno-error=X`
/// takes an error mapping back to a warning (`DiagnosticsEngine::setSeverity`
/// keeps the flag). Also GCC's classification of `-Wpedantic`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ExtensionMappings {
    classes:            [u8; ExtensionClass::ALL.len()],
    /// The last of `-pedantic-errors` and `-Werror=pedantic` (an error) and
    /// `-Wno-error=pedantic` (a warning), which classifies GCC's `-Wpedantic`
    /// diagnostics.
    gcc_pedantic:       Option<Level>,
    /// Whether `-pedantic-errors` or `-Werror=pedantic` appeared, which
    /// makes the options `-Wpedantic` enables errors for good.
    gcc_pedantic_error: bool,
}

impl ExtensionMappings {
    pub(super) const NONE: Self = Self {
        classes:            [0; ExtensionClass::ALL.len()],
        gcc_pedantic:       None,
        gcc_pedantic_error: false,
    };
    const NO_WERROR: u8 = 1 << 2;

    const fn mapping(self, class: ExtensionClass) -> Option<Level> {
        match self.classes[class as usize] & 3 {
            | 0 => None,
            | 1 => Some(Level::Ignored),
            | 2 => Some(Level::Warning),
            | _ => Some(Level::Error),
        }
    }

    const fn no_werror(self, class: ExtensionClass) -> bool {
        self.classes[class as usize] & Self::NO_WERROR != 0
    }

    const fn with_option(
        mut self,
        option: ExtensionOptionName,
        setting: super::WarningOption,
    ) -> Self {
        use super::WarningOption;
        if matches!(
            option,
            ExtensionOptionName::Pedantic | ExtensionOptionName::PedanticErrors
        ) {
            match setting {
                | WarningOption::Error(true) => {
                    self.gcc_pedantic = Some(Level::Error);
                    self.gcc_pedantic_error = true;
                },
                | WarningOption::Error(false) => self.gcc_pedantic = Some(Level::Warning),
                | WarningOption::Enable(_) => {},
            }
        }
        let mut index = 0;
        while index < ExtensionClass::ALL.len() {
            if option.reaches(ExtensionClass::ALL[index]) {
                let bits = &mut self.classes[index];
                let level = match setting {
                    // A command-line `-WX` keeps an error mapping
                    // (`DiagnosticsEngine::setSeverity`).
                    | WarningOption::Enable(true) if *bits & 3 == Level::Error as u8 + 1 => None,
                    | WarningOption::Enable(true) => Some(Level::Warning),
                    | WarningOption::Enable(false) => Some(Level::Ignored),
                    | WarningOption::Error(true) => Some(Level::Error),
                    | WarningOption::Error(false) => {
                        *bits |= Self::NO_WERROR;
                        if *bits & 3 == Level::Error as u8 + 1 {
                            Some(Level::Warning)
                        } else {
                            None
                        }
                    },
                };
                if let Some(level) = level {
                    *bits = (*bits & !3) | (level as u8 + 1);
                }
            }
            index += 1;
        }
        self
    }
}
