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
    reason = "Carries the input path or an option out of `run`, past its arenas, for `main`'s \
              message."
)]
pub enum MainError {
    /// Reserving a native compilation stack failed before compiler setup.
    #[error("error: cannot create compilation stack: {0}")]
    CompilationStack(io::Error),
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError { path: PathBuf, source: io::Error },
    /// File output must not truncate or replace the source, including aliases.
    #[error("error: output file '{}' is the same as input file '{}'", output.display(), input.display())]
    OutputIsInput { input: PathBuf, output: PathBuf },
    /// An output whose identity cannot be checked is rejected before writing.
    #[error("error: cannot inspect output file '{}': {source}", path.display())]
    InspectOutputFileError { path: PathBuf, source: io::Error },
    /// An option the target does not support, as Clang rejects `-fPIC` for
    /// MSVC.
    #[error("error: unsupported option '{option}' for target '{target}'")]
    UnsupportedOption {
        option: String,
        target: &'static str,
    },
    /// A combination of driver options or inputs that cannot run, worded
    /// as GCC words it.
    #[error("error: {0}")]
    Driver(&'static str),
    /// A `-x` language the driver does not compile.
    #[error("error: language {0} not recognized")]
    LanguageNotRecognized(String),
    /// An output file that cannot be created or written.
    #[error("error: cannot write `{}`: {source}", path.display())]
    WriteOutputFileError { path: PathBuf, source: io::Error },
    /// A driver personality already reported why it stops; the process
    /// exits with this status and prints nothing more.
    #[error("")]
    Reported(i32),
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}
