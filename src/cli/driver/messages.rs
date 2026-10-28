//! The driver's own messages, named as the compiler a personality mimics
//! names its driver's: `gcc: warning: ...`, `clang: error: ...`, `cl :
//! Command line warning : ...`, `tcc: error: ...`; bcc's own command line
//! writes them unnamed (`warning: ...`).

use std::fmt::{
    self,
    Write as _,
};

use super::super::{
    Cli,
    MainError,
    arguments::Identity,
    code_generation::report_stderr,
};
use crate::util::bump::{
    ArenaString,
    Bump,
};

/// How bad a driver message is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::cli) enum Severity {
    Warning,
    Error,
}

/// Writes a driver message to stderr, after the driver's name when
/// `--driver-name` gives one. Under the `cl` identity the message takes
/// `cl`'s command-line form without a code, since bcc invents none.
pub(in crate::cli) fn report(args: &Cli, severity: Severity, message: fmt::Arguments<'_>) {
    let label = match severity {
        | Severity::Warning => "warning",
        | Severity::Error => "error",
    };
    match args.driver.driver_name.as_deref() {
        | None => report_stderr(format_args!("{label}: {message}")),
        | Some(name) if args.compiler_identity == Identity::Msvc =>
            report_stderr(format_args!("{name} : Command line {label} : {message}")),
        | Some(name) => report_stderr(format_args!("{name}: {label}: {message}")),
    }
}

/// Reports `error`, which ends the driver, as [`report`] words an error, and
/// returns the error that stops the process with the failing status.
/// Errors that print themselves elsewhere pass through.
pub(in crate::cli) fn report_error(args: &Cli, error: MainError) -> MainError {
    if args.driver.driver_name.is_none()
        || matches!(
            error,
            MainError::Reported(_) | MainError::ParseArgumentsError(_)
        )
    {
        return error;
    }
    let arena = Bump::new();
    let mut text = ArenaString::new_in(&arena);
    _ = write!(text, "{error}");
    let message = text.strip_prefix("error: ").unwrap_or(&text);
    report(args, Severity::Error, format_args!("{message}"));
    MainError::Reported(1)
}
