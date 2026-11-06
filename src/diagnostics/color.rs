//! Chooses terminal color behavior from options and environment settings.

use crate::configuration::DiagnosticColor;

/// How rendered text is decorated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorChoice {
    Plain,
    Ansi,
}

impl ColorChoice {
    /// Colors as `requested` says. Without an option it colors only for a
    /// terminal, honoring `NO_COLOR` (<https://no-color.org>) and
    /// `CLICOLOR_FORCE`; GCC's and Clang's `auto` asks the terminal alone.
    #[expect(
        clippy::disallowed_methods,
        reason = "Reads `NO_COLOR` and `CLICOLOR_FORCE` once while the CLI sets up its renderer; \
                  std returns environment values owned."
    )]
    pub(crate) fn for_stderr(requested: DiagnosticColor) -> Self {
        use std::io::IsTerminal as _;
        match requested {
            | DiagnosticColor::Always => return Self::Ansi,
            | DiagnosticColor::Never => return Self::Plain,
            | DiagnosticColor::Auto =>
                return if std::io::stderr().is_terminal() {
                    Self::Ansi
                } else {
                    Self::Plain
                },
            | DiagnosticColor::Unset => {},
        }
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("NO_COLOR") {
            Self::Plain
        } else if set("CLICOLOR_FORCE") || std::io::stderr().is_terminal() {
            Self::Ansi
        } else {
            Self::Plain
        }
    }
}
