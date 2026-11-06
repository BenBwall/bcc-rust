//! Chooses terminal color behavior from environment settings.

/// How rendered text is decorated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorChoice {
    Plain,
    Ansi,
}

impl ColorChoice {
    /// Colors as `requested` says, else only for a terminal, honoring
    /// `NO_COLOR` (<https://no-color.org>) and `CLICOLOR_FORCE`.
    #[expect(
        clippy::disallowed_methods,
        reason = "Reads `NO_COLOR` and `CLICOLOR_FORCE` once while the CLI sets up its renderer; \
                  std returns environment values owned."
    )]
    pub(crate) fn for_stderr(requested: Option<bool>) -> Self {
        use std::io::IsTerminal as _;
        match requested {
            | Some(true) => return Self::Ansi,
            | Some(false) => return Self::Plain,
            | None => {},
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
