//! Errors that leave the command-line driver retain owned argument or
//! input-path information after compilation arenas have been released.

use std::io;
#[expect(
    clippy::disallowed_types,
    reason = "clap parses path arguments into `PathBuf`s; the compiler borrows them as `&Path`."
)]
use std::path::PathBuf;

use thiserror::Error;

#[doc(hidden)]
#[derive(Debug, Error)]
#[expect(
    clippy::disallowed_types,
    reason = "Carries the input path out of `run`, past its arenas, for `main`'s message."
)]
pub enum MainError {
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError { path: PathBuf, source: io::Error },
    /// File output must not truncate or replace the source, including aliases.
    #[error("error: output file '{}' is the same as input file '{}'", output.display(), input.display())]
    OutputIsInput { input: PathBuf, output: PathBuf },
    /// An output whose identity cannot be checked is rejected before writing.
    #[error("error: cannot inspect output file '{}': {source}", path.display())]
    InspectOutputFileError { path: PathBuf, source: io::Error },
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}
