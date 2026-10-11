//! Errors that leave the command-line driver retain owned argument or
//! input-path information after compilation arenas have been released.

#[expect(
    clippy::disallowed_types,
    reason = "clap parses path arguments into `PathBuf`s; the compiler borrows them as `&Path`."
)]
use std::path::PathBuf;

use super::{
    Error,
    io,
};

#[doc(hidden)]
#[derive(Debug, Error)]
#[expect(
    clippy::disallowed_types,
    reason = "Carries the input path out of `run`, past its arenas, for `main`'s message."
)]
pub enum MainError {
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError { path: PathBuf, source: io::Error },
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}
